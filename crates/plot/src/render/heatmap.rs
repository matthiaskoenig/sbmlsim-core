//! Heatmaps and colour bars.
//!
//! The cell in row `i` and column `j` of a heatmap spans `j - 0.5..j + 0.5` on the x axis and
//! `i - 0.5..i + 0.5` on the y axis, the positions of the categories of a category axis. An
//! axis of the plot which is linear and has no categories of its own becomes the category axis
//! of the names of the first heatmap of the plot ([`plot_axes`]): its columns along x and its
//! rows along y, row 0 at the bottom unless the axis is reversed. Since 0.30.0 a heatmap may
//! give the edges of its cells along an axis instead (`x_edges`, `y_edges`): its column `j`
//! spans `x_edges[j]..x_edges[j + 1]` of a numeric x axis (its row `i` likewise on y), which
//! stays a numeric axis another plot may share, e.g. the time of a space-time heatmap.
//!
//! The crate computes the colour of every cell with [`color_scale`] and draws the cells as
//! rings with that fill, one Charton layer per colour, so no colour depends on the order in
//! which Charton sees categories (the risk the spike names). The text of a cell is dark on a
//! light cell and light on a dark one, in one size for every cell, which the Inter measure fits
//! into the cells.
//!
//! The colour bars of a plot are drawn by the crate right of the panel inside the cell of the
//! plot, side by side in their order: a gradient of the scale from its low end at the bottom of
//! the panel to its high end at its top, or a band per integer of a categorical bar, with ticks
//! and labels of the values of the scale (or the given ones) and the label of the bar. Since
//! 0.31.0 a bar may go above the panels of its row or into its panel as an inset, horizontal
//! from its low end at the left, carry marks across it at values, and take a length and a
//! thickness of its own.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use charton::error::ChartonError;
use charton::prelude::*;

use super::axis::{self, ResolvedAxis, X_TICK_ROOM};
use super::marks::{Layers, dataset};
use super::stats_marks::rings_layer;
use crate::FontWeight;
use crate::colors::{color_of, color_scale, dark_text_on};
use crate::layout::Rect;
use crate::spec::{
    Axis, AxisScale, ColorBar, ColorBarPlacement, ColorScale, ElementKind, Fonts, LegendLoc, Plot,
    Style, YAxisRef,
};
use crate::svg::{self, LayerInfo, theme};
use crate::text::{extra_lines, label_height, label_width, text_width_one_line};

/// The share of the width of a cell its text may take.
const CELL_TEXT_WIDTH: f64 = 0.9;
/// The share of the height of a cell its text may take.
const CELL_TEXT_HEIGHT: f64 = 0.8;
/// The smallest font size of the text of a cell; text which only fits smaller is not drawn.
const MIN_CELL_TEXT: f64 = 6.0;

/// The gap between the panel and a colour bar in pixels.
pub(crate) const COLORBAR_GAP: f64 = 12.0;
/// The width of a colour bar in pixels.
pub(crate) const COLORBAR_WIDTH: f64 = 12.0;
/// The length of a tick of a colour bar in pixels.
const COLORBAR_TICK: f64 = 4.0;
/// The gap between a tick of a colour bar and its label in pixels.
const COLORBAR_TICK_PADDING: f64 = 3.0;
/// The gap between the tick labels of a colour bar and its label in pixels.
const COLORBAR_LABEL_GAP: f64 = 8.0;
/// The pixels one tick of a colour bar is given: denser than an axis, its labels are short and
/// stacked.
const COLORBAR_TICK_ROOM: f64 = 30.0;
/// The intervals of the gradient of a colour bar; the centre of a scale is a stop of its own.
const GRADIENT_STEPS: usize = 64;

/// The axes of a plot with the categories of its first heatmap: an axis which is linear and
/// has no categories becomes the category axis of the columns (x) or rows (y) of the heatmap.
pub(crate) fn plot_axes(plot: &Plot) -> (Cow<'_, Axis>, Cow<'_, Axis>) {
    let first = plot.elements.iter().find_map(|e| match &e.kind {
        ElementKind::Heatmap {
            rows,
            columns,
            x_edges,
            y_edges,
            ..
        } if e.axis == YAxisRef::Y => Some((
            y_edges.is_none().then_some(rows),
            x_edges.is_none().then_some(columns),
        )),
        _ => None,
    });
    let Some((rows, columns)) = first else {
        return (Cow::Borrowed(&plot.x), Cow::Borrowed(&plot.y));
    };
    // an axis along which the heatmap gives edges stays numeric
    let categorize = |axis: &Axis, names: Option<&Vec<String>>| -> Option<Axis> {
        let names = names?;
        (axis.scale == AxisScale::Linear && axis.categories.is_none()).then(|| Axis {
            scale: AxisScale::Category,
            categories: Some(names.to_vec()),
            frame: None,
            ..axis.clone()
        })
    };
    let x = categorize(&plot.x, columns).map_or(Cow::Borrowed(&plot.x), Cow::Owned);
    let y = categorize(&plot.y, rows).map_or(Cow::Borrowed(&plot.y), Cow::Owned);
    (x, y)
}

/// The cells of a heatmap.
pub(crate) struct Cells<'a> {
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub columns: usize,
    /// Values per row.
    pub values: &'a [Vec<f64>],
    /// Text per row.
    pub text: Option<&'a [Vec<String>]>,
    /// The colour scale of the values.
    pub scale: &'a ColorScale,
    /// The edges of the columns along x, `None` for the categories of x.
    pub x_edges: Option<&'a [f64]>,
    /// The edges of the rows along y, `None` for the categories of y.
    pub y_edges: Option<&'a [f64]>,
}

impl<'a> Cells<'a> {
    /// The cells of a heatmap of the spec.
    pub(crate) fn of(
        (rows, columns): (&'a [String], &'a [String]),
        values: &'a [Vec<f64>],
        text: Option<&'a [Vec<String>]>,
        scale: &'a ColorScale,
        (x_edges, y_edges): (Option<&'a [f64]>, Option<&'a [f64]>),
    ) -> Self {
        let (rows, columns) = crate::spec::heatmap_size(rows, columns, x_edges, y_edges);
        Cells {
            rows,
            columns,
            values,
            text,
            scale,
            x_edges,
            y_edges,
        }
    }

    /// The span of the column `j` on the x axis in its values.
    pub(crate) fn x_span(&self, j: usize) -> (f64, f64) {
        cell_span(self.x_edges, j)
    }

    /// The span of the row `i` on the y axis in its values.
    pub(crate) fn y_span(&self, i: usize) -> (f64, f64) {
        cell_span(self.y_edges, i)
    }
}

/// The span of the cell `k` along an axis of a heatmap in the values of the axis: between its
/// edges, or around the position `k` of its category without edges.
pub(crate) fn cell_span(edges: Option<&[f64]>, k: usize) -> (f64, f64) {
    match edges {
        Some(e) => (e[k], e[k + 1]),
        None => (k as f64 - 0.5, k as f64 + 0.5),
    }
}

/// The layers of a heatmap: a ring per cell in the colour of its value, a layer per colour,
/// and the text of the cells in two layers, dark and light.
#[allow(clippy::too_many_arguments)]
pub(crate) fn heatmap_layers(
    layers: &mut Layers,
    index: usize,
    cells: Cells<'_>,
    style: &Style,
    fonts: &Fonts,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
    (width, height): (f64, f64),
) -> Result<(), ChartonError> {
    // the colour of every cell by its row and column; a value the spec lacks has no colour
    let value = |i: usize, j: usize| {
        cells
            .values
            .get(i)
            .and_then(|r| r.get(j))
            .copied()
            .unwrap_or(f64::NAN)
    };
    let flat: Vec<f64> = (0..cells.rows)
        .flat_map(|i| (0..cells.columns).map(move |j| (i, j)))
        .map(|(i, j)| value(i, j))
        .collect();
    let colors = color_scale(cells.scale, &flat);
    let info = || LayerInfo {
        element: Some(index),
        marker: None,
    };
    // the cells of every colour, in the order of the colours
    let mut by_color: BTreeMap<&str, Vec<Vec<(f64, f64)>>> = BTreeMap::new();
    let mut centers = Vec::new();
    for i in 0..cells.rows {
        for j in 0..cells.columns {
            let ((a, b), (c, d)) = (cells.x_span(j), cells.y_span(i));
            let (x0, x1) = (xa.u(a), xa.u(b));
            let (y0, y1) = (ya.u(c), ya.u(d));
            if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
                continue;
            }
            let color = colors[i * cells.columns + j].as_str();
            by_color
                .entry(color)
                .or_default()
                .push(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
            centers.push((i, j, (x0 + x1) / 2.0, (y0 + y1) / 2.0, color));
        }
    }
    for (color, rings) in &by_color {
        if let Some(l) = rings_layer(rings, Some((color, style.opacity)), None)? {
            layers.push(l, info());
        }
    }
    let Some(text) = cells.text else {
        return Ok(());
    };
    // one size for every cell: the size of the ticks, smaller where the widest text needs it
    // in the narrowest cell (every cell of a category axis is one unit wide)
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    let narrowest = |n: usize, span: &dyn Fn(usize) -> (f64, f64), axis: &ResolvedAxis| {
        (0..n)
            .map(|k| {
                let (a, b) = span(k);
                (axis.u(b) - axis.u(a)).abs()
            })
            .filter(|d| d.is_finite())
            .fold(f64::INFINITY, f64::min)
    };
    let cell_w = width * narrowest(cells.columns, &|j| cells.x_span(j), xa)
        / (x1 - x0).max(f64::MIN_POSITIVE);
    let cell_h =
        height * narrowest(cells.rows, &|i| cells.y_span(i), ya) / (y1 - y0).max(f64::MIN_POSITIVE);
    let widest = text
        .iter()
        .flatten()
        .map(|t| text_width_one_line(t, fonts.tick, FontWeight::Normal))
        .fold(0.0, f64::max);
    let mut size = fonts.tick.min(cell_h * CELL_TEXT_HEIGHT);
    if widest > 0.0 {
        size = size.min(fonts.tick * CELL_TEXT_WIDTH * cell_w / widest);
    }
    if size.is_nan() || size < MIN_CELL_TEXT {
        return Ok(());
    }
    for dark in [true, false] {
        let (mut xs, mut ys, mut ts) = (Vec::new(), Vec::new(), Vec::new());
        for (i, j, x, y, color) in &centers {
            let Some(t) = text.get(*i).and_then(|r| r.get(*j)) else {
                continue;
            };
            if t.is_empty() || dark_text_on(color) != dark {
                continue;
            }
            xs.push(*x);
            ys.push(*y);
            ts.push(t.clone());
        }
        if xs.is_empty() {
            continue;
        }
        let ds = dataset(vec![
            ("x", xs.into_column()),
            ("y", ys.into_column()),
            ("t", ts.into_column()),
        ])?;
        let ink = if dark { theme::FG } else { theme::BACKGROUND };
        let opacity = style.opacity;
        let chart = Chart::build(&ds)?
            .mark_text()?
            .configure_text(move |t| {
                t.with_color(ink)
                    .with_size(size)
                    .with_anchor("middle")
                    .with_opacity(opacity)
            })
            .encode((alt::x("x"), alt::y("y"), alt::text("t")))?;
        layers.push(chart, info());
    }
    Ok(())
}

/// A colour bar laid out: its ticks and the room it takes.
pub(crate) struct ColorBarLayout {
    /// The ticks as shares from the low end of the bar with their labels.
    ticks: Vec<(f64, String)>,
    /// The width of the widest tick label in pixels.
    labels_width: f64,
    /// The height of the tallest tick label in pixels (its lines).
    labels_height: f64,
    /// The pixels the label of the bar takes across the bar with its gap, 0 without one.
    label_room: f64,
    /// Where the bar goes.
    pub placement: ColorBarPlacement,
    /// The thickness of the bar in pixels.
    thickness: f64,
    /// The length of the bar as a share of what it runs along.
    length: f64,
    /// The pixels the bar takes: right of the panel (or the bar before it) for a bar right of
    /// it, above the row for a bar above it, 0 for an inset.
    pub room: f64,
}

/// The bands of a categorical bar: one per integer from `low` to `high`.
fn bands(s: &ColorScale) -> f64 {
    s.high - s.low + 1.0
}

/// The length of a bar of a placement as a share of what it runs along when the spec gives
/// none.
fn default_length(placement: ColorBarPlacement) -> f64 {
    match placement {
        ColorBarPlacement::Right | ColorBarPlacement::Top => 1.0,
        ColorBarPlacement::Inset => 0.5,
    }
}

/// Lays out a colour bar whose side is expected to be `along` pixels long: the height of its
/// panel for a bar right of it, the width of its panel for an inset, the width of the figure
/// for a bar above a row.
///
/// The ticks are the given ones, else the ones of an axis from the low to the high end of the
/// scale, linear or logarithmic (the integers among them on a categorical bar); their labels
/// are the given ones, else their numbers. The room is measured with the Inter widths of the
/// labels, of their lines and superscripts.
pub(crate) fn colorbar_layout(bar: &ColorBar, fonts: &Fonts, along: f64) -> ColorBarLayout {
    let s = &bar.scale;
    let placement = bar.placement;
    let length = bar.length.unwrap_or(default_length(placement));
    let thickness = bar.thickness.unwrap_or(COLORBAR_WIDTH);
    let horizontal = placement != ColorBarPlacement::Right;
    let scale_axis = Axis {
        label: None,
        unit: None,
        scale: if s.log {
            AxisScale::Log
        } else {
            AxisScale::Linear
        },
        categories: None,
        min: Some(s.low),
        max: Some(s.high),
        frame: None,
        max_decades: None,
        reverse: false,
        grid: false,
        label_visible: true,
        ticks_visible: true,
        tick_marks_visible: None,
        line_visible: true,
        tick_rotation: 0.0,
        ticks: None,
        tick_labels: None,
    };
    let room = if horizontal {
        X_TICK_ROOM
    } else {
        COLORBAR_TICK_ROOM
    };
    let resolved = || axis::resolve(&scale_axis, &[], &[], along * length, room);
    // the shares of the ticks from the low end of the bar with their labels
    let n = bands(s);
    // a categorical bar with labels and no ticks has a tick per band
    let band_ticks: Option<Vec<f64>> = match (&bar.ticks, &bar.tick_labels) {
        (None, Some(_)) if bar.categorical => {
            Some((0..n as usize).map(|k| s.low + k as f64).collect())
        }
        _ => None,
    };
    let ticks: Vec<(f64, String)> = match bar.ticks.as_ref().or(band_ticks.as_ref()) {
        Some(values) => {
            let labels = match &bar.tick_labels {
                Some(l) => l.clone(),
                None => axis::axis_labels(values, s.log, false).0,
            };
            values
                .iter()
                .zip(labels)
                .map(|(v, l)| {
                    let t = if bar.categorical {
                        (v - s.low + 0.5) / n
                    } else {
                        share_of_bar(s, *v)
                    };
                    (t, l)
                })
                .collect()
        }
        // the integers among the ticks of the axis, each at the middle of its band
        None if bar.categorical => resolved()
            .ticks
            .into_iter()
            .filter(|(v, _)| v.fract() == 0.0 && *v >= s.low && *v <= s.high)
            .map(|(v, l)| ((v - s.low + 0.5) / n, l))
            .collect(),
        None => {
            let r = resolved();
            let values: Vec<f64> = r.ticks.iter().map(|(v, _)| *v).collect();
            // the labels of the bar carry their powers of ten, a bar has no multiplier
            let (labels, _) = axis::axis_labels(&values, s.log, false);
            let r = ResolvedAxis {
                ticks: values.into_iter().zip(labels).collect(),
                ..r
            };
            r.tick_coordinates()
                .into_iter()
                .map(|(u, l)| (r.share(u), l))
                .collect()
        }
    };
    let size = fonts.y_tick();
    let labels_width = ticks
        .iter()
        .map(|(_, l)| label_width(l, size, fonts.tick_weight))
        .fold(0.0, f64::max);
    let labels_height = ticks
        .iter()
        .map(|(_, l)| label_height(l, size))
        .fold(0.0, f64::max);
    // a label of several lines stacks them outwards, away from the bar
    let label_room = match bar.label.as_deref() {
        Some(l) if !l.is_empty() => COLORBAR_LABEL_GAP + fonts.label + extra_lines(l, fonts.label),
        _ => 0.0,
    };
    let across = thickness + COLORBAR_TICK + COLORBAR_TICK_PADDING;
    let room = match placement {
        ColorBarPlacement::Right => COLORBAR_GAP + across + labels_width + label_room,
        ColorBarPlacement::Top => COLORBAR_GAP + across + labels_height + label_room,
        ColorBarPlacement::Inset => 0.0,
    };
    ColorBarLayout {
        ticks,
        labels_width,
        labels_height,
        label_room,
        placement,
        thickness,
        length,
        room,
    }
}

impl ColorBarLayout {
    /// The half widths of the first and the last tick label of a horizontal bar beyond the
    /// ends of the bar, at most, in pixels.
    fn overhang(&self, fonts: &Fonts) -> f64 {
        self.ticks
            .iter()
            .map(|(_, l)| label_width(l, fonts.y_tick(), fonts.tick_weight) / 2.0)
            .fold(0.0, f64::max)
    }

    /// The pixels a horizontal bar takes across it: the bar, its ticks, tick labels and label.
    fn depth(&self) -> f64 {
        self.thickness
            + COLORBAR_TICK
            + COLORBAR_TICK_PADDING
            + self.labels_height
            + self.label_room
    }

    /// The rectangle of an inset bar with its ticks and labels in a panel (since 0.31.0): the
    /// bar of its length of the width of the panel, the half of its widest tick label beside
    /// each end, at its location [`LEGEND_INSET`](crate::layout) from the edges of the panel.
    pub(crate) fn inset_rect(&self, bar: &ColorBar, panel: Rect, fonts: &Fonts) -> Rect {
        let w = self.length * panel.width + 2.0 * self.overhang(fonts);
        let loc = bar.loc.unwrap_or(LegendLoc::UpperRight);
        crate::layout::loc_rect(panel, (w, self.depth()), loc)
    }
}

/// The value of a colour scale at a share of its bar, from the low end (0) to the high end (1).
fn value_at(s: &ColorScale, t: f64) -> f64 {
    if s.log {
        let (a, b) = (libm::log10(s.low), libm::log10(s.high));
        libm::pow(10.0, a + t * (b - a))
    } else {
        s.low + t * (s.high - s.low)
    }
}

/// The share of the bar of a colour scale at a value.
fn share_of_bar(s: &ColorScale, v: f64) -> f64 {
    if s.log {
        let (a, b) = (libm::log10(s.low), libm::log10(s.high));
        (libm::log10(v) - a) / (b - a)
    } else {
        (v - s.low) / (s.high - s.low)
    }
}

/// Writes ` name="<number>"`.
fn attr(out: &mut String, name: &str, v: f64) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    svg::num(out, v);
    out.push('"');
}

/// The share of the bar of a scale where the map reads the share `p` of its table, for a scale
/// without a centre: `None` outside of the part of the map the scale takes.
fn bar_share_of_table(s: &ColorScale, p: f64) -> Option<f64> {
    if s.center.is_some() || s.end <= s.start {
        return None;
    }
    let t = (p - s.start) / (s.end - s.start);
    (t > 0.0 && t < 1.0).then_some(t)
}

/// Writes the gradient of a colour bar under `id`: the colours of the scale from its low end
/// to its high end, from the bottom up (vertical) or from the left to the right (horizontal).
fn gradient(defs: &mut String, id: &str, s: &ColorScale, horizontal: bool) {
    // the stops on a grid of the bar, at the centre and at the stops of a map of stops
    let mut stops: Vec<f64> = (0..=GRADIENT_STEPS)
        .map(|k| k as f64 / GRADIENT_STEPS as f64)
        .collect();
    if let Some(c) = s.center {
        let t = share_of_bar(s, c);
        if t > 0.0 && t < 1.0 {
            stops.push(t);
        }
    }
    for stop in s.stops.iter().flatten() {
        if let Some(t) = bar_share_of_table(s, stop.0) {
            stops.push(t);
        }
    }
    stops.sort_by(f64::total_cmp);
    stops.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if horizontal {
        let _ = write!(
            defs,
            "<linearGradient id=\"{id}\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">"
        );
    } else {
        let _ = write!(
            defs,
            "<linearGradient id=\"{id}\" x1=\"0\" y1=\"1\" x2=\"0\" y2=\"0\">"
        );
    }
    for t in stops {
        // the ends are the ends of the scale exactly, not the round trip through the value
        let v = if t == 0.0 {
            s.low
        } else if t == 1.0 {
            s.high
        } else {
            value_at(s, t)
        };
        let color = color_of(s, v);
        let _ = write!(
            defs,
            "<stop offset=\"{}\" stop-color=\"{color}\"/>",
            svg::trim(&format!("{t:.4}"))
        );
    }
    defs.push_str("</linearGradient>");
}

/// The rectangle of a colour bar along one direction: `(start, length)` of the bar on a side
/// of `(start, length)` pixels, its length a share of it and centred on it.
fn centred((start, length): (f64, f64), share: f64) -> (f64, f64) {
    let l = length * share;
    (start + (length - l) / 2.0, l)
}

/// Draws the colour bar `index` of a plot: the gradient into `defs` under the id
/// `<prefix>-colorbar` (the first bar) or `<prefix>-colorbar-<index>`, the bar, its marks, its
/// ticks, tick labels and label into `body`. A categorical bar is a band of one colour per
/// integer and needs no gradient.
///
/// `at` is where the bar goes: for a bar right of its panel `(x, y, _, height)` the left edge
/// of its room and the top and the height of the panel, for a horizontal bar `(x, y, width, _)`
/// the left edge, the top and the width of the bar itself.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_colorbar(
    body: &mut String,
    defs: &mut String,
    prefix: &str,
    index: usize,
    bar: &ColorBar,
    layout: &ColorBarLayout,
    fonts: &Fonts,
    at: Rect,
) {
    let s = &bar.scale;
    let horizontal = layout.placement != ColorBarPlacement::Right;
    let th = layout.thickness;
    // the bar: x, y, width and height, and where a share of it lies
    let (bx, by, bw, bh) = if horizontal {
        (at.x, at.y, at.width, th)
    } else {
        let (y, h) = centred((at.y, at.height), layout.length);
        (at.x + COLORBAR_GAP, y, th, h)
    };
    let along = |t: f64| {
        if horizontal {
            bx + bw * t
        } else {
            by + bh * (1.0 - t)
        }
    };
    let class = match layout.placement {
        ColorBarPlacement::Right => "sbmlsim-colorbar",
        ColorBarPlacement::Top => "sbmlsim-colorbar sbmlsim-colorbar-top",
        ColorBarPlacement::Inset => "sbmlsim-colorbar sbmlsim-colorbar-inset",
    };
    let _ = write!(body, "<g class=\"{class}\">");
    let rect = |body: &mut String, (x, y, w, h): (f64, f64, f64, f64)| {
        body.push_str("<rect");
        attr(body, "x", x);
        attr(body, "y", y);
        attr(body, "width", w);
        attr(body, "height", h);
    };
    if bar.categorical {
        // the bands from the low end, each from the edge of the one before it
        let n = bands(s) as usize;
        for k in 0..n {
            let (a, b) = (along(k as f64 / n as f64), along((k + 1) as f64 / n as f64));
            let r = if horizontal {
                (a, by, b - a, bh)
            } else {
                (bx, b, bw, a - b)
            };
            rect(body, r);
            let _ = write!(body, " fill=\"{}\"/>", color_of(s, s.low + k as f64));
        }
    } else {
        let id = if index == 0 {
            format!("{prefix}-colorbar")
        } else {
            format!("{prefix}-colorbar-{index}")
        };
        gradient(defs, &id, s, horizontal);
        rect(body, (bx, by, bw, bh));
        let _ = write!(body, " fill=\"url(#{id})\"/>");
    }
    // the marks across the bar (since 0.31.0), under its outline
    if let Some(marks) = bar.marks.as_ref().filter(|m| !m.is_empty()) {
        body.push_str("<g class=\"sbmlsim-colorbar-marks\">");
        for m in marks {
            let t = if bar.categorical {
                (m.value - s.low + 0.5) / bands(s)
            } else {
                share_of_bar(s, m.value)
            };
            let p = along(t);
            body.push_str("<line");
            if horizontal {
                attr(body, "x1", p);
                attr(body, "y1", by);
                attr(body, "x2", p);
                attr(body, "y2", by + bh);
            } else {
                attr(body, "x1", bx);
                attr(body, "y1", p);
                attr(body, "x2", bx + bw);
                attr(body, "y2", p);
            }
            let color = m.color.as_deref().unwrap_or("#000000");
            let _ = write!(body, " stroke=\"{color}\"");
            attr(body, "stroke-width", m.width);
            body.push_str("/>");
        }
        body.push_str("</g>");
    }
    rect(body, (bx, by, bw, bh));
    let _ = write!(
        body,
        " fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/>",
        theme::FG_SOFT
    );
    // the side of the ticks: right of a vertical bar, above a bar above a row, below an inset
    let up = layout.placement == ColorBarPlacement::Top;
    let edge = if !horizontal {
        bx + bw
    } else if up {
        by
    } else {
        by + bh
    };
    let out = if up { -1.0 } else { 1.0 };
    let _ = write!(
        body,
        "<g class=\"sbmlsim-colorbar-ticks\" stroke=\"{}\" stroke-width=\"1\">",
        theme::FG_SOFT
    );
    for (t, _) in &layout.ticks {
        let p = along(*t);
        body.push_str("<line");
        if horizontal {
            attr(body, "x1", p);
            attr(body, "y1", edge);
            attr(body, "x2", p);
            attr(body, "y2", edge + out * COLORBAR_TICK);
        } else {
            attr(body, "x1", edge);
            attr(body, "y1", p);
            attr(body, "x2", edge + COLORBAR_TICK);
            attr(body, "y2", p);
        }
        body.push_str("/>");
    }
    body.push_str("</g>");
    let size = fonts.y_tick();
    let _ = write!(
        body,
        "<g class=\"sbmlsim-colorbar-tick-labels\" fill=\"{}\"",
        theme::FG_SOFT
    );
    attr(body, "font-size", size);
    svg::attr_weight(body, fonts.tick_weight);
    body.push('>');
    let gap = COLORBAR_TICK + COLORBAR_TICK_PADDING;
    for (t, label) in &layout.ticks {
        let p = along(*t);
        body.push_str("<text");
        let (at, stack) = if !horizontal {
            let at = (edge + gap, p);
            attr(body, "x", at.0);
            attr(body, "y", at.1);
            body.push_str(" dominant-baseline=\"middle\">");
            (at, svg::Stack::Centred)
        } else if up {
            // the last line ends at the padding above the ticks
            let at = (p, edge - gap);
            attr(body, "x", at.0);
            attr(body, "y", at.1);
            body.push_str(" text-anchor=\"middle\" dominant-baseline=\"text-after-edge\">");
            (at, svg::Stack::Up)
        } else {
            let at = (p, edge + gap);
            attr(body, "x", at.0);
            attr(body, "y", at.1);
            body.push_str(" text-anchor=\"middle\" dominant-baseline=\"hanging\">");
            (at, svg::Stack::Down)
        };
        svg::label_lines(body, label, at, size, stack);
        body.push_str("</text>");
    }
    body.push_str("</g>");
    if let Some(label) = bar.label.as_deref().filter(|l| !l.is_empty()) {
        body.push_str("<g class=\"sbmlsim-colorbar-label\"><text");
        if !horizontal {
            let (tx, ty) = (
                edge + gap + layout.labels_width + COLORBAR_LABEL_GAP + fonts.label / 2.0,
                by + bh / 2.0,
            );
            attr(body, "x", tx);
            attr(body, "y", ty);
            attr(body, "font-size", fonts.label);
            svg::attr_weight(body, fonts.label_weight);
            let _ = write!(
                body,
                " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" transform=\"rotate(-90 ",
                theme::FG
            );
            svg::num(body, tx);
            body.push(' ');
            svg::num(body, ty);
            body.push_str(")\">");
            // the lines stack outwards, right of the bar: down in the rotated text
            svg::text_lines(body, label, (tx, ty), fonts.label, svg::Stack::Down);
        } else {
            let tx = bx + bw / 2.0;
            let reach = gap + layout.labels_height + COLORBAR_LABEL_GAP;
            let (ty, baseline, stack) = if up {
                (edge - reach, "text-after-edge", svg::Stack::Up)
            } else {
                (edge + reach, "hanging", svg::Stack::Down)
            };
            attr(body, "x", tx);
            attr(body, "y", ty);
            attr(body, "font-size", fonts.label);
            svg::attr_weight(body, fonts.label_weight);
            let _ = write!(
                body,
                " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"{baseline}\">",
                theme::FG
            );
            svg::text_lines(body, label, (tx, ty), fonts.label, stack);
        }
        body.push_str("</text></g>");
    }
    body.push_str("</g>");
}

/// Where a horizontal bar of a layout goes in the rectangle `block` of its ticks and labels
/// (an inset's, see [`ColorBarLayout::inset_rect`]): the rectangle of the bar itself.
pub(crate) fn inset_bar(layout: &ColorBarLayout, block: Rect, fonts: &Fonts) -> Rect {
    let o = layout.overhang(fonts);
    Rect {
        x: block.x + o,
        y: block.y,
        width: (block.width - 2.0 * o).max(0.0),
        height: layout.thickness,
    }
}

/// The rectangle of a bar above a row: its length of the span `(left, right)` of the panels of
/// the row centred on it, its top above `top`, the top of the room of the row, by its label and
/// tick labels.
pub(crate) fn top_bar(layout: &ColorBarLayout, (left, right): (f64, f64), top: f64) -> Rect {
    let (x, width) = centred((left, right - left), layout.length);
    Rect {
        x,
        y: top + layout.label_room + layout.labels_height + COLORBAR_TICK_PADDING + COLORBAR_TICK,
        width,
        height: layout.thickness,
    }
}

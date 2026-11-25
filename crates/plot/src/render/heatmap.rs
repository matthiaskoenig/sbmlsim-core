//! Heatmaps and colour bars.
//!
//! The cell in row `i` and column `j` of a heatmap spans `j - 0.5..j + 0.5` on the x axis and
//! `i - 0.5..i + 0.5` on the y axis, the positions of the categories of a category axis. An
//! axis of the plot which is linear and has no categories of its own becomes the category axis
//! of the names of the first heatmap of the plot ([`plot_axes`]): its columns along x and its
//! rows along y, row 0 at the bottom unless the axis is reversed.
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
//! and labels of the values of the scale (or the given ones) and the label of the bar.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use charton::error::ChartonError;
use charton::prelude::*;

use super::axis::{self, ResolvedAxis};
use super::marks::{Layers, dataset};
use super::stats_marks::rings_layer;
use crate::colors::{color_of, color_scale, dark_text_on};
use crate::spec::{
    Axis, AxisScale, ColorBar, ColorScale, ElementKind, Fonts, Plot, Style, YAxisRef,
};
use crate::svg::{self, LayerInfo, escape, theme};
use crate::text::text_width;

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
        ElementKind::Heatmap { rows, columns, .. } if e.axis == YAxisRef::Y => {
            Some((rows, columns))
        }
        _ => None,
    });
    let Some((rows, columns)) = first else {
        return (Cow::Borrowed(&plot.x), Cow::Borrowed(&plot.y));
    };
    let categorize = |axis: &Axis, names: &[String]| -> Option<Axis> {
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
            let (r, c) = (i as f64, j as f64);
            let (x0, x1) = (xa.u(c - 0.5), xa.u(c + 0.5));
            let (y0, y1) = (ya.u(r - 0.5), ya.u(r + 0.5));
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
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    let cell_w = width / (x1 - x0).max(f64::MIN_POSITIVE);
    let cell_h = height / (y1 - y0).max(f64::MIN_POSITIVE);
    let widest = text
        .iter()
        .flatten()
        .map(|t| text_width(t, fonts.tick))
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

/// A colour bar laid out: its ticks and the room it takes right of the panel.
pub(crate) struct ColorBarLayout {
    /// The ticks as shares from the bottom of the bar with their labels.
    ticks: Vec<(f64, String)>,
    /// The width of the widest tick label in pixels.
    labels_width: f64,
    /// The pixels the bar takes right of the panel or of the bar before it.
    pub room: f64,
}

/// The bands of a categorical bar: one per integer from `low` to `high`.
fn bands(s: &ColorScale) -> f64 {
    s.high - s.low + 1.0
}

/// Lays out the colour bar of a plot whose panel is expected to be `length` pixels high.
///
/// The ticks are the given ones, else the ones of an axis from the low to the high end of the
/// scale, linear or logarithmic (the integers among them on a categorical bar); their labels
/// are the given ones, else their numbers. The room is measured with the Inter widths of the
/// labels.
pub(crate) fn colorbar_layout(bar: &ColorBar, fonts: &Fonts, length: f64) -> ColorBarLayout {
    let s = &bar.scale;
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
        tick_rotation: 0.0,
    };
    let resolved = || axis::resolve(&scale_axis, &[], &[], length, COLORBAR_TICK_ROOM);
    // the shares of the ticks from the bottom of the bar with their labels
    let n = bands(s);
    let ticks: Vec<(f64, String)> = match &bar.ticks {
        Some(values) => {
            let labels = match &bar.tick_labels {
                Some(l) => l.clone(),
                None => axis::tick_labels(values),
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
            r.tick_coordinates()
                .into_iter()
                .map(|(u, l)| (r.share(u), l))
                .collect()
        }
    };
    let labels_width = ticks
        .iter()
        .map(|(_, l)| text_width(l, fonts.tick))
        .fold(0.0, f64::max);
    let label = match bar.label.as_deref() {
        Some(l) if !l.is_empty() => COLORBAR_LABEL_GAP + fonts.label,
        _ => 0.0,
    };
    ColorBarLayout {
        room: COLORBAR_GAP
            + COLORBAR_WIDTH
            + COLORBAR_TICK
            + COLORBAR_TICK_PADDING
            + labels_width
            + label,
        ticks,
        labels_width,
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

/// Writes the gradient of a colour bar under `id`: the colours of the scale from its low end at
/// the bottom to its high end at the top.
fn gradient(defs: &mut String, id: &str, s: &ColorScale) {
    // the gradient from the bottom up, its stops on a grid of the bar and at the centre
    let mut stops: Vec<f64> = (0..=GRADIENT_STEPS)
        .map(|k| k as f64 / GRADIENT_STEPS as f64)
        .collect();
    if let Some(c) = s.center {
        let t = share_of_bar(s, c);
        if t > 0.0 && t < 1.0 {
            stops.push(t);
        }
    }
    stops.sort_by(f64::total_cmp);
    stops.dedup();
    let _ = write!(
        defs,
        "<linearGradient id=\"{id}\" x1=\"0\" y1=\"1\" x2=\"0\" y2=\"0\">"
    );
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

/// Draws the colour bar `index` of a plot right of `(x, y, width, height)`, its panel and the
/// bars before it: the gradient into `defs` under the id `<prefix>-colorbar` (the first bar)
/// or `<prefix>-colorbar-<index>`, the bar, its ticks, tick labels and label into `body`. A
/// categorical bar is a band of one colour per integer and needs no gradient.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_colorbar(
    body: &mut String,
    defs: &mut String,
    prefix: &str,
    index: usize,
    bar: &ColorBar,
    layout: &ColorBarLayout,
    fonts: &Fonts,
    (px, py, pw, ph): (f64, f64, f64, f64),
) {
    let s = &bar.scale;
    let x = px + pw + COLORBAR_GAP;
    body.push_str("<g class=\"sbmlsim-colorbar\">");
    if bar.categorical {
        // the bands from the bottom up, each from the edge of the one below it
        let n = bands(s) as usize;
        let edge = |k: usize| py + ph * (1.0 - k as f64 / n as f64);
        for k in 0..n {
            let (top, bottom) = (edge(k + 1), edge(k));
            body.push_str("<rect");
            attr(body, "x", x);
            attr(body, "y", top);
            attr(body, "width", COLORBAR_WIDTH);
            attr(body, "height", bottom - top);
            let _ = write!(body, " fill=\"{}\"/>", color_of(s, s.low + k as f64));
        }
    } else {
        let id = if index == 0 {
            format!("{prefix}-colorbar")
        } else {
            format!("{prefix}-colorbar-{index}")
        };
        gradient(defs, &id, s);
        body.push_str("<rect");
        attr(body, "x", x);
        attr(body, "y", py);
        attr(body, "width", COLORBAR_WIDTH);
        attr(body, "height", ph);
        let _ = write!(body, " fill=\"url(#{id})\"/>");
    }
    body.push_str("<rect");
    attr(body, "x", x);
    attr(body, "y", py);
    attr(body, "width", COLORBAR_WIDTH);
    attr(body, "height", ph);
    let _ = write!(
        body,
        " fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/>",
        theme::FG_SOFT
    );
    let right = x + COLORBAR_WIDTH;
    let _ = write!(
        body,
        "<g class=\"sbmlsim-colorbar-ticks\" stroke=\"{}\" stroke-width=\"1\">",
        theme::FG_SOFT
    );
    for (t, _) in &layout.ticks {
        let y = py + ph * (1.0 - t);
        body.push_str("<line");
        attr(body, "x1", right);
        attr(body, "y1", y);
        attr(body, "x2", right + COLORBAR_TICK);
        attr(body, "y2", y);
        body.push_str("/>");
    }
    body.push_str("</g>");
    let _ = write!(
        body,
        "<g class=\"sbmlsim-colorbar-tick-labels\" fill=\"{}\"",
        theme::FG_SOFT
    );
    attr(body, "font-size", fonts.tick);
    body.push('>');
    let lx = right + COLORBAR_TICK + COLORBAR_TICK_PADDING;
    for (t, label) in &layout.ticks {
        body.push_str("<text");
        attr(body, "x", lx);
        attr(body, "y", py + ph * (1.0 - t));
        body.push_str(" dominant-baseline=\"middle\">");
        escape(body, label);
        body.push_str("</text>");
    }
    body.push_str("</g>");
    if let Some(label) = bar.label.as_deref().filter(|l| !l.is_empty()) {
        let (tx, ty) = (
            lx + layout.labels_width + COLORBAR_LABEL_GAP + fonts.label / 2.0,
            py + ph / 2.0,
        );
        body.push_str("<g class=\"sbmlsim-colorbar-label\"><text");
        attr(body, "x", tx);
        attr(body, "y", ty);
        attr(body, "font-size", fonts.label);
        let _ = write!(
            body,
            " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" transform=\"rotate(-90 ",
            theme::FG
        );
        svg::num(body, tx);
        body.push(' ');
        svg::num(body, ty);
        body.push_str(")\">");
        escape(body, label);
        body.push_str("</text></g>");
    }
    body.push_str("</g>");
}

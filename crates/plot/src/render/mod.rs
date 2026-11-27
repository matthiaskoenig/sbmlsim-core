//! The renderer: a figure of the spec in, one SVG string out.
//!
//! A plot is one Charton chart (decision (a) of the spike): the crate resolves the axes
//! ([`axis`]), prepares the layers of the elements ([`marks`]) and lets Charton lay out the
//! panel and draw the axes and the marks through the writer of [`crate::svg`], which groups the
//! layers of every element. A plot is prepared first ([`prepare`]: its axes resolved, the
//! depth of everything around its panel measured), then the figure lays its plots out
//! ([`crate::layout`]) and every plot is drawn with its panel where the layout puts it
//! ([`render_plot`]); the elements of a right y axis are drawn by an overlay chart over the
//! same panel and the axis by the crate ([`right_axis`]), the legends by the crate ([`legend`]).

pub(crate) mod annot;
pub(crate) mod axis;
pub(crate) mod contour;
pub(crate) mod cut;
pub(crate) mod heatmap;
pub(crate) mod legend;
pub(crate) mod marks;
pub(crate) mod rich;
pub(crate) mod right_axis;
pub(crate) mod stats_marks;
pub(crate) mod top_axis;

use std::fmt::Write as _;

use charton::prelude::*;

use crate::layout::{self, LegendOwner, PAD, Rect};
use crate::spec::{AxisScale, ColorBarPlacement, Element, Figure, Fonts, Plot, YAxisRef};
use crate::svg::{
    self, AxisHooks, ElementTag, FONT_FAMILY, GridLines, ID_PLACEHOLDER, MULTIPLIER_GAP,
    PanelProbe, PlotHooks, PlotWriter, Stack, escape, text_lines, theme,
};
use crate::text::{extra_lines, label_extent, label_height, label_width, widest_line};
use axis::{ResolvedAxis, TickRoom, axis_title};
use heatmap::ColorBarLayout;
use right_axis::RightAxis;

/// The anchors of the colour ramp of sbmlsim-web (`RAMP` of `plot.ts`): viridis over its first
/// 85 %, from which the colours of the series are read.
const RAMP: [[f64; 3]; 6] = [
    [68.0, 1.0, 84.0],
    [72.0, 40.0, 120.0],
    [62.0, 73.0, 137.0],
    [49.0, 103.0, 142.0],
    [38.0, 130.0, 142.0],
    [30.0, 156.0, 137.0],
];

/// The colour of the series at `index` of `count`, `seriesColour` of sbmlsim-web.
///
/// The share `index / (count - 1)` is read between the two anchors of the ramp around it, mixed
/// in sRGB as the `color-mix` of the web app with its percentage rounded to two decimals, and
/// written as `#rrggbb`.
pub(crate) fn series_color(index: usize, count: usize) -> String {
    let share = if count > 1 {
        index as f64 / (count - 1) as f64
    } else {
        0.0
    };
    let at = share.clamp(0.0, 1.0) * (RAMP.len() - 1) as f64;
    let first = (at.floor() as usize).min(RAMP.len() - 2);
    let part = at - first as f64;
    let rgb = if part <= 0.0 {
        RAMP[first]
    } else if part >= 1.0 {
        RAMP[first + 1]
    } else {
        let keep = ((1.0 - part) * 10000.0).round() / 10000.0;
        let (a, b) = (RAMP[first], RAMP[first + 1]);
        [0, 1, 2].map(|k| a[k] * keep + b[k] * (1.0 - keep))
    };
    let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    )
}

/// The colour of every element of a plot: its own, else the next of the cycle by its position
/// among the cycled elements without a colour, else the text colour.
fn element_colors(elements: &[Element]) -> Vec<String> {
    let count = elements
        .iter()
        .filter(|e| e.style.color.is_none() && marks::cycled(e))
        .count();
    let mut next = 0;
    elements
        .iter()
        .map(|e| match &e.style.color {
            Some(c) => c.clone(),
            None if marks::cycled(e) => {
                next += 1;
                series_color(next - 1, count)
            }
            None => marks::UNCYCLED.to_string(),
        })
        .collect()
}

/// One plot drawn into its canvas.
pub(crate) struct PlotSvg {
    /// The drawing, in the coordinates of its canvas.
    pub body: String,
    /// Its definitions (clip paths, gradients).
    pub defs: String,
}

/// The length of a tick mark in pixels.
pub(crate) const TICK_LENGTH: f64 = 5.0;
/// The gap between a tick mark and its label in pixels.
pub(crate) const TICK_LABEL_PADDING: f64 = 3.0;
/// The gap between the tick labels and the title of an axis in pixels (Charton's default).
pub(crate) const TITLE_GAP: f64 = 10.0;

/// The shares of the ticks of an axis along it, for its grid lines: of the major ticks of a
/// log axis (since 0.33.0), as matplotlib's grid.
fn grid_shares(axis: &ResolvedAxis) -> Vec<f64> {
    if let Some(marks) = axis.mark_shares() {
        return marks
            .into_iter()
            .filter(|(_, major)| *major)
            .map(|(t, _)| t)
            .collect();
    }
    axis.tick_coordinates()
        .iter()
        .map(|(u, _)| axis.share(*u))
        .collect()
}

/// The ticks of an axis as coordinates with their labels.
type TickLabels = Vec<(f64, String)>;

/// The labels of the ticks of an axis as Charton gets them: empty when they are hidden.
///
/// An axis without tick labels and tick marks has no ticks for Charton (since 0.33.0), so it
/// reserves no room for them: the thin rows of a grid of such axes keep their panels.
fn tick_labels(a: &ResolvedAxis, axis: &crate::spec::Axis) -> TickLabels {
    if !axis.ticks_visible && !axis.marks_visible() {
        return Vec::new();
    }
    let visible = axis.ticks_visible;
    a.tick_coordinates()
        .into_iter()
        .map(|(u, l)| (u, if visible { l } else { String::new() }))
        .collect()
}

/// The room a multiplier of the tick labels of a y axis takes above the panel: its line and
/// [`MULTIPLIER_GAP`].
pub(crate) fn multiplier_room(fonts: &Fonts) -> f64 {
    fonts.y_tick() + MULTIPLIER_GAP
}

/// The labels Charton is given for tick labels: the widest line of each without its
/// superscripts, which it measures and places, with the label the writer writes.
fn given_labels(labels: &[(f64, String)], size: f64, fonts: &Fonts) -> Vec<(String, String)> {
    labels
        .iter()
        .map(|(_, l)| (widest_line(l, size, fonts.tick_weight), l.clone()))
        .collect()
}

/// How one axis is laid out: its hooks for the writer and its depth.
#[derive(Debug, Clone)]
struct AxisLayout {
    hooks: AxisHooks,
    /// The pixels from the spine to the far side of the title (or of the tick labels without a
    /// title), measured in Inter.
    depth: f64,
}

/// Lays out an axis over its tick labels: the bottom axis when `bottom`, else the left one.
///
/// The crate measures the rotated labels with the Inter table of the tick weight; Charton gets
/// them unrotated and no title, and the writer rotates the labels and draws the title at the
/// Inter depth, the lines of a title of several lines stacked outwards.
fn axis_layout(
    axis: &crate::spec::Axis,
    resolved: &ResolvedAxis,
    labels: &[(f64, String)],
    fonts: &Fonts,
    bottom: bool,
) -> AxisLayout {
    let multiplier = resolved.multiplier.as_deref();
    // the colours of the labels, by the index of their ticks
    let label_colors = match &axis.tick_label_color {
        Some(colors) => resolved
            .tick_coordinates_indexed()
            .into_iter()
            .map(|(k, _, _)| {
                // a category is its position, which a cut axis does not start at
                let index = match resolved.transform {
                    axis::Transform::Category => resolved.ticks[k].0 as usize,
                    _ => k,
                };
                colors.of(index).map(str::to_string)
            })
            .collect(),
        None => Vec::new(),
    };
    let angle = axis.tick_rotation;
    let tick = if bottom { fonts.tick } else { fonts.y_tick() };
    let inter = labels
        .iter()
        .map(|(_, l)| {
            label_extent(
                label_width(l, tick, fonts.tick_weight),
                label_height(l, tick),
                angle,
                bottom,
            )
        })
        .fold(0.0, f64::max);
    let name = axis_title(axis);
    let title_room = if axis.label_visible && !name.is_empty() {
        TITLE_GAP + fonts.label + extra_lines(&name, fonts.label)
    } else {
        0.0
    };
    let title_text = (axis.label_visible && !name.is_empty()).then_some(name);
    // hidden tick labels take no room, hidden tick marks neither (since 0.33.0)
    let labels_depth = match (axis.marks_visible(), axis.ticks_visible) {
        (_, true) => TICK_LENGTH + TICK_LABEL_PADDING + inter,
        (true, false) => TICK_LENGTH,
        (false, false) => 0.0,
    };
    // the multiplier of the bottom axis goes below its tick labels at its right end, beside
    // the title (that of a left axis above the panel, see `prepare`)
    let multiplier = multiplier
        .filter(|_| axis.ticks_visible)
        .map(str::to_string);
    let title_room = match &multiplier {
        Some(_) if bottom => title_room.max(TITLE_GAP + tick),
        _ => title_room,
    };
    AxisLayout {
        hooks: AxisHooks {
            labels_hidden: !axis.ticks_visible,
            marks_hidden: !axis.marks_visible(),
            line_hidden: !axis.line_visible,
            tick_size: tick,
            tick_weight: fonts.tick_weight,
            label_angle: -angle,
            title: title_text,
            labels_depth,
            labels: given_labels(labels, tick, fonts),
            multiplier,
            label_colors,
            marks: resolved.mark_shares(),
        },
        depth: labels_depth + title_room,
    }
}

/// Pixels on the four sides of a panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Sides {
    /// Above.
    pub top: f64,
    /// Right.
    pub right: f64,
    /// Below.
    pub bottom: f64,
    /// Left.
    pub left: f64,
}

/// A plot before it is placed: its axes resolved over its data, and the room everything
/// around its panel takes.
pub(crate) struct Prepared {
    /// The panel size (width, height) the plot was prepared for.
    pub panel: (f64, f64),
    /// The x axis.
    pub xa: ResolvedAxis,
    /// The left y axis.
    pub ya: ResolvedAxis,
    /// The right y axis.
    pub y2a: Option<ResolvedAxis>,
    x_labels: TickLabels,
    y_labels: TickLabels,
    xl: AxisLayout,
    yl: AxisLayout,
    right: Option<RightAxis>,
    /// The top axis (since 0.33.0).
    top: Option<top_axis::TopAxis>,
    /// The colour bars laid out.
    pub bars: Vec<ColorBarLayout>,
    /// The room of the bar of the plot above its row, 0 without one (since 0.31.0).
    pub top_bar: f64,
    grid: GridLines,
    /// The colour of every element.
    pub colors: Vec<String>,
    /// Charton's bands around the panel of the chart with margins of 0: what it reserves for
    /// its axes and the plot title.
    bands: Sides,
    /// The room around the panel: the axes measured in Inter (at least Charton's bands), the
    /// plot title, the right axis, the colour bar and the last tick label of the x axis.
    pub depth: Sides,
}

/// Prepares a plot whose panel is expected to be `panel` (width, height) pixels: resolves its
/// axes with a tick count for that size and measures its depths.
pub(crate) fn prepare(figure: &Figure, plot: &Plot, panel: (f64, f64)) -> Prepared {
    let fonts = &figure.fonts;
    let (panel_width, panel_height) = panel;
    // the axes, a linear one as the categories of a heatmap
    let (x_axis, y_axis) = heatmap::plot_axes(plot);
    let (x_axis, y_axis) = (x_axis.as_ref(), y_axis.as_ref());
    // the values of the series beyond the ends the spec gives the x axis are not drawn, so
    // they are no values of the axes either
    let ends = cut::XEnds::of(x_axis);
    let log = |a: Option<&crate::spec::Axis>| a.is_some_and(|a| a.scale == AxisScale::Log);
    let (mut ext, mut ext2) = (marks::Extent::default(), marks::Extent::default());
    for e in &plot.elements {
        match e.axis {
            YAxisRef::Y => marks::extent(e, &mut ext, ends.map(|x| (x, log(Some(y_axis))))),
            YAxisRef::Y2 => marks::extent(e, &mut ext2, ends.map(|x| (x, log(plot.y2.as_ref())))),
        }
    }
    ext.xs.extend_from_slice(&ext2.xs);
    ext.x_sticky.extend_from_slice(&ext2.x_sticky);
    let bars: Vec<ColorBarLayout> = plot
        .color_bars()
        .iter()
        .map(|b| {
            let along = match b.placement {
                ColorBarPlacement::Right | ColorBarPlacement::Panel => panel_height,
                ColorBarPlacement::Inset | ColorBarPlacement::Strip => panel_width,
                ColorBarPlacement::Top => figure.width,
            };
            heatmap::colorbar_layout(b, fonts, along)
        })
        .collect();
    let (x_room, y_room) = (TickRoom::x(fonts.tick), TickRoom::y(fonts.y_tick()));
    let xa = axis::resolve(x_axis, &ext.xs, &ext.x_sticky, panel_width, x_room);
    let ya = axis::resolve(y_axis, &ext.ys, &ext.y_sticky, panel_height, y_room);
    // an aspect widens the limits of one axis for this panel
    let (xa, ya) = match plot.aspect {
        Some(aspect) => axis::with_aspect(
            (x_axis, y_axis),
            (xa, ya),
            aspect.ratio(),
            panel,
            (x_room, y_room),
        ),
        None => (xa, ya),
    };
    let y2a = plot
        .y2
        .as_ref()
        .map(|a| axis::resolve(a, &ext2.ys, &ext2.y_sticky, panel_height, y_room));
    let right = plot
        .y2
        .as_ref()
        .zip(y2a.as_ref())
        .map(|(a, r)| right_axis::layout(a, r, fonts));
    let top = plot
        .x2
        .as_ref()
        .map(|a| top_axis::layout(a, &xa, panel_width, fonts));
    let x_labels = tick_labels(&xa, x_axis);
    let y_labels = tick_labels(&ya, y_axis);
    let mut xl = axis_layout(x_axis, &xa, &x_labels, fonts, true);
    let mut yl = axis_layout(y_axis, &ya, &y_labels, fonts, false);
    // a frame replaces the lines of the axes (since 0.33.0)
    if plot.frame.is_some() {
        xl.hooks.line_hidden = true;
        yl.hooks.line_hidden = true;
    }
    // the last tick label of an unrotated x axis reaches half its width past the panel
    let overhang = match x_labels.last() {
        Some((_, l)) if x_axis.tick_rotation == 0.0 => {
            label_width(l, fonts.tick, fonts.tick_weight) / 2.0
        }
        _ => 0.0,
    };
    let grid = GridLines {
        x: if x_axis.grid {
            grid_shares(&xa)
        } else {
            Vec::new()
        },
        y: if y_axis.grid {
            grid_shares(&ya)
        } else {
            Vec::new()
        },
    };
    // Charton's bands: a chart of the scaffold alone with margins of 0 in a canvas larger than
    // the panel, through a backend which only records the panel
    let (w, h) = (panel_width.max(1.0) + 400.0, panel_height.max(1.0) + 400.0);
    let bands = match marks::build(&[], &[], fonts, (&xa, &ya), panel, YAxisRef::Y) {
        Ok(layers) => {
            let mut chart = frame(
                layers.chart,
                figure,
                plot,
                (&xa, &ya),
                (x_labels.clone(), y_labels.clone()),
                (w.ceil(), h.ceil()),
                Sides::default(),
            );
            let mut probe = PanelProbe::default();
            match (chart.render(&mut probe), probe.panel) {
                (Ok(()), Some(p)) => Sides {
                    top: p.y,
                    right: w.ceil() - p.x - p.width,
                    bottom: h.ceil() - p.y - p.height,
                    left: p.x,
                },
                _ => Sides::default(),
            }
        }
        Err(_) => Sides::default(),
    };
    let right_room = right.as_ref().map_or(0.0, |r| r.depth)
        + bars
            .iter()
            .filter(|b| b.placement == ColorBarPlacement::Right)
            .map(|b| b.room)
            .sum::<f64>();
    // a bar above the row takes room above the room of the plots of the row
    let top_bar = bars
        .iter()
        .filter(|b| b.placement == ColorBarPlacement::Top)
        .map(|b| b.room)
        .fold(0.0, f64::max);
    // a plot title of several lines stacks its lines up from the line Charton places, into
    // room above the band Charton reserves for one line
    let title_lines = plot
        .title
        .as_deref()
        .map_or(0.0, |t| extra_lines(t, fonts.plot_title_size()));
    // the multiplier of a y axis goes above the panel
    let multiplied =
        yl.hooks.multiplier.is_some() || right.as_ref().is_some_and(|r| r.multiplier.is_some());
    let multiplier_top = if multiplied {
        multiplier_room(fonts)
    } else {
        0.0
    };
    let depth = Sides {
        top: bands.top + title_lines + multiplier_top + top.as_ref().map_or(0.0, |t| t.depth),
        right: bands.right.max(overhang).max(right_room),
        bottom: bands.bottom.max(xl.depth),
        left: bands.left.max(yl.depth),
    };
    // a plot of a bar in its panel draws nothing else, the room of the bar right of it
    // (since 0.33.0)
    let depth = match bars
        .iter()
        .find(|b| b.placement == ColorBarPlacement::Panel)
    {
        Some(b) => Sides {
            right: b.room,
            ..Sides::default()
        },
        None => depth,
    };
    Prepared {
        panel,
        colors: element_colors(&plot.elements),
        xa,
        ya,
        y2a,
        x_labels,
        y_labels,
        xl,
        yl,
        right,
        top,
        bars,
        top_bar,
        grid,
        bands,
        depth,
    }
}

/// The tags of the element groups of a plot: a chart writes the groups of the elements of its
/// y axis `here`.
fn element_tags(plot: &Plot, here: YAxisRef) -> Vec<ElementTag> {
    plot.elements
        .iter()
        .map(|e| ElementTag {
            id: e.id.clone(),
            kind: kind_name(e),
            legend: e.legend_group().map(str::to_string),
            here: e.axis == here,
        })
        .collect()
}

/// A Charton chart whose integer canvas starts at `canvas` and the margins which put its panel
/// at `panel` (both in pixels of the figure), given the bands Charton reserves around it.
fn margins_for(canvas: Rect, panel: Rect, bands: Sides) -> ((f64, f64), Sides) {
    let size = (canvas.width.ceil().max(1.0), canvas.height.ceil().max(1.0));
    let (x, y) = (panel.x - canvas.x, panel.y - canvas.y);
    let margins = Sides {
        top: (y - bands.top).max(0.0),
        right: (size.0 - x - panel.width - bands.right).max(0.0),
        bottom: (size.1 - y - panel.height - bands.bottom).max(0.0),
        left: (x - bands.left).max(0.0),
    };
    (size, margins)
}

/// Draws one plot into its canvas, the rectangle of the figure its nested `<svg>` covers, with
/// its panel at `panel` (both in pixels of the figure); the ids it writes start with `prefix`.
pub(crate) fn render_plot(
    figure: &Figure,
    plot: &Plot,
    prep: &Prepared,
    canvas: Rect,
    panel: Rect,
    prefix: String,
    spans: &[Option<(f64, f64)>],
) -> PlotSvg {
    let fonts = &figure.fonts;
    // a plot of a bar in its panel draws the bar alone (since 0.33.0)
    if let Some(k) = prep
        .bars
        .iter()
        .position(|b| b.placement == ColorBarPlacement::Panel)
    {
        let local = Rect {
            x: panel.x - canvas.x,
            y: panel.y - canvas.y,
            ..panel
        };
        let bar = &plot.color_bars()[k];
        let layout = prep.bars[k].with_thickness(bar.thickness.unwrap_or(local.width));
        let (mut body, mut defs) = (String::new(), String::new());
        heatmap::draw_colorbar(&mut body, &mut defs, &prefix, k, bar, &layout, fonts, local);
        return PlotSvg { body, defs };
    }
    let (size, margins) = margins_for(canvas, panel, prep.bands);
    let local = Rect {
        x: panel.x - canvas.x,
        y: panel.y - canvas.y,
        ..panel
    };
    let hooks = PlotHooks {
        layers: Vec::new(),
        elements: element_tags(plot, YAxisRef::Y),
        x: prep.xl.hooks.clone(),
        y: prep.yl.hooks.clone(),
        title_size: fonts.label,
        title_weight: fonts.label_weight,
        plot_title_weight: fonts.plot_title_weight(),
        title_gap: TITLE_GAP,
        grid: prep.grid.clone(),
        inset: plot.inset.is_some(),
        own_title: prep.top.is_some(),
    };
    let built = marks::build(
        &plot.elements,
        &prep.colors,
        fonts,
        (&prep.xa, &prep.ya),
        (panel.width, panel.height),
        YAxisRef::Y,
    )
    .map(|layers| {
        let chart = frame(
            layers.chart,
            figure,
            plot,
            (&prep.xa, &prep.ya),
            (prep.x_labels.clone(), prep.y_labels.clone()),
            size,
            margins,
        );
        (chart, layers.info)
    });
    let (chart, layers, failed) = match built {
        Ok((chart, layers)) => (Some(chart), layers, None),
        Err(e) => (None, Vec::new(), Some(e)),
    };
    let hooks = PlotHooks { layers, ..hooks };
    let mut writer = PlotWriter::new(prefix.clone(), &hooks);
    let failed = match chart {
        Some(mut chart) => chart.render(&mut writer).err(),
        None => failed,
    };
    writer.finish();
    // the panel as Charton drew it, which the overlay of the right axis covers
    let drawn = writer.panel.map_or(local, |r| Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    });
    let mut body = writer.body;
    let mut defs = writer.defs;
    let mut bar_from = drawn.width;
    if let (Some(y2), Some(y2a), Some(right)) = (&plot.y2, &prep.y2a, &prep.right) {
        overlay(
            figure,
            plot,
            prep,
            (y2, y2a),
            size,
            drawn,
            &prefix,
            (&mut body, &mut defs),
        );
        right_axis::draw(&mut body, y2, y2a, right, fonts, drawn);
        bar_from += right.depth;
    }
    // the colour bars stacked right of the panel and the right axis in their order, and the
    // insets in the panel; a bar above the row is drawn by the figure
    for (k, (b, layout)) in plot.color_bars().iter().zip(&prep.bars).enumerate() {
        let at = match layout.placement {
            // a bar over several rows spans their panels (since 0.33.0)
            ColorBarPlacement::Right => {
                let (y, height) = match spans.get(k).copied().flatten() {
                    Some((top, h)) => (top - canvas.y, h),
                    None => (drawn.y, drawn.height),
                };
                Rect {
                    x: drawn.x + bar_from,
                    y,
                    width: 0.0,
                    height,
                }
            }
            ColorBarPlacement::Inset => {
                heatmap::inset_bar(layout, layout.inset_rect(b, drawn, fonts), fonts)
            }
            ColorBarPlacement::Strip => layout.strip_rect(b, drawn),
            ColorBarPlacement::Top | ColorBarPlacement::Panel => continue,
        };
        heatmap::draw_colorbar(&mut body, &mut defs, &prefix, k, b, layout, fonts, at);
        if layout.placement == ColorBarPlacement::Right {
            bar_from += layout.room;
        }
    }
    // the top axis and the title of the plot above it (since 0.33.0)
    if let (Some(x2), Some(top)) = (&plot.x2, &prep.top) {
        top_axis::draw(&mut body, x2, top, fonts, drawn);
        if let Some(title) = plot.title.as_deref().filter(|t| !t.is_empty()) {
            let (tx, ty) = (drawn.x + drawn.width / 2.0, drawn.y - top.depth - TITLE_GAP);
            body.push_str("<g class=\"charton-title\"><text");
            svg::attr_num(&mut body, "x", tx);
            svg::attr_num(&mut body, "y", ty);
            svg::attr_num(&mut body, "font-size", fonts.plot_title_size());
            svg::attr_weight(&mut body, fonts.plot_title_weight());
            let _ = write!(
                body,
                " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"text-after-edge\">",
                theme::FG
            );
            text_lines(
                &mut body,
                title,
                (tx, ty),
                fonts.plot_title_size(),
                Stack::Up,
                "text-after-edge",
            );
            body.push_str("</text></g>");
        }
    }
    // the frame of the panel over the marks (since 0.33.0), as matplotlib's spines
    if let Some(fr) = &plot.frame
        && fr.width > 0.0
    {
        body.push_str("<rect class=\"sbmlsim-frame\"");
        for (name, v) in [
            ("x", drawn.x),
            ("y", drawn.y),
            ("width", drawn.width),
            ("height", drawn.height),
        ] {
            svg::attr_num(&mut body, name, v);
        }
        body.push_str(" fill=\"none\"");
        svg::attr_css(
            &mut body,
            "stroke",
            fr.color.as_deref().unwrap_or(theme::FG_SOFT),
        );
        svg::attr_num(&mut body, "stroke-width", fr.width);
        body.push_str("/>");
    }
    if let Some(e) = failed {
        error_text(&mut body, canvas, &e.to_string());
    }
    PlotSvg { body, defs }
}

/// Draws the elements of the right y axis of a plot: a chart of the same canvas whose panel is
/// `panel`, the panel of the primary chart, without axes, grid and background (decision (b) of
/// the spike). Its writer writes the groups of these elements, the primary one leaves them out.
#[allow(clippy::too_many_arguments)]
fn overlay(
    figure: &Figure,
    plot: &Plot,
    prep: &Prepared,
    (y2, y2a): (&crate::spec::Axis, &ResolvedAxis),
    size: (f64, f64),
    panel: Rect,
    prefix: &str,
    (body, defs): (&mut String, &mut String),
) {
    let fonts = &figure.fonts;
    let hooks = PlotHooks {
        layers: Vec::new(),
        elements: element_tags(plot, YAxisRef::Y2),
        grid: GridLines {
            x: Vec::new(),
            y: if y2.grid {
                grid_shares(y2a)
            } else {
                Vec::new()
            },
        },
        ..PlotHooks::default()
    };
    let zero = Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    };
    let (x0, x1) = prep.xa.domain();
    let (y0, y1) = y2a.domain();
    let (w, h) = size;
    let built = marks::build(
        &plot.elements,
        &prep.colors,
        fonts,
        (&prep.xa, y2a),
        (panel.width, panel.height),
        YAxisRef::Y2,
    )
    .map(|layers| {
        let chart = layers
            .chart
            .with_size(w as u32, h as u32)
            .with_margins(
                panel.y / h,
                (w - panel.x - panel.width) / w,
                (h - panel.y - panel.height) / h,
                panel.x / w,
            )
            .with_x_domain(x0, x1)
            .with_x_expand(zero)
            .with_y_domain(y0, y1)
            .with_y_expand(zero)
            .configure_theme(|t| {
                t.with_show_axes(false)
                    .with_min_panel_size(1.0)
                    .with_grid(false)
                    .with_show_legend(false)
                    .with_background_color("none")
            });
        (chart, layers.info)
    });
    let (chart, layers) = match built {
        Ok((chart, layers)) => (Some(chart), layers),
        Err(_) => (None, Vec::new()),
    };
    let hooks = PlotHooks { layers, ..hooks };
    let mut writer = PlotWriter::new(format!("{prefix}-o"), &hooks);
    if let Some(mut chart) = chart {
        let _ = chart.render(&mut writer);
    }
    writer.finish();
    body.push_str(&writer.body);
    defs.push_str(&writer.defs);
}

/// Writes the message of a plot Charton could not draw.
fn error_text(body: &mut String, canvas: Rect, message: &str) {
    let _ = write!(
        body,
        "<text class=\"sbmlsim-error\" x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\">",
        PAD,
        canvas.height / 2.0,
        theme::FG
    );
    escape(body, message);
    body.push_str("</text>");
}

/// The name of the kind of an element, its class `sbmlsim-<kind>`.
fn kind_name(e: &Element) -> &'static str {
    use crate::spec::ElementKind::*;
    match e.kind {
        Curve { .. } => "curve",
        Band { .. } => "band",
        Area { .. } => "area",
        Bar { .. } => "bar",
        Box { .. } => "box",
        Violin { .. } => "violin",
        Histogram { .. } => "histogram",
        Heatmap { .. } => "heatmap",
        Text { .. } => "text",
        Rule { .. } => "rule",
        Polygon { .. } => "polygon",
        Arrow { .. } => "arrow",
        Contour { .. } => "contour",
    }
}

/// The Charton chart of the layers of a plot, of the canvas `size` with the margins `m`.
///
/// Charton gets the tick labels unrotated and no axis titles (see `axis_layout`); the writer
/// rotates the labels and draws the titles.
fn frame(
    chart: LayeredChart,
    figure: &Figure,
    plot: &Plot,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
    (x_labels, y_labels): (TickLabels, TickLabels),
    (w, h): (f64, f64),
    m: Sides,
) -> LayeredChart {
    let fonts = &figure.fonts;
    let zero = Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    };
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    let (fg, soft) = (theme::FG, theme::FG_SOFT);
    let (tick, label, plot_title) = (fonts.tick, fonts.label, fonts.plot_title_size());
    let mut chart = chart
        .with_size(w as u32, h as u32)
        .with_margins(m.top / h, m.right / w, m.bottom / h, m.left / w)
        .with_x_domain(x0, x1)
        .with_x_expand(zero)
        .with_y_domain(y0, y1)
        .with_y_expand(zero)
        .with_x_ticks(proxies(x_labels, fonts.tick, fonts))
        .with_y_ticks(proxies(y_labels, fonts.y_tick(), fonts))
        .with_x_label("")
        .with_y_label("")
        .configure_theme(move |t| {
            // the layout gives every panel its size, which Charton keeps however small
            // (its least panel of 100 pixels overlapped the plots of thin rows before 0.33.0)
            t.with_show_legend(false)
                .with_min_panel_size(1.0)
                .with_grid(false)
                .with_background_color(theme::BACKGROUND)
                .with_label_size(label)
                .with_label_color(fg)
                .with_tick_label_size(tick)
                .with_tick_label_color(soft)
                .with_axes_color(soft)
                .with_tick_color(soft)
                .with_axis_width(1.0)
                .with_tick_width(1.0)
                .with_tick_length(TICK_LENGTH)
                .with_tick_label_padding(TICK_LABEL_PADDING)
                .with_title_size(plot_title)
                .with_title_color(fg)
                .with_x_tick_label_angle(0.0)
                .with_y_tick_label_angle(0.0)
        });
    if let Some(t) = &plot.title {
        chart = chart.with_title(t.clone());
    }
    chart
}

/// The tick labels as Charton is given them: the widest line of each without its superscripts.
fn proxies(labels: TickLabels, size: f64, fonts: &Fonts) -> TickLabels {
    labels
        .into_iter()
        .map(|(u, l)| (u, widest_line(&l, size, fonts.tick_weight)))
        .collect()
}

/// FNV-1a of a text, for the id prefix of a figure.
fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Renders a figure as SVG.
///
/// Every element of every plot is one group `<g data-element="<id>">`, with `data-legend` when
/// the element is in a legend group. The colours and font sizes are the light theme as
/// presentation attributes, with Charton's semantic classes on the axes and marks, so a host
/// page restyles the figure with a stylesheet. The same figure gives the same bytes natively
/// and in wasm.
pub fn render_svg(figure: &Figure) -> String {
    render_planned(figure, &layout::plan(figure))
}

/// Renders a figure as SVG on its plan, [`layout::plan`] of the same figure: a caller which also
/// needs the geometry of the figure plans it once and passes the plan to both.
pub(crate) fn render_planned(figure: &Figure, plan: &layout::Plan) -> String {
    let fonts = &figure.fonts;
    let lay = &plan.layout;
    let mut defs = String::new();
    let mut plots = String::new();
    for (i, plot) in figure.plots.iter().enumerate() {
        let canvas = plan.canvases[i];
        // the bars of the plot over several rows: from the top of its panel to the bottom of
        // the panels of the last of them
        let spans: Vec<Option<(f64, f64)>> = plot
            .color_bars()
            .iter()
            .map(|b| {
                let rows = b.rows.filter(|r| *r > 1)?;
                let top = lay.plots[i].rect.y;
                let bottom = figure
                    .plots
                    .iter()
                    .zip(&lay.plots)
                    .filter(|(q, _)| {
                        q.inset.is_none()
                            && q.row + q.row_span > plot.row
                            && q.row + q.row_span - 1 < plot.row + rows
                    })
                    .map(|(_, r)| r.rect.bottom())
                    .fold(lay.plots[i].rect.bottom(), f64::max);
                Some((top, bottom - top))
            })
            .collect();
        let p = render_plot(
            figure,
            plot,
            &plan.prepared[i],
            canvas,
            lay.plots[i].rect,
            format!("{ID_PLACEHOLDER}-p{i}"),
            &spans,
        );
        defs.push_str(&p.defs);
        plots.push_str("<svg class=\"sbmlsim-plot\" data-plot=\"");
        escape(&mut plots, &plot.id);
        plots.push('"');
        for (name, v) in [
            ("x", canvas.x),
            ("y", canvas.y),
            ("width", canvas.width.ceil()),
            ("height", canvas.height.ceil()),
        ] {
            svg::attr_num(&mut plots, name, v);
        }
        plots.push_str(" overflow=\"visible\">");
        plots.push_str(&p.body);
        plots.push_str("</svg>");
    }
    // the colour bars above the rows (since 0.31.0)
    for (i, k, at) in &plan.top_bars {
        let plot = &figure.plots[*i];
        heatmap::draw_colorbar(
            &mut plots,
            &mut defs,
            &format!("{ID_PLACEHOLDER}-p{i}"),
            *k,
            &plot.color_bars()[*k],
            &plan.prepared[*i].bars[*k],
            fonts,
            *at,
        );
    }
    // the titles of the rows (since 0.33.0)
    for (band, title) in &plan.row_titles {
        let size = fonts.plot_title_size();
        let at = (band.x + band.width / 2.0, band.y + band.height / 2.0);
        plots.push_str("<g class=\"sbmlsim-row-title\"><text");
        svg::attr_num(&mut plots, "x", at.0);
        svg::attr_num(&mut plots, "y", at.1);
        svg::attr_num(&mut plots, "font-size", size);
        svg::attr_weight(&mut plots, fonts.plot_title_weight());
        let _ = write!(
            plots,
            " text-anchor=\"middle\" dominant-baseline=\"middle\" fill=\"{}\">",
            theme::FG
        );
        text_lines(&mut plots, title, at, size, Stack::Centred, "middle");
        plots.push_str("</text></g>");
    }
    let mut legends = String::new();
    for ((b, entries), shape) in lay.legends.iter().zip(&plan.entries).zip(&plan.shapes) {
        let row;
        let owner = match &b.owner {
            LegendOwner::Plot(id) => ("data-plot", id.as_str()),
            LegendOwner::Row(r) => {
                row = r.to_string();
                ("data-row", row.as_str())
            }
            LegendOwner::Figure => ("data-figure", "legend"),
        };
        legend::draw(&mut legends, owner, b.rect, entries, shape, fonts);
    }

    let mut out = String::with_capacity(plots.len() + defs.len() + legends.len() + 1024);
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\"");
    svg_size(&mut out, lay.width, lay.height);
    out.push_str(" class=\"sbmlsim-figure\" role=\"img\" font-family=\"");
    escape(&mut out, FONT_FAMILY);
    out.push_str("\">");
    let name = figure
        .title
        .as_deref()
        .or_else(|| figure.plots.first().and_then(|p| p.title.as_deref()));
    if let Some(name) = name {
        out.push_str("<title>");
        escape(&mut out, name);
        out.push_str("</title>");
    }
    if !defs.is_empty() {
        out.push_str("<defs>");
        out.push_str(&defs);
        out.push_str("</defs>");
    }
    out.push_str("<g class=\"charton-background\"><rect x=\"0\" y=\"0\"");
    svg_rect_size(&mut out, lay.width, lay.height);
    let _ = write!(out, " fill=\"{}\"/></g>", theme::BACKGROUND);
    if let (Some(title), Some(band)) = (&figure.title, lay.title) {
        out.push_str("<g class=\"sbmlsim-figure-title\"><text");
        svg::attr_num(&mut out, "x", band.x + band.width / 2.0);
        svg::attr_num(&mut out, "y", band.y + band.height / 2.0);
        svg::attr_num(&mut out, "font-size", fonts.title);
        svg::attr_weight(&mut out, fonts.title_weight);
        let _ = write!(
            out,
            " text-anchor=\"middle\" dominant-baseline=\"middle\" fill=\"{}\">",
            theme::FG
        );
        let at = (band.x + band.width / 2.0, band.y + band.height / 2.0);
        text_lines(&mut out, title, at, fonts.title, Stack::Centred, "middle");
        out.push_str("</text></g>");
    }
    out.push_str(&plots);
    out.push_str(&legends);
    out.push_str("</svg>");

    // the ids of the figure: a hash of the drawing, so two figures of a page differ
    let prefix = format!("sbmlsim-{:012x}", fnv1a(&out) & 0xffff_ffff_ffff);
    out.replace(ID_PLACEHOLDER, &prefix)
}

/// Writes ` width`, ` height` and ` viewBox` of the figure.
fn svg_size(out: &mut String, w: f64, h: f64) {
    svg_rect_size(out, w, h);
    out.push_str(" viewBox=\"0 0 ");
    svg::num(out, w);
    out.push(' ');
    svg::num(out, h);
    out.push('"');
}

/// Writes ` width` and ` height`.
fn svg_rect_size(out: &mut String, w: f64, h: f64) {
    out.push_str(" width=\"");
    svg::num(out, w);
    out.push_str("\" height=\"");
    svg::num(out, h);
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_of_the_web_app() {
        assert_eq!(series_color(0, 1), "#440154");
        assert_eq!(series_color(0, 2), "#440154");
        assert_eq!(series_color(1, 2), "#1e9c89");
        // the middle of six anchors: half of the third and the fourth
        assert_eq!(series_color(1, 3), "#38588c");
    }
}

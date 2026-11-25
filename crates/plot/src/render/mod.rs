//! The renderer: a figure of the spec in, one SVG string out.
//!
//! A plot is one Charton chart (decision (a) of the spike): the crate resolves the axes
//! ([`axis`]), prepares the layers of the elements ([`marks`]) and lets Charton lay out the
//! panel and draw the axes and the marks through the writer of [`crate::svg`], which groups the
//! layers of every element. The figure places its plots in the cells of its grid and draws its
//! title; the composition of the figure with spacing, legends and the right axis is
//! [`compose`], which a later step replaces.

pub(crate) mod axis;
pub(crate) mod marks;

use std::fmt::Write as _;

use charton::prelude::*;

use crate::spec::{Element, Figure, Plot};
use crate::svg::{
    self, ElementTag, FONT_FAMILY, GridLines, ID_PLACEHOLDER, PlotHooks, PlotWriter, escape, theme,
};
use crate::text::text_width;
use axis::{ResolvedAxis, X_TICK_ROOM, Y_TICK_ROOM, axis_title};

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

/// The padding of a plot inside its cell, in pixels.
const PAD: f64 = 8.0;

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

/// A rectangle of the figure in pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cell {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// One plot drawn into its cell.
pub(crate) struct PlotSvg {
    /// The drawing, in the coordinates of its cell.
    pub body: String,
    /// Its definitions (clip paths, gradients).
    pub defs: String,
}

/// The shares of the ticks of an axis along it, for its grid lines.
fn grid_shares(axis: &ResolvedAxis) -> Vec<f64> {
    axis.tick_coordinates()
        .iter()
        .map(|(u, _)| axis.share(*u))
        .collect()
}

/// Draws one plot into a cell of the figure; the ids it writes start with `prefix`.
pub(crate) fn render_plot(figure: &Figure, plot: &Plot, cell: Cell, prefix: String) -> PlotSvg {
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for e in plot
        .elements
        .iter()
        .filter(|e| e.axis == crate::spec::YAxisRef::Y)
    {
        marks::extent(e, &mut xs, &mut ys);
    }
    // the panel is the cell less the axes, about 80 by 70 pixels, and a plot title
    let title_room = if plot.title.is_some() { 24.0 } else { 0.0 };
    let xa = axis::resolve(&plot.x, &xs, cell.width - 90.0, X_TICK_ROOM);
    let ya = axis::resolve(&plot.y, &ys, cell.height - 70.0 - title_room, Y_TICK_ROOM);
    let colors = element_colors(&plot.elements);
    let hooks = PlotHooks {
        layers: Vec::new(),
        elements: plot
            .elements
            .iter()
            .map(|e| ElementTag {
                id: e.id.clone(),
                kind: kind_name(e),
                legend: e.legend_group().map(str::to_string),
            })
            .collect(),
        x_ticks_hidden: !plot.x.ticks_visible,
        y_ticks_hidden: !plot.y.ticks_visible,
        x_title_hidden: !plot.x.label_visible,
        y_title_hidden: !plot.y.label_visible,
        grid: GridLines {
            x: if plot.x.grid {
                grid_shares(&xa)
            } else {
                Vec::new()
            },
            y: if plot.y.grid {
                grid_shares(&ya)
            } else {
                Vec::new()
            },
        },
    };
    match chart_of(figure, plot, cell, &colors, &xa, &ya) {
        Ok((chart, layers)) => {
            let hooks = PlotHooks { layers, ..hooks };
            let mut writer = PlotWriter::new(prefix, &hooks);
            let mut chart = chart;
            let failed = chart.render(&mut writer).err();
            writer.finish();
            let mut body = writer.body;
            if let Some(e) = failed {
                error_text(&mut body, cell, &e.to_string());
            }
            PlotSvg {
                body,
                defs: writer.defs,
            }
        }
        Err(e) => {
            let mut writer = PlotWriter::new(prefix, &hooks);
            writer.finish();
            let mut body = writer.body;
            error_text(&mut body, cell, &e.to_string());
            PlotSvg {
                body,
                defs: writer.defs,
            }
        }
    }
}

/// Writes the message of a plot Charton could not draw.
fn error_text(body: &mut String, cell: Cell, message: &str) {
    let _ = write!(
        body,
        "<text class=\"sbmlsim-error\" x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\">",
        PAD,
        cell.height / 2.0,
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
    }
}

/// The Charton chart of a plot and what its layers draw.
fn chart_of(
    figure: &Figure,
    plot: &Plot,
    cell: Cell,
    colors: &[String],
    xa: &ResolvedAxis,
    ya: &ResolvedAxis,
) -> Result<(LayeredChart, Vec<svg::LayerInfo>), charton::error::ChartonError> {
    let fonts = &figure.fonts;
    let layers = marks::build(&plot.elements, colors, fonts, xa, ya)?;
    let zero = Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    };
    let ticks_of = |a: &ResolvedAxis, visible: bool| -> Vec<(f64, String)> {
        a.tick_coordinates()
            .into_iter()
            .map(|(u, l)| (u, if visible { l } else { String::new() }))
            .collect()
    };
    let title_of = |a: &crate::spec::Axis| {
        if a.label_visible {
            axis_title(a)
        } else {
            String::new()
        }
    };
    let x_ticks = ticks_of(xa, plot.x.ticks_visible);
    // the last tick label of the x axis reaches half its width past the panel
    let overhang = x_ticks
        .last()
        .map(|(_, l)| text_width(l, fonts.tick) / 2.0)
        .unwrap_or(0.0);
    let (w, h) = (cell.width.max(1.0), cell.height.max(1.0));
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    let (fg, soft) = (theme::FG, theme::FG_SOFT);
    let (tick, label) = (fonts.tick, fonts.label);
    let (x_angle, y_angle) = (-plot.x.tick_rotation, -plot.y.tick_rotation);
    let mut chart = layers
        .chart
        .with_size(w.round() as u32, h.round() as u32)
        .with_margins(PAD / h, (PAD + overhang) / w, PAD / h, PAD / w)
        .with_x_domain(x0, x1)
        .with_x_expand(zero)
        .with_y_domain(y0, y1)
        .with_y_expand(zero)
        .with_x_ticks(x_ticks)
        .with_y_ticks(ticks_of(ya, plot.y.ticks_visible))
        .with_x_label(title_of(&plot.x))
        .with_y_label(title_of(&plot.y))
        .configure_theme(move |t| {
            t.with_show_legend(false)
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
                .with_title_size(label)
                .with_title_color(fg)
                .with_x_tick_label_angle(x_angle)
                .with_y_tick_label_angle(y_angle)
        });
    if let Some(t) = &plot.title {
        chart = chart.with_title(t.clone());
    }
    Ok((chart, layers.info))
}

/// The cells of the plots of a figure: the grid of the figure below its title band, a plot
/// spanning its rows and columns. The composition with spacing, aligned panels and legends
/// replaces this.
pub(crate) fn compose(figure: &Figure, top: f64) -> Vec<Cell> {
    let (cols, rows) = (f64::from(figure.grid.columns), f64::from(figure.grid.rows));
    let (cw, ch) = (figure.width / cols, (figure.height - top).max(1.0) / rows);
    figure
        .plots
        .iter()
        .map(|p| Cell {
            x: f64::from(p.column) * cw,
            y: top + f64::from(p.row) * ch,
            width: f64::from(p.column_span) * cw,
            height: f64::from(p.row_span) * ch,
        })
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
    let fonts = &figure.fonts;
    let band = if figure.title.is_some() {
        fonts.title * 2.0
    } else {
        0.0
    };
    let cells = compose(figure, band);
    let mut defs = String::new();
    let mut plots = String::new();
    for (i, (plot, cell)) in figure.plots.iter().zip(&cells).enumerate() {
        let p = render_plot(figure, plot, *cell, format!("{ID_PLACEHOLDER}-p{i}"));
        defs.push_str(&p.defs);
        plots.push_str("<g class=\"sbmlsim-plot\" data-plot=\"");
        escape(&mut plots, &plot.id);
        plots.push('"');
        if cell.x != 0.0 || cell.y != 0.0 {
            plots.push_str(" transform=\"translate(");
            svg::num(&mut plots, cell.x);
            plots.push(' ');
            svg::num(&mut plots, cell.y);
            plots.push_str(")\"");
        }
        plots.push('>');
        plots.push_str(&p.body);
        plots.push_str("</g>");
    }

    let mut out = String::with_capacity(plots.len() + defs.len() + 1024);
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\"");
    svg_size(&mut out, figure.width, figure.height);
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
    svg_rect_size(&mut out, figure.width, figure.height);
    let _ = write!(out, " fill=\"{}\"/></g>", theme::BACKGROUND);
    if let Some(title) = &figure.title {
        out.push_str("<g class=\"sbmlsim-figure-title\"><text");
        let _ = write!(out, " x=\"");
        svg::num(&mut out, figure.width / 2.0);
        out.push_str("\" y=\"");
        svg::num(&mut out, band / 2.0);
        out.push_str("\" font-size=\"");
        svg::num(&mut out, fonts.title);
        let _ = write!(
            out,
            "\" font-weight=\"bold\" text-anchor=\"middle\" dominant-baseline=\"middle\" fill=\"{}\">",
            theme::FG
        );
        escape(&mut out, title);
        out.push_str("</text></g>");
    }
    out.push_str(&plots);
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

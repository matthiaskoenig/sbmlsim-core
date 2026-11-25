//! The legends of a figure, drawn by the crate (decision (a) of the spike: Charton's legend
//! draws a point per category only).
//!
//! A legend has one entry per legend group of its elements ([`Element::legend_group`]), in the
//! order the groups first appear: the label of the first element of the group which has one
//! (else the name of the group) and the swatch of that element as it is drawn, a line with its
//! dash, a marker, both, or a filled box with its edge. A legend of a row merges the entries of
//! the plots of the row which agree in group, label and swatch. Every entry is a group
//! `<g class="sbmlsim-legend-entry" data-legend="<group>">`, which a page toggles the elements
//! of the group (`data-legend` on their groups) by.

use std::fmt::Write as _;

use super::marks::{Stroke, fill_of};
use super::stats_marks::{outline_of, solid_fill};
use crate::layout::Rect;
use crate::spec::{Element, ElementKind, Fonts, MarkerType, Plot};
use crate::svg::{MarkerGlyph, attr_css, attr_dash, attr_num, attr_opacity, escape, glyph, theme};
use crate::text::text_width;

/// The padding inside the frame of a legend in pixels.
pub(crate) const PADDING: f64 = 6.0;
/// The width of a swatch in pixels.
const SWATCH_WIDTH: f64 = 24.0;
/// The gap between a swatch and its label in pixels.
const SWATCH_GAP: f64 = 6.0;
/// The height of an entry in font sizes of the legend.
const ENTRY_HEIGHT: f64 = 1.6;
/// The height of a filled swatch in font sizes of the legend.
const BOX_HEIGHT: f64 = 0.8;
/// The opacity of the background of a legend.
const FRAME_OPACITY: f64 = 0.85;

/// One entry of a legend.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// The legend group, the `data-legend` of the entry and of the elements it stands for.
    pub legend: String,
    /// The label.
    pub label: String,
    /// The SVG of the swatch, centred vertically on 0 and from 0 to [`SWATCH_WIDTH`].
    pub swatch: String,
}

/// The entries of the legend of a plot, `colors` the colours of its elements.
pub(crate) fn plot_entries(plot: &Plot, colors: &[String], fonts: &Fonts) -> Vec<Entry> {
    // the groups in the order they first appear, each with its first element and its first
    // element with a label
    let mut groups: Vec<(&str, usize, Option<usize>)> = Vec::new();
    for (i, e) in plot.elements.iter().enumerate() {
        let Some(g) = e.legend_group() else {
            continue;
        };
        match groups.iter_mut().find(|(name, _, _)| *name == g) {
            Some(entry) => {
                if entry.2.is_none() && e.label.is_some() {
                    entry.2 = Some(i);
                }
            }
            None => groups.push((g, i, e.label.is_some().then_some(i))),
        }
    }
    groups
        .into_iter()
        .map(|(g, first, labelled)| {
            let i = labelled.unwrap_or(first);
            let e = &plot.elements[i];
            Entry {
                legend: g.to_string(),
                label: e.label.clone().unwrap_or_else(|| g.to_string()),
                swatch: swatch(e, &colors[i], fonts),
            }
        })
        .collect()
}

/// The entries of the legend of a row: the entries of its plots, an entry which agrees with an
/// earlier one in group, label and swatch merged into it.
pub(crate) fn merge(lists: impl IntoIterator<Item = Vec<Entry>>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for e in lists.into_iter().flatten() {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

/// The height of one entry in pixels.
fn entry_height(fonts: &Fonts) -> f64 {
    ENTRY_HEIGHT * fonts.legend
}

/// The width and height of a legend of entries in pixels.
pub(crate) fn size(entries: &[Entry], fonts: &Fonts) -> (f64, f64) {
    let widest = entries
        .iter()
        .map(|e| text_width(&e.label, fonts.legend))
        .fold(0.0, f64::max);
    (
        2.0 * PADDING + SWATCH_WIDTH + SWATCH_GAP + widest,
        2.0 * PADDING + entries.len() as f64 * entry_height(fonts),
    )
}

/// The rectangles of the entries of a legend whose frame is `frame`.
pub(crate) fn entry_rects(frame: Rect, count: usize, fonts: &Fonts) -> Vec<Rect> {
    let h = entry_height(fonts);
    (0..count)
        .map(|k| Rect {
            x: frame.x + PADDING,
            y: frame.y + PADDING + k as f64 * h,
            width: frame.width - 2.0 * PADDING,
            height: h,
        })
        .collect()
}

/// Draws a legend into `frame`; `owner` is the attribute which names its plot or row.
pub(crate) fn draw(
    out: &mut String,
    owner: (&str, &str),
    frame: Rect,
    entries: &[Entry],
    fonts: &Fonts,
) {
    out.push_str("<g class=\"sbmlsim-legend\" ");
    out.push_str(owner.0);
    out.push_str("=\"");
    escape(out, owner.1);
    out.push_str("\"><rect class=\"sbmlsim-legend-frame\"");
    attr_num(out, "x", frame.x);
    attr_num(out, "y", frame.y);
    attr_num(out, "width", frame.width);
    attr_num(out, "height", frame.height);
    let _ = write!(
        out,
        " rx=\"2\" fill=\"{}\" fill-opacity=\"{FRAME_OPACITY}\" stroke=\"{}\" stroke-width=\"1\"/>",
        theme::BACKGROUND,
        theme::GRID
    );
    for (e, r) in entries.iter().zip(entry_rects(frame, entries.len(), fonts)) {
        let cy = r.y + r.height / 2.0;
        out.push_str("<g class=\"sbmlsim-legend-entry\" data-legend=\"");
        escape(out, &e.legend);
        out.push_str("\">");
        if !e.swatch.is_empty() {
            out.push_str("<g class=\"sbmlsim-legend-swatch\" transform=\"translate(");
            crate::svg::num(out, r.x);
            out.push(' ');
            crate::svg::num(out, cy);
            out.push_str(")\">");
            out.push_str(&e.swatch);
            out.push_str("</g>");
        }
        out.push_str("<text class=\"sbmlsim-legend-label\"");
        attr_num(out, "x", r.x + SWATCH_WIDTH + SWATCH_GAP);
        attr_num(out, "y", cy);
        attr_num(out, "font-size", fonts.legend);
        let _ = write!(
            out,
            " fill=\"{}\" dominant-baseline=\"central\">",
            theme::FG
        );
        escape(out, &e.label);
        out.push_str("</text></g>");
    }
    out.push_str("</g>");
}

/// Writes a horizontal line through the swatch.
fn line(out: &mut String, s: &Stroke) {
    out.push_str("<path d=\"M0 0L");
    crate::svg::num(out, SWATCH_WIDTH);
    out.push_str(" 0\" fill=\"none\"");
    attr_css(out, "stroke", &s.color);
    attr_num(out, "stroke-width", s.width);
    let dash: Vec<f32> = s.dash.iter().map(|d| *d as f32).collect();
    attr_dash(out, &dash);
    attr_opacity(out, "opacity", s.opacity);
    out.push_str("/>");
}

/// Writes a filled box across the swatch with an optional edge.
fn filled(out: &mut String, (fill, opacity): (&str, f64), edge: Option<&Stroke>, fonts: &Fonts) {
    let h = BOX_HEIGHT * fonts.legend;
    out.push_str("<rect x=\"0\"");
    attr_num(out, "y", -h / 2.0);
    attr_num(out, "width", SWATCH_WIDTH);
    attr_num(out, "height", h);
    attr_css(out, "fill", fill);
    attr_opacity(out, "fill-opacity", opacity);
    if let Some(s) = edge {
        attr_css(out, "stroke", &s.color);
        attr_num(out, "stroke-width", s.width);
        attr_opacity(out, "stroke-opacity", s.opacity);
    }
    out.push_str("/>");
}

/// The swatch of an element as it is drawn.
fn swatch(e: &Element, color: &str, fonts: &Fonts) -> String {
    let style = &e.style;
    let mut out = String::new();
    match &e.kind {
        ElementKind::Curve { .. } => {
            if let Some(s) = Stroke::of_line(style, color) {
                line(&mut out, &s);
            }
            let m = &style.marker;
            if m.kind != MarkerType::None && m.size > 0.0 {
                let g = MarkerGlyph {
                    kind: m.kind,
                    size: m.size.min(ENTRY_HEIGHT * fonts.legend),
                    fill: m.fill.clone().unwrap_or_else(|| color.to_string()),
                    edge: m.edge.clone().unwrap_or_else(|| color.to_string()),
                    edge_width: m.edge_width,
                    opacity: style.opacity,
                };
                glyph(&mut out, &g, SWATCH_WIDTH / 2.0, 0.0);
            }
        }
        ElementKind::Band { median, .. } => {
            let (fill, opacity) = fill_of(style, color);
            filled(&mut out, (&fill, opacity), None, fonts);
            if median.is_some()
                && let Some(s) = Stroke::of_line(style, color)
            {
                line(&mut out, &s);
            }
        }
        ElementKind::Area { .. } => {
            let (fill, opacity) = fill_of(style, color);
            filled(&mut out, (&fill, opacity), None, fonts);
        }
        ElementKind::Bar { .. } | ElementKind::Histogram { .. } => {
            let (fill, opacity) = solid_fill(style, color);
            filled(&mut out, (&fill, opacity), None, fonts);
        }
        ElementKind::Box { .. } | ElementKind::Violin { .. } => {
            let (fill, opacity) = fill_of(style, color);
            filled(
                &mut out,
                (&fill, opacity),
                Some(&outline_of(style, color)),
                fonts,
            );
        }
        ElementKind::Polygon { .. } => {
            let (fill, opacity) = fill_of(style, color);
            let edge = Stroke::of_line(style, color);
            filled(&mut out, (&fill, opacity), edge.as_ref(), fonts);
        }
        ElementKind::Heatmap { .. } => filled(&mut out, (color, style.opacity), None, fonts),
        ElementKind::Rule { .. } => {
            if let Some(s) = Stroke::of_line(style, color) {
                line(&mut out, &s);
            }
        }
        ElementKind::Text { .. } => {}
    }
    out
}

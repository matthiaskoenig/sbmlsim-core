//! The legends of a figure, drawn by the crate (decision (a) of the spike: Charton's legend
//! draws a point per category only).
//!
//! A legend has one entry per legend group of its elements
//! ([`Element::legend_group`](crate::spec::Element::legend_group)), in the
//! order the groups first appear: the label of the first element of the group which has one
//! (else the name of the group) and the swatch of that element as it is drawn, a line with its
//! dash, a marker, both, or a filled box with its edge. A legend of a row merges the entries of
//! the plots of the row which agree in group, label and swatch. Every entry is a group
//! `<g class="sbmlsim-legend-entry" data-legend="<group>">`, which a page toggles the elements
//! of the group (`data-legend` on their groups) by.
//!
//! `Plot::legend_order` puts the groups it lists first, in its order, so a writer draws frozen
//! runs under the live curves and still lists them last (as `plot.ts` does). A group whose
//! elements differ in colour, a frozen run of every series of a panel, belongs to none of them:
//! its swatch is drawn in the neutral colour `FG_SOFT`, with its dash. `Plot::legend_swatches`
//! (since 0.28.0) gives the swatch of a group a style of its own (the lines of a scan in the
//! middle colour of their map), drawn as the kind of the element the entry stands for. The
//! frame of a legend is the room it takes in the layout ([`room`]), which holds the largest
//! legend it may show, so nothing moves when entries come and go; the entries drawn now fill it
//! from the top. A label of several lines (since 0.28.0) makes its entry taller by its lines
//! after the first, the lines centred on the entry and its swatch at its middle. Since 0.31.0 a
//! legend may have a title above its entries and fill several columns (the legend below the
//! grid), and the swatch of a group of a band and a curve draws the band behind the curve.

use std::fmt::Write as _;

use super::marks::{ERROR_WIDTH, Stroke, fill_of};
use super::stats_marks::{outline_of, solid_fill};
use crate::layout::Rect;
use crate::spec::{ElementKind, Fonts, MarkerType, Plot, Style};
use crate::svg::{
    MarkerGlyph, Stack, attr_css, attr_dash, attr_num, attr_opacity, attr_weight, escape, glyph,
    text_lines, theme,
};
use crate::text::{extra_lines, text_width_weighted};

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
/// The length of the error bar of a swatch in font sizes of the legend.
const ERROR_BAR: f64 = 1.1;
/// The opacity of the background of a legend.
const FRAME_OPACITY: f64 = 0.85;
/// The gap between two columns of entries in font sizes of the legend (matplotlib's
/// `columnspacing`).
const COLUMN_GAP: f64 = 2.0;
/// The height of the title of a legend in font sizes of the legend, a line and its gap to the
/// entries.
const TITLE_HEIGHT: f64 = 1.6;

/// How a legend lays out its entries (since 0.31.0): its title above them and the columns they
/// fill, top down and then left to right, as matplotlib's `title` and `ncols`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Shape {
    /// The title, drawn centred above the entries.
    pub title: Option<String>,
    /// The columns, at least 1.
    pub columns: usize,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            title: None,
            columns: 1,
        }
    }
}

impl Shape {
    /// The height of the title with its lines and its gap to the entries, 0 without one.
    fn title_height(&self, fonts: &Fonts) -> f64 {
        match self.title.as_deref() {
            Some(t) if !t.is_empty() => TITLE_HEIGHT * fonts.legend + extra_lines(t, fonts.legend),
            _ => 0.0,
        }
    }

    /// The width of the title, 0 without one.
    fn title_width(&self, fonts: &Fonts) -> f64 {
        self.title.as_deref().map_or(0.0, |t| {
            text_width_weighted(t, fonts.legend, fonts.legend_weight)
        })
    }

    /// The rows of `n` entries in the columns.
    fn rows(&self, n: usize) -> usize {
        n.div_ceil(self.columns.max(1)).max(1)
    }
}

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
    // the listed groups first, in the order of the list, the others after them
    if let Some(order) = &plot.legend_order {
        let rank = |g: &str| order.iter().position(|o| o == g).unwrap_or(order.len());
        groups.sort_by_key(|(g, _, _)| rank(g));
    }
    groups
        .into_iter()
        .map(|(g, first, labelled)| {
            let i = labelled.unwrap_or(first);
            let e = &plot.elements[i];
            // a group of elements of different colours (a frozen run of every series) is no
            // one of them: its swatch is drawn in the neutral colour, with its dash
            let mut members = plot
                .elements
                .iter()
                .enumerate()
                .filter(|(_, m)| m.legend_group() == Some(g));
            let mixed = members.any(|(k, _)| colors[k] != colors[i]);
            let color = if mixed { theme::FG_SOFT } else { &colors[i] };
            // the swatch the plot gives the group, in its colour or the one of the entry
            let given = plot.legend_swatches.as_ref().and_then(|s| s.get(g));
            // a group of a band and a curve draws the band behind the line and the marker of
            // the curve (since 0.31.0), as matplotlib's handler of a tuple
            let of_kind = |band: bool| {
                plot.elements.iter().enumerate().find(|(_, m)| {
                    m.legend_group() == Some(g)
                        && if band {
                            matches!(m.kind, ElementKind::Band { .. })
                        } else {
                            matches!(m.kind, ElementKind::Curve { .. })
                        }
                })
            };
            let tinted = |k: usize| {
                if mixed {
                    theme::FG_SOFT
                } else {
                    colors[k].as_str()
                }
            };
            let swatch = match (given, of_kind(true), of_kind(false)) {
                (Some(style), _, _) => swatch(
                    &e.kind,
                    style,
                    style.color.as_deref().unwrap_or(color),
                    fonts,
                ),
                (None, Some((b, band)), Some((c, curve))) => {
                    let mut out = String::new();
                    let (fill, opacity) = fill_of(&band.style, tinted(b));
                    filled(&mut out, (&fill, opacity), None, fonts);
                    out.push_str(&swatch(&curve.kind, &curve.style, tinted(c), fonts));
                    out
                }
                (None, _, _) => swatch(&e.kind, &e.style, color, fonts),
            };
            Entry {
                legend: g.to_string(),
                label: e.label.clone().unwrap_or_else(|| g.to_string()),
                swatch,
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

/// [`merge`] sorted by the `legend_order` of the plots of the row: the groups any of them lists
/// first, in the order of the first plot which lists them, the others after them in the order
/// of their first appearance.
pub(crate) fn merge_ordered(
    lists: impl IntoIterator<Item = Vec<Entry>>,
    orders: &[&[String]],
) -> Vec<Entry> {
    let mut out = merge(lists);
    let mut combined: Vec<&str> = Vec::new();
    for g in orders.iter().flat_map(|o| o.iter()) {
        if !combined.contains(&g.as_str()) {
            combined.push(g);
        }
    }
    if !combined.is_empty() {
        out.sort_by_key(|e| {
            combined
                .iter()
                .position(|g| *g == e.legend)
                .unwrap_or(combined.len())
        });
    }
    out
}

/// The height of the entry of a label in pixels: [`ENTRY_HEIGHT`] font sizes and the lines of
/// the label after its first.
fn entry_height(label: &str, fonts: &Fonts) -> f64 {
    ENTRY_HEIGHT * fonts.legend + extra_lines(label, fonts.legend)
}

/// The width and height of a legend of entries in pixels.
pub(crate) fn size(entries: &[Entry], shape: &Shape, fonts: &Fonts) -> (f64, f64) {
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    size_of_labels(&labels, shape, fonts)
}

/// The widths of the columns of a legend of labels: of the swatch, its gap and the widest label
/// of each column.
fn column_widths(labels: &[&str], shape: &Shape, fonts: &Fonts) -> Vec<f64> {
    let rows = shape.rows(labels.len());
    labels
        .chunks(rows)
        .map(|column| {
            let widest = column
                .iter()
                .map(|l| text_width_weighted(l, fonts.legend, fonts.legend_weight))
                .fold(0.0, f64::max);
            SWATCH_WIDTH + SWATCH_GAP + widest
        })
        .collect()
}

/// The width and height of a legend of entries of these labels in pixels.
fn size_of_labels(labels: &[&str], shape: &Shape, fonts: &Fonts) -> (f64, f64) {
    let rows = shape.rows(labels.len());
    let widths = column_widths(labels, shape, fonts);
    let gaps = widths.len().saturating_sub(1) as f64 * COLUMN_GAP * fonts.legend;
    let entries_width = if widths.is_empty() {
        SWATCH_WIDTH + SWATCH_GAP
    } else {
        widths.iter().sum::<f64>() + gaps
    };
    let height = labels
        .chunks(rows)
        .map(|column| column.iter().map(|l| entry_height(l, fonts)).sum::<f64>())
        .fold(0.0, f64::max);
    (
        2.0 * PADDING + entries_width.max(shape.title_width(fonts)),
        2.0 * PADDING + shape.title_height(fonts) + height,
    )
}

/// The room a legend takes in the layout: the largest of the legends it may show, `None` for a
/// legend which shows nothing in any state.
///
/// `declared` are the declared legends (`declaredLegends` of `plot.ts`): the lists of labels
/// the legend may show in some state of a page, more than its elements carry now. A drawn
/// entry whose label is in none of the declared lists counts as shown in every state (a live
/// curve, a frozen run), so it is part of every list the room is taken over; every list is laid out
/// together with these entries, and so are the entries drawn now, so the legend is as wide as
/// the widest label of any of them and as tall as the one of the most rows. The room is then a
/// function of what a page may show and not of what it shows now: two states of a page which
/// draw different lists keep their plot and their figure where they are.
pub(crate) fn room(
    entries: &[Entry],
    declared: &[&[String]],
    shape: &Shape,
    fonts: &Fonts,
) -> Option<(f64, f64)> {
    let lists: Vec<&[String]> = declared.iter().copied().filter(|l| !l.is_empty()).collect();
    if entries.is_empty() && lists.is_empty() {
        return None;
    }
    let (mut width, mut height) = size(entries, shape, fonts);
    let held = |label: &str| lists.iter().any(|l| l.iter().any(|d| d == label));
    let always: Vec<&str> = entries
        .iter()
        .map(|e| e.label.as_str())
        .filter(|l| !held(l))
        .collect();
    for list in lists {
        let labels: Vec<&str> = always
            .iter()
            .copied()
            .chain(list.iter().map(String::as_str))
            .collect();
        let (w, h) = size_of_labels(&labels, shape, fonts);
        width = width.max(w);
        height = height.max(h);
    }
    Some((width, height))
}

/// The rectangles of the entries of a legend whose frame is `frame`, from its top below its
/// title, column by column.
pub(crate) fn entry_rects(
    frame: Rect,
    entries: &[Entry],
    shape: &Shape,
    fonts: &Fonts,
) -> Vec<Rect> {
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    let widths = column_widths(&labels, shape, fonts);
    let rows = shape.rows(entries.len());
    let top = frame.y + PADDING + shape.title_height(fonts);
    let one = widths.len() <= 1;
    let mut x = frame.x + PADDING;
    let mut out = Vec::with_capacity(entries.len());
    for (column, width) in entries.chunks(rows).zip(widths) {
        let mut y = top;
        for e in column {
            let height = entry_height(&e.label, fonts);
            out.push(Rect {
                x,
                y,
                // a legend of one column: the entries over the width of the frame
                width: if one {
                    frame.width - 2.0 * PADDING
                } else {
                    width
                },
                height,
            });
            y += height;
        }
        x += width + COLUMN_GAP * fonts.legend;
    }
    out
}

/// Draws a legend into `frame`; `owner` is the attribute which names its plot or row.
pub(crate) fn draw(
    out: &mut String,
    owner: (&str, &str),
    frame: Rect,
    entries: &[Entry],
    shape: &Shape,
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
    if let Some(title) = shape.title.as_deref().filter(|t| !t.is_empty()) {
        // centred above the entries, as matplotlib's title of a legend
        let (x, y) = (frame.x + frame.width / 2.0, frame.y + PADDING);
        out.push_str("<text class=\"sbmlsim-legend-title\"");
        attr_num(out, "x", x);
        attr_num(out, "y", y);
        attr_num(out, "font-size", fonts.legend);
        attr_weight(out, fonts.legend_weight);
        let _ = write!(
            out,
            " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"hanging\">",
            theme::FG
        );
        text_lines(out, title, (x, y), fonts.legend, Stack::Down);
        out.push_str("</text>");
    }
    for (e, r) in entries
        .iter()
        .zip(entry_rects(frame, entries, shape, fonts))
    {
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
        let x = r.x + SWATCH_WIDTH + SWATCH_GAP;
        attr_num(out, "x", x);
        attr_num(out, "y", cy);
        attr_num(out, "font-size", fonts.legend);
        attr_weight(out, fonts.legend_weight);
        let _ = write!(
            out,
            " fill=\"{}\" dominant-baseline=\"central\">",
            theme::FG
        );
        text_lines(out, &e.label, (x, cy), fonts.legend, Stack::Centred);
        out.push_str("</text></g>");
    }
    out.push_str("</g>");
}

/// Writes a horizontal line through the swatch.
fn line(out: &mut String, s: &Stroke) {
    segment(out, s, (0.0, 0.0), (SWATCH_WIDTH, 0.0));
}

/// Writes a segment of a stroke in the swatch.
fn segment(out: &mut String, s: &Stroke, (x1, y1): (f64, f64), (x2, y2): (f64, f64)) {
    out.push_str("<path d=\"M");
    crate::svg::num(out, x1);
    out.push(' ');
    crate::svg::num(out, y1);
    out.push('L');
    crate::svg::num(out, x2);
    out.push(' ');
    crate::svg::num(out, y2);
    out.push_str("\" fill=\"none\"");
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

/// The swatch of an element of a kind drawn with a style in a colour.
fn swatch(kind: &ElementKind, style: &Style, color: &str, fonts: &Fonts) -> String {
    let mut out = String::new();
    match kind {
        ElementKind::Curve { xerr, yerr, .. } => {
            if let Some(s) = Stroke::of_line(style, color) {
                line(&mut out, &s);
            }
            // the error bars of the curve, a short bar through the middle of the swatch
            let bar = Stroke {
                color: color.to_string(),
                width: ERROR_WIDTH,
                dash: Vec::new(),
                opacity: style.opacity,
            };
            let half = ERROR_BAR * fonts.legend / 2.0;
            let mid = SWATCH_WIDTH / 2.0;
            if yerr.is_some() {
                segment(&mut out, &bar, (mid, -half), (mid, half));
            }
            if xerr.is_some() {
                segment(&mut out, &bar, (mid - half, 0.0), (mid + half, 0.0));
            }
            let m = &style.marker;
            if m.kind != MarkerType::None && m.size > 0.0 {
                let g = MarkerGlyph {
                    size: m.size.min(ENTRY_HEIGHT * fonts.legend),
                    ..MarkerGlyph::of(m, color, style.opacity)
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
        ElementKind::Bar { .. } => {
            // the outline of the bars: the line the style gives
            let (fill, opacity) = solid_fill(style, color);
            let edge = style.line.and_then(|l| Stroke::of(l, style, color));
            filled(&mut out, (&fill, opacity), edge.as_ref(), fonts);
        }
        ElementKind::Histogram { .. } => {
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

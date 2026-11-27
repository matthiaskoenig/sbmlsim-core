//! The second x axis of a plot on top of its panel (since 0.33.0), matplotlib's `twiny` with
//! ticks and labels of its own: its limits are its own `min` and `max`, else those of the x
//! axis, its ticks and labels its own or the ones it computes. No element is drawn against it.
//! The crate draws its spine, its ticks, its tick labels above them and its title above those
//! into room above the panel, and the title of the plot above the axis.

use std::fmt::Write as _;

use super::axis::{ResolvedAxis, TickRoom, axis_title};
use super::{TICK_LABEL_PADDING, TICK_LENGTH, TITLE_GAP};
use crate::layout::Rect;
use crate::spec::{Axis, Fonts};
use crate::svg::{Stack, attr_css, attr_num, attr_weight, label_lines, num, text_lines, theme};
use crate::text::{extra_lines, label_extent, label_height, label_width};

/// The top axis laid out over its ticks.
#[derive(Debug, Clone)]
pub(crate) struct TopAxis {
    /// The axis resolved.
    pub resolved: ResolvedAxis,
    /// The distance from the spine to the far side of the tick labels.
    labels_depth: f64,
    /// The room the axis takes above the panel.
    pub depth: f64,
}

/// Resolves and lays out the top axis `axis` of a panel `length` pixels wide, the x axis
/// resolved as `xa`.
pub(crate) fn layout(axis: &Axis, xa: &ResolvedAxis, length: f64, fonts: &Fonts) -> TopAxis {
    let ends = Axis {
        min: Some(axis.min.unwrap_or(xa.low)),
        max: Some(axis.max.unwrap_or(xa.high)),
        ..axis.clone()
    };
    let resolved = super::axis::resolve(&ends, &[], &[], length, TickRoom::x(fonts.tick));
    let (labels, marks) = (axis.ticks_visible, axis.marks_visible());
    let tallest = resolved
        .tick_coordinates()
        .iter()
        .map(|(_, l)| {
            label_extent(
                label_width(l, fonts.tick, fonts.tick_weight),
                label_height(l, fonts.tick),
                axis.tick_rotation,
                true,
            )
        })
        .fold(0.0, f64::max);
    let labels_depth = match (labels, marks) {
        (true, _) => TICK_LENGTH + TICK_LABEL_PADDING + tallest,
        (false, true) => TICK_LENGTH,
        (false, false) => 0.0,
    };
    let name = axis_title(axis);
    let title = axis.label_visible && !name.is_empty();
    let depth = labels_depth
        + if title {
            TITLE_GAP + fonts.label + extra_lines(&name, fonts.label)
        } else {
            0.0
        };
    TopAxis {
        resolved,
        labels_depth,
        depth,
    }
}

/// Draws the top axis at the top edge of `panel`.
pub(crate) fn draw(out: &mut String, axis: &Axis, top: &TopAxis, fonts: &Fonts, panel: Rect) {
    let r = &top.resolved;
    let y = panel.y;
    let x_of = |u: f64| panel.x + panel.width * r.share(u);
    let ticks = r.tick_coordinates_indexed();
    out.push_str("<g class=\"charton-axis charton-axis-x2\" data-axis=\"x2\">");
    if axis.line_visible {
        let _ = write!(out, "<g class=\"charton-axis-line\"><path d=\"M");
        num(out, panel.x);
        out.push(' ');
        num(out, y);
        out.push('H');
        num(out, panel.x + panel.width);
        let _ = write!(
            out,
            "\" fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/></g>",
            theme::FG_SOFT
        );
    }
    if axis.marks_visible() {
        let _ = write!(
            out,
            "<g class=\"charton-tick\" stroke=\"{}\">",
            theme::FG_SOFT
        );
        let marks: Vec<(f64, bool)> = r
            .mark_shares()
            .unwrap_or_else(|| ticks.iter().map(|(_, u, _)| (r.share(*u), true)).collect());
        for (t, major) in marks {
            let (length, width) = if major {
                (TICK_LENGTH, 1.0)
            } else {
                (crate::svg::MINOR_TICK_LENGTH, crate::svg::MINOR_TICK_WIDTH)
            };
            out.push_str("<path d=\"M");
            num(out, panel.x + panel.width * t);
            out.push(' ');
            num(out, y);
            out.push('V');
            num(out, y - length);
            out.push('"');
            attr_num(out, "stroke-width", width);
            out.push_str("/>");
        }
        out.push_str("</g>");
    }
    if axis.ticks_visible {
        let _ = write!(
            out,
            "<g class=\"charton-tick-label\" fill=\"{}\"",
            theme::FG_SOFT
        );
        attr_num(out, "font-size", fonts.tick);
        attr_weight(out, fonts.tick_weight);
        out.push('>');
        let ly = y - TICK_LENGTH - TICK_LABEL_PADDING;
        for (k, u, label) in &ticks {
            let x = x_of(*u);
            out.push_str("<text");
            attr_num(out, "x", x);
            attr_num(out, "y", ly);
            if let Some(c) = axis.tick_label_color.as_ref().and_then(|c| c.of(*k)) {
                attr_css(out, "fill", c);
            }
            if axis.tick_rotation != 0.0 {
                // a rotated label starts at its tick and runs up, as matplotlib's
                out.push_str(" text-anchor=\"start\" transform=\"rotate(");
                num(out, -axis.tick_rotation);
                out.push(' ');
                num(out, x);
                out.push(' ');
                num(out, ly);
                out.push_str(")\"");
            } else {
                out.push_str(" text-anchor=\"middle\"");
            }
            out.push_str(" dominant-baseline=\"text-after-edge\">");
            label_lines(
                out,
                label,
                (x, ly),
                fonts.tick,
                Stack::Up,
                "text-after-edge",
            );
            out.push_str("</text>");
        }
        out.push_str("</g>");
    }
    let name = axis_title(axis);
    if axis.label_visible && !name.is_empty() {
        let (tx, ty) = (
            panel.x + panel.width / 2.0,
            y - top.labels_depth - TITLE_GAP,
        );
        out.push_str("<g class=\"charton-axis-title\"><text");
        attr_num(out, "x", tx);
        attr_num(out, "y", ty);
        attr_num(out, "font-size", fonts.label);
        attr_weight(out, fonts.label_weight);
        let _ = write!(
            out,
            " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"text-after-edge\">",
            theme::FG
        );
        text_lines(
            out,
            &name,
            (tx, ty),
            fonts.label,
            Stack::Up,
            "text-after-edge",
        );
        out.push_str("</text></g>");
    }
    out.push_str("</g>");
}

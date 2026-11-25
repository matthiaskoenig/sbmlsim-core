//! The right y axis of a plot (decision (b) of the spike).
//!
//! Charton has no secondary axis: the elements of the right axis are drawn by an overlay chart
//! whose panel is the panel of the primary chart, and the crate draws the spine, the ticks, the
//! tick labels and the title of the axis into the room right of the panel, which the layout
//! reserves from the Inter widths of the labels. The groups carry Charton's classes with the
//! axis `y2`, so a stylesheet restyles them with the other axes.

use std::fmt::Write as _;

use super::axis::{ResolvedAxis, axis_title};
use super::{TICK_LABEL_PADDING, TICK_LENGTH, TITLE_GAP};
use crate::layout::Rect;
use crate::spec::{Axis, Fonts};
use crate::svg::{attr_num, escape, num, theme};
use crate::text::{label_extent, text_width};

/// The right axis laid out over its ticks.
#[derive(Debug, Clone)]
pub(crate) struct RightAxis {
    /// The ticks as coordinates of the axis with their labels.
    ticks: Vec<(f64, String)>,
    /// The title, `None` when it is hidden or empty.
    title: Option<String>,
    /// The distance from the spine to the far side of the tick labels.
    labels_depth: f64,
    /// The room the axis takes right of the panel.
    pub depth: f64,
}

/// Lays out the right axis `axis`, resolved as `resolved`.
pub(crate) fn layout(axis: &Axis, resolved: &ResolvedAxis, fonts: &Fonts) -> RightAxis {
    let ticks = if axis.ticks_visible {
        resolved.tick_coordinates()
    } else {
        Vec::new()
    };
    let widest = ticks
        .iter()
        .map(|(_, l)| {
            label_extent(
                text_width(l, fonts.tick),
                fonts.tick,
                axis.tick_rotation,
                false,
            )
        })
        .fold(0.0, f64::max);
    let labels_depth = if axis.ticks_visible {
        TICK_LENGTH + TICK_LABEL_PADDING + widest
    } else {
        0.0
    };
    let name = axis_title(axis);
    let title = (axis.label_visible && !name.is_empty()).then_some(name);
    let depth = labels_depth
        + if title.is_some() {
            TITLE_GAP + fonts.label
        } else {
            0.0
        };
    RightAxis {
        ticks,
        title,
        labels_depth,
        depth,
    }
}

/// Draws the right axis at the right edge of `panel`.
pub(crate) fn draw(
    out: &mut String,
    axis: &Axis,
    resolved: &ResolvedAxis,
    layout: &RightAxis,
    fonts: &Fonts,
    panel: Rect,
) {
    let x = panel.x + panel.width;
    let y_of = |u: f64| panel.y + panel.height * (1.0 - resolved.share(u));
    out.push_str("<g class=\"charton-axis charton-axis-y2\" data-axis=\"y2\">");
    out.push_str("<g class=\"charton-axis-line\"><path d=\"M");
    num(out, x);
    out.push(' ');
    num(out, panel.y);
    out.push('L');
    num(out, x);
    out.push(' ');
    num(out, panel.y + panel.height);
    let _ = write!(
        out,
        "\" fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/></g>",
        theme::FG_SOFT
    );
    if !layout.ticks.is_empty() {
        out.push_str("<g class=\"charton-tick\">");
        for (u, _) in &layout.ticks {
            let y = y_of(*u);
            out.push_str("<path d=\"M");
            num(out, x);
            out.push(' ');
            num(out, y);
            out.push('L');
            num(out, x + TICK_LENGTH);
            out.push(' ');
            num(out, y);
            let _ = write!(
                out,
                "\" fill=\"none\" stroke=\"{}\" stroke-width=\"1\"/>",
                theme::FG_SOFT
            );
        }
        out.push_str("</g><g class=\"charton-tick-label\">");
        let lx = x + TICK_LENGTH + TICK_LABEL_PADDING;
        for (u, label) in &layout.ticks {
            let y = y_of(*u);
            out.push_str("<text");
            attr_num(out, "x", lx);
            attr_num(out, "y", y);
            attr_num(out, "font-size", fonts.tick);
            let _ = write!(
                out,
                " fill=\"{}\" dominant-baseline=\"central\"",
                theme::FG_SOFT
            );
            if axis.tick_rotation != 0.0 {
                out.push_str(" transform=\"rotate(");
                num(out, -axis.tick_rotation);
                out.push(' ');
                num(out, lx);
                out.push(' ');
                num(out, y);
                out.push_str(")\"");
            }
            out.push('>');
            escape(out, label);
            out.push_str("</text>");
        }
        out.push_str("</g>");
    }
    if let Some(title) = &layout.title {
        let (tx, ty) = (
            x + layout.labels_depth + TITLE_GAP + fonts.label / 2.0,
            panel.y + panel.height / 2.0,
        );
        out.push_str("<g class=\"charton-axis-title\"><text");
        attr_num(out, "x", tx);
        attr_num(out, "y", ty);
        attr_num(out, "font-size", fonts.label);
        let _ = write!(
            out,
            " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" transform=\"rotate(-90 ",
            theme::FG
        );
        num(out, tx);
        out.push(' ');
        num(out, ty);
        out.push_str(")\">");
        escape(out, title);
        out.push_str("</text></g>");
    }
    out.push_str("</g>");
}

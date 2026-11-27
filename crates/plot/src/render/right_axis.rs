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
use crate::svg::{
    MULTIPLIER_GAP, Stack, attr_num, attr_weight, label_lines, num, text_lines, theme,
};
use crate::text::{extra_lines, label_extent, label_height, label_width};

/// The right axis laid out over its ticks.
#[derive(Debug, Clone)]
pub(crate) struct RightAxis {
    /// The ticks as coordinates of the axis with their labels, none when neither the marks nor
    /// the labels are shown.
    ticks: Vec<(f64, String)>,
    /// Whether the tick marks are shown.
    marks: bool,
    /// Whether the tick labels are shown.
    labels: bool,
    /// Whether the line of the axis is shown.
    line: bool,
    /// The title, `None` when it is hidden or empty.
    title: Option<String>,
    /// The distance from the spine to the far side of the tick labels.
    labels_depth: f64,
    /// The room the axis takes right of the panel.
    pub depth: f64,
    /// The multiplier of the tick labels, written above the right end of the panel (since
    /// 0.31.0).
    pub multiplier: Option<String>,
}

/// Lays out the right axis `axis`, resolved as `resolved`.
pub(crate) fn layout(axis: &Axis, resolved: &ResolvedAxis, fonts: &Fonts) -> RightAxis {
    let (labels, marks) = (axis.ticks_visible, axis.marks_visible());
    let ticks = if labels || marks {
        resolved.tick_coordinates()
    } else {
        Vec::new()
    };
    let tick = fonts.y_tick();
    let widest = ticks
        .iter()
        .map(|(_, l)| {
            label_extent(
                label_width(l, tick, fonts.tick_weight),
                label_height(l, tick),
                axis.tick_rotation,
                false,
            )
        })
        .fold(0.0, f64::max);
    let labels_depth = if labels {
        TICK_LENGTH + TICK_LABEL_PADDING + widest
    } else if marks {
        TICK_LENGTH
    } else {
        0.0
    };
    let name = axis_title(axis);
    // a title of several lines stacks them outwards, away from the axis
    let title_room = TITLE_GAP + fonts.label + extra_lines(&name, fonts.label);
    let title = (axis.label_visible && !name.is_empty()).then_some(name);
    let depth = labels_depth + if title.is_some() { title_room } else { 0.0 };
    RightAxis {
        ticks,
        marks,
        labels,
        line: axis.line_visible,
        title,
        labels_depth,
        depth,
        multiplier: resolved.multiplier.clone().filter(|_| labels),
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
    if layout.line {
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
    }
    // the marks of a log axis, the minor ones shorter and thinner (since 0.33.0)
    if let Some(marks) = resolved.mark_shares().filter(|_| layout.marks) {
        let _ = write!(
            out,
            "<g class=\"charton-tick\" stroke=\"{}\">",
            theme::FG_SOFT
        );
        for (t, major) in marks {
            let y = panel.y + panel.height - t * panel.height;
            let (length, width) = if major {
                (TICK_LENGTH, 1.0)
            } else {
                (crate::svg::MINOR_TICK_LENGTH, crate::svg::MINOR_TICK_WIDTH)
            };
            out.push_str("<path d=\"M");
            num(out, x);
            out.push(' ');
            num(out, y);
            out.push('H');
            num(out, x + length);
            out.push('"');
            attr_num(out, "stroke-width", width);
            out.push_str("/>");
        }
        out.push_str("</g>");
    } else if layout.marks && !layout.ticks.is_empty() {
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
        out.push_str("</g>");
    }
    if layout.labels && !layout.ticks.is_empty() {
        out.push_str("<g class=\"charton-tick-label\">");
        // a label rotated about the middle of its start reaches half its line height times the
        // sine back towards the axis; it starts that much further out, so the rotated label
        // spans `label_extent` from the padding on, as the layout reserved
        let sin = if axis.tick_rotation == 0.0 {
            0.0
        } else {
            libm::sin(axis.tick_rotation * std::f64::consts::PI / 180.0).abs()
        };
        let tick = fonts.y_tick();
        let lx = x + TICK_LENGTH + TICK_LABEL_PADDING + tick / 2.0 * sin;
        for (u, label) in &layout.ticks {
            let y = y_of(*u);
            out.push_str("<text");
            attr_num(out, "x", lx);
            attr_num(out, "y", y);
            attr_num(out, "font-size", tick);
            attr_weight(out, fonts.tick_weight);
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
            label_lines(out, label, (lx, y), tick, Stack::Centred, "central");
            out.push_str("</text>");
        }
        out.push_str("</g>");
    }
    if let Some(m) = &layout.multiplier {
        // right aligned above the right end of the panel, as matplotlib's offset text
        let (mx, my) = (x, panel.y - MULTIPLIER_GAP);
        out.push_str("<g class=\"sbmlsim-axis-multiplier\"><text");
        attr_num(out, "x", mx);
        attr_num(out, "y", my);
        attr_num(out, "font-size", fonts.y_tick());
        attr_weight(out, fonts.tick_weight);
        let _ = write!(out, " fill=\"{}\" text-anchor=\"end\">", theme::FG_SOFT);
        label_lines(out, m, (mx, my), fonts.y_tick(), Stack::Down, "alphabetic");
        out.push_str("</text></g>");
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
        attr_weight(out, fonts.label_weight);
        let _ = write!(
            out,
            " fill=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" transform=\"rotate(-90 ",
            theme::FG
        );
        num(out, tx);
        out.push(' ');
        num(out, ty);
        out.push_str(")\">");
        // the lines stack outwards, right of the axis: down in the rotated text
        text_lines(out, title, (tx, ty), fonts.label, Stack::Down, "middle");
        out.push_str("</text></g>");
    }
    out.push_str("</g>");
}

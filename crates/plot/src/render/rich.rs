//! Texts the crate lays out itself (since 0.33.0): rotated, aligned by a vertical anchor, with
//! a box behind them, as matplotlib's `ax.text(..., rotation=, ha=, va=, bbox=)` draws them.
//!
//! A text is a block of lines [`LINE_HEIGHT`] font sizes apart, every line from
//! [`ASCENT`] above its baseline to [`DESCENT`] below it (the ascender of `l` and the
//! descender of `p`, which matplotlib's layout of a text takes), as wide as its widest line.
//! The block is rotated about the position of the text and the box around the rotated block
//! aligned at the position by its anchors, as matplotlib's default `rotation_mode`: a text
//! rotated by 90 degrees and anchored at its bottom has the end of its last line at the
//! position. The baseline anchor puts the baseline of the last line at the position. Every
//! text is written with the alphabetic baseline and the lines at positions the crate computes,
//! so a browser and the PNG draw it alike; its box is written before it, in the same rotation.

use std::fmt::Write as _;

use crate::spec::{Anchor, FontFamily, FontWeight, TextBox, TextBoxStyle, VerticalAnchor};
use crate::svg::{MONO_FAMILY, attr_css, attr_num, attr_opacity, attr_weight, escape, label_lines};
use crate::text::{LINE_HEIGHT, line_count, text_width_family};

/// The ascent of a line above its baseline in font sizes.
pub(crate) const ASCENT: f64 = 0.76;
/// The descent of a line below its baseline in font sizes.
pub(crate) const DESCENT: f64 = 0.21;
/// The padding of the box of a text without one of its own, in font sizes (matplotlib's
/// `pad=0.3` of a `bbox`).
pub(crate) const BOX_PADDING: f64 = 0.3;

/// How a text is drawn.
#[derive(Debug, Clone)]
pub(crate) struct TextLook {
    /// The font size in pixels.
    pub size: f64,
    /// The weight.
    pub weight: FontWeight,
    /// The family.
    pub family: FontFamily,
    /// The colour `#rrggbb`.
    pub color: String,
    /// The opacity.
    pub opacity: f64,
    /// The horizontal anchor.
    pub anchor: Anchor,
    /// The vertical anchor.
    pub baseline: VerticalAnchor,
    /// The rotation in degrees, counterclockwise.
    pub rotation: f64,
}

/// The width and the height of the block of a text: its widest line and its lines from the
/// ascent of the first to the descent of the last.
pub(crate) fn block_size(
    text: &str,
    size: f64,
    weight: FontWeight,
    family: FontFamily,
) -> (f64, f64) {
    let n = line_count(text) as f64;
    (
        text_width_family(text, size, weight, family),
        (ASCENT + DESCENT + (n - 1.0) * LINE_HEIGHT) * size,
    )
}

/// The padding of a box of a text of a font size: its own, else [`BOX_PADDING`] font sizes.
pub(crate) fn box_padding(b: &TextBox, size: f64) -> f64 {
    b.padding.unwrap_or(BOX_PADDING * size)
}

/// A text placed: the translation of its block, its rotation, its size and the box around
/// it (with its padding) in the pixels it is drawn in.
pub(crate) struct Placed {
    /// Where the upper left corner of the block goes before the rotation.
    translate: (f64, f64),
    /// The width and the height of the block.
    block: (f64, f64),
    /// The axis-aligned box of the rotated block and its padding: x, y, width, height.
    pub bounds: (f64, f64, f64, f64),
}

/// Places a text at `at`: the block of `size` rotated and aligned by the anchors, with a
/// padding of `pad` around it in the bounds.
pub(crate) fn place(at: (f64, f64), block: (f64, f64), look: &TextLook, pad: f64) -> Placed {
    let (w, h) = block;
    let radians = look.rotation.to_radians();
    let (sin, cos) = if look.rotation == 0.0 {
        (0.0, 1.0)
    } else {
        (libm::sin(radians), libm::cos(radians))
    };
    // a point of the block (y down from its top) rotated counterclockwise on the screen
    let rot = |(x, y): (f64, f64)| (x * cos + y * sin, -x * sin + y * cos);
    let corners = [
        (-pad, -pad),
        (w + pad, -pad),
        (w + pad, h + pad),
        (-pad, h + pad),
    ]
    .map(rot);
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    // the anchors align the box of the rotated block without its padding, as matplotlib
    let inner = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(rot);
    let (mut a0, mut a1, mut b0, mut b1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in inner {
        a0 = a0.min(x);
        a1 = a1.max(x);
        b0 = b0.min(y);
        b1 = b1.max(y);
    }
    let dx = match look.anchor {
        Anchor::Start => a0,
        Anchor::Middle => (a0 + a1) / 2.0,
        Anchor::End => a1,
    };
    let dy = match look.baseline {
        VerticalAnchor::Top => b0,
        VerticalAnchor::Center => (b0 + b1) / 2.0,
        VerticalAnchor::Bottom => b1,
        // the baseline of the last line, rotated with the block
        VerticalAnchor::Baseline => rot((0.0, h - DESCENT * look.size)).1.max(b0).min(b1),
    };
    let translate = (at.0 - dx, at.1 - dy);
    Placed {
        translate,
        block,
        bounds: (translate.0 + x0, translate.1 + y0, x1 - x0, y1 - y0),
    }
}

/// Writes a text at `at` with an optional box behind it; returns the axis-aligned bounds of
/// the text and its box.
pub(crate) fn write_text(
    out: &mut String,
    text: &str,
    at: (f64, f64),
    look: &TextLook,
    frame: Option<&TextBox>,
) -> (f64, f64, f64, f64) {
    let block = block_size(text, look.size, look.weight, look.family);
    let pad = frame.map_or(0.0, |b| box_padding(b, look.size));
    let p = place(at, block, look, pad);
    write_placed(out, text, &p, look, frame, pad);
    p.bounds
}

/// Writes a placed text and its box.
pub(crate) fn write_placed(
    out: &mut String,
    text: &str,
    p: &Placed,
    look: &TextLook,
    frame: Option<&TextBox>,
    pad: f64,
) {
    let (w, h) = p.block;
    out.push_str("<g transform=\"translate(");
    crate::svg::num(out, p.translate.0);
    out.push(' ');
    crate::svg::num(out, p.translate.1);
    out.push(')');
    if look.rotation != 0.0 {
        out.push_str(" rotate(");
        crate::svg::num(out, -look.rotation);
        out.push(')');
    }
    out.push_str("\">");
    if let Some(b) = frame {
        out.push_str("<rect class=\"sbmlsim-text-box\"");
        attr_num(out, "x", -pad);
        attr_num(out, "y", -pad);
        attr_num(out, "width", w + 2.0 * pad);
        attr_num(out, "height", h + 2.0 * pad);
        if b.style == TextBoxStyle::Round {
            attr_num(out, "rx", pad);
        }
        match b.fill.as_deref() {
            Some("none") => out.push_str(" fill=\"none\""),
            fill => {
                attr_css(out, "fill", fill.unwrap_or("#ffffff"));
                attr_opacity(out, "fill-opacity", b.opacity);
            }
        }
        match b.edge.as_deref() {
            Some(edge) if edge != "none" && b.edge_width > 0.0 => {
                attr_css(out, "stroke", edge);
                attr_num(out, "stroke-width", b.edge_width);
            }
            _ => {}
        }
        attr_opacity(out, "opacity", look.opacity);
        out.push_str("/>");
    }
    let (x, text_anchor) = match look.anchor {
        Anchor::Start => (0.0, None),
        Anchor::Middle => (w / 2.0, Some("middle")),
        Anchor::End => (w, Some("end")),
    };
    let y = ASCENT * look.size;
    out.push_str("<text");
    attr_num(out, "x", x);
    attr_num(out, "y", y);
    attr_num(out, "font-size", look.size);
    attr_css(out, "fill", &look.color);
    if let Some(a) = text_anchor {
        let _ = write!(out, " text-anchor=\"{a}\"");
    }
    attr_weight(out, look.weight);
    if look.family == FontFamily::Monospace {
        out.push_str(" font-family=\"");
        escape(out, MONO_FAMILY);
        out.push('"');
    }
    attr_opacity(out, "opacity", look.opacity);
    out.push('>');
    label_lines(
        out,
        text,
        (x, y),
        look.size,
        crate::svg::Stack::Down,
        "alphabetic",
    );
    out.push_str("</text></g>");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(anchor: Anchor, baseline: VerticalAnchor, rotation: f64) -> TextLook {
        TextLook {
            size: 10.0,
            weight: FontWeight::Normal,
            family: FontFamily::Sans,
            color: "#000000".into(),
            opacity: 1.0,
            anchor,
            baseline,
            rotation,
        }
    }

    #[test]
    fn anchors_align_the_rotated_block() {
        // a block of 40 by 9.7 at (100, 50)
        let block = (40.0, 9.7);
        let p = place(
            (100.0, 50.0),
            block,
            &look(Anchor::Start, VerticalAnchor::Top, 0.0),
            0.0,
        );
        assert_eq!(p.bounds, (100.0, 50.0, 40.0, 9.7));
        let p = place(
            (100.0, 50.0),
            block,
            &look(Anchor::Middle, VerticalAnchor::Center, 0.0),
            0.0,
        );
        assert!((p.bounds.0 - 80.0).abs() < 1e-9 && (p.bounds.1 - 45.15).abs() < 1e-9);
        // the baseline of the line at the position
        let p = place(
            (0.0, 0.0),
            block,
            &look(Anchor::Start, VerticalAnchor::Baseline, 0.0),
            0.0,
        );
        assert!((p.bounds.1 + 7.6).abs() < 1e-9, "{:?}", p.bounds);
        // rotated by 90 degrees and anchored at its lower left: the box stands on the position
        let p = place(
            (0.0, 0.0),
            block,
            &look(Anchor::Start, VerticalAnchor::Bottom, 90.0),
            0.0,
        );
        let (x, y, w, h) = p.bounds;
        assert!(x.abs() < 1e-9 && (y + 40.0).abs() < 1e-9, "{:?}", p.bounds);
        assert!((w - 9.7).abs() < 1e-9 && (h - 40.0).abs() < 1e-9);
    }
}

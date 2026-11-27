//! The texts and arrows the crate draws itself (since 0.33.0), as raw layers of the writer
//! (`RawSvg`): a text with a rotation, a vertical anchor, a box, superscripts or subscripts, in
//! the fractions of the panel, unclipped or anchored in a corner of the panel (matplotlib's
//! `AnchoredText`), and every arrow, from its tail to its head, with its optional text at the
//! tail, as matplotlib's `annotate` draws it.
//!
//! An arrow of a line is the line of its style with an open or a filled head of 0.4 of its
//! mutation size long and 0.2 of it to each side (matplotlib's `->` and `-|>`, whose mutation
//! size is the size of the text of an annotation, the `tick` font size here), ending 2 pixels
//! before both of its ends (`shrinkA` and `shrinkB`) and moved back by the half of its line
//! width at its tip, so the stroke of the head ends at the head. An arrow with a body is a
//! filled polygon of a body and a head (the `simple` arrow matplotlib draws for `arrowprops`
//! without an `arrowstyle`), shortened by `shrink` of its length at both ends, filled and
//! edged by a line of 1 pixel in its colour, as matplotlib's patch. With a text the arrow starts
//! at the edge of the text: of its box padded by 2 pixels for an arrow of a line, at the corner
//! or the middle of the side of the text nearest to the head for an arrow with a body.

use std::fmt::Write as _;

use super::marks::Pixels;
use super::rich::{self, TextLook};
use crate::layout::Rect;
use crate::spec::{
    Anchor, ArrowHead, Coordinates, Element, ElementKind, FontFamily, Fonts, LegendLoc, LineType,
    TextBox, TextBoxStyle, VerticalAnchor,
};
use crate::svg::{RawSvg, attr_css, attr_num, attr_opacity, num};

/// The padding of an anchored text in font sizes (matplotlib's `pad` of `AnchoredText`).
const ANCHORED_PAD: f64 = 0.4;
/// The distance of an anchored text from the edges of the panel in font sizes (`borderpad`).
const ANCHORED_BORDER: f64 = 0.5;
/// The head of an arrow of a line: its length in mutation sizes (matplotlib's `head_length`).
const HEAD_LENGTH: f64 = 0.4;
/// The half of the width of the head of an arrow of a line in mutation sizes (`head_width`).
const HEAD_HALF_WIDTH: f64 = 0.2;
/// The pixels an arrow of a line ends before each of its ends (`shrinkA`, `shrinkB`).
const LINE_SHRINK: f64 = 2.0;
/// The padding of the text of an arrow of a line which the arrow starts at (the half of the
/// 4 points matplotlib pads the box of the text by).
const TEXT_PAD: f64 = 2.0;
/// The head of an arrow with a body, in body widths, without a head of its own (matplotlib's
/// `headwidth` and `headlength` of 12 points over a `width` of 4).
const BODY_HEAD: f64 = 3.0;

/// Whether a text element needs the crate's layout: a rotation, a vertical anchor, a box, a
/// location, superscripts or subscripts, fractions of the panel or no clip; the others are drawn
/// by Charton as before.
pub(crate) fn rich_text(e: &Element) -> bool {
    match &e.kind {
        ElementKind::Text {
            text,
            rotation,
            baseline,
            text_box,
            loc,
            ..
        } => {
            *rotation != 0.0
                || baseline.is_some()
                || text_box.is_some()
                || loc.is_some()
                || crate::text::has_scripts(text)
                || e.coordinates != Coordinates::Data
                || !e.clip
        }
        _ => false,
    }
}

/// The raw layer of an element, clipped or not.
fn raw(e: &Element, svg: String) -> RawSvg {
    if e.clip {
        RawSvg {
            clipped: svg,
            unclipped: String::new(),
        }
    } else {
        RawSvg {
            clipped: String::new(),
            unclipped: svg,
        }
    }
}

/// The SVG of a text element the crate lays out, `None` for one at a position the axes
/// cannot show.
pub(crate) fn text_layer(e: &Element, color: &str, fonts: &Fonts, px: &Pixels) -> Option<RawSvg> {
    let ElementKind::Text {
        x,
        y,
        text,
        anchor,
        size,
        weight,
        family,
        rotation,
        baseline,
        text_box,
        loc,
    } = &e.kind
    else {
        return None;
    };
    let size = size.unwrap_or(fonts.tick);
    // a dot operator of a monospace text is the middle dot, as Charton's text writes it
    let text = if *family == FontFamily::Monospace {
        text.replace('\u{22c5}', "\u{b7}")
    } else {
        text.clone()
    };
    let mut look = TextLook {
        size,
        weight: *weight,
        family: *family,
        color: color.to_string(),
        opacity: e.style.opacity,
        anchor: *anchor,
        baseline: baseline.unwrap_or(VerticalAnchor::Center),
        rotation: *rotation,
    };
    let mut svg = String::new();
    if let Some(loc) = loc {
        // matplotlib's AnchoredText: the text in a white box with a black edge in a corner
        let anchored = TextBox {
            fill: None,
            opacity: 1.0,
            edge: Some("#000000".into()),
            edge_width: 1.0,
            padding: Some(ANCHORED_PAD * size),
            style: TextBoxStyle::Square,
        };
        let frame = text_box.clone().unwrap_or(anchored);
        let pad = rich::box_padding(&frame, size);
        look.rotation = 0.0;
        look.anchor = Anchor::Start;
        look.baseline = VerticalAnchor::Top;
        let (w, h) = rich::block_size(&text, size, *weight, *family);
        let outer = (w + 2.0 * pad, h + 2.0 * pad);
        let border = ANCHORED_BORDER * size;
        let panel = Rect {
            x: border,
            y: border,
            width: (px.w - 2.0 * border).max(0.0),
            height: (px.h - 2.0 * border).max(0.0),
        };
        let r = loc_rect_exact(panel, outer, *loc);
        rich::write_text(&mut svg, &text, (r.x + pad, r.y + pad), &look, Some(&frame));
    } else {
        let (x, y) = (x.unwrap_or(f64::NAN), y.unwrap_or(f64::NAN));
        let at = px.at(e.coordinates, x, y);
        if !(at.0.is_finite() && at.1.is_finite()) {
            return None;
        }
        rich::write_text(&mut svg, &text, at, &look, text_box.as_ref());
    }
    Some(raw(e, svg))
}

/// The rectangle of `size` at a location of a rectangle, touching its edges.
fn loc_rect_exact(r: Rect, (w, h): (f64, f64), loc: LegendLoc) -> Rect {
    let (left, center, right) = (r.x, r.x + (r.width - w) / 2.0, r.right() - w);
    let (top, middle, bottom) = (r.y, r.y + (r.height - h) / 2.0, r.bottom() - h);
    let (x, y) = match loc {
        LegendLoc::Best | LegendLoc::UpperRight => (right, top),
        LegendLoc::UpperLeft => (left, top),
        LegendLoc::LowerLeft => (left, bottom),
        LegendLoc::LowerRight => (right, bottom),
        LegendLoc::Right | LegendLoc::CenterRight => (right, middle),
        LegendLoc::CenterLeft => (left, middle),
        LegendLoc::LowerCenter => (center, bottom),
        LegendLoc::UpperCenter => (center, top),
        LegendLoc::Center => (center, middle),
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// The point a segment from inside a rectangle towards `to` leaves it, the start if the end
/// is inside.
fn exit(r: (f64, f64, f64, f64), from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
    let (x0, y0, w, h) = r;
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let mut t = 1.0f64;
    if dx > 0.0 {
        t = t.min((x0 + w - from.0) / dx);
    } else if dx < 0.0 {
        t = t.min((x0 - from.0) / dx);
    }
    if dy > 0.0 {
        t = t.min((y0 + h - from.1) / dy);
    } else if dy < 0.0 {
        t = t.min((y0 - from.1) / dy);
    }
    let t = t.clamp(0.0, 1.0);
    (from.0 + t * dx, from.1 + t * dy)
}

/// The point `d` pixels from `a` towards `b`.
fn along(a: (f64, f64), b: (f64, f64), d: f64) -> (f64, f64) {
    let l = (b.0 - a.0).hypot(b.1 - a.1);
    if l <= 0.0 {
        return a;
    }
    (a.0 + (b.0 - a.0) * d / l, a.1 + (b.1 - a.1) * d / l)
}

/// Writes ` d="M ... Z"` of a polygon.
fn polygon(out: &mut String, points: &[(f64, f64)]) {
    out.push_str(" d=\"");
    for (i, (x, y)) in points.iter().enumerate() {
        out.push(if i == 0 { 'M' } else { 'L' });
        num(out, *x);
        out.push(' ');
        num(out, *y);
    }
    out.push_str("Z\"");
}

/// The SVG of an arrow element, `None` for one whose ends the axes cannot show.
pub(crate) fn arrow_layer(e: &Element, color: &str, fonts: &Fonts, px: &Pixels) -> Option<RawSvg> {
    let ElementKind::Arrow {
        x,
        y,
        x_from,
        y_from,
        head,
        head_length,
        head_width,
        body,
        shrink,
        text,
        size,
        weight,
        anchor,
        baseline,
    } = &e.kind
    else {
        return None;
    };
    let tip = px.at(e.coordinates, *x, *y);
    let tail = px.at(e.coordinates, *x_from, *y_from);
    if ![tip.0, tip.1, tail.0, tail.1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let opacity = e.style.opacity;
    let mut svg = String::new();
    let size = size.unwrap_or(fonts.tick);
    // the text at the tail, and the point the arrow starts at
    let mut start = tail;
    if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
        let look = TextLook {
            size,
            weight: *weight,
            family: FontFamily::Sans,
            color: color.to_string(),
            opacity,
            anchor: *anchor,
            baseline: baseline.unwrap_or(VerticalAnchor::Baseline),
            rotation: 0.0,
        };
        let mut text_svg = String::new();
        let (bx, by, bw, bh) = rich::write_text(&mut text_svg, t, tail, &look, None);
        start = if body.is_some() {
            // the corner or the middle of a side nearest to the head
            let pick = |lo: f64, extent: f64, target: f64| {
                [lo, lo + extent / 2.0, lo + extent]
                    .into_iter()
                    .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
                    .unwrap_or(lo)
            };
            (pick(bx, bw, tip.0), pick(by, bh, tip.1))
        } else {
            let padded = (
                bx - TEXT_PAD,
                by - TEXT_PAD,
                bw + 2.0 * TEXT_PAD,
                bh + 2.0 * TEXT_PAD,
            );
            exit(padded, (bx + bw / 2.0, by + bh / 2.0), tip)
        };
        svg.push_str(&text_svg);
    }
    let length = (tip.0 - start.0).hypot(tip.1 - start.1);
    if length <= 0.0 {
        return Some(raw(e, svg));
    }
    let (ux, uy) = ((tip.0 - start.0) / length, (tip.1 - start.1) / length);
    // the normal of the arrow on the screen
    let (nx, ny) = (-uy, ux);
    if let Some(w) = body {
        let cut = shrink * length;
        let (a, b) = (along(start, tip, cut), along(tip, start, cut));
        let l = (length - 2.0 * cut).max(0.0);
        let hl = head_length.unwrap_or(BODY_HEAD * w).min(l);
        let hw = head_width.unwrap_or(BODY_HEAD * w) / 2.0;
        let neck = along(b, a, hl);
        let off = |p: (f64, f64), d: f64| (p.0 + nx * d, p.1 + ny * d);
        let points = [
            off(a, w / 2.0),
            off(neck, w / 2.0),
            off(neck, hw),
            b,
            off(neck, -hw),
            off(neck, -w / 2.0),
            off(a, -w / 2.0),
        ];
        svg.push_str("<path class=\"sbmlsim-arrow\"");
        polygon(&mut svg, &points);
        attr_css(&mut svg, "fill", color);
        attr_opacity(&mut svg, "fill-opacity", opacity);
        attr_css(&mut svg, "stroke", color);
        svg.push_str(" stroke-width=\"1\" stroke-linejoin=\"miter\"");
        attr_opacity(&mut svg, "stroke-opacity", opacity);
        svg.push_str("/>");
        return Some(raw(e, svg));
    }
    // an arrow of a line
    let line = e.style.line_or_default();
    let lw = if line.kind == LineType::None {
        0.0
    } else {
        line.width
    };
    let ms = fonts.tick;
    let hl = head_length.unwrap_or(HEAD_LENGTH * ms);
    let half = head_width.map_or(HEAD_HALF_WIDTH * ms, |w| w / 2.0);
    let a = along(start, tip, LINE_SHRINK.min(length / 2.0));
    let mut b = along(tip, start, LINE_SHRINK.min(length / 2.0));
    let sin = if hl > 0.0 || half > 0.0 {
        half / hl.hypot(half)
    } else {
        1.0
    };
    // the tip of a head moved back by the half of the line width over the sine of its half
    // angle, so the stroke ends at the end (matplotlib's `pad_projected`)
    if *head != ArrowHead::None && sin > 0.0 {
        b = along(b, a, 0.5 * lw / sin);
    }
    let wing = |side: f64| {
        let base = along(b, a, hl);
        (base.0 + nx * half * side, base.1 + ny * half * side)
    };
    let dash = super::marks::dash(line.kind, line.width);
    let stroke = |out: &mut String| {
        attr_css(out, "stroke", color);
        attr_num(out, "stroke-width", lw);
        attr_opacity(out, "opacity", opacity);
    };
    if lw > 0.0 {
        // the line ends at the base of a filled head, at the tip of the others
        let end = if *head == ArrowHead::Filled {
            along(b, a, hl)
        } else {
            b
        };
        let _ = write!(svg, "<path class=\"sbmlsim-arrow\" d=\"M");
        num(&mut svg, a.0);
        svg.push(' ');
        num(&mut svg, a.1);
        svg.push('L');
        num(&mut svg, end.0);
        svg.push(' ');
        num(&mut svg, end.1);
        svg.push_str("\" fill=\"none\"");
        stroke(&mut svg);
        let f: Vec<f32> = dash.iter().map(|d| *d as f32).collect();
        crate::svg::attr_dash(&mut svg, &f);
        svg.push_str("/>");
    }
    match head {
        ArrowHead::None => {}
        ArrowHead::Open => {
            let (l, r) = (wing(1.0), wing(-1.0));
            svg.push_str("<path class=\"sbmlsim-arrow-head\" d=\"M");
            for (i, p) in [l, b, r].iter().enumerate() {
                if i > 0 {
                    svg.push('L');
                }
                num(&mut svg, p.0);
                svg.push(' ');
                num(&mut svg, p.1);
            }
            svg.push_str("\" fill=\"none\" stroke-linejoin=\"miter\"");
            stroke(&mut svg);
            svg.push_str("/>");
        }
        ArrowHead::Filled => {
            svg.push_str("<path class=\"sbmlsim-arrow-head\"");
            polygon(&mut svg, &[wing(1.0), b, wing(-1.0)]);
            attr_css(&mut svg, "fill", color);
            if lw > 0.0 {
                svg.push_str(" stroke-linejoin=\"miter\"");
                stroke(&mut svg);
            } else {
                attr_opacity(&mut svg, "opacity", opacity);
            }
            svg.push_str("/>");
        }
    }
    Some(raw(e, svg))
}

/// Whether a curve or a polygon is drawn by the crate (since 0.33.0): one in fractions of the
/// panel or not clipped to it.
pub(crate) fn raw_marks(e: &Element) -> bool {
    matches!(
        e.kind,
        ElementKind::Curve { .. } | ElementKind::Polygon { .. }
    ) && (e.coordinates == Coordinates::Axes || !e.clip)
}

/// Writes the `d` of the runs of finite pixels of a line.
fn runs_d(out: &mut String, points: &[(f64, f64)], close: bool) {
    let mut open = false;
    for p in points {
        if !(p.0.is_finite() && p.1.is_finite()) {
            open = false;
            continue;
        }
        out.push(if open { 'L' } else { 'M' });
        num(out, p.0);
        out.push(' ');
        num(out, p.1);
        open = true;
    }
    if close && !points.is_empty() {
        out.push('Z');
    }
}

/// The SVG of a curve or a polygon in fractions of the panel or not clipped (see
/// [`raw_marks`]): the line, the error bars and the markers of a curve, the filled and outlined
/// rings of a polygon, as Charton would draw them.
pub(crate) fn marks_layer(
    e: &Element,
    color: &str,
    px: &Pixels,
    error_width: f64,
) -> Option<RawSvg> {
    use super::marks::{Stroke, error_runs, fill_of, per_pixel};
    let style = &e.style;
    let mut svg = String::new();
    let stroke_attrs = |out: &mut String, s: &Stroke| {
        attr_css(out, "stroke", &s.color);
        attr_num(out, "stroke-width", s.width);
        let f: Vec<f32> = s.dash.iter().map(|d| *d as f32).collect();
        crate::svg::attr_dash(out, &f);
        attr_opacity(out, "opacity", s.opacity);
    };
    match &e.kind {
        ElementKind::Curve {
            x,
            y,
            xerr,
            yerr,
            error_cap,
            ..
        } => {
            let points: Vec<(f64, f64)> = x
                .iter()
                .zip(y)
                .map(|(a, b)| px.at(e.coordinates, *a, *b))
                .collect();
            if let Some(s) = Stroke::of_line(style, color) {
                svg.push_str("<path d=\"");
                runs_d(&mut svg, &points, false);
                svg.push_str("\" fill=\"none\" stroke-linejoin=\"round\"");
                stroke_attrs(&mut svg, &s);
                svg.push_str("/>");
            }
            // the error bars of a curve in values; fractions have none
            if e.coordinates == Coordinates::Data {
                let bar = Stroke {
                    color: color.to_string(),
                    width: error_width,
                    dash: Vec::new(),
                    opacity: style.opacity,
                };
                let xu: Vec<f64> = x.iter().map(|v| px.xa.u(*v)).collect();
                let yu: Vec<f64> = y.iter().map(|v| px.ya.u(*v)).collect();
                let mut runs = Vec::new();
                if let Some(err) = xerr {
                    let cap = error_cap * per_pixel(px.ya, px.h);
                    runs.extend(error_runs(x, err, &yu, px.xa, true, cap, &|_| true));
                }
                if let Some(err) = yerr {
                    let cap = error_cap * per_pixel(px.xa, px.w);
                    runs.extend(error_runs(y, err, &xu, px.ya, false, cap, &|_| true));
                }
                if !runs.is_empty() {
                    svg.push_str("<path d=\"");
                    for (us, vs) in &runs {
                        let pts: Vec<(f64, f64)> =
                            us.iter().zip(vs).map(|(u, v)| px.of_u(*u, *v)).collect();
                        runs_d(&mut svg, &pts, false);
                    }
                    svg.push_str("\" fill=\"none\"");
                    stroke_attrs(&mut svg, &bar);
                    svg.push_str("/>");
                }
            }
            let m = &style.marker;
            if m.kind != crate::spec::MarkerType::None && m.size > 0.0 {
                let g = crate::svg::MarkerGlyph::of(m, color, style.opacity);
                for p in points.iter().filter(|p| p.0.is_finite() && p.1.is_finite()) {
                    crate::svg::glyph(&mut svg, &g, p.0, p.1);
                }
            }
        }
        ElementKind::Polygon { paths } => {
            let (fill, opacity) = fill_of(style, color);
            svg.push_str("<path d=\"");
            for ring in paths {
                let pts: Vec<(f64, f64)> = ring
                    .iter()
                    .map(|[a, b]| px.at(e.coordinates, *a, *b))
                    .collect();
                if pts.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
                    runs_d(&mut svg, &pts, true);
                }
            }
            svg.push('"');
            attr_css(&mut svg, "fill", &fill);
            attr_opacity(&mut svg, "fill-opacity", opacity);
            if let Some(s) = Stroke::of_line(style, color) {
                attr_css(&mut svg, "stroke", &s.color);
                attr_num(&mut svg, "stroke-width", s.width);
                let f: Vec<f32> = s.dash.iter().map(|d| *d as f32).collect();
                crate::svg::attr_dash(&mut svg, &f);
                attr_opacity(&mut svg, "stroke-opacity", s.opacity);
            }
            svg.push_str("/>");
        }
        _ => return None,
    }
    Some(raw(e, svg))
}

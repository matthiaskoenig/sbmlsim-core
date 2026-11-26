//! The marks of bars, boxes, violins, histograms and polygons as layers of Charton.
//!
//! The crate draws every one of them as closed rings (Charton's polygon, a path group per ring)
//! and open lines from the numbers of the spec, and computes no statistics: a box is drawn at
//! the five numbers and outliers it is given, a violin along the density it is given and a
//! histogram over the edges and counts it is given (decision (e) of the spike; Charton's own
//! box, violin and histogram compute their statistics from raw data). Every value becomes the
//! coordinate of its axis (`ResolvedAxis::u`); a part with a value the axis cannot show is left
//! out, never drawn with a NaN.
//!
//! Widths along the axis of the positions are shares of the smallest distance between two
//! positions ([`spacing`]), so a box, a violin or a bar keeps its look on a category axis
//! (positions 0, 1, ...) and on a linear axis of any unit.

use charton::error::ChartonError;
use charton::prelude::*;

use super::axis::ResolvedAxis;
use super::marks::{
    ERROR_WIDTH, Layers, Stroke, dash, dataset, error_runs, fill_of, group_name, path_layer,
    per_pixel,
};
use crate::spec::{BoxStats, Density, MarkerType, Style};
use crate::svg::{LayerInfo, MarkerGlyph, theme};

/// The width of a box, a share of the spacing of the positions (matplotlib's 0.5).
pub(crate) const BOX_WIDTH: f64 = 0.5;
/// The width of a violin at its widest, a share of the spacing of the positions.
pub(crate) const VIOLIN_WIDTH: f64 = 0.8;
/// The width of a bar without a width of its own, a share of the spacing of the positions
/// (matplotlib's 0.8).
pub(crate) const BAR_WIDTH: f64 = 0.8;
/// The width of the edge between the bars of a histogram in pixels.
pub(crate) const HISTOGRAM_EDGE: f64 = 0.8;
/// The diameter of the marker of an outlier of a box in pixels.
pub(crate) const OUTLIER_SIZE: f64 = 6.0;

/// The smallest distance between two distinct finite positions, 1 for fewer than two.
pub(crate) fn spacing(positions: &[f64]) -> f64 {
    let mut p: Vec<f64> = positions
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    p.sort_by(f64::total_cmp);
    let smallest = p
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.0)
        .fold(f64::INFINITY, f64::min);
    if smallest.is_finite() { smallest } else { 1.0 }
}

/// A colour `#rrggbb` with an alpha, for the outline of a polygon, whose opacity Charton fixes
/// at 1.
fn with_alpha(color: &str, alpha: f64) -> SingleColor {
    let channel = |k: usize| {
        color
            .get(1 + 2 * k..3 + 2 * k)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, |b| f64::from(b) / 255.0)
    };
    SingleColor::from_rgba(channel(0), channel(1), channel(2), alpha)
}

/// The closed rings of an element as a polygon layer, a path group per ring in their order.
///
/// A ring of fewer than three points, or with a point which is not finite, is left out. `fill`
/// is the colour and opacity of the inside, `None` for none; `outline` the stroke of the
/// rings, `None` for none.
pub(crate) fn rings_layer(
    rings: &[Vec<(f64, f64)>],
    fill: Option<(&str, f64)>,
    outline: Option<&Stroke>,
) -> Result<Option<Chart<MarkPolygon>>, ChartonError> {
    let (mut xs, mut ys, mut gs) = (Vec::new(), Vec::new(), Vec::new());
    let mut k = 0;
    for ring in rings {
        if ring.len() < 3 || !ring.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
            continue;
        }
        for (x, y) in ring {
            xs.push(*x);
            ys.push(*y);
            gs.push(group_name(k));
        }
        k += 1;
    }
    if xs.is_empty() || (fill.is_none() && outline.is_none()) {
        return Ok(None);
    }
    let ds = dataset(vec![
        ("x", xs.into_column()),
        ("y", ys.into_column()),
        ("g", gs.into_column()),
    ])?;
    let (fill_color, fill_opacity) = match fill {
        Some((c, o)) => (SingleColor::new(c), o),
        None => (SingleColor::none(), 1.0),
    };
    let (stroke, width, dashes) = match outline {
        Some(s) => (with_alpha(&s.color, s.opacity), s.width, s.dash.clone()),
        None => (SingleColor::none(), 0.0, Vec::new()),
    };
    let chart = Chart::build(&ds)?
        .mark_polygon()?
        .configure_polygon(move |p| {
            p.with_fill(fill_color)
                .with_opacity(fill_opacity)
                .with_stroke(stroke)
                .with_stroke_width(width)
                .with_dash(dashes)
        })
        .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    Ok(Some(chart))
}

/// The solid fill of a bar or a bin: its fill, else its colour, at the opacity of the style.
pub(crate) fn solid_fill(style: &Style, color: &str) -> (String, f64) {
    match &style.fill {
        Some(f) => (
            f.color.clone().unwrap_or_else(|| color.to_string()),
            f.opacity * style.opacity,
        ),
        None => (color.to_string(), style.opacity),
    }
}

/// The lines of a box or a violin: the line of the style, a solid line of [`ERROR_WIDTH`] for
/// a style without one, since a box is its lines.
pub(crate) fn outline_of(style: &Style, color: &str) -> Stroke {
    let line = style.line_or_default();
    let width = if line.width > 0.0 {
        line.width
    } else {
        ERROR_WIDTH
    };
    Stroke {
        color: color.to_string(),
        width,
        dash: dash(line.kind, width),
        opacity: style.opacity,
    }
}

/// The coordinate where a bar starts: 0, or the lower end of an axis which cannot show 0.
pub(crate) fn base_of(axis: &ResolvedAxis) -> f64 {
    let zero = axis.u(0.0);
    if zero.is_finite() {
        zero
    } else {
        axis.u(axis.low)
    }
}

/// The coordinates of the ends of the body of a box, from `q1` to `q3`: on a logarithmic axis
/// a `q1` the axis cannot show (zero or negative) is clipped at the start of the axis, as the
/// base of a bar is, so the body still shows from there to `q3`.
pub(crate) fn box_body(axis: &ResolvedAxis, q1: f64, q3: f64) -> (f64, f64) {
    let (bottom, top) = (axis.u(q1), axis.u(q3));
    if bottom.is_nan() && top.is_finite() && q1.is_finite() {
        (base_of(axis), top)
    } else {
        (bottom, top)
    }
}

/// The data of bars.
pub(crate) struct Bars<'a> {
    /// Positions.
    pub x: &'a [f64],
    /// Heights.
    pub height: &'a [f64],
    /// Errors of the heights.
    pub err: Option<&'a [f64]>,
    /// Whether the bars are horizontal.
    pub horizontal: bool,
    /// Width in units of the axis of the positions.
    pub width: Option<f64>,
    /// The length of the caps of the error bars in pixels, 0 for none.
    pub error_cap: f64,
    /// The width and the height of the panel in pixels.
    pub panel: (f64, f64),
}

/// The layers of bars: one ring per bar from 0 to its height, outlined by the line the style
/// gives (none without one), and a line per error.
pub(crate) fn bar_layers(
    layers: &mut Layers,
    index: usize,
    bars: Bars<'_>,
    style: &Style,
    color: &str,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let (along, across) = if bars.horizontal { (ya, xa) } else { (xa, ya) };
    let w = bars.width.unwrap_or(BAR_WIDTH * spacing(bars.x));
    let base = base_of(across);
    let point = |p: f64, v: f64| if bars.horizontal { (v, p) } else { (p, v) };
    let rings: Vec<Vec<(f64, f64)>> = bars
        .x
        .iter()
        .zip(bars.height)
        .map(|(x, h)| {
            let (a, b, top) = (along.u(x - w / 2.0), along.u(x + w / 2.0), across.u(*h));
            vec![point(a, base), point(b, base), point(b, top), point(a, top)]
        })
        .collect();
    let (fill, opacity) = solid_fill(style, color);
    // the outline of the bars: the line of the style when it gives one (since 0.28.0)
    let outline = style.line.and_then(|l| Stroke::of(l, style, color));
    if let Some(l) = rings_layer(&rings, Some((&fill, opacity)), outline.as_ref())? {
        layers.push(l, info(index));
    }
    if let Some(err) = bars.err {
        let centers: Vec<f64> = bars.x.iter().map(|x| along.u(*x)).collect();
        let stroke = Stroke {
            color: theme::FG.to_string(),
            width: ERROR_WIDTH,
            dash: Vec::new(),
            opacity: style.opacity,
        };
        // a cap runs along the axis of the positions
        let pixels = if bars.horizontal {
            bars.panel.1
        } else {
            bars.panel.0
        };
        let cap = bars.error_cap * per_pixel(along, pixels);
        let runs = error_runs(bars.height, err, &centers, across, bars.horizontal, cap);
        if let Some(l) = path_layer(&runs, &stroke, true)? {
            layers.push(l, info(index));
        }
    }
    Ok(())
}

/// What a layer of an element is: a layer of its marks, without a marker.
fn info(index: usize) -> LayerInfo {
    LayerInfo {
        element: Some(index),
        marker: None,
    }
}

/// The layers of box plots from their statistics: the box from the first to the third
/// quartile, the whiskers with their caps, the median across the box and a marker per outlier.
pub(crate) fn box_layers(
    layers: &mut Layers,
    index: usize,
    (positions, stats): (&[f64], &[BoxStats]),
    style: &Style,
    color: &str,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let w = BOX_WIDTH * spacing(positions);
    let (mut rings, mut whiskers, mut medians) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ox, mut oy) = (Vec::new(), Vec::new());
    for (p, b) in positions.iter().zip(stats) {
        let (left, center, right) = (xa.u(p - w / 2.0), xa.u(*p), xa.u(p + w / 2.0));
        let (cap_left, cap_right) = (xa.u(p - w / 4.0), xa.u(p + w / 4.0));
        let [lw, med, hw] = [b.low_whisker, b.median, b.high_whisker].map(|v| ya.u(v));
        let (q1, q3) = box_body(ya, b.q1, b.q3);
        rings.push(vec![(left, q1), (right, q1), (right, q3), (left, q3)]);
        whiskers.push((vec![center, center], vec![lw, q1]));
        whiskers.push((vec![center, center], vec![q3, hw]));
        whiskers.push((vec![cap_left, cap_right], vec![lw, lw]));
        whiskers.push((vec![cap_left, cap_right], vec![hw, hw]));
        medians.push((vec![left, right], vec![med, med]));
        for o in &b.outliers {
            ox.push(center);
            oy.push(ya.u(*o));
        }
    }
    let finite = |runs: Vec<(Vec<f64>, Vec<f64>)>| -> Vec<(Vec<f64>, Vec<f64>)> {
        runs.into_iter()
            .filter(|(x, y)| x.iter().chain(y).all(|v| v.is_finite()))
            .collect()
    };
    let stroke = outline_of(style, color);
    let (fill, opacity) = fill_of(style, color);
    if let Some(l) = rings_layer(&rings, Some((&fill, opacity)), Some(&stroke))? {
        layers.push(l, info(index));
    }
    let solid = Stroke {
        dash: Vec::new(),
        ..stroke.clone()
    };
    if let Some(l) = path_layer(&finite(whiskers), &solid, true)? {
        layers.push(l, info(index));
    }
    if let Some(l) = path_layer(&finite(medians), &solid, true)? {
        layers.push(l, info(index));
    }
    let (ox, oy): (Vec<f64>, Vec<f64>) = ox
        .into_iter()
        .zip(oy)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .unzip();
    if !ox.is_empty() {
        let m = &style.marker;
        let glyph = if m.kind == MarkerType::None {
            MarkerGlyph {
                kind: MarkerType::Circle,
                size: OUTLIER_SIZE,
                fill: "none".to_string(),
                edge: color.to_string(),
                edge_width: ERROR_WIDTH,
                opacity: style.opacity,
            }
        } else {
            MarkerGlyph {
                kind: m.kind,
                size: m.size,
                fill: m.fill.clone().unwrap_or_else(|| "none".to_string()),
                edge: m.edge.clone().unwrap_or_else(|| color.to_string()),
                edge_width: m.edge_width,
                opacity: style.opacity,
            }
        };
        let ds = dataset(vec![("x", ox.into_column()), ("y", oy.into_column())])?;
        let radius = glyph.size / 2.0;
        let chart = Chart::build(&ds)?
            .mark_point()?
            .configure_point(move |p| p.with_size(radius))
            .encode((alt::x("x"), alt::y("y")))?;
        layers.push(
            chart,
            LayerInfo {
                element: Some(index),
                marker: Some(glyph),
            },
        );
    }
    Ok(())
}

/// The layers of violins from their densities: one closed outline per position, the density
/// to the right and mirrored to the left, its widest point [`VIOLIN_WIDTH`] of the spacing of
/// the positions wide.
pub(crate) fn violin_layers(
    layers: &mut Layers,
    index: usize,
    (positions, densities): (&[f64], &[Density]),
    style: &Style,
    color: &str,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let half = VIOLIN_WIDTH * spacing(positions) / 2.0;
    let mut rings = Vec::new();
    for (p, d) in positions.iter().zip(densities) {
        let points: Vec<(f64, f64)> = d
            .values
            .iter()
            .zip(&d.density)
            .filter(|(v, w)| v.is_finite() && w.is_finite() && **w >= 0.0)
            .map(|(v, w)| (*v, *w))
            .collect();
        let widest = points.iter().map(|(_, w)| *w).fold(0.0, f64::max);
        if points.len() < 2 || widest <= 0.0 {
            continue;
        }
        let side = |sign: f64, (v, w): (f64, f64)| (xa.u(p + sign * half * w / widest), ya.u(v));
        let mut ring: Vec<(f64, f64)> = points.iter().map(|q| side(1.0, *q)).collect();
        ring.extend(points.iter().rev().map(|q| side(-1.0, *q)));
        rings.push(ring);
    }
    let (fill, opacity) = fill_of(style, color);
    let stroke = outline_of(style, color);
    if let Some(l) = rings_layer(&rings, Some((&fill, opacity)), Some(&stroke))? {
        layers.push(l, info(index));
    }
    Ok(())
}

/// The layers of a histogram from its bins: one ring per bin of a count which is not zero, from
/// 0 to its count, with an edge of the background between neighbouring bins.
pub(crate) fn histogram_layers(
    layers: &mut Layers,
    index: usize,
    (edges, counts): (&[f64], &[f64]),
    style: &Style,
    color: &str,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let base = base_of(ya);
    let rings: Vec<Vec<(f64, f64)>> = counts
        .iter()
        .zip(edges.windows(2))
        .filter(|(c, _)| **c != 0.0)
        .map(|(c, e)| {
            let (a, b, top) = (xa.u(e[0]), xa.u(e[1]), ya.u(*c));
            vec![(a, base), (b, base), (b, top), (a, top)]
        })
        .collect();
    let (fill, opacity) = solid_fill(style, color);
    let edge = Stroke {
        color: theme::BACKGROUND.to_string(),
        width: HISTOGRAM_EDGE,
        dash: Vec::new(),
        opacity: 1.0,
    };
    if let Some(l) = rings_layer(&rings, Some((&fill, opacity)), Some(&edge))? {
        layers.push(l, info(index));
    }
    Ok(())
}

/// The layers of polygons: one filled ring per path, outlined by the line of the style.
pub(crate) fn polygon_layers(
    layers: &mut Layers,
    index: usize,
    paths: &[Vec<[f64; 2]>],
    style: &Style,
    color: &str,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let rings: Vec<Vec<(f64, f64)>> = paths
        .iter()
        .map(|ring| ring.iter().map(|[x, y]| (xa.u(*x), ya.u(*y))).collect())
        .collect();
    let (fill, opacity) = fill_of(style, color);
    let stroke = Stroke::of_line(style, color);
    if let Some(l) = rings_layer(&rings, Some((&fill, opacity)), stroke.as_ref())? {
        layers.push(l, info(index));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_of_positions() {
        assert_eq!(spacing(&[0.0, 1.0, 2.0]), 1.0);
        assert_eq!(spacing(&[10.0, 0.0, 30.0]), 10.0);
        assert_eq!(spacing(&[5.0]), 1.0);
        assert_eq!(spacing(&[]), 1.0);
        assert_eq!(spacing(&[2.0, 2.0, f64::NAN]), 1.0);
    }
}

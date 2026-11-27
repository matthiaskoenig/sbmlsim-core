//! Contours of a grid of values (since 0.33.0): lines at levels with their labels inline, and
//! the bands between levels filled, as matplotlib's `contour`, `clabel` and `contourf` draw
//! them.
//!
//! The grid is given by the coordinates of its columns along x and of its rows along y; a
//! cell of four values with every value a number is cut by marching squares, the crossing of a
//! level on an edge of a cell interpolated linearly in the values of the axes between its ends,
//! as contourpy does, so a level line and the edge of a band of the same level coincide, and
//! the crossings of neighbouring cells are the same points. A saddle (two opposite corners
//! above a level and two below) is resolved by the mean of its four values. The segments of a
//! level are joined into lines over the edges they share.
//!
//! A band from one level to the next covers every part of a cell whose value lies between
//! them, the cell clipped by both levels along its edges; every cell of a band is one subpath
//! of one path of the band, all of one orientation, so neighbouring cells meet without a seam
//! (a run of whole cells of a row is one rectangle). The value of a band is its middle, the
//! colour of a line the one of its level, as matplotlib's colours of a `ContourSet`.
//!
//! The labels of the lines follow matplotlib's `clabel` with `inline=True` on the lines in the
//! pixels of the panel: a line holds a label when it has more than ten label widths of vertices
//! or reaches over more than 1.2 label widths along an axis; the label sits in the middle of
//! the block of a label width of vertices which runs straightest, not nearer to a label placed
//! before than 1.2 label widths (else the straightest), rotated along the line and never upside
//! down, and the line is broken from half a label width and 5 pixels before it to as far after
//! it.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::marks::{Pixels, Stroke, dash};
use super::rich::{self, TextLook};
use crate::colors::color_of;
use crate::spec::{
    Anchor, ColorScale, FontFamily, FontWeight, Fonts, LineType, Style, VerticalAnchor,
};
use crate::svg::{RawSvg, attr_css, attr_dash, attr_num, attr_opacity, num};
use crate::text::text_width_weighted;

/// The pixels a broken line keeps free beside a label (matplotlib's `inline_spacing`).
const INLINE_SPACING: f64 = 5.0;

/// The data of a contour element.
pub(crate) struct Grid<'a> {
    /// The coordinates of the columns along x.
    pub x: &'a [f64],
    /// The coordinates of the rows along y.
    pub y: &'a [f64],
    /// The values, per row.
    pub values: &'a [Vec<f64>],
}

impl Grid<'_> {
    fn value(&self, i: usize, j: usize) -> f64 {
        self.values
            .get(i)
            .and_then(|r| r.get(j))
            .copied()
            .unwrap_or(f64::NAN)
    }
}

/// An edge of the grid: horizontal from `(i, j)` to `(i, j + 1)`, or vertical from `(i, j)` to
/// `(i + 1, j)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Edge {
    H(usize, usize),
    V(usize, usize),
}

/// The point a level crosses an edge, in the values of the axes.
fn crossing(g: &Grid, e: Edge, level: f64) -> (f64, f64) {
    let ((i0, j0), (i1, j1)) = match e {
        Edge::H(i, j) => ((i, j), (i, j + 1)),
        Edge::V(i, j) => ((i, j), (i + 1, j)),
    };
    let (a, b) = (g.value(i0, j0), g.value(i1, j1));
    let t = if b != a { (level - a) / (b - a) } else { 0.5 };
    let t = t.clamp(0.0, 1.0);
    (
        g.x[j0] + t * (g.x[j1] - g.x[j0]),
        g.y[i0] + t * (g.y[i1] - g.y[i0]),
    )
}

/// The lines of a level, each a list of points in the values of the axes; a closed line ends at
/// its first point.
pub(crate) fn level_lines(g: &Grid, level: f64) -> Vec<Vec<(f64, f64)>> {
    let (m, n) = (g.y.len(), g.x.len());
    let mut segments: Vec<(Edge, Edge)> = Vec::new();
    for i in 0..m.saturating_sub(1) {
        for j in 0..n.saturating_sub(1) {
            let v = [
                g.value(i, j),
                g.value(i, j + 1),
                g.value(i + 1, j + 1),
                g.value(i + 1, j),
            ];
            if v.iter().any(|x| !x.is_finite()) {
                continue;
            }
            // the corners above the level, counterclockwise from (i, j), as contourpy's `z >
            // level` (since 0.34.0; at or above before): a level at the least value of the data
            // draws the edge of the region of that value, one at the largest value nothing
            let up = v.map(|x| x > level);
            // the edges of the cell in the order of its corners: bottom, right, top, left
            let edges = [
                Edge::H(i, j),
                Edge::V(i, j + 1),
                Edge::H(i + 1, j),
                Edge::V(i, j),
            ];
            let cut: Vec<usize> = (0..4).filter(|&k| up[k] != up[(k + 1) % 4]).collect();
            // a segment cuts off the corner between two edges; it runs with the corners at or
            // above the level on its left, as contourpy orients its lines
            let cut_corner = |segments: &mut Vec<(Edge, Edge)>, a: usize| {
                let (from, to) = (a, (a + 1) % 4);
                if up[to] {
                    segments.push((edges[to], edges[from]));
                } else {
                    segments.push((edges[from], edges[to]));
                }
            };
            match cut.len() {
                2 => {
                    // the corners between the two cut edges, counterclockwise
                    let (c0, c1) = (cut[0], cut[1]);
                    if up[(c0 + 1) % 4] {
                        segments.push((edges[c1], edges[c0]));
                    } else {
                        segments.push((edges[c0], edges[c1]));
                    }
                }
                4 => {
                    // a saddle: the mean decides which corners are cut off
                    let centre_up = (v[0] + v[1] + v[2] + v[3]) / 4.0 > level;
                    if centre_up == up[0] {
                        // corners 1 and 3 are cut off
                        cut_corner(&mut segments, 0);
                        cut_corner(&mut segments, 2);
                    } else {
                        // corners 0 and 2 are cut off
                        cut_corner(&mut segments, 3);
                        cut_corner(&mut segments, 1);
                    }
                }
                _ => {}
            }
        }
    }
    // join the segments, each starting where the one before it ends
    let mut from: HashMap<Edge, usize> = HashMap::new();
    let mut into: HashMap<Edge, usize> = HashMap::new();
    for (k, (a, b)) in segments.iter().enumerate() {
        from.insert(*a, k);
        into.insert(*b, k);
    }
    let mut used = vec![false; segments.len()];
    let mut lines: Vec<(usize, Vec<Edge>)> = Vec::new();
    // an open line from its first segment, which no segment leads into
    for k in 0..segments.len() {
        if used[k] || into.contains_key(&segments[k].0) {
            continue;
        }
        let mut edges = vec![segments[k].0];
        let mut next = Some(k);
        while let Some(s) = next.filter(|s| !used[*s]) {
            used[s] = true;
            edges.push(segments[s].1);
            next = from.get(&segments[s].1).copied();
        }
        lines.push((k, edges));
    }
    // a closed line from the end of its first segment in the order of the cells, as contourpy
    for k in 0..segments.len() {
        if used[k] {
            continue;
        }
        used[k] = true;
        let start = segments[k].1;
        let mut edges = vec![start];
        let mut next = from.get(&start).copied();
        while let Some(s) = next.filter(|s| !used[*s]) {
            used[s] = true;
            edges.push(segments[s].1);
            next = from.get(&segments[s].1).copied();
        }
        if edges.last() != Some(&start) {
            edges.push(start);
        }
        lines.push((k, edges));
    }
    lines.sort_by_key(|(k, _)| *k);
    lines
        .into_iter()
        .map(|(_, edges)| edges.iter().map(|e| crossing(g, *e, level)).collect())
        .collect()
}

/// The part of a polygon with values at its corners where `keep` holds of the value, cut at
/// `level` along its edges, with the values at the new corners.
fn clip_by(
    points: &[(f64, f64, f64)],
    level: f64,
    keep: impl Fn(f64) -> bool,
) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::with_capacity(points.len() + 2);
    for k in 0..points.len() {
        let (a, b) = (points[k], points[(k + 1) % points.len()]);
        if keep(a.2) {
            out.push(a);
        }
        if keep(a.2) != keep(b.2) {
            let t = if b.2 != a.2 {
                ((level - a.2) / (b.2 - a.2)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            out.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1), level));
        }
    }
    out
}

/// The rings of a band above `lo` up to `hi` (`lo` included for the first band, as
/// matplotlib's intervals of `contourf`), in the values of the axes: per row the runs of whole
/// cells as rectangles, the cells it cuts clipped.
pub(crate) fn band_rings(g: &Grid, lo: f64, hi: f64, first: bool) -> Vec<Vec<(f64, f64)>> {
    let (m, n) = (g.y.len(), g.x.len());
    let above = move |v: f64| if first { v >= lo } else { v > lo };
    let within = move |v: f64| above(v) && v <= hi;
    let mut rings = Vec::new();
    for i in 0..m.saturating_sub(1) {
        let mut run: Option<usize> = None;
        let close = |rings: &mut Vec<Vec<(f64, f64)>>, from: usize, to: usize| {
            rings.push(vec![
                (g.x[from], g.y[i]),
                (g.x[to], g.y[i]),
                (g.x[to], g.y[i + 1]),
                (g.x[from], g.y[i + 1]),
            ]);
        };
        for j in 0..n.saturating_sub(1) {
            let corners = [(i, j), (i, j + 1), (i + 1, j + 1), (i + 1, j)];
            let v = corners.map(|(a, b)| g.value(a, b));
            if v.iter().any(|x| !x.is_finite()) {
                if let Some(from) = run.take() {
                    close(&mut rings, from, j);
                }
                continue;
            }
            if v.iter().all(|x| within(*x)) {
                run.get_or_insert(j);
                continue;
            }
            if let Some(from) = run.take() {
                close(&mut rings, from, j);
            }
            let quad: Vec<(f64, f64, f64)> = corners
                .iter()
                .zip(v)
                .map(|(&(a, b), z)| (g.x[b], g.y[a], z))
                .collect();
            let part = clip_by(&quad, lo, above);
            let part = clip_by(&part, hi, |v| v <= hi);
            if part.len() >= 3 {
                rings.push(part.iter().map(|p| (p.0, p.1)).collect());
            }
        }
        if let Some(from) = run {
            close(&mut rings, from, n - 1);
        }
    }
    rings
}

/// What a contour element draws besides its grid.
pub(crate) struct Look<'a> {
    /// The levels, increasing.
    pub levels: &'a [f64],
    /// Whether the bands are filled.
    pub filled: bool,
    /// The colours of the levels or bands.
    pub scale: Option<&'a ColorScale>,
    /// The labels of the levels, inline on the lines.
    pub labels: Option<&'a [String]>,
    /// The font size of the labels.
    pub label_size: Option<f64>,
    /// The style of the element.
    pub style: &'a Style,
    /// The colour of the element.
    pub color: &'a str,
}

/// Writes the `d` of the runs of finite pixels of a line.
fn line_d(out: &mut String, points: &[(f64, f64)]) {
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
}

/// The SVG of a contour element in the pixels of its panel.
pub(crate) fn contour_svg(g: &Grid, look: &Look, fonts: &Fonts, px: &Pixels) -> RawSvg {
    let to_px = |(x, y): (f64, f64)| px.of_u(px.xa.u(x), px.ya.u(y));
    let mut out = String::new();
    let levels = look.levels;
    if look.filled {
        let n = levels.len().saturating_sub(1);
        for k in 0..n {
            let (lo, hi) = (levels[k], levels[k + 1]);
            let rings = band_rings(g, lo, hi, k == 0);
            if rings.is_empty() {
                continue;
            }
            let color = match look.scale {
                Some(s) => color_of(s, (lo + hi) / 2.0),
                None => look.color.to_string(),
            };
            // without antialiasing, as matplotlib's `contourf`, so bands meet without seams
            out.push_str(
                "<path class=\"sbmlsim-contour-band\" shape-rendering=\"crispEdges\" d=\"",
            );
            for ring in &rings {
                let pts: Vec<(f64, f64)> = ring.iter().map(|p| to_px(*p)).collect();
                if pts.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
                    line_d(&mut out, &pts);
                    out.push('Z');
                }
            }
            out.push('"');
            attr_css(&mut out, "fill", &color);
            attr_opacity(&mut out, "fill-opacity", look.style.opacity);
            out.push_str("/>");
        }
        return RawSvg {
            clipped: out,
            unclipped: String::new(),
        };
    }
    let line = look.style.line_or_default();
    let mut labels_svg = String::new();
    let mut placed: Vec<(f64, f64)> = Vec::new();
    for (k, level) in levels.iter().enumerate() {
        let color = match look.scale {
            Some(s) => color_of(s, *level),
            None => look.color.to_string(),
        };
        let lines: Vec<Vec<(f64, f64)>> = level_lines(g, *level)
            .into_iter()
            .map(|l| l.into_iter().map(to_px).collect())
            .collect();
        let label = look.labels.and_then(|l| l.get(k)).filter(|l| !l.is_empty());
        let size = look.label_size.unwrap_or(fonts.tick);
        let mut kept: Vec<Vec<(f64, f64)>> = Vec::new();
        for pts in lines {
            let Some(label) = label else {
                kept.push(pts);
                continue;
            };
            let lw = text_width_weighted(label, size, FontWeight::Normal);
            match place_label(&pts, lw, &placed) {
                Some((at, angle, parts)) => {
                    placed.push(at);
                    kept.extend(parts);
                    let look_text = TextLook {
                        size,
                        weight: FontWeight::Normal,
                        family: FontFamily::Sans,
                        color: color.clone(),
                        opacity: look.style.opacity,
                        anchor: Anchor::Middle,
                        baseline: VerticalAnchor::Center,
                        rotation: angle,
                    };
                    labels_svg.push_str("<g class=\"sbmlsim-contour-label\">");
                    rich::write_text(&mut labels_svg, label, at, &look_text, None);
                    labels_svg.push_str("</g>");
                }
                None => kept.push(pts),
            }
        }
        if line.kind == LineType::None || line.width <= 0.0 || kept.is_empty() {
            continue;
        }
        let stroke = Stroke {
            color,
            width: line.width,
            dash: dash(line.kind, line.width),
            opacity: look.style.opacity,
        };
        out.push_str("<path class=\"sbmlsim-contour-line\" d=\"");
        for pts in &kept {
            line_d(&mut out, pts);
        }
        out.push_str("\" fill=\"none\" stroke-linejoin=\"round\"");
        attr_css(&mut out, "stroke", &stroke.color);
        attr_num(&mut out, "stroke-width", stroke.width);
        let f: Vec<f32> = stroke.dash.iter().map(|d| *d as f32).collect();
        attr_dash(&mut out, &f);
        attr_opacity(&mut out, "opacity", stroke.opacity);
        let _ = write!(out, " data-level=\"{level}\"");
        out.push_str("/>");
    }
    out.push_str(&labels_svg);
    RawSvg {
        clipped: out,
        unclipped: String::new(),
    }
}

/// A label placed on a line: its point, its angle and the parts of the line beside it.
type LabelPlace = ((f64, f64), f64, Vec<Vec<(f64, f64)>>);

/// The place of a label of `lw` pixels on a line of pixels, matplotlib's `print_label`,
/// `locate_label` and `_split_path_and_get_label_rotation`: the point, the angle in degrees
/// counterclockwise and the parts of the line left beside the label; `None` for a line too
/// short to hold it.
fn place_label(pts: &[(f64, f64)], lw: f64, placed: &[(f64, f64)]) -> Option<LabelPlace> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in pts {
        x0 = x0.min(p.0);
        x1 = x1.max(p.0);
        y0 = y0.min(p.1);
        y1 = y1.max(p.1);
    }
    if !(n as f64 > 10.0 * lw || x1 - x0 > 1.2 * lw || y1 - y0 > 1.2 * lw) {
        return None;
    }
    // the blocks of a label width of vertices, the last one filled from the start
    let blocks = if lw > 1.0 {
        (n as f64 / lw).ceil() as usize
    } else {
        1
    };
    let size = if blocks == 1 { n } else { lw as usize };
    let size = size.max(1);
    let point = |b: usize, k: usize| pts[(b * size + k) % n];
    let mut distances: Vec<(f64, usize)> = (0..blocks)
        .map(|b| {
            let (f, l) = (point(b, 0), point(b, size - 1));
            let len = (l.0 - f.0).hypot(l.1 - f.1);
            let d: f64 = (0..size)
                .map(|k| {
                    let p = point(b, k);
                    let s = (f.1 - p.1) * (l.0 - f.0) - (f.0 - p.0) * (l.1 - f.1);
                    if len > 0.0 { s.abs() / len } else { f64::NAN }
                })
                .sum();
            (d, b)
        })
        .collect();
    // NaN sorts last as numpy's argsort; a stable sort keeps the order of ties
    distances.sort_by(|a, b| match (a.0.is_nan(), b.0.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => a.0.total_cmp(&b.0),
    });
    let half = size / 2;
    let too_close = |p: (f64, f64)| {
        placed
            .iter()
            .any(|q| (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2) < (1.2 * lw).powi(2))
    };
    let mut chosen = distances[0].1;
    for (_, b) in &distances {
        if !too_close(point(*b, half)) {
            chosen = *b;
            break;
        }
    }
    let idx = (chosen * size + half) % n;
    let at = pts[idx];
    // the line as a curve of its length from the label; a closed line starts at the label
    let closed = n > 2 && pts[0] == pts[n - 1];
    let line: Vec<(f64, f64)> = if closed {
        pts[idx..n - 1]
            .iter()
            .chain(&pts[..=idx])
            .copied()
            .collect()
    } else {
        pts.to_vec()
    };
    let idx = if closed { 0 } else { idx };
    let mut cum = vec![0.0];
    for w in line.windows(2) {
        let d = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        cum.push(cum.last().copied().unwrap_or(0.0) + d);
    }
    let base = cum[idx];
    let cpl: Vec<f64> = cum.iter().map(|c| c - base).collect();
    let total = cpl[cpl.len() - 1] - cpl[0];
    let interp = |t: f64| -> Option<(f64, f64)> {
        if t < cpl[0] || t > cpl[cpl.len() - 1] {
            // np.interp holds the ends
            let p = if t < cpl[0] {
                line[0]
            } else {
                line[line.len() - 1]
            };
            return Some(p);
        }
        let k = cpl.partition_point(|c| *c < t).max(1).min(cpl.len() - 1);
        let (a, b) = (cpl[k - 1], cpl[k]);
        let s = if b > a { (t - a) / (b - a) } else { 0.0 };
        let (p, q) = (line[k - 1], line[k]);
        Some((p.0 + s * (q.0 - p.0), p.1 + s * (q.1 - p.1)))
    };
    let mut targets = [-lw / 2.0, lw / 2.0];
    if closed {
        targets[0] += total;
    }
    let (s0, s1) = (interp(targets[0])?, interp(targets[1])?);
    // the angle in the screen of matplotlib (y up), never upside down
    let angle = (-(s1.1 - s0.1)).atan2(s1.0 - s0.0).to_degrees();
    let angle = (angle + 90.0).rem_euclid(180.0) - 90.0;
    let (t0, t1) = (targets[0] - INLINE_SPACING, targets[1] + INLINE_SPACING);
    let index_at = |t: f64| -> Option<f64> {
        if t < cpl[0] || t > cpl[cpl.len() - 1] {
            return None;
        }
        let k = cpl.partition_point(|c| *c < t).max(1).min(cpl.len() - 1);
        let (a, b) = (cpl[k - 1], cpl[k]);
        Some((k - 1) as f64 + if b > a { (t - a) / (b - a) } else { 0.0 })
    };
    let (i0, i1) = (index_at(t0), index_at(t1));
    let (p0, p1) = (interp(t0)?, interp(t1)?);
    let mut parts = Vec::new();
    if closed {
        if let (Some(i0), Some(i1)) = (i0, i1) {
            let (i0, i1) = (i0.floor() as usize, i1.ceil() as usize);
            let mut part = vec![p1];
            if i1 <= i0 {
                part.extend_from_slice(&line[i1..=i0]);
            }
            part.push(p0);
            parts.push(part);
        }
    } else {
        if let Some(i0) = i0 {
            let i0 = i0.floor() as usize;
            let mut part = line[..=i0].to_vec();
            part.push(p0);
            parts.push(part);
        }
        if let Some(i1) = i1 {
            let i1 = i1.ceil() as usize;
            let mut part = vec![p1];
            part.extend_from_slice(&line[i1.min(line.len())..]);
            parts.push(part);
        }
    }
    parts.retain(|p| p.len() >= 2);
    Some((at, angle, parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(values: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
        (
            (0..values[0].len()).map(|j| j as f64).collect(),
            (0..values.len()).map(|i| i as f64).collect(),
        )
    }

    #[test]
    fn a_peak_has_a_closed_line() {
        let values = vec![
            vec![0.0, 0.0, 0.0],
            vec![0.0, 2.0, 0.0],
            vec![0.0, 0.0, 0.0],
        ];
        let (x, y) = grid(&values);
        let g = Grid {
            x: &x,
            y: &y,
            values: &values,
        };
        let lines = level_lines(&g, 1.0);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.first(), l.last(), "{l:?}");
        // the crossings half way between the peak and its neighbours
        assert!(l.contains(&(0.5, 1.0)) && l.contains(&(1.0, 1.5)), "{l:?}");
    }

    /// The lines of matplotlib 3.11 (contourpy) of this grid at its least value, at 1 and at
    /// its largest value, their repeated points (a corner at the level) left out.
    #[test]
    fn levels_at_the_ends_of_the_data_as_matplotlib() {
        let values = vec![
            vec![0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 2.0],
            vec![0.0, 1.0, 2.0, 3.0],
            vec![1.0, 2.0, 3.0, 4.0],
        ];
        let (x, y) = grid(&values);
        let g = Grid {
            x: &x,
            y: &y,
            values: &values,
        };
        let lines = |level: f64| -> Vec<Vec<(f64, f64)>> {
            level_lines(&g, level)
                .into_iter()
                .map(|mut l| {
                    l.dedup();
                    l
                })
                .collect()
        };
        assert_eq!(
            lines(0.0),
            vec![vec![(0.0, 2.0), (1.0, 1.0), (2.0, 0.0), (3.0, 0.0)]]
        );
        assert_eq!(
            lines(1.0),
            vec![vec![(0.0, 3.0), (1.0, 2.0), (2.0, 1.0), (3.0, 0.5)]]
        );
        assert!(lines(4.0).is_empty());
    }

    #[test]
    fn bands_cover_the_cells() {
        // a ramp from 0 to 3 along x: the band 1..2 is the strip from x = 1 to x = 2
        let values = vec![vec![0.0, 1.0, 2.0, 3.0], vec![0.0, 1.0, 2.0, 3.0]];
        let (x, y) = grid(&values);
        let g = Grid {
            x: &x,
            y: &y,
            values: &values,
        };
        let area = |rings: &[Vec<(f64, f64)>]| -> f64 {
            rings
                .iter()
                .map(|r| {
                    let n = r.len();
                    (0..n)
                        .map(|k| r[k].0 * r[(k + 1) % n].1 - r[(k + 1) % n].0 * r[k].1)
                        .sum::<f64>()
                        / 2.0
                })
                .sum()
        };
        let middle = band_rings(&g, 1.0, 2.0, false);
        assert!((area(&middle) - 1.0).abs() < 1e-12, "{middle:?}");
        let whole: f64 = [(0.0, 1.0, true), (1.0, 2.0, false), (2.0, 3.0, false)]
            .iter()
            .map(|(lo, hi, last)| area(&band_rings(&g, *lo, *hi, *last)))
            .sum();
        assert!((whole - 3.0).abs() < 1e-12);
    }
}

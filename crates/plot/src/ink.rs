//! The data of a plot as matplotlib's `best` legend placement sees it (since 0.33.0; the area
//! of ink under a frame before): the lines and the boxes of its artists in the pixels of its
//! panel, from which the badness of a frame is counted as matplotlib counts it
//! (`Legend._find_best_position`).
//!
//! matplotlib collects the vertices of the path of every `Line2D` (a curve, with markers alone
//! too, the caps of its error bars, a rule, the lines of a box), the path of every other
//! patch and of every `PolyCollection` (a band, an area, a violin, a polygon, an arrow), the
//! bounding box of every `Rectangle` (a bar, a bin of a histogram) and of every text, and the
//! offsets of other collections (none of the kinds of the spec: a heatmap and the contours have
//! none). The badness of a frame is the number of vertices strictly inside it, of the boxes
//! which overlap it, and of the lines with a segment which meets it. A vertex the axes cannot
//! show (NaN) counts nothing and breaks its line. The sums run in the order of the elements, so
//! the same figure gives the same placement natively and in wasm.

use crate::layout::Rect;
use crate::render::Prepared;
use crate::render::axis::ResolvedAxis;
use crate::render::rich::{self, TextLook};
use crate::render::stats_marks::{BAR_WIDTH, VIOLIN_WIDTH, box_body, box_width, spacing, tops};
use crate::spec::{Coordinates, ElementKind, Fonts, Orientation, Plot, VerticalAnchor, YAxisRef};

/// The data of a panel in its pixels, from its upper left corner.
#[derive(Debug, Default)]
pub(crate) struct Ink {
    /// The paths of lines and patches: their vertices, a NaN vertex a break.
    lines: Vec<Vec<(f64, f64)>>,
    /// The boxes of rectangles and texts: left, top, right, bottom.
    boxes: Vec<(f64, f64, f64, f64)>,
}

impl Ink {
    /// The data of a plot in its panel of `width` by `height` pixels.
    pub(crate) fn of_plot(
        plot: &Plot,
        prep: &Prepared,
        fonts: &Fonts,
        (width, height): (f64, f64),
    ) -> Self {
        let mut ink = Ink::default();
        for e in plot
            .elements
            .iter()
            .filter(|e| e.coordinates == Coordinates::Data)
        {
            let ya = match (e.axis, &prep.y2a) {
                (YAxisRef::Y2, Some(a)) => a,
                _ => &prep.ya,
            };
            ink.element(&e.kind, fonts, (&prep.xa, ya), (width, height));
        }
        ink
    }

    /// The badness of a frame, as matplotlib's: the vertices inside it, the boxes overlapping
    /// it and the lines meeting it.
    pub(crate) fn badness(&self, r: &Rect) -> usize {
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
        let inside = |p: &(f64, f64)| x0 < p.0 && p.0 < x1 && y0 < p.1 && p.1 < y1;
        let vertices: usize = self
            .lines
            .iter()
            .map(|l| l.iter().filter(|p| inside(p)).count())
            .sum();
        let boxes = self
            .boxes
            .iter()
            .filter(|b| !(b.2 <= x0 || b.0 >= x1 || b.3 <= y0 || b.1 >= y1))
            .count();
        let ok = |p: &(f64, f64)| p.0.is_finite() && p.1.is_finite();
        let crossed = self
            .lines
            .iter()
            .filter(|l| {
                l.windows(2)
                    .any(|w| ok(&w[0]) && ok(&w[1]) && segment_meets(w[0], w[1], r))
            })
            .count();
        vertices + boxes + crossed
    }

    /// Adds the data of an element.
    fn element(
        &mut self,
        kind: &ElementKind,
        fonts: &Fonts,
        (xa, ya): (&ResolvedAxis, &ResolvedAxis),
        (w, h): (f64, f64),
    ) {
        let pu = move |u: f64, v: f64| (w * xa.share(u), h * (1.0 - ya.share(v)));
        let at = move |x: f64, y: f64| pu(xa.u(x), ya.u(y));
        match kind {
            ElementKind::Curve {
                x,
                y,
                xerr,
                yerr,
                error_cap,
                ..
            } => {
                self.lines
                    .push(x.iter().zip(y).map(|(a, b)| at(*a, *b)).collect());
                // the caps of the error bars are lines of markers at the ends
                if *error_cap > 0.0 {
                    for (err, along_x) in [(xerr, true), (yerr, false)] {
                        let Some(err) = err else { continue };
                        let (mut lo, mut hi) = (Vec::new(), Vec::new());
                        for (i, (a, b)) in x.iter().zip(y).enumerate() {
                            let (el, eh) = err.at(i);
                            let end = |e: f64, sign: f64| {
                                if along_x {
                                    at(a + sign * e, *b)
                                } else {
                                    at(*a, b + sign * e)
                                }
                            };
                            lo.push(end(el, -1.0));
                            hi.push(end(eh, 1.0));
                        }
                        self.lines.push(lo);
                        self.lines.push(hi);
                    }
                }
            }
            ElementKind::Band {
                x,
                low,
                high,
                median,
            } => {
                self.lines.push(between(x, low, high, &at));
                if let Some(m) = median {
                    self.lines
                        .push(x.iter().zip(m).map(|(a, b)| at(*a, *b)).collect());
                }
            }
            ElementKind::Area { x, from, to } => self.lines.push(between(x, from, to, &at)),
            ElementKind::Bar {
                x,
                height,
                horizontal,
                width,
                base,
                ..
            } => {
                let bw = width.unwrap_or(BAR_WIDTH * spacing(x));
                let tops = tops(height, base.as_deref());
                for (k, p) in x.iter().enumerate() {
                    let b = base.as_ref().map_or(0.0, |b| b[k]);
                    let (a, c) = if *horizontal {
                        (at(b, p - bw / 2.0), at(tops[k], p + bw / 2.0))
                    } else {
                        (at(p - bw / 2.0, b), at(p + bw / 2.0, tops[k]))
                    };
                    self.rect(a, c);
                }
            }
            ElementKind::Histogram {
                edges,
                counts,
                base,
                ..
            } => {
                let tops = tops(counts, base.as_deref());
                for (k, e) in edges.windows(2).enumerate() {
                    if counts[k] == 0.0 {
                        continue;
                    }
                    let b = base.as_ref().map_or(0.0, |b| b[k]);
                    self.rect(at(e[0], b), at(e[1], tops[k]));
                }
            }
            ElementKind::Box {
                positions,
                stats,
                width,
                ..
            } => {
                let bw = box_width(positions, *width);
                for (p, s) in positions.iter().zip(stats) {
                    let (l, c, r) = (xa.u(p - bw / 2.0), xa.u(*p), xa.u(p + bw / 2.0));
                    let (cl, cr) = (xa.u(p - bw / 4.0), xa.u(p + bw / 4.0));
                    let (q1, q3) = box_body(ya, s.q1, s.q3);
                    let [lw, med, hw] = [s.low_whisker, s.median, s.high_whisker].map(|v| ya.u(v));
                    self.lines
                        .push(vec![pu(l, q1), pu(r, q1), pu(r, q3), pu(l, q3), pu(l, q1)]);
                    self.lines.push(vec![pu(c, q1), pu(c, lw)]);
                    self.lines.push(vec![pu(c, q3), pu(c, hw)]);
                    self.lines.push(vec![pu(cl, lw), pu(cr, lw)]);
                    self.lines.push(vec![pu(cl, hw), pu(cr, hw)]);
                    self.lines.push(vec![pu(l, med), pu(r, med)]);
                    self.lines
                        .push(s.outliers.iter().map(|o| pu(c, ya.u(*o))).collect());
                }
            }
            ElementKind::Violin {
                positions,
                densities,
            } => {
                let half = VIOLIN_WIDTH * spacing(positions) / 2.0;
                for (p, d) in positions.iter().zip(densities) {
                    let widest = d.density.iter().copied().fold(0.0, f64::max);
                    if widest <= 0.0 {
                        continue;
                    }
                    let side = |sign: f64, v: f64, dd: f64| at(p + sign * half * dd / widest, v);
                    let mut ring: Vec<(f64, f64)> = d
                        .values
                        .iter()
                        .zip(&d.density)
                        .map(|(v, dd)| side(1.0, *v, *dd))
                        .collect();
                    ring.extend(
                        d.values
                            .iter()
                            .zip(&d.density)
                            .rev()
                            .map(|(v, dd)| side(-1.0, *v, *dd)),
                    );
                    self.lines.push(ring);
                }
            }
            ElementKind::Rule { orientation, value } => match orientation {
                Orientation::Horizontal => {
                    let y = h * (1.0 - ya.share(ya.u(*value)));
                    self.lines.push(vec![(0.0, y), (w, y)]);
                }
                Orientation::Vertical => {
                    let x = w * xa.share(xa.u(*value));
                    self.lines.push(vec![(x, 0.0), (x, h)]);
                }
            },
            ElementKind::Polygon { paths } => {
                for ring in paths {
                    let mut l: Vec<(f64, f64)> = ring.iter().map(|[x, y]| at(*x, *y)).collect();
                    if let Some(first) = l.first().copied() {
                        l.push(first);
                    }
                    self.lines.push(l);
                }
            }
            ElementKind::Arrow {
                x,
                y,
                x_from,
                y_from,
                ..
            } => self.lines.push(vec![at(*x_from, *y_from), at(*x, *y)]),
            ElementKind::Text {
                x: Some(x),
                y: Some(y),
                text,
                anchor,
                size,
                weight,
                family,
                rotation,
                baseline,
                ..
            } => {
                let p = at(*x, *y);
                if !(p.0.is_finite() && p.1.is_finite()) {
                    return;
                }
                let look = TextLook {
                    size: size.unwrap_or(fonts.tick),
                    weight: *weight,
                    family: *family,
                    color: String::new(),
                    opacity: 1.0,
                    anchor: *anchor,
                    baseline: baseline.unwrap_or(VerticalAnchor::Center),
                    rotation: *rotation,
                };
                let block = rich::block_size(text, look.size, *weight, *family);
                let (bx, by, bw, bh) = rich::place(p, block, &look, 0.0).bounds;
                self.boxes.push((bx, by, bx + bw, by + bh));
            }
            ElementKind::Text { .. }
            | ElementKind::Heatmap { .. }
            | ElementKind::Contour { .. }
            | ElementKind::Image { .. } => {}
        }
    }

    /// Adds the box of a rectangle between two corners, in any order.
    fn rect(&mut self, a: (f64, f64), b: (f64, f64)) {
        if [a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
            self.boxes
                .push((a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)));
        }
    }
}

/// The closed path between two series over x, along `upper` and back along `lower`, as
/// matplotlib's path of `fill_between`.
fn between(
    x: &[f64],
    lower: &[f64],
    upper: &[f64],
    at: &impl Fn(f64, f64) -> (f64, f64),
) -> Vec<(f64, f64)> {
    let mut path: Vec<(f64, f64)> = x.iter().zip(upper).map(|(a, u)| at(*a, *u)).collect();
    path.extend(x.iter().zip(lower).rev().map(|(a, l)| at(*a, *l)));
    if let Some(first) = path.first().copied() {
        path.push(first);
    }
    path
}

/// Whether the segment from `a` to `b` meets the rectangle `r` (matplotlib's
/// `segment_intersects_rectangle`, a test of separating axes).
fn segment_meets(a: (f64, f64), b: (f64, f64), r: &Rect) -> bool {
    let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let (w, h) = (r.width, r.height);
    let ((x1, y1), (x2, y2)) = (a, b);
    (x1 + x2 - 2.0 * cx).abs() < (x1 - x2).abs() + w
        && (y1 + y2 - 2.0 * cy).abs() < (y1 - y2).abs() + h
        && 2.0 * ((x1 - cx) * (y1 - y2) - (y1 - cy) * (x1 - x2)).abs()
            < w * (y1 - y2).abs() + h * (x1 - x2).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn badness_counts_as_matplotlib() {
        let ink = Ink {
            // a line with two vertices in the frame, one crossing it, one beside it
            lines: vec![
                vec![(1.0, 1.0), (2.0, 2.0), (20.0, 2.0)],
                vec![(-5.0, 5.0), (15.0, 5.0)],
                vec![(30.0, 30.0), (40.0, 40.0)],
            ],
            // a box overlapping the frame and one touching its edge
            boxes: vec![(8.0, 8.0, 12.0, 12.0), (10.0, 0.0, 12.0, 4.0)],
        };
        // 2 vertices, 1 box and 2 lines
        assert_eq!(ink.badness(&rect(0.0, 0.0, 10.0, 10.0)), 5);
        assert_eq!(ink.badness(&rect(50.0, 50.0, 5.0, 5.0)), 0);
    }

    #[test]
    fn a_segment_beside_a_frame_misses_it() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        assert!(segment_meets((-5.0, 5.0), (15.0, 5.0), &r));
        assert!(segment_meets((2.0, 2.0), (3.0, 3.0), &r));
        assert!(!segment_meets((11.0, 0.0), (20.0, 9.0), &r));
    }
}

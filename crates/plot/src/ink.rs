//! The ink of the data of a plot: how much of a rectangle of its panel the drawn data covers,
//! which decides the corner of a legend inside the panel.
//!
//! The data is rasterized into a coarse occupancy grid of cells of [`CELL`] pixels over the
//! panel, each holding the ink drawn into it in square pixels (at most its area): a line is
//! sampled every pixel of its length and gives each sample its length times the width of the
//! line, a marker its square, a bar, a box, a histogram bin and a heatmap their rectangles, and
//! a band, an area, a violin and a polygon their interiors (nonzero per ring, as drawn), filled
//! on rows of a pixel. The cost is linear in the length of the lines and the area of the fills,
//! a few tenths of a millisecond for a dense figure, and the sums run in the order of the
//! elements and their points, so the same figure gives the same ink natively and in wasm.

use crate::render::Prepared;
use crate::render::axis::ResolvedAxis;
use crate::render::marks::ERROR_WIDTH;
use crate::render::stats_marks::{
    BAR_WIDTH, BOX_WIDTH, OUTLIER_SIZE, VIOLIN_WIDTH, base_of, box_body, spacing,
};
use crate::spec::{ElementKind, LineType, MarkerType, Orientation, Plot, Style, YAxisRef};

/// The side of a cell of the grid, in pixels.
const CELL: f64 = 4.0;
/// The distance between two samples of a line, in pixels.
const SAMPLE: f64 = 1.0;
/// The size of the ink of the anchor of a text, in pixels.
const POINT: f64 = 6.0;

/// The ink of a panel: a grid of cells over the panel, in pixels from its upper left corner.
pub(crate) struct Ink {
    width: f64,
    height: f64,
    columns: usize,
    rows: usize,
    cells: Vec<f64>,
}

impl Ink {
    /// An empty grid over a panel of `width` by `height` pixels.
    fn new(width: f64, height: f64) -> Self {
        let (width, height) = (width.max(0.0), height.max(0.0));
        let columns = (width / CELL).ceil() as usize;
        let rows = (height / CELL).ceil() as usize;
        Ink {
            width,
            height,
            columns,
            rows,
            cells: vec![0.0; columns * rows],
        }
    }

    /// The ink of the data of a plot in its panel of `width` by `height` pixels.
    pub(crate) fn of_plot(plot: &Plot, prep: &Prepared, (width, height): (f64, f64)) -> Self {
        let mut ink = Ink::new(width, height);
        for e in &plot.elements {
            let ya = match (e.axis, &prep.y2a) {
                (YAxisRef::Y2, Some(a)) => a,
                _ => &prep.ya,
            };
            ink.element(&e.kind, &e.style, (&prep.xa, ya));
        }
        ink
    }

    /// The share of a rectangle of the panel (`x`, `y`, `w`, `h` in pixels from its upper left
    /// corner) the ink covers, from 0 to 1.
    pub(crate) fn coverage(&self, (x, y, w, h): (f64, f64, f64, f64)) -> f64 {
        if w <= 0.0 || h <= 0.0 {
            return 0.0;
        }
        let full = CELL * CELL;
        let mut sum = 0.0;
        for (r, top, bottom) in self.span(y, y + h, self.rows) {
            for (c, left, right) in self.span(x, x + w, self.columns) {
                let ink = self.cells[r * self.columns + c].min(full);
                if ink > 0.0 {
                    sum += ink * (right - left) * (bottom - top) / full;
                }
            }
        }
        sum / (w * h)
    }

    /// The cells from `a` to `b` along a side of `n` cells: the index of each and the part of
    /// it inside, clipped to the panel.
    fn span(&self, a: f64, b: f64, n: usize) -> impl Iterator<Item = (usize, f64, f64)> {
        let a = a.max(0.0);
        let b = b.min(n as f64 * CELL);
        let first = if a < b { (a / CELL) as usize } else { n };
        let last = if a < b {
            ((b / CELL).ceil() as usize).min(n)
        } else {
            n
        };
        (first..last).map(move |i| {
            let (lo, hi) = (i as f64 * CELL, (i + 1) as f64 * CELL);
            (i, a.max(lo), b.min(hi))
        })
    }

    /// Adds the ink of a rectangle from (`x0`, `y0`) to (`x1`, `y1`), in any order.
    fn rect(&mut self, (x0, y0): (f64, f64), (x1, y1): (f64, f64)) {
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return;
        }
        let (left, right) = (x0.min(x1).max(0.0), x0.max(x1).min(self.width));
        let (top, bottom) = (y0.min(y1).max(0.0), y0.max(y1).min(self.height));
        if left >= right || top >= bottom {
            return;
        }
        let rows: Vec<_> = self.span(top, bottom, self.rows).collect();
        let columns: Vec<_> = self.span(left, right, self.columns).collect();
        for (r, t, b) in rows {
            for &(c, l, ri) in &columns {
                self.cells[r * self.columns + c] += (ri - l) * (b - t);
            }
        }
    }

    /// Adds the ink of a marker of `size` pixels at a point.
    fn marker(&mut self, (x, y): (f64, f64), size: f64) {
        let half = size / 2.0;
        self.rect((x - half, y - half), (x + half, y + half));
    }

    /// Adds the ink of a line of `width` pixels from `a` to `b`: its part inside the panel is
    /// sampled every [`SAMPLE`] pixels.
    fn segment(&mut self, a: (f64, f64), b: (f64, f64), width: f64) {
        let drawable = a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite();
        if !drawable || width <= 0.0 || self.cells.is_empty() {
            return;
        }
        let Some((a, b)) = clip(a, b, (self.width, self.height)) else {
            return;
        };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = libm::hypot(dx, dy);
        if length == 0.0 {
            return;
        }
        let n = (length / SAMPLE).ceil().max(1.0);
        let each = length / n * width;
        for k in 0..n as usize {
            let t = (k as f64 + 0.5) / n;
            let (x, y) = (a.0 + t * dx, a.1 + t * dy);
            let c = ((x / CELL) as usize).min(self.columns.saturating_sub(1));
            let r = ((y / CELL) as usize).min(self.rows.saturating_sub(1));
            self.cells[r * self.columns + c] += each;
        }
    }

    /// Adds the ink of a line of `width` pixels through consecutive drawable points.
    fn polyline(&mut self, points: impl IntoIterator<Item = (f64, f64)>, width: f64) {
        let mut last: Option<(f64, f64)> = None;
        for p in points {
            let ok = p.0.is_finite() && p.1.is_finite();
            if let (true, Some(a)) = (ok, last) {
                self.segment(a, p, width);
            }
            last = ok.then_some(p);
        }
    }

    /// Adds the ink of the interior of closed rings, each ring filled on its own by the
    /// nonzero winding rule and the rings together as their union, as the renderer draws them
    /// (every ring is a `<path>` of one closed subpath without a `fill-rule`). The interior is
    /// sampled on rows of [`SAMPLE`] pixels, at the middle of each, so a band of two pixels
    /// inks two rows of the cells it lies in; a ring with a point which cannot be drawn is left
    /// out.
    fn fill(&mut self, rings: &[Vec<(f64, f64)>]) {
        let rows = (self.height / SAMPLE).ceil() as usize;
        // the crossings of a ring with the middles of the rows: row, x and direction
        let mut crossings: Vec<(usize, f64, i32)> = Vec::new();
        for ring in rings {
            if ring.len() < 3 || ring.iter().any(|(x, y)| !(x.is_finite() && y.is_finite())) {
                continue;
            }
            crossings.clear();
            for (k, a) in ring.iter().enumerate() {
                let b = ring[(k + 1) % ring.len()];
                if a.1 == b.1 {
                    continue;
                }
                let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
                let dir = if b.1 > a.1 { 1 } else { -1 };
                // the rows whose middle lies in [lo, hi)
                let mut r = (lo / SAMPLE - 0.5).ceil().max(0.0) as usize;
                while r < rows {
                    let yc = (r as f64 + 0.5) * SAMPLE;
                    if yc >= hi {
                        break;
                    }
                    crossings.push((r, a.0 + (yc - a.1) * (b.0 - a.0) / (b.1 - a.1), dir));
                    r += 1;
                }
            }
            crossings.sort_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)).then(p.2.cmp(&q.2)));
            let (mut row, mut winding, mut start) = (usize::MAX, 0, 0.0);
            for &(r, x, dir) in &crossings {
                if r != row {
                    (row, winding) = (r, 0);
                }
                let before = winding;
                winding += dir;
                if before == 0 && winding != 0 {
                    start = x;
                } else if before != 0 && winding == 0 {
                    let top = r as f64 * SAMPLE;
                    self.rect((start, top), (x, (top + SAMPLE).min(self.height)));
                }
            }
        }
    }

    /// Adds the ink of an element.
    fn element(
        &mut self,
        kind: &ElementKind,
        style: &Style,
        (xa, ya): (&ResolvedAxis, &ResolvedAxis),
    ) {
        let (w, h) = (self.width, self.height);
        // pixels from the coordinates of the axes, and from values
        let pu = move |x: f64, y: f64| (w * xa.share(x), h * (1.0 - ya.share(y)));
        let at = move |x: f64, y: f64| pu(xa.u(x), ya.u(y));
        let line = style.line_or_default();
        let lined = line.kind != LineType::None && line.width > 0.0;
        let line_width = if lined { line.width } else { 0.0 };
        let marker = &style.marker;
        match kind {
            ElementKind::Curve { x, y, xerr, yerr } => {
                self.polyline(x.iter().zip(y).map(|(a, b)| at(*a, *b)), line_width);
                if marker.kind != MarkerType::None {
                    for (a, b) in x.iter().zip(y) {
                        self.marker(at(*a, *b), marker.size);
                    }
                }
                let ends = |axis: &ResolvedAxis, v: f64, e: f64| {
                    let end = |w: f64| {
                        let u = axis.u(w);
                        if u.is_finite() { u } else { axis.u(axis.low) }
                    };
                    (end(v - e.abs()), end(v + e.abs()))
                };
                if let Some(err) = xerr {
                    for ((a, b), e) in x.iter().zip(y).zip(err) {
                        let (lo, hi) = ends(xa, *a, *e);
                        let yu = ya.u(*b);
                        self.segment(pu(lo, yu), pu(hi, yu), ERROR_WIDTH);
                    }
                }
                if let Some(err) = yerr {
                    for ((a, b), e) in x.iter().zip(y).zip(err) {
                        let (lo, hi) = ends(ya, *b, *e);
                        let xu = xa.u(*a);
                        self.segment(pu(xu, lo), pu(xu, hi), ERROR_WIDTH);
                    }
                }
            }
            ElementKind::Band {
                x,
                low,
                high,
                median,
            } => {
                self.fill(&between(x, low, high, &at));
                if let Some(m) = median {
                    self.polyline(x.iter().zip(m).map(|(a, b)| at(*a, *b)), line_width);
                }
            }
            ElementKind::Area { x, from, to } => self.fill(&between(x, from, to, &at)),
            ElementKind::Bar {
                x,
                height,
                err,
                horizontal,
                width,
            } => {
                let (along, across) = if *horizontal { (ya, xa) } else { (xa, ya) };
                let bw = width.unwrap_or(BAR_WIDTH * spacing(x));
                let base = base_of(across);
                let point = |p: f64, v: f64| if *horizontal { pu(v, p) } else { pu(p, v) };
                for (k, (p, v)) in x.iter().zip(height).enumerate() {
                    let (a, b) = (along.u(p - bw / 2.0), along.u(p + bw / 2.0));
                    self.rect(point(a, base), point(b, across.u(*v)));
                    if let Some(e) = err.as_ref().and_then(|e| e.get(k)) {
                        let c = along.u(*p);
                        let (lo, hi) = (across.u(v - e.abs()), across.u(v + e.abs()));
                        let lo = if lo.is_finite() { lo } else { base };
                        self.segment(point(c, lo), point(c, hi), ERROR_WIDTH);
                    }
                }
            }
            ElementKind::Box { positions, stats } => {
                let bw = BOX_WIDTH * spacing(positions);
                let stroke = if lined { line.width } else { ERROR_WIDTH };
                let size = if marker.kind != MarkerType::None {
                    marker.size
                } else {
                    OUTLIER_SIZE
                };
                for (p, b) in positions.iter().zip(stats) {
                    let (left, center, right) = (xa.u(p - bw / 2.0), xa.u(*p), xa.u(p + bw / 2.0));
                    let (q1, q3) = box_body(ya, b.q1, b.q3);
                    self.rect(pu(left, q1), pu(right, q3));
                    let (lw, hw) = (ya.u(b.low_whisker), ya.u(b.high_whisker));
                    self.segment(pu(center, lw), pu(center, q1), stroke);
                    self.segment(pu(center, q3), pu(center, hw), stroke);
                    for o in &b.outliers {
                        self.marker(pu(center, ya.u(*o)), size);
                    }
                }
            }
            ElementKind::Violin {
                positions,
                densities,
            } => {
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
                    let side = |sign: f64, (v, w): (f64, f64)| at(p + sign * half * w / widest, v);
                    let mut ring: Vec<(f64, f64)> = points.iter().map(|q| side(1.0, *q)).collect();
                    ring.extend(points.iter().rev().map(|q| side(-1.0, *q)));
                    rings.push(ring);
                }
                self.fill(&rings);
            }
            ElementKind::Histogram { edges, counts } => {
                let base = base_of(ya);
                for (e, c) in edges.windows(2).zip(counts) {
                    if *c != 0.0 {
                        self.rect(pu(xa.u(e[0]), base), pu(xa.u(e[1]), ya.u(*c)));
                    }
                }
            }
            ElementKind::Heatmap { rows, columns, .. } => {
                // the cells as the renderer draws them, a value without a colour included
                for i in 0..rows.len() {
                    for j in 0..columns.len() {
                        let (r, c) = (i as f64, j as f64);
                        let (x0, x1) = (xa.u(c - 0.5), xa.u(c + 0.5));
                        let (y0, y1) = (ya.u(r - 0.5), ya.u(r + 0.5));
                        self.rect(pu(x0, y0), pu(x1, y1));
                    }
                }
            }
            ElementKind::Text { x, y, .. } => self.marker(at(*x, *y), POINT),
            ElementKind::Rule { orientation, value } => match orientation {
                Orientation::Horizontal => {
                    let y = h * (1.0 - ya.share(ya.u(*value)));
                    self.segment((0.0, y), (w, y), line_width);
                }
                Orientation::Vertical => {
                    let x = w * xa.share(xa.u(*value));
                    self.segment((x, 0.0), (x, h), line_width);
                }
            },
            ElementKind::Polygon { paths } => {
                let rings: Vec<Vec<(f64, f64)>> = paths
                    .iter()
                    .map(|ring| ring.iter().map(|[x, y]| at(*x, *y)).collect())
                    .collect();
                self.fill(&rings);
                for ring in &rings {
                    self.polyline(ring.iter().chain(ring.first()).copied(), line_width);
                }
            }
        }
    }
}

/// A point of the lower and of the upper series at one x, in pixels.
type Pair = ((f64, f64), (f64, f64));

/// The rings between two series over x: one ring per run of points which can all be drawn,
/// along `upper` and back along `lower`.
fn between(
    x: &[f64],
    lower: &[f64],
    upper: &[f64],
    at: &impl Fn(f64, f64) -> (f64, f64),
) -> Vec<Vec<(f64, f64)>> {
    let mut rings = Vec::new();
    let mut run: Vec<Pair> = Vec::new();
    let mut close = |run: &mut Vec<Pair>| {
        if run.len() >= 2 {
            let mut ring: Vec<(f64, f64)> = run.iter().map(|(_, u)| *u).collect();
            ring.extend(run.iter().rev().map(|(l, _)| *l));
            rings.push(ring);
        }
        run.clear();
    };
    for ((x, l), u) in x.iter().zip(lower).zip(upper) {
        let (a, b) = (at(*x, *l), at(*x, *u));
        if a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite() {
            run.push((a, b));
        } else {
            close(&mut run);
        }
    }
    close(&mut run);
    rings
}

/// The part of the segment from `a` to `b` inside the rectangle from (0, 0) to `size`, none
/// when it misses it (Liang and Barsky).
fn clip(a: (f64, f64), b: (f64, f64), (w, h): (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0), (dx, w - a.0), (-dy, a.1), (dy, h - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((
        (a.0 + t0 * dx, a.1 + t0 * dy),
        (a.0 + t1 * dx, a.1 + t1 * dy),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_clipped_to_the_panel() {
        let size = (10.0, 10.0);
        assert!(clip((-5.0, 5.0), (15.0, 5.0), size).is_some());
        assert_eq!(
            clip((-5.0, 5.0), (15.0, 5.0), size),
            Some(((0.0, 5.0), (10.0, 5.0)))
        );
        assert!(clip((-5.0, -1.0), (15.0, -1.0), size).is_none());
        assert!(clip((11.0, 0.0), (20.0, 10.0), size).is_none());
    }

    #[test]
    fn a_line_covers_its_width() {
        let mut ink = Ink::new(100.0, 100.0);
        ink.segment((0.0, 50.0), (100.0, 50.0), 2.0);
        // a line of 2 px across a frame 20 px high covers a tenth of it
        let share = ink.coverage((10.0, 40.0, 40.0, 20.0));
        assert!((share - 0.1).abs() < 1e-12, "{share}");
        assert_eq!(ink.coverage((10.0, 0.0, 40.0, 20.0)), 0.0);
    }

    #[test]
    fn a_ring_covers_its_interior() {
        let mut ink = Ink::new(100.0, 100.0);
        ink.fill(&[vec![(20.0, 20.0), (80.0, 20.0), (80.0, 80.0), (20.0, 80.0)]]);
        assert_eq!(ink.coverage((30.0, 30.0, 20.0, 20.0)), 1.0);
        assert_eq!(ink.coverage((0.0, 0.0, 20.0, 20.0)), 0.0);
        let half = ink.coverage((0.0, 40.0, 40.0, 20.0));
        assert!((half - 0.5).abs() < 1e-12, "{half}");
    }

    #[test]
    fn a_segment_with_a_point_which_cannot_be_drawn_inks_nothing() {
        let mut ink = Ink::new(100.0, 100.0);
        ink.segment((f64::NAN, 50.0), (50.0, 50.0), 2.0);
        ink.segment((10.0, 10.0), (10.0, f64::INFINITY), 2.0);
        assert!(ink.cells.iter().all(|c| *c == 0.0));
    }

    #[test]
    fn a_band_thinner_than_a_cell_inks_its_cells() {
        // two pixels high and off the middle of its row of cells (48 to 52)
        let mut ink = Ink::new(100.0, 100.0);
        ink.fill(&[vec![(0.0, 50.2), (100.0, 50.2), (100.0, 52.2), (0.0, 52.2)]]);
        let share = ink.coverage((10.0, 40.0, 40.0, 20.0));
        assert!((share - 0.1).abs() < 1e-12, "{share}");
    }

    #[test]
    fn a_ring_fills_by_the_nonzero_rule() {
        // a ring around the square from 20 to 40 twice: winding 2 inside, filled as drawn
        let twice = vec![
            (20.0, 20.0),
            (40.0, 20.0),
            (40.0, 40.0),
            (20.0, 40.0),
            (20.0, 20.0),
            (40.0, 20.0),
            (40.0, 40.0),
            (20.0, 40.0),
        ];
        let mut ink = Ink::new(100.0, 100.0);
        ink.fill(&[twice]);
        assert_eq!(ink.coverage((24.0, 24.0, 12.0, 12.0)), 1.0);
        // two overlapping rings are their union
        let square = |x: f64| vec![(x, 60.0), (x + 20.0, 60.0), (x + 20.0, 80.0), (x, 80.0)];
        let mut ink = Ink::new(100.0, 100.0);
        ink.fill(&[square(20.0), square(30.0)]);
        assert_eq!(ink.coverage((32.0, 64.0, 4.0, 12.0)), 1.0);
    }

    #[test]
    fn a_heatmap_inks_the_cells_it_covers() {
        let axis = |low: f64, high: f64| ResolvedAxis {
            transform: crate::render::axis::Transform::Category,
            reverse: false,
            low,
            high,
            ticks: Vec::new(),
        };
        // two by two cells from -0.5 to 1.5 on axes from -0.5 to 3.5: the lower left quarter
        let (xa, ya) = (axis(-0.5, 3.5), axis(-0.5, 3.5));
        let kind = ElementKind::Heatmap {
            rows: vec!["a".into(), "b".into()],
            columns: vec!["c".into(), "d".into()],
            values: vec![vec![1.0, 2.0], vec![3.0, f64::NAN]],
            text: None,
            scale: serde_json::from_str(r#"{"name": "viridis", "low": 0, "high": 4}"#).unwrap(),
        };
        let mut ink = Ink::new(100.0, 100.0);
        ink.element(&kind, &Style::default(), (&xa, &ya));
        // away from the cells of the grid its edge splits
        assert_eq!(ink.coverage((0.0, 52.0, 48.0, 48.0)), 1.0);
        assert_eq!(ink.coverage((52.0, 0.0, 48.0, 100.0)), 0.0);
        assert_eq!(ink.coverage((0.0, 0.0, 48.0, 48.0)), 0.0);
    }
}

//! The thinning of a line of a figure to the points which can be seen, with the algorithm of
//! Ramer, Douglas and Peucker.
//!
//! A curve of a simulation carries every step of the integrator: thousands of points for a
//! line which is a few hundred pixels long. The chart builder of the python package
//! (`sbmlsim_core.chart.thinning`) reduces such a line to the points which are visibly apart
//! before it writes the spec of a figure; this crate is its core, [`kept_points`], which the
//! python package calls through its bindings. The python module chooses the coordinates (the
//! ones of the panel in units of the span of its axes, the `log10` of a value on a
//! logarithmic axis) and the points which are kept in any case; this crate decides which of
//! the others are kept.
//!
//! A point is left out when it is within the tolerance of the segment between the points kept
//! around it, on every column of the line (one for a curve, the low and the high end and the
//! median of a band), and a segment whose farthest point is beyond the tolerance is split at
//! it, the first such point of the segment on a tie. The kept points are bitwise the ones of
//! the earlier implementation of the python module on numpy, which split all segments of a
//! level at once: the split of a segment depends on nothing but its points, so the order in
//! which the segments are split does not change the result, and the distance of a point is
//! computed by the same operations in the same order (no fused multiply-add), with the `hypot`
//! of the C library which numpy calls as well.
//!
//! The farthest point of a segment is found without a `hypot` per point: the square of the
//! distance, which the processor computes many times faster, orders the points as the
//! distance does up to a few roundings, so only the points whose square is within
//! [`SCREEN_MARGIN`] of the largest one can be the farthest, and only their distances are
//! computed. A segment whose squares cannot be trusted (a square which is not a number, an
//! overflow, or one so small that it is subnormal) has the distance of every point computed.

/// The relative margin below the largest square of the distance of the points of a segment
/// within which a point is a candidate for the farthest one. The square and the distance of
/// a point are each a few roundings (about `1e-16` each) off their exact values, so a point
/// whose square is smaller by more than this margin is nearer than the point of the largest
/// square, whatever the roundings.
pub const SCREEN_MARGIN: f64 = 1e-9;

/// The smallest largest square of a segment which the screening trusts: a square below it
/// may be subnormal and have lost the precision the margin assumes.
pub const SCREEN_FLOOR: f64 = 1e-290;

/// Get the offset of a point from the nearest point of a segment, in one column.
///
/// The operations of the python module on numpy in their order, so the offset is bitwise
/// its one: `t` is the projection of the point onto the segment as a share of it, clipped to
/// the segment, `0` for a segment of no length.
///
/// Args: `px`, `py` the point and `dx`, `dy` the end of the segment relative to its start,
/// `length` the square of the length of the segment.
#[inline(always)]
fn offset(px: f64, py: f64, dx: f64, dy: f64, length: f64) -> (f64, f64) {
    let t = if length > 0.0 {
        (px * dx + py * dy) / length
    } else {
        0.0
    };
    let t = t.clamp(0.0, 1.0);
    (px - t * dx, py - t * dy)
}

/// Get the larger of two numbers, not a number if either is not one, as `numpy.maximum`.
#[inline(always)]
fn maximum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b > a {
        b
    } else {
        a
    }
}

/// A line: its x coordinates and the y coordinates of its columns over them.
struct Line<'a> {
    x: &'a [f64],
    columns: &'a [&'a [f64]],
}

impl Line<'_> {
    /// Get the distance of a point to the segment between two points, the most over the
    /// columns, not a number if one of them is not a number.
    fn distance(&self, p: usize, a: usize, b: usize) -> f64 {
        let dx = self.x[b] - self.x[a];
        let px = self.x[p] - self.x[a];
        let mut distance = f64::NEG_INFINITY;
        for (c, y) in self.columns.iter().enumerate() {
            let dy = y[b] - y[a];
            let py = y[p] - y[a];
            let (ex, ey) = offset(px, py, dx, dy, dx * dx + dy * dy);
            let d = ex.hypot(ey);
            distance = if c == 0 { d } else { maximum(distance, d) };
        }
        distance
    }

    /// Get the squares of the distances of the points inside a segment, the most over the
    /// columns, into `squares`, a point after the other.
    fn squares(&self, a: usize, b: usize, squares: &mut Vec<f64>) {
        squares.clear();
        let dx = self.x[b] - self.x[a];
        let xa = self.x[a];
        let xs = &self.x[a + 1..b];
        for (c, y) in self.columns.iter().enumerate() {
            let dy = y[b] - y[a];
            let ya = y[a];
            let length = dx * dx + dy * dy;
            let ys = &y[a + 1..b];
            let square = |x: f64, y: f64| {
                let (ex, ey) = offset(x - xa, y - ya, dx, dy, length);
                ex * ex + ey * ey
            };
            if c == 0 {
                squares.extend(xs.iter().zip(ys).map(|(&x, &y)| square(x, y)));
            } else {
                for ((s, &x), &y) in squares.iter_mut().zip(xs).zip(ys) {
                    *s = maximum(*s, square(x, y));
                }
            }
        }
    }

    /// Get the point of a segment which is farthest from it, the first one on a tie, with
    /// its distance, which is not a number if the distance of a point is not one.
    fn farthest(&self, a: usize, b: usize, squares: &mut Vec<f64>) -> (usize, f64) {
        self.squares(a, b, squares);
        let mut largest = f64::NEG_INFINITY;
        let mut nan = false;
        for &s in squares.iter() {
            nan |= s.is_nan();
            largest = if s > largest { s } else { largest };
        }
        let screened = !nan && largest.is_finite() && largest >= SCREEN_FLOOR;
        let floor = largest * (1.0 - SCREEN_MARGIN);
        let mut peak = (a + 1, f64::NEG_INFINITY);
        for (p, &s) in (a + 1..b).zip(squares.iter()) {
            if screened && s < floor {
                continue;
            }
            let d = self.distance(p, a, b);
            if d.is_nan() {
                return (p, f64::NAN);
            }
            if d > peak.1 {
                peak = (p, d);
            }
        }
        peak
    }
}

/// Mark the points of a line which are kept, Ramer-Douglas-Peucker.
///
/// A segment between two kept points is split at its point farthest from it while that
/// point is farther than the tolerance on a column, see the crate. Kept are the first and the
/// last point, the ones already marked in `kept`, every point which is not finite on the x
/// axis or a column and its neighbours, and the points which keep every other point within
/// the tolerance of the segment between the kept points around it. A segment with a distance
/// which is not a number (an overflow of the coordinates) is not split.
///
/// Args: `x` the x coordinates of the points, `columns` the y coordinates of one or more
/// columns over them, `tolerance` the largest distance of a point which is left out, and
/// `kept` whether a point is kept, the points which are kept in any case on entry and the
/// kept ones on return.
///
/// # Panics
///
/// If there is no column, or if a column or `kept` is not as long as `x`.
pub fn kept_points(x: &[f64], columns: &[&[f64]], tolerance: f64, kept: &mut [bool]) {
    let n = x.len();
    assert!(!columns.is_empty(), "a line has at least one column");
    assert!(
        columns.iter().all(|y| y.len() == n) && kept.len() == n,
        "the columns and the kept points are as long as the x coordinates"
    );
    if n <= 2 {
        kept.fill(true);
        return;
    }
    kept[0] = true;
    kept[n - 1] = true;
    let gap = |i: usize| !(x[i].is_finite() && columns.iter().all(|y| y[i].is_finite()));
    let mut previous = false;
    for i in 0..n {
        let current = gap(i);
        if current {
            kept[i] = true;
            if i > 0 {
                kept[i - 1] = true;
            }
        } else if previous {
            kept[i] = true;
        }
        previous = current;
    }
    let mut segments = Vec::new();
    let mut start = 0;
    for (i, &k) in kept.iter().enumerate().skip(1) {
        if k {
            if i - start > 1 {
                segments.push((start, i));
            }
            start = i;
        }
    }
    let line = Line { x, columns };
    let mut squares = Vec::new();
    while let Some((a, b)) = segments.pop() {
        let (p, d) = line.farthest(a, b, &mut squares);
        if d > tolerance {
            kept[p] = true;
            if p - a > 1 {
                segments.push((a, p));
            }
            if b - p > 1 {
                segments.push((p, b));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Get the kept points of a line of one column, the given ones and the ends.
    fn kept(x: &[f64], y: &[f64], tolerance: f64) -> Vec<usize> {
        let mut kept = vec![false; x.len()];
        kept_points(x, &[y], tolerance, &mut kept);
        (0..x.len()).filter(|&i| kept[i]).collect()
    }

    /// Get the farthest point of a segment by the distance of every point, the search the
    /// screening replaces.
    fn exact(line: &Line<'_>, a: usize, b: usize) -> (usize, f64) {
        let mut peak = (a + 1, f64::NEG_INFINITY);
        for p in a + 1..b {
            let d = line.distance(p, a, b);
            if d.is_nan() {
                return (p, f64::NAN);
            }
            if d > peak.1 {
                peak = (p, d);
            }
        }
        peak
    }

    /// A generator of pseudo random numbers in [0, 1), xorshift64*.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn a_short_line_is_kept() {
        assert_eq!(kept(&[0.0, 1.0], &[0.0, 5.0], 0.1), vec![0, 1]);
        assert_eq!(kept(&[], &[], 0.1), Vec::<usize>::new());
    }

    #[test]
    fn a_straight_line_keeps_its_ends() {
        let x: Vec<f64> = (0..100).map(|i| f64::from(i) / 99.0).collect();
        assert_eq!(kept(&x, &x, 1e-4), vec![0, 99]);
    }

    #[test]
    fn a_corner_is_kept() {
        let x: Vec<f64> = (0..11).map(|i| f64::from(i) / 10.0).collect();
        let y: Vec<f64> = x.iter().map(|&x| (x - 0.5).abs()).collect();
        assert_eq!(kept(&x, &y, 1e-4), vec![0, 5, 10]);
    }

    #[test]
    fn a_gap_and_its_neighbours_are_kept() {
        let x: Vec<f64> = (0..10).map(f64::from).collect();
        let mut y = vec![0.0; 10];
        y[4] = f64::NAN;
        assert_eq!(kept(&x, &y, 1e-4), vec![0, 3, 4, 5, 9]);
    }

    #[test]
    fn the_given_points_are_kept() {
        let x: Vec<f64> = (0..10).map(f64::from).collect();
        let mut kept = vec![false; 10];
        kept[6] = true;
        kept_points(&x, &[&x], 1e-4, &mut kept);
        assert_eq!(kept.iter().filter(|&&k| k).count(), 3);
        assert!(kept[0] && kept[6] && kept[9]);
    }

    #[test]
    fn a_band_is_thinned_on_every_column() {
        let x: Vec<f64> = (0..11).map(|i| f64::from(i) / 10.0).collect();
        let low = vec![0.0; 11];
        let mut high = vec![1.0; 11];
        high[7] = 2.0;
        let mut kept = vec![false; 11];
        kept_points(&x, &[&low, &high], 1e-4, &mut kept);
        assert_eq!(
            (0..11).filter(|&i| kept[i]).collect::<Vec<_>>(),
            vec![0, 6, 7, 8, 10]
        );
    }

    #[test]
    fn a_tie_splits_at_the_first_point() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0];
        let y = [0.0, 1.0, 0.0, 1.0, 0.0];
        let line = Line {
            x: &x,
            columns: &[&y],
        };
        assert_eq!(line.farthest(0, 4, &mut Vec::new()), (1, 1.0));
    }

    #[test]
    fn an_overflow_splits_nothing() {
        // the square of the length of the segment overflows, so the projection is not a number
        let x = [0.0, 1e200, 2e200, 3e200];
        let y = [0.0, 1.0, -1.0, 0.0];
        assert_eq!(kept(&x, &y, 1e-4), vec![0, 3]);
    }

    #[test]
    fn the_screening_finds_the_farthest_point_of_every_point() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        let mut squares = Vec::new();
        for case in 0..4000 {
            let n = 3 + (random.next() * 60.0) as usize;
            // scales from the subnormals to the overflow, flat and repeated values
            let scale = 10f64.powi((random.next() * 620.0) as i32 - 310);
            let mut x: Vec<f64> = (0..n).map(|_| random.next() * scale).collect();
            x.sort_by(f64::total_cmp);
            let columns: Vec<Vec<f64>> = (0..1 + case % 3)
                .map(|_| {
                    (0..n)
                        .map(|i| match case % 5 {
                            0 => 0.0,
                            1 => (i % 3) as f64 * scale,
                            2 => (random.next() * 4.0).floor() * scale,
                            _ => random.next() * scale,
                        })
                        .collect()
                })
                .collect();
            let columns: Vec<&[f64]> = columns.iter().map(Vec::as_slice).collect();
            let line = Line {
                x: &x,
                columns: &columns,
            };
            let (p, d) = line.farthest(0, n - 1, &mut squares);
            let (q, e) = exact(&line, 0, n - 1);
            assert!(
                p == q && (d == e || d.is_nan() && e.is_nan()),
                "case {case}: the screening found {p} at {d}, the search {q} at {e}"
            );
        }
    }
}

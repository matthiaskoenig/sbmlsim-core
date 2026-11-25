//! The cut of a series at the ends of its x axis (`xEnd` of `plot.ts`): nothing beyond an end is
//! drawn, and a value beyond it is no value of the y axis either.
//!
//! A series over x (a curve, a band, an area) is cut into the runs of its finite points inside
//! the interval of its x axis. A segment which crosses an end of the interval ends at the end,
//! its x the end exactly and its other columns interpolated linearly in the coordinates of
//! their axes (the `log10` of a value on a logarithmic axis), so a line meets the edge of the
//! panel where its straight segment does. A point of the series beyond an end is dropped, and so
//! are its marker and its error bars, which `holds` tells apart; the points an end adds carry no
//! marker. A point inside the interval keeps its bits.
//!
//! The y axes count only the values of a series within the ends the spec gives the x axis and
//! the value where it crosses an end ([`XEnds::values`]), the values the panel shows. This is
//! intended: it is what a panel of the web app frames its value axis by, while `plot.ts` puts
//! the values of a curve beyond `xEnd` on its axis as well, which a panel covers with its frame
//! (`yLimits`).

use super::axis::{log10, pow10};
use crate::spec::{Axis, AxisScale};

/// The interval of the x coordinate a series is cut to, its ends included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Span {
    /// The lower end, `-inf` for none.
    pub lo: f64,
    /// The upper end, `inf` for none.
    pub hi: f64,
}

impl Span {
    /// Whether a coordinate lies inside the interval.
    pub(crate) fn holds(&self, x: f64) -> bool {
        x >= self.lo && x <= self.hi
    }
}

/// The ends a spec gives its x axis, which the values of the y axes are taken inside of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct XEnds {
    /// The ends in coordinates of the x axis.
    span: Span,
    /// Whether the x axis is logarithmic.
    log: bool,
}

impl XEnds {
    /// The ends of a linear or logarithmic x axis its spec gives, as the axis takes them: an end
    /// which is not finite, or not positive on a logarithmic axis, is none, and so are both of a
    /// pair which is no span. `None` without an end.
    pub(crate) fn of(axis: &Axis) -> Option<XEnds> {
        let log = match axis.scale {
            AxisScale::Linear => false,
            AxisScale::Log => true,
            AxisScale::Category => return None,
        };
        let usable = |e: Option<f64>| e.filter(|v| v.is_finite() && (!log || *v > 0.0));
        let (low, high) = match (usable(axis.min), usable(axis.max)) {
            (Some(l), Some(h)) if l >= h => (None, None),
            ends => ends,
        };
        if low.is_none() && high.is_none() {
            return None;
        }
        let coordinate = |v: f64| if log { log10(v) } else { v };
        Some(XEnds {
            span: Span {
                lo: low.map_or(f64::NEG_INFINITY, coordinate),
                hi: high.map_or(f64::INFINITY, coordinate),
            },
            log,
        })
    }

    /// The coordinate of an x, NaN for one the axis cannot show.
    fn coordinate(&self, v: f64) -> f64 {
        match (self.log, v > 0.0) {
            (false, _) => v,
            (true, true) => log10(v),
            (true, false) => f64::NAN,
        }
    }

    /// Whether an x lies inside the ends.
    pub(crate) fn holds(&self, x: f64) -> bool {
        self.span.holds(self.coordinate(x))
    }

    /// The values of the columns of a series inside the ends, in data units: at its points
    /// inside and where its segments cross an end, interpolated in the coordinates of a
    /// logarithmic y axis when `log_y`, every column after the other.
    pub(crate) fn values(&self, x: &[f64], columns: &[&[f64]], log_y: bool) -> Vec<f64> {
        let xu: Vec<f64> = x.iter().map(|v| self.coordinate(*v)).collect();
        // a value a logarithmic axis cannot show is a gap, as it is in the drawing
        let shown: Vec<Vec<f64>> = columns
            .iter()
            .map(|c| {
                c.iter()
                    .map(|v| if log_y && *v <= 0.0 { f64::NAN } else { *v })
                    .collect()
            })
            .collect();
        let refs: Vec<&[f64]> = shown.iter().map(Vec::as_slice).collect();
        let lerp = |a: f64, b: f64, t: f64| {
            if log_y {
                pow10(linear(log10(a), log10(b), t))
            } else {
                linear(a, b, t)
            }
        };
        let runs = cut_runs(&xu, &refs, self.span, lerp);
        let mut out = Vec::new();
        for k in 0..columns.len() {
            for (_, run) in &runs {
                out.extend_from_slice(&run[k]);
            }
        }
        out
    }
}

/// Interpolates between two coordinates.
pub(crate) fn linear(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

/// A run of a series: its x and the values of every column at them.
pub(crate) type Run = (Vec<f64>, Vec<Vec<f64>>);

/// The runs of a series of points `(x[i], columns[k][i])` inside `span`.
///
/// A run is a stretch of consecutive points whose x and columns are finite; it is cut where it
/// leaves the span and starts again where it enters it, at the end it crosses. A run of a single
/// point inside the span is kept, a caller which draws lines skips it.
///
/// `x` are coordinates of the x axis; `lerp(a, b, t)` interpolates a column between two of its
/// values at the share `t` of the way, [`linear`] for coordinates.
pub(crate) fn cut_runs(
    x: &[f64],
    columns: &[&[f64]],
    span: Span,
    lerp: impl Fn(f64, f64, f64) -> f64,
) -> Vec<Run> {
    let n = columns.iter().fold(x.len(), |n, c| n.min(c.len()));
    let finite = |i: usize| x[i].is_finite() && columns.iter().all(|c| c[i].is_finite());
    let exact = |i: usize| -> (f64, Vec<f64>) { (x[i], columns.iter().map(|c| c[i]).collect()) };
    let mut out: Vec<Run> = Vec::new();
    let mut current: Option<Run> = None;
    let close = |current: &mut Option<Run>, out: &mut Vec<Run>| {
        if let Some(run) = current.take() {
            out.push(run);
        }
    };
    let push = |current: &mut Option<Run>, (px, pc): (f64, Vec<f64>)| {
        let run = current.get_or_insert_with(|| (Vec::new(), vec![Vec::new(); pc.len()]));
        run.0.push(px);
        for (column, v) in run.1.iter_mut().zip(pc) {
            column.push(v);
        }
    };
    let mut i = 0;
    while i < n {
        if !finite(i) {
            close(&mut current, &mut out);
            i += 1;
            continue;
        }
        let at_end_of_run = i + 1 >= n || !finite(i + 1);
        if at_end_of_run {
            // a point without a segment after it: its own run, or the end of the one it closes
            if current.is_none() && span.holds(x[i]) {
                push(&mut current, exact(i));
            }
            close(&mut current, &mut out);
            i += 1;
            continue;
        }
        let (a, b) = (i, i + 1);
        let (xa, xb) = (x[a], x[b]);
        // the part of the segment inside the span, as shares `t0 <= t1` of it, and the ends
        // it enters and leaves through
        let (t0, enter, t1, leave) = if xa == xb {
            if span.holds(xa) {
                (0.0, xa, 1.0, xa)
            } else {
                (1.0, xa, 0.0, xa)
            }
        } else {
            let share = |end: f64| (end - xa) / (xb - xa);
            let (first, last) = if xb > xa {
                (span.lo, span.hi)
            } else {
                (span.hi, span.lo)
            };
            (share(first).max(0.0), first, share(last).min(1.0), last)
        };
        if t0 > t1 {
            close(&mut current, &mut out);
            i += 1;
            continue;
        }
        let point = |t: f64, end: f64| -> (f64, Vec<f64>) {
            if t <= 0.0 {
                exact(a)
            } else if t >= 1.0 {
                exact(b)
            } else {
                let values = columns.iter().map(|c| lerp(c[a], c[b], t)).collect();
                (end, values)
            }
        };
        if t0 > 0.0 {
            close(&mut current, &mut out);
        }
        if current.is_none() {
            push(&mut current, point(t0, enter));
        }
        if t1 > t0 {
            push(&mut current, point(t1, leave));
        }
        if t1 < 1.0 {
            close(&mut current, &mut out);
        }
        i += 1;
    }
    close(&mut current, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xs(runs: &[Run]) -> Vec<Vec<f64>> {
        runs.iter().map(|r| r.0.clone()).collect()
    }

    #[test]
    fn an_open_span_keeps_the_runs() {
        let y = [1.0, 2.0, f64::NAN, 4.0, 5.0, 6.0];
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let runs = cut_runs(
            &x,
            &[&y],
            Span {
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            },
            linear,
        );
        assert_eq!(xs(&runs), vec![vec![0.0, 1.0], vec![3.0, 4.0, 5.0]]);
        assert_eq!(runs[1].1, vec![vec![4.0, 5.0, 6.0]]);
    }

    #[test]
    fn a_crossing_ends_at_the_end() {
        let x = [0.0, 1.0, 2.0, 3.0];
        let y = [0.0, 10.0, 20.0, 30.0];
        let runs = cut_runs(&x, &[&y], Span { lo: 0.5, hi: 2.25 }, linear);
        assert_eq!(xs(&runs), vec![vec![0.5, 1.0, 2.0, 2.25]]);
        assert_eq!(runs[0].1, vec![vec![5.0, 10.0, 20.0, 22.5]]);
    }

    #[test]
    fn a_series_which_leaves_and_comes_back_is_two_runs() {
        let x = [0.0, 2.0, 0.0];
        let y = [0.0, 2.0, 4.0];
        let runs = cut_runs(&x, &[&y], Span { lo: -1.0, hi: 1.0 }, linear);
        assert_eq!(xs(&runs), vec![vec![0.0, 1.0], vec![1.0, 0.0]]);
        assert_eq!(runs[1].1, vec![vec![3.0, 4.0]]);
    }

    #[test]
    fn single_points_and_points_outside() {
        let x = [5.0, f64::NAN, 0.5, f64::NAN, 3.0, 4.0];
        let y = [1.0, 1.0, 2.0, 1.0, 3.0, 4.0];
        let runs = cut_runs(&x, &[&y], Span { lo: 0.0, hi: 2.0 }, linear);
        assert_eq!(xs(&runs), vec![vec![0.5]]);
    }

    #[test]
    fn a_point_at_an_end_is_kept() {
        let x = [0.0, 1.0, 2.0];
        let y = [1.0, 2.0, 3.0];
        let runs = cut_runs(&x, &[&y], Span { lo: 0.0, hi: 1.0 }, linear);
        assert_eq!(xs(&runs), vec![vec![0.0, 1.0]]);
    }
}

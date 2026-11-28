//! The rules of the timecourses of sbmlsim, one implementation for the fit (`sbmlsim-fit`) and
//! the planner of a scan (`sbmlsim-scan`), ported operation for operation from numpy so a value
//! is the one of sbmlsim to the bit:
//!
//! - [`grid_weights`] and [`Weight::apply`]: the interpolation of a timecourse onto a grid of
//!   times (`grid_weights` and `apply_weights` of `sbmlsim.result.timecourse`);
//! - [`kept_rows`]: the rows of a timecourse without the state before every change, the samples
//!   pkpdutils analyses (`without_states_before`), and [`padded_length`], the length the
//!   samples of an analysis are padded to (`_padded` of `sbmlsim.simulator.pk`);
//! - [`extreme`] and [`mean`]: the reductions `max`, `min` and `mean` over the time of a
//!   simulation (`_extreme` and `_mean` of `sbmlsim.simulator.formula`).
//!
//! The output of a simulation has two time points at the time of a change after the start, the
//! states before and after it, and at an event of the model: a jump. A chain of jumps starts at
//! a jump and has the time points after it at most the resolution of the time apart. A time of
//! the grid before a chain is interpolated toward the state before it, a time at a jump or
//! inside a chain sees the state after its last jump, a time at most the resolution before a
//! chain of an event the state after the chain, and a time of `left` the state before the jump
//! at it. A row of a time which is not finite (the padding, the steady state after the end) is
//! no time point.
//!
//! ```
//! use sbmlsim_timecourse::grid_weights;
//!
//! let time = [0.0, 1.0, 2.0, 2.0, 3.0];
//! let values = [0.0, 1.0, 2.0, 10.0, 11.0];
//! let weights = grid_weights(&time, &[1.5, 2.0, 2.0, 2.5], 1e-9, &[2.0], None);
//! let at: Vec<f64> = weights.iter().map(|w| w.value(&values)).collect();
//! assert_eq!(at, [1.5, 2.0, 10.0, 10.5]);
//! ```

/// The weights which interpolate one time of a grid between two rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weight {
    /// The row below the time.
    pub lower: usize,
    /// The row above it.
    pub upper: usize,
    /// The weight of the upper row, the lower one takes `1 - weight`.
    pub weight: f64,
    /// Whether the time is within the finite time points, outside of which it has no value.
    pub valid: bool,
}

impl Weight {
    /// The interpolated value of the values of the two rows, `apply_weights` of sbmlsim: an
    /// exact row needs that row only, so a `NaN` or an infinity of its neighbour does not reach
    /// the value through a product with zero; `NaN` outside of the time points.
    #[must_use]
    pub fn apply(&self, lower: f64, upper: f64) -> f64 {
        if !self.valid {
            return f64::NAN;
        }
        if self.weight == 0.0 {
            return lower;
        }
        if self.weight == 1.0 {
            return upper;
        }
        lower * (1.0 - self.weight) + upper * self.weight
    }

    /// The interpolated value of a row of values, [`Weight::apply`]; `NaN` for no values.
    #[must_use]
    pub fn value(&self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return f64::NAN;
        }
        self.apply(values[self.lower], values[self.upper])
    }
}

/// The first index of a sorted slice whose value is above `value` (`side="right"`), or at or
/// above it (`side="left"`), `searchsorted` of numpy.
fn search_sorted(sorted: &[f64], value: f64, right: bool) -> usize {
    sorted.partition_point(|x| if right { *x <= value } else { *x < value })
}

/// The indices and weights which interpolate the rows of a timecourse at the times of a grid,
/// `grid_weights` of sbmlsim, see the crate.
///
/// `time` is the time of every row, sorted; `tolerance` the resolution of the time of the
/// simulation (`0` takes only a time point which appears twice as a jump); `changes` the times
/// of the changes after the start in the time of the rows; `left` whether a time of the grid
/// reads the state before a jump at it, by default the first of two equal times of the grid.
///
/// # Panics
///
/// A `left` of another length than the grid.
#[must_use]
pub fn grid_weights(
    time: &[f64],
    grid: &[f64],
    tolerance: f64,
    changes: &[f64],
    left: Option<&[bool]>,
) -> Vec<Weight> {
    let left: Vec<bool> = match left {
        Some(left) => {
            assert_eq!(
                left.len(),
                grid.len(),
                "a flag of `left` per time of the grid"
            );
            left.to_vec()
        }
        None => (0..grid.len())
            .map(|k| k + 1 < grid.len() && grid[k] == grid[k + 1])
            .collect(),
    };
    let finite: Vec<usize> = (0..time.len()).filter(|&r| time[r].is_finite()).collect();
    let t: Vec<f64> = finite.iter().map(|&r| time[r]).collect();
    let n = t.len();
    if n == 0 {
        let none = Weight {
            lower: 0,
            upper: 0,
            weight: 0.0,
            valid: false,
        };
        return vec![none; grid.len()];
    }
    let is_change = |value: f64| changes.contains(&value);
    // the jumps: a time point which appears twice, its second point
    let twice: Vec<bool> = (0..n).map(|i| i > 0 && t[i] == t[i - 1]).collect();
    // the chains: time points at most the tolerance apart, which start at their first jump
    let mut split: Vec<bool> = (0..n - 1).map(|i| t[i + 1] - t[i] > tolerance).collect();
    let mut close = vec![0usize; n];
    for i in 1..n {
        close[i] = close[i - 1] + usize::from(split[i - 1]);
    }
    let mut first_jump = vec![n; close[n - 1] + 1];
    for i in 0..n {
        if twice[i] {
            first_jump[close[i]] = first_jump[close[i]].min(i - 1);
        }
    }
    for i in 0..n - 1 {
        split[i] |= i + 1 == first_jump[close[i + 1]];
    }
    let mut chain = vec![0usize; n];
    for i in 1..n {
        chain[i] = chain[i - 1] + usize::from(split[i - 1]);
    }
    let starts: Vec<usize> = (0..n).filter(|&i| i == 0 || split[i - 1]).collect();
    let ends: Vec<usize> = (0..n).filter(|&i| i == n - 1 || split[i]).collect();
    let first: Vec<usize> = (0..n).map(|i| starts[chain[i]]).collect();
    let last: Vec<usize> = (0..n).map(|i| ends[chain[i]]).collect();
    // the state after the last jump of a chain
    let mut last_jump: Vec<Option<usize>> = vec![None; chain[n - 1] + 1];
    for i in 0..n {
        if twice[i] {
            last_jump[chain[i]] = Some(last_jump[chain[i]].map_or(i, |j| j.max(i)));
        }
    }
    let end: Vec<usize> = (0..n)
        .map(|i| last_jump[chain[i]].unwrap_or(last[i]))
        .collect();
    grid.iter()
        .zip(left)
        .map(|(&g, left)| {
            let valid = g >= t[0] && g <= t[n - 1];
            let i = search_sorted(&t, g, true).saturating_sub(1).min(n - 1);
            let upper = (i + 1).min(n - 1);
            let span = t[upper] - t[i];
            let weight = if span > 0.0 { (g - t[i]) / span } else { 0.0 };
            let at = t[i] == g;
            let own = at && (twice[i] || is_change(t[i]));
            let inside = !own && end[i] > i;
            let following = upper;
            let ahead = !own
                && !inside
                && following > i
                && first[following] == following
                && last[following] > following
                && t[following] - g <= tolerance
                && !is_change(t[following]);
            let at_own = if last_jump[chain[i]].is_some() {
                end[i].max(i)
            } else {
                i
            };
            let mut to = if own {
                at_own
            } else if inside {
                end[i]
            } else {
                end[following]
            };
            let after = !left && (own || inside || ahead);
            let before = left && ((at && twice[i]) || inside);
            if before {
                to = if at && twice[i] {
                    search_sorted(&t, g, false)
                } else {
                    first[i]
                };
            }
            let fixed = after || before;
            Weight {
                lower: finite[if fixed { to } else { i }],
                upper: finite[if fixed { to } else { upper }],
                weight: if fixed { 0.0 } else { weight },
                valid,
            }
        })
        .collect()
}

/// The rows of a timecourse of a finite time without the state before every change after the
/// start, the first of two rows at the time of one of `changes`; the two rows of an event of
/// the model stay. `without_states_before` of sbmlsim, the samples pkpdutils analyses.
#[must_use]
pub fn kept_rows(time: &[f64], changes: &[f64]) -> Vec<usize> {
    let n = time.len();
    (0..n)
        .filter(|&row| {
            let t = time[row];
            if !t.is_finite() {
                return false;
            }
            let first = row == 0 || time[row] != time[row - 1];
            let twice = row + 1 < n && time[row] == time[row + 1];
            !(first && twice && changes.contains(&t))
        })
        .collect()
}

/// The shortest length of a padded sample of an analysis, `MIN_PADDED_LENGTH` of sbmlsim.
pub const MIN_PADDED_LENGTH: usize = 16;

/// The length a sample of a number of time points is padded to with `NaN`: the next power of
/// two, at least [`MIN_PADDED_LENGTH`], so a parameter of the analysis depends on its sample
/// alone (`_padded` of sbmlsim).
#[must_use]
pub fn padded_length(points: usize) -> usize {
    MIN_PADDED_LENGTH.max(points.max(1).next_power_of_two())
}

/// The largest (`max`) or the smallest value of the rows of a finite time and the first row it
/// is at, ignoring `NaN` as `fmax` and `fmin` of numpy do; `NaN` and no row without such a
/// value.
#[must_use]
pub fn extreme(time: &[f64], max: bool, value: impl Fn(usize) -> f64) -> (f64, Option<usize>) {
    let mut best: (f64, Option<usize>) = (f64::NAN, None);
    for (row, t) in time.iter().enumerate() {
        if !t.is_finite() {
            continue;
        }
        let x = value(row);
        if x.is_nan() {
            continue;
        }
        let better = match best.1 {
            None => true,
            Some(_) if max => x > best.0,
            Some(_) => x < best.0,
        };
        if better {
            best = (x, Some(row));
        }
    }
    best
}

/// The time weighted mean of the rows, `_mean` of sbmlsim operation for operation: the
/// trapezoidal area, `0.5 * (x[r] + x[r - 1]) * (t[r] - t[r - 1])` summed in the order of the
/// rows as `cumsum` sums (an interval with a time which is not finite adds zero), over the time
/// between the first and the last finite time point; the value at the last finite time point
/// where there is no time between, `NaN` without one.
#[must_use]
pub fn mean(time: &[f64], value: impl Fn(usize) -> f64) -> f64 {
    let mut area = 0.0;
    for row in 1..time.len() {
        let (t0, t1) = (time[row - 1], time[row]);
        let piece = if t0.is_finite() && t1.is_finite() {
            0.5 * (value(row) + value(row - 1)) * (t1 - t0)
        } else {
            0.0
        };
        // the first piece is the start of the sum, as the first element of `cumsum`
        area = if row == 1 { piece } else { area + piece };
    }
    let finite = || (0..time.len()).filter(|&row| time[row].is_finite());
    let Some(last) = finite().next_back() else {
        return f64::NAN;
    };
    let span = finite().map(|row| time[row]).fold(f64::NAN, f64::max)
        - finite().map(|row| time[row]).fold(f64::NAN, f64::min);
    if span > 0.0 { area / span } else { value(last) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_at_a_change_sees_the_state_after_it_unless_it_reads_the_state_before() {
        let times = [0.0, 1.0, 1.0, 2.0];
        let after = grid_weights(&times, &[1.0, 0.5], 0.0, &[1.0], Some(&[false, false]));
        assert_eq!(
            (after[0].lower, after[0].upper, after[0].weight),
            (2, 2, 0.0)
        );
        assert_eq!(
            (after[1].lower, after[1].upper, after[1].weight),
            (0, 1, 0.5)
        );
        let before = grid_weights(&times, &[1.0], 0.0, &[1.0], Some(&[true]));
        assert_eq!((before[0].lower, before[0].upper), (1, 1));
        let outside = grid_weights(&times, &[3.0], 0.0, &[1.0], None);
        assert!(!outside[0].valid);
        // by default the first of two equal times of the grid reads the state before
        let both = grid_weights(&times, &[1.0, 1.0], 0.0, &[1.0], None);
        assert_eq!((both[0].lower, both[1].lower), (1, 2));
    }

    #[test]
    fn the_samples_leave_out_the_state_before_a_change_and_the_times_not_finite() {
        let time = [0.0, 1.0, 2.0, 2.0, 3.0, f64::INFINITY];
        assert_eq!(kept_rows(&time, &[2.0]), [0, 1, 3, 4]);
        assert_eq!(kept_rows(&time, &[]), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn the_padded_length_is_the_next_power_of_two_and_at_least_sixteen() {
        assert_eq!(padded_length(0), 16);
        assert_eq!(padded_length(16), 16);
        assert_eq!(padded_length(17), 32);
        assert_eq!(padded_length(874), 1024);
    }

    #[test]
    fn the_reductions_of_no_time_point_and_of_one() {
        assert!(mean(&[], |_| 1.0).is_nan());
        assert!(mean(&[f64::NAN], |_| 1.0).is_nan());
        assert_eq!(mean(&[2.0], |_| 3.0), 3.0);
        let time = [0.0, 1.0, 1.0, 2.0];
        let values = [1.0, 1.0, 3.0, 3.0];
        assert_eq!(mean(&time, |r| values[r]), 2.0);
        assert_eq!(extreme(&time, true, |r| values[r]), (3.0, Some(2)));
        let (value, row) = extreme(&[f64::NAN], false, |_| 1.0);
        assert!(value.is_nan() && row.is_none());
    }
}

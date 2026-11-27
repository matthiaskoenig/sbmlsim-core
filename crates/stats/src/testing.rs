//! The probability statements about predictions, robust to the number of draws, as
//! `sbmlsim.uncertainty.testing` makes them: the anytime-valid p-values and the confidence
//! sequence of a probability from whether every draw satisfies a hypothesis, in the order of the
//! draws.
//!
//! The mixture of the likelihood ratios of the first `m` draws, of which `k_m` satisfy the
//! hypothesis, `M_m(p) = B(a + k_m, b + m - k_m) / B(a, b) / (p^k_m (1 - p)^(m - k_m))` with
//! the Jeffreys mixture `a = b = 1/2` ([`MIXTURE`]), is a nonnegative martingale of mean 1
//! under the true probability (Robbins 1970; Lai 1976; Howard et al. 2021). Its anytime-valid
//! p-value of `P > p0` (`P < p0`) after `m` draws is the running minimum of `1 / M(p0)` over
//! the numbers of draws whose estimate is above (below) `p0` ([`p_values`]), the equivalence
//! with `p0` within a margin the larger p-value of the two one-sided tests of the ends of the
//! margin (Schuirmann 1987). The confidence sequence is the intersection of the sets
//! `{p: M_m(p) < 1 / alpha}` of every number of draws, its ends found by bisections
//! ([`confidence_sequence`]). [`decision`] decides a statement from its p-values, [`needed`]
//! gives the draws which would decide an undecided one at its estimate, and [`family_level`]
//! the level of a statement among several (Bonferroni).
//!
//! The arithmetic is the python's, operation by operation; the logarithm of the beta function
//! is the crate's ([`special::ln_beta`]), which agrees with scipy's `betaln` to about 1e-15.

use crate::python::{FLOAT_INFINITY, FLOAT_NAN, math_log, python_div};
use crate::{Result, StatsError, special};

/// The parameters `(a, b)` of the Jeffreys mixture of the confidence sequence, `MIXTURE`.
pub const MIXTURE: (f64, f64) = (0.5, 0.5);

/// The margin of a threshold by default, a difference of probability which does not matter.
pub const MARGIN: f64 = 0.05;

/// The most draws [`needed`] searches, `MAX_NEEDED`.
pub const MAX_NEEDED: u64 = 1 << 26;

/// The iterations of the bisection of each end of the confidence sequence, `BISECTIONS`.
pub const BISECTIONS: usize = 52;

/// The alternative of a p-value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `P > p0` (of the null `P <= p0`), `greater`.
    Greater,
    /// `P < p0` (of the null `P >= p0`), `less`.
    Less,
    /// `|P - p0| < margin`, the two one-sided tests of `p0 - margin` and `p0 + margin`.
    Equivalent,
}

/// The decision of a statement with a threshold, `_decide`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// `P >= p0` is shown, `accepted`.
    Accepted,
    /// `P < p0` is shown, `rejected`.
    Rejected,
    /// `P` within the margin of `p0` is shown, `equivalent`.
    Equivalent,
    /// Nothing is shown, `undecided`.
    Undecided,
}

/// The counts of the first `m` draws for every `m`, `_Prefixes`: `k_m` of them satisfy the
/// hypothesis, and the part of `log M_m(p)` which does not depend on `p`.
struct Prefixes {
    k: Vec<f64>,
    m: Vec<f64>,
    base: Vec<f64>,
}

impl Prefixes {
    /// The prefixes of the counts `k` of the numbers of draws `m`, `_Prefixes(k, m)`:
    /// `betaln(a + k, b + m - k) - betaln(a, b)` of every prefix.
    fn new(k: Vec<f64>, m: Vec<f64>) -> Self {
        let (a, b) = MIXTURE;
        let mixture = special::ln_beta(a, b);
        let base = k
            .iter()
            .zip(&m)
            .map(|(&k, &m)| special::ln_beta(a + k, b + m - k) - mixture)
            .collect();
        Self { k, m, base }
    }

    /// The prefixes of the draws, `_Prefixes.of`: the running count as a double and the
    /// numbers of draws 1 to `n`.
    fn of(satisfied: &[bool]) -> Self {
        let mut count = 0.0;
        let k = satisfied
            .iter()
            .map(|&x| {
                count += f64::from(u8::from(x));
                count
            })
            .collect();
        let m = (1..=satisfied.len()).map(|m| m as f64).collect();
        Self::new(k, m)
    }

    /// The number of prefixes.
    fn len(&self) -> usize {
        self.m.len()
    }

    /// The estimate `k / m` of the prefix `i`.
    fn mle(&self, i: usize) -> f64 {
        self.k[i] / self.m[i]
    }

    /// `log M_m(p)` of the prefix `i` of `log(p)` and `log1p(-p)`, `log_mixture`.
    fn log_mixture(&self, i: usize, log_p: f64, log_q: f64) -> f64 {
        self.base[i] - self.k[i] * log_p - (self.m[i] - self.k[i]) * log_q
    }

    /// The anytime-valid p-values of `P > p0` (`greater`) or `P < p0` after every prefix,
    /// `p_values`: the running minimum of `1 / M_m(p0)` over the prefixes whose estimate is on
    /// the side of the alternative, 0 of a certain and 1 of an impossible alternative.
    fn p_values(&self, p0: f64, greater: bool) -> Vec<f64> {
        let (mut certain, mut impossible) = (p0 <= 0.0, p0 >= 1.0);
        if !greater {
            (certain, impossible) = (impossible, certain);
        }
        if certain {
            return vec![0.0; self.len()];
        }
        if impossible {
            return vec![1.0; self.len()];
        }
        let (log_p, log_q) = (libm::log(p0), libm::log1p(-p0));
        let mut running = f64::INFINITY;
        (0..self.len())
            .map(|i| {
                let mle = self.mle(i);
                let on_side = if greater { mle > p0 } else { mle < p0 };
                let p = if on_side {
                    libm::exp(-self.log_mixture(i, log_p, log_q))
                } else {
                    1.0
                };
                // `np.clip` to [0, 1], then `np.minimum.accumulate`, which keeps a not a number
                let p = p.clamp(0.0, 1.0);
                running = if p.is_nan() || running.is_nan() {
                    f64::NAN
                } else {
                    running.min(p)
                };
                running
            })
            .collect()
    }

    /// Whether one of the first `n` prefixes excludes `p` below (`below`) or above its
    /// estimate at the logarithm `level` of `1 / alpha`, `outside`.
    fn outside(&self, n: usize, p: f64, level: f64, below: bool) -> bool {
        if !(0.0 < p && p < 1.0) {
            return false;
        }
        let (log_p, log_q) = (libm::log(p), libm::log1p(-p));
        (0..n).any(|i| {
            let mle = self.mle(i);
            let side = if below { mle > p } else { mle < p };
            side && self.log_mixture(i, log_p, log_q) >= level
        })
    }

    /// The intersection of the confidence sets of the first `n` prefixes, `bounds`: the
    /// largest lower and the smallest upper end, each by [`BISECTIONS`] bisections.
    fn bounds(&self, n: usize, level: f64) -> (f64, f64) {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..BISECTIONS {
            let mid = (lo + hi) / 2.0;
            if self.outside(n, mid, level, true) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let low = lo;
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..BISECTIONS {
            let mid = (lo + hi) / 2.0;
            if self.outside(n, mid, level, false) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        (low, hi)
    }

    /// The p-values of the side `side` of `p0` after every prefix, `_p_values`: of the
    /// equivalence the larger of `P > p0 - margin` and `P < p0 + margin`.
    fn side_p_values(&self, p0: f64, side: Side, margin: f64) -> Vec<f64> {
        match side {
            Side::Greater => self.p_values(p0, true),
            Side::Less => self.p_values(p0, false),
            Side::Equivalent => {
                let greater = self.p_values(p0 - margin, true);
                let less = self.p_values(p0 + margin, false);
                // `np.maximum`, which keeps a not a number
                greater
                    .into_iter()
                    .zip(less)
                    .map(|(a, b)| {
                        if a.is_nan() || b.is_nan() {
                            f64::NAN
                        } else {
                            a.max(b)
                        }
                    })
                    .collect()
            }
        }
    }
}

/// The anytime-valid p-values of the side `side` of the probability `p0` after every number of
/// draws, `_Prefixes.p_values` and `_p_values` of `sbmlsim.uncertainty.testing`.
///
/// `satisfied` is whether every draw satisfies the hypothesis, in the order of the draws. The
/// p-value of [`Side::Greater`] after `m` draws is the smallest `1 / M_j(p0)` of the first `j
/// <= m` draws whose estimate is above `p0` (1 without one), clipped to `[0, 1]`; 0 for a
/// threshold at most 0 (certain) and 1 for one at least 1 (impossible). [`Side::Less`] is the
/// mirror, and [`Side::Equivalent`] the larger p-value of `P > p0 - margin` and `P < p0 +
/// margin` (Schuirmann 1987); `margin` is used by it only. A threshold which is not a number is
/// a p-value of 1. No draws are no p-values.
#[must_use]
pub fn p_values(satisfied: &[bool], p0: f64, side: Side, margin: f64) -> Vec<f64> {
    Prefixes::of(satisfied).side_p_values(p0, side, margin)
}

/// The confidence sequence of the probability that a draw satisfies the hypothesis after the
/// draws, at the level `1 - alpha`, `_sequence` (and `confidence_sequence` of the level `1 -
/// alpha`) of `sbmlsim.uncertainty.testing`.
///
/// The lower and the upper end of the intersection of the sets `{p: M_m(p) < 1 / alpha}` of
/// every number of draws `m`, each end by [`BISECTIONS`] bisections; should the sets of two
/// numbers of draws exclude each other (the lower end above the upper one), the intersection
/// of the most draws whose sets intersect, by a bisection on the number of draws. `(0, 1)`
/// without draws.
///
/// # Errors
///
/// The errors of `math.log(1.0 / alpha)` of python, after at least one draw: the
/// ZeroDivisionError `division by zero` of an alpha of 0 and the ValueError `expected a
/// positive input, got <1 / alpha>` of an alpha below 0 or infinite.
pub fn confidence_sequence(satisfied: &[bool], alpha: f64) -> Result<(f64, f64)> {
    let n = satisfied.len();
    if n == 0 {
        return Ok((0.0, 1.0));
    }
    let prefixes = Prefixes::of(satisfied);
    let level = math_log(python_div(1.0, alpha)?)?;
    let (low, high) = prefixes.bounds(n, level);
    if low <= high {
        return Ok((low, high));
    }
    let (mut good, mut bad) = (1, n);
    while bad - good > 1 {
        let middle = (good + bad) / 2;
        let (low, high) = prefixes.bounds(middle, level);
        if low <= high {
            good = middle;
        } else {
            bad = middle;
        }
    }
    Ok(prefixes.bounds(good, level))
}

/// The number of draws at which a p-value first is at most `alpha`, the draws `m[_first(p <=
/// alpha)]` of `_decide`; `None` if it never is.
#[must_use]
pub fn decide(p_values: &[f64], alpha: f64) -> Option<usize> {
    p_values.iter().position(|&p| p <= alpha).map(|i| i + 1)
}

/// The decision of a statement from its p-values after every number of draws and the number of
/// draws at which it was first shown, `_decide`: the direction shown first ([`Decision::Accepted`]
/// on a tie), else the equivalence, else [`Decision::Undecided`] without a number of draws.
#[must_use]
pub fn decision(
    p_greater: &[f64],
    p_less: &[f64],
    p_equivalent: &[f64],
    alpha: f64,
) -> (Decision, Option<usize>) {
    let (greater, less) = (decide(p_greater, alpha), decide(p_less, alpha));
    match (greater, less) {
        (Some(g), l) if l.is_none_or(|l| g <= l) => (Decision::Accepted, Some(g)),
        (_, Some(l)) => (Decision::Rejected, Some(l)),
        _ => match decide(p_equivalent, alpha) {
            Some(e) => (Decision::Equivalent, Some(e)),
            None => (Decision::Undecided, None),
        },
    }
}

/// The number of draws which decides a statement at its estimate, `_needed`: the draws doubled
/// from `n` until `round(estimate * m)` of `m` draws decides the threshold `p0` with the margin
/// at `alpha`, `None` once the draws reach [`MAX_NEEDED`] undecided (the last doubling may pass
/// it) and of no draws, where the doubling of the python would not end.
///
/// The estimate is a fraction of the draws, in `[0, 1]`. Outside, a count of the draws is
/// negative or above the draws, where scipy's `betaln` of the python continues the beta function
/// to negative parameters and [`special::ln_beta`] is not a number, so the python may decide
/// where the crate does not.
///
/// # Errors
///
/// The errors of `round` of python of an estimate which is not finite: the ValueError `cannot
/// convert float NaN to integer` and the OverflowError `cannot convert float infinity to
/// integer`.
pub fn needed(estimate: f64, n: u64, p0: f64, margin: f64, alpha: f64) -> Result<Option<u64>> {
    if n == 0 {
        return Ok(None);
    }
    let mut m = n;
    while m < MAX_NEEDED {
        m *= 2;
        let k = python_round(estimate * m as f64)?;
        let prefix = Prefixes::new(vec![k], vec![m as f64]);
        let [greater, less, equivalent] = [Side::Greater, Side::Less, Side::Equivalent]
            .map(|side| prefix.side_p_values(p0, side, margin));
        if decision(&greater, &less, &equivalent, alpha).0 != Decision::Undecided {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// `round(x)` of python as a double: to the nearest integer, a half to the even one.
fn python_round(x: f64) -> Result<f64> {
    if x.is_nan() {
        return Err(StatsError(FLOAT_NAN.to_owned()));
    }
    if x.is_infinite() {
        return Err(StatsError(FLOAT_INFINITY.to_owned()));
    }
    Ok(libm::rint(x))
}

/// The level of a statement with a threshold in a report of `m` of them, `family_level` of the
/// adjustment `bonferroni`: `1 - (1 - level) / m`, the level itself for `m` at most 1.
#[must_use]
pub fn family_level(level: f64, m: i64) -> f64 {
    if m <= 1 {
        return level;
    }
    1.0 - (1.0 - level) / m as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_rounds_a_half_to_the_even_integer() {
        for (x, rounded) in [(0.5, 0.0), (1.5, 2.0), (2.5, 2.0), (-0.5, -0.0), (2.6, 3.0)] {
            assert_eq!(python_round(x), Ok(rounded), "{x}");
        }
        assert_eq!(python_round(f64::NAN).unwrap_err().0, FLOAT_NAN);
        assert_eq!(
            python_round(f64::NEG_INFINITY).unwrap_err().0,
            FLOAT_INFINITY
        );
    }

    #[test]
    fn the_p_values_of_no_draws_and_of_degenerate_thresholds() {
        assert!(p_values(&[], 0.5, Side::Greater, MARGIN).is_empty());
        let x = [true, true, false];
        assert_eq!(p_values(&x, 0.0, Side::Greater, MARGIN), [0.0; 3]);
        assert_eq!(p_values(&x, 0.0, Side::Less, MARGIN), [1.0; 3]);
        assert_eq!(p_values(&x, 1.0, Side::Less, MARGIN), [0.0; 3]);
        assert_eq!(p_values(&x, f64::NAN, Side::Equivalent, MARGIN), [1.0; 3]);
    }
}

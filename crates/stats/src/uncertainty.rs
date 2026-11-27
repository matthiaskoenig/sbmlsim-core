//! The statistics of the draws of an uncertainty analysis with their errors and the parameters
//! of a reported interval, as `sbmlsim.uncertainty` computes them.
//!
//! The statistics of draws, as `sbmlsim.uncertainty.statistics`: the mean and its standard
//! error ([`mean_se`]), the standard deviation and its standard error by the delta method
//! ([`sd_se`]), a quantile and its distribution-free interval of order statistics
//! ([`quantile_ci`]) and the first-order index of an input from given draws, by binning
//! ([`first_order_given_data`]). The precision of the draws of an output, `_precise` of
//! `sbmlsim.uncertainty.propagation` ([`precise`]), the quantiles of numpy's linear method
//! ([`quantile`], [`quantiles`]) and the parameters of the distribution of a reported
//! interval, `Uncertain.interval` ([`interval_parameters`]).
//!
//! Every function drops the draws which are not a number first, as the python does; an
//! infinite draw is a value. The edges are the ones of the python: `nan` where it returns
//! `nan`, and an error with the message of the exception where it raises one. The python
//! raises more than a `ValueError` there (an `OverflowError` of a power or an exponential past
//! the largest double, a `ZeroDivisionError`, an `IndexError` of numpy), whose class
//! [`exception`] names. A sum is numpy's pairwise sum, so a mean or a variance is the one of
//! numpy to the last bits.

use crate::numpy::{numpy_mean, numpy_quantile_sorted, numpy_var};
use crate::python::{
    DIVISION_BY_ZERO, MATH_RANGE, POWER_OVERFLOW, math_exp, math_expm1, python_div, python_max,
    python_max_of, python_min, python_pow, python_repr, python_sum,
};
use crate::{Result, StatsError, special};

/// The message numpy refuses a probability of a quantile outside `[0, 1]` with.
const QUANTILES_RANGE: &str = "Quantiles must be in the range [0, 1]";

/// The start of the message of the IndexError of numpy of a boolean mask of another length.
const BOOLEAN_INDEX: &str = "boolean index did not match indexed array";

/// The class of the exception the python raised for an error of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    /// `ValueError`, of an argument the python refuses.
    Value,
    /// `OverflowError`, of a power or an exponential past the largest double.
    Overflow,
    /// `ZeroDivisionError`, of a division by zero.
    ZeroDivision,
    /// `IndexError`, of numpy indexing draws by a mask of the other draws.
    Index,
}

impl Exception {
    /// The name of the class of python.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Value => "ValueError",
            Self::Overflow => "OverflowError",
            Self::ZeroDivision => "ZeroDivisionError",
            Self::Index => "IndexError",
        }
    }
}

/// The class of the exception the python raised for an error of a function of this module.
#[must_use]
pub fn exception(error: &StatsError) -> Exception {
    match error.0.as_str() {
        POWER_OVERFLOW | MATH_RANGE => Exception::Overflow,
        DIVISION_BY_ZERO => Exception::ZeroDivision,
        message if message.starts_with(BOOLEAN_INDEX) => Exception::Index,
        _ => Exception::Value,
    }
}

/// The draws which are not a number, `_valid` of the python.
fn valid(x: &[f64]) -> Vec<f64> {
    x.iter().copied().filter(|value| !value.is_nan()).collect()
}

/// The draws which are not a number in ascending order, `np.sort(_valid(x))`.
fn sorted_valid(x: &[f64]) -> Vec<f64> {
    let mut values = valid(x);
    values.sort_by(|a, b| a.partial_cmp(b).expect("no draw is not a number"));
    values
}

/// The standard deviation of draws, `np.std(values, ddof=1)`.
fn std_ddof1(values: &[f64]) -> f64 {
    libm::sqrt(numpy_var(values, 1))
}

/// The mean, its standard error `s / sqrt(n)` and the number of draws which are not a number,
/// `mean_se`: `nan` without draws, the error `nan` without two of them and of infinite draws.
#[must_use]
pub fn mean_se(x: &[f64]) -> (f64, f64, usize) {
    let values = valid(x);
    let n = values.len();
    if n == 0 {
        return (f64::NAN, f64::NAN, 0);
    }
    let mean = numpy_mean(n, |i| values[i]);
    if n < 2 || !values.iter().all(|value| value.is_finite()) {
        // the standard deviation of infinite draws is not a number
        return (mean, f64::NAN, n);
    }
    (mean, std_ddof1(&values) / libm::sqrt(n as f64), n)
}

/// The standard deviation, its standard error by the delta method with the fourth central
/// moment, `sqrt((m4 - s^4) / n) / (2 s)`, and the number of draws which are not a number,
/// `sd_se`: `nan` without two draws and of infinite draws, 0 of draws without variance.
///
/// # Errors
///
/// The OverflowError of python of a fourth power of the standard deviation past the largest
/// double, `(34, 'Numerical result out of range')`.
pub fn sd_se(x: &[f64]) -> Result<(f64, f64, usize)> {
    let values = valid(x);
    let n = values.len();
    if n < 2 || !values.iter().all(|value| value.is_finite()) {
        return Ok((f64::NAN, f64::NAN, n));
    }
    let sd = std_ddof1(&values);
    if sd == 0.0 {
        return Ok((0.0, 0.0, n));
    }
    let mean = numpy_mean(n, |i| values[i]);
    // numpy's power of an exponent 4 is `pow` of the C library
    let m4 = numpy_mean(n, |i| libm::pow(values[i] - mean, 4.0));
    let excess = python_max(m4 - python_pow(sd, 4)?, 0.0);
    Ok((sd, libm::sqrt(excess / n as f64) / (2.0 * sd), n))
}

/// The error of a probability of a quantile outside `[0, 1]`, as numpy refuses it.
fn check_quantile(q: f64) -> Result<()> {
    if (0.0..=1.0).contains(&q) {
        Ok(())
    } else {
        Err(StatsError(QUANTILES_RANGE.to_owned()))
    }
}

/// The quantile `q` of the draws which are not a number, as `np.nanquantile` of the linear
/// method (the default) interpolates it: `nan` without such draws, and an infinite draw gives
/// an infinity or not a number as numpy does.
///
/// # Errors
///
/// A probability outside `[0, 1]` or not a number, with the message of numpy.
pub fn quantile(x: &[f64], q: f64) -> Result<f64> {
    check_quantile(q)?;
    Ok(numpy_quantile_sorted(&sorted_valid(x), q))
}

/// The quantiles `qs` of the draws which are not a number, `np.nanquantile(x, qs)`, see
/// [`quantile`].
///
/// # Errors
///
/// A probability outside `[0, 1]` or not a number, with the message of numpy.
pub fn quantiles(x: &[f64], qs: &[f64]) -> Result<Vec<f64>> {
    for &q in qs {
        check_quantile(q)?;
    }
    let values = sorted_valid(x);
    Ok(qs
        .iter()
        .map(|&q| numpy_quantile_sorted(&values, q))
        .collect())
}

/// The rank of a quantile of the binomial distribution, `int(stats.binom.ppf(...))`.
fn rank(probability: f64, n: usize, q: f64) -> Result<i64> {
    let rank = special::binom_ppf(probability, n as u64, q);
    if rank.is_nan() {
        return Err(StatsError("cannot convert float NaN to integer".to_owned()));
    }
    Ok(rank as i64)
}

/// A quantile, the lower and the upper end of its distribution-free interval and the number
/// of draws which are not a number, `quantile_ci`.
///
/// The ends are the order statistics `x_(r) <= x_(s)` whose ranks are the `(1 - level) / 2`
/// and `(1 + level) / 2` quantiles of the binomial distribution `B(n, q)` (Hahn & Meeker
/// 1991); an end beyond the draws is `nan`, and all are `nan` without draws. As the python, a
/// rank of 0 for the upper end (the level -1) is the last draw.
///
/// # Errors
///
/// A probability outside `[0, 1]` (numpy's message) and a level outside `[-1, 1]`, whose rank
/// is not a number (`cannot convert float NaN to integer`).
pub fn quantile_ci(x: &[f64], q: f64, level: f64) -> Result<(f64, f64, f64, usize)> {
    let values = sorted_valid(x);
    let n = values.len();
    if n == 0 {
        return Ok((f64::NAN, f64::NAN, f64::NAN, 0));
    }
    check_quantile(q)?;
    let estimate = numpy_quantile_sorted(&values, q);
    let alpha = 1.0 - level;
    let low_rank = rank(alpha / 2.0, n, q)?;
    let high_rank = rank(1.0 - alpha / 2.0, n, q)? + 1;
    let low = if low_rank >= 1 {
        values[(low_rank - 1) as usize]
    } else {
        f64::NAN
    };
    let high = match high_rank {
        // `values[-1]` of python
        0 => values[n - 1],
        rank if rank <= n as i64 => values[(rank - 1) as usize],
        _ => f64::NAN,
    };
    Ok((estimate, low, high, n))
}

/// The pairs of draws of an input and an output which are both finite, as the python masks
/// them (`x[finite]`, `y[finite]` of `finite = isfinite(x) & isfinite(y)`), or `None` where
/// numpy broadcasts one draw against none to no pairs.
fn finite_pairs(x: &[f64], y: &[f64]) -> Result<Option<(Vec<f64>, Vec<f64>)>> {
    match (x.len(), y.len()) {
        (a, b) if a == b => {}
        (1, 0) | (0, 1) => return Ok(None),
        (1, other) | (other, 1) => {
            return Err(StatsError(format!(
                "{BOOLEAN_INDEX} along axis 0; size of axis is 1 but size of corresponding \
                 boolean axis is {other}"
            )));
        }
        (a, b) => {
            return Err(StatsError(format!(
                "operands could not be broadcast together with shapes ({a},) ({b},) "
            )));
        }
    }
    Ok(Some(
        x.iter()
            .zip(y)
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|(&x, &y)| (x, y))
            .unzip(),
    ))
}

/// The first-order index of an input from given draws, by binning, `first_order_given_data`.
///
/// The draws are split into `n_bins` bins of equal probability of the input (`max(2,
/// sqrt(n) / 2)` by default; the order of the input stable, the sizes of `np.array_split`),
/// and the index is the variance of the means of the output in the bins over the variance of
/// the output, less `(n_bins - 1) / n` (Plischke, Borgonovo & Smith 2013), clipped to
/// `[0, 1]`: `nan` with fewer than 8 finite pairs, an output without variance or an empty
/// bin.
///
/// # Errors
///
/// Draws of other lengths (the messages of numpy), no bins (`number sections must be larger
/// than 0.`) and the OverflowError of python of a square of a bin past the largest double.
pub fn first_order_given_data(x: &[f64], y: &[f64], n_bins: Option<usize>) -> Result<f64> {
    let Some((xs, ys)) = finite_pairs(x, y)? else {
        return Ok(f64::NAN);
    };
    let n = xs.len();
    if n < 8 {
        return Ok(f64::NAN);
    }
    let variance = numpy_var(&ys, 0);
    if variance == 0.0 {
        return Ok(f64::NAN);
    }
    let bins = n_bins.unwrap_or_else(|| 2.max(libm::sqrt(n as f64) as usize / 2));
    if bins == 0 {
        return Err(StatsError(
            "number sections must be larger than 0.".to_owned(),
        ));
    }
    let mut order: Vec<usize> = (0..n).collect();
    // stable, as `np.argsort(kind="stable")`: ties keep the order of the draws
    order.sort_by(|&a, &b| xs[a].partial_cmp(&xs[b]).expect("the input is finite"));
    let sorted: Vec<f64> = order.iter().map(|&i| ys[i]).collect();
    let mean = numpy_mean(n, |i| ys[i]);
    // the sizes of `np.array_split`: the first `n % bins` bins one larger
    let (each, extras) = (n / bins, n % bins);
    // more bins than pairs: the bins past the pairs are empty, the mean of one not a number,
    // which the python adds after the bins of one pair each (so no loop over all of them)
    let filled = bins.min(n);
    let mut items = Vec::with_capacity(filled + 1);
    let mut start = 0;
    for bin in 0..filled {
        let size = each + usize::from(bin < extras);
        let group = &sorted[start..start + size];
        let group_mean = numpy_mean(size, |i| group[i]);
        items.push(size as f64 * python_pow(group_mean - mean, 2)?);
        start += size;
    }
    if bins > n {
        items.push(f64::NAN);
    }
    let between = python_sum(items) / n as f64;
    let index = between / variance - (bins - 1) as f64 / n as f64;
    Ok(python_min(python_max(index, 0.0), 1.0))
}

/// Whether the statistics of the draws of an output are precise enough, and the largest half
/// width relative to the standard deviation, `_precise` of `sbmlsim.uncertainty.propagation`.
///
/// The half widths of the 95 % intervals of the mean, the standard deviation and the 5 % and
/// 95 % quantiles must be at most `rel_tol` times the standard deviation of the output (an
/// end of an interval beyond the draws an infinite width): not with fewer than 8 draws which
/// are not a number (the width infinite), and of draws without variance (the width 0).
///
/// # Errors
///
/// The OverflowError of python of [`sd_se`].
pub fn precise(values: &[f64], rel_tol: f64) -> Result<(bool, f64)> {
    let finite = valid(values);
    if finite.len() < 8 {
        return Ok((false, f64::INFINITY));
    }
    let (sd, sd_error, _) = sd_se(&finite)?;
    if sd == 0.0 {
        return Ok((true, 0.0));
    }
    let mut widths = vec![1.96 * mean_se(&finite).1, 1.96 * sd_error];
    for q in [0.05, 0.95] {
        let (_, low, high, _) = quantile_ci(&finite, q, 0.95)?;
        widths.push(if low.is_finite() && high.is_finite() {
            0.5 * (high - low)
        } else {
            f64::INFINITY
        });
    }
    let largest = python_max_of(&widths) / sd;
    Ok((largest <= rel_tol, largest))
}

/// Whether `a > b`, false of a not a number: the python's `not a > b` is `!greater(a, b)`.
fn greater(a: f64, b: f64) -> bool {
    a > b
}

/// The parameters of the distribution of a reported interval, `Uncertain.interval`: the
/// bounds are the quantiles `(1 - level) / 2` and `(1 + level) / 2` of a log-normal (the
/// median and the coefficient of variation) or of a normal (the mean and the standard
/// deviation). The choice of `shape="auto"`, a log-normal of a positive interval, is the
/// caller's.
///
/// # Errors
///
/// A level outside `(0, 1)`, an empty interval, a log-normal interval which is not positive,
/// and a spread of the distribution which is not positive, with the messages of the python
/// (the numbers as python writes floats); the ZeroDivisionError of a level so small that the
/// quantile is 0 and the OverflowError of the exponential of a log-normal past the largest
/// double.
pub fn interval_parameters(low: f64, high: f64, level: f64, lognormal: bool) -> Result<(f64, f64)> {
    if !(0.0 < level && level < 1.0) {
        return Err(StatsError(format!(
            "The level {} of an interval must be in (0, 1).",
            python_repr(level)
        )));
    }
    let bounds = format!("[{}, {}]", python_repr(low), python_repr(high));
    if !greater(high, low) {
        return Err(StatsError(format!("The interval {bounds} is empty.")));
    }
    let z = special::normal_ppf(0.5 + level / 2.0);
    if lognormal {
        if !greater(low, 0.0) {
            return Err(StatsError(format!(
                "The log-normal interval {bounds} must be positive."
            )));
        }
        let (log_low, log_high) = (libm::log(low), libm::log(high));
        let mu = 0.5 * (log_low + log_high);
        let sigma = python_div(log_high - log_low, 2.0 * z)?;
        let median = math_exp(mu)?;
        let cv = libm::sqrt(math_expm1(python_pow(sigma, 2)?)?);
        if !greater(cv, 0.0) {
            return Err(StatsError(format!(
                "LogNormal(median={}, cv={}) needs a positive coefficient of variation.",
                python_repr(median),
                python_repr(cv)
            )));
        }
        Ok((median, cv))
    } else {
        let mean = 0.5 * (low + high);
        let sd = python_div(high - low, 2.0 * z)?;
        if !greater(sd, 0.0) {
            return Err(StatsError(format!(
                "Normal(mean={}, sd={}, cv=None) needs a positive standard deviation.",
                python_repr(mean),
                python_repr(sd)
            )));
        }
        Ok((mean, sd))
    }
}

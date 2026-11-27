//! The metrics of a fit and the criteria of its evaluation, as `sbmlsim.fit.metrics` and
//! `sbmlsim.fit.evaluation` compute them.
//!
//! The metrics of residuals: the sum of squares ([`sse`]), the mean square ([`mse`]), its root
//! ([`rmse_from_mse`]), the information criteria of a least squares fit ([`aic_from_mse`],
//! [`bic_from_mse`]), the coefficient of determination ([`r_squared`]) and the root mean square
//! of the normalized residuals ([`nrmse`]).
//!
//! The criteria of the data points, a measured value `y` and its prediction `f`: the fold
//! criterion `y/fold <= f <= y*fold` of the points with a ratio, a positive and finite value
//! and prediction ([`fold_counts`], [`fold_accepted`]), the uncertainty criterion
//! `|y - f| <= k sd` of the points with a standard deviation ([`sd_accepted`]), the fold errors
//! ([`gmfe`], [`afe`]) and the limits of agreement of a subset ([`subset_agreement`]), and the
//! criterion of Guest et al. for ratios ([`guest_limits`], [`within_guest`]).
//!
//! The edges are the ones of the python: `nan` where it returns `nan`, `-inf` for the
//! information criteria of a perfect fit, and an error with the message of the `ValueError`
//! where it raises one. A sum is numpy's pairwise sum, so a metric is the one of numpy to the
//! last bits, not only to its rounding.

use crate::numpy::{numpy_mean, numpy_sum};
use crate::python::python_repr;
use crate::{Result, StatsError, special};

/// The points with a ratio a subset needs for its own limits of agreement, see
/// [`subset_agreement`].
pub const MIN_POINTS_SUBSET_AGREEMENT: usize = 10;

/// The error of a fold criterion of points of another number of predictions than values.
fn check_lengths(y: &[f64], f: &[f64]) -> Result<()> {
    if y.len() == f.len() {
        return Ok(());
    }
    Err(StatsError(format!(
        "The fold criterion requires as many predictions as data points, but got '{}' for \
         '{}'.",
        f.len(),
        y.len()
    )))
}

/// The error of a fold factor which is not larger than 1.
fn check_fold(fold: f64) -> Result<()> {
    if fold > 1.0 {
        return Ok(());
    }
    Err(StatsError(format!(
        "The fold factor must be larger than 1, but '{}' given.",
        python_repr(fold)
    )))
}

/// Sum of Squared Errors (SSE) of the residuals, 0 of none.
pub fn sse(residuals: &[f64]) -> f64 {
    numpy_sum(residuals.len(), |i| residuals[i] * residuals[i])
}

/// Mean Squared Error (MSE) of the residuals.
///
/// # Errors
///
/// No residual.
pub fn mse(residuals: &[f64]) -> Result<f64> {
    if residuals.is_empty() {
        return Err(StatsError("MSE requires at least one residual.".to_owned()));
    }
    Ok(numpy_mean(residuals.len(), |i| residuals[i] * residuals[i]))
}

/// Root Mean Squared Error (RMSE) from the mean squared error.
///
/// # Errors
///
/// A negative mean squared error.
pub fn rmse_from_mse(mse: f64) -> Result<f64> {
    if mse < 0.0 {
        return Err(StatsError(format!(
            "RMSE requires a non-negative MSE, but '{}' given.",
            python_repr(mse)
        )));
    }
    Ok(libm::sqrt(mse))
}

/// The checks of the information criterion `name` of a mean squared error of `n` points.
fn check_criterion(name: &str, mse: f64, n: i64) -> Result<()> {
    if mse < 0.0 {
        return Err(StatsError(format!(
            "{name} requires a non-negative MSE, but '{}' given.",
            python_repr(mse)
        )));
    }
    if n <= 0 {
        return Err(StatsError(format!(
            "{name} requires a positive number of points, but '{n}' given."
        )));
    }
    Ok(())
}

/// Akaike Information Criterion (AIC) of a least squares fit from its mean squared error,
/// `n ln(MSE) + 2 k` of `n` data points and `k` fitted parameters, `-inf` for an MSE of 0.
///
/// # Errors
///
/// A negative mean squared error or a number of points which is not positive.
pub fn aic_from_mse(mse: f64, n: i64, k: i64) -> Result<f64> {
    check_criterion("AIC", mse, n)?;
    if mse == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    // 2 k of python's integers as a double: the double of k doubled, exact, and no overflow
    Ok(n as f64 * libm::log(mse) + 2.0 * k as f64)
}

/// Bayesian Information Criterion (BIC) of a least squares fit from its mean squared error,
/// `n ln(MSE) + k ln(n)` of `n` data points and `k` fitted parameters, `-inf` for an MSE of 0.
///
/// # Errors
///
/// A negative mean squared error or a number of points which is not positive.
pub fn bic_from_mse(mse: f64, n: i64, k: i64) -> Result<f64> {
    check_criterion("BIC", mse, n)?;
    if mse == 0.0 {
        return Ok(f64::NEG_INFINITY);
    }
    Ok(n as f64 * libm::log(mse) + k as f64 * libm::log(n as f64))
}

/// Coefficient of determination (R²) of a prediction, `1 - SSE / SST`, negative for a
/// prediction worse than the mean of the data; `nan` for fewer than two data points and for
/// data without variance (where the python logs a warning).
///
/// # Errors
///
/// Another number of predictions than of data points.
pub fn r_squared(y_observed: &[f64], y_predicted: &[f64]) -> Result<f64> {
    if y_observed.len() != y_predicted.len() {
        return Err(StatsError(format!(
            "R² requires as many predictions as data points, but got '{}' for '{}'.",
            y_predicted.len(),
            y_observed.len()
        )));
    }
    let n = y_observed.len();
    if n < 2 {
        return Ok(f64::NAN);
    }
    let mean = numpy_mean(n, |i| y_observed[i]);
    let sst = numpy_sum(n, |i| {
        let d = y_observed[i] - mean;
        d * d
    });
    if sst == 0.0 {
        return Ok(f64::NAN);
    }
    let sse = numpy_sum(n, |i| {
        let d = y_observed[i] - y_predicted[i];
        d * d
    });
    Ok(1.0 - sse / sst)
}

/// Whether a point has a ratio: a positive and finite value and prediction.
fn has_ratio(y: f64, f: f64) -> bool {
    y.is_finite() && f.is_finite() && y > 0.0 && f > 0.0
}

/// Whether a prediction is within the fold factor of its value, the bounds as products, so
/// that twofold is exact in floating point.
fn within(y: f64, f: f64, fold: f64) -> bool {
    f >= y / fold && f <= y * fold
}

/// The number of data points whose prediction is within the fold factor, `1/fold <= f/y <=
/// fold` with the bounds, and the number `n_+` of the points with a ratio, a positive and
/// finite value and prediction, which are the ones counted.
///
/// # Errors
///
/// A fold factor which is not larger than 1, another number of predictions than of values.
pub fn fold_counts(y: &[f64], f: &[f64], fold: f64) -> Result<(usize, usize)> {
    check_fold(fold)?;
    check_lengths(y, f)?;
    let (mut inside, mut n_ratio) = (0, 0);
    for (&y, &f) in y.iter().zip(f) {
        if has_ratio(y, f) {
            n_ratio += 1;
            inside += usize::from(within(y, f, fold));
        }
    }
    Ok((inside, n_ratio))
}

/// The fold criterion of every point, `Fold(fold).accepted`: 1 within the fold factor, 0
/// outside and `nan` for a point without a ratio.
///
/// # Errors
///
/// A fold factor which is not larger than 1, another number of predictions than of values.
pub fn fold_accepted(y: &[f64], f: &[f64], fold: f64) -> Result<Vec<f64>> {
    check_fold(fold)?;
    check_lengths(y, f)?;
    Ok(y.iter()
        .zip(f)
        .map(|(&y, &f)| {
            if has_ratio(y, f) {
                f64::from(u8::from(within(y, f, fold)))
            } else {
                f64::NAN
            }
        })
        .collect())
}

/// The uncertainty criterion of every point, `Uncertainty(k).accepted`: 1 for a prediction
/// within `k` standard deviations, `|y - f| <= k sd`, 0 outside and `nan` for a point without
/// a positive and finite standard deviation or with a value or prediction which is not finite.
///
/// # Errors
///
/// Another number of predictions or of standard deviations than of values.
pub fn sd_accepted(y: &[f64], f: &[f64], sd: &[f64], k: f64) -> Result<Vec<f64>> {
    if f.len() != y.len() || sd.len() != y.len() {
        return Err(StatsError(format!(
            "The uncertainty criterion requires as many predictions and standard deviations \
             as data points, but got '{}' and '{}' for '{}'.",
            f.len(),
            sd.len(),
            y.len()
        )));
    }
    Ok(y.iter()
        .zip(f)
        .zip(sd)
        .map(|((&y, &f), &sd)| {
            if sd.is_finite() && sd > 0.0 && f.is_finite() && y.is_finite() {
                f64::from(u8::from((y - f).abs() <= k * sd))
            } else {
                f64::NAN
            }
        })
        .collect())
}

/// `log10(f/y)` of the points with a ratio.
fn log_ratios(y: &[f64], f: &[f64]) -> Result<Vec<f64>> {
    check_lengths(y, f)?;
    Ok(y.iter()
        .zip(f)
        .filter(|&(&y, &f)| has_ratio(y, f))
        .map(|(&y, &f)| libm::log10(f / y))
        .collect())
}

/// The geometric mean fold error `10**mean(|log10(f/y)|)` of the points with a ratio, `nan`
/// without one.
///
/// # Errors
///
/// Another number of predictions than of values.
pub fn gmfe(y: &[f64], f: &[f64]) -> Result<f64> {
    let ratios = log_ratios(y, f)?;
    if ratios.is_empty() {
        return Ok(f64::NAN);
    }
    Ok(libm::pow(
        10.0,
        numpy_mean(ratios.len(), |i| ratios[i].abs()),
    ))
}

/// The average fold error `10**mean(log10(f/y))` of the points with a ratio, the bias as a
/// fold factor, `nan` without one.
///
/// # Errors
///
/// Another number of predictions than of values.
pub fn afe(y: &[f64], f: &[f64]) -> Result<f64> {
    let ratios = log_ratios(y, f)?;
    if ratios.is_empty() {
        return Ok(f64::NAN);
    }
    Ok(libm::pow(10.0, numpy_mean(ratios.len(), |i| ratios[i])))
}

/// The root mean square of the normalized residuals, `nan` without one or with one which is
/// not finite (`evaluation._rms`).
pub fn nrmse(nres: &[f64]) -> f64 {
    if nres.is_empty() || !nres.iter().all(|value| value.is_finite()) {
        return f64::NAN;
    }
    libm::sqrt(numpy_mean(nres.len(), |i| nres[i] * nres[i]))
}

/// The lower and the upper limit of the predicted ratio of an observed ratio `R` of the
/// criterion of Guest et al. (2011) with `delta = 1`: with `R` inverted below 1, the limit
/// `(1 + 2 (R - 1)) / R` and the predicted ratio accepted between `R / limit` and
/// `R * limit`; the limits of a ratio below 1 are the inverted limits of its inverse.
pub fn guest_limits(ratio: f64) -> (f64, f64) {
    let inverted = ratio < 1.0;
    let r = if inverted { 1.0 / ratio } else { ratio };
    let limit = (1.0 + 2.0 * (r - 1.0)) / r;
    let (low, high) = (r / limit, r * limit);
    if inverted {
        (1.0 / high, 1.0 / low)
    } else {
        (low, high)
    }
}

/// The percentage (0 to 100) of the finite predicted ratios within the limits of
/// [`guest_limits`] of their observed ratios, the limits included with a tolerance of 1e-12
/// relative; `nan` without a finite predicted ratio.
///
/// # Errors
///
/// Another number of observed than of predicted ratios, an observed ratio which is not
/// positive and finite.
pub fn within_guest(observed: &[f64], predicted: &[f64]) -> Result<f64> {
    if observed.len() != predicted.len() {
        return Err(StatsError(format!(
            "The observed and the predicted ratios have different lengths, {} and {}.",
            observed.len(),
            predicted.len()
        )));
    }
    if !observed.iter().all(|&r| r.is_finite() && r > 0.0) {
        let list: Vec<String> = observed.iter().map(|&r| python_repr(r)).collect();
        return Err(StatsError(format!(
            "The observed ratios must be positive and finite: [{}].",
            list.join(", ")
        )));
    }
    let rtol = 1e-12;
    let (mut inside, mut n) = (0_usize, 0_usize);
    for (&observed, &predicted) in observed.iter().zip(predicted) {
        if !predicted.is_finite() {
            continue;
        }
        n += 1;
        let (low, high) = guest_limits(observed);
        inside += usize::from(predicted >= low * (1.0 - rtol) && predicted <= high * (1.0 + rtol));
    }
    if n == 0 {
        return Ok(f64::NAN);
    }
    // np.mean of the booleans: their count, exact, over their number
    Ok(100.0 * (inside as f64 / n as f64))
}

/// The agreement of a subset of the data points (Bland and Altman 1999) of the points with a
/// ratio: the bias `mean(log10(f/y))`, the half width `1.96 SD` of the limits of agreement and
/// the half widths of the 95% confidence intervals of the bias, `t SD / sqrt(n)`, and of a
/// limit, `t sqrt(3 SD² / n)`, with `t` the 97.5% quantile of the t distribution of `n - 1`
/// degrees of freedom, in decades; `None` for fewer than [`MIN_POINTS_SUBSET_AGREEMENT`]
/// points with a ratio.
///
/// # Errors
///
/// Another number of predictions than of values.
pub fn subset_agreement(y: &[f64], f: &[f64]) -> Result<Option<(f64, f64, f64, f64)>> {
    let ratios = log_ratios(y, f)?;
    let n = ratios.len();
    if n < MIN_POINTS_SUBSET_AGREEMENT {
        return Ok(None);
    }
    let mean = numpy_mean(n, |i| ratios[i]);
    // np.std(ddof=1): the squares of the deviations from the mean, summed, over n - 1
    let variance = numpy_sum(n, |i| {
        let d = ratios[i] - mean;
        d * d
    }) / (n - 1) as f64;
    let sd = libm::sqrt(variance);
    let t = special::t_ppf(0.975, (n - 1) as f64);
    let count = n as f64;
    Ok(Some((
        mean,
        1.96 * sd,
        t * sd / libm::sqrt(count),
        t * libm::sqrt(3.0 * (sd * sd) / count),
    )))
}

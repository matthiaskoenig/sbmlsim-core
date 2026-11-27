//! The correlation matrices of an uncertainty analysis: the nearest correlation matrix, the
//! test of positive semi-definiteness, the validation of a covariance or a correlation and the
//! coefficient of the normal scores of a covariance of two marginals, as
//! `sbmlsim.uncertainty.covariance` and `sbmlsim.uncertainty.draws` give them.
//!
//! A matrix is a row-major slice of `k * k` doubles. The eigendecompositions are the ones of
//! [`linalg`](mod@crate::linalg) (the cyclic Jacobi method), so a result is numpy's to the
//! rounding of its eigenvectors, and the same bits on every target.
//!
//! Higham NJ (2002). Computing the nearest correlation matrix: a problem from finance. IMA J
//! Numer Anal 22:329-343.

use crate::linalg;
use crate::python::{python_div, python_format_g, python_repr, python_str_list};
use crate::{Result, StatsError};

/// The relative tolerance of the smallest eigenvalue of a positive semi-definite matrix,
/// relative to the largest one, `PSD_RTOL` of `sbmlsim.uncertainty.covariance`.
pub const PSD_RTOL: f64 = 1e-10;

/// The most iterations of [`nearest_correlation`], the default of the python.
pub const NEAREST_MAX_ITERATIONS: usize = 100;

/// The tolerance of the change of an iteration of [`nearest_correlation`] in the Frobenius
/// norm, the default of the python.
pub const NEAREST_TOL: f64 = 1e-10;

/// The message of the ValueError of numpy of the largest value of an empty matrix, which the
/// validation of a matrix without targets raises.
const EMPTY_MAXIMUM: &str = "zero-size array to reduction operation maximum which has no identity";

/// The marginal of a variable of a covariance as `_moments` of `sbmlsim.uncertainty.draws`
/// gives it, on the scale of the covariance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Marginal {
    /// A normal marginal of the standard deviation `sd`, of a covariance on the linear scale.
    Normal {
        /// The standard deviation.
        sd: f64,
    },
    /// A log-normal marginal of a covariance on the linear scale: its arithmetic mean and the
    /// standard deviation `sigma` of its logarithm.
    LogNormal {
        /// The arithmetic mean.
        mean: f64,
        /// The standard deviation of the logarithm.
        sigma: f64,
    },
    /// A log-normal marginal of a covariance on the log scale (of the logarithms): the standard
    /// deviation `sd` of its logarithm.
    Log {
        /// The standard deviation of the logarithm.
        sd: f64,
    },
}

/// The coefficient of the normal scores of the covariance `m_ab` of the marginals `a` and `b`,
/// the double loop of `normal_score_correlation` of `sbmlsim.uncertainty.draws`:
///
/// - normal marginals or a covariance of the logarithms: `m / (sd_a sd_b)`, the arithmetic of
///   numpy (infinite or not a number for a standard deviation zero)
/// - log-normal marginals of a covariance on the linear scale: the exact conversion
///   `log1p(m / (mean_a mean_b)) / (sigma_a sigma_b)`, the arithmetic of `math`
///
/// # Errors
///
/// Marginals of two families, which the python refuses before with the targets it names; and
/// for log-normal marginals the errors of `math`, a ratio `m / (mean_a mean_b)` of at most -1
/// (the ValueError of `math.log1p`) and a product of the standard deviations zero (the
/// ZeroDivisionError of python, see [`uncertainty::exception`](crate::uncertainty::exception)).
pub fn normal_score_rho(m_ab: f64, a: Marginal, b: Marginal) -> Result<f64> {
    match (a, b) {
        (Marginal::Normal { sd: sd_a }, Marginal::Normal { sd: sd_b })
        | (Marginal::Log { sd: sd_a }, Marginal::Log { sd: sd_b }) => Ok(m_ab / (sd_a * sd_b)),
        (
            Marginal::LogNormal {
                mean: mean_a,
                sigma: sigma_a,
            },
            Marginal::LogNormal {
                mean: mean_b,
                sigma: sigma_b,
            },
        ) => {
            let ratio = m_ab / (mean_a * mean_b);
            // math.log1p refuses -1 and below (python 3.14), a not a number passes
            if ratio <= -1.0 {
                return Err(StatsError(format!(
                    "expected argument value > -1, got {}",
                    python_repr(ratio)
                )));
            }
            python_div(libm::log1p(ratio), sigma_a * sigma_b)
        }
        _ => Err(StatsError(format!(
            "The normal scores of a covariance need marginals of one family, not {a:?} and \
             {b:?}."
        ))),
    }
}

/// The matrix of `k * k` values, or the error of a slice of another size.
fn check_size(m: &[f64], k: usize) -> Result<()> {
    if m.len() != k * k {
        return Err(StatsError(format!(
            "A matrix of {k} x {k} needs {} values, not {}.",
            k * k,
            m.len()
        )));
    }
    Ok(())
}

/// The Frobenius norm of a matrix.
fn frobenius(m: &[f64]) -> f64 {
    libm::sqrt(m.iter().map(|v| v * v).sum())
}

/// The projection of the symmetric part of `r` onto the positive semi-definite matrices,
/// `V clip(values, 0) Vᵀ` of the eigendecomposition of `(r + rᵀ) / 2`.
fn psd_projection(r: &[f64], k: usize) -> Result<Vec<f64>> {
    let mut symmetric = vec![0.0; k * k];
    for i in 0..k {
        for j in 0..k {
            symmetric[i * k + j] = (r[i * k + j] + r[j * k + i]) / 2.0;
        }
    }
    let (values, vectors) = linalg::eigh(&symmetric, k)?;
    let clipped: Vec<f64> = values.iter().map(|&v| v.max(0.0)).collect();
    let mut x = vec![0.0; k * k];
    for i in 0..k {
        for j in 0..k {
            let mut sum = 0.0;
            for (p, &value) in clipped.iter().enumerate() {
                sum += vectors[i * k + p] * value * vectors[j * k + p];
            }
            x[i * k + j] = sum;
        }
    }
    Ok(x)
}

/// The nearest correlation matrix of the `k x k` matrix `m` in the Frobenius norm,
/// `nearest_correlation` of `sbmlsim.uncertainty.covariance`: the alternating projections of
/// Higham (2002) with the correction of Dykstra, onto the positive semi-definite matrices and
/// onto the matrices of a unit diagonal, until an iteration changes the matrix less than `tol`
/// times its norm (at least 1) or after `max_iterations`; then a last projection onto the
/// positive semi-definite matrices, scaled to the unit diagonal and symmetrized.
///
/// A matrix with a value which is not finite is not a number off the diagonal and 1 on it, as
/// the iterations of numpy on its eigenvalues of not a number give it.
///
/// # Errors
///
/// A slice which is not of `k * k` values.
pub fn nearest_correlation(
    m: &[f64],
    k: usize,
    max_iterations: usize,
    tol: f64,
) -> Result<Vec<f64>> {
    check_size(m, k)?;
    if m.iter().any(|v| !v.is_finite()) {
        let mut nan = vec![f64::NAN; k * k];
        for i in 0..k {
            nan[i * k + i] = 1.0;
        }
        return Ok(nan);
    }
    let mut y = m.to_vec();
    let mut correction = vec![0.0; k * k];
    for _ in 0..max_iterations {
        let r: Vec<f64> = y.iter().zip(&correction).map(|(y, c)| y - c).collect();
        let x = psd_projection(&r, k)?;
        correction = x.iter().zip(&r).map(|(x, r)| x - r).collect();
        let mut y_new = x;
        for i in 0..k {
            y_new[i * k + i] = 1.0;
        }
        let change: Vec<f64> = y_new.iter().zip(&y).map(|(a, b)| a - b).collect();
        let converged = frobenius(&change) <= tol * frobenius(&y).max(1.0);
        y = y_new;
        if converged {
            break;
        }
    }
    let mut y = psd_projection(&y, k)?;
    let d: Vec<f64> = (0..k)
        .map(|i| libm::sqrt(y[i * k + i].max(1e-300)))
        .collect();
    for i in 0..k {
        for j in 0..k {
            y[i * k + j] /= d[i] * d[j];
        }
        y[i * k + i] = 1.0;
    }
    let mut out = vec![0.0; k * k];
    for i in 0..k {
        for j in 0..k {
            out[i * k + j] = (y[i * k + j] + y[j * k + i]) / 2.0;
        }
    }
    Ok(out)
}

/// Whether the `k x k` matrix `m` is positive semi-definite, `_is_psd` of
/// `sbmlsim.uncertainty.covariance`: its smallest eigenvalue at least `-rtol` times its largest
/// one (at least 1e-300).
///
/// The matrix is the symmetric one of its lower triangle, as `numpy.linalg.eigvalsh` reads it.
/// A matrix with a value which is not finite is not positive semi-definite (numpy's eigenvalues
/// are not a number), and so is a slice which is not of `k * k` values. A matrix without
/// targets is positive semi-definite; the python raises numpy's ValueError of the smallest
/// value of an empty array.
#[must_use]
pub fn is_psd(m: &[f64], k: usize, rtol: f64) -> bool {
    if check_size(m, k).is_err() {
        return false;
    }
    match linalg::eigvalsh(&lower_symmetric(m, k), k) {
        Ok(values) if values.is_empty() => true,
        // largest first
        Ok(values) => values[k - 1] >= -rtol * values[0].max(1e-300),
        Err(_) => false,
    }
}

/// The symmetric matrix of the lower triangle of `m`, the one `numpy.linalg.eigvalsh` reads.
fn lower_symmetric(m: &[f64], k: usize) -> Vec<f64> {
    let mut lower = vec![0.0; k * k];
    for i in 0..k {
        for j in 0..=i {
            lower[i * k + j] = m[i * k + j];
            lower[j * k + i] = m[i * k + j];
        }
    }
    lower
}

/// The text python writes a shape with, its tuple: `(3,)` of one dimension, `()` of none.
fn shape_text(shape: &[usize]) -> String {
    match shape {
        [n] => format!("({n},)"),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Checks a covariance or a correlation (`is_correlation`) of the targets, `Covariance._validate`
/// of `sbmlsim.uncertainty.covariance`, with its messages and in its order: the targets once
/// each, the matrix of the shape `shape` square of the size of the targets, finite and
/// symmetric (`|m_ij - m_ji| <= 1e-12 max|m|`); a correlation of a unit diagonal (to 1e-12)
/// and coefficients `|rho| <= 1 + 1e-12`, a covariance of positive variances and one unit per
/// target (`units`, their number, `None` without); and positive semi-definite ([`is_psd`] at
/// [`PSD_RTOL`]), else the smallest eigenvalue and the largest correlation of the message.
///
/// `m` are the values of the matrix of the shape `shape`, row-major.
///
/// # Errors
///
/// The ValueError of the python of a matrix which is not valid, and numpy's of a matrix
/// without targets (the largest value of an empty array).
pub fn validate(
    m: &[f64],
    shape: &[usize],
    targets: &[&str],
    is_correlation: bool,
    units: Option<usize>,
) -> Result<()> {
    let k = targets.len();
    let names = python_str_list(targets);
    let mut twice: Vec<&str> = targets
        .iter()
        .copied()
        .filter(|t| targets.iter().filter(|u| *u == t).count() > 1)
        .collect();
    if !twice.is_empty() {
        twice.sort_unstable();
        twice.dedup();
        return Err(StatsError(format!(
            "The targets {} are given twice.",
            python_str_list(&twice)
        )));
    }
    if shape != [k, k] {
        return Err(StatsError(format!(
            "The matrix of {names} must be square of size {k}, not of the shape {}.",
            shape_text(shape)
        )));
    }
    check_size(m, k)?;
    if m.iter().any(|v| !v.is_finite()) {
        return Err(StatsError(format!("The matrix of {names} is not finite.")));
    }
    if k == 0 {
        return Err(StatsError(EMPTY_MAXIMUM.to_owned()));
    }
    let scale = m.iter().fold(0.0_f64, |a, v| a.max(v.abs())).max(1e-300);
    for i in 0..k {
        for j in 0..k {
            if (m[i * k + j] - m[j * k + i]).abs() > 1e-12 * scale {
                return Err(StatsError(format!(
                    "The matrix of {names} is not symmetric."
                )));
            }
        }
    }
    let kind = if is_correlation {
        if (0..k).any(|i| (m[i * k + i] - 1.0).abs() > 1e-12) {
            return Err(StatsError(format!(
                "The diagonal of the correlation of {names} must be 1."
            )));
        }
        if m.iter().any(|v| v.abs() > 1.0 + 1e-12) {
            return Err(StatsError(format!(
                "The correlation of {names} has a coefficient with |rho| > 1."
            )));
        }
        "correlation"
    } else {
        if (0..k).any(|i| m[i * k + i] <= 0.0) {
            return Err(StatsError(format!(
                "The covariance of {names} must have a positive variance of every target."
            )));
        }
        if units.is_some_and(|n| n != k) {
            return Err(StatsError(format!(
                "The covariance of {names} needs one unit per target."
            )));
        }
        "covariance"
    };
    if is_psd(m, k, PSD_RTOL) {
        return Ok(());
    }
    let values = linalg::eigvalsh(&lower_symmetric(m, k), k)?;
    let smallest = values[k - 1];
    // the correlation of the matrix, the largest coefficient off the diagonal, the first of
    // the largest in the order of the rows as np.argmax
    let d: Vec<f64> = (0..k).map(|i| libm::sqrt(m[i * k + i])).collect();
    let (mut largest, mut at) = (f64::NEG_INFINITY, (0, 0));
    let mut coefficient = 0.0;
    for i in 0..k {
        for j in 0..k {
            let c = if is_correlation {
                m[i * k + j]
            } else {
                m[i * k + j] / (d[i] * d[j])
            };
            let off = (c - if i == j { 1.0 } else { 0.0 }).abs();
            if off > largest {
                (largest, at, coefficient) = (off, (i, j), c);
            }
        }
    }
    Err(StatsError(format!(
        "The {kind} of {names} is not positive semi-definite: its smallest eigenvalue is {}; \
         the largest correlation is {} of ('{}', '{}').",
        python_format_g(smallest, 3),
        python_format_g(coefficient, 3),
        targets[at.0],
        targets[at.1],
    )))
}

//! The Laplace approximation of the posterior of the parameters of a fit and the correlations
//! and marginals of the posterior, as `sbmlsim.fit.posterior` gives them.
//!
//! The posterior is the normal in the space of the optimizer whose precision is the curvature
//! of the negative log posterior at the estimate, the Fisher information divided by the
//! variance of the residuals plus the Hessian of the priors (which the python computes); its
//! covariance is the inverse of the precision ([`laplace`]). A direction which neither the
//! data nor the priors constrain, an eigenvalue of the precision at most `rank_tolerance`
//! times the largest one, has no normal, and the approximation is an error which names the
//! parameters of every such direction.
//!
//! The correlation of the normal scores of the posterior ([`normal_correlation`]), its rank
//! correlation ([`rank_correlation`]) and the marginal of a parameter on a logarithmic scale,
//! a log-normal of the standard deviation of the natural logarithm [`log_sd`] and the
//! coefficient of variation [`lognormal_cv`], are those of `ParameterPosterior`; its intervals
//! are [`crate::fisher::intervals`] of the normal distribution.

use crate::{Result, Scale, StatsError, linalg};

/// A component of the eigenvector of a weak direction above this absolute value names its
/// parameter in the error, `posterior_laplace`.
const DIRECTION_COMPONENT: f64 = 0.1;

/// The covariance of the Laplace approximation of the posterior of the precision `precision`
/// (row-major `k x k`), the inverse of its symmetric part, `posterior_laplace` from the
/// precision on; an error which names the parameters of the weak directions by their indices.
///
/// # Errors
///
/// As [`laplace_of`] without an id of the problem, the parameters named by their indices
/// (`of [[1, 3]]`).
pub fn laplace(precision: &[f64], k: usize, rank_tolerance: f64) -> Result<Vec<f64>> {
    if k > 0 {
        // the size before a name per parameter is written, in the order of `laplace_of`
        linalg::check_square(precision, k)?;
    }
    let names: Vec<String> = (0..k).map(|i| i.to_string()).collect();
    laplace_of(precision, k, rank_tolerance, None, &names)
}

/// The covariance of the Laplace approximation of the posterior of the precision `precision`
/// (row-major `k x k`), `posterior_laplace` from the precision on: the precision symmetrized,
/// `(P + Pᵀ) / 2`, decomposed ([`linalg::eigh`]) and inverted, `V diag(1/w) Vᵀ`, symmetrized.
///
/// An eigenvalue at most `rank_tolerance` times the largest one (every one of a largest one
/// which is not positive) is a weak direction, which the approximation cannot describe.
///
/// # Errors
///
/// The error of `posterior_laplace` of weak directions, `'<opid>': ` first with an `opid`,
/// which names the parameters of every direction by `names` (the parameters as the message
/// writes them, the `repr` of a pid in python): the directions in the order of their
/// eigenvalues, smallest first, and in a direction the parameters of an absolute component of
/// the eigenvector above 0.1, the largest first (equal ones in their order; numpy's sort is
/// not stable, and the eigenvectors of equal eigenvalues are any basis of their space, so a
/// direction is numpy's where its eigenvalue and its components are distinct). The error of
/// numpy of no parameter, another number of names than of parameters and a precision which is
/// not of `k * k` finite values, as [`linalg::eigh`].
pub fn laplace_of(
    precision: &[f64],
    k: usize,
    rank_tolerance: f64,
    opid: Option<&str>,
    names: &[String],
) -> Result<Vec<f64>> {
    if names.len() != k {
        return Err(StatsError::value(format!(
            "The weak directions of a precision of {k} parameters need a name per parameter, \
             not {} names.",
            names.len()
        )));
    }
    if k == 0 {
        // the python takes the largest eigenvalue of none
        return Err(StatsError::value(
            "zero-size array to reduction operation maximum which has no identity".to_string(),
        ));
    }
    // as the check of the matrix, before the symmetric part is formed
    linalg::check_square(precision, k)?;
    let symmetric = linalg::symmetric_part(precision, k);
    let (eigenvalues, vectors) = linalg::eigh(&symmetric, k)?;
    let largest = eigenvalues[0];
    let mut weak: Vec<usize> = (0..k)
        .filter(|&d| largest <= 0.0 || eigenvalues[d] <= largest * rank_tolerance)
        .collect();
    if !weak.is_empty() {
        // numpy's order, the smallest eigenvalue first
        weak.sort_by(|&a, &b| eigenvalues[a].total_cmp(&eigenvalues[b]));
        let directions: Vec<String> = weak
            .iter()
            .map(|&d| {
                let mut order: Vec<usize> = (0..k).collect();
                order.sort_by(|&a, &b| {
                    vectors[b * k + d]
                        .abs()
                        .total_cmp(&vectors[a * k + d].abs())
                });
                let named: Vec<&str> = order
                    .into_iter()
                    .filter(|&i| vectors[i * k + d].abs() > DIRECTION_COMPONENT)
                    .map(|i| names[i].as_str())
                    .collect();
                format!("[{}]", named.join(", "))
            })
            .collect();
        let prefix = opid.map(|opid| format!("'{opid}': ")).unwrap_or_default();
        return Err(StatsError::value(format!(
            "{prefix}the data and the priors do not constrain {} direction(s) of the \
             parameters, of [{}]; the posterior is the uniform prior of the bounds there, \
             which the Laplace approximation cannot describe. Give these parameters priors \
             and fit the objective 'posterior', or see the profile likelihood.",
            directions.len(),
            directions.join(", ")
        )));
    }
    // (V / w) Vᵀ, then its symmetric part, as the python
    let mut covariance = vec![0.0; k * k];
    for i in 0..k {
        for l in 0..k {
            let mut sum = 0.0;
            for (j, &w) in eigenvalues.iter().enumerate() {
                sum += vectors[i * k + j] / w * vectors[l * k + j];
            }
            covariance[i * k + l] = sum;
        }
    }
    Ok(linalg::symmetric_part(&covariance, k))
}

/// The Spearman rank correlation of the normal of the correlation `r` (row-major `k x k`),
/// `ParameterPosterior.rank_correlation`: `6 / π asin(clip(r, -1, 1) / 2)` (Pearson 1907),
/// the diagonal 1, symmetrized; not a number where `r` is.
///
/// # Errors
///
/// A slice which is not of `k * k` values.
pub fn rank_correlation(r: &[f64], k: usize) -> Result<Vec<f64>> {
    linalg::check_square(r, k)?;
    let factor = 6.0 / core::f64::consts::PI;
    let mut rank: Vec<f64> = r
        .iter()
        .map(|&v| factor * libm::asin(v.clamp(-1.0, 1.0) / 2.0))
        .collect();
    for i in 0..k {
        rank[i * k + i] = 1.0;
    }
    Ok(linalg::symmetric_part(&rank, k))
}

/// The correlation of the normal scores of the posterior of its correlation `r` (row-major
/// `k x k`), `ParameterPosterior.normal_correlation`: the symmetric part, the diagonal 1.
///
/// # Errors
///
/// A slice which is not of `k * k` values.
pub fn normal_correlation(r: &[f64], k: usize) -> Result<Vec<f64>> {
    linalg::check_square(r, k)?;
    let mut matrix = linalg::symmetric_part(r, k);
    for i in 0..k {
        matrix[i * k + i] = 1.0;
    }
    Ok(matrix)
}

/// The standard deviation of the natural logarithm of a standard deviation `sd` on the scale
/// `scale`, `posterior._log_sd`: `ln(10) sd` on the decadic logarithm, `sd` otherwise.
pub fn log_sd(scale: Scale, sd: f64) -> f64 {
    if scale == Scale::Log10 {
        sd * core::f64::consts::LN_10
    } else {
        sd
    }
}

/// The coefficient of variation of the log-normal of the standard deviation `sigma` of the
/// natural logarithm, `sqrt(exp(sigma²) - 1)`, the marginal of a parameter on a logarithmic
/// scale of `ParameterPosterior.distributions`.
pub fn lognormal_cv(sigma: f64) -> f64 {
    libm::sqrt(libm::expm1(sigma * sigma))
}

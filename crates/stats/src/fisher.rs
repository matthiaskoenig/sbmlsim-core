//! The statistics of the Fisher information of a fit, as `sbmlsim.fit.fisher.FisherInformation`
//! gives them.
//!
//! The Fisher information of a fit of the weighted residuals `r` with the jacobian `J` is
//! `JᵀJ` ([`crate::linalg::gram`]); its pseudo-inverse, scaled by the variance of the residuals
//! `σ² = 2 cost / (n - k)` of a least squares fit (1 for a likelihood, whose residuals are in
//! units of the noise), is the covariance of the parameters in the space the optimizer
//! searches. From it follow the standard errors, the correlation, the intervals and the
//! relative errors; from the eigenvalues the rank and the condition number.
//!
//! The edges are the python's: a variance without degrees of freedom is not a number, an
//! eigenvalue at most `rank_tolerance` times the largest one is a direction the data do not
//! constrain, a smallest eigenvalue which is not positive an infinite condition, and the
//! error of a parameter whose variance is not positive (zero allowed of an identifiable
//! information) not a number. The pseudo-inverse is the one of numpy, which cuts eigenvalues
//! at `1e-15` times the largest ([`crate::linalg::pinv_symmetric`]).

use crate::special::{normal_ppf, t_ppf};
use crate::{Result, Scale, StatsError, linalg};

/// Eigenvalues below this fraction of the largest one are a direction the data do not
/// constrain, `sbmlsim.fit.fisher.DEFAULT_RANK_TOLERANCE`.
pub const DEFAULT_RANK_TOLERANCE: f64 = 1e-8;

/// The statistics of a Fisher information of `k` parameters, the matrices row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct FisherStatistics {
    /// The variance of the residuals, `2 cost / (n - k)`, not a number without degrees of
    /// freedom, 1 for a likelihood (`FisherInformation.sigma2`).
    pub sigma2: f64,
    /// The eigenvalues of the information, largest first (`eigenvalues`).
    pub spectrum: Vec<f64>,
    /// The number of directions the data constrain (`rank`).
    pub rank: usize,
    /// The ratio of the largest to the smallest eigenvalue, infinite of a smallest one which
    /// is not positive (`condition_number`).
    pub condition: f64,
    /// The covariance `σ² (JᵀJ)⁺` in the space of the optimizer, `k x k` (`covariance`).
    pub covariance: Vec<f64>,
    /// The standard error of every parameter, not a number of a variance which is not
    /// positive, zero allowed of an identifiable information (`standard_errors`).
    pub standard_errors: Vec<f64>,
    /// The correlation of the parameters, `cov_ij / (sd_i sd_j)`, `k x k`, not a number of a
    /// variance of zero (`correlation`).
    pub correlation: Vec<f64>,
}

/// The variance of the residuals, `FisherInformation.sigma2`: 1 for a likelihood, else
/// `2 cost / (n - k)`, not a number without degrees of freedom (`n <= k`). The number of the
/// data points is a `u64`, so it is counted alike where a `usize` has 32 bits (wasm).
pub fn sigma2(cost: f64, n: u64, k: usize, likelihood: bool) -> f64 {
    if likelihood {
        return 1.0;
    }
    let k = k as u64;
    if n <= k {
        return f64::NAN;
    }
    2.0 * cost / (n - k) as f64
}

/// The statistics of the Fisher information `information` (`JᵀJ`, row-major `k x k`) of a fit
/// of `n` data points with the cost `cost` (the weighted least squares or the negative
/// log-likelihood of the data), of a likelihood with `likelihood`; an eigenvalue at most
/// `rank_tolerance` times the largest one is a direction the data do not constrain
/// ([`DEFAULT_RANK_TOLERANCE`]).
///
/// The statistics of its [`decomposition`], [`statistics_of_decomposition`], to the bit.
///
/// # Errors
///
/// No parameter (the python fails on the eigenvalues of an empty matrix), and an information
/// which is not of `k * k` finite values or not symmetric, as [`linalg::eigh`].
pub fn statistics(
    information: &[f64],
    k: usize,
    cost: f64,
    n: u64,
    likelihood: bool,
    rank_tolerance: f64,
) -> Result<FisherStatistics> {
    let (spectrum, inverse) = decomposition(information, k)?;
    statistics_of_decomposition(&spectrum, &inverse, k, cost, n, likelihood, rank_tolerance)
}

/// The decomposition of the Fisher information `information` (row-major `k x k`) the
/// statistics come from: its eigenvalues, largest first, and its pseudo-inverse, both of one
/// eigendecomposition (the cyclic Jacobi method of [`linalg::eigh`]), the pseudo-inverse the
/// one of numpy, which cuts the eigenvalues at `1e-15` times the largest
/// ([`linalg::pinv_of_eigh`]); `FisherInformation.spectrum` and `.inverse` of a record.
///
/// # Errors
///
/// No parameter, and an information which is not of `k * k` finite values or not symmetric,
/// as [`linalg::eigh`].
pub fn decomposition(information: &[f64], k: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    if k == 0 {
        return Err(no_parameters());
    }
    // one decomposition for the eigenvalues and the pseudo-inverse, numpy's of both
    let (spectrum, vectors) = linalg::eigh(information, k)?;
    let inverse = linalg::pinv_of_eigh(&spectrum, &vectors, k)?;
    Ok((spectrum, inverse))
}

/// The statistics of a Fisher information of `k` parameters from its decomposition, the
/// eigenvalues `spectrum` (largest first) and the pseudo-inverse `inverse` (row-major
/// `k x k`), as `FisherInformation` derives them from a record which stores them, so a
/// decomposition of another method (the SVD of LAPACK of a record of a release before
/// sbmlsim-core computed them) gives the numbers it was written with, to the bit: the rank
/// and the condition from the eigenvalues, the covariance `σ² inverse`, the standard errors
/// and the correlation from the covariance. The pseudo-inverse is taken as it is, without a
/// check of its symmetry (one of LAPACK is not symmetric to the bit) and with the values which
/// are not finite of a pseudo-inverse of numpy which overflows (of eigenvalues below
/// `1 / f64::MAX`), which give the statistics of not a number and infinities numpy gives.
///
/// # Errors
///
/// No parameter, eigenvalues and a pseudo-inverse of other lengths than `k` and `k * k`, and
/// eigenvalues which are not largest first.
pub fn statistics_of_decomposition(
    spectrum: &[f64],
    inverse: &[f64],
    k: usize,
    cost: f64,
    n: u64,
    likelihood: bool,
    rank_tolerance: f64,
) -> Result<FisherStatistics> {
    if k == 0 {
        return Err(no_parameters());
    }
    if spectrum.len() != k || k.checked_mul(k) != Some(inverse.len()) {
        return Err(StatsError::value(format!(
            "The decomposition of a Fisher information of {k} parameters needs {k} \
             eigenvalues and a pseudo-inverse of {k} x {k} values, not {} and {}.",
            spectrum.len(),
            inverse.len()
        )));
    }
    if spectrum.windows(2).any(|pair| pair[0] < pair[1]) {
        return Err(StatsError::value(
            "The eigenvalues of a Fisher information must be largest first.",
        ));
    }
    let (first, last) = (spectrum[0], spectrum[k - 1]);
    let condition = if last <= 0.0 {
        f64::INFINITY
    } else {
        first / last
    };
    let rank = if first <= 0.0 {
        0
    } else {
        spectrum
            .iter()
            .filter(|&&v| v > first * rank_tolerance)
            .count()
    };
    let sigma2 = sigma2(cost, n, k, likelihood);
    let covariance: Vec<f64> = inverse.iter().map(|&v| sigma2 * v).collect();
    let identifiable = rank == k;
    let standard_errors = (0..k)
        .map(|i| {
            let variance = covariance[i * k + i];
            // a perfect identifiable fit has a variance of zero; a zero on the diagonal of a
            // deficient pseudo-inverse may be a direction the data do not constrain
            let valid = if identifiable {
                variance >= 0.0
            } else {
                variance > 0.0
            };
            if valid {
                libm::sqrt(variance)
            } else {
                f64::NAN
            }
        })
        .collect();
    let sd: Vec<f64> = (0..k).map(|i| libm::sqrt(covariance[i * k + i])).collect();
    let correlation = (0..k * k)
        .map(|index| covariance[index] / (sd[index / k] * sd[index % k]))
        .collect();
    Ok(FisherStatistics {
        sigma2,
        spectrum: spectrum.to_vec(),
        rank,
        condition,
        covariance,
        standard_errors,
        correlation,
    })
}

/// The error of an information of no parameters, whose eigenvalues the python fails on.
fn no_parameters() -> StatsError {
    StatsError::value("The Fisher information of no parameters has no statistics.")
}

/// The confidence interval of every parameter, `FisherInformation.confidence_intervals`: the
/// value `values[i]` (in the units of the model) on its scale `scales[i]` plus and minus the
/// quantile of the level `level` times its standard error `se[i]`, transformed back into the
/// units of the model, so not symmetric on a logarithmic scale.
///
/// The quantile is the one of the t distribution of `dof` degrees of freedom (`n - k` of a
/// least squares fit), of the normal distribution without (`None`, a likelihood); intervals
/// of not a number for degrees of freedom which are not positive. The quantile of the normal
/// distribution is also the one of the credible interval of the Laplace approximation,
/// `ParameterPosterior.intervals` (with the standard deviations of the posterior).
///
/// # Errors
///
/// Values, standard errors and scales of different lengths.
pub fn intervals(
    values: &[f64],
    se: &[f64],
    scales: &[Scale],
    level: f64,
    dof: Option<f64>,
) -> Result<Vec<(f64, f64)>> {
    check_lengths(values, se, scales)?;
    let p = 0.5 + level / 2.0;
    let quantile = match dof {
        None => normal_ppf(p),
        Some(dof) if dof > 0.0 => t_ppf(p, dof),
        Some(_) => return Ok(vec![(f64::NAN, f64::NAN); values.len()]),
    };
    Ok(values
        .iter()
        .zip(se)
        .zip(scales)
        .map(|((&value, &se), &scale)| {
            let (scaled, delta) = (scale.to_scale(value), quantile * se);
            (
                scale.from_scale(scaled - delta),
                scale.from_scale(scaled + delta),
            )
        })
        .collect())
}

/// The standard error of every parameter relative to its value, `FisherInformation.
/// relative_errors`: the coefficient of variation of the value to first order, `|se / value|`
/// on the linear scale, `se` on the natural and `ln(10) se` on the decadic logarithm.
///
/// # Errors
///
/// Values, standard errors and scales of different lengths.
pub fn relative_errors(values: &[f64], se: &[f64], scales: &[Scale]) -> Result<Vec<f64>> {
    check_lengths(values, se, scales)?;
    Ok(values
        .iter()
        .zip(se)
        .zip(scales)
        .map(|((&value, &se), scale)| match scale {
            Scale::Log10 => core::f64::consts::LN_10 * se,
            Scale::Log => se,
            Scale::Linear => (se / value).abs(),
        })
        .collect())
}

/// The error unless there is a standard error and a scale per value.
fn check_lengths(values: &[f64], se: &[f64], scales: &[Scale]) -> Result<()> {
    if values.len() != se.len() || values.len() != scales.len() {
        return Err(StatsError::value(format!(
            "{} values, {} standard errors and {} scales",
            values.len(),
            se.len(),
            scales.len()
        )));
    }
    Ok(())
}

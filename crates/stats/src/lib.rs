//! The statistics of the analyses of sbmlsim-core: pure functions on slices of doubles, which
//! replace the kernels of numpy and scipy in the python package sbmlsim.
//!
//! The crate knows no simulation, no problem and no table: a function takes numbers and returns
//! numbers, and the caller (the python bindings, later the browser) keeps the flow. The modules:
//!
//! - [`special`]: the special functions of the distributions, the logarithm of the beta
//!   function, the regularized incomplete beta function and the distribution functions and
//!   quantiles of the normal, the Student t and the binomial distribution, as scipy gives them.
//! - [`linalg`]: the linear algebra of symmetric matrices, the eigendecomposition (cyclic
//!   Jacobi), the Cholesky factor, the factor of a correlation, the pseudo-inverse and `JᵀJ`
//!   of a jacobian, as numpy gives them.
//! - [`metrics`]: the metrics of a fit and the criteria of its evaluation, as
//!   `sbmlsim.fit.metrics` and `sbmlsim.fit.evaluation` give them, their sums numpy's.
//! - [`fisher`]: the statistics of the Fisher information of a fit, its eigenvalues, rank,
//!   condition, covariance, standard errors, correlation and intervals, as
//!   `sbmlsim.fit.fisher.FisherInformation` gives them.
//! - [`posterior`]: the Laplace approximation of the posterior from its precision and the
//!   correlations and marginals of the posterior, as `sbmlsim.fit.posterior` gives them.
//! - [`uncertainty`]: the statistics of the draws of an uncertainty analysis with their errors
//!   (the mean, the standard deviation, the quantiles and their intervals, the first-order
//!   index from given data), the precision of an output and the parameters of a reported
//!   interval, as `sbmlsim.uncertainty` gives them.
//! - [`correlation`]: the correlation matrices of an uncertainty analysis, the nearest
//!   correlation matrix (Higham 2002), the test of positive semi-definiteness, the validation of
//!   a covariance and the coefficient of the normal scores of a covariance, as
//!   `sbmlsim.uncertainty.covariance` and `sbmlsim.uncertainty.draws` give them.
//! - [`draws`]: the points of the unit cube of the designs of the draws (Sobol', Latin
//!   hypercube, random) and the Gaussian copula which correlates them, as
//!   `sbmlsim.uncertainty.draws` gives them, the streams of sbmlsim-core rather than scipy's.
//! - [`testing`]: the probability statements about predictions, robust to the number of draws:
//!   the anytime-valid p-values and the confidence sequence of the beta-binomial mixture, the
//!   decision of a statement and the draws an undecided one needs, as
//!   `sbmlsim.uncertainty.testing` gives them.
//! - [`variability`]: the virtual groups of subjects and the bands of the visual predictive
//!   check, as `predictive_check` of `sbmlsim.fit.variability` gives them, the groups resampled
//!   from the streams of sbmlsim-core rather than numpy's.
//!
//! Every transcendental function comes from the crate libm, so one input gives the same bits
//! on every target, natively and in wasm. The tests hold every function to the values of numpy
//! and scipy, and the metrics to the functions of sbmlsim, computed once by
//! `oracles/make_oracles.py` and stored next to it as JSON.

pub mod correlation;
pub mod draws;
pub mod fisher;
pub mod linalg;
pub mod metrics;
mod numpy;
pub mod posterior;
mod python;
pub mod special;
pub mod testing;
pub mod uncertainty;
pub mod variability;

/// An error of the arguments of a function.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct StatsError(pub String);

/// The result of a function of the crate.
pub type Result<T> = std::result::Result<T, StatsError>;

/// The space the optimizer searches a parameter in, `sbmlsim.fit.options.ParameterScaleType`:
/// the value in the units of the model, its natural or its decadic logarithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scale {
    /// The value itself (`LINEAR`, PEtab's `lin`).
    Linear,
    /// The natural logarithm of the value (`LOG`, PEtab's `log`).
    Log,
    /// The decadic logarithm of the value (`LOG10`, PEtab's `log10`).
    Log10,
}

impl Scale {
    /// The value of the model in the space of the optimizer, `ParameterScaleType.to_scale`:
    /// a logarithm of zero is minus infinity, of a negative value not a number.
    pub fn to_scale(self, x: f64) -> f64 {
        match self {
            Self::Linear => x,
            Self::Log => libm::log(x),
            Self::Log10 => libm::log10(x),
        }
    }

    /// The value of the optimizer in the units of the model, `ParameterScaleType.from_scale`:
    /// infinite past the largest double.
    pub fn from_scale(self, x: f64) -> f64 {
        match self {
            Self::Linear => x,
            Self::Log => libm::exp(x),
            Self::Log10 => libm::pow(10.0, x),
        }
    }
}

//! The statistics of the analyses of sbmlsim-core: pure functions on slices of doubles, which
//! replace the kernels of numpy and scipy in the python package sbmlsim.
//!
//! The crate knows no simulation, no problem and no table: a function takes numbers and returns
//! numbers, and the caller (the python bindings, the JavaScript ones of `compiler.wasm`) keeps the
//! flow. The modules:
//!
//! - [`special`]: the special functions of the distributions, the logarithm of the beta
//!   function, the regularized incomplete beta and gamma functions, the error function, the
//!   gamma function and the distribution functions and quantiles of the normal, the Student
//!   t, the chi-squared and the binomial distribution, as scipy gives them.
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
//! - [`binding`]: the functions as the bindings take their arguments, arrays of any shape with
//!   their shape, signed integers and names, checked with the messages of the python, which the
//!   python and the JavaScript bindings share.
//! - [`variability`]: the virtual groups of subjects and the bands of the visual predictive
//!   check, as `predictive_check` of `sbmlsim.fit.variability` gives them, the groups resampled
//!   from the streams of sbmlsim-core rather than numpy's.
//!
//! # Accuracy
//!
//! The tests hold every function to the values of numpy and scipy, and the metrics to the
//! functions of sbmlsim, computed once by `oracles/make_oracles.py` and stored next to it as
//! JSON (`tests/oracle_<module>.rs`): the deterministic kernels (the metrics, the Fisher
//! statistics, the posterior, the statistics of the draws, the logarithm of the beta function,
//! the p-values and the confidence sequence) to 1e-12 relative, the normal distribution, the
//! gamma function and the chi-squared quantile to 1e-13 and the error function to 1e-15 (of
//! mpmath at 40 digits), the incomplete gamma function and the chi-squared distribution
//! function to 1e-13 times their condition `max(1, min(|x - a|, (x - a)² / a))`, the incomplete beta function and the t and binomial distributions to 1e-10, the
//! binomial quantiles to the same integers and the linear algebra of up to 200 parameters to
//! 1e-10 of the largest value of the matrix (LAPACK sums in another order). Where the crate
//! and scipy differ beyond the rounding, the crate is the more accurate: [`special::ln_beta`]
//! keeps its digits at a large and a small parameter, where `scipy.special.betaln` loses them
//! (1.5e-10 relative at 0.5 and 1e6), so the largest difference of the p-values of [`testing`]
//! to the python is the error of scipy's `betaln`.
//!
//! # Errors and sizes
//!
//! An error is a [`StatsError`]: the message of the exception the python raised and its
//! [`ErrorKind`], the class of that exception (a `ValueError` mostly), set where the error is
//! made. A size of an array a function makes is counted without a wrap, on every target (a
//! `usize` of 32 bits in wasm): one past what an array holds, its bytes past the largest
//! `isize`, is numpy's `ValueError` (`array is too big`) before any memory is asked for. A
//! size below is reserved with `try_reserve_exact`, which is no check of the memory where the
//! system overcommits: there, as with numpy, the memory is the caller's.
//!
//! # Determinism
//!
//! Every transcendental function comes from the crate libm, and every random draw from
//! xoshiro256++ of sbmlsim-core (`sbmlsim_sensitivity::random`), so one input and one seed give
//! the same bits on every target, natively and in wasm: the tests hold the hashes of the bits
//! of every module, natively and under wasmtime (`--target wasm32-wasip1`).
//!
//! The random streams are deliberately not the ones of scipy and numpy which sbmlsim drew from:
//! the Sobol' points of [`draws`] are the Sobol' sequence of sbmlsim-core with its linear matrix
//! scrambling and digital shift (LMS+shift, not the scrambling of Owen), and the random points,
//! the Latin hypercube and the resampling of the group means of [`variability`] draw from
//! xoshiro256++, not from PCG64. The draws are held to their distributions, and a seed of a
//! result from before reproduces it only with the release it was computed with.

pub mod binding;
pub mod correlation;
pub mod draws;
pub mod fisher;
pub mod linalg;
pub mod metrics;
mod numpy;
pub mod posterior;
mod python;
mod size;
pub mod special;
pub mod testing;
pub mod uncertainty;
pub mod variability;

/// An error of the arguments of a function: its kind and the message of the exception the
/// python raised.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct StatsError {
    /// The class of the exception the python raised, set where the error is made.
    pub kind: ErrorKind,
    /// The message of the exception of the python.
    pub message: String,
}

impl StatsError {
    /// An error of the kind with the message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// An error of an argument the python refuses with a `ValueError`, the kind of most errors.
    pub fn value(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Value, message)
    }
}

/// The kind of a [`StatsError`], the class of the exception the python raised for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// `ValueError`, of an argument the python refuses (numpy's `array is too big` of a size
    /// past what an array holds included).
    Value,
    /// `OverflowError`, of a power, an exponential or a rounding past the largest double.
    Overflow,
    /// `ZeroDivisionError`, of a division by zero.
    ZeroDivision,
    /// `IndexError`, of numpy indexing draws by a mask of the other draws.
    Index,
}

impl ErrorKind {
    /// The name of the class of the exception of python.
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

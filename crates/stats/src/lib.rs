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
//!
//! Every transcendental function comes from the crate libm, so one input gives the same bits
//! on every target, natively and in wasm. The tests hold every function to the values of numpy
//! and scipy, computed once by `oracles/make_oracles.py` and stored next to it as JSON.

pub mod linalg;
pub mod special;

/// An error of the arguments of a function.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct StatsError(pub String);

/// The result of a function of the crate.
pub type Result<T> = std::result::Result<T, StatsError>;

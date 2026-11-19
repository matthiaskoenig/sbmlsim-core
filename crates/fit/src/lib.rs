//! The parameter fit of sbmlsim-core: the objective of a fit on the batch of the engine.
//!
//! A fit asks the same question of a model many times over: one evaluation of its objective
//! is one simulation per condition at the same parameters, the jacobian of a step of a least
//! squares optimizer is that evaluation once per parameter and once at the parameters
//! themselves, and a generation of differential evolution is one evaluation per member. None
//! of these simulations depends on another, so they are the points of a batch, which the
//! engine runs on threads of its own ([`sbmlsim_batch`]).
//!
//! This crate is that objective and the optimizers over it:
//!
//! - [`Definition`] is the fit as a host hands it over: the parameters with their bounds and
//!   their scale, a request of `simulation/1` per condition and the data of every curve.
//! - [`Problem`] resolves it against a loaded model once (the prepared request of every
//!   condition, the column and the row of every data point) and answers the
//!   [`Problem::residuals`], the [`Problem::cost`] and the [`Problem::jacobian`] of a
//!   parameter vector. The whole jacobian is **one** fan out of `(k + 1) * G` points, not a
//!   fan out per column: the residuals of a point are computed on the thread of the point,
//!   so a point costs one integration and nothing of a host crosses a thread.
//! - [`Problem::optimize`] runs the optimizers of [basin](https://basin.rs) over it: trust
//!   region reflective and the bounded Levenberg-Marquardt of a least squares fit, and the
//!   differential evolution of its global search.
//! - [`fit`] runs a fit of the contract [`FIT_CONTRACT`], which is a definition, its
//!   optimizer and its starts as a file: what a host hands over and reads back.
//! - [`profile`] and [`fisher`] say how well the data determines the optimum a fit found: the
//!   profile of the likelihood of a parameter holds it at a value and fits the others again
//!   ([`Problem::fixing`], which shares the conditions and the pool of the fit), the Fisher
//!   information is the curvature of the cost at the optimum.
//!
//! The parameters live on a scale ([`Scale`]): an optimizer searches the logarithm of a
//! parameter whose bounds span decades, which is what a fit of a PBPK model does, and the
//! model always sees the value itself.

mod contract;
mod identifiability;
mod optimize;
mod problem;

pub use contract::{
    ConditionRequest, CurveRequest, FIT_CONTRACT, FitAnswer, FitRequest, FittedParameter,
    Identifiability, OnRun, ParameterRequest, RunAnswer, fit, fit_reporting, fit_with, problem_of,
};
pub use identifiability::{
    DELTA_95, Fisher, Profile, ProfilePoint, Walk, fisher, profile, profiles,
};
pub use optimize::{Algorithm, Evolution, Fit, Settings, Stop};
pub use problem::{
    Curve, DIFF_STEP, Definition, FAILED_RESIDUAL, FitError, JacobianChoice, Parameter, Problem,
    Scale,
};

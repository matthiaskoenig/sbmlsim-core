//! PEtab v2 in sbmlsim-core: a problem read, simulated and fitted in Rust.
//!
//! [PEtab](https://petab.readthedocs.io) describes a parameter estimation problem of a model of
//! systems biology as tables: the conditions an experiment applies at its times, the
//! observables and the noise of the measurements, the measurements themselves and the
//! parameters to estimate with their bounds and priors. This crate executes such a problem on
//! the engine of sbmlsim-core, with no python and no sympy on the way:
//!
//! 1. [`PetabProblem::read`] reads the YAML file, the models and the tables through [`Files`],
//!    contents by name, so the reader touches no file system and runs in a browser as well.
//! 2. [`Plan::new`] translates the problem into what the engine simulates. The math of the
//!    tables is read by [`math::parse`] into the math of SBML; every observable, every noise
//!    formula which reads the simulation and every value of a condition after the start is an
//!    assignment rule of a parameter added to the model of the problem
//!    ([`PlannedModel::document`]), so the compiled model computes it with the math of SBML;
//!    every experiment of a model is one request of `simulation/1`, whose steady state is the
//!    period at `-inf`, whose pre-initialization is the parameter table and the condition of
//!    the first period, and whose changes are the conditions of the later periods, each a form
//!    which reads the assignment rule of its value in the state before the change.
//! 3. [`Plan::evaluate`] reads the simulated observables and the noise off the results of the
//!    requests and gives the log-likelihood, `chi2` and the log priors of PEtab
//!    ([`Evaluation`]); it is pure, so a host of JavaScript which simulates the requests calls
//!    it as well.
//! 4. With the feature `native` (the default), [`Executor`] compiles the models and simulates
//!    the requests on the engine, and [`Executor::problem`] is the problem of `sbmlsim-fit`
//!    whose optimizers fit the parameters of the problem.
//!
//! The semantics are the ones of the specification of PEtab v2; [`Plan`] says how each of them
//! maps onto a request of the engine, and `docs/petab.md` what is supported.

pub mod likelihood;
pub mod math;
mod plan;
pub mod problem;
mod tables;

#[cfg(feature = "native")]
mod execute;

pub use likelihood::{NoiseDistribution, Prior, PriorKind};
pub use plan::{
    DataAtChange, Evaluation, ExtensionPolicy, InitialValue, Noise, Options, Plan, PlanParameter,
    PlannedMeasurement, PlannedModel, PlannedSimulation,
};
pub use problem::{Directory, Files, InMemory, PetabProblem};

#[cfg(feature = "native")]
pub use execute::{Executor, FitAnswer};

use thiserror::Error;

/// What keeps a problem from being read, planned or executed.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PetabError {
    /// A file which is missing or which is no file of PEtab v2.
    #[error("{0}")]
    File(String),
    /// A table which lacks a column or holds a cell which is no value of it.
    #[error("{0}")]
    Table(String),
    /// A formula which is no expression of PEtab.
    #[error("{0}")]
    Math(String),
    /// A problem whose parts do not fit together, e.g. a measurement of an observable the
    /// problem has not.
    #[error("{0}")]
    Invalid(String),
    /// A prior which is no prior of PEtab v2.
    #[error("the prior {0}")]
    Prior(String),
    /// A part of PEtab v2 the crate does not execute, e.g. a model of another language or an
    /// extension which is required.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// A model which does not compile.
    #[error("the model '{model}' does not compile: {message}")]
    Compile {
        /// The id of the model.
        model: String,
        /// The message of the compiler.
        message: String,
    },
    /// A failure of the engine or of a fit.
    #[error("{0}")]
    Engine(String),
}

impl From<math::MathError> for PetabError {
    fn from(error: math::MathError) -> Self {
        PetabError::Math(error.to_string())
    }
}

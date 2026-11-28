//! The C target of sbmlsim-core: the functions of a model as native code for the engine,
//! without WebAssembly.
//!
//! [`generate`] translates an SBML document into C (`emit.rs`): the functions of the contract
//! `model/2` (`defaults`, `init`, `rhs`, `jac`, `roots`, `trigger`, `priority`, `delay`,
//! `event_values`, `event_apply`, `selections`, `assign`, and `sizes` and `event_sizes`), each
//! named with the prefix of the model ([`symbol_prefix`]), a pure function of `t`, pointers to
//! `y` and `p` and its outputs, and the [`ModelDescription`](sbmlsim_abi::ModelDescription) the
//! WebAssembly target gives the model (without `model_jac_p`, so the forward sensitivities take
//! the difference quotient of the right hand side). [`Library::build`] compiles the C with the C
//! compiler of the machine into a shared library, cached across processes, and loads it;
//! [`CModel`] is its functions as the [`ModelFns`](sbmlsim_engine::ModelFns) of the engine:
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let sbml = std::fs::read_to_string("model.xml")?;
//! let source = sbmlsim_c::generate(&sbml)?;
//! let library = sbmlsim_c::Library::build(&source.code)?;
//! let model = sbmlsim_c::CModel::new(library, &source.description)?;
//! let engine = sbmlsim_engine::Engine::new(Box::new(model), source.description)?;
//! # Ok(())
//! # }
//! ```
//!
//! The math is the one of the WebAssembly model, operation for operation, but with the
//! functions of `<math.h>` (glibc, the C library of macOS) instead of those of mathlib, so the
//! right hand side agrees with the one of `model.wasm` to the last bits of the math functions,
//! and a simulation within the tolerances of the integration, not bitwise.
//!
//! Refused ([`CError::Unsupported`], naming the feature): a model which reads its history (a
//! `delay`), and a model without an analytic Jacobian (a rate which reads `factorial` of a
//! state).
//!
//! [`generate_with`] and [`generate_qfunction_with`] drive species ([`sbmlsim_ir::driven`]): each
//! follows a path of three parameters, which a caller sets per interval (and per point for the
//! QFunction), and the constants behind the parameters are computed again from them, by
//! `assign` of the native model and by every application of the QFunction.
//!
//! [`generate_qfunction`] is the second target: the model with a Rosenbrock integrator (RODAS4)
//! as one libCEED QFunction which advances any number of points (the cells of a tissue) by up to
//! a number of step attempts each per application, for libCEED's CPU backends and its JIT backends
//! (`qfunction.rs`); [`run_points`] runs it as libCEED calls it, without libCEED.

mod build;
mod emit;
mod model;
mod qfunction;

pub use build::{BuildOptions, FLAGS, Library};
pub use emit::{CSource, MAX_NESTING, generate, generate_model, generate_with, symbol_prefix};
pub use model::CModel;
pub use qfunction::{
    CONTROL, CONTROL_DONE, CONTROL_FAILED, CONTROL_H, CONTROL_T, COUNTS, INPUTS, OUTPUTS, Points,
    QSource, ROSENBROCK_ORDER, RosenbrockContext, apply_points, generate_qfunction,
    generate_qfunction_model, generate_qfunction_with, run_points,
};

/// A failure of the C target.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CError {
    /// A model the C target does not support.
    #[error("the C target does not support {feature}")]
    Unsupported {
        /// The feature.
        feature: String,
    },
    /// The C compiler does not start.
    #[error("the C compiler `{command}` was not found (set CC to a C compiler)")]
    NoCompiler {
        /// The compiler, as `CC` gives it.
        command: String,
    },
    /// The C compiler failed.
    #[error("the C compiler `{command}` failed: {stderr}")]
    Compiler {
        /// The compiler, as `CC` gives it.
        command: String,
        /// What it wrote to its standard error.
        stderr: String,
    },
    /// The library cannot be written to the cache or loaded.
    #[error("the compiled model does not load: {0}")]
    Load(String),
    /// A model the reader or the model representation refuses, or a library of another model.
    #[error("{0}")]
    Model(String),
    /// An input of the harness of a QFunction ([`run_points`], [`apply_points`]) which does not
    /// fit its model, or points which do not finish.
    #[error("{0}")]
    Invalid(String),
}

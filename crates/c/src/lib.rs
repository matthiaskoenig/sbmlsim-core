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

mod build;
mod emit;
mod model;

pub use build::{BuildOptions, FLAGS, Library};
pub use emit::{CSource, MAX_NESTING, generate, generate_model, symbol_prefix};
pub use model::CModel;

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
}

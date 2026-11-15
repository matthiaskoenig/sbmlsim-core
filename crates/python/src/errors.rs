//! The exceptions of `sbmlsim_core` and the mapping of the errors of the engine onto them.

use pyo3::prelude::*;
use sbmlsim_engine::EngineError;
use std::sync::PoisonError;

pyo3::create_exception!(
    sbmlsim_core,
    SbmlsimCoreError,
    pyo3::exceptions::PyException,
    "The base class of every error of sbmlsim_core."
);
pyo3::create_exception!(
    sbmlsim_core,
    CompileError,
    SbmlsimCoreError,
    "An SBML model which cannot be compiled: not a document of level 3, not valid, or using a feature the engine does not simulate."
);

pyo3::create_exception!(
    sbmlsim_core,
    SimulationError,
    SbmlsimCoreError,
    "The base class of the errors of a simulation."
);
pyo3::create_exception!(
    sbmlsim_core,
    RequestError,
    SimulationError,
    "A request which the engine refuses: an unknown selection, an invalid time or setting."
);
pyo3::create_exception!(
    sbmlsim_core,
    IntegrationError,
    SimulationError,
    "An integration which failed. `flag` is the return flag of CVODE and `time` the time it reached."
);
pyo3::create_exception!(
    sbmlsim_core,
    UnsupportedError,
    SimulationError,
    "A simulation the engine does not support."
);
pyo3::create_exception!(
    sbmlsim_core,
    InternalError,
    SimulationError,
    "A failure of the engine itself. The instance which raised it is not used again."
);

/// The exception of an error of a simulation, with `flag` and `time` for an integration.
pub(crate) fn simulation_error(error: EngineError) -> PyErr {
    let message = error.to_string();
    match error {
        EngineError::Request(_) => RequestError::new_err(message),
        EngineError::Unsupported(_) => UnsupportedError::new_err(message),
        EngineError::Internal(_) => InternalError::new_err(message),
        EngineError::Integration { flag, time, .. } => {
            let err = IntegrationError::new_err(message);
            Python::attach(|py| {
                let value = err.value(py);
                // setting an attribute of a fresh exception cannot fail
                let _ = value.setattr("flag", flag);
                let _ = value.setattr("time", time);
            });
            err
        }
    }
}

/// The error of an instance whose lock a panic poisoned.
pub(crate) fn poisoned<T>(_: PoisonError<T>) -> EngineError {
    EngineError::Internal("the instance panicked before and is not used again".into())
}

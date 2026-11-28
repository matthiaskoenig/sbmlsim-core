//! The exceptions of `sbmlsim_core` and the mapping of the errors of the engine onto them.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sbmlsim_abi::EngineError;
use sbmlsim_batch::{BatchError, PointFailure};
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
    LoadError,
    SbmlsimCoreError,
    "A model or a runtime which cannot be loaded: a model which does not match its description, an engine which does not start, an instance which cannot be made."
);

pyo3::create_exception!(
    sbmlsim_core,
    UnitError,
    SbmlsimCoreError,
    "A unit expression which is no unit, a unit with an offset, two units of other dimensions or a unit defined again with another value."
);

pyo3::create_exception!(
    sbmlsim_core,
    ScanError,
    SbmlsimCoreError,
    "A scan which cannot be planned or which failed. `kind` is `request` (a request which does not fit itself or the model, or a call out of turn), `units` (a time or a value which cannot be converted into the units of the model), `unsupported` (a simulation which is not supported) or `point` (a point of a scan of `on_error: raise` which failed), and `point` the flat index of that point, none for the other kinds."
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

/// The exception of an error of a scan, with its `kind` and its `point`.
pub(crate) fn scan_error(error: sbmlsim_scan::ScanError) -> PyErr {
    let kind = match error.kind {
        sbmlsim_scan::ScanErrorKind::Request => "request",
        sbmlsim_scan::ScanErrorKind::Units => "units",
        sbmlsim_scan::ScanErrorKind::Unsupported => "unsupported",
        sbmlsim_scan::ScanErrorKind::Point => "point",
    };
    let err = ScanError::new_err(error.message);
    Python::attach(|py| {
        let value = err.value(py);
        // setting an attribute of a fresh exception cannot fail
        let _ = value.setattr("kind", kind);
        let _ = value.setattr("point", error.point);
    });
    err
}

/// The exception of a batch which cannot run: a number of threads which is none, a batch of
/// no model, or a pool of threads which does not start.
pub(crate) fn batch_error(error: BatchError) -> PyErr {
    let message = error.to_string();
    match error {
        BatchError::Threads(_) | BatchError::Models => PyValueError::new_err(message),
        BatchError::Pool { .. } => LoadError::new_err(message),
    }
}

/// The exception of a fit which cannot run: a request which does not resolve, a definition
/// which cannot be fitted, a pool which does not start or a failure of the engine.
pub(crate) fn fit_error(error: sbmlsim_fit::FitError) -> PyErr {
    let message = error.to_string();
    match error {
        sbmlsim_fit::FitError::Engine(error) => simulation_error(error),
        sbmlsim_fit::FitError::Pool(error) => batch_error(error),
        sbmlsim_fit::FitError::Point(_) => InternalError::new_err(message),
        // the request of the fit, which a caller writes: its parameters, its conditions, its
        // curves, its starts and its optimizer
        _ => PyValueError::new_err(message),
    }
}

/// The error of an instance whose lock a panic poisoned.
pub(crate) fn poisoned<T>(_: PoisonError<T>) -> EngineError {
    EngineError::Internal("the instance panicked before and is not used again".into())
}

/// A point of a batch which failed: the error of its simulation as a value, in the position of
/// its request, so that the points around it keep their results.
///
/// It is the counterpart of the `SimulationError`s a single simulation raises: `kind` is
/// `"request"`, `"integration"`, `"unsupported"` or `"internal"`, and `flag` and `time` are the
/// return flag of CVODE and the time it reached of an integration which failed, none otherwise.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct PointError {
    pub(crate) kind: &'static str,
    pub(crate) message: String,
    pub(crate) flag: Option<i32>,
    pub(crate) time: Option<f64>,
}

impl PointError {
    /// The error of a simulation as the failed point of a batch.
    pub(crate) fn of(error: EngineError) -> PointError {
        let message = error.to_string();
        let (kind, flag, time) = match error {
            EngineError::Request(_) => ("request", None, None),
            EngineError::Unsupported(_) => ("unsupported", None, None),
            EngineError::Internal(_) => ("internal", None, None),
            EngineError::Integration { flag, time, .. } => ("integration", Some(flag), Some(time)),
        };
        PointError {
            kind,
            message,
            flag,
            time,
        }
    }

    /// The failure of a point of a batch of `sbmlsim-batch`.
    pub(crate) fn of_failure(failure: PointFailure) -> PointError {
        match failure {
            PointFailure::Engine(error) => PointError::of(error),
            PointFailure::Internal(message) => PointError::internal(message),
        }
    }

    /// A failure of the host around a point: its instance, its table, or a panic.
    pub(crate) fn internal(message: String) -> PointError {
        PointError {
            kind: "internal",
            message,
            flag: None,
            time: None,
        }
    }
}

#[pymethods]
impl PointError {
    /// The kind of the error: `"request"`, `"integration"`, `"unsupported"` or `"internal"`.
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind
    }

    /// The message of the error.
    #[getter]
    fn message(&self) -> String {
        self.message.clone()
    }

    /// The return flag of CVODE of an integration which failed, none for every other error.
    #[getter]
    fn flag(&self) -> Option<i32> {
        self.flag
    }

    /// The time an integration which failed reached, none for every other error.
    #[getter]
    fn time(&self) -> Option<f64> {
        self.time
    }

    /// The kind and the message of the error.
    fn __repr__(&self) -> String {
        format!(
            "PointError(kind={:?}, message={:?})",
            self.kind, self.message
        )
    }
}

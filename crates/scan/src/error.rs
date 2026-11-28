//! The errors of a scan and of its points.

use sbmlsim_abi::EngineError;
use serde::{Deserialize, Serialize};

/// What failed a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanErrorKind {
    /// The request does not fit the model or itself, e.g. a target which is no target of the
    /// model, a time outside of the simulation, an answer which does not fit the question.
    Request,
    /// A time or a value cannot be converted into the units of the model.
    Units,
    /// A simulation the planner or the engine does not simulate as sbmlsim's reference does,
    /// which fails every point of the scan alike.
    Unsupported,
    /// A point of a scan of `on_error: raise` failed.
    Point,
}

/// The error of a scan, in words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ScanError {
    /// What failed.
    pub kind: ScanErrorKind,
    /// The message.
    pub message: String,
    /// The flat index of the point in the scan of an error of [`ScanErrorKind::Point`].
    pub point: Option<usize>,
}

impl ScanError {
    /// An error of the request.
    pub fn request(message: impl Into<String>) -> ScanError {
        ScanError {
            kind: ScanErrorKind::Request,
            message: message.into(),
            point: None,
        }
    }

    /// An error of the units.
    pub fn units(message: impl Into<String>) -> ScanError {
        ScanError {
            kind: ScanErrorKind::Units,
            message: message.into(),
            point: None,
        }
    }

    /// A simulation which is not supported.
    pub fn unsupported(message: impl Into<String>) -> ScanError {
        ScanError {
            kind: ScanErrorKind::Unsupported,
            message: message.into(),
            point: None,
        }
    }

    /// The failure of a point.
    pub fn point(point: usize, message: impl Into<String>) -> ScanError {
        ScanError {
            kind: ScanErrorKind::Point,
            message: message.into(),
            point: Some(point),
        }
    }

    /// The error with a text before its message, e.g. the simulation it is about.
    pub(crate) fn within(mut self, prefix: &str) -> ScanError {
        self.message = format!("{prefix}: {}", self.message);
        self
    }
}

/// What failed a point of a batch of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointErrorKind {
    /// The engine refused the request.
    Request,
    /// The integration failed.
    Integration,
    /// The engine does not simulate the request.
    Unsupported,
    /// The engine failed in itself.
    Internal,
}

/// A point of a batch which failed, the error of its simulation as a value: the `PointError` a
/// batch of the bindings answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointError {
    /// What failed.
    pub kind: PointErrorKind,
    /// The message of the engine.
    pub message: String,
    /// The return flag of CVODE of an integration which failed.
    #[serde(default)]
    pub flag: Option<i32>,
    /// The time an integration which failed reached.
    #[serde(default)]
    pub time: Option<f64>,
}

impl From<EngineError> for PointError {
    fn from(error: EngineError) -> PointError {
        let message = error.to_string();
        let (kind, flag, time) = match error {
            EngineError::Request(_) => (PointErrorKind::Request, None, None),
            EngineError::Unsupported(_) => (PointErrorKind::Unsupported, None, None),
            EngineError::Internal(_) => (PointErrorKind::Internal, None, None),
            EngineError::Integration { flag, time, .. } => {
                (PointErrorKind::Integration, Some(flag), Some(time))
            }
        };
        PointError {
            kind,
            message,
            flag,
            time,
        }
    }
}

/// The flag of an integration of the engine which did not reach a steady state.
pub const STEADY_STATE_FLAG: i32 = -1004;

/// The failure of a point in the words of sbmlsim: the type of its error and its message, as
/// `executor_core._error_of` writes them, `Err` for an error which fails the whole scan (a
/// simulation the engine does not support).
pub(crate) fn point_failure(error: &PointError, label: &str) -> Result<String, ScanError> {
    let model = format!("the model '{label}'");
    match error.kind {
        PointErrorKind::Unsupported => Err(ScanError::unsupported(format!(
            "The engine 'core' does not simulate {model} as roadrunner: {}",
            error.message
        ))),
        PointErrorKind::Internal => Ok(format!(
            "RuntimeError: The engine 'core' failed internally on {model}, see the bindings of \
             sbmlsim-core: {}",
            error.message
        )),
        PointErrorKind::Integration if error.flag == Some(STEADY_STATE_FLAG) => Ok(format!(
            "SteadyStateError: The engine 'core' failed to simulate {model}: {}",
            error.message
        )),
        PointErrorKind::Integration | PointErrorKind::Request => Ok(format!(
            "RuntimeError: The engine 'core' failed to simulate {model}: {}",
            error.message
        )),
    }
}

//! The instance which simulates requests, and the table it answers with.

use std::collections::HashMap;
use std::sync::Mutex;

use numpy::PyArray2;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sbmlsim_abi::{SimulationRequest, SimulationStats};

use crate::errors::{InternalError, poisoned, simulation_error};

/// A model in an engine of its own; simulates one request after the other.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct Instance {
    // a store is Send and not Sync, which a python object must be; the lock is taken without
    // the GIL, so a second thread which simulates on the instance waits without holding it
    instance: Mutex<sbmlsim_host::Instance>,
}

impl Instance {
    /// Wraps an instance of the host.
    pub(crate) fn new(instance: sbmlsim_host::Instance) -> Instance {
        Instance {
            instance: Mutex::new(instance),
        }
    }
}

#[pymethods]
impl Instance {
    /// Simulates a request of `simulation/1` (a dict) and gives its table.
    ///
    /// Raises `ValueError` for a request which is none (an unknown field, a wrong type) and a
    /// `SimulationError` for one the engine refuses or fails on.
    fn simulate(&self, py: Python<'_>, request: &Bound<'_, PyAny>) -> PyResult<SimulationOutput> {
        let request: SimulationRequest =
            serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(request))
                .map_err(|error| {
                    // the path names the field (`settings.maximum_num_steps`), which serde
                    // alone leaves out for a wrong type
                    PyValueError::new_err(format!("request: {}: {}", error.path(), error.inner()))
                })?;
        let result = py
            .detach(|| self.instance.lock().map_err(poisoned)?.simulate(&request))
            .map_err(simulation_error)?;
        let values = numpy::ndarray::Array2::from_shape_vec(
            (result.n_rows, result.columns.len()),
            result.values,
        )
        .map_err(|error| {
            InternalError::new_err(format!("the table of the engine has no shape: {error}"))
        })?;
        let values = PyArray2::from_owned_array(py, values).unbind();
        Ok(SimulationOutput {
            columns: result.columns,
            values,
        })
    }

    /// The counters of the integrator of the last simulation, by name.
    fn stats(&self, py: Python<'_>) -> PyResult<HashMap<&'static str, u64>> {
        let stats = py
            .detach(|| self.instance.lock().map_err(poisoned)?.stats())
            .map_err(simulation_error)?;
        Ok(SimulationStats::NAMES
            .into_iter()
            .zip(stats.words())
            .collect())
    }
}

/// The table of a simulation.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct SimulationOutput {
    columns: Vec<String>,
    values: Py<PyArray2<f64>>,
}

#[pymethods]
impl SimulationOutput {
    /// The names of the columns, `time` first.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }

    /// The values as an array of float64 of the shape `(n_rows, len(columns))`, in C order.
    #[getter]
    fn values<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray2<f64>> {
        self.values.bind(py).clone()
    }
}

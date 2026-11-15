//! The runtime which loads models, and the loaded model which makes instances.

use std::collections::HashMap;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sbmlsim_abi::ModelDescription;

use crate::errors::SbmlsimCoreError;
use crate::simulate::Instance;

/// `engine.wasm` and `mathlib.wasm`, compiled once, which load models.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct Runtime {
    runtime: sbmlsim_host::Runtime,
}

#[pymethods]
impl Runtime {
    /// The runtime of the embedded engine. `max_memory_bytes` bounds the memory an instance
    /// grows to (the default of the host without it); a result which does not fit is a
    /// `RequestError`.
    #[new]
    #[pyo3(signature = (max_memory_bytes=None))]
    fn new(max_memory_bytes: Option<usize>) -> PyResult<Runtime> {
        let mut runtime = sbmlsim_host::Runtime::embedded()
            .map_err(|error| SbmlsimCoreError::new_err(format!("{error:#}")))?;
        if let Some(bytes) = max_memory_bytes {
            runtime = runtime.with_max_memory(bytes);
        }
        Ok(Runtime { runtime })
    }

    /// Loads `model.wasm` with its description (`model/2`), as `CompiledModel.wasm` and
    /// `CompiledModel.description` give them. Raises `ValueError` for a description which is
    /// none and `SbmlsimCoreError` for a model which does not match it.
    fn load(
        &self,
        py: Python<'_>,
        wasm: &[u8],
        description: &Bound<'_, PyAny>,
    ) -> PyResult<LoadedModel> {
        let description: ModelDescription = pythonize::depythonize(description)
            .map_err(|error| PyValueError::new_err(format!("description: {error}")))?;
        let model = py
            .detach(|| self.runtime.load(wasm, &description))
            .map_err(|error| SbmlsimCoreError::new_err(format!("{error:#}")))?;
        Ok(LoadedModel {
            model: Arc::new(model),
        })
    }
}

/// A model compiled for an engine, which makes the instances that simulate it.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct LoadedModel {
    model: Arc<sbmlsim_host::LoadedModel>,
}

#[pymethods]
impl LoadedModel {
    /// The description of the model, `model/2`, as a dict.
    #[getter]
    fn description<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize::pythonize(py, self.model.description()).map_err(Into::into)
    }

    /// The values of the selections of the model as loaded, by the id of the selection.
    fn initial_values(&self, py: Python<'_>) -> PyResult<HashMap<String, f64>> {
        let values = py
            .detach(|| self.model.initial_selections())
            .map_err(|error| SbmlsimCoreError::new_err(format!("{error:#}")))?;
        Ok(self
            .model
            .description()
            .selections
            .iter()
            .zip(values)
            .map(|(selection, value)| (selection.id.clone(), value))
            .collect())
    }

    /// A new instance of the model, with a store of its own, which simulates one request after
    /// the other. The result of a simulation does not depend on the ones before it.
    fn instance(&self, py: Python<'_>) -> PyResult<Instance> {
        let instance = py
            .detach(|| self.model.instantiate())
            .map_err(|error| SbmlsimCoreError::new_err(format!("{error:#}")))?;
        Ok(Instance::new(instance))
    }
}

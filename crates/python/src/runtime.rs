//! The runtime which loads models, and the loaded model which makes instances.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sbmlsim_abi::ModelDescription;

use crate::errors::LoadError;
use crate::simulate::{BatchExecutor, Instance, PreparedSimulation, batch, request_of};

/// `engine.wasm` and `mathlib.wasm`, compiled once per runtime or loaded from the persistent
/// Wasmtime compilation cache, which load models.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct Runtime {
    runtime: sbmlsim_host::Runtime,
}

#[pymethods]
impl Runtime {
    /// The runtime of the embedded engine. `max_memory_bytes` bounds the memory an instance
    /// grows to (the default of the host without it); a result which does not fit is a
    /// `RequestError`. `timeout_ms` bounds each host execution (30 seconds by default);
    /// zero disables the deadline. `cache` enables persistent Wasmtime compilation caching;
    /// `cache_dir` selects a directory instead of the platform default. Cache failures fall
    /// back to compilation. The constructor releases the GIL.
    #[new]
    #[pyo3(signature = (max_memory_bytes=None, *, timeout_ms=30_000, cache=true, cache_dir=None))]
    fn new(
        py: Python<'_>,
        max_memory_bytes: Option<usize>,
        timeout_ms: u64,
        cache: bool,
        cache_dir: Option<PathBuf>,
    ) -> PyResult<Runtime> {
        if !cache && cache_dir.is_some() {
            return Err(PyValueError::new_err("cache_dir requires cache=True"));
        }
        let cache = if !cache {
            sbmlsim_host::ModuleCache::Disabled
        } else if let Some(path) = cache_dir {
            sbmlsim_host::ModuleCache::Directory(path)
        } else {
            sbmlsim_host::ModuleCache::Default
        };
        let mut runtime = py
            .detach(|| sbmlsim_host::Runtime::embedded_with_cache(cache))
            .map_err(|error| LoadError::new_err(format!("{error:#}")))?;
        if let Some(bytes) = max_memory_bytes {
            runtime = runtime.with_max_memory(bytes);
        }
        runtime = runtime.with_execution_timeout(
            (timeout_ms != 0).then(|| std::time::Duration::from_millis(timeout_ms)),
        );
        Ok(Runtime { runtime })
    }

    /// The active cache directory as a pathlib.Path, or None if disabled or unavailable.
    #[getter]
    fn cache_dir(&self) -> Option<PathBuf> {
        self.runtime.cache_directory().map(PathBuf::from)
    }

    /// Loads `model.wasm` with its description (`model/2`), as `CompiledModel.wasm` and
    /// `CompiledModel.description` give them. Raises `ValueError` for a description which is
    /// none and `LoadError` for a model which does not match it.
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
            .map_err(|error| LoadError::new_err(format!("{error:#}")))?;
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
    /// The description of the model, `model/2`, as a dict, built anew by every access (up to
    /// the size of the description, 704 kB of JSON for a large model), so read it once.
    #[getter]
    fn description<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize::pythonize(py, self.model.description()).map_err(Into::into)
    }

    /// The values of the selections of the model as loaded, by the id of the selection,
    /// evaluated anew by every call (the model is linked in a store of its own and the
    /// initial values are read from it), so call it once and keep the dict.
    fn initial_values(&self, py: Python<'_>) -> PyResult<HashMap<String, f64>> {
        let values = py
            .detach(|| self.model.initial_selections())
            .map_err(|error| LoadError::new_err(format!("{error:#}")))?;
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
            .detach(|| self.model.instantiate_native())
            .map_err(|error| LoadError::new_err(format!("{error:#}")))?;
        Ok(Instance::new(instance))
    }

    /// Validates a reusable request and ordered numeric input IDs.
    #[pyo3(signature = (request, *, varying=None))]
    fn prepare(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        varying: Option<Vec<String>>,
    ) -> PyResult<PreparedSimulation> {
        let request = request_of(request, "request")?;
        let varying = varying.unwrap_or_default();
        let prepared = py
            .detach(|| self.model.prepare(&request, &varying))
            .map_err(|error| PyValueError::new_err(format!("{error:#}")))?;
        Ok(PreparedSimulation {
            prepared,
            model: self.model.clone(),
        })
    }

    /// A persistent batch executor, with at most eight workers by default.
    #[pyo3(signature = (*, threads=None))]
    fn batch_executor(&self, threads: Option<i64>) -> PyResult<BatchExecutor> {
        BatchExecutor::new(self.model.clone(), threads)
    }

    /// Simulates every request of `simulation/1` (dicts) on `threads` threads and gives one
    /// result per request, in the order of the requests: a `SimulationOutput` for a point
    /// which ran and a `PointError` for one which failed, so that a point which fails leaves
    /// the points around it their results.
    ///
    /// Every thread simulates on an instance of its own, so the results are those of a serial
    /// run bitwise, whatever `threads` is. The parallelism of a batch is
    /// `min(threads, len(requests))`, and its pool and its instances live only for the
    /// duration of the call (an instance is made per thread at its first point, which costs
    /// 0.3 s for a model of 700 kB of description), so give a batch as many points as you can
    /// rather than many small batches. `threads` is every CPU of the process when it is
    /// none; raises `ValueError` for fewer than 1 and for more than 1024 threads and for a
    /// request which is none (an unknown field, a wrong type), before anything is simulated,
    /// and `KeyboardInterrupt` for a batch which is interrupted, which drops the results of
    /// the points which ran.
    #[pyo3(signature = (requests, *, threads=None))]
    fn simulate_many(
        &self,
        py: Python<'_>,
        requests: &Bound<'_, PyAny>,
        threads: Option<i64>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        batch(py, &self.model, requests, threads)
    }
}

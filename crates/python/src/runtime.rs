//! The runtime which loads models, and the loaded model which makes instances.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use sbmlsim_abi::ModelDescription;

use crate::errors::{InternalError, LoadError, fit_error};
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

    /// Fits the parameters of the model to data and gives what the fit found.
    ///
    /// `request` is a fit of the contract `fit/1` (a dict): its parameters with their bounds
    /// and their scale, a request of `simulation/1` per condition, the data of every curve,
    /// the optimizer to run and the points to start from. The answer is a dict of the same
    /// contract: the parameters of the optimum, its cost, its residuals, the counters of the
    /// optimizer and every run of the fit.
    ///
    /// The fit runs on a thread of its own without the GIL, and every evaluation of its
    /// objective is one batch of the engine on `threads` threads (the ones of the request,
    /// else every CPU of the process): the residuals of a point are computed on the thread of
    /// the point, so nothing of python is built per simulation. Ctrl-C stops the optimizer at
    /// its next iteration and raises `KeyboardInterrupt`.
    ///
    /// `on_run` is called with the index of a run of the fit and the run itself (a dict of
    /// the answer's `runs`) as soon as that run finished, which is what a caller of a long
    /// fit reports its progress by, stores its runs by and keeps the runs of an interrupted
    /// fit by: the runs of a fit which is stopped are reported before `KeyboardInterrupt` is
    /// raised, and the runs of one which finished are all reported before it answers. It is
    /// called on this thread and not on the ones of the fit, so it holds the GIL and the fit
    /// runs on while it does; an error it raises stops the fit and reaches the caller.
    ///
    /// Raises `ValueError` for a request which is none of the contract and for a fit which
    /// cannot run (a curve of a condition the fit has not, a parameter the model has not, a
    /// start outside the bounds), and the errors of a simulation for a failure of the engine;
    /// a failed integration is no error, it is a point the optimizer may not take.
    #[pyo3(signature = (request, *, threads=None, on_run=None))]
    fn fit<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'_, PyAny>,
        threads: Option<usize>,
        on_run: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        run_fit(
            py,
            vec![(sbmlsim_fit::DEFAULT_MODEL.to_string(), self.model.clone())],
            request,
            threads,
            on_run,
        )
    }

    /// The problem of a fit of the contract `fit/1`, for an optimizer which stays in python.
    ///
    /// The conditions of the request are prepared once and the column and the row of every
    /// measurement are resolved once, so a call of the problem transfers the values of the
    /// parameters and nothing else. `FitProblem.residuals` and `FitProblem.jacobian` are one
    /// batch of the engine each, on `threads` threads (the ones of the request, else every
    /// CPU of the process), and hold no GIL; `LoadedModel.fit` runs the whole fit instead.
    ///
    /// Raises the errors of `LoadedModel.fit`: the starts of the request are not read here.
    #[pyo3(signature = (request, *, threads=None))]
    fn fit_problem(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        threads: Option<usize>,
    ) -> PyResult<crate::fit::FitProblem> {
        let request: sbmlsim_fit::FitRequest =
            serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(request))
                .map_err(|error| {
                    PyValueError::new_err(format!("request: {}: {}", error.path(), error.inner()))
                })?;
        let model = self.model.clone();
        let problem = py
            .detach(|| sbmlsim_fit::problem_of(&request, model, threads))
            .map_err(fit_error)?;
        Ok(crate::fit::FitProblem::new(problem))
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
    /// request which is none (an unknown field, a wrong type), and `KeyboardInterrupt` for a
    /// batch which is interrupted. The batch reads its requests while its pool simulates the
    /// ones it read before, so a request which is none and an interrupt stop it where it is
    /// and drop the results of the points which ran.
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

/// Reports the runs of a fit which finished since the last report to `on_run`.
///
/// Called on the thread of the caller of `LoadedModel.fit` with the GIL, see there. Nothing
/// is reported for a fit which was not asked to report, whose runs are no channel.
fn report_runs<'py>(
    py: Python<'py>,
    runs: &std::sync::mpsc::Receiver<(usize, sbmlsim_fit::RunAnswer)>,
    on_run: Option<&Bound<'py, PyAny>>,
) -> PyResult<()> {
    let Some(on_run) = on_run else {
        return Ok(());
    };
    while let Ok((index, run)) = runs.try_recv() {
        on_run.call1((index, pythonize::pythonize(py, &run)?))?;
    }
    Ok(())
}

/// Runs a fit of the contract `fit/1` on the given models, see `LoadedModel.fit`.
fn run_fit<'py>(
    py: Python<'py>,
    models: Vec<(String, Arc<sbmlsim_host::LoadedModel>)>,
    request: &Bound<'_, PyAny>,
    threads: Option<usize>,
    on_run: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let request: sbmlsim_fit::FitRequest =
        serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(request))
            .map_err(|error| {
                PyValueError::new_err(format!("request: {}: {}", error.path(), error.inner()))
            })?;
    let stop = sbmlsim_fit::Stop::new();
    let (sender, runs) = std::sync::mpsc::channel::<(usize, sbmlsim_fit::RunAnswer)>();
    let reporting = on_run.is_some();
    let worker = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("sbmlsim-fit".to_string())
            .spawn(move || {
                if !reporting {
                    drop(sender);
                    return sbmlsim_fit::fit_models(&request, models, threads, &stop);
                }
                // the runs are reported from the threads of the pool and are sent to the
                // thread of the caller, which is the one that may hold the GIL, so the
                // sender of them is locked: one lock per run of the fit
                let sender = std::sync::Mutex::new(sender);
                let report = |index: usize, run: &sbmlsim_fit::RunAnswer| {
                    let sender = sender.lock().unwrap_or_else(|held| held.into_inner());
                    drop(sender.send((index, run.clone())));
                };
                sbmlsim_fit::fit_reporting_models(&request, models, threads, &stop, Some(&report))
            })
            .map_err(|error| InternalError::new_err(format!("the fit has no thread: {error}")))?
    };
    // the fit holds no GIL, so the signals of python are checked here while it runs: an
    // interrupt stops the optimizer at its next iteration and the thread is joined, which
    // is what keeps a stopped fit from reaching into the model of a caller which went on
    while !worker.is_finished() {
        py.detach(|| std::thread::sleep(std::time::Duration::from_millis(20)));
        if let Err(error) = report_runs(py, &runs, on_run.as_ref()) {
            stop.stop();
            py.detach(|| drop(worker.join()));
            return Err(error);
        }
        if let Err(interrupt) = py.check_signals() {
            stop.stop();
            py.detach(|| drop(worker.join()));
            // the runs which finished before the fit was stopped are what a caller keeps
            // of it; an error of the report is none of the interrupt it asked for
            drop(report_runs(py, &runs, on_run.as_ref()));
            return Err(interrupt);
        }
    }
    let answer = py
        .detach(|| worker.join())
        .map_err(|_| InternalError::new_err("the fit panicked"))?
        .map_err(fit_error)?;
    // the runs which finished while the fit was joined are reported as well, so every run
    // of the answer was reported before it is answered
    report_runs(py, &runs, on_run.as_ref())?;
    pythonize::pythonize(py, &answer).map_err(Into::into)
}

/// The models of a fit, in the order of the mapping, which is the order of their instances.
fn models_of(
    models: &Bound<'_, PyDict>,
) -> PyResult<Vec<(String, Arc<sbmlsim_host::LoadedModel>)>> {
    if models.is_empty() {
        return Err(PyValueError::new_err("a fit simulates at least one model"));
    }
    let mut of = Vec::with_capacity(models.len());
    for (id, model) in models.iter() {
        let id: String = id.extract().map_err(|_| {
            PyValueError::new_err("the models of a fit are keyed by the id of a model")
        })?;
        let model: PyRef<'_, LoadedModel> = model.extract().map_err(|_| {
            PyValueError::new_err(format!("the model '{id}' of the fit is no loaded model"))
        })?;
        of.push((id, model.model.clone()));
    }
    Ok(of)
}

/// Fits the parameters of several models to data and gives what the fit found.
///
/// This is `LoadedModel.fit` of a fit of more than one model, e.g. of data which is the data
/// of two builds of a model: `models` are the loaded models by the id the conditions of the
/// request name them by, and every condition of the request names the model it simulates
/// (`model` of a condition of `fit/1`). A thread of the fit holds an instance of every model
/// its points met, so a fit of two models is twice the memory of a fit of one.
///
/// Raises `ValueError` for a mapping which is empty or holds something which is no loaded
/// model, for a condition which names a model the fit has not or names none at all, and
/// otherwise as `LoadedModel.fit`.
#[pyfunction]
#[pyo3(signature = (models, request, *, threads=None, on_run=None))]
pub(crate) fn fit_models<'py>(
    py: Python<'py>,
    models: &Bound<'_, PyDict>,
    request: &Bound<'_, PyAny>,
    threads: Option<usize>,
    on_run: Option<Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    run_fit(py, models_of(models)?, request, threads, on_run)
}

/// The problem of a fit of several models, for an optimizer which stays in python.
///
/// This is `LoadedModel.fit_problem` of a fit of more than one model, see `fit_models`.
///
/// Raises the errors of `fit_models`: the starts of the request are not read here.
#[pyfunction]
#[pyo3(signature = (models, request, *, threads=None))]
pub(crate) fn fit_problem_of_models(
    py: Python<'_>,
    models: &Bound<'_, PyDict>,
    request: &Bound<'_, PyAny>,
    threads: Option<usize>,
) -> PyResult<crate::fit::FitProblem> {
    let request: sbmlsim_fit::FitRequest =
        serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(request))
            .map_err(|error| {
                PyValueError::new_err(format!("request: {}: {}", error.path(), error.inner()))
            })?;
    let models = models_of(models)?;
    let problem = py
        .detach(|| sbmlsim_fit::problem_of_models(&request, models, threads))
        .map_err(fit_error)?;
    Ok(crate::fit::FitProblem::new(problem))
}

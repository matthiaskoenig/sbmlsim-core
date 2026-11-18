//! The instance which simulates requests, the table it answers with, and the batch of requests
//! over the threads of a pool of `sbmlsim-batch`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use numpy::PyArray2;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use sbmlsim_abi::EngineError;
use sbmlsim_abi::{SimulationRequest, SimulationResult, SimulationStats};
use sbmlsim_batch::{Outcome, WorkerPool, Workers, thread_count};

use crate::errors::{InternalError, PointError, batch_error, poisoned, simulation_error};

/// A model in an engine of its own; simulates one request after the other.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct Instance {
    // a store is Send and not Sync, which a python object must be; the lock is taken without
    // the GIL, so a second thread which simulates on the instance waits without holding it
    instance: Mutex<sbmlsim_host::NativeInstance>,
}

impl Instance {
    /// Wraps an instance of the host.
    pub(crate) fn new(instance: sbmlsim_host::NativeInstance) -> Instance {
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
        let request = request_of(request, "request")?;
        let result = py
            .detach(|| self.instance.lock().map_err(poisoned)?.simulate(&request))
            .map_err(simulation_error)?;
        output(py, result).map_err(InternalError::new_err)
    }

    /// Runs a prepared request with numeric inputs copied before releasing the GIL.
    #[pyo3(signature = (prepared, values=None))]
    fn simulate_prepared(
        &self,
        py: Python<'_>,
        prepared: &PreparedSimulation,
        values: Option<Vec<f64>>,
    ) -> PyResult<SimulationOutput> {
        let values = values.unwrap_or_default();
        let result = py
            .detach(|| {
                self.instance
                    .lock()
                    .map_err(poisoned)?
                    .simulate_prepared(&prepared.prepared, &values)
            })
            .map_err(simulation_error)?;
        output(py, result).map_err(InternalError::new_err)
    }

    /// The counters of the integrator of the last simulation, by name.
    fn stats(&self, py: Python<'_>) -> PyResult<HashMap<&'static str, u64>> {
        // the native engine answers with the counters themselves and not with a result, since
        // reading them needs no call into a guest
        let stats = py
            .detach(|| -> Result<SimulationStats, EngineError> {
                Ok(self.instance.lock().map_err(poisoned)?.stats())
            })
            .map_err(simulation_error)?;
        Ok(SimulationStats::NAMES
            .into_iter()
            .zip(stats.words())
            .collect())
    }
}

/// Validated request bound to one loaded model.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct PreparedSimulation {
    pub(crate) prepared: sbmlsim_host::PreparedSimulation,
    pub(crate) model: Arc<sbmlsim_host::LoadedModel>,
}
#[pymethods]
impl PreparedSimulation {
    #[getter]
    fn n_values(&self) -> usize {
        self.prepared.n_values()
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
    /// The names of the columns, `time` first, as a new list by every access, so read it once
    /// and not per point.
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

/// The request of `simulation/1` a python object describes; `at` names it in the message of a
/// request which is none (`request`, `requests[3]`).
pub(crate) fn request_of(object: &Bound<'_, PyAny>, at: &str) -> PyResult<SimulationRequest> {
    serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(object)).map_err(
        |error| {
            // the path names the field (`settings.maximum_num_steps`), which serde alone
            // leaves out for a wrong type
            PyValueError::new_err(format!("{at}: {}: {}", error.path(), error.inner()))
        },
    )
}

/// The table of a result as the output of a simulation, or the message of a table which has no
/// shape; the values are the ones of the engine and are not copied.
fn output(py: Python<'_>, result: SimulationResult) -> Result<SimulationOutput, String> {
    let values = numpy::ndarray::Array2::from_shape_vec(
        (result.n_rows, result.columns.len()),
        result.values,
    )
    .map_err(|error| format!("the table of the engine has no shape: {error}"))?;
    Ok(SimulationOutput {
        columns: result.columns,
        values: PyArray2::from_owned_array(py, values).unbind(),
    })
}

/// The requests of a batch, read while the GIL is held, so that a request which is none is
/// refused before anything is simulated.
fn requests_of(requests: &Bound<'_, PyAny>) -> PyResult<Vec<SimulationRequest>> {
    requests
        .try_iter()?
        .enumerate()
        .map(|(at, object)| request_of(&object?, &format!("requests[{at}]")))
        .collect()
}

/// The points of a prepared batch, which are the values of its varying targets.
fn points_of(points: &Bound<'_, PyAny>) -> PyResult<Vec<Vec<f64>>> {
    points
        .try_iter()?
        .enumerate()
        .map(|(at, row)| {
            row?.extract::<Vec<f64>>()
                .map_err(|error| PyValueError::new_err(format!("points[{at}]: {error}")))
        })
        .collect()
}

/// Simulates every request of a batch (`LoadedModel.simulate_many`) and gives one result per
/// request, in the order of the requests.
///
/// The batch runs on a pool of its own, of at most one thread per request and of none at all
/// for a batch without requests, which is an empty list.
pub(crate) fn batch(
    py: Python<'_>,
    model: &Arc<sbmlsim_host::LoadedModel>,
    requests: &Bound<'_, PyAny>,
    threads: Option<i64>,
) -> PyResult<Vec<Py<PyAny>>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let requests = requests_of(requests)?;
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let threads = threads.min(requests.len());
    let workers = py
        .detach(|| Workers::new(model.clone(), threads))
        .map_err(batch_error)?;
    run_slices(py, &workers, &requests, |slice| workers.simulate(slice))
}

/// Runs a batch in slices of `Workers::slice_len` points and gives one object per point, in
/// the order of the points: the table of a point which ran and a `PointError` for one which
/// failed.
///
/// The points of a slice run without the GIL and give the tables of the engine, which become
/// the objects of python between two slices, where the GIL is held again, and where the
/// signals are checked: a thread of the pool never reaches into python, and a long batch is
/// interrupted there.
fn run_slices<T: Sync>(
    py: Python<'_>,
    workers: &Workers,
    items: &[T],
    run: impl Fn(&[T]) -> Vec<Outcome<SimulationResult>> + Sync,
) -> PyResult<Vec<Py<PyAny>>> {
    let mut results = Vec::with_capacity(items.len());
    for slice in items.chunks(workers.slice_len()) {
        let points = py.detach(|| run(slice));
        for outcome in points {
            results.push(match outcome {
                Ok(table) => match output(py, table) {
                    Ok(out) => Py::new(py, out)?.into_any(),
                    Err(message) => Py::new(py, PointError::internal(message))?.into_any(),
                },
                Err(failure) => Py::new(py, PointError::of_failure(failure))?.into_any(),
            });
        }
        py.check_signals()?;
    }
    Ok(results)
}

/// A bounded reusable pool with one active submission at a time.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct BatchExecutor {
    state: Mutex<ExecutorState>,
}

struct ExecutorState {
    closed: bool,
    pool: WorkerPool,
}

/// The default bound of the workers of an executor, which a caller of many small batches holds
/// between them.
const DEFAULT_EXECUTOR_THREADS: usize = 8;

impl BatchExecutor {
    pub(crate) fn new(
        model: Arc<sbmlsim_host::LoadedModel>,
        threads: Option<i64>,
    ) -> PyResult<Self> {
        let threads = match threads {
            None => thread_count(None)
                .map_err(batch_error)?
                .min(DEFAULT_EXECUTOR_THREADS),
            threads => thread_count(threads).map_err(batch_error)?,
        };
        Ok(Self {
            state: Mutex::new(ExecutorState {
                closed: false,
                pool: WorkerPool::new(model, threads),
            }),
        })
    }

    /// The state of the executor, or why it has none: a call of another thread holds it, or it
    /// is closed.
    fn state(&self) -> PyResult<std::sync::MutexGuard<'_, ExecutorState>> {
        let state = self
            .state
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("batch executor is busy"))?;
        if state.closed {
            return Err(PyRuntimeError::new_err("batch executor is closed"));
        }
        Ok(state)
    }
}

#[pymethods]
impl BatchExecutor {
    /// Reuses workers and instances. Concurrent calls raise RuntimeError.
    fn simulate_many(
        &self,
        py: Python<'_>,
        requests: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let mut state = self.state()?;
        let requests = requests_of(requests)?;
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        // the guard of the lock is not Send, the state behind it is, so the pool grows while
        // the GIL is released
        let state = &mut *state;
        py.detach(|| state.pool.ensure(requests.len()))
            .map_err(batch_error)?;
        let workers = state.pool.workers().expect("the pool of a batch which ran");
        run_slices(py, workers, &requests, |slice| workers.simulate(slice))
    }

    /// Runs numeric rows in order on the retained workers.
    fn simulate_prepared(
        &self,
        py: Python<'_>,
        prepared: &PreparedSimulation,
        points: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let mut state = self.state()?;
        if !Arc::ptr_eq(state.pool.model(), &prepared.model) {
            return Err(PyValueError::new_err(
                "prepared simulation belongs to another model",
            ));
        }
        let points = points_of(points)?;
        if points.is_empty() {
            return Ok(Vec::new());
        }
        let state = &mut *state;
        py.detach(|| state.pool.ensure(points.len()))
            .map_err(batch_error)?;
        let workers = state.pool.workers().expect("the pool of a batch which ran");
        run_slices(py, workers, &points, |slice| {
            workers.simulate_prepared(&prepared.prepared, slice)
        })
    }

    /// Idempotently releases retained resources. Concurrent calls raise RuntimeError.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("batch executor is busy"))?;
        state.closed = true;
        let workers = state.pool.take();
        // the threads of the pool and the instances of the threads go without the GIL
        py.detach(|| drop(workers));
        Ok(())
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        drop(slf.state()?);
        Ok(slf)
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _ty: &Bound<'_, PyAny>,
        _value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.close(py)
    }
}

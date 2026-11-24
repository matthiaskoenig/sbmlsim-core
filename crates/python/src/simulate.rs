//! The instance which simulates requests, the table it answers with, and the batch of requests
//! over the threads of a pool of `sbmlsim-batch`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use numpy::{PyArray2, PyArray3};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use sbmlsim_abi::EngineError;
use sbmlsim_abi::{SimulationRequest, SimulationResult, SimulationStats};
use sbmlsim_batch::{Plan, WorkerPool, Workers, thread_count};
use sbmlsim_host::NativeInstance;

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

    /// The statistics of the integrator of the last simulation, by name: the counts as
    /// integers, the times and steps after them as floats.
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        // the native engine answers with the counters themselves and not with a result, since
        // reading them needs no call into a guest
        let stats = py
            .detach(|| -> Result<SimulationStats, EngineError> {
                Ok(self.instance.lock().map_err(poisoned)?.stats())
            })
            .map_err(simulation_error)?;
        let out = PyDict::new(py);
        for (at, (name, word)) in SimulationStats::NAMES
            .into_iter()
            .zip(stats.words())
            .enumerate()
        {
            if at < SimulationStats::COUNTS {
                out.set_item(name, word)?;
            } else {
                out.set_item(name, f64::from_bits(word))?;
            }
        }
        Ok(out)
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
    sensitivities: Option<Py<OutputSensitivities>>,
}

/// The forward sensitivities of the columns of a table.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct OutputSensitivities {
    parameters: Vec<String>,
    values: Py<PyArray3<f64>>,
}

#[pymethods]
impl OutputSensitivities {
    /// The ids of the parameters, the `sensitivities` of the request, in their order; a new
    /// list by every access.
    #[getter]
    fn parameters(&self) -> Vec<String> {
        self.parameters.clone()
    }

    /// The derivatives as an array of float64 of the shape
    /// `(n_rows, len(columns), len(parameters))`, in C order: the derivative of the column `c`
    /// of the row `r` in the parameter `j` is `values[r, c, j]`. The column `time` is no
    /// function of a parameter, so its derivatives are 0.
    #[getter]
    fn values<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray3<f64>> {
        self.values.bind(py).clone()
    }
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

    /// The forward sensitivities of the columns, `None` for a request which did not ask for
    /// them (`sensitivities` of the request, which only the native engine carries).
    #[getter]
    fn sensitivities(&self, py: Python<'_>) -> Option<Py<OutputSensitivities>> {
        self.sensitivities
            .as_ref()
            .map(|sensitivities| sensitivities.clone_ref(py))
    }
}

/// The request of `simulation/1` a python object describes; `at` names it in the message of a
/// request which is none (`request`, `requests[3]`).
pub(crate) fn request_of(object: &Bound<'_, PyAny>, at: &str) -> PyResult<SimulationRequest> {
    parse_request(object).map_err(|message| PyValueError::new_err(format!("{at}: {message}")))
}

/// The request of `simulation/1` a python object describes, or the message of one which is
/// none.
///
/// The request is read without the path of its fields first, and a request which is none
/// is read again with it for the message: tracking the path costs as much as reading the
/// rest of the request, and a batch reads all of its requests on the one thread which holds
/// the GIL.
fn parse_request(object: &Bound<'_, PyAny>) -> Result<SimulationRequest, String> {
    pythonize::depythonize(object).or_else(|_| {
        serde_path_to_error::deserialize(&mut pythonize::Depythonizer::from_object(object))
            // the path names the field (`settings.maximum_num_steps`), which serde alone
            // leaves out for a wrong type
            .map_err(|error| format!("{}: {}", error.path(), error.inner()))
    })
}

/// The table of a result as the output of a simulation, or the message of a table which has no
/// shape; the values are the ones of the engine and are not copied.
fn output(py: Python<'_>, result: SimulationResult) -> Result<SimulationOutput, String> {
    let values = numpy::ndarray::Array2::from_shape_vec(
        (result.n_rows, result.columns.len()),
        result.values,
    )
    .map_err(|error| format!("the table of the engine has no shape: {error}"))?;
    let sensitivities = result
        .sensitivities
        .map(|sensitivities| {
            let shape = (
                result.n_rows,
                result.columns.len(),
                sensitivities.parameters.len(),
            );
            let derivatives = numpy::ndarray::Array3::from_shape_vec(shape, sensitivities.values)
                .map_err(|error| {
                format!("the sensitivities of the engine have no shape: {error}")
            })?;
            Py::new(
                py,
                OutputSensitivities {
                    parameters: sensitivities.parameters,
                    values: PyArray3::from_owned_array(py, derivatives).unbind(),
                },
            )
            .map_err(|error| format!("the sensitivities of the engine: {error}"))
        })
        .transpose()?;
    Ok(SimulationOutput {
        columns: result.columns,
        values: PyArray2::from_owned_array(py, values).unbind(),
        sensitivities,
    })
}

/// The objects of an iterable of a batch, its requests, points or plans, which the batch reads
/// while its pool simulates the ones it read before.
fn objects_of(items: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
    items
        .try_iter()?
        .map(|object| object.map(Bound::unbind))
        .collect()
}

/// The request of the point `at` of a batch.
fn request_at(object: &Bound<'_, PyAny>, at: usize) -> PyResult<SimulationRequest> {
    parse_request(object)
        .map_err(|message| PyValueError::new_err(format!("requests[{at}]: {message}")))
}

/// The point `at` of a prepared batch, which are the values of its varying targets.
fn point_at(object: &Bound<'_, PyAny>, at: usize) -> PyResult<Vec<f64>> {
    object
        .extract::<Vec<f64>>()
        .map_err(|error| PyValueError::new_err(format!("points[{at}]: {error}")))
}

/// The plan `at` of a batch, a pair of a prepared request and the values of its varying
/// targets; `model` is the model of the pool the plans run on, which every one of them belongs
/// to.
fn plan_at(
    object: &Bound<'_, PyAny>,
    at: usize,
    model: &Arc<sbmlsim_host::LoadedModel>,
) -> PyResult<Plan> {
    let (prepared, values) = object
        .extract::<(PyRef<'_, PreparedSimulation>, Vec<f64>)>()
        .map_err(|error| PyValueError::new_err(format!("plans[{at}]: {error}")))?;
    if !Arc::ptr_eq(model, &prepared.model) {
        return Err(PyValueError::new_err(format!(
            "plans[{at}]: prepared simulation belongs to another model"
        )));
    }
    Ok(Plan {
        prepared: prepared.prepared.clone(),
        values,
    })
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
    let requests = objects_of(requests)?;
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let threads = threads.min(requests.len());
    let workers = py
        .detach(|| Workers::new(model.clone(), threads))
        .map_err(batch_error)?;
    run_batch(py, &workers, &requests, request_at, |instance, request| {
        instance.simulate(request)
    })
}

/// How often the thread of a batch checks the signals of python while the pool simulates.
const SIGNALS: Duration = Duration::from_millis(20);

/// Reads every object of a batch with `read` and runs `work` on it, and gives one object per
/// item, in the order of the items: the table of a point which ran and a `PointError` for one
/// which failed.
///
/// The objects are read on the thread of the caller, which holds the GIL for a range of them
/// at a time, while the pool simulates the points of the ranges before without it: reading a
/// request costs a few percent of a short simulation, and every point read before the pool
/// starts is a point the threads of the pool wait for. The tables of the engine become the
/// objects of python after the last point. The thread of the caller checks the signals of
/// python while the pool simulates, since a thread of the pool never reaches into python, and
/// an interrupt, or an object which is none, stops the batch: the points which have not
/// started do not run, and the error is raised once the points which run have finished.
fn run_batch<T: Send + Sync>(
    py: Python<'_>,
    workers: &Workers,
    objects: &[Py<PyAny>],
    read: impl Fn(&Bound<'_, PyAny>, usize) -> PyResult<T> + Sync,
    work: impl Fn(&mut NativeInstance, &T) -> Result<SimulationResult, EngineError> + Sync,
) -> PyResult<Vec<Py<PyAny>>> {
    let points = py.detach(|| {
        workers.map_fed(
            objects.len(),
            |range| Python::attach(|py| range.map(|at| read(objects[at].bind(py), at)).collect()),
            &work,
            SIGNALS,
            || Python::attach(|py| py.check_signals()),
        )
    })?;
    points
        .into_iter()
        .map(|outcome| {
            Ok(match outcome {
                Ok(table) => match output(py, table) {
                    Ok(out) => Py::new(py, out)?.into_any(),
                    Err(message) => Py::new(py, PointError::internal(message))?.into_any(),
                },
                Err(failure) => Py::new(py, PointError::of_failure(failure))?.into_any(),
            })
        })
        .collect()
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
        let requests = objects_of(requests)?;
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        // the guard of the lock is not Send, the state behind it is, so the pool grows while
        // the GIL is released
        let state = &mut *state;
        py.detach(|| state.pool.ensure(requests.len()))
            .map_err(batch_error)?;
        let workers = state.pool.workers().expect("the pool of a batch which ran");
        run_batch(py, workers, &requests, request_at, |instance, request| {
            instance.simulate(request)
        })
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
        let points = objects_of(points)?;
        if points.is_empty() {
            return Ok(Vec::new());
        }
        let state = &mut *state;
        py.detach(|| state.pool.ensure(points.len()))
            .map_err(batch_error)?;
        let workers = state.pool.workers().expect("the pool of a batch which ran");
        run_batch(py, workers, &points, point_at, |instance, values| {
            instance.simulate_prepared(&prepared.prepared, values)
        })
    }

    /// Runs the plans of a batch on the retained workers, in order: a plan is a prepared
    /// request with the values of its varying targets, so the plans of one batch are requests
    /// of their own, and one evaluation of an objective, or the whole Jacobian of one, is a
    /// batch instead of one call per condition.
    fn simulate_plans(&self, py: Python<'_>, plans: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
        let mut state = self.state()?;
        let plans = objects_of(plans)?;
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let state = &mut *state;
        py.detach(|| state.pool.ensure(plans.len()))
            .map_err(batch_error)?;
        let workers = state.pool.workers().expect("the pool of a batch which ran");
        let model = workers.model().clone();
        let read = move |plan: &Bound<'_, PyAny>, at: usize| plan_at(plan, at, &model);
        run_batch(py, workers, &plans, read, |instance, plan| {
            instance.simulate_prepared(&plan.prepared, &plan.values)
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

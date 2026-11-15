//! The instance which simulates requests, the table it answers with, and the batch of requests
//! over the threads of a rayon pool.

use std::any::Any;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, PoisonError};

use numpy::PyArray2;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rayon::prelude::*;
use sbmlsim_abi::{SimulationRequest, SimulationResult, SimulationStats};
use sbmlsim_engine::EngineError;

use crate::errors::{InternalError, PointError, SbmlsimCoreError, poisoned, simulation_error};

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
        let request = request_of(request, "request")?;
        let result = py
            .detach(|| self.instance.lock().map_err(poisoned)?.simulate(&request))
            .map_err(simulation_error)?;
        output(py, result).map_err(InternalError::new_err)
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

/// The request of `simulation/1` a python object describes; `at` names it in the message of a
/// request which is none (`request`, `requests[3]`).
fn request_of(object: &Bound<'_, PyAny>, at: &str) -> PyResult<SimulationRequest> {
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

/// How many requests of a batch one thread of the pool gets per slice.
const SLICE_PER_THREAD: usize = 8;

/// The most threads a batch runs on. More of them is no use for simulations, which are CPU
/// work alone, and a thread costs a stack and an instance, so a number nothing bounds is an
/// allocation the process would be killed for rather than an error it answers with.
const MAX_THREADS: usize = 1024;

/// The instance a thread of the pool simulates on, taken out of its slot while it simulates.
type Slot = Mutex<Option<sbmlsim_host::Instance>>;

/// One point of a batch: the table of its simulation, or the error which failed it.
enum Point {
    Table(SimulationResult),
    Failed(PointError),
}

/// The number of threads of a batch: the given one, or every CPU of the process without it.
fn thread_count(threads: Option<i64>) -> PyResult<usize> {
    let Some(threads) = threads else {
        return Ok(std::thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .min(MAX_THREADS));
    };
    usize::try_from(threads)
        .ok()
        .filter(|threads| (1..=MAX_THREADS).contains(threads))
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "threads must be at least 1 and at most {MAX_THREADS}, not {threads}"
            ))
        })
}

/// The message of the payload of a panic.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic of no message")
}

/// Simulates one point of a batch on the instance of its thread; gives its error and never
/// raises, so that one point never ends the batch.
fn point(model: &sbmlsim_host::LoadedModel, slots: &[Slot], request: &SimulationRequest) -> Point {
    // a panic of wasmtime or of the host fails its own point and no other one; the instance of
    // the point is dropped while it unwinds, so the thread gets a fresh one for its next point
    // and the poisoned lock of its slot is read as the empty slot it is
    catch_unwind(AssertUnwindSafe(|| simulate_point(model, slots, request))).unwrap_or_else(
        |payload| {
            Point::Failed(PointError::internal(format!(
                "the point panicked: {}",
                panic_message(&*payload)
            )))
        },
    )
}

/// Simulates a point on the instance of the thread of the pool which runs it, which it takes
/// out of its slot while it simulates and puts back for the points after it.
fn simulate_point(
    model: &sbmlsim_host::LoadedModel,
    slots: &[Slot],
    request: &SimulationRequest,
) -> Point {
    // the slot of the thread which runs the point, which no other thread reads, so that its
    // lock is never contended; a point outside of the pool, which rayon does not run, and one
    // whose slot a panic left empty simulate on an instance of their own
    let slot = rayon::current_thread_index().and_then(|thread| slots.get(thread));
    let mut slot = slot.map(|slot| slot.lock().unwrap_or_else(PoisonError::into_inner));
    let mut instance = match slot.as_mut().and_then(|slot| slot.take()) {
        Some(instance) => instance,
        None => match model.instantiate() {
            Ok(instance) => instance,
            Err(error) => {
                return Point::Failed(PointError::internal(format!(
                    "the point has no instance: {error:#}"
                )));
            }
        },
    };
    let result = instance.simulate(request);
    // an instance which trapped refuses every request after it (`EngineError::Internal`), so
    // it is dropped here and the thread gets a fresh one for its next point
    if let Some(slot) = slot.as_mut()
        && !matches!(result, Err(EngineError::Internal(_)))
    {
        **slot = Some(instance);
    }
    match result {
        Ok(table) => Point::Table(table),
        Err(error) => Point::Failed(PointError::of(error)),
    }
}

/// Simulates every request of a batch (`LoadedModel.simulate_many`) and gives one result per
/// request, in the order of the requests.
///
/// The batch runs in slices of `SLICE_PER_THREAD * threads` requests on a pool of its own, of
/// at most one thread per request and of none at all for a batch without requests, which is an
/// empty list; every thread of the pool simulates on an instance of its own, made at its first
/// point and reused by every point after it, the ones of a later slice as well, so the results
/// are those of a serial run bitwise, whatever the threads and the slices are. Between two
/// slices the GIL is taken back, the results of the slice are built and the signals are
/// checked, which is where a long batch is interrupted.
pub(crate) fn batch(
    py: Python<'_>,
    model: &sbmlsim_host::LoadedModel,
    requests: &Bound<'_, PyAny>,
    threads: Option<i64>,
) -> PyResult<Vec<Py<PyAny>>> {
    let threads = thread_count(threads)?;
    // the requests are read while the GIL is held, so a request which is none is refused
    // before anything is simulated
    let requests: Vec<SimulationRequest> = requests
        .try_iter()?
        .enumerate()
        .map(|(at, object)| request_of(&object?, &format!("requests[{at}]")))
        .collect::<PyResult<_>>()?;
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    // a thread without a point of its own is a thread started for nothing, and rayon's pool
    // does not wait for its threads when it is dropped, so a caller of many small batches
    // would hold more threads than it asked for
    let threads = threads.min(requests.len());
    let pool = py
        .detach(|| rayon::ThreadPoolBuilder::new().num_threads(threads).build())
        .map_err(|error| {
            SbmlsimCoreError::new_err(format!("no pool of {threads} threads: {error}"))
        })?;
    let slots: Vec<Slot> = (0..threads).map(|_| Mutex::new(None)).collect();
    let mut results = Vec::with_capacity(requests.len());
    for slice in requests.chunks(SLICE_PER_THREAD * threads) {
        // the points of a slice are simulated without the GIL and give the tables of the
        // engine, which become the objects of python here, where the GIL is held again: a
        // thread of the pool never reaches into python
        let points: Vec<Point> = py.detach(|| {
            pool.install(|| {
                slice
                    .par_iter()
                    .map(|request| point(model, &slots, request))
                    .collect()
            })
        });
        for outcome in points {
            results.push(match outcome {
                Point::Table(table) => match output(py, table) {
                    Ok(out) => Py::new(py, out)?.into_any(),
                    Err(message) => Py::new(py, PointError::internal(message))?.into_any(),
                },
                Point::Failed(error) => Py::new(py, error)?.into_any(),
            });
        }
        py.check_signals()?;
    }
    Ok(results)
}

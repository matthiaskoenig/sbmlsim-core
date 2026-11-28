//! The planner of a scan of sbmlsim (`sbmlsim-scan`) as an object of python, which
//! `sbmlsim_core.scan` wraps.
//!
//! The request `scan/1` arrives as the JSON text of the dict of the caller, so its mappings
//! keep the order of the caller (a mapping of python is ordered, a map of serde is not
//! necessarily), and is read straight into the request of the planner. The rounds of the
//! planner reach python as dicts: the requests of `simulation/1` (`ask`), and the native time
//! and columns of the observables of the host as arrays of numpy. `advance` runs the rounds of
//! simulations on the workers of a `BatchExecutor` (one per model of a dimension of models)
//! without a request ever becoming an object of python. The result `scanresult/1` is a dict whose arrays are arrays of numpy of float64
//! in the shape of their variable, built from the vectors of the planner without JSON, so
//! `NaN` and the infinities cross as the values they are.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use numpy::ndarray::{ArrayD, IxDyn};
use numpy::{PyArray1, PyArrayDyn, PyArrayMethods, PyReadonlyArray1, PyUntypedArrayMethods};
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};
use sbmlsim_abi::ModelDescription;
use sbmlsim_abi::SimulationResult;
use sbmlsim_batch::PointFailure;
use sbmlsim_scan::{
    Ask, CoordValues, HostError, HostObservables, HostObserved, Layout, PlanRequest,
    PointErrorKind, Scan, ScanError, ScanRequest, ScanResult,
};

use crate::compile::CompiledModel;
use crate::errors::{PointError, scan_error};
use crate::runtime::LoadedModel;
use crate::simulate::{BatchExecutor, SimulationOutput};

/// A scan planned against a loaded model: the state machine of `sbmlsim-scan`, asked for its
/// rounds and told their answers.
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct ScanPlanner {
    // `None` once the result was taken
    scan: Mutex<Option<Scan>>,
    // the points of the block which ran when the progress was reported last
    reported: Mutex<usize>,
    // the key of the model of the last round of simulations `ask` gave, `None` for the model
    // of the run
    round_model: Mutex<Option<String>>,
}

impl ScanPlanner {
    /// The scan, or why there is none: a call of another thread holds it, or its result was
    /// taken.
    fn scan(&self) -> PyResult<MutexGuard<'_, Option<Scan>>> {
        let guard = self
            .scan
            .try_lock()
            .map_err(|_| PyRuntimeError::new_err("the scan is busy in another call"))?;
        if guard.is_none() {
            return Err(scan_error(ScanError::request(
                "The result of the scan was taken: a scan gives its result once.",
            )));
        }
        Ok(guard)
    }

    /// Runs `work` on the scan.
    fn with<T>(&self, work: impl FnOnce(&mut Scan) -> PyResult<T>) -> PyResult<T> {
        let mut guard = self.scan()?;
        work(guard.as_mut().expect("a scan which was not taken"))
    }

    /// The progress to report, when points ran since the last report.
    fn to_report(&self) -> PyResult<Option<(usize, usize)>> {
        let (done, total) = self.with(|scan| Ok(scan.progress()))?;
        let mut reported = self
            .reported
            .lock()
            .map_err(|_| PyRuntimeError::new_err("the scan panicked before"))?;
        if done > *reported {
            *reported = done;
            Ok(Some((done, total)))
        } else {
            Ok(None)
        }
    }
}

/// A round of the scan as the pair of python: `("simulate", [request, ...])`,
/// `("observe", {...})` or `("done", None)`.
fn round<'py>(py: Python<'py>, ask: Ask) -> PyResult<Bound<'py, PyTuple>> {
    match ask {
        Ask::Simulate { requests, .. } => {
            let requests = pythonize::pythonize(py, &requests)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            PyTuple::new(py, [PyString::new(py, "simulate").into_any(), requests])
        }
        Ask::Observe(question) => PyTuple::new(
            py,
            [
                PyString::new(py, "observe").into_any(),
                question_dict(py, question)?.into_any(),
            ],
        ),
        Ask::Done => PyTuple::new(
            py,
            [
                PyString::new(py, "done").into_any(),
                py.None().into_bound(py),
            ],
        ),
    }
}

/// The question of a round of the observables of the host: `{"observables": [id, ...],
/// "points": [{"index": i, "time": array, "columns": {symbol: array}}, ...], "scalars":
/// [symbol, ...]}`.
fn question_dict(py: Python<'_>, question: HostObservables) -> PyResult<Bound<'_, PyDict>> {
    let points = PyList::empty(py);
    for point in question.points {
        let columns = PyDict::new(py);
        for (symbol, values) in point.columns {
            columns.set_item(symbol, PyArray1::from_vec(py, values))?;
        }
        let entry = PyDict::new(py);
        entry.set_item("index", point.index)?;
        entry.set_item("time", PyArray1::from_vec(py, point.time))?;
        entry.set_item("columns", columns)?;
        points.append(entry)?;
    }
    let out = PyDict::new(py);
    out.set_item("observables", question.observables)?;
    out.set_item("points", points)?;
    out.set_item("scalars", question.scalars)?;
    Ok(out)
}

/// The kind of a point error of the engine by its name.
fn point_error_kind(kind: &str) -> PointErrorKind {
    match kind {
        "request" => PointErrorKind::Request,
        "integration" => PointErrorKind::Integration,
        "unsupported" => PointErrorKind::Unsupported,
        _ => PointErrorKind::Internal,
    }
}

/// The answer of a request: the table of a `SimulationOutput`, or the error of a `PointError`.
fn answer_of(
    object: &Bound<'_, PyAny>,
    at: usize,
) -> PyResult<Result<SimulationResult, sbmlsim_scan::PointError>> {
    if let Ok(output) = object.cast::<SimulationOutput>() {
        let output = output.get();
        let values = output.values.bind(object.py());
        let n_rows = values.shape()[0];
        let values = values
            .to_vec()
            .map_err(|error| PyValueError::new_err(format!("results[{at}]: {error}")))?;
        return Ok(Ok(SimulationResult {
            columns: output.columns.clone(),
            n_rows,
            values,
            sensitivities: None,
            local_errors: None,
        }));
    }
    if let Ok(error) = object.cast::<PointError>() {
        let error = error.get();
        return Ok(Err(sbmlsim_scan::PointError {
            kind: point_error_kind(error.kind),
            message: error.message.clone(),
            flag: error.flag,
            time: error.time,
        }));
    }
    Err(PyTypeError::new_err(format!(
        "results[{at}] is a {}, not a SimulationOutput or a PointError",
        object.get_type().name()?
    )))
}

/// The outcome of a point of a batch of the executor as the answer the scan reads.
fn answer_of_outcome(
    outcome: Result<SimulationResult, PointFailure>,
) -> Result<SimulationResult, sbmlsim_scan::PointError> {
    outcome.map_err(|failure| match failure {
        PointFailure::Engine(error) => sbmlsim_scan::PointError::from(error),
        PointFailure::Internal(message) => sbmlsim_scan::PointError {
            kind: PointErrorKind::Internal,
            message,
            flag: None,
            time: None,
        },
    })
}

/// The values of one point of an observable of the host: a number, or a sequence of numbers.
fn point_values(object: &Bound<'_, PyAny>, at: &str) -> PyResult<Vec<f64>> {
    if let Ok(array) = object.extract::<PyReadonlyArray1<'_, f64>>() {
        return Ok(array.as_array().to_vec());
    }
    if let Ok(number) = object.extract::<f64>() {
        return Ok(vec![number]);
    }
    object.extract::<Vec<f64>>().map_err(|_| {
        PyTypeError::new_err(format!(
            "{at} is neither a number nor a sequence of numbers"
        ))
    })
}

/// The answer of the host to a round of its observables, from the dict of python
/// `{"values": {id: [values of every point]}, "errors": [{"observable", "index", "message"}]}`.
fn observed_of(answer: &Bound<'_, PyAny>) -> PyResult<HostObserved> {
    let answer = answer
        .cast::<PyDict>()
        .map_err(|_| PyTypeError::new_err("the answer of the observables is a dict"))?;
    for key in answer.keys() {
        let key: String = key.extract()?;
        if key != "values" && key != "errors" {
            return Err(PyValueError::new_err(format!(
                "the answer of the observables has the unknown key '{key}' (values, errors)"
            )));
        }
    }
    let mut observed = HostObserved {
        values: Default::default(),
        errors: Vec::new(),
    };
    if let Some(values) = answer.get_item("values")? {
        let values = values
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("values is a dict of the observables"))?;
        for (id, points) in values.iter() {
            let id: String = id.extract()?;
            let mut list = Vec::new();
            for (k, point) in points.try_iter()?.enumerate() {
                list.push(point_values(&point?, &format!("values['{id}'][{k}]"))?);
            }
            observed.values.insert(id, list);
        }
    }
    if let Some(errors) = answer.get_item("errors")?
        && !errors.is_none()
    {
        for (k, error) in errors.try_iter()?.enumerate() {
            let error = error?;
            let field = |name: &str| -> PyResult<Bound<'_, PyAny>> {
                error
                    .get_item(name)
                    .map_err(|_| PyValueError::new_err(format!("errors[{k}] has no '{name}'")))
            };
            observed.errors.push(HostError {
                observable: field("observable")?.extract()?,
                index: field("index")?.extract()?,
                message: field("message")?.extract()?,
            });
        }
    }
    Ok(observed)
}

/// An array of numpy of float64 in a shape, the vector moved into it.
fn shaped<'py>(
    py: Python<'py>,
    values: Vec<f64>,
    shape: &[usize],
) -> PyResult<Bound<'py, PyArrayDyn<f64>>> {
    let array = ArrayD::from_shape_vec(IxDyn(shape), values).map_err(|error| {
        PyValueError::new_err(format!("an array of the result has no shape: {error}"))
    })?;
    Ok(PyArrayDyn::from_owned_array(py, array))
}

/// The pairs of a list of tuples.
fn pairs<'py>(py: Python<'py>, pairs: Vec<(String, String)>) -> PyResult<Bound<'py, PyList>> {
    PyList::new(py, pairs)
}

/// The result `scanresult/1` as a dict, see the module.
fn result_dict(py: Python<'_>, result: ScanResult) -> PyResult<Bound<'_, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("contract", result.contract)?;
    let sizes = PyList::empty(py);
    for size in result.sizes {
        let entry = PyDict::new(py);
        entry.set_item("name", size.name)?;
        entry.set_item("size", size.size)?;
        sizes.append(entry)?;
    }
    out.set_item("sizes", sizes)?;
    out.set_item("dims", result.dims)?;
    out.set_item(
        "layout",
        match result.layout {
            Layout::Grid => "grid",
            Layout::Ragged => "ragged",
        },
    )?;
    let coords = PyList::empty(py);
    for coord in result.coords {
        let entry = PyDict::new(py);
        entry.set_item("name", coord.name)?;
        entry.set_item("dims", coord.dims)?;
        entry.set_item("unit", coord.unit)?;
        let values = PyDict::new(py);
        match coord.values {
            CoordValues::Numbers(numbers) => {
                values.set_item("numbers", PyArray1::from_vec(py, numbers))?;
            }
            CoordValues::Labels(labels) => {
                let labels = pythonize::pythonize(py, &labels)
                    .map_err(|error| PyValueError::new_err(error.to_string()))?;
                values.set_item("labels", labels)?;
            }
            CoordValues::Indices(indices) => {
                let indices: Vec<i64> = indices.into_iter().map(|i| i as i64).collect();
                values.set_item("indices", PyArray1::from_vec(py, indices))?;
            }
        }
        entry.set_item("values", values)?;
        coords.append(entry)?;
    }
    out.set_item("coords", coords)?;
    let variables = PyList::empty(py);
    for variable in result.variables {
        let entry = PyDict::new(py);
        entry.set_item("name", variable.name)?;
        entry.set_item("dims", variable.dims)?;
        entry.set_item("shape", variable.shape.clone())?;
        entry.set_item("unit", variable.unit)?;
        entry.set_item("values", shaped(py, variable.values, &variable.shape)?)?;
        variables.append(entry)?;
    }
    out.set_item("variables", variables)?;
    out.set_item("units", pairs(py, result.units)?)?;
    out.set_item("changes", PyArray1::from_vec(py, result.changes))?;
    match result.errors {
        None => out.set_item("errors", py.None())?,
        Some(errors) => {
            let list = PyList::empty(py);
            for error in errors {
                let entry = PyDict::new(py);
                entry.set_item("index", error.index)?;
                entry.set_item("point", error.point)?;
                entry.set_item("message", error.message)?;
                list.append(entry)?;
            }
            out.set_item("errors", list)?;
        }
    }
    match result.block {
        None => out.set_item("block", py.None())?,
        Some(block) => {
            let entry = PyDict::new(py);
            entry.set_item("start", block.start)?;
            entry.set_item("stop", block.stop)?;
            entry.set_item("shape", block.shape)?;
            entry.set_item("dims", block.dims)?;
            entry.set_item("coords", pairs(py, block.coords)?)?;
            out.set_item("block", entry)?;
        }
    }
    Ok(out)
}

#[pymethods]
impl ScanPlanner {
    /// Plans the scan of the request `scan/1`, given as its JSON text, against a loaded model,
    /// the model of the run, or against the loaded models of a dimension of models by their
    /// keys.
    ///
    /// Raises `ScanError` for a request which is no such JSON or which does not fit itself or
    /// its models.
    #[new]
    #[pyo3(signature = (request, model=None, models=None))]
    fn new(
        py: Python<'_>,
        request: &str,
        model: Option<&LoadedModel>,
        models: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<ScanPlanner> {
        let mut reader = serde_json::Deserializer::from_str(request);
        let request: ScanRequest =
            serde_path_to_error::deserialize(&mut reader).map_err(|error| {
                scan_error(ScanError::request(format!(
                    "The request is no request of scan/1: {}: {}",
                    error.path(),
                    error.inner()
                )))
            })?;
        let description = model.map(|model| model.model.description().clone());
        let mut descriptions = Vec::new();
        if let Some(models) = models {
            for (key, model) in models.iter() {
                let key: String = key.extract().map_err(|_| {
                    PyTypeError::new_err("the keys of the models are the keys of the dimension")
                })?;
                let model = model.cast::<LoadedModel>().map_err(|_| {
                    PyTypeError::new_err(format!("the model '{key}' is no LoadedModel"))
                })?;
                descriptions.push((key, model.get().model.description().clone()));
            }
        }
        let scan = py
            .detach(|| Scan::open_with_models(request, description, descriptions))
            .map_err(scan_error)?;
        Ok(ScanPlanner {
            scan: Mutex::new(Some(scan)),
            reported: Mutex::new(0),
            round_model: Mutex::new(None),
        })
    }

    /// The next round: `("simulate", [request, ...])`, `("observe", {...})` or
    /// `("done", None)`; the model of a round of simulations is `round_model`.
    fn ask<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let ask = self.with(|scan| Ok(py.detach(|| scan.ask())))?;
        if let Ask::Simulate { model, .. } = &ask {
            *self
                .round_model
                .lock()
                .map_err(|_| PyRuntimeError::new_err("the scan panicked before"))? = model.clone();
        }
        round(py, ask)
    }

    /// The key of the model of the requests of the last round of simulations `ask` gave in the
    /// dimension of models, `None` for the model of the run.
    #[getter]
    fn round_model(&self) -> PyResult<Option<String>> {
        Ok(self
            .round_model
            .lock()
            .map_err(|_| PyRuntimeError::new_err("the scan panicked before"))?
            .clone())
    }

    /// Takes the answers of the round of simulations, one `SimulationOutput` or `PointError`
    /// per request in their order.
    fn tell(&self, py: Python<'_>, results: &Bound<'_, PyAny>) -> PyResult<()> {
        let answers = results
            .try_iter()?
            .enumerate()
            .map(|(at, object)| answer_of(&object?, at))
            .collect::<PyResult<Vec<_>>>()?;
        self.with(|scan| py.detach(|| scan.tell(answers)).map_err(scan_error))
    }

    /// Takes the values of the round of the observables of the host.
    fn tell_observed(&self, py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let observed = observed_of(values)?;
        self.with(|scan| {
            py.detach(|| scan.tell_observed(observed))
                .map_err(scan_error)
        })
    }

    /// Runs the rounds of simulations on the workers of `executor` (the model of the run) and
    /// of `executors` (the models of a dimension of models by their keys) until the scan asks
    /// for the observables of the host or is done, and gives that round as `ask` does
    /// (`("observe", {...})` or `("done", None)`), or `("model", key)` for a round of a model
    /// of the dimension which `executors` has no executor of, which the caller opens before it
    /// advances again (the round is asked again); `on_batch(done, total)` is called with the
    /// points of the block which ran whenever more of them ran. With `observe` false the
    /// caller computes no observables of the host, and a round of them raises `ScanError` of
    /// the kind `request`.
    #[pyo3(signature = (executor, on_batch=None, executors=None, observe=true))]
    fn advance<'py>(
        &self,
        py: Python<'py>,
        executor: Option<PyRef<'py, BatchExecutor>>,
        on_batch: Option<&Bound<'py, PyAny>>,
        executors: Option<HashMap<String, PyRef<'py, BatchExecutor>>>,
        observe: bool,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let executors = executors.unwrap_or_default();
        loop {
            if let Some((done, total)) = self.to_report()?
                && let Some(on_batch) = on_batch
            {
                on_batch.call1((done, total))?;
            }
            let ask = self.with(|scan| Ok(py.detach(|| scan.ask())))?;
            let Ask::Simulate { model, requests } = ask else {
                if matches!(ask, Ask::Observe(_)) && !observe {
                    return Err(scan_error(ScanError::request(
                        "The scan has observables of the host (Custom) and run was given no \
                         observe function to compute them.",
                    )));
                }
                return round(py, ask);
            };
            let executor = match &model {
                None => executor.as_deref().ok_or_else(|| {
                    PyValueError::new_err("advance was given no executor of the model of the run")
                })?,
                Some(key) => match executors.get(key) {
                    Some(executor) => &**executor,
                    None => {
                        return PyTuple::new(
                            py,
                            [
                                PyString::new(py, "model").into_any(),
                                PyString::new(py, key).into_any(),
                            ],
                        );
                    }
                },
            };
            let outcomes = executor.simulate_requests(py, &requests)?;
            drop(requests);
            let answers = outcomes.into_iter().map(answer_of_outcome).collect();
            self.with(|scan| py.detach(|| scan.tell(answers)).map_err(scan_error))?;
        }
    }

    /// The keys of the models of a dimension of models whose points have all run, whose
    /// executors a host may close.
    fn finished_models(&self) -> PyResult<Vec<String>> {
        self.with(|scan| Ok(scan.finished_models()))
    }

    /// The points of the block which ran and the points of the block, `(done, total)`.
    #[getter]
    fn progress(&self) -> PyResult<(usize, usize)> {
        self.with(|scan| Ok(scan.progress()))
    }

    /// The result `scanresult/1` of a scan which is done, as a dict; a scan gives it once.
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let scan = {
            let mut guard = self.scan()?;
            guard
                .as_ref()
                .expect("a scan which was not taken")
                .finished()
                .map_err(scan_error)?;
            guard.take().expect("a scan which was not taken")
        };
        let result = py.detach(|| scan.result()).map_err(scan_error)?;
        result_dict(py, result)
    }
}

/// The description of a model of `compile_plans`: a `CompiledModel`, a `LoadedModel` or the
/// description `model/2` as a dict.
fn description_of(model: &Bound<'_, PyAny>, what: &str) -> PyResult<ModelDescription> {
    if let Ok(compiled) = model.cast::<CompiledModel>() {
        return Ok(compiled.get().model_description().clone());
    }
    if let Ok(loaded) = model.cast::<LoadedModel>() {
        return Ok(loaded.get().model.description().clone());
    }
    pythonize::depythonize(model).map_err(|error| {
        PyTypeError::new_err(format!(
            "{what} is no CompiledModel, LoadedModel or description of model/2: {error}"
        ))
    })
}

/// Compiles the plans of a scan, `plan/1`: every simulation of the request `scan/1`, given as
/// its JSON text, against the model of the run or against the models of a dimension of models
/// by their keys (each a `CompiledModel`, a `LoadedModel` or the description `model/2` as a
/// dict). Returns the JSON text of the plans.
///
/// Raises `ScanError` for a request which is no such JSON or which does not fit itself or its
/// models.
#[pyfunction]
#[pyo3(signature = (request, model=None, models=None))]
pub(crate) fn compile_plans(
    py: Python<'_>,
    request: &str,
    model: Option<&Bound<'_, PyAny>>,
    models: Option<&Bound<'_, PyDict>>,
) -> PyResult<String> {
    let mut reader = serde_json::Deserializer::from_str(request);
    let request: PlanRequest = serde_path_to_error::deserialize(&mut reader).map_err(|error| {
        scan_error(ScanError::request(format!(
            "The request is no request of scan/1: {}: {}",
            error.path(),
            error.inner()
        )))
    })?;
    let description = model
        .filter(|model| !model.is_none())
        .map(|model| description_of(model, "the model"))
        .transpose()?;
    let mut descriptions = Vec::new();
    if let Some(models) = models {
        for (key, model) in models.iter() {
            let key: String = key.extract().map_err(|_| {
                PyTypeError::new_err("the keys of the models are the keys of the dimension")
            })?;
            let description = description_of(&model, &format!("the model '{key}'"))?;
            descriptions.push((key, description));
        }
    }
    let compiled = py
        .detach(|| sbmlsim_scan::compile_plans(&request, description, descriptions))
        .map_err(scan_error)?;
    serde_json::to_string(&compiled)
        .map_err(|error| PyRuntimeError::new_err(format!("the plans as JSON: {error}")))
}

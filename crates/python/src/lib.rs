//! The python bindings of sbmlsim-core: the extension module `sbmlsim_core._core`, which the
//! package `sbmlsim_core` re-exports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use numpy::PyArray2;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use sbmlsim_abi::{
    JacobianChoice, MODEL_CONTRACT, ModelDescription, SIMULATION_CONTRACT, SimulationRequest,
    SimulationStats,
};
use sbmlsim_compiler::{CompileOptions, CompiledModel as Compiled};
use sbmlsim_engine::EngineError;

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

/// A model compiled to `model.wasm` and its description (`model/2`).
#[pyclass(frozen, module = "sbmlsim_core")]
struct CompiledModel {
    compiled: Compiled,
}

#[pymethods]
impl CompiledModel {
    /// The module `model.wasm`.
    #[getter]
    fn wasm<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.compiled.wasm)
    }

    /// The description of the model, `model.json` of `model/2`, as a dict, built anew
    /// by every access.
    #[getter]
    fn description<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize::pythonize(py, &self.compiled.description).map_err(Into::into)
    }
}

/// The source of the Jacobian named by the python argument.
fn jacobian_choice(name: &str) -> PyResult<JacobianChoice> {
    match name {
        "auto" => Ok(JacobianChoice::Auto),
        "analytic" => Ok(JacobianChoice::Analytic),
        "difference_quotient" => Ok(JacobianChoice::DifferenceQuotient),
        other => Err(PyValueError::new_err(format!(
            "jacobian must be 'auto', 'analytic' or 'difference_quotient', not '{other}'"
        ))),
    }
}

/// Compiles the text of an SBML Level 3 document into `model.wasm` and its description.
///
/// `optimize` runs the optimizer of the compiler, whose results are bitwise those without it.
/// `jacobian` is `"auto"`, `"analytic"` or `"difference_quotient"`. Raises `CompileError` for
/// a model which cannot be compiled and `ValueError` for another `jacobian`.
#[pyfunction]
#[pyo3(signature = (sbml, *, optimize=true, jacobian="auto"))]
fn compile_sbml(
    py: Python<'_>,
    sbml: &str,
    optimize: bool,
    jacobian: &str,
) -> PyResult<CompiledModel> {
    let options = CompileOptions {
        optimize,
        jacobian: jacobian_choice(jacobian)?,
    };
    let compiled = py
        .detach(|| sbmlsim_compiler::compile(sbml, &options))
        .map_err(|error| CompileError::new_err(error.to_string()))?;
    Ok(CompiledModel { compiled })
}

/// The exception of an error of a simulation, with `flag` and `time` for an integration.
fn simulation_error(error: EngineError) -> PyErr {
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

/// `engine.wasm` and `mathlib.wasm`, compiled once, which load models.
#[pyclass(frozen, module = "sbmlsim_core")]
struct Runtime {
    runtime: sbmlsim_host::Runtime,
}

#[pymethods]
impl Runtime {
    /// The runtime of the embedded engine.
    #[new]
    fn new() -> PyResult<Runtime> {
        let runtime = sbmlsim_host::Runtime::embedded()
            .map_err(|error| SbmlsimCoreError::new_err(format!("{error:#}")))?;
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
struct LoadedModel {
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
        Ok(Instance {
            instance: Mutex::new(instance),
        })
    }
}

/// A model in an engine of its own; simulates one request after the other.
#[pyclass(frozen, module = "sbmlsim_core")]
struct Instance {
    // a store is Send and not Sync, which a python object must be; the lock is taken without
    // the GIL, so a second thread which simulates on the instance waits without holding it
    instance: Mutex<sbmlsim_host::Instance>,
}

/// The error of an instance whose lock a panic poisoned.
fn poisoned<T>(_: PoisonError<T>) -> EngineError {
    EngineError::Internal("the instance panicked before and is not used again".into())
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
struct SimulationOutput {
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

/// The extension module of the package `sbmlsim_core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("MODEL_CONTRACT", MODEL_CONTRACT)?;
    m.add("SIMULATION_CONTRACT", SIMULATION_CONTRACT)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("SbmlsimCoreError", m.py().get_type::<SbmlsimCoreError>())?;
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    m.add("SimulationError", m.py().get_type::<SimulationError>())?;
    m.add("RequestError", m.py().get_type::<RequestError>())?;
    m.add("IntegrationError", m.py().get_type::<IntegrationError>())?;
    m.add("UnsupportedError", m.py().get_type::<UnsupportedError>())?;
    m.add("InternalError", m.py().get_type::<InternalError>())?;
    m.add_class::<CompiledModel>()?;
    m.add_class::<Runtime>()?;
    m.add_class::<LoadedModel>()?;
    m.add_class::<Instance>()?;
    m.add_class::<SimulationOutput>()?;
    m.add_function(wrap_pyfunction!(compile_sbml, m)?)?;
    Ok(())
}

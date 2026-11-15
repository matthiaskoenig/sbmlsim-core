//! The python bindings of sbmlsim-core: the extension module `sbmlsim_core._core`, which the
//! package `sbmlsim_core` re-exports.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use sbmlsim_abi::{JacobianChoice, MODEL_CONTRACT, SIMULATION_CONTRACT};
use sbmlsim_compiler::{CompileOptions, CompiledModel as Compiled};

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

/// The extension module of the package `sbmlsim_core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("MODEL_CONTRACT", MODEL_CONTRACT)?;
    m.add("SIMULATION_CONTRACT", SIMULATION_CONTRACT)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("SbmlsimCoreError", m.py().get_type::<SbmlsimCoreError>())?;
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    m.add_class::<CompiledModel>()?;
    m.add_function(wrap_pyfunction!(compile_sbml, m)?)?;
    Ok(())
}

//! The compiler: SBML to `model.wasm` and its description.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use sbmlsim_abi::JacobianChoice;
use sbmlsim_compiler::{CompileOptions, CompiledModel as Compiled};

use crate::errors::CompileError;

/// A model compiled to `model.wasm` and its description (`model/2`).
#[pyclass(frozen, module = "sbmlsim_core")]
pub(crate) struct CompiledModel {
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
/// `parameter_jacobian=False` omits analytic parameter derivatives and their constants;
/// forward sensitivities then use the numerical parameter-derivative fallback.
#[pyfunction]
#[pyo3(signature = (sbml, *, optimize=true, jacobian="auto", parameter_jacobian=true))]
pub(crate) fn compile_sbml(
    py: Python<'_>,
    sbml: &str,
    optimize: bool,
    jacobian: &str,
    parameter_jacobian: bool,
) -> PyResult<CompiledModel> {
    let options = CompileOptions {
        optimize,
        jacobian: jacobian_choice(jacobian)?,
        parameter_jacobian,
        ..CompileOptions::default()
    };
    let compiled = py
        .detach(|| sbmlsim_compiler::compile(sbml, &options))
        .map_err(|error| CompileError::new_err(error.to_string()))?;
    Ok(CompiledModel { compiled })
}

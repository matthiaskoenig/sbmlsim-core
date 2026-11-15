//! The python bindings of sbmlsim-core: the extension module `sbmlsim_core._core`, which the
//! package `sbmlsim_core` re-exports.

mod compile;
mod errors;
mod runtime;
mod simulate;

use pyo3::prelude::*;
use sbmlsim_abi::{MODEL_CONTRACT, SIMULATION_CONTRACT};

use compile::{CompiledModel, compile_sbml};
use errors::{
    CompileError, IntegrationError, InternalError, PointError, RequestError, SbmlsimCoreError,
    SimulationError, UnsupportedError,
};
use runtime::{LoadedModel, Runtime};
use simulate::{Instance, SimulationOutput};

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
    m.add_class::<PointError>()?;
    m.add_class::<SimulationOutput>()?;
    m.add_function(wrap_pyfunction!(compile_sbml, m)?)?;
    Ok(())
}

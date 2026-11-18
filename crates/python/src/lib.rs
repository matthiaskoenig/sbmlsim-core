//! The python bindings of sbmlsim-core: the extension module `sbmlsim_core._core`, which the
//! package `sbmlsim_core` re-exports.

/// The allocator of the extension module.
///
/// A simulation allocates inside the linear memory of the engine, which no allocator of the host
/// sees, so it is not the simulation this is for. The compiler of a model and Cranelift do
/// allocate on their hot path, which is visible from outside the process: glibc without its
/// thread cache (`GLIBC_TUNABLES=glibc.malloc.tcache_count=0`) costs 16 percent of the
/// compilation of the flat machine perfusion model (919 states), 7 percent of a PBPK model and
/// 2 percent of a `Runtime`, and nothing of a simulation or a batch. mimalloc takes 21.0 percent
/// off that compilation, 17.2 percent off a PBPK model, 7.5 percent off the `load` of the large
/// one and 4.7 percent off a `Runtime`, and lowers the peak resident set size of such a process
/// by 23.0 percent (the arenas of Cranelift and of the compiler, which it reuses better than
/// glibc does); a simulation and a batch are unchanged either way. The wheel keeps its tag and
/// its floor (`manylinux_2_28_x86_64`, no symbol above `GLIBC_2.28`) and grows by 54 kB.
/// `bench/RESULTS.md` ("CPU features, the instruction set and the allocator") has the numbers.
///
/// It is set here and not in a crate which is also an rlib, so that it takes the choice away
/// from no rust caller: a rust program links `sbmlsim-host` and `sbmlsim-compiler` instead.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod compile;
mod errors;
mod fit;
mod runtime;
mod simulate;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use sbmlsim_abi::{MODEL_CONTRACT, SIMULATION_CONTRACT};

use compile::{CompiledModel, compile_sbml};
use errors::{
    CompileError, IntegrationError, InternalError, LoadError, PointError, RequestError,
    SbmlsimCoreError, SimulationError, UnsupportedError,
};
use runtime::{LoadedModel, Runtime};
use simulate::{Instance, SimulationOutput};

/// The extension module of the package `sbmlsim_core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // rust-numpy imports the array API of numpy at the first array it builds, and an interrupt
    // pending then would abort that import as a panic; build one here, where none of ours can
    // be pending, so a Ctrl-C during the first batch is a KeyboardInterrupt
    drop(numpy::PyArray1::<f64>::zeros(m.py(), 0, false));
    m.add("MODEL_CONTRACT", MODEL_CONTRACT)?;
    m.add("SIMULATION_CONTRACT", SIMULATION_CONTRACT)?;
    m.add("FIT_CONTRACT", sbmlsim_fit::FIT_CONTRACT)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("THIRD_PARTY", sbmlsim_host::THIRD_PARTY_NOTICES)?;
    m.add("SbmlsimCoreError", m.py().get_type::<SbmlsimCoreError>())?;
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    m.add("LoadError", m.py().get_type::<LoadError>())?;
    m.add("SimulationError", m.py().get_type::<SimulationError>())?;
    m.add("RequestError", m.py().get_type::<RequestError>())?;
    m.add("IntegrationError", m.py().get_type::<IntegrationError>())?;
    m.add("UnsupportedError", m.py().get_type::<UnsupportedError>())?;
    m.add("InternalError", m.py().get_type::<InternalError>())?;
    m.add_class::<CompiledModel>()?;
    m.add_class::<Runtime>()?;
    m.add_class::<LoadedModel>()?;
    m.add_class::<fit::FitProblem>()?;
    m.add_class::<simulate::BatchExecutor>()?;
    m.add_class::<simulate::PreparedSimulation>()?;
    m.add_class::<Instance>()?;
    m.add_class::<PointError>()?;
    m.add_class::<SimulationOutput>()?;
    m.add_function(wrap_pyfunction!(compile_sbml, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_wasm, m)?)?;
    Ok(())
}

/// The bytes of the runtime this extension embeds, `engine.wasm` and `mathlib.wasm` (the ones of
/// `sbmlsim-core runtime` and of the npm package of the release), by the keys `engine` and
/// `mathlib`: what a page which simulates the models compiled here runs them with.
#[pyfunction]
fn runtime_wasm(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let files = PyDict::new(py);
    files.set_item("engine", PyBytes::new(py, sbmlsim_host::ENGINE_WASM))?;
    files.set_item("mathlib", PyBytes::new(py, sbmlsim_host::MATHLIB_WASM))?;
    Ok(files)
}

//! The python bindings of sbmlsim-core: the extension module `sbmlsim_core._core`, which the
//! package `sbmlsim_core` re-exports.

/// The allocator of the extension module on every target but macOS.
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
///
/// On macOS it is not a choice this wheel is allowed to make, and the macOS wheel links no
/// mimalloc at all. mimalloc keeps the heap of a thread in a thread local, and on macOS, where a
/// first access to a `__thread` variable can call `malloc` and recurse into the allocator, it
/// does not use one: it claims two slots of the pthread TLS block by compiled in number (108 and
/// 109 in mimalloc v3, which is what `mimalloc 0.1` builds unless asked for v2, which claimed
/// 89) and reads them with inline assembly off `tpidrro_el0`. The slots are process wide and the
/// numbers are baked into the artifact, so two extension modules which each link a copy of
/// mimalloc own the same two: every allocation of the one reads the heap the other left in the
/// slot, takes blocks out of the free lists of pages owned by a heap of the other instance
/// without a lock, because the heap of a thread needs none, and frees blocks through a page map
/// which never saw the arena they came from. What follows is heap corruption, and its symptom is
/// anything at all: an impossible value, a dependency panicking on a state it cannot reach, a
/// SIGBUS while a backtrace is symbolized. mimalloc's own header says it of the model it picks
/// there, "This goes wrong though if the OS or a library uses the same fixed slot".
///
/// This was observed and not deduced, next to this very wheel. `pkpdutils-core` linked the same
/// mimalloc as its own `#[global_allocator]`, and the test suite of `sbmlsim`, which imports
/// both extensions into one interpreter, failed on macOS and only on macOS (sbmlsim#565): 25
/// failures and 26 errors with both modules loaded, a clean run of the same commit with the
/// other module held back, and clean runs on linux x86_64, linux aarch64 and windows with both.
/// That module fixed it on its side first (pkpdutils-core#11), which took the collision out of
/// that pair; the hazard is a property of this wheel, not of the pair, since any third extension
/// which links mimalloc v3 brings it back and a wheel cannot know what else an interpreter will
/// load. The two slot numbers are not reserved either: mimalloc moved from 89 to 108 and 109
/// between v2 and v3 because 89 "seems used now", so pinning the other major version would only
/// move the collision to whatever pins that one. On macOS the system allocator it is (#40).
///
/// Every other platform keeps mimalloc, because no other platform has the collision: the ELF
/// targets give each shared object its own initial-exec TLS block and windows allocates a slot
/// per instance with `TlsAlloc`. What macOS gives up is not measured: the figures above are
/// against glibc malloc on linux, and macOS allocates through libmalloc, which is another design
/// with its own per thread caches, so they are no estimate of what the gate costs there.
///
/// `sbmlsim_core.__allocator__` reports which of the two a build is, so that the question is
/// answered by the artifact and not by this comment.
#[cfg(not(target_os = "macos"))]
mod allocator {
    /// The value of `__allocator__` of the module.
    pub const NAME: &str = "mimalloc";

    #[global_allocator]
    static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
}

/// The allocator of the extension module on macOS: the system one, for the reason above.
#[cfg(target_os = "macos")]
mod allocator {
    /// The value of `__allocator__` of the module.
    pub const NAME: &str = "system";
}

mod chaos;
mod compile;
mod errors;
mod fit;
mod ode;
mod petab;
mod plot;
mod runtime;
mod sensitivity;
mod simulate;
mod stats;
mod thin;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use sbmlsim_abi::{MODEL_CONTRACT, SIMULATION_CONTRACT};

use compile::{CompiledModel, compile_sbml};
use errors::{
    CompileError, IntegrationError, InternalError, LoadError, PointError, RequestError,
    SbmlsimCoreError, SimulationError, UnsupportedError,
};
use runtime::{LoadedModel, Runtime};
use simulate::{Instance, OutputLocalErrors, OutputSensitivities, SimulationOutput};

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
    m.add("FIT_REDUCTIONS", sbmlsim_fit::REDUCTIONS.to_vec())?;
    m.add("FIT_FIELDS", sbmlsim_fit::FIELDS.to_vec())?;
    m.add("FIGURE_CONTRACT", sbmlsim_plot::FIGURE_CONTRACT)?;
    m.add("COLOR_SCALES", sbmlsim_plot::COLOR_SCALES.to_vec())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    // the allocator this build routes its allocations through, which is not the same on
    // every platform: see `mod allocator`
    m.add("__allocator__", allocator::NAME)?;
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
    m.add_class::<OutputSensitivities>()?;
    m.add_class::<OutputLocalErrors>()?;
    m.add_function(wrap_pyfunction!(runtime::fit_models, m)?)?;
    m.add_function(wrap_pyfunction!(runtime::fit_problem_of_models, m)?)?;
    m.add_class::<petab::PetabProblemPy>()?;
    m.add("ODE_CONTRACT", sbmlsim_ode::ODE_CONTRACT)?;
    m.add_class::<ode::EquationSystem>()?;
    m.add_function(wrap_pyfunction!(compile_sbml, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_wasm, m)?)?;
    m.add_function(wrap_pyfunction!(plot_wasm, m)?)?;
    m.add_function(wrap_pyfunction!(chaos::chaos_expansion, m)?)?;
    m.add_function(wrap_pyfunction!(chaos::chaos_design_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(chaos::chaos_predict, m)?)?;
    m.add_function(wrap_pyfunction!(plot::render_svg, m)?)?;
    m.add_function(wrap_pyfunction!(plot::render_png, m)?)?;
    m.add_function(wrap_pyfunction!(plot::figure_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(plot::text_width, m)?)?;
    m.add_function(wrap_pyfunction!(plot::pick, m)?)?;
    m.add_function(wrap_pyfunction!(plot::color_scale, m)?)?;
    m.add_function(wrap_pyfunction!(plot::image_png, m)?)?;
    m.add_function(wrap_pyfunction!(thin::thin_kept_points, m)?)?;
    // the statistics of the analyses, `sbmlsim_core.stats`
    stats::register(m)?;
    m.add_function(wrap_pyfunction!(
        sensitivity::sensitivity_sobol_sequence,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(sensitivity::sensitivity_saltelli, m)?)?;
    m.add_function(wrap_pyfunction!(sensitivity::sensitivity_fast_sample, m)?)?;
    m.add_function(wrap_pyfunction!(sensitivity::sensitivity_morris_sample, m)?)?;
    m.add_function(wrap_pyfunction!(sensitivity::sensitivity_sobol_indices, m)?)?;
    m.add_function(wrap_pyfunction!(sensitivity::sensitivity_fast_indices, m)?)?;
    m.add_function(wrap_pyfunction!(
        sensitivity::sensitivity_morris_effects,
        m
    )?)?;
    Ok(())
}

/// The bytes of the runtime this extension embeds, `engine.wasm`, `mathlib.wasm` and
/// `compiler.wasm` (the ones of `sbmlsim-core runtime` and of the npm package of the release),
/// by the keys `engine`, `mathlib` and `compiler`: what a page which simulates the models
/// compiled here runs them with, and the compiler it compiles a model of its own with
/// (`js/compile.mjs`).
#[pyfunction]
fn runtime_wasm(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let files = PyDict::new(py);
    files.set_item("engine", PyBytes::new(py, sbmlsim_host::ENGINE_WASM))?;
    files.set_item("mathlib", PyBytes::new(py, sbmlsim_host::MATHLIB_WASM))?;
    files.set_item("compiler", PyBytes::new(py, sbmlsim_host::COMPILER_WASM))?;
    Ok(files)
}

/// The bytes of `plot.wasm` this extension embeds (the one of `sbmlsim-core runtime` and of the
/// npm package of the release), the renderer of the figures in a page: the SVG of a `figure/1`
/// spec byte for byte as `render_svg` gives it, its geometry and the pick of a point, through
/// the C ABI of `crates/plot-wasm`.
#[pyfunction]
fn plot_wasm(py: Python<'_>) -> Bound<'_, PyBytes> {
    PyBytes::new(py, sbmlsim_host::PLOT_WASM)
}

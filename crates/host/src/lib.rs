//! The wasmtime host of sbmlsim-core: links mathlib, a compiled model and the engine.
//!
//! A [`Runtime`] loads `engine.wasm` and `mathlib.wasm` from Wasmtime's persistent compilation
//! cache, or compiles them on a miss (the embedded [`ENGINE_WASM`]
//! and [`MATHLIB_WASM`], or others); [`Runtime::instantiate`] instantiates, in a store of their
//! own, a memory of the type the engine imports, mathlib, the model (which imports that memory
//! as `env.memory` and the functions of mathlib as `math.*`) and the engine (which imports the
//! memory, the functions of the model as `model.*` and WASI preview1), so every call between
//! them is a direct call from wasm to wasm, and loads the description of the model. An
//! [`Instance`] simulates requests with it.
//!
//! The engine gets a minimal WASI preview1 adapter with bounded local stdout and stderr
//! capture and an empty environment. Other capabilities trap; it calls none while it loads or simulates
//! ([`Instance::wasi_calls`] counts the calls). The exports of `engine.wasm` and the format of
//! its output are documented in `crates/engine/src/exports.rs`.
//!
//! The memory of an instance grows to at most [`MAX_MEMORY_BYTES`] (a resource limiter of the
//! store, as `StoreLimits`; the JavaScript loader gives its memory the same `maximum`), so a
//! `memory.grow` beyond it fails in the engine. A result which does not fit is the error
//! `request` of the engine, which allocates the result with `try_reserve`, and not an abort of
//! its allocator. Every `memory.grow` (and `table.grow`) is a call into the host for wasmtime's
//! call hook; the limiter counts them, and [`Instance::wasi_calls`] leaves them out.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Weak};

use anyhow::{Context, anyhow, bail, ensure};
pub use sbmlsim_abi::EngineError;
use sbmlsim_abi::{
    ENGINE_OUTPUT_FORMAT, ENGINE_SECTION, MODEL_CONTRACT, ModelDescription, SimulationRequest,
    SimulationResult, SimulationStats, check_contract,
};
use serde::Deserialize;
use wasmtime::wasmparser::{Parser, Payload};
use wasmtime::{
    CallHook, Config, Engine, ExternType, FuncType, Linker, Memory, MemoryType, Module,
    ResourceLimiter, Store, TypedFunc,
};

mod cache;
mod execution;
#[cfg(feature = "native")]
mod native;
mod wasi;

pub use cache::ModuleCache;
pub use execution::DEFAULT_EXECUTION_TIMEOUT;
use execution::Execution;
#[cfg(feature = "native")]
pub use native::NativeInstance;

/// `engine.wasm` of this build (`sbmlsim-engine` for `wasm32-wasip1`).
pub static ENGINE_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/engine.wasm"));

/// `mathlib.wasm` of this build (`sbmlsim-mathlib` for `wasm32-unknown-unknown`).
pub static MATHLIB_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mathlib.wasm"));

/// The notices of the libraries [`ENGINE_WASM`] links statically (SUNDIALS, BSD-3-Clause; KLU
/// and BTF, LGPL-2.1-or-later; AMD, COLAMD and SuiteSparse_config, BSD-3-Clause) with their
/// versions, the commits of their submodules and their license texts: `THIRD_PARTY.md` of the
/// repository, which `sbmlsim-core runtime -o` writes next to the modules.
pub static THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD_PARTY.md");

/// The module of the WASI preview1 imports of the engine.
const WASI_MODULE: &str = "wasi_snapshot_preview1";

/// The exports of `engine.wasm`.
const ENGINE_EXPORTS: [&str; 7] = [
    "engine_alloc",
    "engine_free",
    "engine_load",
    "engine_simulate",
    "engine_output_ptr",
    "engine_output_len",
    "engine_stats_ptr",
];

/// The most bytes the memory of an instance grows to: 2 GiB, `32768` pages of 64 KiB, the
/// `maximum` the JavaScript loader gives its memory (`js/loader.mjs`). The largest result
/// (64 million values, 512 MiB) is recorded in place of its output, so it
/// fits, also while the steps of the integrator grow it by doubling.
pub const MAX_MEMORY_BYTES: usize = 2 << 30;

/// Maximum elements in a runtime table. Compiled model modules may not define tables.
pub const MAX_TABLE_ELEMENTS: usize = 65_536;

/// The data of a store: the WASI context of the engine, the number of its calls into the host
/// and the limit of its memory.
struct HostState {
    wasi: wasi::Output,
    host_calls: u64,
    memory: MemoryLimit,
}

/// The limit of the memory of a store, which counts the growths of the memory and of the tables
/// (each a call into the host for the call hook).
struct MemoryLimit {
    max_bytes: usize,
    grows: u64,
}

impl ResourceLimiter for MemoryLimit {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.grows += 1;
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // a `table.grow` is a call into the host for the call hook as a `memory.grow` is
        self.grows += 1;
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }
}

impl HostState {
    /// The calls into the host but the growths of the memory and the tables: the WASI calls.
    fn wasi_calls(&self) -> u64 {
        self.host_calls - self.memory.grows
    }
}

/// A store whose call hook counts the calls of wasm into the host, with a memory of at most
/// `max_memory` bytes.
fn store(engine: &Engine, max_memory: usize) -> Store<HostState> {
    let mut store = Store::new(
        engine,
        HostState {
            wasi: wasi::Output::default(),
            host_calls: 0,
            memory: MemoryLimit {
                max_bytes: max_memory,
                grows: 0,
            },
        },
    );
    store.limiter(|state| &mut state.memory);
    store.call_hook(|mut state, hook| {
        if matches!(hook, CallHook::CallingHost) {
            state.data_mut().host_calls += 1;
        }
        Ok(())
    });
    store
}

/// A linker without WASI, with the native model's resource limiter.
#[cfg(feature = "native")]
fn linker_without_wasi(engine: &Engine) -> Linker<MemoryLimit> {
    Linker::new(engine)
}

/// A linker with the minimal WASI preview1 adapter.
fn linker(engine: &Engine) -> anyhow::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    wasi::add_to_linker(&mut linker)?;
    Ok(linker)
}

/// The number of calls into the host (the WASI calls) of one call of the export `export`
/// (without parameters and with one `i32` result) of a module which imports only WASI
/// preview1, counted as an [`Instance`] counts the calls of the engine.
pub fn count_wasi_calls(wasm: &[u8], export: &str) -> anyhow::Result<u64> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm)?;
    let mut store = store(&engine, MAX_MEMORY_BYTES);
    let mut linker = linker(&engine)?;
    wasi::reject_other_imports(&mut linker, &module)?;
    let instance = linker.instantiate(&mut store, &module)?;
    store.data_mut().memory.grows = 0;
    let function = instance.get_typed_func::<(), i32>(&mut store, export)?;
    let before = store.data().wasi_calls();
    function.call(&mut store, ())?;
    Ok(store.data().wasi_calls() - before)
}

/// `engine.wasm` and `mathlib.wasm`, compiled once or read from the persistent cache;
/// instantiates models.
pub struct Runtime {
    engine: Engine,
    engine_module: Module,
    mathlib_module: Module,
    memory_type: MemoryType,
    max_memory: usize,
    execution: Execution,
    cache: Option<wasmtime::Cache>,
}

impl Runtime {
    /// Compiles `engine.wasm` and `mathlib.wasm` and checks their imports and exports and the
    /// version of the output of the engine (its custom section [`ENGINE_SECTION`]). Uses
    /// the platform compilation cache when available, falling back to compilation on a miss.
    pub fn new(engine_wasm: &[u8], mathlib_wasm: &[u8]) -> anyhow::Result<Runtime> {
        Self::new_with_cache(engine_wasm, mathlib_wasm, ModuleCache::Default)
    }

    /// As [`Runtime::new`], with a chosen persistent compilation cache. Cache failures fall
    /// back to compilation. Wasmtime keys entries by the module bytes, compiler version,
    /// target CPU features and compilation settings; no instances or simulation state are
    /// shared. Cache hits still undergo the same module and description validation.
    pub fn new_with_cache(
        engine_wasm: &[u8],
        mathlib_wasm: &[u8],
        cache: ModuleCache,
    ) -> anyhow::Result<Runtime> {
        check_output_format(engine_wasm)?;
        let cache = cache.open();
        // These modules use core wasm32 and fixed-width SIMD. Reject features which
        // provide additional heaps, shared state or component resources outside our policy.
        let engine = Engine::new(
            Config::new()
                .epoch_interruption(true)
                .wasm_gc(false)
                .wasm_memory64(false)
                .wasm_multi_memory(false)
                .wasm_relaxed_simd(false)
                .cache(cache.clone()),
        )?;
        let execution =
            Execution::new(&engine).context("cannot start the execution deadline clock")?;
        let engine_module = Module::new(&engine, engine_wasm)
            .map_err(anyhow::Error::from)
            .context("engine.wasm does not compile")?;
        let mathlib_module = Module::new(&engine, mathlib_wasm)
            .map_err(anyhow::Error::from)
            .context("mathlib.wasm does not compile")?;
        let mut memory_type = None;
        for import in engine_module.imports() {
            match (import.module(), import.name(), import.ty()) {
                ("env", "memory", ExternType::Memory(ty)) => memory_type = Some(ty),
                ("model" | WASI_MODULE, _, ExternType::Func(_)) => {}
                (module, name, _) => bail!("engine.wasm imports {module}.{name}"),
            }
        }
        let memory_type = memory_type.ok_or_else(|| anyhow!("engine.wasm imports no memory"))?;
        for name in ENGINE_EXPORTS {
            ensure!(
                engine_module.get_export(name).is_some(),
                "engine.wasm does not export {name}"
            );
        }
        if let Some(import) = mathlib_module.imports().next() {
            bail!(
                "mathlib.wasm imports {}.{}, it imports nothing",
                import.module(),
                import.name()
            );
        }
        Ok(Runtime {
            cache,
            execution,
            engine,
            engine_module,
            mathlib_module,
            memory_type,
            max_memory: MAX_MEMORY_BYTES,
        })
    }

    /// The runtime with instances whose memory grows to at most `bytes` (instead of
    /// [`MAX_MEMORY_BYTES`]).
    pub fn with_max_memory(mut self, bytes: usize) -> Runtime {
        self.max_memory = bytes;
        self
    }

    /// Bounds each model initialization, simulation and direct function call. The default
    /// is 30 seconds; `None` disables the deadline for trusted workloads. A timed-out
    /// instance is discarded, just like any other trapped instance.
    pub fn with_execution_timeout(mut self, timeout: Option<std::time::Duration>) -> Runtime {
        self.execution.timeout = timeout;
        self
    }

    /// The runtime of the embedded [`ENGINE_WASM`] and [`MATHLIB_WASM`].
    pub fn embedded() -> anyhow::Result<Runtime> {
        Runtime::new(ENGINE_WASM, MATHLIB_WASM)
    }

    /// The embedded runtime with a chosen persistent compilation cache.
    pub fn embedded_with_cache(cache: ModuleCache) -> anyhow::Result<Runtime> {
        Runtime::new_with_cache(ENGINE_WASM, MATHLIB_WASM, cache)
    }

    /// The active cache directory, or `None` if disabled or unavailable. Cache entries may
    /// still miss or fail to be written (for example, when a disk becomes full).
    pub fn cache_directory(&self) -> Option<&Path> {
        self.cache.as_ref().map(|cache| cache.directory().as_path())
    }

    /// Compiles a model and checks it against its description, without instantiating it.
    ///
    /// Checks the contract of the description and the module of the model: it imports
    /// `env.memory` and otherwise only functions of mathlib (each of the type mathlib exports),
    /// and defines no memory, no mutable global, no data segment and no start function (a model
    /// holds no state and writes nothing into the memory of the engine but where it is told);
    /// compiles `model.wasm` once, so that every [`LoadedModel::instantiate`] reuses the module.
    pub fn load(&self, model_wasm: &[u8], desc: &ModelDescription) -> anyhow::Result<LoadedModel> {
        check_contract(&desc.contract, MODEL_CONTRACT)?;
        check_model_sections(model_wasm)?;
        let model_module = Module::new(&self.engine, model_wasm)
            .map_err(anyhow::Error::from)
            .context("model.wasm does not compile")?;
        let mut memory_imported = false;
        for import in model_module.imports() {
            match (import.module(), import.name(), import.ty()) {
                ("env", "memory", ExternType::Memory(_)) => memory_imported = true,
                ("math", name, ExternType::Func(ty)) => {
                    match self.mathlib_module.get_export(name) {
                        Some(ExternType::Func(math)) if FuncType::eq(&ty, &math) => {}
                        Some(ExternType::Func(math)) => bail!(
                            "model.wasm imports math.{name} as {ty}, mathlib exports it as {math}"
                        ),
                        _ => {
                            bail!("model.wasm imports math.{name}, which is no function of mathlib")
                        }
                    }
                }
                (module, name, _) => bail!(
                    "model.wasm imports {module}.{name}, which is neither env.memory nor a \
                     function of mathlib"
                ),
            }
        }
        ensure!(memory_imported, "model.wasm does not import env.memory");
        Ok(LoadedModel {
            identity: Arc::new(()),
            execution: self.execution.clone(),
            engine: self.engine.clone(),
            engine_module: self.engine_module.clone(),
            mathlib_module: self.mathlib_module.clone(),
            model_module,
            memory_type: self.memory_type.clone(),
            max_memory: self.max_memory,
            desc: desc.clone(),
        })
    }

    /// Instantiates a model with its description and loads it into a new engine (`load`, then
    /// [`LoadedModel::instantiate`]); gives the error of `engine_load` for a description the
    /// engine refuses.
    pub fn instantiate(
        &self,
        model_wasm: &[u8],
        desc: &ModelDescription,
    ) -> anyhow::Result<Instance> {
        self.load(model_wasm, desc)?.instantiate()
    }
}

/// What [`link_model`] makes: the store, the memory the model imports, the linker which has
/// `env.memory` and `math`, and the instance of the model (not yet registered as `model`).
struct Linked {
    store: Store<HostState>,
    memory: Memory,
    linker: Linker<HostState>,
    model: wasmtime::Instance,
}

/// Instantiates mathlib and the model in a new store with a memory of `memory_type`: defines
/// `env.memory`, instantiates mathlib as `math` and the model against them.
fn link_model(
    engine: &Engine,
    mathlib_module: &Module,
    model_module: &Module,
    memory_type: &MemoryType,
    max_memory: usize,
    execution: &Execution,
) -> anyhow::Result<Linked> {
    let mut store = store(engine, max_memory);
    execution.begin(&mut store);
    let memory = Memory::new(&mut store, memory_type.clone())?;
    let mut linker = linker(engine)?;
    linker.define(&store, "env", "memory", memory)?;
    let math = linker.instantiate(&mut store, mathlib_module)?;
    linker.instance(&mut store, "math", math)?;
    let model = linker
        .instantiate(&mut store, model_module)
        .map_err(anyhow::Error::from)
        .context("model.wasm does not instantiate")?;
    Ok(Linked {
        store,
        memory,
        linker,
        model,
    })
}

/// A model compiled once ([`Runtime::load`]), which makes any number of independent
/// [`Instance`]s; it is `Send + Sync`, so threads share it and each makes its own instances.
pub struct LoadedModel {
    identity: Arc<()>,
    engine: Engine,
    engine_module: Module,
    mathlib_module: Module,
    model_module: Module,
    memory_type: MemoryType,
    max_memory: usize,
    desc: ModelDescription,
    execution: Execution,
}

impl LoadedModel {
    /// The description the model was loaded with.
    pub fn description(&self) -> &ModelDescription {
        &self.desc
    }

    /// The wasmtime engine the modules of this model were compiled with, for
    /// [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn wasmtime_engine(&self) -> &Engine {
        &self.engine
    }

    /// `mathlib.wasm` of this model, for [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn mathlib(&self) -> &Module {
        &self.mathlib_module
    }

    /// `model.wasm` of this model, compiled once, for [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn model_module(&self) -> &Module {
        &self.model_module
    }

    /// The memory an instance of this model may use, for [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn max_memory(&self) -> usize {
        self.max_memory
    }

    /// The execution limits of this model, for [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn execution(&self) -> &Execution {
        &self.execution
    }

    /// What a prepared request of this model is checked against, for
    /// [`LoadedModel::instantiate_native`].
    #[cfg(feature = "native")]
    pub(crate) fn identity(&self) -> &Arc<()> {
        &self.identity
    }

    /// Instantiates the model in a new engine, which holds the description; gives the error of
    /// `engine_load` for a description the engine refuses.
    pub fn instantiate(&self) -> anyhow::Result<Instance> {
        let desc = &self.desc;
        let model_module = &self.model_module;
        let Linked {
            mut store,
            memory,
            mut linker,
            model,
        } = link_model(
            &self.engine,
            &self.mathlib_module,
            model_module,
            &self.memory_type,
            self.max_memory,
            &self.execution,
        )?;
        linker.instance(&mut store, "model", model)?;
        wasi::reject_other_imports(&mut linker, &self.engine_module)?;
        let engine = linker
            .instantiate(&mut store, &self.engine_module)
            .map_err(anyhow::Error::from)
            .context("engine.wasm does not instantiate with the model")?;
        // the limiter saw the memories the instantiation created, which are no calls into
        // the host
        store.data_mut().memory.grows = 0;
        let before = store.data().wasi_calls();
        if let Ok(initialize) = engine.get_typed_func::<(), ()>(&mut store, "_initialize") {
            initialize.call(&mut store, ())?;
        }
        let initialize_calls = store.data().wasi_calls() - before;
        let mut instance = Instance {
            identity: self.identity.clone(),
            prepared: BTreeMap::new(),
            prepare_fn: engine.get_typed_func(&mut store, "engine_prepare").ok(),
            simulate_prepared_fn: engine
                .get_typed_func(&mut store, "engine_simulate_prepared")
                .ok(),
            release_prepared_fn: engine
                .get_typed_func(&mut store, "engine_release_prepared")
                .ok(),
            input_ptr: 0,
            input_capacity: 0,
            input_bytes: Vec::new(),
            execution: self.execution.clone(),
            alloc: engine.get_typed_func(&mut store, "engine_alloc")?,
            free: engine.get_typed_func(&mut store, "engine_free")?,
            load: engine.get_typed_func(&mut store, "engine_load")?,
            simulate: engine.get_typed_func(&mut store, "engine_simulate")?,
            initial_values: if desc.n_algebraic == 0 {
                None
            } else {
                engine
                    .get_typed_func(&mut store, "engine_initial_values")
                    .ok()
            },
            output_ptr: engine.get_typed_func(&mut store, "engine_output_ptr")?,
            output_len: engine.get_typed_func(&mut store, "engine_output_len")?,
            stats_ptr: engine.get_typed_func(&mut store, "engine_stats_ptr")?,
            store,
            memory,
            trapped: false,
            initialize_calls,
            load_calls: 0,
        };
        let before = instance.wasi_calls();
        let input = serde_json::to_vec(desc)?;
        let code = instance.call(instance.load.clone(), &input)?;
        instance.load_calls = instance.wasi_calls() - before;
        if code != 0 {
            let message = match instance.output()? {
                Output::Error(error) if error.kind == "load" => error.message,
                other => bail!("engine_load failed without an error of the kind load: {other:?}"),
            };
            bail!(
                "the engine refuses the description of model {}: {message}",
                desc.id
            );
        }
        Ok(instance)
    }
}

impl LoadedModel {
    /// The value of every selection of the description, in its order, for the model as loaded:
    /// `model_defaults`, `model_init` at the time 0 with no fixed entity, `model_assign`, then
    /// `model_selections` with every selection masked in. It links mathlib and the model in a
    /// store of its own. For a DAE model the engine solves consistent initial conditions
    /// before evaluating selections, without executing events. It refuses a model whose
    /// `model_sizes` differ from the description.
    pub fn initial_selections(&self) -> anyhow::Result<Vec<f64>> {
        let desc = &self.desc;
        if desc.n_algebraic > 0 {
            return Ok(self.instantiate()?.initial_selections()?);
        }
        let (np, ns, nsel) = (
            desc.parameters.len(),
            desc.states.len(),
            desc.selections.len(),
        );
        let Linked {
            mut store,
            memory,
            model,
            ..
        } = link_model(
            &self.engine,
            &self.mathlib_module,
            &self.model_module,
            &self.memory_type,
            self.max_memory,
            &self.execution,
        )?;
        let defaults = model.get_typed_func::<(i32, i32), ()>(&mut store, sbmlsim_abi::DEFAULTS)?;
        let init =
            model.get_typed_func::<(f64, i32, i32, i32), ()>(&mut store, sbmlsim_abi::INIT)?;
        let assign =
            model.get_typed_func::<(f64, i32, i32), ()>(&mut store, sbmlsim_abi::ASSIGN)?;
        let selections = model
            .get_typed_func::<(f64, i32, i32, i32, i32), ()>(&mut store, sbmlsim_abi::SELECTIONS)?;
        let sizes = model.get_typed_func::<i32, ()>(&mut store, sbmlsim_abi::SIZES)?;
        // the memory: p (the parameters, then the constants), y, the selections, the words of
        // the sizes, then the bytes of fixed and of the mask
        let p_at = 0usize;
        let y_at = p_at + 8 * (np + desc.n_constants as usize);
        let out_at = y_at + 8 * ns;
        let words_at = out_at + 8 * nsel;
        let fixed_at = words_at + 4 * sbmlsim_abi::ModelSizes::WORDS;
        let mask_at = fixed_at + np + ns;
        let end = mask_at + nsel;
        let size = memory.data_size(&store);
        if end > size {
            memory.grow(&mut store, (end - size).div_ceil(65536) as u64)?;
        }
        let address = |a: usize| i32::try_from(a).context("the vectors do not fit the memory");
        sizes.call(&mut store, address(words_at)?)?;
        let mut words = [0u8; 4 * sbmlsim_abi::ModelSizes::WORDS];
        memory.read(&store, words_at, &mut words)?;
        let read: Vec<u32> = words
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect();
        let expected = [
            ns as u32,
            np as u32,
            nsel as u32,
            desc.jacobian.pattern().row_idx.len() as u32,
            desc.n_roots,
            desc.n_constants,
            desc.events.len() as u32,
        ];
        ensure!(
            read == expected,
            "model.wasm has the sizes {read:?} (states, parameters, selections, Jacobian entries, \
             roots, constants, events), the description {expected:?}"
        );
        // the zeros of a fresh memory: no entity is fixed, every selection is in the mask
        memory.write(&mut store, fixed_at, &vec![0u8; np + ns])?;
        memory.write(&mut store, mask_at, &vec![1u8; nsel])?;
        let (p, y) = (address(p_at)?, address(y_at)?);
        defaults.call(&mut store, (p, y))?;
        init.call(&mut store, (0.0, p, y, address(fixed_at)?))?;
        assign.call(&mut store, (0.0, y, p))?;
        selections.call(&mut store, (0.0, y, p, address(mask_at)?, address(out_at)?))?;
        let mut out = vec![0u8; 8 * nsel];
        memory.read(&store, out_at, &mut out)?;
        Ok(out
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| f64::from_le_bytes(*b))
            .collect())
    }
}

impl Runtime {
    /// Instantiates a model with mathlib alone, without the engine, to call its right hand side
    /// and its Jacobian directly (tests and diagnostics); `desc` gives the sizes.
    pub fn model_functions(
        &self,
        model_wasm: &[u8],
        desc: &ModelDescription,
    ) -> anyhow::Result<ModelFunctions> {
        let module = self.load(model_wasm, desc)?.model_module;
        let Linked {
            mut store,
            memory,
            model,
            ..
        } = link_model(
            &self.engine,
            &self.mathlib_module,
            &module,
            &self.memory_type,
            self.max_memory,
            &self.execution,
        )?;
        let rhs = model.get_typed_func(&mut store, sbmlsim_abi::RHS)?;
        let jac = model.get_typed_func(&mut store, sbmlsim_abi::JAC)?;
        let assign = model.get_typed_func(&mut store, sbmlsim_abi::ASSIGN)?;
        Ok(ModelFunctions {
            trapped: false,
            execution: self.execution.clone(),
            store,
            memory,
            rhs,
            jac,
            assign,
            n: desc.states.len(),
            n_params: desc.parameters.len(),
            n_constants: desc.n_constants as usize,
            nnz: desc.jacobian.pattern().nnz(),
        })
    }
}

/// The right hand side and the Jacobian of a model instantiated without the engine
/// ([`Runtime::model_functions`]).
pub struct ModelFunctions {
    trapped: bool,
    execution: Execution,
    store: Store<HostState>,
    memory: Memory,
    rhs: TypedFunc<(f64, i32, i32, i32), ()>,
    jac: TypedFunc<(f64, i32, i32, i32), ()>,
    assign: TypedFunc<(f64, i32, i32), ()>,
    n: usize,
    n_params: usize,
    n_constants: usize,
    nnz: usize,
}

impl ModelFunctions {
    /// The right hand side at `(t, y)` with the parameters `p`.
    pub fn rhs(&mut self, t: f64, y: &[f64], p: &[f64]) -> anyhow::Result<Vec<f64>> {
        let function = self.rhs.clone();
        self.call(function, t, y, p, self.n)
    }

    /// The entries of the Jacobian at `(t, y)` with the parameters `p`, in the CSC order of the
    /// pattern of the description (`model_jac` traps for a model without an analytic one).
    pub fn jac(&mut self, t: f64, y: &[f64], p: &[f64]) -> anyhow::Result<Vec<f64>> {
        let function = self.jac.clone();
        self.call(function, t, y, p, self.nnz)
    }

    /// Calls `function(t, y, p, out)` with the vectors in the memory, `y` first, after
    /// `model_assign(t, y, p)` on them, which computes the constants of the model behind the
    /// parameters (and the targets of the assignment rules, which the functions do not read).
    fn call(
        &mut self,
        function: TypedFunc<(f64, i32, i32, i32), ()>,
        t: f64,
        y: &[f64],
        p: &[f64],
        n_out: usize,
    ) -> anyhow::Result<Vec<f64>> {
        ensure!(
            !self.trapped,
            "the model instance trapped before and is not used again"
        );
        ensure!(
            y.len() == self.n && p.len() == self.n_params,
            "{} states and {} parameters for a model of {} and {}",
            y.len(),
            p.len(),
            self.n,
            self.n_params
        );
        self.execution.begin(&mut self.store);
        let result = (|| {
            let bytes = |v: &[f64]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
            let (y_at, p_at) = (0usize, 8 * y.len());
            let out_at = p_at + 8 * (p.len() + self.n_constants);
            let end = out_at + 8 * n_out;
            let size = self.memory.data_size(&self.store);
            if end > size {
                self.memory
                    .grow(&mut self.store, (end - size).div_ceil(65536) as u64)?;
            }
            self.memory.write(&mut self.store, y_at, &bytes(y))?;
            self.memory.write(&mut self.store, p_at, &bytes(p))?;
            let address = |a: usize| i32::try_from(a).context("the vectors do not fit the memory");
            self.assign
                .call(&mut self.store, (t, address(y_at)?, address(p_at)?))?;
            function.call(
                &mut self.store,
                (t, address(y_at)?, address(p_at)?, address(out_at)?),
            )?;
            let mut out = vec![0u8; 8 * n_out];
            self.memory.read(&self.store, out_at, &mut out)?;
            Ok(out
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_le_bytes(*b))
                .collect())
        })();
        if result.is_err() {
            self.trapped = true;
        }
        result
    }
}

/// Checks that a model defines no memory, no mutable global, no data segment and no start
/// function.
fn check_model_sections(model_wasm: &[u8]) -> anyhow::Result<()> {
    for payload in Parser::new(0).parse_all(model_wasm) {
        match payload.context("model.wasm is not a WebAssembly module")? {
            Payload::DataSection(section) if section.count() > 0 => {
                bail!("model.wasm has a data segment, it has no memory content of its own")
            }
            Payload::StartSection { .. } => bail!("model.wasm has a start function"),
            Payload::MemorySection(section) if section.count() > 0 => {
                bail!("model.wasm defines a memory, it imports env.memory")
            }
            Payload::TableSection(section) if section.count() > 0 => {
                bail!("model.wasm defines a table, a model holds no state")
            }
            Payload::ElementSection(section) if section.count() > 0 => {
                bail!("model.wasm has an element segment, a model holds no state")
            }
            Payload::GlobalSection(section) => {
                for global in section {
                    let global = global.context("model.wasm is not a WebAssembly module")?;
                    ensure!(
                        !global.ty.mutable,
                        "model.wasm defines a mutable global, a model holds no state"
                    );
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The custom section [`ENGINE_SECTION`] of `engine.wasm`.
#[derive(Debug, Deserialize)]
struct EngineSection {
    /// The version of the output of the engine.
    output_format: Option<u32>,
}

/// Checks that `engine.wasm` has one custom section [`ENGINE_SECTION`] whose output format is
/// [`ENGINE_OUTPUT_FORMAT`], the layout of the output this host reads.
fn check_output_format(engine_wasm: &[u8]) -> anyhow::Result<()> {
    let mut sections = Vec::new();
    for payload in Parser::new(0).parse_all(engine_wasm) {
        if let Payload::CustomSection(section) =
            payload.context("engine.wasm is not a WebAssembly module")?
            && section.name() == ENGINE_SECTION
        {
            sections.push(section.data().to_vec());
        }
    }
    let [data] = sections.as_slice() else {
        bail!(
            "engine.wasm has {} custom sections {ENGINE_SECTION}, one with the version of its \
             output is expected (the build script of sbmlsim-host adds it)",
            sections.len()
        );
    };
    let section: EngineSection = serde_json::from_slice(data)
        .with_context(|| format!("the custom section {ENGINE_SECTION} of engine.wasm"))?;
    match section.output_format {
        Some(ENGINE_OUTPUT_FORMAT) => Ok(()),
        Some(other) => bail!(
            "engine.wasm writes the output format {other}, this host reads \
             {ENGINE_OUTPUT_FORMAT}"
        ),
        None => bail!(
            "engine.wasm declares no output format in its section {ENGINE_SECTION}, this host \
             reads {ENGINE_OUTPUT_FORMAT}"
        ),
    }
}

/// The error of an output of the engine.
#[derive(Debug, Deserialize)]
struct ErrorField {
    kind: String,
    message: String,
    #[serde(default)]
    flag: Option<i32>,
    /// `null` for a time which is not finite.
    #[serde(default)]
    time: Option<f64>,
}

/// The header of an output of the engine.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Header {
    Prepared { prepared: u32, n_values: usize },
    Error { error: ErrorField },
    Table { columns: Vec<String>, n_rows: usize },
}

/// An output of the engine.
#[derive(Debug)]
enum Output {
    Prepared { id: u32, n_values: usize },
    Error(ErrorField),
    Table(SimulationResult),
}

/// An internal error.
fn internal(message: impl Into<String>) -> EngineError {
    EngineError::Internal(message.into())
}

/// Reads an output of the engine (see `crates/engine/src/exports.rs`).
fn read_output(bytes: &[u8]) -> Result<Output, EngineError> {
    let length = bytes
        .get(..4)
        .ok_or_else(|| internal("the output of the engine is empty"))?;
    let length = u32::from_le_bytes(length.try_into().unwrap()) as usize;
    let header = bytes
        .get(4..4 + length)
        .ok_or_else(|| internal("the header of the output of the engine is cut off"))?;
    let header: Header = serde_json::from_slice(header)
        .map_err(|e| internal(format!("the header of the output of the engine: {e}")))?;
    let start = (4 + length).next_multiple_of(8);
    match header {
        Header::Prepared { prepared, n_values } => Ok(Output::Prepared {
            id: prepared,
            n_values,
        }),
        Header::Error { error } => Ok(Output::Error(error)),
        Header::Table { columns, n_rows } => {
            let n = n_rows
                .checked_mul(columns.len())
                .ok_or_else(|| internal("the table of the engine is too large"))?;
            let data = bytes
                .get(start..)
                .filter(|data| Some(data.len()) == n.checked_mul(8))
                .ok_or_else(|| {
                    internal(format!(
                        "the output of the engine does not hold {n_rows} rows of {} columns",
                        columns.len()
                    ))
                })?;
            let values = data
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_le_bytes(*b))
                .collect();
            Ok(Output::Table(SimulationResult {
                columns,
                n_rows,
                values,
            }))
        }
    }
}

/// The [`EngineError`] of an error of the output.
fn engine_error(error: ErrorField) -> EngineError {
    match error.kind.as_str() {
        "request" => EngineError::Request(error.message),
        "unsupported" => EngineError::Unsupported(error.message),
        "integration" => match error.flag {
            Some(flag) => EngineError::Integration {
                flag,
                time: error.time.unwrap_or(f64::NAN),
                message: error.message,
            },
            None => internal("an integration error of the engine without a flag"),
        },
        "panic" => internal(format!("engine.wasm panicked: {}", error.message)),
        kind => internal(format!(
            "an error of the unknown kind {kind}: {}",
            error.message
        )),
    }
}

/// An immutable validated request that can be shared by instances of one LoadedModel.
#[derive(Clone)]
pub struct PreparedSimulation {
    inner: Arc<PreparedInner>,
}

pub(crate) struct PreparedInner {
    model: Arc<()>,
    input: Vec<u8>,
    n_values: usize,
    /// The request and its varying targets as given, which the native engine resolves into a
    /// plan of its own ([`crate::NativeInstance::simulate_prepared`]); `input` is the same two
    /// as the JSON `engine.wasm` parses.
    #[cfg(feature = "native")]
    request: SimulationRequest,
    #[cfg(feature = "native")]
    varying: Vec<String>,
}

impl std::fmt::Debug for PreparedSimulation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSimulation")
            .field("n_values", &self.n_values())
            .finish_non_exhaustive()
    }
}

impl PreparedSimulation {
    /// Number of values supplied per point, in the order of the varying targets.
    pub fn n_values(&self) -> usize {
        self.inner.n_values
    }
}

struct CachedPrepared {
    owner: Weak<PreparedInner>,
    id: u32,
}

#[derive(serde::Serialize)]
struct PrepareInput<'a> {
    request: &'a SimulationRequest,
    varying: &'a [String],
}

#[cfg(feature = "native")]
impl PreparedSimulation {
    /// The loaded model this request was prepared for.
    pub(crate) fn model(&self) -> &Arc<()> {
        &self.inner.model
    }

    /// The request as given, see [`PreparedInner`].
    pub(crate) fn request(&self) -> &SimulationRequest {
        &self.inner.request
    }

    /// The varying targets as given, see [`PreparedInner`].
    pub(crate) fn varying(&self) -> &[String] {
        &self.inner.varying
    }

    /// The key a native instance caches the plan of this request under, the address of its
    /// shared part, with a weak handle which says whether the request is still alive.
    pub(crate) fn cache_entry(&self) -> (usize, Weak<PreparedInner>) {
        (
            Arc::as_ptr(&self.inner) as usize,
            Arc::downgrade(&self.inner),
        )
    }
}

impl LoadedModel {
    /// Validates a reusable request once. Each worker lazily resolves its own guest plan;
    /// subsequent points transfer only a numeric vector. Plans are bound to this loaded model.
    pub fn prepare(
        &self,
        request: &SimulationRequest,
        varying: &[String],
    ) -> anyhow::Result<PreparedSimulation> {
        Ok(self.instantiate()?.prepare(request, varying)?)
    }
}

/// A model loaded into an engine, in a store of its own; one per model and thread.
pub struct Instance {
    identity: Arc<()>,
    prepared: BTreeMap<usize, CachedPrepared>,
    prepare_fn: Option<TypedFunc<(i32, i32), i32>>,
    simulate_prepared_fn: Option<TypedFunc<(i32, i32, i32), i32>>,
    release_prepared_fn: Option<TypedFunc<i32, ()>>,
    input_ptr: i32,
    input_capacity: usize,
    input_bytes: Vec<u8>,
    execution: Execution,
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    load: TypedFunc<(i32, i32), i32>,
    simulate: TypedFunc<(i32, i32), i32>,
    initial_values: Option<TypedFunc<(), i32>>,
    output_ptr: TypedFunc<(), i32>,
    output_len: TypedFunc<(), i32>,
    stats_ptr: TypedFunc<(), i32>,
    trapped: bool,
    initialize_calls: u64,
    load_calls: u64,
}

impl Instance {
    /// Reads initialized selections without applying events, including DAE constraints.
    fn initial_selections(&mut self) -> Result<Vec<f64>, EngineError> {
        let function = self
            .initial_values
            .clone()
            .ok_or_else(|| internal("the engine does not support consistent initial selections"))?;
        self.execution.begin(&mut self.store);
        let code = self.guard(|this| function.call(&mut this.store, ()))?;
        match (code, self.output()?) {
            (0, Output::Table(result)) if result.n_rows == 1 => Ok(result.values),
            (_, Output::Error(error)) => Err(engine_error(error)),
            (code, output) => Err(internal(format!(
                "engine_initial_values returned {code} with the output {output:?}"
            ))),
        }
    }

    /// Simulates a request; the result does not depend on the simulations before.
    ///
    /// The errors are the ones of the native engine, and [`EngineError::Internal`] for a trap
    /// (a panic of the engine, a trap of the model) or a failure of the host; an instance which
    /// trapped refuses every later request with [`EngineError::Internal`].
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        let mut input = std::mem::take(&mut self.input_bytes);
        input.clear();
        let encoded = serde_json::to_writer(&mut input, req)
            .map_err(|e| internal(format!("the request does not serialize: {e}")));
        let code = encoded.and_then(|()| self.call(self.simulate.clone(), &input));
        self.input_bytes = input;
        let code = code?;
        match (code, self.output()?) {
            (0, Output::Table(result)) => Ok(result),
            (1, Output::Error(error)) => Err(engine_error(error)),
            (code, output) => Err(internal(format!(
                "engine_simulate returned {code} with the output {output:?}"
            ))),
        }
    }

    /// The counters of the integrator of the last simulation (`engine_stats_ptr`).
    pub fn stats(&mut self) -> Result<SimulationStats, EngineError> {
        if self.trapped {
            return Err(internal(
                "the instance trapped before and is not used again",
            ));
        }
        self.execution.begin(&mut self.store);
        let ptr = self.guard(|this| this.stats_ptr.call(&mut this.store, ()))? as u32 as usize;
        let mut bytes = [0u8; 8 * SimulationStats::WORDS];
        self.memory
            .read(&self.store, ptr, &mut bytes)
            .map_err(|e| internal(format!("the counters are outside of the memory: {e}")))?;
        let mut words = [0u64; SimulationStats::WORDS];
        for (word, chunk) in words.iter_mut().zip(bytes.as_chunks::<8>().0) {
            *word = u64::from_le_bytes(*chunk);
        }
        Ok(SimulationStats::from_words(words))
    }

    /// The number of calls of the engine into the host (all of them WASI calls) since the
    /// instance was created, `_initialize` and `engine_load` included.
    pub fn wasi_calls(&self) -> u64 {
        self.store.data().wasi_calls()
    }

    /// The size of the memory of the instance in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    /// The address of the output of the last call in the memory, aligned to 8
    /// (`crates/engine/src/exports.rs`).
    pub fn output_address(&mut self) -> Result<usize, EngineError> {
        self.execution.begin(&mut self.store);
        Ok(self.guard(Self::output_range)?.start)
    }

    /// The number of WASI calls of `_initialize`.
    pub fn initialize_wasi_calls(&self) -> u64 {
        self.initialize_calls
    }

    /// The number of WASI calls of `engine_load`.
    pub fn load_wasi_calls(&self) -> u64 {
        self.load_calls
    }

    /// Calls `engine_load` or `engine_simulate` with `input` in a buffer of the engine.
    fn call(
        &mut self,
        function: TypedFunc<(i32, i32), i32>,
        input: &[u8],
    ) -> Result<i32, EngineError> {
        if self.trapped {
            return Err(internal(
                "the instance trapped before and is not used again",
            ));
        }
        self.execution.begin(&mut self.store);
        let len =
            i32::try_from(input.len()).map_err(|_| internal("the input is larger than 2 GiB"))?;
        let ptr = self.input_buffer(input.len())?;
        self.memory
            .write(&mut self.store, ptr as u32 as usize, input)
            .map_err(|e| internal(format!("the input does not fit into the memory: {e}")))?;
        self.guard(|this| function.call(&mut this.store, (ptr, len)))
    }

    /// Validates and resolves a plan in this instance and returns a shareable model-bound handle.
    pub fn prepare(
        &mut self,
        request: &SimulationRequest,
        varying: &[String],
    ) -> Result<PreparedSimulation, EngineError> {
        self.clean_prepared()?;
        let input = serde_json::to_vec(&PrepareInput { request, varying })
            .map_err(|error| internal(format!("prepared request serialization failed: {error}")))?;
        if input.len() > 16 * 1024 * 1024 {
            return Err(EngineError::Request(
                "the prepared request exceeds 16 MiB".into(),
            ));
        }
        let (id, n_values) = self.prepare_guest(&input)?;
        let prepared = PreparedSimulation {
            inner: Arc::new(PreparedInner {
                model: self.identity.clone(),
                input,
                n_values,
                #[cfg(feature = "native")]
                request: request.clone(),
                #[cfg(feature = "native")]
                varying: varying.to_vec(),
            }),
        };
        self.prepared.insert(
            Arc::as_ptr(&prepared.inner) as usize,
            CachedPrepared {
                owner: Arc::downgrade(&prepared.inner),
                id,
            },
        );
        Ok(prepared)
    }

    /// Executes a prepared plan with one value per varying target, avoiding JSON and name lookups.
    pub fn simulate_prepared(
        &mut self,
        prepared: &PreparedSimulation,
        values: &[f64],
    ) -> Result<SimulationResult, EngineError> {
        if !Arc::ptr_eq(&self.identity, &prepared.inner.model) {
            return Err(EngineError::Request(
                "prepared request belongs to a different loaded model".into(),
            ));
        }
        if values.len() != prepared.n_values() || values.iter().any(|value| !value.is_finite()) {
            return Err(EngineError::Request(format!(
                "expected {} finite prepared values, received {}",
                prepared.n_values(),
                values.len()
            )));
        }
        let function = self.simulate_prepared_fn.clone().ok_or_else(|| {
            EngineError::Unsupported("engine.wasm does not support prepared requests".into())
        })?;
        self.clean_prepared()?;
        let key = Arc::as_ptr(&prepared.inner) as usize;
        let id = match self.prepared.get(&key) {
            Some(cached) => cached.id,
            None => {
                let (id, n_values) = self.prepare_guest(&prepared.inner.input)?;
                if n_values != prepared.n_values() {
                    return Err(internal("prepared value count changed between instances"));
                }
                self.prepared.insert(
                    key,
                    CachedPrepared {
                        owner: Arc::downgrade(&prepared.inner),
                        id,
                    },
                );
                id
            }
        };
        if self.trapped {
            return Err(internal(
                "the instance trapped before and is not used again",
            ));
        }
        self.execution.begin(&mut self.store);
        let length = values
            .len()
            .checked_mul(8)
            .filter(|n| *n <= i32::MAX as usize)
            .ok_or_else(|| EngineError::Request("prepared values exceed the input limit".into()))?;
        let ptr = self.input_buffer(length)?;
        // Write directly into the aligned guest buffer; no temporary byte vector per point.
        let data = self.memory.data_mut(&mut self.store);
        let start = ptr as u32 as usize;
        let destination = data
            .get_mut(start..start + length)
            .ok_or_else(|| internal("prepared input is outside memory"))?;
        for (bytes, value) in destination.as_chunks_mut::<8>().0.iter_mut().zip(values) {
            *bytes = value.to_le_bytes();
        }
        let code = self
            .guard(|this| function.call(&mut this.store, (id as i32, ptr, values.len() as i32)))?;
        match (code, self.output()?) {
            (0, Output::Table(result)) => Ok(result),
            (1, Output::Error(error)) => Err(engine_error(error)),
            (code, output) => Err(internal(format!(
                "prepared simulation returned {code} with {output:?}"
            ))),
        }
    }

    fn prepare_guest(&mut self, input: &[u8]) -> Result<(u32, usize), EngineError> {
        if self.simulate_prepared_fn.is_none() || self.release_prepared_fn.is_none() {
            return Err(EngineError::Unsupported(
                "engine.wasm does not support prepared requests".into(),
            ));
        }
        if self.prepared.len() >= sbmlsim_abi::MAX_PREPARED_REQUESTS {
            return Err(EngineError::Request(
                "the prepared request limit is reached; drop unused plans".into(),
            ));
        }
        let function = self.prepare_fn.clone().ok_or_else(|| {
            EngineError::Unsupported("engine.wasm does not support prepared requests".into())
        })?;
        let code = self.call(function, input)?;
        match (code, self.output()?) {
            (0, Output::Prepared { id, n_values }) => Ok((id, n_values)),
            (1, Output::Error(error)) => Err(engine_error(error)),
            (code, output) => Err(internal(format!(
                "preparation returned {code} with {output:?}"
            ))),
        }
    }

    fn clean_prepared(&mut self) -> Result<(), EngineError> {
        if self.trapped {
            return Err(internal(
                "the instance trapped before and is not used again",
            ));
        }
        let expired: Vec<_> = self
            .prepared
            .iter()
            .filter(|(_, entry)| entry.owner.strong_count() == 0)
            .map(|(&key, entry)| (key, entry.id))
            .collect();
        if !expired.is_empty() {
            self.execution.begin(&mut self.store);
            let function = self
                .release_prepared_fn
                .clone()
                .ok_or_else(|| internal("prepared release export is missing"))?;
            for (key, id) in expired {
                self.guard(|this| function.call(&mut this.store, id as i32))?;
                self.prepared.remove(&key);
            }
        }
        Ok(())
    }

    fn input_buffer(&mut self, length: usize) -> Result<i32, EngineError> {
        let length_i32 = i32::try_from(length)
            .map_err(|_| EngineError::Request("input exceeds 2 GiB".into()))?;
        if length <= self.input_capacity {
            return Ok(self.input_ptr);
        }
        if self.input_capacity > 0 {
            let (ptr, capacity) = (self.input_ptr, self.input_capacity as i32);
            self.guard(|this| this.free.call(&mut this.store, (ptr, capacity)))?;
            self.input_capacity = 0;
            self.input_ptr = 0;
        }
        let ptr = self.guard(|this| this.alloc.call(&mut this.store, length_i32))?;
        if ptr == 0 && length > 0 {
            return Err(internal("engine_alloc found no memory for the input"));
        }
        self.input_ptr = ptr;
        self.input_capacity = length;
        Ok(ptr)
    }

    /// Runs a call of the engine; a trap marks the instance and is an internal error, with the
    /// message of a panic of the engine when it wrote one.
    fn guard<R>(
        &mut self,
        call: impl FnOnce(&mut Self) -> wasmtime::Result<R>,
    ) -> Result<R, EngineError> {
        call(self).map_err(|error| {
            self.trapped = true;
            let panic = match self.output_bytes().map(|bytes| read_output(&bytes)) {
                Ok(Ok(Output::Error(e))) if e.kind == "panic" => format!(" ({})", e.message),
                _ => String::new(),
            };
            internal(format!("engine.wasm trapped: {error:?}{panic}"))
        })
    }

    /// The range of the output of the last call in the memory.
    fn output_range(&mut self) -> wasmtime::Result<std::ops::Range<usize>> {
        let ptr = self.output_ptr.call(&mut self.store, ())? as u32 as usize;
        let len = self.output_len.call(&mut self.store, ())? as u32 as usize;
        let size = self.memory.data_size(&self.store);
        match ptr.checked_add(len) {
            Some(end) if end <= size => Ok(ptr..end),
            _ => Err(wasmtime::format_err!("the output is outside of the memory")),
        }
    }

    /// The bytes of the output of the last call.
    fn output_bytes(&mut self) -> wasmtime::Result<Vec<u8>> {
        let range = self.output_range()?;
        Ok(self.memory.data(&self.store)[range].to_vec())
    }

    /// The output of the last call, read from the memory in place (the values are copied once,
    /// into the result).
    fn output(&mut self) -> Result<Output, EngineError> {
        let range = self.guard(Self::output_range)?;
        read_output(&self.memory.data(&self.store)[range])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An output of the format of `engine.wasm`.
    fn output(header: &str, values: &[f64]) -> Vec<u8> {
        let mut out = (header.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(header.as_bytes());
        out.resize((4 + header.len()).next_multiple_of(8), 0);
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_table_is_read() {
        let values = [0.0, 1.0, 1.0, 0.5, f64::NAN, -0.0];
        let bytes = output(r#"{"columns":["time","y"],"n_rows":3}"#, &values);
        match read_output(&bytes).unwrap() {
            Output::Table(result) => {
                assert_eq!(result.columns, ["time", "y"]);
                assert_eq!(result.n_rows, 3);
                let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&result.values), bits(&values));
            }
            other => panic!("{other:?}"),
        }
        // a value too few or too many
        for n in [5, 7] {
            let values = vec![1.0; n];
            let bytes = output(r#"{"columns":["time","y"],"n_rows":3}"#, &values);
            assert!(matches!(read_output(&bytes), Err(EngineError::Internal(_))));
        }
    }

    #[test]
    fn errors_are_read() {
        let error = |header: &str| match read_output(&output(header, &[])).unwrap() {
            Output::Error(error) => engine_error(error),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            error(r#"{"error":{"kind":"request","message":"no"}}"#),
            EngineError::Request("no".to_string())
        );
        assert_eq!(
            error(r#"{"error":{"kind":"unsupported","message":"events"}}"#),
            EngineError::Unsupported("events".to_string())
        );
        assert_eq!(
            error(r#"{"error":{"kind":"integration","message":"m","flag":-1,"time":0.1}}"#),
            EngineError::Integration {
                flag: -1,
                time: 0.1,
                message: "m".to_string()
            }
        );
        match error(r#"{"error":{"kind":"integration","message":"m","flag":-3,"time":null}}"#) {
            EngineError::Integration { flag: -3, time, .. } => assert!(time.is_nan()),
            other => panic!("{other:?}"),
        }
        match error(r#"{"error":{"kind":"panic","message":"index out of bounds"}}"#) {
            EngineError::Internal(message) => {
                assert!(message.contains("panicked: index out of bounds"))
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            error(r#"{"error":{"kind":"integration","message":"m"}}"#),
            EngineError::Internal(_)
        ));
    }

    #[test]
    fn broken_outputs_are_internal_errors() {
        for bytes in [vec![], vec![200, 0, 0, 0, b'{'], output("{}", &[])] {
            assert!(matches!(read_output(&bytes), Err(EngineError::Internal(_))));
        }
    }
}

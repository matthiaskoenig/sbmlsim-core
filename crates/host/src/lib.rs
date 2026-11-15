//! The wasmtime host of sbmlsim-core: links mathlib, a compiled model and the engine.
//!
//! A [`Runtime`] compiles `engine.wasm` and `mathlib.wasm` once (the embedded [`ENGINE_WASM`]
//! and [`MATHLIB_WASM`], or others); [`Runtime::instantiate`] instantiates, in a store of their
//! own, a memory of the type the engine imports, mathlib, the model (which imports that memory
//! as `env.memory` and the functions of mathlib as `math.*`) and the engine (which imports the
//! memory, the functions of the model as `model.*` and WASI preview1), so every call between
//! them is a direct call from wasm to wasm, and loads the description of the model. An
//! [`Instance`] simulates requests with it.
//!
//! The engine gets WASI preview1 without a file system, an environment or arguments, with
//! stdout and stderr inherited; it calls none of it while it loads or simulates
//! ([`Instance::wasi_calls`] counts the calls). The exports of `engine.wasm` and the format of
//! its output are documented in `crates/engine/src/exports.rs`.
//!
//! The memory of an instance grows to at most [`MAX_MEMORY_BYTES`] (a resource limiter of the
//! store, as `StoreLimits`; the JavaScript loader gives its memory the same `maximum`), so a
//! `memory.grow` beyond it fails in the engine. A result which does not fit is the error
//! `request` of the engine, which allocates the result with `try_reserve`, and not an abort of
//! its allocator. Every `memory.grow` is a call into the host for wasmtime's call hook; the
//! limiter counts them, and [`Instance::wasi_calls`] leaves them out.

use anyhow::{Context, anyhow, bail, ensure};
use sbmlsim_abi::{
    MODEL_CONTRACT, ModelDescription, SimulationRequest, SimulationResult, check_contract,
};
use sbmlsim_engine::EngineError;
use serde::Deserialize;
use wasmtime::wasmparser::{Parser, Payload};
use wasmtime::{
    CallHook, Config, Engine, ExternType, FuncType, Linker, Memory, MemoryType, Module,
    ResourceLimiter, Store, TypedFunc,
};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::{self, WasiP1Ctx};

/// `engine.wasm` of this build (`sbmlsim-engine` for `wasm32-wasip1`).
pub static ENGINE_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/engine.wasm"));

/// `mathlib.wasm` of this build (`sbmlsim-mathlib` for `wasm32-unknown-unknown`).
pub static MATHLIB_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mathlib.wasm"));

/// The module of the WASI preview1 imports of the engine.
const WASI_MODULE: &str = "wasi_snapshot_preview1";

/// The exports of `engine.wasm`.
const ENGINE_EXPORTS: [&str; 6] = [
    "engine_alloc",
    "engine_free",
    "engine_load",
    "engine_simulate",
    "engine_output_ptr",
    "engine_output_len",
];

/// The most bytes the memory of an instance grows to: 2 GiB, `32768` pages of 64 KiB, the
/// `maximum` the JavaScript loader gives its memory (`js/loader.mjs`). The largest result
/// ([`sbmlsim_engine::MAX_OUTPUT_VALUES`], 512 MiB) is recorded in place of its output, so it
/// fits, also while the steps of the integrator grow it by doubling.
pub const MAX_MEMORY_BYTES: usize = 2 << 30;

/// The data of a store: the WASI context of the engine, the number of its calls into the host
/// and the limit of its memory.
struct HostState {
    wasi: WasiP1Ctx,
    host_calls: u64,
    memory: MemoryLimit,
}

/// The limit of the memory of a store, which counts the growths of the memory (each a call
/// into the host for the call hook).
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
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

impl HostState {
    /// The calls into the host but the growths of the memory: the WASI calls.
    fn wasi_calls(&self) -> u64 {
        self.host_calls - self.memory.grows
    }
}

/// A store whose call hook counts the calls of wasm into the host, with a memory of at most
/// `max_memory` bytes.
fn store(engine: &Engine, max_memory: usize) -> Store<HostState> {
    let wasi = WasiCtxBuilder::new()
        .inherit_stdout()
        .inherit_stderr()
        .build_p1();
    let mut store = Store::new(
        engine,
        HostState {
            wasi,
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

/// A linker with WASI preview1.
fn linker(engine: &Engine) -> anyhow::Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)?;
    Ok(linker)
}

/// The number of calls into the host (the WASI calls) of one call of the export `export`
/// (without parameters and with one `i32` result) of a module which imports only WASI
/// preview1, counted as an [`Instance`] counts the calls of the engine.
pub fn count_wasi_calls(wasm: &[u8], export: &str) -> anyhow::Result<u64> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm)?;
    let mut store = store(&engine, MAX_MEMORY_BYTES);
    let instance = linker(&engine)?.instantiate(&mut store, &module)?;
    store.data_mut().memory.grows = 0;
    let function = instance.get_typed_func::<(), i32>(&mut store, export)?;
    let before = store.data().wasi_calls();
    function.call(&mut store, ())?;
    Ok(store.data().wasi_calls() - before)
}

/// `engine.wasm` and `mathlib.wasm`, compiled once; instantiates models.
pub struct Runtime {
    engine: Engine,
    engine_module: Module,
    mathlib_module: Module,
    memory_type: MemoryType,
    max_memory: usize,
}

impl Runtime {
    /// Compiles `engine.wasm` and `mathlib.wasm` and checks their imports and exports.
    pub fn new(engine_wasm: &[u8], mathlib_wasm: &[u8]) -> anyhow::Result<Runtime> {
        let engine = Engine::new(&Config::new())?;
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

    /// The runtime of the embedded [`ENGINE_WASM`] and [`MATHLIB_WASM`].
    pub fn embedded() -> anyhow::Result<Runtime> {
        Runtime::new(ENGINE_WASM, MATHLIB_WASM)
    }

    /// Instantiates a model with its description and loads it into a new engine.
    ///
    /// Checks the contract of the description and the module of the model: it imports
    /// `env.memory` and otherwise only functions of mathlib (each of the type mathlib exports),
    /// and defines no memory, no mutable global, no data segment and no start function (a model
    /// holds no state and writes nothing into the memory of the engine but where it is told);
    /// gives the error of `engine_load` for a description the engine refuses.
    pub fn instantiate(
        &self,
        model_wasm: &[u8],
        desc: &ModelDescription,
    ) -> anyhow::Result<Instance> {
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
        let mut store = store(&self.engine, self.max_memory);
        let memory = Memory::new(&mut store, self.memory_type.clone())?;
        let mut linker = linker(&self.engine)?;
        linker.define(&store, "env", "memory", memory)?;
        let math = linker.instantiate(&mut store, &self.mathlib_module)?;
        linker.instance(&mut store, "math", math)?;
        let model = linker
            .instantiate(&mut store, &model_module)
            .map_err(anyhow::Error::from)
            .context("model.wasm does not instantiate")?;
        linker.instance(&mut store, "model", model)?;
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
            alloc: engine.get_typed_func(&mut store, "engine_alloc")?,
            free: engine.get_typed_func(&mut store, "engine_free")?,
            load: engine.get_typed_func(&mut store, "engine_load")?,
            simulate: engine.get_typed_func(&mut store, "engine_simulate")?,
            output_ptr: engine.get_typed_func(&mut store, "engine_output_ptr")?,
            output_len: engine.get_typed_func(&mut store, "engine_output_len")?,
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

impl Runtime {
    /// Instantiates a model with mathlib alone, without the engine, to call its right hand side
    /// and its Jacobian directly (tests and diagnostics); `desc` gives the sizes.
    pub fn model_functions(
        &self,
        model_wasm: &[u8],
        desc: &ModelDescription,
    ) -> anyhow::Result<ModelFunctions> {
        check_model_sections(model_wasm)?;
        let module = Module::new(&self.engine, model_wasm)
            .map_err(anyhow::Error::from)
            .context("model.wasm does not compile")?;
        let mut store = store(&self.engine, self.max_memory);
        let memory = Memory::new(&mut store, self.memory_type.clone())?;
        let mut linker = linker(&self.engine)?;
        linker.define(&store, "env", "memory", memory)?;
        let math = linker.instantiate(&mut store, &self.mathlib_module)?;
        linker.instance(&mut store, "math", math)?;
        let model = linker
            .instantiate(&mut store, &module)
            .map_err(anyhow::Error::from)
            .context("model.wasm does not instantiate")?;
        let rhs = model.get_typed_func(&mut store, sbmlsim_abi::RHS)?;
        let jac = model.get_typed_func(&mut store, sbmlsim_abi::JAC)?;
        Ok(ModelFunctions {
            store,
            memory,
            rhs,
            jac,
            n: desc.states.len(),
            n_params: desc.parameters.len(),
            nnz: desc.jacobian.pattern().nnz(),
        })
    }
}

/// The right hand side and the Jacobian of a model instantiated without the engine
/// ([`Runtime::model_functions`]).
pub struct ModelFunctions {
    store: Store<HostState>,
    memory: Memory,
    rhs: TypedFunc<(f64, i32, i32, i32), ()>,
    jac: TypedFunc<(f64, i32, i32, i32), ()>,
    n: usize,
    n_params: usize,
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

    /// Calls `function(t, y, p, out)` with the vectors in the memory, `y` first.
    fn call(
        &mut self,
        function: TypedFunc<(f64, i32, i32, i32), ()>,
        t: f64,
        y: &[f64],
        p: &[f64],
        n_out: usize,
    ) -> anyhow::Result<Vec<f64>> {
        ensure!(
            y.len() == self.n && p.len() == self.n_params,
            "{} states and {} parameters for a model of {} and {}",
            y.len(),
            p.len(),
            self.n,
            self.n_params
        );
        let bytes = |v: &[f64]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let (y_at, p_at) = (0usize, 8 * y.len());
        let out_at = p_at + 8 * p.len();
        let end = out_at + 8 * n_out;
        let size = self.memory.data_size(&self.store);
        if end > size {
            self.memory
                .grow(&mut self.store, (end - size).div_ceil(65536) as u64)?;
        }
        self.memory.write(&mut self.store, y_at, &bytes(y))?;
        self.memory.write(&mut self.store, p_at, &bytes(p))?;
        let address = |a: usize| i32::try_from(a).context("the vectors do not fit the memory");
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
    Error { error: ErrorField },
    Table { columns: Vec<String>, n_rows: usize },
}

/// An output of the engine.
#[derive(Debug)]
enum Output {
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

/// A model loaded into an engine, in a store of its own; one per model and thread.
pub struct Instance {
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    load: TypedFunc<(i32, i32), i32>,
    simulate: TypedFunc<(i32, i32), i32>,
    output_ptr: TypedFunc<(), i32>,
    output_len: TypedFunc<(), i32>,
    trapped: bool,
    initialize_calls: u64,
    load_calls: u64,
}

impl Instance {
    /// Simulates a request; the result does not depend on the simulations before.
    ///
    /// The errors are the ones of the native engine, and [`EngineError::Internal`] for a trap
    /// (a panic of the engine, a trap of the model) or a failure of the host; an instance which
    /// trapped refuses every later request with [`EngineError::Internal`].
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        let input = serde_json::to_vec(req)
            .map_err(|e| internal(format!("the request does not serialize: {e}")))?;
        let code = self.call(self.simulate.clone(), &input)?;
        match (code, self.output()?) {
            (0, Output::Table(result)) => Ok(result),
            (1, Output::Error(error)) => Err(engine_error(error)),
            (code, output) => Err(internal(format!(
                "engine_simulate returned {code} with the output {output:?}"
            ))),
        }
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
        let len =
            i32::try_from(input.len()).map_err(|_| internal("the input is larger than 2 GiB"))?;
        let ptr = self.guard(|this| this.alloc.call(&mut this.store, len))?;
        if ptr == 0 && len > 0 {
            return Err(internal("engine_alloc found no memory for the input"));
        }
        self.memory
            .write(&mut self.store, ptr as u32 as usize, input)
            .map_err(|e| internal(format!("the input does not fit into the memory: {e}")))?;
        let code = self.guard(|this| function.call(&mut this.store, (ptr, len)))?;
        self.guard(|this| this.free.call(&mut this.store, (ptr, len)))?;
        Ok(code)
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

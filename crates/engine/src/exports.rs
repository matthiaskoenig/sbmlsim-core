//! The exports of `engine.wasm` and the model it imports (wasm32 only).
//!
//! `engine.wasm` is this crate with the feature `exports`, built as a `cdylib` for
//! `wasm32-wasip1` with an imported memory, which the model shares:
//!
//! ```text
//! cargo rustc -p sbmlsim-engine --lib --crate-type cdylib --target wasm32-wasip1 --release \
//!     --features exports -- -C link-arg=--import-memory -C link-arg=--export-memory
//! ```
//!
//! (the build script of `sbmlsim-host` runs it). It imports the memory as `env.memory`, the
//! functions of the contract `model/1` as `model.<export name>` ([`ImportedModel`]) and the
//! functions of WASI preview1 which Rust's std and wasi-libc reference; no WASI function is
//! called by [`engine_load`] or [`engine_simulate`]. It exports the memory again (as `memory`,
//! which a WASI host reads) and:
//!
//! - `engine_alloc(len) -> ptr`: `len` bytes aligned to 8 (0 for `len <= 0` or when the memory
//!   is exhausted), `engine_free(ptr, len)` gives them back.
//! - `engine_load(desc_ptr, desc_len) -> i32`: loads the model from the JSON of its
//!   [`ModelDescription`]; 0 on success, 1 with an error of the kind `load` in the output.
//! - `engine_simulate(req_ptr, req_len) -> i32`: simulates the JSON of a [`SimulationRequest`];
//!   0 with the table in the output, 1 with an error in the output.
//! - `engine_output_ptr() -> ptr` and `engine_output_len() -> len`: the output of the last call,
//!   valid until the next call of `engine_load` or `engine_simulate`.
//!
//! The output is a `u32` (little endian) with the length of a JSON header, the header, zero
//! bytes up to a multiple of 8 from the start of the output, then the values as `f64` (little
//! endian), row major. The header of a table is `{"columns":[...],"n_rows":n}` and is followed
//! by `n * columns` values; the header of an error is
//! `{"error":{"kind":"...","message":"..."}}` and is followed by nothing. The kinds are `load`
//! (a description which is not valid), `request`, `unsupported`, `integration` (with the fields
//! `flag` and `time`, `null` for a time which is not finite), as the variants of
//! [`EngineError`], and `panic` (see below).
//!
//! wasm32-wasip1 builds with `panic = "abort"`, so a panic cannot be caught: it traps. The
//! panic hook writes the error `panic` with the message of the panic into the output first
//! (without a WASI call, the default hook writes to stderr), which the host reads after the
//! trap; the instance is not used again.

use std::alloc::{Layout, alloc, dealloc};
use std::cell::RefCell;
use std::sync::Once;

use sbmlsim_abi::{ModelDescription, ModelSizes, SimulationRequest, SimulationResult};
use serde::Serialize;

use crate::{Engine, EngineError, ModelFns};

/// The functions of the contract `model/1`, imported from the module `model`; the names are the
/// export names of `sbmlsim_abi` (`DEFAULTS`, `INIT`, ...). Every pointer is an offset into the
/// shared memory.
mod imports {
    #[link(wasm_import_module = "model")]
    unsafe extern "C" {
        pub fn model_defaults(p: *mut f64, y: *mut f64);
        pub fn model_init(t: f64, p: *mut f64, y: *mut f64, fixed: *const u8);
        pub fn model_rhs(t: f64, y: *const f64, p: *const f64, ydot: *mut f64);
        pub fn model_jac(t: f64, y: *const f64, p: *const f64, values: *mut f64);
        pub fn model_roots(t: f64, y: *const f64, p: *const f64, g: *mut f64);
        pub fn model_trigger(i: u32, t: f64, y: *const f64, p: *const f64) -> i32;
        pub fn model_priority(i: u32, t: f64, y: *const f64, p: *const f64) -> f64;
        pub fn model_delay(i: u32, t: f64, y: *const f64, p: *const f64) -> f64;
        pub fn model_event_values(i: u32, t: f64, y: *const f64, p: *const f64, out: *mut f64);
        pub fn model_event_apply(i: u32, t: f64, values: *const f64, y: *mut f64, p: *mut f64);
        pub fn model_selections(t: f64, y: *const f64, p: *const f64, out: *mut f64);
        pub fn model_assign(t: f64, y: *mut f64, p: *mut f64);
        pub fn model_sizes(out: *mut u32);
    }
}

/// The model of the instance: [`ModelFns`] over the imports of the module `model`.
///
/// The slices the engine passes live in the memory the model imports, so a pointer of a slice
/// is the offset the model reads and writes; the model accesses no more than its sizes
/// (`model_sizes`), which the engine checks against the description when it loads the model,
/// and which are the sizes of the slices.
struct ImportedModel;

// SAFETY (every call below): the imported function reads and writes the given slices within
// the lengths the description of the model gives, which are their lengths.
impl ModelFns for ImportedModel {
    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        unsafe { imports::model_defaults(p.as_mut_ptr(), y.as_mut_ptr()) }
    }
    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
        unsafe { imports::model_init(t, p.as_mut_ptr(), y.as_mut_ptr(), fixed.as_ptr()) }
    }
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
        unsafe { imports::model_rhs(t, y.as_ptr(), p.as_ptr(), ydot.as_mut_ptr()) }
    }
    fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
        unsafe { imports::model_jac(t, y.as_ptr(), p.as_ptr(), values.as_mut_ptr()) }
    }
    fn roots(&self, t: f64, y: &[f64], p: &[f64], g: &mut [f64]) {
        unsafe { imports::model_roots(t, y.as_ptr(), p.as_ptr(), g.as_mut_ptr()) }
    }
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        unsafe { imports::model_trigger(i, t, y.as_ptr(), p.as_ptr()) != 0 }
    }
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        unsafe { imports::model_priority(i, t, y.as_ptr(), p.as_ptr()) }
    }
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        unsafe { imports::model_delay(i, t, y.as_ptr(), p.as_ptr()) }
    }
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        unsafe { imports::model_event_values(i, t, y.as_ptr(), p.as_ptr(), out.as_mut_ptr()) }
    }
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        unsafe { imports::model_event_apply(i, t, values.as_ptr(), y.as_mut_ptr(), p.as_mut_ptr()) }
    }
    fn selections(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        unsafe { imports::model_selections(t, y.as_ptr(), p.as_ptr(), out.as_mut_ptr()) }
    }
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        unsafe { imports::model_assign(t, y.as_mut_ptr(), p.as_mut_ptr()) }
    }
    fn sizes(&self) -> ModelSizes {
        let mut words = [0u32; ModelSizes::WORDS];
        // SAFETY: the model writes its six sizes into the buffer of six `u32`
        unsafe { imports::model_sizes(words.as_mut_ptr()) };
        ModelSizes::from_words(words.map(u32::from_le))
    }
}

/// The engine of the instance and the output of its last call.
struct State {
    engine: Option<Engine>,
    output: Vec<u8>,
}

thread_local! {
    /// The one state of the instance: wasm32-wasip1 has a single thread, so this is a plain
    /// static, and the `RefCell` makes every access checked. No borrow is held while the engine
    /// runs, so the panic hook can write the output.
    static STATE: RefCell<State> = const {
        RefCell::new(State {
            engine: None,
            output: Vec::new(),
        })
    };
}

/// The fields of an error in the header of the output.
#[derive(Serialize)]
struct ErrorBody<'a> {
    kind: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    flag: Option<i32>,
    /// `None` is written as `null` for a time which is not finite, and left out for an error
    /// without a time.
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<Option<f64>>,
}

/// The header of an error.
#[derive(Serialize)]
struct ErrorHeader<'a> {
    error: ErrorBody<'a>,
}

/// The header of a table.
#[derive(Serialize)]
struct TableHeader<'a> {
    columns: &'a [String],
    n_rows: usize,
}

/// The output of a header and the values which follow it.
fn output(header: &impl Serialize, values: &[f64]) -> Vec<u8> {
    let header = serde_json::to_vec(header).unwrap_or_else(|_| {
        br#"{"error":{"kind":"panic","message":"the header does not serialize"}}"#.to_vec()
    });
    let start = (4 + header.len()).next_multiple_of(8);
    let mut out = Vec::with_capacity(start + 8 * values.len());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.resize(start, 0);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// The output of an error of the given kind.
fn error_output(kind: &str, message: &str) -> Vec<u8> {
    let body = ErrorBody {
        kind,
        message,
        flag: None,
        time: None,
    };
    output(&ErrorHeader { error: body }, &[])
}

/// The output of an error of the engine.
fn engine_error_output(error: &EngineError) -> Vec<u8> {
    let (kind, message) = match error {
        EngineError::Request(message) => ("request", message),
        EngineError::Unsupported(message) => ("unsupported", message),
        EngineError::Internal(message) => ("panic", message),
        EngineError::Integration {
            flag,
            time,
            message,
        } => {
            let body = ErrorBody {
                kind: "integration",
                message,
                flag: Some(*flag),
                time: Some(time.is_finite().then_some(*time)),
            };
            return output(&ErrorHeader { error: body }, &[]);
        }
    };
    error_output(kind, message)
}

/// The output of a table.
fn table_output(result: &SimulationResult) -> Vec<u8> {
    let header = TableHeader {
        columns: &result.columns,
        n_rows: result.n_rows,
    };
    output(&header, &result.values)
}

/// Sets the output of the call.
fn set_output(out: Vec<u8>) {
    STATE.with_borrow_mut(|state| state.output = out);
}

/// Installs the panic hook once: it writes the error `panic` into the output and calls nothing
/// of WASI (the default hook reads `RUST_BACKTRACE` and writes to stderr).
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let out = error_output("panic", &info.to_string());
            STATE.with(|state| {
                if let Ok(mut state) = state.try_borrow_mut() {
                    state.output = out;
                }
            });
        }));
    });
}

/// The bytes of an input of the host.
///
/// # Safety
///
/// `ptr` and `len` are a buffer of [`engine_alloc`] (or any `len` readable bytes at `ptr`) which
/// nothing writes during the call.
unsafe fn input<'a>(ptr: i32, len: i32) -> &'a [u8] {
    if len <= 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr as usize as *const u8, len as usize) }
    }
}

/// The layout of a buffer of `len` bytes.
fn layout(len: i32) -> Option<Layout> {
    if len <= 0 {
        return None;
    }
    Layout::from_size_align(len as usize, 8).ok()
}

/// Allocates `len` bytes aligned to 8 in the memory of the instance; 0 for `len <= 0` or when
/// the memory is exhausted.
#[unsafe(no_mangle)]
pub extern "C" fn engine_alloc(len: i32) -> i32 {
    match layout(len) {
        // SAFETY: the layout has a size above 0
        Some(layout) => unsafe { alloc(layout) as usize as i32 },
        None => 0,
    }
}

/// Frees a buffer of [`engine_alloc`].
///
/// # Safety
///
/// `ptr` is 0 or a buffer [`engine_alloc`] returned for the same `len`, not freed before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_free(ptr: i32, len: i32) {
    if let (Some(layout), true) = (layout(len), ptr != 0) {
        unsafe { dealloc(ptr as usize as *mut u8, layout) }
    }
}

/// Loads the model from the JSON of its description; 0 on success, 1 with an error of the kind
/// `load` in the output. A model loaded before is replaced.
///
/// # Safety
///
/// As [`input`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_load(desc_ptr: i32, desc_len: i32) -> i32 {
    install_panic_hook();
    let bytes = unsafe { input(desc_ptr, desc_len) };
    let loaded = serde_json::from_slice::<ModelDescription>(bytes)
        .map_err(|e| format!("the description is not one of model/1: {e}"))
        .and_then(|desc| {
            let model = ImportedModel;
            Engine::new(Box::new(model), desc).map_err(|e| match e {
                EngineError::Request(message) => message,
                other => other.to_string(),
            })
        });
    let (engine, out, code) = match loaded {
        Ok(engine) => (Some(engine), Vec::new(), 0),
        Err(message) => (None, error_output("load", &message), 1),
    };
    STATE.with_borrow_mut(|state| {
        state.engine = engine;
        state.output = out;
    });
    code
}

/// Simulates the JSON of a request with the loaded model; 0 with the table in the output, 1
/// with an error in the output.
///
/// # Safety
///
/// As [`input`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_simulate(req_ptr: i32, req_len: i32) -> i32 {
    install_panic_hook();
    let bytes = unsafe { input(req_ptr, req_len) };
    let request = match serde_json::from_slice::<SimulationRequest>(bytes) {
        Ok(request) => request,
        Err(e) => {
            let message = format!("the request is not one of simulation/1: {e}");
            set_output(error_output("request", &message));
            return 1;
        }
    };
    // the engine is taken out of the state while it runs, so no borrow is held
    let Some(mut engine) = STATE.with_borrow_mut(|state| state.engine.take()) else {
        set_output(error_output("request", "no model is loaded"));
        return 1;
    };
    let result = engine.simulate(&request);
    STATE.with_borrow_mut(|state| state.engine = Some(engine));
    match result {
        Ok(result) => {
            set_output(table_output(&result));
            0
        }
        Err(error) => {
            set_output(engine_error_output(&error));
            1
        }
    }
}

/// The pointer of the output of the last call.
#[unsafe(no_mangle)]
pub extern "C" fn engine_output_ptr() -> i32 {
    STATE.with_borrow(|state| state.output.as_ptr() as usize as i32)
}

/// The length of the output of the last call.
#[unsafe(no_mangle)]
pub extern "C" fn engine_output_len() -> i32 {
    STATE.with_borrow(|state| state.output.len() as i32)
}

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
//! endian), row major. The output starts at an address aligned to 8, so the values are aligned
//! to 8 as well: a host reads them in place (a `Float64Array` over the memory at
//! `ptr + start`, valid until the next call or a growth of the memory) without a copy. The
//! header of a table may end in spaces (JSON whitespace): the result is recorded right behind
//! room for its header, whose length is known before the simulation but for the digits of
//! `n_rows`, so the output is the result itself and not a second copy of it. The header of a
//! table is `{"columns":[...],"n_rows":n}` and is followed by `n * columns` values; the header
//! of an error is
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

use sbmlsim_abi::{ModelDescription, ModelSizes, SimulationRequest};
use serde::Serialize;

use crate::{Engine, EngineError, ModelFns, Recorded};

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
        pub fn model_event_sizes(out: *mut u32);
    }
}

/// The model of the instance: [`ModelFns`] over the imports of the module `model`.
///
/// The slices the engine passes live in the memory the model imports, so a pointer of a slice
/// is the offset the model reads and writes. The model accesses no more of a vector than its
/// own sizes (`model_sizes` and `model_event_sizes`): the states (`y`, `ydot`), the parameters
/// (`p`), both (`fixed`), the entries of its Jacobian, its root functions (`g`), the
/// assignments of each event (`values`, `out` of `model_event_values`) and its selections.
/// `Engine::new` compares every one of them with the description when the model is loaded
/// (`check_sizes`), and the engine sizes every slice it passes by the description.
struct ImportedModel;

// SAFETY (every call below but `sizes`): the slices have the lengths the description gives,
// which `Engine::new` checked to be the sizes of the model, so the imported function reads and
// writes within them.
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
        let words = words.map(u32::from_le);
        // the number of events the model gives sizes the buffer of its counts of assignments
        let mut assignments = vec![0u32; words[ModelSizes::WORDS - 1] as usize];
        // SAFETY: the model writes one `u32` per event of its own, the length of the buffer
        unsafe { imports::model_event_sizes(assignments.as_mut_ptr()) };
        ModelSizes::from_words(words, assignments.into_iter().map(u32::from_le).collect())
    }
}

/// The output of a call: its bytes in words of 8 bytes, so it starts at an address aligned to 8.
#[derive(Default)]
struct OutputBuffer {
    words: Vec<f64>,
    /// The number of bytes of the output (at most `8 * words.len()`).
    len: usize,
    /// The output is [`NO_MEMORY_OUTPUT`] (an output which found no memory).
    no_memory: bool,
}

impl OutputBuffer {
    /// An output of the bytes; one of no memory is the error `request` in a static buffer.
    fn of_bytes(bytes: &[u8]) -> OutputBuffer {
        let mut words = Vec::new();
        if words.try_reserve_exact(bytes.len().div_ceil(8)).is_err() {
            return OutputBuffer::no_memory();
        }
        words.extend(bytes.chunks(8).map(|chunk| {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            f64::from_le_bytes(word)
        }));
        OutputBuffer {
            words,
            len: bytes.len(),
            no_memory: false,
        }
    }

    /// The error `request` of an output which does not fit into the memory, without an
    /// allocation ([`NO_MEMORY_OUTPUT`]).
    fn no_memory() -> OutputBuffer {
        OutputBuffer {
            no_memory: true,
            ..OutputBuffer::default()
        }
    }
}

/// The header of an output which does not fit into the memory of the engine.
const NO_MEMORY: &[u8] = br#"{"error":{"kind":"request","message":"the output does not fit into the memory of the engine"}}"#;

/// The length of [`NO_MEMORY_OUTPUT`].
const NO_MEMORY_LEN: usize = (4 + NO_MEMORY.len()).next_multiple_of(8);

/// Bytes aligned to 8.
#[repr(C, align(8))]
struct Aligned<const N: usize>([u8; N]);

/// The output of [`NO_MEMORY`], in static memory, so it needs no allocation.
static NO_MEMORY_OUTPUT: Aligned<NO_MEMORY_LEN> = {
    let mut bytes = [0u8; NO_MEMORY_LEN];
    let length = (NO_MEMORY.len() as u32).to_le_bytes();
    let mut i = 0;
    while i < 4 {
        bytes[i] = length[i];
        i += 1;
    }
    while i < 4 + NO_MEMORY.len() {
        bytes[i] = NO_MEMORY[i - 4];
        i += 1;
    }
    Aligned(bytes)
};

/// The engine of the instance and the output of its last call.
struct State {
    engine: Option<Engine>,
    output: OutputBuffer,
}

thread_local! {
    /// The one state of the instance: wasm32-wasip1 has a single thread, so this is a plain
    /// static, and the `RefCell` makes every access checked. No borrow is held while the engine
    /// runs, so the panic hook can write the output.
    static STATE: RefCell<State> = const {
        RefCell::new(State {
            engine: None,
            output: OutputBuffer {
                words: Vec::new(),
                len: 0,
                no_memory: false,
            },
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
    n_rows: u64,
}

/// The output of a header which no values follow.
fn output(header: &impl Serialize) -> OutputBuffer {
    let header = serde_json::to_vec(header).unwrap_or_else(|_| {
        br#"{"error":{"kind":"panic","message":"the header does not serialize"}}"#.to_vec()
    });
    let start = (4 + header.len()).next_multiple_of(8);
    let mut out = Vec::with_capacity(start);
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.resize(start, 0);
    OutputBuffer::of_bytes(&out)
}

/// The output of an error of the given kind.
fn error_output(kind: &str, message: &str) -> OutputBuffer {
    let body = ErrorBody {
        kind,
        message,
        flag: None,
        time: None,
    };
    output(&ErrorHeader { error: body })
}

/// The output of an error of the engine.
fn engine_error_output(error: &EngineError) -> OutputBuffer {
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
            return output(&ErrorHeader { error: body });
        }
    };
    error_output(kind, message)
}

/// The JSON of the header of a table.
fn table_header(columns: &[String], n_rows: u64) -> Vec<u8> {
    serde_json::to_vec(&TableHeader { columns, n_rows }).unwrap_or_default()
}

/// The slots of 8 bytes of the length and the header of a table of these columns of any number
/// of rows: the room a result is recorded behind ([`Engine::simulate_with_prefix`]).
fn header_slots(columns: &[String]) -> usize {
    (4 + table_header(columns, u64::MAX).len()).div_ceil(8)
}

/// The output of a table recorded behind the room of its header: the header is written into
/// the room (padded with spaces), so the values are not copied.
fn table_output(recorded: Recorded) -> OutputBuffer {
    let Recorded {
        columns,
        n_rows,
        mut values,
        prefix,
    } = recorded;
    let header = table_header(&columns, n_rows as u64);
    let room = 8 * prefix - 4;
    debug_assert!(header.len() <= room && prefix == header_slots(&columns));
    // SAFETY: the bytes of the first `prefix` slots of the values; every bit pattern is an f64
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(values.as_mut_ptr().cast::<u8>(), 8 * prefix) };
    bytes[..4].copy_from_slice(&(room as u32).to_le_bytes());
    bytes[4..4 + header.len()].copy_from_slice(&header);
    bytes[4 + header.len()..].fill(b' ');
    let len = 8 * values.len();
    OutputBuffer {
        words: values,
        len,
        no_memory: false,
    }
}

/// Sets the output of the call.
fn set_output(out: OutputBuffer) {
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
        Ok(engine) => (Some(engine), OutputBuffer::default(), 0),
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
    // the output of the last call is released before the simulation, which may need its memory
    set_output(OutputBuffer::default());
    let result = engine.simulate_with_prefix(&request, &header_slots);
    STATE.with_borrow_mut(|state| state.engine = Some(engine));
    match result {
        Ok(recorded) => {
            set_output(table_output(recorded));
            0
        }
        Err(error) => {
            set_output(engine_error_output(&error));
            1
        }
    }
}

/// The pointer of the output of the last call, aligned to 8.
#[unsafe(no_mangle)]
pub extern "C" fn engine_output_ptr() -> i32 {
    STATE.with_borrow(|state| match state.output.no_memory {
        true => NO_MEMORY_OUTPUT.0.as_ptr().addr() as i32,
        false => state.output.words.as_ptr().addr() as i32,
    })
}

/// The length of the output of the last call.
#[unsafe(no_mangle)]
pub extern "C" fn engine_output_len() -> i32 {
    STATE.with_borrow(|state| match state.output.no_memory {
        true => NO_MEMORY_LEN as i32,
        false => state.output.len as i32,
    })
}

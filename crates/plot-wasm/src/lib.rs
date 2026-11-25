//! The renderer of the figures in a browser, `plot.wasm`.
//!
//! The module renders a `figure/1` spec to the bytes [`sbmlsim_plot::render_svg`] gives
//! natively, gives the geometry of the figure and picks the data point under a pointer, so a
//! page draws a figure of the python package without a server and answers the pointer over
//! it. The renderer computes the same bits on every target (its transcendental functions are
//! the ones of `libm`, the PNG of the feature `png` is native only and not in the module),
//! which is why the bytes are the same; the tests of this crate compare them with the native
//! renderer, and `check_golden.mjs` runs the module in Node against the golden files of
//! `sbmlsim-plot`.
//!
//! The module is built for `wasm32-wasip1` with the profile `wasm`, as a `cdylib` like
//! `engine.wasm`, and not for `wasm32-unknown-unknown` like `compiler.wasm`: Charton depends
//! on `wasm-bindgen` and `web-sys` for every wasm target and on `ahash`, which seeds its hash
//! maps through `getrandom` with `wasm-bindgen` on `wasm32-unknown-unknown`, so a module for
//! that target imports the placeholders of `wasm-bindgen` (`__wbindgen_placeholder__`,
//! `__wbindgen_externref_xform__`), whose names and signatures are the internals of a version
//! of `wasm-bindgen` and which only its command line resolves. On `wasm32-wasip1` the same
//! code imports five functions of WASI preview1, a stable interface, and nothing else:
//! `random_get`, which it calls once per instance for the seeds of `ahash` (any bytes do,
//! zeros included: the seeds order the hash maps of Charton and decide no byte of a result),
//! and `fd_write`, `environ_get`, `environ_sizes_get` and `proc_exit`, which the standard
//! library calls at most on a panic (to write its message before the module traps), so a host
//! gives stubs which throw, as `js/loader.mjs` does for `engine.wasm`. A host calls the export
//! `_initialize` once before the first call.
//!
//! The interface is the C ABI of `compiler.wasm` (`sbmlsim-wasm`), five functions which need
//! no binding generator and no tool beyond cargo. Every string crosses the boundary as the
//! bytes of its UTF-8 in the memory of the module:
//!
//! 1. `allocate(len)` gives a buffer of `len` bytes, which the caller writes the spec into,
//! 2. `render(pointer, len, out)` renders the spec and writes the pointer and the length of
//!    the SVG to the two words at `out` (a buffer of two `usize`, 8 bytes of `allocate` for the
//!    32 bit words of `wasm32`),
//! 3. `geometry(pointer, len, out)` writes the JSON of the [`sbmlsim_plot::Geometry`] of the
//!    figure, its panels, their scales and the entries of its legends in its pixels,
//! 4. `pick(pointer, len, x, y, radius, out)` writes the JSON of the [`sbmlsim_plot::Pick`] of
//!    the data point nearest to the pixel `(x, y)` within `radius` pixels, or the JSON `null`
//!    when there is none,
//! 5. `deallocate(pointer, len)` gives a buffer back, the results included: every buffer of
//!    the interface is a slice of bytes of exactly its length, so one function frees them all.
//!
//! `render`, `geometry` and `pick` give 1 for a spec they read and 0 for one they refuse,
//! whose message (`path: what is wrong`, as the python package raises it) is the result.
//!
//! ```
//! let spec = r#"{"contract": "figure/1", "width": 300, "height": 200,
//!     "grid": {"rows": 1, "columns": 1},
//!     "plots": [{"id": "p", "row": 0, "column": 0, "x": {"label": "t"}, "y": {"label": "c"},
//!                "elements": [{"id": "c", "kind": "curve", "x": [0, 1], "y": [1, 2]}]}]}"#;
//! let svg = sbmlsim_plot_wasm::render_figure(spec).expect("the spec renders");
//! assert!(svg.starts_with("<svg"), "{svg}");
//! let refused = sbmlsim_plot_wasm::render_figure("{}").unwrap_err();
//! assert!(!refused.is_empty());
//! ```

use std::alloc::{Layout, alloc, dealloc};

use sbmlsim_plot::{Figure, SpecError, parse};

/// The message of a spec which is not read: the path of the field and what is wrong.
fn message_of(error: SpecError) -> String {
    format!("{}: {}", error.path, error.message)
}

/// Reads a spec, the first step of every function of the interface.
fn figure_of(spec: &str) -> Result<Figure, String> {
    parse(spec).map_err(message_of)
}

/// The JSON text of a result of the renderer.
fn json_of(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("the result is not JSON: {e}"))
}

/// The SVG of a figure given as the JSON text of a `figure/1` spec, the bytes of
/// [`sbmlsim_plot::render_svg`].
///
/// # Errors
///
/// Gives the message of a spec which is not read, `path: what is wrong`.
pub fn render_figure(spec: &str) -> Result<String, String> {
    figure_of(spec).map(|figure| sbmlsim_plot::render_svg(&figure))
}

/// The JSON of the [`sbmlsim_plot::Geometry`] of a figure.
///
/// # Errors
///
/// Gives the message of a spec which is not read.
pub fn figure_geometry(spec: &str) -> Result<String, String> {
    json_of(&sbmlsim_plot::geometry(&figure_of(spec)?))
}

/// The JSON of the [`sbmlsim_plot::Pick`] of the data point nearest to the pixel `(x, y)`
/// within `radius` pixels, the JSON `null` without one.
///
/// # Errors
///
/// Gives the message of a spec which is not read.
pub fn pick_point(spec: &str, x: f64, y: f64, radius: f64) -> Result<String, String> {
    json_of(&sbmlsim_plot::pick(&figure_of(spec)?, x, y, radius))
}

/// The layout of a buffer of the interface: bytes, without an alignment.
///
/// # Panics
///
/// Panics for a length above `isize::MAX`, which no spec reaches.
fn layout(len: usize) -> Layout {
    Layout::from_size_align(len, 1).expect("a layout of bytes")
}

/// A buffer of `len` bytes for the caller to write into.
///
/// The buffer is uninitialized and belongs to the caller until it gives it back with
/// [`deallocate`]. A length of zero gives the null pointer.
///
/// # Safety
///
/// The caller frees the buffer with [`deallocate`] and the same length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn allocate(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the layout is not of size zero, which the branch above returns for
    unsafe { alloc(layout(len)) }
}

/// Gives a buffer of [`allocate`] or of a result back.
///
/// # Safety
///
/// `pointer` is a buffer of [`allocate`] or a result of [`render`], [`geometry`] or [`pick`]
/// which was not freed yet, and `len` its length; a null pointer and a length of zero are
/// ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deallocate(pointer: *mut u8, len: usize) {
    if pointer.is_null() || len == 0 {
        return;
    }
    // SAFETY: the pointer is a buffer of `allocate` of exactly this length
    unsafe { dealloc(pointer, layout(len)) }
}

/// The text of the spec in a buffer of the caller.
///
/// # Safety
///
/// `pointer` is a buffer of `len` bytes of [`allocate`], or null with a length of zero.
unsafe fn spec_of<'a>(pointer: *const u8, len: usize) -> Result<&'a str, String> {
    let bytes = if pointer.is_null() || len == 0 {
        // a spec of no bytes, which the parser refuses as no JSON
        &[][..]
    } else {
        // SAFETY: the caller gives a buffer of `allocate` of exactly this length
        unsafe { std::slice::from_raw_parts(pointer, len) }
    };
    std::str::from_utf8(bytes).map_err(|_| "the spec is not UTF-8".to_owned())
}

/// Writes a result or a message into the two words at `out`, which the caller then owns, and
/// gives 1 for a result and 0 for a message.
///
/// # Safety
///
/// `out` points at two words which the caller allocated.
unsafe fn answer(result: Result<String, String>, out: *mut usize) -> u32 {
    let (status, text) = match result {
        Ok(text) => (1, text),
        Err(message) => (0, message),
    };
    let mut bytes = text.into_bytes().into_boxed_slice();
    let len = bytes.len();
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    // SAFETY: the caller gives two words of its own memory
    unsafe {
        *out = pointer as usize;
        *out.add(1) = len;
    }
    status
}

/// Renders the spec of the buffer to SVG.
///
/// Writes the pointer and the length of the SVG to the two words at `out` and gives 1; for a
/// spec it refuses it writes the pointer and the length of the message and gives 0. The result
/// belongs to the caller, which frees it with [`deallocate`].
///
/// # Safety
///
/// `pointer` is a buffer of `len` bytes of [`allocate`] with the UTF-8 of the spec and `out` a
/// buffer of two words (8 bytes of [`allocate`] for `wasm32`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn render(pointer: *const u8, len: usize, out: *mut usize) -> u32 {
    // SAFETY: the caller gives the buffers documented above
    unsafe { answer(spec_of(pointer, len).and_then(render_figure), out) }
}

/// Gives the JSON of the geometry of the figure of the buffer, as [`render`] gives its SVG.
///
/// # Safety
///
/// The buffers satisfy the requirements of [`render`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geometry(pointer: *const u8, len: usize, out: *mut usize) -> u32 {
    // SAFETY: the caller gives the buffers documented on `render`
    unsafe { answer(spec_of(pointer, len).and_then(figure_geometry), out) }
}

/// Gives the JSON of the data point of the figure of the buffer nearest to the pixel `(x, y)`
/// within `radius` pixels, the JSON `null` without one, as [`render`] gives its SVG.
///
/// # Safety
///
/// The buffers satisfy the requirements of [`render`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pick(
    pointer: *const u8,
    len: usize,
    x: f64,
    y: f64,
    radius: f64,
    out: *mut usize,
) -> u32 {
    // SAFETY: the caller gives the buffers documented on `render`
    unsafe {
        answer(
            spec_of(pointer, len).and_then(|spec| pick_point(spec, x, y, radius)),
            out,
        )
    }
}

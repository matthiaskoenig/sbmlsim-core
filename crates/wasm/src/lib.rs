//! The compiler in a browser, `compiler.wasm`.
//!
//! The module is built for `wasm32-unknown-unknown` and has no dependency on a host: it reads
//! no file and calls nothing of the outside, as [`sbmlsim_compiler`] does neither, so a page
//! which loads it compiles an SBML model without a server. `js/compile.mjs` is the caller,
//! which hands the `model.wasm` and the description of a compilation to `js/loader.mjs`, the
//! host which runs them.
//!
//! The interface is the C ABI of four functions, since the module needs no binding generator
//! and no tool beyond cargo. Every string crosses the boundary as the bytes of its UTF-8 in
//! the memory of the module:
//!
//! 1. `allocate(len)` gives a buffer of `len` bytes, which the caller writes the document and
//!    the options into,
//! 2. `compile(pointer, len, options, options_len, out)` compiles the document with the options
//!    (the JSON of [`ModelOptions`], an empty buffer for the defaults) and writes four words to
//!    `out` (a buffer of four `usize`, 16 bytes of `allocate` for the 32 bit words of
//!    `wasm32`): the pointer and the length of the description of the model, which is the JSON
//!    of `model.json`, and the pointer and the length of `model.wasm`. It gives 1 for a model
//!    it compiled and 0 for one it refuses (or options which are not those of the JSON), whose
//!    message is the first buffer, with the second one empty,
//! 3. `compile_kernel(pointer, len, options, options_len, out)` compiles the document into a
//!    batched kernel (the model with its RODAS4 integrator in one module, `kernel/2`) with the
//!    options of [`KernelOptions`] (JSON, an empty buffer for the defaults) and the same result:
//!    the description is the JSON of `kernel.json`, the module `kernel.wasm`,
//! 4. `deallocate(pointer, len)` gives a buffer back, the results included: every buffer of
//!    the interface is a slice of bytes of exactly its length, so one function frees them all.
//!
//! The features `petab` and `fit` add the calls of a PEtab problem and its fit ([`petab`]), and
//! the feature `stats` the statistics of the analyses, `stats_call` ([`stats`]), all on by
//! default.
//!
//! The options of `compile` are `{"optimize": true, "jacobian": "auto", "parameter_jacobian":
//! true, "driven": []}`, each optional with that default: whether the optimizer of the compiler
//! runs (the results are the same either way), the source of the Jacobian of the model
//! (`"auto"` the analytic one where the model has one and the colored difference quotient
//! otherwise, `"difference_quotient"` always the difference quotient, `"analytic"` the analytic
//! one, which refuses a model which has none), whether the analytic derivatives of the right
//! hand side in the parameters are included, and the species to drive
//! (`sbmlsim_compiler::CompileOptions::driven`). Those of `compile_kernel` are `{"driven": []}`.
//!
//! ```
//! let model = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3"
//!     version="2"><model id="m"><listOfCompartments><compartment id="c" size="1"
//!     constant="true"/></listOfCompartments><listOfSpecies><species id="s" compartment="c"
//!     initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false"
//!     constant="false"/></listOfSpecies></model></sbml>"#;
//! let options = sbmlsim_wasm::ModelOptions::default();
//! let compiled = sbmlsim_wasm::compile_model(model, &options).expect("the model compiles");
//! assert!(compiled.wasm.starts_with(b"\0asm"));
//! assert!(compiled.description.contains(r#""s""#), "{}", compiled.description);
//! ```

use std::alloc::{Layout, alloc, dealloc};

#[cfg(feature = "petab")]
pub mod petab;
#[cfg(feature = "stats")]
pub mod stats;

use sbmlsim_abi::JacobianChoice;
use sbmlsim_compiler::CompileOptions;
pub use sbmlsim_compiler::KernelOptions;

/// A model the compiler translated: the description as JSON and `model.wasm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The description of the model, the JSON of `model.json`.
    pub description: String,
    /// The module `model.wasm`.
    pub wasm: Vec<u8>,
}

/// The options of `compile`, its JSON: those of [`CompileOptions`], each optional with the
/// default of [`CompileOptions::default`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOptions {
    /// [`CompileOptions::optimize`].
    pub optimize: bool,
    /// [`CompileOptions::jacobian`]: `"auto"`, `"difference_quotient"` or `"analytic"`.
    pub jacobian: JacobianChoice,
    /// [`CompileOptions::parameter_jacobian`].
    pub parameter_jacobian: bool,
    /// [`CompileOptions::driven`].
    pub driven: Vec<String>,
}

impl Default for ModelOptions {
    fn default() -> Self {
        let defaults = CompileOptions::default();
        Self {
            optimize: defaults.optimize,
            jacobian: defaults.jacobian,
            parameter_jacobian: defaults.parameter_jacobian,
            driven: defaults.driven,
        }
    }
}

/// Compiles an SBML document, the entry point of the interface and of the tests.
///
/// # Errors
///
/// Gives the message of the reader, of the builder, of the driven species or of the code
/// generation for a document which is not compiled.
pub fn compile_model(sbml: &str, options: &ModelOptions) -> Result<Compiled, String> {
    let options = CompileOptions {
        optimize: options.optimize,
        jacobian: options.jacobian,
        parameter_jacobian: options.parameter_jacobian,
        driven: options.driven.clone(),
    };
    let compiled = sbmlsim_compiler::compile(sbml, &options).map_err(|err| err.to_string())?;
    let description = serde_json::to_string(&compiled.description)
        .map_err(|err| format!("the description of the model is not JSON: {err}"))?;
    Ok(Compiled {
        description,
        wasm: compiled.wasm,
    })
}

/// Compiles an SBML document into a batched kernel (`kernel/2`): the description is the JSON of
/// `kernel.json`, the module `kernel.wasm`.
///
/// # Errors
///
/// Gives the message of the reader, of the builder, of the driven species or of the code
/// generation for a document which is not compiled, e.g. a model with events.
pub fn compile_kernel_model(sbml: &str, options: &KernelOptions) -> Result<Compiled, String> {
    let compiled =
        sbmlsim_compiler::compile_kernel(sbml, options).map_err(|err| err.to_string())?;
    let description = serde_json::to_string(&compiled.description)
        .map_err(|err| format!("the description of the kernel is not JSON: {err}"))?;
    Ok(Compiled {
        description,
        wasm: compiled.wasm,
    })
}

/// The bytes of a buffer of the caller: empty for a null pointer or a length of zero.
///
/// # Safety
///
/// A pointer which is not null is a buffer of [`allocate`] of exactly `len` bytes.
unsafe fn bytes_of<'a>(pointer: *const u8, len: usize) -> &'a [u8] {
    if pointer.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller gives a buffer of `allocate` of exactly this length
        unsafe { std::slice::from_raw_parts(pointer, len) }
    }
}

/// The document and the options of a compilation, read from the buffers of the caller: the
/// text of the document and the options of their JSON (the defaults for an empty buffer).
///
/// # Safety
///
/// The buffers satisfy the requirements of [`bytes_of`].
unsafe fn request<'a, T: serde::de::DeserializeOwned + Default>(
    pointer: *const u8,
    len: usize,
    options: *const u8,
    options_len: usize,
) -> Result<(&'a str, T), String> {
    // SAFETY: the caller gives buffers of `allocate` of exactly these lengths
    let (document, options) = unsafe { (bytes_of(pointer, len), bytes_of(options, options_len)) };
    let text = std::str::from_utf8(document).map_err(|_| "the document is not UTF-8".to_owned())?;
    let options = if options.is_empty() {
        T::default()
    } else {
        serde_json::from_slice(options)
            .map_err(|err| format!("the options of the compilation are not valid: {err}"))?
    };
    Ok((text, options))
}

/// The layout of a buffer of the interface: bytes, without an alignment.
///
/// # Panics
///
/// Panics for a length above `isize::MAX`, which no document reaches.
fn layout(len: usize) -> Layout {
    Layout::from_size_align(len, 1).expect("a layout of bytes")
}

/// A buffer of `len` bytes for the caller to write into.
///
/// The buffer is uninitialized and belongs to the caller until it gives it back with
/// [`deallocate`]. A length of zero gives the null pointer, and so does a buffer which cannot
/// be allocated (the memory cannot grow), which the host checks before it writes into it. The
/// buffer has the alignment of bytes: a word the caller reads from it or the module writes to
/// it is unaligned.
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
/// `pointer` is a buffer of [`allocate`] or a result of [`compile`] which was not freed yet,
/// and `len` its length; a null pointer and a length of zero are ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deallocate(pointer: *mut u8, len: usize) {
    if pointer.is_null() || len == 0 {
        return;
    }
    // SAFETY: the pointer is a buffer of `allocate` of exactly this length
    unsafe { dealloc(pointer, layout(len)) }
}

/// Writes `value` to the word `k` of `out`.
///
/// The word is written unaligned: `out` is a buffer of [`allocate`], whose layout has the
/// alignment of bytes, so it is at any address the allocator gives.
///
/// # Safety
///
/// `out` points at more than `k` words (`(k + 1) * size_of::<usize>()` bytes) which the caller
/// allocated.
unsafe fn put_word(out: *mut usize, k: usize, value: usize) {
    // SAFETY: the word is in the buffer of the caller; `write_unaligned` needs no alignment
    unsafe { out.add(k).write_unaligned(value) }
}

/// Writes a byte vector into the two words at `out` and forgets it, so the caller owns it.
///
/// The words are written unaligned, see [`put_word`].
///
/// # Safety
///
/// `out` points at two words which the caller allocated.
unsafe fn give(bytes: Vec<u8>, out: *mut usize) {
    let mut bytes = bytes.into_boxed_slice();
    let len = bytes.len();
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    // SAFETY: the caller gives two words of its own memory
    unsafe {
        put_word(out, 0, pointer as usize);
        put_word(out, 1, len);
    }
}

/// Options and explicit companion documents of a standalone source export.
#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct OdeRequest {
    format: String,
    location: String,
    documents: std::collections::BTreeMap<String, String>,
    max_expanded_elements: usize,
    render: sbmlsim_ode::RenderOptions,
}
impl Default for OdeRequest {
    fn default() -> Self {
        Self {
            format: "python".into(),
            location: "./model.xml".into(),
            documents: Default::default(),
            max_expanded_elements: 10000,
            render: Default::default(),
        }
    }
}

/// Generate standalone source code or a document with the same Rust kernel as Python.
///
/// The result uses the compiler's four-word ownership protocol. The first buffer is
/// a JSON export (`filename`, `content`) on success and a diagnostic on failure;
/// the second buffer is empty. No WASI, filesystem or network import is required.
///
/// # Safety
/// All three input/output regions belong to the caller and were allocated by
/// [`allocate`]. The options are UTF-8 JSON and `out` holds four words.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn export_ode(
    pointer: *const u8,
    len: usize,
    options: *const u8,
    options_len: usize,
    out: *mut usize,
) -> u32 {
    let result = (|| {
        if len > 100 * 1024 * 1024 || options_len > 100 * 1024 * 1024 {
            return Err("The source export exceeds 100 MB.".to_owned());
        }
        // SAFETY: the caller owns both regions, as required by this function's contract.
        let source = if pointer.is_null() || len == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(pointer, len) }
        };
        let options = if options.is_null() || options_len == 0 {
            &b"{}"[..]
        } else {
            unsafe { std::slice::from_raw_parts(options, options_len) }
        };
        let request: OdeRequest = serde_json::from_slice(options).map_err(|e| e.to_string())?;
        let documents = request
            .documents
            .into_iter()
            .map(|(name, xml)| (name, xml.into_bytes()))
            .collect::<Vec<_>>();
        let system = sbmlsim_ode::OdeSystem::read_with_sources(
            source,
            &request.location,
            &documents,
            request.max_expanded_elements,
        )?;
        let export = system.render(&request.format, &request.render)?;
        serde_json::to_vec(&export).map_err(|e| e.to_string())
    })();
    let (success, bytes) = match result {
        Ok(bytes) => (1, bytes),
        Err(error) => (0, error.into_bytes()),
    };
    // SAFETY: the caller supplied four writable words; `give` transfers the owned result.
    unsafe {
        give(bytes, out);
        put_word(out, 2, 0);
        put_word(out, 3, 0);
    }
    success
}

/// Compiles the SBML document of the buffer into `model.wasm` and its description, with the
/// options of the JSON of [`ModelOptions`] (an empty buffer for the defaults).
///
/// Writes the pointer and the length of the description and of the module to the four words at
/// `out` and gives 1; for a document it refuses (or options which are not those of the JSON) it
/// writes the pointer and the length of the message, an empty module, and gives 0. Every buffer
/// belongs to the caller, which frees it with [`deallocate`].
///
/// # Safety
///
/// `pointer` is a buffer of `len` bytes of [`allocate`] with the UTF-8 of the document,
/// `options` one of `options_len` bytes with the UTF-8 of the options (or null and 0), and `out`
/// a buffer of four words (16 bytes of [`allocate`] for `wasm32`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compile(
    pointer: *const u8,
    len: usize,
    options: *const u8,
    options_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the caller gives the buffers documented above
    let result = unsafe { request::<ModelOptions>(pointer, len, options, options_len) }
        .and_then(|(text, options)| compile_model(text, &options));
    // SAFETY: the caller gives four words of its own memory
    unsafe { deliver(result, out) }
}

/// Compiles the SBML document of the buffer into a batched kernel, `kernel.wasm` and its
/// description (the JSON of `kernel.json`), with the options `{"driven": [...]}` (an empty
/// buffer for the defaults) and the result of [`compile`].
///
/// # Safety
///
/// The buffers satisfy the same requirements as those of [`compile`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compile_kernel(
    pointer: *const u8,
    len: usize,
    options: *const u8,
    options_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the caller gives the buffers documented above
    let result = unsafe { request::<KernelOptions>(pointer, len, options, options_len) }
        .and_then(|(text, options)| compile_kernel_model(text, &options));
    // SAFETY: the caller gives four words of its own memory
    unsafe { deliver(result, out) }
}

/// Writes a compilation to the four words at `out`: the description and the module and 1, or
/// the message, an empty module and 0.
///
/// # Safety
///
/// `out` points at four words which the caller allocated.
unsafe fn deliver(result: Result<Compiled, String>, out: *mut usize) -> u32 {
    match result {
        Ok(compiled) => {
            // SAFETY: the caller gives four words of its own memory
            unsafe {
                give(compiled.description.into_bytes(), out);
                give(compiled.wasm, out.add(2));
            }
            1
        }
        Err(message) => {
            // SAFETY: the caller gives four words of its own memory
            unsafe {
                give(message.into_bytes(), out);
                put_word(out, 2, 0);
                put_word(out, 3, 0);
            }
            0
        }
    }
}

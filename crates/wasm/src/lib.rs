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
//! 1. `allocate(len)` gives a buffer of `len` bytes, which the caller writes the document
//!    into,
//! 2. `compile(pointer, len, optimize, jacobian, out)` compiles the document and writes four
//!    words to `out` (a buffer of four `usize`, 16 bytes of `allocate` for the 32 bit words of
//!    `wasm32`): the pointer and the length of the description of the model, which is the JSON
//!    of `model.json`, and the pointer and the length of `model.wasm`. It gives 1 for a model
//!    it compiled and 0 for one it refuses, whose message is the first buffer, with the second
//!    one empty,
//! 3. `deallocate(pointer, len)` gives a buffer back, the results included: every buffer of
//!    the interface is a slice of bytes of exactly its length, so one function frees them all.
//!
//! `optimize` is whether the optimizer of the compiler runs (1 or 0, the results are the same
//! either way) and `jacobian` the source of the Jacobian of the model: 0 the analytic one
//! where the model has one and the colored difference quotient otherwise, 1 the difference
//! quotient always, 2 the analytic one, which refuses a model which has none.
//!
//! ```
//! let model = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3"
//!     version="2"><model id="m"><listOfCompartments><compartment id="c" size="1"
//!     constant="true"/></listOfCompartments><listOfSpecies><species id="s" compartment="c"
//!     initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false"
//!     constant="false"/></listOfSpecies></model></sbml>"#;
//! let compiled = sbmlsim_wasm::compile_model(model, true, 0).expect("the model compiles");
//! assert!(compiled.wasm.starts_with(b"\0asm"));
//! assert!(compiled.description.contains(r#""s""#), "{}", compiled.description);
//! ```

use std::alloc::{Layout, alloc, dealloc};

use sbmlsim_abi::JacobianChoice;
use sbmlsim_compiler::CompileOptions;

/// A model the compiler translated: the description as JSON and `model.wasm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The description of the model, the JSON of `model.json`.
    pub description: String,
    /// The module `model.wasm`.
    pub wasm: Vec<u8>,
}

/// The source of the Jacobian a number of the interface stands for.
///
/// Anything but 1 and 2 is the default, the analytic Jacobian where the model has one.
fn jacobian_of(choice: u32) -> JacobianChoice {
    match choice {
        1 => JacobianChoice::DifferenceQuotient,
        2 => JacobianChoice::Analytic,
        _ => JacobianChoice::Auto,
    }
}

/// Compiles an SBML document, the entry point of the interface and of the tests.
///
/// # Errors
///
/// Gives the message of the reader, of the builder or of the code generation for a document
/// which is not compiled.
pub fn compile_model(sbml: &str, optimize: bool, jacobian: u32) -> Result<Compiled, String> {
    let options = CompileOptions {
        optimize,
        jacobian: jacobian_of(jacobian),
    };
    let compiled = sbmlsim_compiler::compile(sbml, &options).map_err(|err| err.to_string())?;
    let description = serde_json::to_string(&compiled.description)
        .map_err(|err| format!("the description of the model is not JSON: {err}"))?;
    Ok(Compiled {
        description,
        wasm: compiled.wasm,
    })
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

/// Writes a byte vector into the two words at `out` and forgets it, so the caller owns it.
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
        *out = pointer as usize;
        *out.add(1) = len;
    }
}

/// Compiles the SBML document of the buffer into `model.wasm` and its description.
///
/// Writes the pointer and the length of the description and of the module to the four words at
/// `out` and gives 1; for a document it refuses it writes the pointer and the length of the
/// message, an empty module, and gives 0. Every buffer belongs to the caller, which frees it
/// with [`deallocate`].
///
/// # Safety
///
/// `pointer` is a buffer of `len` bytes of [`allocate`] with the UTF-8 of the document and
/// `out` a buffer of four words (16 bytes of [`allocate`] for `wasm32`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compile(
    pointer: *const u8,
    len: usize,
    optimize: u32,
    jacobian: u32,
    out: *mut usize,
) -> u32 {
    let bytes = if pointer.is_null() || len == 0 {
        // a document of no bytes, which the compiler refuses as an empty one
        &[][..]
    } else {
        // SAFETY: the caller gives a buffer of `allocate` of exactly this length
        unsafe { std::slice::from_raw_parts(pointer, len) }
    };
    let result = match std::str::from_utf8(bytes) {
        Ok(text) => compile_model(text, optimize != 0, jacobian),
        Err(_) => Err("the document is not UTF-8".to_owned()),
    };
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
                *out.add(2) = 0;
                *out.add(3) = 0;
            }
            0
        }
    }
}

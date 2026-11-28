//! The C ABI of `compiler.wasm`, driven on the host as `js/compile.mjs` drives it.
//!
//! The test writes a document into a buffer of `allocate`, calls `compile` with a buffer of
//! four words, reads the description and the module out of the memory and frees every buffer,
//! which is what the JavaScript host does with the memory of the module.

use std::slice;

use sbmlsim_wasm::{
    allocate, compile, compile_kernel, compile_with_options, deallocate, export_ode,
};

/// A model with one species a reaction changes, which compiles.
const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="decay">
    <listOfCompartments>
      <compartment id="c" size="1" constant="true"/>
    </listOfCompartments>
    <listOfSpecies>
      <species id="s" compartment="c" initialConcentration="10"
               hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="k" value="0.1" constant="true"/>
    </listOfParameters>
    <listOfReactions>
      <reaction id="r" reversible="false">
        <listOfReactants>
          <speciesReference species="s" stoichiometry="1" constant="true"/>
        </listOfReactants>
        <kineticLaw>
          <math xmlns="http://www.w3.org/1998/Math/MathML">
            <apply><times/><ci>k</ci><ci>s</ci><ci>c</ci></apply>
          </math>
        </kineticLaw>
      </reaction>
    </listOfReactions>
  </model>
</sbml>
"#;

/// The bytes of a buffer of a result, which is empty when the interface gives none.
///
/// # Safety
///
/// `pointer` and `len` are a buffer of a result which was not freed yet.
unsafe fn bytes_of(pointer: usize, len: usize) -> Vec<u8> {
    if pointer == 0 || len == 0 {
        return Vec::new();
    }
    // SAFETY: the caller gives a buffer of the interface of exactly this length
    unsafe { slice::from_raw_parts(pointer as *const u8, len).to_vec() }
}

/// Compiles a document through the interface and gives the description and the module.
///
/// # Panics
///
/// Panics when the interface gives a result which is not UTF-8.
fn through_the_interface(sbml: &str) -> (u32, String, Vec<u8>) {
    through_the_interface_with_options(sbml, true)
}

fn through_the_interface_with_options(
    sbml: &str,
    parameter_jacobian: bool,
) -> (u32, String, Vec<u8>) {
    // SAFETY: the buffers are the ones of the interface
    through(sbml, |input, len, words| unsafe {
        if parameter_jacobian {
            compile(input, len, 1, 0, words)
        } else {
            compile_with_options(input, len, 1, 0, 0, words)
        }
    })
}

/// A document through a function of the interface (`call(input, len, words)`), which writes
/// its result to four words: the status, the description (or the message) and the module.
fn through(
    sbml: &str,
    call: impl Fn(*const u8, usize, *mut usize) -> u32,
) -> (u32, String, Vec<u8>) {
    let bytes = sbml.as_bytes();
    // SAFETY: the buffers are the ones of the interface and are freed below
    unsafe {
        let input = allocate(bytes.len());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), input, bytes.len());
        let words = allocate(4 * size_of::<usize>()).cast::<usize>();
        let status = call(input, bytes.len(), words);
        let description = bytes_of(*words, *words.add(1));
        let wasm = bytes_of(*words.add(2), *words.add(3));
        deallocate(*words as *mut u8, *words.add(1));
        deallocate(*words.add(2) as *mut u8, *words.add(3));
        deallocate(words.cast::<u8>(), 4 * size_of::<usize>());
        deallocate(input, bytes.len());
        (
            status,
            String::from_utf8(description).expect("the description is UTF-8"),
            wasm,
        )
    }
}

#[test]
fn a_model_gives_its_module_and_its_description() {
    let (status, description, wasm) = through_the_interface(MODEL);
    assert_eq!(status, 1, "{description}");
    assert!(wasm.starts_with(b"\0asm"), "the module is WebAssembly");
    let value: serde_json::Value =
        serde_json::from_str(&description).expect("the description is JSON");
    assert_eq!(value["id"], "decay");
    assert!(
        description.contains(r#""s""#),
        "the description names the species: {description}"
    );
}

#[test]
fn a_document_which_is_no_model_gives_its_message() {
    let (status, message, wasm) = through_the_interface("<not-sbml/>");
    assert_eq!(status, 0, "{message}");
    assert!(wasm.is_empty(), "a refused document has no module");
    assert!(!message.is_empty(), "a refused document has a message");
}

#[test]
fn an_empty_document_is_refused_without_a_module() {
    let (status, message, wasm) = through_the_interface("");
    assert_eq!(status, 0, "{message}");
    assert!(wasm.is_empty());
}

#[test]
fn parameter_derivatives_are_optional_in_the_extended_interface() {
    let (status, description, wasm) = through_the_interface_with_options(MODEL, false);
    assert_eq!(status, 1, "{description}");
    assert!(wasm.starts_with(b"\0asm"));
    let description: serde_json::Value = serde_json::from_str(&description).unwrap();
    assert!(description.get("jacobian_p").is_none());
}

#[test]
fn a_kernel_gives_its_module_and_its_description() {
    // SAFETY: the buffers are the ones of the interface
    let kernel = |sbml| {
        through(sbml, |input, len, words| unsafe {
            compile_kernel(input, len, words)
        })
    };
    let (status, description, wasm) = kernel(MODEL);
    assert_eq!(status, 1, "{description}");
    assert!(wasm.starts_with(b"\0asm"));
    let value: serde_json::Value = serde_json::from_str(&description).unwrap();
    assert_eq!(value["version"], "kernel/1");
    assert_eq!(value["state_ids"], serde_json::json!(["s"]));
    assert_eq!(value["model"]["id"], "decay");
    let compiled = sbmlsim_wasm::compile_kernel_model(MODEL).unwrap();
    assert_eq!((compiled.description, compiled.wasm), (description, wasm));
    // a model with an event is refused with the message of the code generation
    let event = MODEL.replace(
        "</listOfReactions>",
        r#"</listOfReactions><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger></event></listOfEvents>"#,
    );
    let (status, message, wasm) = kernel(&event);
    assert_eq!(status, 0);
    assert!(wasm.is_empty());
    assert!(message.contains("events"), "{message}");
}

/// Calls a function of the interface with its four words of the result at an odd address, as a
/// buffer of `allocate` (whose layout has the alignment of bytes) may be, and gives the status
/// and the two buffers of the result, which it frees.
fn at_an_odd_address(call: impl Fn(*mut usize) -> u32) -> (u32, Vec<u8>, Vec<u8>) {
    let mut storage = vec![0_u8; 4 * size_of::<usize>() + 1];
    let out = storage.as_mut_ptr().wrapping_add(1).cast::<usize>();
    assert_ne!(
        out as usize % align_of::<usize>(),
        0,
        "the words are not aligned"
    );
    let status = call(out);
    // SAFETY: the four words are in `storage`, read unaligned, and their buffers are the
    // interface's, freed once
    unsafe {
        let words: Vec<usize> = (0..4).map(|k| out.add(k).read_unaligned()).collect();
        let first = bytes_of(words[0], words[1]);
        let second = bytes_of(words[2], words[3]);
        deallocate(words[0] as *mut u8, words[1]);
        deallocate(words[2] as *mut u8, words[3]);
        (status, first, second)
    }
}

#[test]
fn the_words_of_a_result_are_written_at_any_alignment() {
    let model = MODEL.as_bytes();
    let refused = b"<not-sbml/>";
    // SAFETY: the documents are buffers of exactly their lengths, the words are four
    let (status, description, wasm) =
        at_an_odd_address(|out| unsafe { compile(model.as_ptr(), model.len(), 1, 0, out) });
    assert_eq!(status, 1, "{}", String::from_utf8_lossy(&description));
    assert!(wasm.starts_with(b"\0asm"));
    let (status, message, wasm) =
        at_an_odd_address(|out| unsafe { compile(refused.as_ptr(), refused.len(), 1, 0, out) });
    assert_eq!(status, 0);
    assert!(!message.is_empty() && wasm.is_empty());
    let (status, _, kernel) =
        at_an_odd_address(|out| unsafe { compile_kernel(model.as_ptr(), model.len(), out) });
    assert_eq!(status, 1);
    assert!(kernel.starts_with(b"\0asm"));
    let options = br#"{"format":"markdown"}"#;
    let (status, export, empty) = at_an_odd_address(|out| unsafe {
        export_ode(
            model.as_ptr(),
            model.len(),
            options.as_ptr(),
            options.len(),
            out,
        )
    });
    assert_eq!(status, 1, "{}", String::from_utf8_lossy(&export));
    assert!(empty.is_empty());
}

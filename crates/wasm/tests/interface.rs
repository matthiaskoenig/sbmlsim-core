//! The C ABI of `compiler.wasm`, driven on the host as `js/compile.mjs` drives it.
//!
//! The test writes a document and its options into buffers of `allocate`, calls `compile` (or
//! `compile_kernel`) with a buffer of four words, reads the description and the module out of
//! the memory and frees every buffer, which is what the JavaScript host does with the memory of
//! the module.

use std::slice;

use sbmlsim_wasm::{KernelOptions, ModelOptions, allocate, compile, compile_kernel, deallocate};

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

/// Compiles a document through the interface with the default options (an empty buffer of
/// options) and gives the status, the description and the module.
///
/// # Panics
///
/// Panics when the interface gives a result which is not UTF-8.
fn through_the_interface(sbml: &str) -> (u32, String, Vec<u8>) {
    through_the_interface_with_options(sbml, "")
}

/// [`through_the_interface`] with the options of the JSON `options`.
fn through_the_interface_with_options(sbml: &str, options: &str) -> (u32, String, Vec<u8>) {
    // SAFETY: the buffers are the ones of the interface
    through(
        sbml,
        options,
        |input, len, options, options_len, words| unsafe {
            compile(input, len, options, options_len, words)
        },
    )
}

/// [`through_the_interface_with_options`] of `compile_kernel`.
fn kernel_through_the_interface(sbml: &str, options: &str) -> (u32, String, Vec<u8>) {
    // SAFETY: the buffers are the ones of the interface
    through(
        sbml,
        options,
        |input, len, options, options_len, words| unsafe {
            compile_kernel(input, len, options, options_len, words)
        },
    )
}

/// A function of the interface: `call(input, len, options, options_len, words)`.
type Call = fn(*const u8, usize, *const u8, usize, *mut usize) -> u32;

/// A document through a function of the interface, which writes its result to four words: the
/// status, the description (or the message) and the module. Empty options are a null pointer
/// of the length 0, as the JavaScript host passes none.
fn through(sbml: &str, options: &str, call: Call) -> (u32, String, Vec<u8>) {
    let bytes = sbml.as_bytes();
    let options = options.as_bytes();
    // SAFETY: the buffers are the ones of the interface and are freed below
    unsafe {
        let input = allocate(bytes.len());
        if !bytes.is_empty() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), input, bytes.len());
        }
        let config = allocate(options.len());
        if !options.is_empty() {
            std::ptr::copy_nonoverlapping(options.as_ptr(), config, options.len());
        }
        let words = allocate(4 * size_of::<usize>()).cast::<usize>();
        let status = call(input, bytes.len(), config, options.len(), words);
        let description = bytes_of(*words, *words.add(1));
        let wasm = bytes_of(*words.add(2), *words.add(3));
        deallocate(*words as *mut u8, *words.add(1));
        deallocate(*words.add(2) as *mut u8, *words.add(3));
        deallocate(words.cast::<u8>(), 4 * size_of::<usize>());
        deallocate(config, options.len());
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
fn parameter_derivatives_are_optional() {
    let (status, description, wasm) =
        through_the_interface_with_options(MODEL, r#"{"parameter_jacobian": false}"#);
    assert_eq!(status, 1, "{description}");
    assert!(wasm.starts_with(b"\0asm"));
    let description: serde_json::Value = serde_json::from_str(&description).unwrap();
    assert!(description.get("jacobian_p").is_none());
}

/// Every option of `compile` gives the module of the compiler with the same options, and the
/// defaults are the empty buffer and `{}`.
#[test]
fn the_options_are_those_of_the_compiler() {
    let defaults = through_the_interface(MODEL);
    assert_eq!(through_the_interface_with_options(MODEL, "{}"), defaults);
    let expected = sbmlsim_wasm::compile_model(MODEL, &ModelOptions::default()).unwrap();
    assert_eq!(
        (defaults.1, defaults.2),
        (expected.description, expected.wasm)
    );
    for (json, options) in [
        (
            r#"{"optimize": false}"#,
            ModelOptions {
                optimize: false,
                ..ModelOptions::default()
            },
        ),
        (
            r#"{"jacobian": "difference_quotient"}"#,
            ModelOptions {
                jacobian: sbmlsim_abi::JacobianChoice::DifferenceQuotient,
                ..ModelOptions::default()
            },
        ),
        (
            r#"{"driven": ["s"], "parameter_jacobian": false}"#,
            ModelOptions {
                driven: vec!["s".into()],
                parameter_jacobian: false,
                ..ModelOptions::default()
            },
        ),
    ] {
        let (status, description, wasm) = through_the_interface_with_options(MODEL, json);
        assert_eq!(status, 1, "{json}: {description}");
        let expected = sbmlsim_wasm::compile_model(MODEL, &options).unwrap();
        assert_eq!(
            (description, wasm),
            (expected.description, expected.wasm),
            "{json}"
        );
    }
    // the driven species of the description are those of the compiler
    let (_, description, _) = through_the_interface_with_options(MODEL, r#"{"driven": ["s"]}"#);
    let description: sbmlsim_abi::ModelDescription = serde_json::from_str(&description).unwrap();
    let options = sbmlsim_compiler::CompileOptions {
        driven: vec!["s".into()],
        ..sbmlsim_compiler::CompileOptions::default()
    };
    let compiled = sbmlsim_compiler::compile(MODEL, &options).unwrap();
    assert_eq!(description, compiled.description);
    assert_eq!(description.driven[0].id, "s");
}

/// Options which are not those of the JSON, and a driven species the compiler refuses, give
/// their message and no module.
#[test]
fn invalid_options_and_refused_species_give_their_message() {
    for options in [
        r#"{"driven": "s"}"#,
        r#"{"jacobian": "exact"}"#,
        r#"{"other": 1}"#,
        "{",
    ] {
        let (status, message, wasm) = through_the_interface_with_options(MODEL, options);
        assert_eq!(status, 0, "{options}");
        assert!(wasm.is_empty());
        assert!(
            message.starts_with("the options of the compilation are not valid"),
            "{options}: {message}"
        );
        // the options of a kernel are `driven` alone
        let (status, message, wasm) = kernel_through_the_interface(MODEL, options);
        assert_eq!(status, 0, "{options}");
        assert!(wasm.is_empty());
        assert!(
            message.starts_with("the options of the compilation are not valid"),
            "{options}: {message}"
        );
    }
    let options = sbmlsim_compiler::CompileOptions {
        driven: vec!["Q".into()],
        ..sbmlsim_compiler::CompileOptions::default()
    };
    let expected = sbmlsim_compiler::compile(MODEL, &options)
        .unwrap_err()
        .to_string();
    let (status, message, wasm) = through_the_interface_with_options(MODEL, r#"{"driven": ["Q"]}"#);
    assert_eq!((status, wasm.len()), (0, 0));
    assert_eq!(message, expected);
    let (status, message, wasm) = kernel_through_the_interface(MODEL, r#"{"driven": ["Q"]}"#);
    assert_eq!((status, wasm.len()), (0, 0));
    assert_eq!(message, expected);
}

#[test]
fn a_kernel_gives_its_module_and_its_description() {
    let (status, description, wasm) = kernel_through_the_interface(MODEL, "");
    assert_eq!(status, 1, "{description}");
    assert!(wasm.starts_with(b"\0asm"));
    let value: serde_json::Value = serde_json::from_str(&description).unwrap();
    assert_eq!(value["version"], "kernel/2");
    assert_eq!(value["state_ids"], serde_json::json!(["s"]));
    assert_eq!(value["driven"], serde_json::json!([]));
    assert_eq!(value["model"]["id"], "decay");
    let compiled = sbmlsim_wasm::compile_kernel_model(MODEL, &KernelOptions::default()).unwrap();
    assert_eq!((compiled.description, compiled.wasm), (description, wasm));
    assert_eq!(kernel_through_the_interface(MODEL, "{}").0, 1);
    // a model with an event is refused with the message of the code generation
    let event = MODEL.replace(
        "</listOfReactions>",
        r#"</listOfReactions><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math xmlns="http://www.w3.org/1998/Math/MathML"><true/></math></trigger></event></listOfEvents>"#,
    );
    let (status, message, wasm) = kernel_through_the_interface(&event, "");
    assert_eq!(status, 0);
    assert!(wasm.is_empty());
    assert!(message.contains("events"), "{message}");
}

/// A driven kernel through the interface: the kernel of the compiler with the same species
/// driven, whose description names them.
#[test]
fn a_driven_kernel_gives_its_module_and_its_description() {
    let (status, description, wasm) = kernel_through_the_interface(MODEL, r#"{"driven": ["s"]}"#);
    assert_eq!(status, 1, "{description}");
    let options = KernelOptions {
        driven: vec!["s".into()],
    };
    let compiled = sbmlsim_compiler::compile_kernel(MODEL, &options).unwrap();
    assert_eq!(wasm, compiled.wasm);
    let description: sbmlsim_abi::KernelDescription = serde_json::from_str(&description).unwrap();
    assert_eq!(description, compiled.description);
    assert_eq!(description.state_ids, ["s__exchange"]);
    assert_eq!(description.driven[0].id, "s");
    assert_eq!(
        description.parameter_ids[description.t_ref.unwrap() as usize],
        "driven__t_ref"
    );
}

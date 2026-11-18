//! The wire formats of the contracts as golden files: `testdata/golden/model.json` (the
//! description `model/2` the compiler writes for `testdata/golden/model.xml`) and
//! `testdata/golden/request.json` (a request `simulation/1` with every field). Both are read and
//! written back byte for byte, the request runs on the model, and a description or a request of
//! another contract is refused by every reader (the engine, the host; the loader in
//! `js/test/golden.test.mjs`), and so is an `engine.wasm` of another output format.
//!
//! `SBMLSIM_CORE_BLESS=1` writes `model.json` again (a change of the format is a change of the
//! contract: its version or a revision note in `crates/abi`).

use std::path::PathBuf;

use sbmlsim_abi::{DEFAULT_SEED, ModelDescription, SimulationRequest};
use sbmlsim_compiler::{COMPILER, CompileOptions, compile};
use sbmlsim_engine::EngineError;
use sbmlsim_host::Runtime;

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/golden")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(golden().join(name)).unwrap()
}

/// Pretty JSON with a final newline, as the golden files are written.
fn pretty<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap() + "\n"
}

#[test]
fn the_golden_description_is_the_one_the_compiler_writes_and_roundtrips() {
    let compiled = compile(&read("model.xml"), &CompileOptions::default()).unwrap();
    let written = pretty(&compiled.description);
    if std::env::var_os("SBMLSIM_CORE_BLESS").is_some() {
        std::fs::write(golden().join("model.json"), &written).unwrap();
    }
    let text = read("model.json");
    assert_eq!(
        written, text,
        "model.json is stale, write it again with SBMLSIM_CORE_BLESS=1"
    );
    let desc: ModelDescription = serde_json::from_str(&text).unwrap();
    assert_eq!(desc, compiled.description);
    assert_eq!(pretty(&desc), text);
    assert_eq!(desc.compiler, COMPILER);
    assert!(desc.compiler.starts_with("sbmlsim-compiler "));
    // the model has every part of the format: species, a compartment, rules, a reaction, an
    // event with a delay and a priority, a root function of a discontinuity, units
    assert_eq!(desc.events.len(), 1);
    assert!(desc.events[0].has_delay && desc.events[0].has_priority);
    assert_eq!(desc.n_discontinuity_roots, 1);
    assert!(!desc.units.is_empty());
}

#[test]
fn the_golden_request_roundtrips_and_runs() {
    let text = read("request.json");
    let req: SimulationRequest = serde_json::from_str(&text).unwrap();
    assert_eq!(pretty(&req), text);
    // without a seed: 1
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("seed");
    let unseeded: SimulationRequest = serde_json::from_value(value).unwrap();
    assert_eq!(unseeded.seed, DEFAULT_SEED);
    assert_eq!(DEFAULT_SEED, 1);
    assert_eq!(unseeded, req);

    let compiled = compile(&read("model.xml"), &CompileOptions::default()).unwrap();
    let runtime = Runtime::embedded().unwrap();
    let mut instance = runtime
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(&req).unwrap();
    // the five output times and the two rows of the change at 5
    assert_eq!(result.n_rows, 6);
    assert_eq!(result.columns, ["time", "S", "[S]", "P", "x"]);
    // the same request again on the instance: the same bits
    let again = instance.simulate(&req).unwrap();
    assert_eq!(
        result
            .values
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        again.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn another_contract_is_refused_by_the_engine_and_the_host() {
    let compiled = compile(&read("model.xml"), &CompileOptions::default()).unwrap();
    let runtime = Runtime::embedded().unwrap();
    // a description of another contract: the host refuses it before the engine sees it, the
    // engine refuses it too
    let desc: ModelDescription =
        serde_json::from_str(&read("model.json").replace("\"model/2\"", "\"model/1\"")).unwrap();
    let message = runtime
        .instantiate(&compiled.wasm, &desc)
        .err()
        .unwrap()
        .to_string();
    assert!(
        message.contains("model/1") && message.contains("model/2"),
        "{message}"
    );
    let decay = sbmlsim_engine::model::Decay { k: 0.5 };
    let mut decay_desc = decay.description();
    decay_desc.contract = "model/1".to_string();
    match sbmlsim_engine::Engine::new(Box::new(decay), decay_desc) {
        Err(EngineError::Request(m)) => assert!(m.contains("model/1"), "{m}"),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("model/1 accepted"),
    }
    // a request of another contract: the engine in the host and the native engine
    let req: SimulationRequest =
        serde_json::from_str(&read("request.json").replace("\"simulation/1\"", "\"simulation/2\""))
            .unwrap();
    let mut instance = runtime
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    match instance.simulate(&req) {
        Err(EngineError::Request(m)) => {
            assert!(
                m.contains("simulation/2") && m.contains("simulation/1"),
                "{m}"
            )
        }
        other => panic!("{other:?}"),
    }
    let decay = sbmlsim_engine::model::Decay { k: 0.5 };
    match sbmlsim_engine::simulate(&decay, &decay.description(), &req) {
        Err(EngineError::Request(m)) => assert!(m.contains("simulation/2"), "{m}"),
        other => panic!("{other:?}"),
    }
}

/// `engine.wasm` with its output format `2` replaced by `format` (one digit).
fn engine_with_format(format: u8) -> Vec<u8> {
    let needle = br#""output_format":2}"#;
    let mut bytes = sbmlsim_host::ENGINE_WASM.to_vec();
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("engine.wasm declares its output format");
    bytes[at + needle.len() - 2] = format;
    bytes
}

#[test]
fn an_engine_of_another_output_format_is_refused() {
    let mathlib = sbmlsim_host::MATHLIB_WASM;
    Runtime::new(&engine_with_format(b'2'), mathlib).unwrap();
    let message = Runtime::new(&engine_with_format(b'1'), mathlib)
        .err()
        .unwrap()
        .to_string();
    assert!(
        message.contains("output format 1") && message.contains("reads 2"),
        "{message}"
    );
    // a second section of the name: refused as well
    let mut twice = sbmlsim_host::ENGINE_WASM.to_vec();
    let payload = br#"{"memory_minimum_pages":17,"output_format":2}"#;
    let name = sbmlsim_abi::ENGINE_SECTION.as_bytes();
    twice.push(0);
    twice.push((1 + name.len() + payload.len()) as u8);
    twice.push(name.len() as u8);
    twice.extend_from_slice(name);
    twice.extend_from_slice(payload);
    let message = Runtime::new(&twice, mathlib).err().unwrap().to_string();
    assert!(message.contains("2 custom sections"), "{message}");
}

#[test]
fn a_model_without_a_function_of_the_contract_is_refused() {
    // the export model_rhs renamed to model_rhx (the same length): the engine does not
    // instantiate with it (the loader gives the same message instead of a raw LinkError)
    let compiled = compile(&read("model.xml"), &CompileOptions::default()).unwrap();
    let needle = b"model_rhs";
    let mut wasm = compiled.wasm.clone();
    let at = wasm
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap();
    wasm[at + needle.len() - 1] = b'x';
    let error = Runtime::embedded()
        .unwrap()
        .instantiate(&wasm, &compiled.description)
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(
        message.starts_with("engine.wasm does not instantiate with the model: "),
        "{message}"
    );
}

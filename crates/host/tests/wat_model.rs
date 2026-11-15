//! The chain mathlib -> model -> engine in wasmtime, with the hand-written model `decay.wat`,
//! against the native test model `Decay` of the engine.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sbmlsim_abi::{
    Change, IntegratorSettings, ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest,
    SimulationResult,
};
use sbmlsim_engine::model::Decay;
use sbmlsim_engine::{EngineError, simulate};
use sbmlsim_host::{ENGINE_WASM, Instance, MATHLIB_WASM, Runtime};

/// The runtime of the embedded engine and mathlib, compiled once for the tests.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new(ENGINE_WASM, MATHLIB_WASM).unwrap())
}

/// The directory of the tests.
fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// `decay.wat` as text.
fn decay_wat() -> String {
    std::fs::read_to_string(tests_dir().join("decay.wat")).unwrap()
}

/// `model.wasm` of `decay.wat`.
fn decay_wasm() -> Vec<u8> {
    wat::parse_str(decay_wat()).unwrap()
}

/// The native decay model of `decay.wat`.
fn native() -> Decay {
    Decay { k: 0.5 }
}

/// The description of the decay model.
fn description() -> ModelDescription {
    native().description()
}

/// An instance of the decay model.
fn instance() -> Instance {
    runtime()
        .instantiate(&decay_wasm(), &description())
        .unwrap()
}

/// The request of `decay_uniform_output` (`crates/engine/tests/basic.rs`): `0` to `10` in 10
/// uniform steps, the columns `time` and `y`, `rtol 1e-10`, the non-stiff method.
fn request() -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 10.0,
        output: Output::Uniform { steps: 10 },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: vec!["time".to_string(), "y".to_string()],
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            stiff: false,
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The native result of a request.
fn native_result(req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
    simulate(&native(), &description(), req)
}

/// Asserts that `wasm` is `native` within `1e-12` relative and prints whether it is bitwise.
fn assert_close(wasm: &SimulationResult, native: &SimulationResult) {
    assert_eq!(wasm.columns, native.columns);
    assert_eq!(wasm.n_rows, native.n_rows);
    assert_eq!(wasm.values.len(), native.values.len());
    let mut bitwise = true;
    let mut largest: f64 = 0.0;
    for (w, n) in wasm.values.iter().zip(&native.values) {
        bitwise &= w.to_bits() == n.to_bits();
        let difference = if w == n { 0.0 } else { ((w - n) / n).abs() };
        largest = largest.max(difference);
        assert!(difference <= 1e-12, "wasm {w:e} and native {n:e}");
    }
    println!("wasm and native: bitwise equal {bitwise}, largest relative difference {largest:e}");
}

#[test]
fn wat_decay_matches_native() {
    let req = request();
    let wasm = instance().simulate(&req).unwrap();
    assert_eq!(wasm.columns, ["time", "y"]);
    assert_eq!(wasm.n_rows, 11);
    assert_close(&wasm, &native_result(&req).unwrap());
    // the other selections, the assignment rule through math_log, and BDF with the analytic
    // Jacobian (dense and KLU) of the model
    for (stiff, solver) in [
        (false, sbmlsim_abi::LinearSolver::Auto),
        (true, sbmlsim_abi::LinearSolver::Dense),
        (true, sbmlsim_abi::LinearSolver::Klu),
    ] {
        let mut req = request();
        req.selections = vec![];
        req.settings.stiff = stiff;
        req.settings.linear_solver = solver;
        req.preinit = vec![("k".to_string(), 0.25)];
        let wasm = instance().simulate(&req).unwrap();
        assert_eq!(wasm.columns, ["time", "y", "t_half"]);
        assert_close(&wasm, &native_result(&req).unwrap());
    }
}

#[test]
fn an_instance_simulates_again_bitwise() {
    let mut instance = instance();
    let first = instance.simulate(&request()).unwrap();
    let mut other = request();
    other.output = Output::IntegratorSteps;
    other.settings.stiff = true;
    let steps = instance.simulate(&other).unwrap();
    assert_close(&steps, &native_result(&other).unwrap());
    let again = instance.simulate(&request()).unwrap();
    assert_eq!(first.columns, again.columns);
    let bits = |r: &SimulationResult| r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&first), bits(&again));
}

#[test]
fn instances_on_threads_agree() {
    let expected = instance().simulate(&request()).unwrap();
    let results: Vec<SimulationResult> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| instance().simulate(&request()).unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for result in results {
        assert_eq!(result, expected);
    }
}

#[test]
fn errors_of_the_engine_come_back() {
    let mut instance = instance();
    // integration: too few steps for rtol 1e-10
    let mut req = request();
    req.settings.maximum_num_steps = 5;
    let wasm = instance.simulate(&req).unwrap_err();
    assert_eq!(wasm, native_result(&req).unwrap_err());
    match &wasm {
        EngineError::Integration {
            flag,
            time,
            message,
        } => {
            assert_eq!(*flag, -1);
            assert!(*time > 0.0 && *time < 10.0, "{time}");
            assert!(message.contains("mxstep"), "{message}");
        }
        other => panic!("not an integration error: {other:?}"),
    }
    // request: no steps
    let mut req = request();
    req.output = Output::Uniform { steps: 0 };
    let wasm = instance.simulate(&req).unwrap_err();
    assert!(matches!(wasm, EngineError::Request(_)), "{wasm:?}");
    assert_eq!(wasm, native_result(&req).unwrap_err());
    // request: another contract
    let mut req = request();
    req.contract = "simulation/0".to_string();
    let wasm = instance.simulate(&req).unwrap_err();
    assert_eq!(wasm, native_result(&req).unwrap_err());
    // unsupported: a change
    let mut req = request();
    req.changes = vec![Change {
        time: 5.0,
        values: vec![("k".to_string(), 1.0)],
    }];
    let wasm = instance.simulate(&req).unwrap_err();
    assert!(matches!(wasm, EngineError::Unsupported(_)), "{wasm:?}");
    assert_eq!(wasm, native_result(&req).unwrap_err());
    // and the instance still simulates
    assert_close(
        &instance.simulate(&request()).unwrap(),
        &native_result(&request()).unwrap(),
    );
}

#[test]
fn descriptions_are_checked() {
    let mut desc = description();
    desc.contract = "model/0".to_string();
    let message = runtime()
        .instantiate(&decay_wasm(), &desc)
        .err()
        .unwrap()
        .to_string();
    assert!(
        message.contains("model/0") && message.contains("model/1"),
        "{message}"
    );
    // refused by engine_load: a Jacobian pattern of two states for one state
    let mut desc = description();
    if let sbmlsim_abi::JacobianKind::Analytic(pattern) = &mut desc.jacobian {
        pattern.n = 2;
    }
    let message = format!(
        "{:#}",
        runtime().instantiate(&decay_wasm(), &desc).err().unwrap()
    );
    assert!(
        message.contains("not a CSC pattern of 1 states"),
        "{message}"
    );
}

#[test]
fn models_with_other_imports_are_refused() {
    let wat = decay_wat().replace(
        "(import \"math\" \"math_exp\"",
        "(import \"wasi_snapshot_preview1\" \"proc_exit\" (func (param i32)))\n  \
         (import \"math\" \"math_exp\"",
    );
    let message = format!(
        "{:#}",
        runtime()
            .instantiate(&wat::parse_str(wat).unwrap(), &description())
            .err()
            .unwrap()
    );
    assert!(
        message.contains("wasi_snapshot_preview1.proc_exit"),
        "{message}"
    );
}

/// The error of instantiating a model.
fn refusal(wasm: &[u8]) -> String {
    format!(
        "{:#}",
        runtime().instantiate(wasm, &description()).err().unwrap()
    )
}

/// `decay.wat` with `extra` after its imports.
fn decay_with(extra: &str) -> Vec<u8> {
    let imports = "(import \"math\" \"math_log\" (func $log (param f64) (result f64)))";
    let wat = decay_wat();
    assert!(wat.contains(imports));
    wat::parse_str(wat.replace(imports, &format!("{imports}\n  {extra}"))).unwrap()
}

/// `decay.wat` with a memory of its own instead of `env.memory`.
fn own_memory_wat() -> String {
    let imports = "(import \"math\" \"math_log\" (func $log (param f64) (result f64)))";
    decay_wat()
        .replace("(import \"env\" \"memory\" (memory 0))", "")
        .replace(imports, &format!("{imports}\n  (memory 1)"))
}

#[test]
fn models_with_state_or_memory_of_their_own_are_refused() {
    let cases = [
        (
            decay_with("(data (i32.const 0) \"x\")"),
            "has a data segment",
        ),
        (decay_with("(func $s) (start $s)"), "has a start function"),
        (
            decay_with("(global $g (mut f64) (f64.const 0))"),
            "defines a mutable global",
        ),
        (
            wat::parse_str(own_memory_wat()).unwrap(),
            "defines a memory",
        ),
        (
            wat::parse_str("(module)").unwrap(),
            "does not import env.memory",
        ),
    ];
    for (wasm, expected) in cases {
        let message = refusal(&wasm);
        assert!(message.contains(expected), "{expected}: {message}");
    }
    // an immutable global is no state
    let wasm = decay_with("(global $g f64 (f64.const 0))");
    runtime().instantiate(&wasm, &description()).unwrap();
}

#[test]
fn math_imports_are_functions_of_mathlib() {
    // the memory mathlib exports is no function of mathlib
    let message = refusal(&decay_with(
        "(import \"math\" \"memory\" (func (param f64) (result f64)))",
    ));
    assert!(
        message.contains("math.memory, which is no function of mathlib"),
        "{message}"
    );
    let message = refusal(&decay_with("(import \"math\" \"memory\" (memory 0))"));
    assert!(message.contains("math.memory"), "{message}");
    // a function of mathlib with another signature
    let message = refusal(&decay_with(
        "(import \"math\" \"math_pow\" (func (param f64) (result f64)))",
    ));
    assert!(message.contains("mathlib exports it as"), "{message}");
    let message = refusal(&decay_with(
        "(import \"math\" \"math_nothing\" (func (param f64) (result f64)))",
    ));
    assert!(message.contains("math.math_nothing"), "{message}");
}

#[test]
fn a_trap_poisons_the_instance() {
    let rhs = "(f64.store (local.get $ydot)\n      \
               (f64.mul (f64.neg (f64.load (local.get $p))) (f64.load (local.get $y))))";
    let wat = decay_wat();
    assert!(wat.contains(rhs));
    let model = wat::parse_str(wat.replace(rhs, "unreachable")).unwrap();
    let mut instance = runtime().instantiate(&model, &description()).unwrap();
    let error = instance.simulate(&request()).unwrap_err();
    match &error {
        EngineError::Internal(message) => {
            println!("{message}");
            assert!(message.contains("unreachable"), "{message}");
            assert!(message.contains("ModelFns>::rhs"), "{message}");
        }
        other => panic!("not an internal error: {other:?}"),
    }
    let error = instance.simulate(&request()).unwrap_err();
    match &error {
        EngineError::Internal(message) => assert!(message.contains("trapped before"), "{message}"),
        other => panic!("not an internal error: {other:?}"),
    }
}

/// R15: no WASI call during `engine_load` and `engine_simulate`.
///
/// Every import of the engine which the host satisfies is a function of WASI preview1 (the
/// model and mathlib are linked as instances, their calls never leave wasm), so the calls of the
/// engine into the host, which the call hook of the store counts, are its WASI calls.
#[test]
fn no_wasi_calls_during_simulate() {
    let mut instance = instance();
    println!(
        "WASI calls during _initialize: {}, during engine_load: {}",
        instance.initialize_wasi_calls(),
        instance.load_wasi_calls()
    );
    assert_eq!(instance.load_wasi_calls(), 0);
    let before = instance.wasi_calls();
    // a successful simulation
    instance.simulate(&request()).unwrap();
    // an integration error
    let mut req = request();
    req.settings.maximum_num_steps = 5;
    assert!(matches!(
        instance.simulate(&req),
        Err(EngineError::Integration { .. })
    ));
    // a refused request
    let mut req = request();
    req.output = Output::Uniform { steps: 0 };
    assert!(matches!(
        instance.simulate(&req),
        Err(EngineError::Request(_))
    ));
    assert_eq!(instance.wasi_calls() - before, 0);
}

/// The call hook counts a WASI call when one happens (the control of the test above).
#[test]
fn the_wasi_count_sees_a_call() {
    let wat = r#"(module
        (import "wasi_snapshot_preview1" "fd_write"
          (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "write") (result i32)
          ;; no iovecs: writes nothing to stdout
          (call $fd_write (i32.const 1) (i32.const 0) (i32.const 0) (i32.const 64))))"#;
    let calls = sbmlsim_host::count_wasi_calls(&wat::parse_str(wat).unwrap(), "write").unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn runtime_is_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Runtime>();
    fn send<T: Send>() {}
    send::<Instance>();
}

/// The model directory of the command line example is the one of `decay.wat`: its
/// `model.wasm` is `decay.wat` compiled, its `model.json` the description and its
/// `request.json` the request of the tests (`SBMLSIM_CORE_BLESS=1` writes them).
#[test]
fn decay_model_dir_is_current() {
    let dir = tests_dir().join("decay-model");
    let model = decay_wasm();
    let desc = serde_json::to_string_pretty(&description()).unwrap() + "\n";
    let req = serde_json::to_string_pretty(&request()).unwrap() + "\n";
    if std::env::var_os("SBMLSIM_CORE_BLESS").is_some() {
        std::fs::write(dir.join("model.wasm"), &model).unwrap();
        std::fs::write(dir.join("model.json"), &desc).unwrap();
        std::fs::write(dir.join("request.json"), &req).unwrap();
    }
    let stale = "is stale, write it again with SBMLSIM_CORE_BLESS=1";
    assert!(
        std::fs::read(dir.join("model.wasm")).unwrap() == model,
        "model.wasm {stale}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("model.json")).unwrap(),
        desc,
        "model.json {stale}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("request.json")).unwrap(),
        req,
        "request.json {stale}"
    );
}

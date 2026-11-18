//! The native engine (`Runtime::instantiate_native`: `sbmlsim-engine` compiled for the machine,
//! the model in wasmtime) against `engine.wasm`: the same rows and columns, values within the
//! tolerances of the integration (not bitwise: the C library of the host in the step sizes of
//! CVODE), on the golden model (events, root functions of discontinuities, constants of the
//! optimizer, changes and a steady state in its request).

use std::path::PathBuf;

use sbmlsim_abi::{LinearSolver, SimulationRequest};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

fn golden(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/golden");
    std::fs::read_to_string(path.join(name)).unwrap()
}

#[test]
fn the_native_engine_agrees_with_engine_wasm() {
    let compiled = compile(&golden("model.xml"), &CompileOptions::default()).unwrap();
    assert!(compiled.description.n_constants > 0);
    let runtime = Runtime::embedded().unwrap();
    let mut wasm = runtime
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let mut native = runtime
        .instantiate_native(&compiled.wasm, &compiled.description)
        .unwrap();
    let base: SimulationRequest = serde_json::from_str(&golden("request.json")).unwrap();
    for solver in [LinearSolver::Dense, LinearSolver::Klu, LinearSolver::Gmres] {
        let mut req = base.clone();
        req.settings.linear_solver = solver;
        req.settings.stiff = true;
        let a = wasm.simulate(&req).unwrap();
        let b = native.simulate(&req).unwrap();
        assert_eq!((&a.columns, a.n_rows), (&b.columns, b.n_rows));
        let rtol = 1e3 * req.settings.relative_tolerance;
        for (x, y) in a.values.iter().zip(&b.values) {
            assert!(
                (x - y).abs() <= 1e-9 + rtol * x.abs().max(y.abs()),
                "{solver:?}: {x} != {y}"
            );
        }
        // a second simulation of the native engine repeats its bits
        let again = native.simulate(&req).unwrap();
        assert!(
            b.values
                .iter()
                .zip(&again.values)
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
        assert!(native.stats().steps > 0);
    }
}

/// A native instance is `Send`, which the python bindings need: a batch moves one into each
/// worker of its pool and keeps it there for the points after the first.
#[test]
fn a_native_instance_moves_between_threads() {
    let compiled = compile(&golden("model.xml"), &CompileOptions::default()).unwrap();
    let runtime = Runtime::embedded().unwrap();
    let loaded = runtime.load(&compiled.wasm, &compiled.description).unwrap();
    let request: SimulationRequest = serde_json::from_str(&golden("request.json")).unwrap();
    let mut instance = loaded.instantiate_native().unwrap();
    let first = instance.simulate(&request).unwrap();
    // the instance is moved into a thread of its own, simulates there and comes back
    let moved = request.clone();
    let (mut instance, second) = std::thread::spawn(move || {
        let result = instance.simulate(&moved).unwrap();
        (instance, result)
    })
    .join()
    .unwrap();
    let third = instance.simulate(&request).unwrap();
    assert_eq!(first.values, second.values);
    assert_eq!(first.values, third.values);
}

/// A loaded model makes native instances which reuse its compiled `model.wasm`, and each one is
/// independent of the others.
#[test]
fn a_loaded_model_makes_independent_native_instances() {
    let compiled = compile(&golden("model.xml"), &CompileOptions::default()).unwrap();
    let runtime = Runtime::embedded().unwrap();
    let loaded = runtime.load(&compiled.wasm, &compiled.description).unwrap();
    let request: SimulationRequest = serde_json::from_str(&golden("request.json")).unwrap();
    let one = loaded
        .instantiate_native()
        .unwrap()
        .simulate(&request)
        .unwrap();
    let other = loaded
        .instantiate_native()
        .unwrap()
        .simulate(&request)
        .unwrap();
    assert_eq!(one.values, other.values);
}

/// The memory limit of a runtime bounds the result of a native instance, as the memory of its
/// guest bounds the result of an instance of `engine.wasm`.
#[test]
fn the_memory_limit_bounds_a_native_result() {
    let compiled = compile(&golden("model.xml"), &CompileOptions::default()).unwrap();
    let runtime = Runtime::embedded()
        .unwrap()
        .with_max_memory(32 * 1024 * 1024);
    let loaded = runtime.load(&compiled.wasm, &compiled.description).unwrap();
    let mut instance = loaded.instantiate_native().unwrap();
    let base: SimulationRequest = serde_json::from_str(&golden("request.json")).unwrap();
    let mut huge = base.clone();
    huge.output = sbmlsim_abi::Output::Uniform { steps: 3_000_000 };
    let error = instance.simulate(&huge).unwrap_err();
    assert!(
        format!("{error}").contains("does not fit into the memory"),
        "{error}"
    );
    // the instance answers the request after it
    assert!(instance.simulate(&base).is_ok());
}

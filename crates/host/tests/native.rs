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
    for solver in [LinearSolver::Dense, LinearSolver::Klu] {
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

//! The memory of an instance: a result is recorded in place of its output (no second copy), the
//! output is aligned to 8, and a result which does not fit into the memory is an error of the
//! request and not an abort of the allocator of the engine.

use std::path::Path;

use sbmlsim_abi::{
    IntegratorSettings, ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest,
};
use sbmlsim_engine::EngineError;
use sbmlsim_host::{Instance, MAX_MEMORY_BYTES, Runtime};

/// The decay model `y' = -k y` (`crates/host/tests/decay-model`).
fn decay(runtime: &Runtime) -> Instance {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay-model");
    let wasm = std::fs::read(dir.join("model.wasm")).unwrap();
    let desc: ModelDescription =
        serde_json::from_slice(&std::fs::read(dir.join("model.json")).unwrap()).unwrap();
    runtime.instantiate(&wasm, &desc).unwrap()
}

fn request(end: f64, output: Output) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: vec!["time".to_string(), "y".to_string()],
        settings: IntegratorSettings {
            stiff: false,
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The largest result is recorded in place of its output: after a result of 64 MiB the memory
/// has grown by the result and its output times (half of it, 2 columns), not by a second copy.
#[test]
fn a_result_is_its_output_without_a_copy() {
    let runtime = Runtime::embedded().unwrap();
    let mut instance = decay(&runtime);
    instance
        .simulate(&request(1.0, Output::Uniform { steps: 10 }))
        .unwrap();
    let before = instance.memory_bytes();
    let rows = 1 << 22;
    let result = instance
        .simulate(&request(1.0, Output::Uniform { steps: rows - 1 }))
        .unwrap();
    assert_eq!(result.n_rows, rows as usize);
    let bytes = 8 * result.values.len();
    let grown = instance.memory_bytes() - before;
    eprintln!(
        "a result of {} MiB grows the memory by {} MiB",
        bytes >> 20,
        grown >> 20
    );
    // the values and the output times (half of them); a copy of the values would be 2.5 times
    assert!(grown <= bytes * 8 / 5, "{grown} > 1.6 * {bytes}");
    assert_eq!(instance.output_address().unwrap() % 8, 0);
}

/// Every output starts at an address aligned to 8, so a JavaScript host reads its values
/// through a `Float64Array` without a copy.
#[test]
fn every_output_is_aligned_to_8() {
    let runtime = Runtime::embedded().unwrap();
    let mut instance = decay(&runtime);
    for (steps, selections) in [
        (1, vec!["time"]),
        (3, vec!["time", "y"]),
        (7, vec!["y"]),
        (100, vec!["time", "y", "t_half"]),
    ] {
        let mut req = request(1.0, Output::Uniform { steps });
        req.selections = selections.iter().map(|s| s.to_string()).collect();
        instance.simulate(&req).unwrap();
        assert_eq!(instance.output_address().unwrap() % 8, 0);
    }
    // an error
    let mut req = request(1.0, Output::Uniform { steps: 1 });
    req.selections = vec!["unknown".to_string()];
    assert!(matches!(
        instance.simulate(&req),
        Err(EngineError::Request(_))
    ));
    assert_eq!(instance.output_address().unwrap() % 8, 0);
}

/// A result which does not fit into the memory (here of at most 32 MiB) is the error `request`:
/// the engine allocates it with `try_reserve` and calls nothing of WASI (its allocator would
/// write "memory allocation failed" to stderr and abort), and the instance simulates the next
/// request.
#[test]
fn a_result_without_memory_is_an_error_of_the_request() {
    let runtime = Runtime::embedded().unwrap().with_max_memory(32 << 20);
    let mut instance = decay(&runtime);
    let small = request(1.0, Output::Uniform { steps: 10 });
    let first = instance.simulate(&small).unwrap();
    let calls = instance.wasi_calls();
    // an output of times: 3 million rows of 2 columns, 48 MB
    let error = instance
        .simulate(&request(1.0, Output::Uniform { steps: 3_000_000 }))
        .unwrap_err();
    match &error {
        EngineError::Request(message) => {
            assert!(
                message.contains("does not fit into the memory"),
                "{message}"
            )
        }
        other => panic!("{other:?}"),
    }
    // the steps of the integrator, which grow the result by doubling
    let mut steps = request(1e7, Output::IntegratorSteps);
    steps.settings.maximum_time_step = 1.0;
    steps.settings.maximum_num_steps = 100_000_000;
    let error = instance.simulate(&steps).unwrap_err();
    assert!(
        matches!(&error, EngineError::Request(m) if m.contains("does not fit into the memory")),
        "{error:?}"
    );
    assert_eq!(instance.wasi_calls(), calls);
    assert!(instance.memory_bytes() <= 32 << 20);
    let again = instance.simulate(&small).unwrap();
    assert_eq!(again, first);
}

#[test]
fn the_memory_limit_is_2_gib() {
    assert_eq!(MAX_MEMORY_BYTES, 32768 * 65536);
}

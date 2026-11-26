//! The stepper: an instance of the engine advanced from a given state, against one simulation
//! of the same model by the WebAssembly engine through the host.
//!
//! The stepper advances the native model of the C target; the cell model with the 50 parameter
//! sets and the cases of the suite are the ones of the tests of the C target
//! (`crates/c/tests/common`). The tests over `testdata/cases` skip with a note when the
//! directory is absent, unless `SBMLSIM_CORE_REQUIRE_CASES` is set. They run natively only: the
//! C target and the host are no crates of `wasm32`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../../c/tests/common/mod.rs"]
mod common;

use std::sync::OnceLock;
use std::time::Instant;

use common::{ALGEBRAIC_CASES, Case, EVENT_CASES, cases, cell_model, cell_parameter_sets};
use sbmlsim_abi::{
    Entity, EntityKind, IntegratorSettings, ModelDescription, Output, SIMULATION_CONTRACT,
    SimulationRequest, Tolerance,
};
use sbmlsim_c::{CModel, Library, generate};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_engine::{EngineError, Stepper};
use sbmlsim_host::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

/// The native model of the C target and its description.
fn native(sbml: &str) -> (CModel, ModelDescription) {
    let source = generate(sbml).unwrap();
    let library = Library::build(&source.code).unwrap();
    let model = CModel::new(library, &source.description).unwrap();
    (model, source.description)
}

/// The WebAssembly model of the options which give the description of the C target.
fn wasm(sbml: &str) -> CompiledModel {
    let options = CompileOptions {
        parameter_jacobian: false,
        ..CompileOptions::default()
    };
    compile(sbml, &options).unwrap()
}

/// The tolerances of the advances and the simulations of the cell model: a hundred times those
/// of the tests of the C target. Every advance starts the integrator again, whose first steps
/// at the first order each add an error at the tolerance, so the 1440 advances of 1 min at the
/// tolerances of the C target (1e-10, 1e-12) move the toxic product by up to 1.6e-6 of itself
/// from the simulation, beyond the `1e-6 |y| + 1e-9` of the comparison.
fn settings() -> IntegratorSettings {
    IntegratorSettings {
        relative_tolerance: 1e-12,
        absolute_tolerance: Tolerance::Scalar(1e-14),
        ..IntegratorSettings::default()
    }
}

/// The selection of the value a state stores: the concentration `[S]` of a species which
/// stores it (the target of a rule), else the id (the amount of a species).
fn stored(state: &Entity) -> String {
    match state.kind {
        EntityKind::Species {
            stores_concentration: true,
            ..
        } => format!("[{}]", state.id),
        _ => state.id.clone(),
    }
}

/// The states of one simulation of the WebAssembly model from `times[0]` to the last of
/// `times`, at those times: one row per time.
fn simulate(
    compiled: &CompiledModel,
    preinit: Vec<(String, f64)>,
    times: &[f64],
    settings: IntegratorSettings,
) -> Vec<Vec<f64>> {
    let states: Vec<String> = compiled.description.states.iter().map(stored).collect();
    let req = SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_string(),
        start: times[0],
        end: *times.last().unwrap(),
        output: Output::Times(times.to_vec()),
        preinit,
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: std::iter::once("time".to_string())
            .chain(states.iter().cloned())
            .collect(),
        settings,
        seed: 1,
    };
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(&req).unwrap();
    let k = result.columns.len();
    assert_eq!(result.n_rows, times.len());
    result
        .values
        .chunks(k)
        .map(|row| row[1..].to_vec())
        .collect()
}

/// Checks a state of the stepper against the state of the simulation, within
/// `1e-6 |y| + 1e-9`.
fn check(state: &[f64], expected: &[f64], what: &dyn Fn() -> String) {
    assert_eq!(state.len(), expected.len());
    for (k, (a, b)) in state.iter().zip(expected).enumerate() {
        assert!(
            (a - b).abs() <= 1e-6 * b.abs() + 1e-9,
            "{}: state {k}: stepper {a}, simulation {b}",
            what()
        );
    }
}

#[test]
fn advance_matches_simulate() {
    let sbml = cell_model();
    let (model, description) = native(&sbml);
    let compiled = wasm(&sbml);
    let times: Vec<f64> = (0..=1440).map(f64::from).collect();
    for (set, preinit) in cell_parameter_sets().into_iter().enumerate() {
        let expected = simulate(&compiled, preinit.clone(), &times, settings());
        for dt in [1, 10, 60] {
            let mut stepper = Stepper::new(
                Box::new(model.clone()),
                description.clone(),
                settings(),
                &preinit,
            )
            .unwrap();
            check(stepper.state(), &expected[0], &|| {
                format!("set {set}, start")
            });
            let mut steps = 0;
            for t0 in (0..1440).step_by(dt) {
                let t1 = t0 + dt;
                let y = stepper.state().to_vec();
                steps += stepper.advance(&y, t0 as f64, t1 as f64).unwrap().steps;
                check(stepper.state(), &expected[t1], &|| {
                    format!("set {set} {preinit:?}, steps of {dt}, at {t1}")
                });
            }
            assert!(steps > 0);
        }
    }
}

#[test]
fn advance_from_changed_state() {
    let sbml = cell_model();
    let (model, description) = native(&sbml);
    let compiled = wasm(&sbml);
    let s_ext = description
        .states
        .iter()
        .position(|s| s.id == "S_ext")
        .unwrap();
    for (set, preinit) in cell_parameter_sets().into_iter().enumerate() {
        let mut stepper = Stepper::new(
            Box::new(model.clone()),
            description.clone(),
            settings(),
            &preinit,
        )
        .unwrap();
        let y = stepper.state().to_vec();
        stepper.advance(&y, 0.0, 60.0).unwrap();
        let mut changed = stepper.state().to_vec();
        assert!(changed[s_ext] > 0.0, "set {set}: {changed:?}");
        changed[s_ext] = 0.0;
        stepper.advance(&changed, 60.0, 120.0).unwrap();

        // the simulation from the changed state: the parameters of the set and the states as
        // the initial values of the request
        let initial: Vec<(String, f64)> = preinit
            .iter()
            .filter(|(id, _)| {
                let id = id.trim_start_matches('[').trim_end_matches(']');
                !description.states.iter().any(|s| s.id == id)
            })
            .cloned()
            .chain(
                description
                    .states
                    .iter()
                    .zip(&changed)
                    .map(|(s, &value)| (stored(s), value)),
            )
            .collect();
        let expected = simulate(&compiled, initial, &[60.0, 120.0], settings());
        check(&changed, &expected[0], &|| format!("set {set}, at 60"));
        check(stepper.state(), &expected[1], &|| {
            format!("set {set}, at 120")
        });
    }
}

/// The ends of the 7 uneven steps of a case, as fractions of its duration.
const UNEVEN: [f64; 7] = [0.1, 0.25, 0.43, 0.52, 0.64, 0.81, 1.0];

/// Advances a case of the suite in the 7 uneven steps and compares every step end with one
/// simulation, at the tolerances of the runner of the suite.
fn check_case_in_steps(name: &str) {
    let Some(cases) = cases() else { return };
    let case = Case::read(&cases.join(name));
    let (model, description) = native(&case.sbml);
    let compiled = wasm(&case.sbml);
    let settings = IntegratorSettings {
        relative_tolerance: 1e-12,
        absolute_tolerance: Tolerance::Scalar(1e-12),
        ..IntegratorSettings::default()
    };
    let times: Vec<f64> = std::iter::once(case.start)
        .chain(UNEVEN.iter().map(|f| case.start + f * case.duration))
        .collect();
    let expected = simulate(&compiled, vec![], &times, settings.clone());
    let mut stepper = Stepper::new(Box::new(model), description, settings, &[]).unwrap();
    check(stepper.state(), &expected[0], &|| {
        format!("{name} at the start")
    });
    for (k, pair) in times.windows(2).enumerate() {
        let y = stepper.state().to_vec();
        stepper.advance(&y, pair[0], pair[1]).unwrap();
        check(stepper.state(), &expected[k + 1], &|| {
            format!("{name} at {}", pair[1])
        });
    }
}

#[test]
fn events_inside_interval() {
    for case in EVENT_CASES {
        check_case_in_steps(case);
    }
}

#[test]
fn dae_model_advances() {
    for case in ALGEBRAIC_CASES {
        check_case_in_steps(case);
    }
}

#[test]
fn refusals_leave_the_stepper_usable() {
    let (model, description) = native(&cell_model());
    let n = description.states.len();
    let mut stepper = Stepper::new(
        Box::new(model.clone()),
        description.clone(),
        settings(),
        &[],
    )
    .unwrap();
    let y = stepper.state().to_vec();
    for (y, t0, t1) in [
        (&y[1..], 0.0, 1.0),
        (&y[..], 1.0, 0.0),
        (&y[..], 0.0, f64::NAN),
        (&y[..], f64::INFINITY, f64::INFINITY),
    ] {
        match stepper.advance(y, t0, t1) {
            Err(EngineError::Request(message)) => assert!(!message.is_empty()),
            other => panic!("{} states from {t0} to {t1}: {other:?}", y.len()),
        }
    }
    let mut nan = y.clone();
    nan[0] = f64::NAN;
    assert!(matches!(
        stepper.advance(&nan, 0.0, 1.0),
        Err(EngineError::Request(_))
    ));
    // the refused calls changed nothing
    assert_eq!(stepper.state(), &y[..]);
    stepper.advance(&y, 0.0, 1.0).unwrap();
    assert_eq!(stepper.state().len(), n);

    // an unknown id of the changes and settings which are no settings are refused
    assert!(matches!(
        Stepper::new(
            Box::new(model.clone()),
            description.clone(),
            settings(),
            &[("no such id".to_string(), 1.0)],
        ),
        Err(EngineError::Request(_))
    ));
    let invalid = IntegratorSettings {
        relative_tolerance: -1.0,
        ..settings()
    };
    assert!(matches!(
        Stepper::new(Box::new(model), description, invalid, &[]),
        Err(EngineError::Request(_))
    ));
}

#[test]
fn memory_bytes_counts_the_solver() {
    let (model, description) = native(&cell_model());
    let mut stepper = Stepper::new(Box::new(model), description, settings(), &[]).unwrap();
    let created = stepper.memory_bytes();
    let y = stepper.state().to_vec();
    stepper.advance(&y, 0.0, 1.0).unwrap();
    let advanced = stepper.memory_bytes();
    // CVODE allocates its vectors at the first start: the history of the BDF and seven more,
    // each with its own table of operations, which outweighs its 7 values
    assert!(advanced > created + 13 * 500, "{created} {advanced}");
    // the resident memory of an advanced stepper of the cell model is 19 kB
    assert!((10_000..40_000).contains(&advanced), "{advanced}");
    stepper.advance(&y, 1.0, 2.0).unwrap();
    assert_eq!(stepper.memory_bytes(), advanced);
}

/// The memory and the time of an advance of the cell model at the tolerances of the tests of
/// the C target (1e-10, 1e-12), for the performance report:
/// `cargo test --release -p sbmlsim-engine --test stepper -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a test"]
fn report_memory_and_timing() {
    let settings = || IntegratorSettings {
        relative_tolerance: 1e-10,
        absolute_tolerance: Tolerance::Scalar(1e-12),
        ..IntegratorSettings::default()
    };
    let (model, description) = native(&cell_model());
    // the steppers of one model share its description
    let description = std::sync::Arc::new(description);
    let sets = cell_parameter_sets();
    let new = |set: usize| {
        Stepper::new(
            Box::new(model.clone()),
            description.clone(),
            settings(),
            &sets[set % sets.len()],
        )
        .unwrap()
    };
    let resident = || -> usize {
        let status = std::fs::read_to_string("/proc/self/statm").unwrap();
        let pages: usize = status.split_whitespace().nth(1).unwrap().parse().unwrap();
        pages * 4096
    };
    let mut stepper = new(0);
    let before_advance = stepper.memory_bytes();
    let y = stepper.state().to_vec();
    stepper.advance(&y, 0.0, 1.0).unwrap();
    println!(
        "memory_bytes: {before_advance} after new, {} after the first advance",
        stepper.memory_bytes()
    );
    drop(stepper);

    // the resident memory of 10000 steppers, each advanced once (CVODE allocates the rest of
    // its memory at the first start)
    let count = 10_000;
    let before = resident();
    let started = Instant::now();
    let mut steppers: Vec<Stepper> = (0..count).map(new).collect();
    let construction = started.elapsed();
    for stepper in &mut steppers {
        let y = stepper.state().to_vec();
        stepper.advance(&y, 0.0, 1.0).unwrap();
    }
    let after = resident();
    println!(
        "resident memory per stepper: {} bytes; construction {:.2} us per stepper",
        (after - before) / count,
        construction.as_secs_f64() * 1e6 / count as f64
    );
    drop(steppers);

    for dt in [1usize, 10, 60] {
        let mut advances = 0;
        let (mut steps, mut rejected) = (0, 0);
        let started = Instant::now();
        for set in 0..sets.len() {
            let mut stepper = new(set);
            for t0 in (0..1440).step_by(dt) {
                let y = stepper.state().to_vec();
                let stats = stepper.advance(&y, t0 as f64, (t0 + dt) as f64).unwrap();
                steps += stats.steps;
                rejected += stats.rejected;
                advances += 1;
            }
        }
        let elapsed = started.elapsed();
        println!(
            "steps of {dt} min: {:.2} us per advance, {:.2} steps and {:.3} rejected per \
             advance ({advances} advances)",
            elapsed.as_secs_f64() * 1e6 / advances as f64,
            steps as f64 / advances as f64,
            rejected as f64 / advances as f64,
        );
    }
}

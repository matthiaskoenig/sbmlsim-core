//! Prepared plans cross the real JSON/wasm ABI and remain usable on retained workers.
use std::sync::OnceLock;

use sbmlsim_abi::{
    Change, EngineError, MAX_PREPARED_REQUESTS, ModelDescription, SimulationRequest,
    SimulationResult, SteadyState,
};
use sbmlsim_host::{LoadedModel, Runtime};

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn model() -> LoadedModel {
    let wasm = wat::parse_str(include_str!("decay.wat")).unwrap();
    let desc: ModelDescription =
        serde_json::from_str(include_str!("decay-model/model.json")).unwrap();
    runtime().load(&wasm, &desc).unwrap()
}

fn request() -> SimulationRequest {
    serde_json::from_str(include_str!("decay-model/request.json")).unwrap()
}

fn equal(actual: &SimulationResult, expected: &SimulationResult) {
    assert_eq!(actual.columns, expected.columns);
    assert_eq!(actual.n_rows, expected.n_rows);
    assert!(
        actual
            .values
            .iter()
            .zip(&expected.values)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert_eq!(actual.values.len(), expected.values.len());
}

#[test]
fn prepared_plans_are_shared_by_workers_and_preserve_results() {
    let model = model();
    let mut first = model.instantiate().unwrap();
    let mut second = model.instantiate().unwrap();
    let mut ordinary = model.instantiate().unwrap();
    for steady_state in [None, Some(SteadyState::default())] {
        let mut template = request();
        template.preinit = vec![("y".into(), 2.0)];
        template.changes = vec![Change {
            time: 4.0,
            values: vec![("y".into(), 3.0)],
            increments: Vec::new(),
        }];
        template.steady_state = steady_state;
        let plan = model.prepare(&template, &["k".into(), "y".into()]).unwrap();
        assert_eq!(plan.n_values(), 2);
        for values in [[0.25, 3.0], [0.9, 1.0], [0.25, 3.0]] {
            let mut concrete = template.clone();
            concrete.preinit = vec![("y".into(), values[1]), ("k".into(), values[0])];
            let expected = ordinary.simulate(&concrete).unwrap();
            let retained = first.simulate_prepared(&plan, &values).unwrap();
            equal(&retained, &expected);
            equal(
                &second.simulate_prepared(&plan, &values).unwrap(),
                &expected,
            );
            assert!(first.stats().unwrap().steps > 0);
            // An intervening ordinary request cannot change either the plan or its output.
            first.simulate(&request()).unwrap();
            equal(&retained, &expected);
            equal(&first.simulate_prepared(&plan, &values).unwrap(), &expected);
        }
    }
    assert_eq!(first.wasi_calls(), 0);
    assert_eq!(second.wasi_calls(), 0);
}

#[test]
fn prepared_validation_and_plan_limits_allow_recovery() {
    let loaded = model();
    let mut instance = loaded.instantiate().unwrap();
    for ids in [
        vec!["missing".into()],
        vec!["t_half".into()],
        vec!["k".into(), "k".into()],
    ] {
        assert!(matches!(
            instance.prepare(&request(), &ids),
            Err(EngineError::Request(_))
        ));
    }
    let plan = instance.prepare(&request(), &["k".into()]).unwrap();
    for values in [vec![], vec![0.5, 1.0], vec![f64::NAN], vec![f64::INFINITY]] {
        assert!(matches!(
            instance.simulate_prepared(&plan, &values),
            Err(EngineError::Request(_))
        ));
    }
    let mut foreign = model().instantiate().unwrap();
    assert!(matches!(
        foreign.simulate_prepared(&plan, &[0.5]),
        Err(EngineError::Request(_))
    ));
    equal(
        &instance.simulate_prepared(&plan, &[0.5]).unwrap(),
        &foreign.simulate(&request()).unwrap(),
    );
    drop(plan);

    // Dropping a plan returns its guest slot on the next prepared operation.
    for _ in 0..MAX_PREPARED_REQUESTS * 2 {
        let plan = instance.prepare(&request(), &[]).unwrap();
        assert_eq!(plan.n_values(), 0);
        equal(
            &instance.simulate_prepared(&plan, &[]).unwrap(),
            &foreign.simulate(&request()).unwrap(),
        );
    }
    let mut live = (0..MAX_PREPARED_REQUESTS)
        .map(|_| instance.prepare(&request(), &[]).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        instance.prepare(&request(), &[]),
        Err(EngineError::Request(_))
    ));
    let retained = live.pop().unwrap();
    let clone = retained.clone();
    drop(retained);
    assert!(instance.prepare(&request(), &[]).is_err());
    drop(clone);
    let replacement = instance.prepare(&request(), &[]).unwrap();
    equal(
        &instance.simulate_prepared(&replacement, &[]).unwrap(),
        &foreign.simulate(&request()).unwrap(),
    );
}

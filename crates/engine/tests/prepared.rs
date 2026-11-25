//! Prepared requests must preserve numerical results and reset all run state.
use sbmlsim_abi::{
    Change, IntegratorSettings, JacobianChoice, LinearSolver, Output, SIMULATION_CONTRACT,
    SimulationRequest, SimulationResult, SimulationStats, SteadyState, Tolerance,
};
use sbmlsim_engine::model::{Compartmental, Decay, EventModel, Target, TestEvent};
use sbmlsim_engine::{Engine, EngineError};

fn request() -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.into(),
        start: 0.0,
        end: 3.0,
        output: Output::Uniform { steps: 12 },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec![],
        settings: IntegratorSettings::default(),
        seed: 123,
    }
}

fn decay() -> Engine {
    let model = Decay { k: 0.5 };
    Engine::new(Box::new(model), model.description()).unwrap()
}

fn equal(actual: &SimulationResult, expected: &SimulationResult) {
    assert_eq!(actual.columns, expected.columns);
    assert_eq!(actual.n_rows, expected.n_rows);
    assert_eq!(
        actual
            .values
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        expected
            .values
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
}

#[test]
fn prepared_runs_are_bit_identical_across_settings_and_changes() {
    let mut engine = decay();
    let mut req = request();
    req.preinit = vec![("y".into(), 2.0), ("k".into(), 0.25)];
    req.changes = vec![Change {
        time: 1.5,
        values: vec![("y".into(), 4.0), ("k".into(), 0.7)],
        increments: Vec::new(),
        forms: Vec::new(),
    }];
    for stiff in [true, false, true] {
        for jacobian in [JacobianChoice::Auto, JacobianChoice::DifferenceQuotient] {
            for linear_solver in [LinearSolver::Dense, LinearSolver::Klu] {
                req.settings.stiff = stiff;
                req.settings.jacobian = jacobian;
                req.settings.linear_solver = linear_solver;
                req.settings.absolute_tolerance = Tolerance::PerState(vec![1e-11]);
                for output in [
                    Output::Uniform { steps: 12 },
                    Output::IntegratorSteps,
                    Output::Times(vec![0.0, 0.25, 1.5, 1.5, 3.0]),
                ] {
                    req.output = output;
                    let plan = engine.prepare(&req, &["k".into(), "y".into()]).unwrap();
                    assert_eq!(plan.n_values(), 2);
                    let plan = plan.clone();
                    for values in [[0.3, 5.0], [0.9, 0.4], [0.3, 5.0]] {
                        let actual = engine.simulate_prepared(&plan, &values).unwrap();
                        let mut concrete = req.clone();
                        concrete.preinit[0].1 = values[1];
                        concrete.preinit[1].1 = values[0];
                        equal(&actual, &decay().simulate(&concrete).unwrap());
                    }
                }
            }
        }
    }
}

#[test]
fn a_point_request_records_each_requested_row_from_the_initialized_ode() {
    let mut engine = decay();
    let trajectory = request();
    let original = engine.simulate(&trajectory).unwrap();
    let trajectory_stats = engine.stats();
    assert!(trajectory_stats.steps > 0);

    let mut req = request();
    req.start = 5.0;
    req.end = 5.0;
    req.selections = vec!["y".into()];
    for (output, rows) in [
        (Output::Times(vec![]), 0),
        (Output::Times(vec![5.0, 5.0]), 2),
        (Output::Uniform { steps: 3 }, 4),
        (Output::IntegratorSteps, 1),
    ] {
        req.output = output;
        req.preinit = vec![("y".into(), 3.0)];
        let ordinary = engine.simulate(&req).unwrap();
        assert_eq!(ordinary.n_rows, rows);
        assert_eq!(ordinary.values, [5.0, 3.0].repeat(rows));
        assert_eq!(engine.stats(), SimulationStats::default());
        let prepared = engine.prepare(&req, &["y".into()]).unwrap();
        for value in [7.0, 3.0] {
            let result = engine.simulate_prepared(&prepared, &[value]).unwrap();
            assert_eq!(result.columns, ordinary.columns);
            assert_eq!(result.n_rows, rows);
            assert_eq!(result.values, [5.0, value].repeat(rows));
            assert_eq!(engine.stats(), SimulationStats::default());
        }
    }

    assert!(matches!(
        decay().with_max_output_values(1).simulate(&req),
        Err(EngineError::Integration {
            flag: sbmlsim_engine::OUTPUT_LIMIT_FLAG,
            ..
        })
    ));
    req.changes = vec![Change {
        time: req.start,
        values: vec![("y".into(), 11.0)],
        increments: vec![],
        forms: Vec::new(),
    }];
    assert_eq!(engine.simulate(&req).unwrap().values, [5.0, 11.0]);
    equal(&engine.simulate(&trajectory).unwrap(), &original);
    assert_eq!(engine.stats(), trajectory_stats);
}

#[test]
fn concentration_overrides_keep_template_quantity_and_order() {
    let model = Compartmental { k: 0.5 };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
    let mut req = request();
    req.preinit = vec![("C".into(), 3.0), ("[S]".into(), 2.0), ("R".into(), 4.0)];
    req.changes = vec![Change {
        time: 1.5,
        values: vec![("C".into(), 5.0), ("[S]".into(), 1.0)],
        increments: Vec::new(),
        forms: Vec::new(),
    }];
    // S overrides the entity's existing concentration slot without changing its quantity.
    let plan = engine
        .prepare(&req, &["S".into(), "C".into(), "[H]".into()])
        .unwrap();
    for values in [[4.0, 2.0, 3.0], [1.0, 7.0, 2.0], [4.0, 2.0, 3.0]] {
        let actual = engine.simulate_prepared(&plan, &values).unwrap();
        let mut concrete = req.clone();
        concrete.preinit[0].1 = values[1];
        concrete.preinit[1].1 = values[0];
        concrete.preinit.push(("[H]".into(), values[2]));
        let model = Compartmental { k: 0.5 };
        equal(
            &actual,
            &Engine::new(Box::new(model), model.description())
                .unwrap()
                .simulate(&concrete)
                .unwrap(),
        );
    }
}

#[test]
fn steady_preinit_is_combined_and_failures_do_not_dirty_scratch() {
    let mut engine = decay();
    let mut req = request();
    req.steady_state = Some(SteadyState {
        preinit: vec![("k".into(), 0.5)],
        ..SteadyState::default()
    });
    let plan = engine.prepare(&req, &["k".into()]).unwrap();
    for value in [0.5, 1.0, 0.5] {
        let actual = engine.simulate_prepared(&plan, &[value]).unwrap();
        let mut concrete = req.clone();
        concrete.steady_state.as_mut().unwrap().preinit[0].1 = value;
        equal(&actual, &decay().simulate(&concrete).unwrap());
    }
    req.preinit.push(("k".into(), 0.25));
    assert!(engine.prepare(&req, &["k".into()]).is_err());
    req.preinit.clear();
    req.steady_state.as_mut().unwrap().max_time = 0.0001;
    let failing = engine.prepare(&req, &["k".into()]).unwrap();
    assert!(matches!(
        engine.simulate_prepared(&failing, &[0.5]),
        Err(EngineError::Integration { .. })
    ));
    equal(
        &engine.simulate_prepared(&plan, &[0.5]).unwrap(),
        &decay()
            .simulate(&{
                let mut req = request();
                req.steady_state = Some(SteadyState::default());
                req
            })
            .unwrap(),
    );
}

#[test]
fn invalid_bindings_and_foreign_plans_are_refused_without_poisoning_engine() {
    let mut engine = decay();
    for ids in [
        vec!["missing".into()],
        vec!["t_half".into()],
        vec!["k".into(), "k".into()],
        vec!["[k]".into()],
    ] {
        assert!(engine.prepare(&request(), &ids).is_err());
    }
    let plan = engine.prepare(&request(), &["k".into()]).unwrap();
    for values in [vec![], vec![1.0, 2.0], vec![f64::NAN], vec![f64::INFINITY]] {
        assert!(matches!(
            engine.simulate_prepared(&plan, &values),
            Err(EngineError::Request(_))
        ));
        assert_eq!(engine.stats(), Default::default());
    }
    assert!(matches!(
        decay().simulate_prepared(&plan, &[0.5]),
        Err(EngineError::Request(_))
    ));
    let retained = engine.simulate_prepared(&plan, &[0.5]).unwrap();
    let original = retained.clone();
    engine.simulate_prepared(&plan, &[2.0]).unwrap();
    equal(&retained, &original);
    equal(&retained, &decay().simulate(&request()).unwrap());
    let mut invalid = request();
    invalid.settings.relative_tolerance = -1.0;
    assert!(engine.prepare(&invalid, &[]).is_err());
}

fn event_engine() -> Engine {
    let mut event = TestEvent::new("pulse", |t, _, _| t >= 1.0, vec![|t, _, _| t - 1.0]);
    event.delay = Some(|_, _, p| p[0]);
    event.assignments = vec![(Target::State(0), |_, _, _| 7.0)];
    let model = EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![("delay", 0.2)],
        rhs: |_, _, _, rates| rates[0] = 1.0,
        events: vec![event],
        discontinuities: vec![],
    };
    let desc = model.description();
    Engine::new(Box::new(model), desc).unwrap()
}

#[test]
fn gmres_prepared_replay_applies_delayed_events_and_changes() {
    let mut engine = event_engine();
    let mut req = request();
    req.settings.linear_solver = LinearSolver::Gmres;
    req.selections = vec!["y".into()];
    req.changes = vec![Change {
        time: 2.0,
        values: vec![("y".into(), 4.0)],
        increments: vec![],
        forms: Vec::new(),
    }];
    let plan = engine.prepare(&req, &["delay".into()]).unwrap();
    for delay in [0.2, 1.5, 0.2] {
        let result = engine.simulate_prepared(&plan, &[delay]).unwrap();
        let mut direct = req.clone();
        direct.settings.linear_solver = LinearSolver::Dense;
        direct.preinit = vec![("delay".into(), delay)];
        let expected = event_engine().simulate(&direct).unwrap();
        assert_eq!(result.columns, expected.columns);
        assert_eq!(result.n_rows, expected.n_rows);
        for (v, r) in result.values.iter().zip(&expected.values) {
            assert!((v - r).abs() < 1e-8, "{v} versus {r}");
        }
        let last = *result.values.last().unwrap();
        let expected_last = if delay < 1.0 { 5.0 } else { 7.5 };
        assert!((last - expected_last).abs() < 1e-8);
    }
}

#[test]
fn a_prepared_ode_probes_all_differential_states_without_consistency_steps() {
    let mut event = TestEvent::new("crossing", |_, y, _| y[1] > 0.0, vec![|_, y, _| y[1]]);
    event.assignments = vec![(Target::State(0), |_, _, p| p[0])];
    let model = EventModel {
        states: vec![("observed", 0.0), ("ramp", 0.0)],
        parameters: vec![("amount", 0.0)],
        rhs: |_, _, _, rates| rates.copy_from_slice(&[0.0, 1.0]),
        events: vec![event],
        discontinuities: vec![],
    };
    let desc = model.description();
    let mut engine = Engine::new(Box::new(model), desc).unwrap();
    let mut req = request();
    req.end = req.start;
    req.output = Output::Times(vec![0.0]);
    req.changes = vec![Change {
        time: 0.0,
        values: vec![],
        increments: vec![("amount".into(), 1.0)],
        forms: Vec::new(),
    }];
    req.selections = vec!["observed".into()];
    let plan = engine.prepare(&req, &["amount".into()]).unwrap();
    for amount in [2.0, 4.0, 2.0] {
        let result = engine.simulate_prepared(&plan, &[amount]).unwrap();
        assert_eq!(result.values, [0.0, amount + 1.0]);
        assert_eq!(
            engine.stats(),
            SimulationStats {
                probe_rhs_evals: 2,
                ..SimulationStats::default()
            }
        );
    }
}

#[test]
fn event_scratch_resets_after_delays_and_integration_failures() {
    let mut engine = event_engine();
    let mut req = request();
    req.output = Output::IntegratorSteps;
    let plan = engine.prepare(&req, &["delay".into()]).unwrap();
    for delay in [0.2, 10.0, -1.0, 0.2] {
        let actual = engine.simulate_prepared(&plan, &[delay]);
        let mut concrete = req.clone();
        concrete.preinit = vec![("delay".into(), delay)];
        match (actual, event_engine().simulate(&concrete)) {
            (Ok(actual), Ok(expected)) => equal(&actual, &expected),
            (Err(actual), Err(expected)) => assert_eq!(actual, expected),
            other => panic!("different results: {other:?}"),
        }
    }
}

#[test]
fn alternating_plans_reset_preinit_and_selection_masks() {
    let model = Compartmental { k: 0.5 };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
    let full = request();
    let full_plan = engine.prepare(&full, &[]).unwrap();
    let mut partial = request();
    partial.preinit = vec![("C".into(), 7.0), ("[S]".into(), 3.0)];
    partial.selections = vec!["C".into()];
    partial.settings.stiff = false;
    let partial_plan = engine.prepare(&partial, &[]).unwrap();
    for _ in 0..3 {
        for (plan, concrete) in [(&full_plan, &full), (&partial_plan, &partial)] {
            let model = Compartmental { k: 0.5 };
            let expected = Engine::new(Box::new(model), model.description())
                .unwrap()
                .simulate(concrete)
                .unwrap();
            equal(&engine.simulate_prepared(plan, &[]).unwrap(), &expected);
        }
    }
}

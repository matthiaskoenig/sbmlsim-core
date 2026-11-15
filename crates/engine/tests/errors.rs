//! The errors of the engine: failed integrations, invalid requests refused before any
//! integration, the features of later tasks, and the reuse of an engine after an error.

use sbmlsim_abi::{
    Change, EventInfo, IntegratorSettings, MODEL_CONTRACT, ModelDescription, Output,
    SIMULATION_CONTRACT, SimulationRequest, SimulationResult, SteadyState, Tolerance,
};
use sbmlsim_engine::model::{Decay, Derived, NanRhs, Robertson};
use sbmlsim_engine::{Engine, EngineError, MAX_OUTPUT_VALUES, ModelFns, simulate};

/// A request from `start` to `end` with the given output, all selections, the default settings.
fn request(start: f64, end: f64, output: Output) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: vec![],
        settings: IntegratorSettings::default(),
        seed: 0,
    }
}

fn decay(req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
    let model = Decay { k: 0.5 };
    simulate(&model, &model.description(), req)
}

/// Asserts that the result is a `Request` error whose message contains every text.
fn assert_request<T: std::fmt::Debug>(result: Result<T, EngineError>, texts: &[&str]) {
    match result {
        Err(EngineError::Request(message)) => {
            for text in texts {
                assert!(message.contains(text), "{text} not in: {message}");
            }
        }
        other => panic!("expected a Request error with {texts:?}, got {other:?}"),
    }
}

#[test]
fn integration_failure_is_an_error() {
    // a right hand side of NaN at t > 1
    let model = NanRhs;
    let req = request(0.0, 2.0, Output::Uniform { steps: 3 });
    match simulate(&model, &model.description(), &req) {
        Err(EngineError::Integration { flag, time, .. }) => {
            assert!(flag < 0, "{flag}");
            // CVODE reports the last time it reached, which cannot pass 1: a step beyond it
            // evaluates the NaN and fails (the Zeno steps towards 1 end at CV_TOO_MUCH_WORK)
            assert!(time > 0.5 && time <= 1.0, "{time}");
        }
        other => panic!("{other:?}"),
    }
    // the steps of the integrator stall at 1 and end with an error, quickly
    let req = request(0.0, 2.0, Output::IntegratorSteps);
    match simulate(&model, &model.description(), &req) {
        Err(EngineError::Integration {
            flag,
            time,
            message,
        }) => {
            assert!(time <= 1.0, "{time}");
            assert_eq!(flag, sbmlsim_engine::STALL_FLAG);
            assert!(message.contains("stalled"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // too many steps for one call of CVODE, on Robertson: CV_TOO_MUCH_WORK
    let model = Robertson;
    let mut req = request(0.0, 4e10, Output::Uniform { steps: 1 });
    req.settings.maximum_num_steps = 10;
    match simulate(&model, &model.description(), &req) {
        Err(EngineError::Integration { flag, time, .. }) => {
            assert_eq!(flag, -1, "CV_TOO_MUCH_WORK");
            assert!(time > 0.0 && time < 4e10, "{time}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn invalid_requests_are_refused() {
    let base = request(0.0, 10.0, Output::Uniform { steps: 10 });
    let with = |f: &dyn Fn(&mut SimulationRequest)| {
        let mut req = base.clone();
        f(&mut req);
        req
    };
    // an unknown selection
    assert_request(
        decay(&with(&|r| r.selections = vec!["nope".to_string()])),
        &["nope"],
    );
    // a change after the end (a feature of a later task, but the request is checked first)
    let change = |time: f64, value: f64| Change {
        time,
        values: vec![("k".to_string(), value)],
    };
    assert_request(
        decay(&with(&|r| r.changes = vec![change(20.0, 1.0)])),
        &["change at t = 20 "],
    );
    for (changes, text) in [
        (vec![change(0.0, 1.0)], "change at t = 0 "),
        (vec![change(f64::NAN, 1.0)], "change at t = NaN "),
        (vec![change(5.0, 1.0), change(4.0, 1.0)], "change at t = 4 "),
        (
            vec![change(5.0, f64::INFINITY)],
            "the value inf of k is not finite",
        ),
    ] {
        assert_request(decay(&with(&|r| r.changes = changes.clone())), &[text]);
    }
    // a change of an unknown entity and of one which is not settable
    for id in ["nothing", "t_half"] {
        let r = with(&|r| {
            r.changes = vec![Change {
                time: 5.0,
                values: vec![(id.to_string(), 1.0)],
            }]
        });
        assert_request(decay(&r), &[id]);
    }
    // output times out of order, outside the simulation, not finite
    for times in [vec![1.0, 0.5], vec![-1.0], vec![11.0], vec![f64::NAN]] {
        assert_request(
            decay(&with(&|r| r.output = Output::Times(times.clone()))),
            &["output times"],
        );
    }
    // finite ends whose difference is not
    for output in [Output::Uniform { steps: 4 }, Output::Times(vec![0.0])] {
        assert_request(
            decay(&request(-1e308, 1e308, output)),
            &["interval is not finite"],
        );
    }
    assert_request(
        decay(&request(5.0, 1.0, base.output.clone())),
        &["from 5 to 1"],
    );
    assert_request(
        decay(&request(f64::NAN, 1.0, base.output.clone())),
        &["NaN"],
    );
    assert_request(
        decay(&request(0.0, f64::INFINITY, base.output.clone())),
        &["inf"],
    );
    assert_request(
        decay(&with(&|r| r.output = Output::Uniform { steps: 0 })),
        &["0 steps"],
    );
    // contracts name both strings
    assert_request(
        decay(&with(&|r| r.contract = "simulation/2".to_string())),
        &["simulation/2", SIMULATION_CONTRACT],
    );
    let model = Decay { k: 0.5 };
    let mut desc = model.description();
    desc.contract = "model/2".to_string();
    assert_request(simulate(&model, &desc, &base), &["model/2", MODEL_CONTRACT]);
    assert_request(
        Engine::new(Box::new(model), desc).map(|_| ()),
        &["model/2", MODEL_CONTRACT],
    );
}

#[test]
fn invalid_settings_are_refused() {
    let settings = |f: &dyn Fn(&mut IntegratorSettings)| {
        let mut req = request(0.0, 1.0, Output::Uniform { steps: 2 });
        f(&mut req.settings);
        req
    };
    let cases: Vec<(SimulationRequest, &str)> = vec![
        (
            settings(&|s| s.relative_tolerance = -1e-6),
            "relative tolerance",
        ),
        (
            settings(&|s| s.relative_tolerance = f64::NAN),
            "relative tolerance",
        ),
        (
            settings(&|s| s.absolute_tolerance = Tolerance::Scalar(-1.0)),
            "absolute tolerance",
        ),
        (
            settings(&|s| s.absolute_tolerance = Tolerance::PerState(vec![1e-9, 1e-9])),
            "absolute tolerances",
        ),
        (
            settings(&|s| s.absolute_tolerance = Tolerance::PerState(vec![f64::INFINITY])),
            "absolute tolerance",
        ),
        // rtol 0 with atol 0: no error weight, refused here and not as an integration error
        (
            settings(&|s| {
                s.relative_tolerance = 0.0;
                s.absolute_tolerance = Tolerance::Scalar(0.0);
            }),
            "relative tolerance 0",
        ),
        (settings(&|s| s.maximum_num_steps = 0), "number of steps"),
        (
            settings(&|s| s.maximum_time_step = -1.0),
            "maximum time step",
        ),
        (
            settings(&|s| s.initial_time_step = f64::NAN),
            "initial time step",
        ),
    ];
    for (req, text) in cases {
        assert_request(decay(&req), &[text]);
    }
    // an absolute tolerance of 0 with a positive relative tolerance, and the opposite, are valid
    let ok = settings(&|s| s.absolute_tolerance = Tolerance::Scalar(0.0));
    decay(&ok).unwrap();
    let ok = settings(&|s| {
        s.relative_tolerance = 0.0;
        s.absolute_tolerance = Tolerance::Scalar(1e-9);
    });
    decay(&ok).unwrap();
}

#[test]
fn preinit_values_are_checked() {
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 2 });
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        req.preinit = vec![("k".to_string(), value)];
        assert_request(decay(&req), &["k", "finite"]);
    }
    req.preinit = vec![("k".to_string(), 1.0), ("k".to_string(), 2.0)];
    assert_request(decay(&req), &["k", "more than once"]);
}

#[test]
fn output_larger_than_the_limit_is_refused_upfront() {
    // nothing is allocated for these, `steps` of `u32::MAX` would be 4e9 rows
    for steps in [u32::MAX, (MAX_OUTPUT_VALUES / 3) as u32] {
        assert_request(
            decay(&request(0.0, 1.0, Output::Uniform { steps })),
            &["exceeds the limit"],
        );
    }
    // the limit counts values, so the columns count: a million columns and 129 rows are over it
    // with few rows (the columns repeat an id)
    let columns = 1 << 20;
    assert!(129 * columns > MAX_OUTPUT_VALUES);
    for output in [
        Output::Times(vec![0.5; 129]),
        Output::Uniform { steps: 128 },
    ] {
        let mut req = request(0.0, 1.0, output);
        req.selections = vec!["y".to_string(); columns];
        assert_request(decay(&req), &["exceeds the limit"]);
    }
}

#[test]
fn features_of_later_tasks_are_unsupported() {
    let model = Decay { k: 0.5 };
    let base = request(0.0, 1.0, Output::Uniform { steps: 2 });

    let mut req = base.clone();
    req.changes = vec![Change {
        time: 0.5,
        values: vec![("k".to_string(), 1.0)],
    }];
    assert!(matches!(
        simulate(&model, &model.description(), &req),
        Err(EngineError::Unsupported(m)) if m.contains("changes")
    ));

    let mut req = base.clone();
    req.steady_state = Some(SteadyState::default());
    assert!(matches!(
        simulate(&model, &model.description(), &req),
        Err(EngineError::Unsupported(m)) if m.contains("steady")
    ));
    // an invalid steady state is a Request error, checked first
    req.steady_state = Some(SteadyState {
        max_time: -1.0,
        ..SteadyState::default()
    });
    assert_request(
        simulate(&model, &model.description(), &req),
        &["maximum time"],
    );

    // events: of the description ...
    let mut desc: ModelDescription = model.description();
    desc.events = vec![EventInfo {
        id: "e".to_string(),
        has_delay: false,
        has_priority: false,
        persistent: true,
        initial_value: false,
        use_values_from_trigger_time: false,
        n_assignments: 1,
    }];
    assert!(matches!(
        simulate(&model, &desc, &base),
        Err(EngineError::Unsupported(m)) if m.contains("events")
    ));
    // ... and of the functions of the model (root functions)
    struct Rooted(Decay);
    impl ModelFns for Rooted {
        fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
            self.0.defaults(p, y)
        }
        fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
            self.0.init(t, p, y, fixed)
        }
        fn rhs(&self, t: f64, y: &[f64], p: &[f64], ydot: &mut [f64]) {
            self.0.rhs(t, y, p, ydot)
        }
        fn jac(&self, t: f64, y: &[f64], p: &[f64], values: &mut [f64]) {
            self.0.jac(t, y, p, values)
        }
        fn roots(&self, _t: f64, _y: &[f64], _p: &[f64], g: &mut [f64]) {
            g[0] = 1.0;
        }
        fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
            self.0.trigger(i, t, y, p)
        }
        fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
            self.0.priority(i, t, y, p)
        }
        fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
            self.0.delay(i, t, y, p)
        }
        fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            self.0.event_values(i, t, y, p, out)
        }
        fn event_apply(&self, i: u32, values: &[f64], y: &mut [f64], p: &mut [f64]) {
            self.0.event_apply(i, values, y, p)
        }
        fn selections(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
            self.0.selections(t, y, p, out)
        }
        fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
            self.0.assign(t, y, p)
        }
        fn n_roots(&self) -> usize {
            1
        }
    }
    let rooted = Rooted(model);
    assert!(matches!(
        simulate(&rooted, &model.description(), &base),
        Err(EngineError::Unsupported(m)) if m.contains("events")
    ));
    // the plain model has neither
    assert!(model.description().events.is_empty());
    assert_eq!(model.n_roots(), 0);
    simulate(&model, &model.description(), &base).unwrap();
}

#[test]
fn malformed_selection_indices_are_refused() {
    let model = Decay { k: 0.5 };
    let req = request(0.0, 1.0, Output::Uniform { steps: 2 });
    // (indices of `y` and `t_half`, the model has two selections)
    for indices in [[0, 0], [0, 2], [1, 1], [5, 0]] {
        let mut desc = model.description();
        desc.selections[0].index = indices[0];
        desc.selections[1].index = indices[1];
        assert_request(simulate(&model, &desc, &req), &["selection indices"]);
        assert_request(
            Engine::new(Box::new(model), desc).map(|_| ()),
            &["selection indices"],
        );
    }
    // a permutation is valid
    let mut desc = model.description();
    desc.selections[0].index = 1;
    desc.selections[1].index = 0;
    let mut engine = Engine::new(Box::new(model), desc).unwrap();
    let result = engine.simulate(&req).unwrap();
    assert_eq!(result.columns, ["time", "y", "t_half"]);
    // the index of `t_half` is now 0: the column `y` has the value of the first selection
    assert_eq!(result.values[1], 2f64.ln() / 0.5);
}

#[test]
fn an_engine_works_after_an_integration_error() {
    let model = Robertson;
    let desc = model.description();
    let mut good = request(0.0, 4.0, Output::Times(vec![0.0, 0.04, 0.4, 1.0, 4.0]));
    good.settings.relative_tolerance = 1e-8;
    good.settings.absolute_tolerance = Tolerance::PerState(vec![1e-12, 1e-14, 1e-10]);
    let mut bad = good.clone();
    bad.end = 4e10;
    bad.output = Output::Uniform { steps: 1 };
    bad.settings.maximum_num_steps = 10;

    let fresh = simulate(&model, &desc, &good).unwrap();
    for solver in [
        sbmlsim_abi::LinearSolver::Dense,
        sbmlsim_abi::LinearSolver::Klu,
    ] {
        let (mut good, mut bad) = (good.clone(), bad.clone());
        good.settings.linear_solver = solver;
        bad.settings.linear_solver = solver;
        let reference = simulate(&model, &desc, &good).unwrap();
        let mut engine = Engine::new(Box::new(model), desc.clone()).unwrap();
        assert!(matches!(
            engine.simulate(&bad),
            Err(EngineError::Integration { flag: -1, .. })
        ));
        let after = engine.simulate(&good).unwrap();
        assert_eq!(after.columns, reference.columns);
        let bits = |r: &SimulationResult| r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&after), bits(&reference), "{solver:?}");
        // a request which is refused leaves the engine as it was, too
        let mut invalid = good.clone();
        invalid.end = -1.0;
        assert!(engine.simulate(&invalid).is_err());
        assert_eq!(bits(&engine.simulate(&good).unwrap()), bits(&reference));
    }
    assert_eq!(fresh.n_rows, 5);
}

#[test]
fn fixed_entities_keep_their_values_through_init() {
    let model = Derived { k: 0.5 };
    let desc = model.description();
    let mut req = request(0.0, 2.0, Output::Uniform { steps: 4 });
    // `init` evaluates `a = 2 k` ...
    let result = simulate(&model, &desc, &req).unwrap();
    for (i, t) in [0.0f64, 0.5, 1.0, 1.5, 2.0].iter().enumerate() {
        assert_eq!(result.values[i * 3 + 2], 1.0);
        assert!((result.values[i * 3 + 1] - (-t).exp()).abs() < 1e-5);
    }
    // ... a settable `a` set by the pre-initialization is fixed, `init` leaves it
    req.preinit = vec![("a".to_string(), 3.0)];
    let result = simulate(&model, &desc, &req).unwrap();
    for i in 0..result.n_rows {
        assert_eq!(result.values[i * 3 + 2], 3.0);
    }
    let last = result.values[4 * 3 + 1];
    assert!((last - (-6.0f64).exp()).abs() < 1e-5, "{last}");
    // setting `k` leaves `a` the assignment of the new `k`
    req.preinit = vec![("k".to_string(), 1.0)];
    let result = simulate(&model, &desc, &req).unwrap();
    assert_eq!(result.values[2], 2.0);
}

//! The errors of the engine: failed integrations, invalid requests refused before any
//! integration, a description of another model, and the reuse of an engine after an error.

use sbmlsim_abi::{
    Change, EventInfo, IntegratorSettings, MODEL_CONTRACT, ModelDescription, Output,
    SIMULATION_CONTRACT, SimulationRequest, SimulationResult, SteadyState, Tolerance,
};
use sbmlsim_engine::model::{Decay, Derived, NanRhs, Robertson};
use sbmlsim_engine::{Engine, EngineError, MAX_OUTPUT_VALUES, simulate};

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
    // a change after the end
    let change = |time: f64, value: f64| Change {
        time,
        values: vec![("k".to_string(), value)],
        increments: Vec::new(),
    };
    assert_request(
        decay(&with(&|r| r.changes = vec![change(20.0, 1.0)])),
        &["change at t = 20 "],
    );
    for (changes, text) in [
        (vec![change(-1.0, 1.0)], "change at t = -1 "),
        (vec![change(f64::NAN, 1.0)], "change at t = NaN "),
        (vec![change(5.0, 1.0), change(4.0, 1.0)], "change at t = 4 "),
        (
            vec![change(5.0, f64::INFINITY)],
            "the value inf of k at t = 5 is not finite",
        ),
    ] {
        assert_request(decay(&with(&|r| r.changes = changes.clone())), &[text]);
    }
    // a change of an unknown entity, of one which is not settable and the concentration of an
    // entity which is no species
    for id in ["nothing", "t_half", "[k]", "[nothing]"] {
        let r = with(&|r| {
            r.changes = vec![Change {
                time: 5.0,
                values: vec![(id.to_string(), 1.0)],
                increments: Vec::new(),
            }]
        });
        assert_request(decay(&r), &[id]);
    }
    // an entity set twice at one time, also by two changes of that time
    let twice = |a: f64, b: f64| {
        with(&|r| {
            r.changes = vec![change(a, 1.0), change(b, 2.0)];
        })
    };
    assert_request(
        decay(&twice(5.0, 5.0)),
        &["k is set more than once at t = 5"],
    );
    decay(&twice(5.0, 6.0)).unwrap();
    // a target of the pre-initialization of the request and of its steady state
    let r = with(&|r| {
        r.preinit = vec![("k".to_string(), 1.0)];
        r.steady_state = Some(SteadyState {
            preinit: vec![("k".to_string(), 2.0)],
            ..SteadyState::default()
        });
    });
    assert_request(decay(&r), &["k is set more than once"]);
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
    desc.contract = "model/1".to_string();
    assert_request(simulate(&model, &desc, &base), &["model/1", MODEL_CONTRACT]);
    assert_request(
        Engine::new(Box::new(model), desc).map(|_| ()),
        &["model/1", MODEL_CONTRACT],
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
fn invalid_steady_states_and_other_descriptions_are_refused() {
    let model = Decay { k: 0.5 };
    let base = request(0.0, 1.0, Output::Uniform { steps: 2 });
    for steady in [
        SteadyState {
            max_time: -1.0,
            ..SteadyState::default()
        },
        SteadyState {
            max_time: f64::INFINITY,
            ..SteadyState::default()
        },
    ] {
        let mut req = base.clone();
        req.steady_state = Some(steady);
        assert_request(
            simulate(&model, &model.description(), &req),
            &["maximum time"],
        );
    }
    let mut req = base.clone();
    req.steady_state = Some(SteadyState {
        absolute_tolerance: f64::NAN,
        ..SteadyState::default()
    });
    assert_request(
        simulate(&model, &model.description(), &req),
        &["steady state tolerances"],
    );

    // events are simulated: a description of events which the model does not have is a
    // description of another model, refused by the check of the sizes
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
    assert_request(simulate(&model, &desc, &base), &["1 events", "0 events"]);
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

/// A fallible bridge whose selections export records an error instead of panicking.
struct FailingSelections {
    calls: std::rc::Rc<std::cell::Cell<usize>>,
}

impl sbmlsim_engine::ModelFns for FailingSelections {
    fn check_error(&self) -> Result<(), EngineError> {
        if self.calls.get() > 0 {
            Err(EngineError::Internal("selections trapped".into()))
        } else {
            Ok(())
        }
    }
    fn selections(&self, _t: f64, _y: &[f64], _p: &[f64], _mask: &[u8], _out: &mut [f64]) {
        self.calls.set(self.calls.get() + 1);
    }
    fn defaults(&self, p: &mut [f64], y: &mut [f64]) {
        sbmlsim_engine::ModelFns::defaults(&Decay { k: 0.5 }, p, y)
    }
    fn init(&self, t: f64, p: &mut [f64], y: &mut [f64], fixed: &[u8]) {
        sbmlsim_engine::ModelFns::init(&Decay { k: 0.5 }, t, p, y, fixed)
    }
    fn rhs(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        sbmlsim_engine::ModelFns::rhs(&Decay { k: 0.5 }, t, y, p, out)
    }
    fn jac(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        sbmlsim_engine::ModelFns::jac(&Decay { k: 0.5 }, t, y, p, out)
    }
    fn roots(&self, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        sbmlsim_engine::ModelFns::roots(&Decay { k: 0.5 }, t, y, p, out)
    }
    fn trigger(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> bool {
        sbmlsim_engine::ModelFns::trigger(&Decay { k: 0.5 }, i, t, y, p)
    }
    fn priority(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        sbmlsim_engine::ModelFns::priority(&Decay { k: 0.5 }, i, t, y, p)
    }
    fn delay(&self, i: u32, t: f64, y: &[f64], p: &[f64]) -> f64 {
        sbmlsim_engine::ModelFns::delay(&Decay { k: 0.5 }, i, t, y, p)
    }
    fn event_values(&self, i: u32, t: f64, y: &[f64], p: &[f64], out: &mut [f64]) {
        sbmlsim_engine::ModelFns::event_values(&Decay { k: 0.5 }, i, t, y, p, out)
    }
    fn event_apply(&self, i: u32, t: f64, values: &[f64], y: &mut [f64], p: &mut [f64]) {
        sbmlsim_engine::ModelFns::event_apply(&Decay { k: 0.5 }, i, t, values, y, p)
    }
    fn assign(&self, t: f64, y: &mut [f64], p: &mut [f64]) {
        sbmlsim_engine::ModelFns::assign(&Decay { k: 0.5 }, t, y, p)
    }
    fn sizes(&self) -> sbmlsim_abi::ModelSizes {
        sbmlsim_engine::ModelFns::sizes(&Decay { k: 0.5 })
    }
}

#[test]
fn a_selection_failure_stops_a_large_output_request_at_the_first_row() {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let model = FailingSelections {
        calls: calls.clone(),
    };
    let mut engine = Engine::new(Box::new(model), Decay { k: 0.5 }.description()).unwrap();
    let req = request(0.0, 1.0, Output::Uniform { steps: 1_000_000 });
    assert!(
        matches!(engine.simulate(&req), Err(EngineError::Internal(ref message)) if message == "selections trapped")
    );
    assert_eq!(
        calls.get(),
        1,
        "a failed bridge must not be called for later output rows"
    );
    assert!(matches!(
        engine.simulate(&req),
        Err(EngineError::Internal(_))
    ));
    assert_eq!(
        calls.get(),
        1,
        "a failed engine must not execute another request"
    );
}

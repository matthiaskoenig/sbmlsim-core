//! The events: the times of their triggers, persistence, priorities, the values at the trigger
//! or at the execution, the event at the end, the order of ties and the limit of executions.

use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
};
use sbmlsim_engine::model::{EventModel, Fun, Target, TestEvent};
use sbmlsim_engine::{DELAY_FLAG, EVENT_LIMIT_FLAG, Engine, EngineError, simulate};

/// A request from `start` to `end` of the output, tight tolerances.
fn request(start: f64, end: f64, output: Output, seed: u64) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: vec![],
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: sbmlsim_abi::Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        },
        seed,
    }
}

/// The values of column `name` of a result.
fn column(result: &SimulationResult, name: &str) -> Vec<f64> {
    let k = result.columns.len();
    let j = result
        .columns
        .iter()
        .position(|c| c == name)
        .unwrap_or_else(|| panic!("no column {name}"));
    (0..result.n_rows)
        .map(|i| result.values[i * k + j])
        .collect()
}

fn run(model: &EventModel, req: &SimulationRequest) -> SimulationResult {
    simulate(model, &model.description(), req).unwrap()
}

/// `y' = 1`, `y(0) = 0`; `x` and `n` are parameters, 0.
fn ramp(events: Vec<TestEvent>) -> EventModel {
    EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![("x", 0.0), ("n", 0.0)],
        rhs: |_, _, _, ydot| ydot[0] = 1.0,
        events,
    }
}

/// `time > 1`, its root function `time - 1`.
fn after_one() -> TestEvent {
    TestEvent::new("after_one", |t, _, _| t > 1.0, vec![|t, _, _| t - 1.0])
}

const Y: Target = Target::State(0);
const X: Target = Target::Param(0);
const N: Target = Target::Param(1);

#[test]
fn time_trigger_fires_once() {
    // time > 1: y = 0, n = n + 1
    let mut event = after_one();
    event.assignments = vec![(Y, |_, _, _| 0.0), (N, |_, _, p| p[1] + 1.0)];
    let model = ramp(vec![event]);
    // an output at the time of a strict trigger at its threshold sees the state before it
    let r = run(&model, &request(0.0, 3.0, Output::Uniform { steps: 3 }, 1));
    let (y, n) = (column(&r, "y"), column(&r, "n"));
    assert_eq!(n, [0.0, 0.0, 1.0, 1.0]);
    assert!(
        (y[1] - 1.0).abs() < 1e-9 && (y[2] - 1.0).abs() < 1e-9,
        "{y:?}"
    );
    assert!((y[3] - 2.0).abs() < 1e-9, "{y:?}");
    // the steps of the integrator: two rows at the time 1, before and after, once
    let r = run(&model, &request(0.0, 3.0, Output::IntegratorSteps, 1));
    let (t, y, n) = (column(&r, "time"), column(&r, "y"), column(&r, "n"));
    let pairs: Vec<usize> = (1..t.len()).filter(|&i| t[i] == t[i - 1]).collect();
    assert_eq!(pairs.len(), 1, "{t:?}");
    let i = pairs[0];
    assert!((t[i] - 1.0).abs() < 1e-12, "{}", t[i]);
    assert!(
        (y[i - 1] - 1.0).abs() < 1e-9 && y[i] == 0.0,
        "{} {}",
        y[i - 1],
        y[i]
    );
    assert_eq!((n[i - 1], n[i], *n.last().unwrap()), (0.0, 1.0, 1.0));
    assert!(t.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*t.last().unwrap(), 3.0);
}

#[test]
fn persistent_and_not() {
    // 1 < time < 1.5, the delay 1: the trigger is false again before the execution at 2
    let window = |persistent: bool, target: Target| {
        let mut e = TestEvent::new(
            "window",
            |t, _, _| t > 1.0 && t < 1.5,
            vec![|t, _, _| t - 1.0, |t, _, _| 1.5 - t],
        );
        e.delay = Some(|_, _, _| 1.0);
        e.persistent = persistent;
        e.assignments = vec![(target, |_, _, _| 1.0)];
        e
    };
    let model = ramp(vec![window(true, X), window(false, N)]);
    let r = run(&model, &request(0.0, 3.0, Output::Uniform { steps: 6 }, 1));
    // the persistent one executes at 2, the other is cancelled at 1.5
    assert_eq!(column(&r, "x"), [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    assert_eq!(column(&r, "n"), [0.0; 7]);
}

/// `x = 10 x + digit` at execution time, so `x` records the order of the executions.
fn recorder_event(id: &'static str, priority: Option<Fun>, digit: u8) -> TestEvent {
    let mut e = after_one();
    e.id = id;
    e.priority = priority;
    e.use_values_from_trigger_time = false;
    e.assignments = vec![(
        X,
        match digit {
            1 => |_, _, p: &[f64]| 10.0 * p[0] + 1.0,
            _ => |_, _, p: &[f64]| 10.0 * p[0] + 2.0,
        },
    )];
    e
}

#[test]
fn priorities_order() {
    let req = request(0.0, 2.0, Output::Uniform { steps: 2 }, 1);
    // the priority 2 before the priority 1, whatever the order of the document
    let model = ramp(vec![
        recorder_event("one", Some(|_, _, _| 1.0), 1),
        recorder_event("two", Some(|_, _, _| 2.0), 2),
    ]);
    assert_eq!(*column(&run(&model, &req), "x").last().unwrap(), 21.0);
    let model = ramp(vec![
        recorder_event("one", Some(|_, _, _| 2.0), 1),
        recorder_event("two", Some(|_, _, _| 1.0), 2),
    ]);
    assert_eq!(*column(&run(&model, &req), "x").last().unwrap(), 12.0);
    // a priority is evaluated when the events execute: here 2 - x, so after the first
    // execution (x = 2) the other one (priority 1) is above it (priority 0)
    let model = ramp(vec![
        recorder_event("one", Some(|_, _, _| 1.0), 1),
        recorder_event("two", Some(|_, _, p| 2.0 - p[0]), 2),
    ]);
    assert_eq!(*column(&run(&model, &req), "x").last().unwrap(), 21.0);
}

#[test]
fn values_at_trigger_time() {
    // time > 1 with the delay 1: x = y, at the trigger (y = 1) or at the execution (y = 2)
    let event = |at_trigger: bool| {
        let mut e = after_one();
        e.delay = Some(|_, _, _| 1.0);
        e.use_values_from_trigger_time = at_trigger;
        e.assignments = vec![(X, |_, y, _| y[0])];
        e
    };
    let req = request(0.0, 3.0, Output::Uniform { steps: 3 }, 1);
    for (at_trigger, expected) in [(true, 1.0), (false, 2.0)] {
        let x = column(&run(&ramp(vec![event(at_trigger)]), &req), "x");
        assert_eq!(x[1], 0.0);
        // executed at 2: the output at 2 sees it
        assert!((x[2] - expected).abs() < 1e-9, "{at_trigger}: {x:?}");
        assert!((x[3] - expected).abs() < 1e-9, "{at_trigger}: {x:?}");
    }
}

#[test]
fn event_at_end_two_rows() {
    // time >= 1: y = 0, at the end 1
    let mut event = TestEvent::new("at_one", |t, _, _| t >= 1.0, vec![|t, _, _| t - 1.0]);
    event.assignments = vec![(Y, |_, _, _| 0.0)];
    let model = ramp(vec![event]);
    let r = run(&model, &request(0.0, 1.0, Output::IntegratorSteps, 1));
    let (t, y) = (column(&r, "time"), column(&r, "y"));
    let k = t.len();
    assert_eq!((t[k - 2], t[k - 1]), (1.0, 1.0), "{t:?}");
    assert!((y[k - 2] - 1.0).abs() < 1e-9 && y[k - 1] == 0.0, "{y:?}");
    // an output of times at the end sees the state after it, also for a strict trigger whose
    // event comes right after the end
    let r = run(&model, &request(0.0, 1.0, Output::Uniform { steps: 2 }, 1));
    assert_eq!(*column(&r, "y").last().unwrap(), 0.0);
    let mut strict = after_one();
    strict.assignments = vec![(Y, |_, _, _| 0.0)];
    let model = ramp(vec![strict]);
    let r = run(&model, &request(0.0, 1.0, Output::Uniform { steps: 2 }, 1));
    assert_eq!(*column(&r, "y").last().unwrap(), 0.0);
    let r = run(&model, &request(0.0, 1.0, Output::IntegratorSteps, 1));
    let (t, y) = (column(&r, "time"), column(&r, "y"));
    let k = t.len();
    assert_eq!((t[k - 2], t[k - 1]), (1.0, 1.0), "{t:?}");
    assert_eq!(y[k - 1], 0.0);
}

#[test]
fn equal_priority_seeded() {
    // two events of equal priority at the same time: the order is the generator's
    let model = ramp(vec![
        recorder_event("one", None, 1),
        recorder_event("two", None, 2),
    ]);
    let x = |seed: u64| {
        let r = run(
            &model,
            &request(0.0, 2.0, Output::Uniform { steps: 2 }, seed),
        );
        *column(&r, "x").last().unwrap()
    };
    let orders: Vec<f64> = (0..32).map(x).collect();
    // the same seed gives the same order, run after run and on an engine which is reused
    assert_eq!(orders, (0..32).map(x).collect::<Vec<_>>());
    let mut engine = Engine::new(Box::new(model.clone()), model.description()).unwrap();
    for (seed, order) in orders.iter().enumerate() {
        let r = engine
            .simulate(&request(
                0.0,
                2.0,
                Output::Uniform { steps: 2 },
                seed as u64,
            ))
            .unwrap();
        assert_eq!(column(&r, "x").last(), Some(order));
    }
    // both orders occur over the seeds (the probability of one order only is 2^-31)
    assert!(
        orders.iter().all(|x| *x == 12.0 || *x == 21.0),
        "{orders:?}"
    );
    assert!(
        orders.contains(&12.0) && orders.contains(&21.0),
        "{orders:?}"
    );
}

#[test]
fn events_at_the_start() {
    // a trigger which is true at the start fires there when its initial value is false
    let event = |initial_value: bool, trigger: fn(f64, &[f64], &[f64]) -> bool| {
        let mut e = TestEvent::new("start", trigger, vec![|t, _, _| t]);
        e.initial_value = initial_value;
        e.assignments = vec![(N, |_, _, p| p[1] + 1.0)];
        e
    };
    let req = request(0.0, 1.0, Output::Uniform { steps: 1 }, 1);
    let n = |e: TestEvent| column(&run(&ramp(vec![e]), &req), "n");
    assert_eq!(n(event(false, |t, _, _| t >= 0.0)), [1.0, 1.0]);
    assert_eq!(n(event(true, |t, _, _| t >= 0.0)), [0.0, 0.0]);
    // time > 0 becomes true right after the start: the output at the start is before it
    assert_eq!(n(event(false, |t, _, _| t > 0.0)), [0.0, 1.0]);
    assert_eq!(n(event(true, |t, _, _| t > 0.0)), [0.0, 1.0]);
}

#[test]
fn a_model_without_states_locates_its_triggers() {
    let mut event = after_one();
    event.assignments = vec![(X, |t, _, _| t)];
    let model = EventModel {
        states: vec![],
        parameters: vec![("x", 0.0)],
        rhs: |_, _, _, _| {},
        events: vec![event],
    };
    let r = run(&model, &request(0.0, 2.0, Output::Uniform { steps: 4 }, 1));
    let x = column(&r, "x");
    assert_eq!(&x[..3], [0.0, 0.0, 0.0]);
    assert!((x[3] - 1.0).abs() < 1e-9 && x[3] == x[4], "{x:?}");
}

#[test]
fn events_in_a_cycle_stop_at_the_limit() {
    // x < 0.5: x = 1 and x > 0.5: x = 0 trigger each other forever at the time 1
    let mut up = TestEvent::new(
        "up",
        |t, _, p| t >= 1.0 && p[0] < 0.5,
        vec![|t, _, _| t - 1.0],
    );
    up.assignments = vec![(X, |_, _, _| 1.0)];
    let mut down = TestEvent::new(
        "down",
        |t, _, p| t >= 1.0 && p[0] > 0.5,
        vec![|t, _, _| t - 1.0],
    );
    down.assignments = vec![(X, |_, _, _| 0.0)];
    let model = ramp(vec![up, down]);
    match simulate(
        &model,
        &model.description(),
        &request(0.0, 2.0, Output::Uniform { steps: 2 }, 1),
    ) {
        Err(EngineError::Integration {
            flag,
            time,
            message,
        }) => {
            assert_eq!(flag, EVENT_LIMIT_FLAG);
            assert!((time - 1.0).abs() < 1e-12, "{time}");
            assert!(message.contains("1000"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_description_of_other_sizes_is_refused() {
    let model = ramp(vec![after_one()]);
    let mut desc = model.description();
    desc.n_roots = 2;
    match Engine::new(Box::new(model), desc) {
        Err(EngineError::Request(message)) => {
            assert!(
                message.contains("1 root functions") && message.contains("2 root functions"),
                "{message}"
            );
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("accepted"),
    }
}

#[test]
fn a_delayed_event_executes_between_outputs_and_inside_a_step() {
    // time > 1 with the delay 0.3: x = y at the execution, 1.3
    let mut event = after_one();
    event.delay = Some(|_, _, _| 0.3);
    event.use_values_from_trigger_time = false;
    event.assignments = vec![(X, |_, y, _| y[0])];
    let model = ramp(vec![event]);
    // between the outputs 1 and 1.5
    let x = column(
        &run(&model, &request(0.0, 2.0, Output::Uniform { steps: 4 }, 1)),
        "x",
    );
    assert_eq!(&x[..3], [0.0, 0.0, 0.0]);
    assert!((x[3] - 1.3).abs() < 1e-9 && x[3] == x[4], "{x:?}");
    // the steps: two rows at 1 + 0.3, the stop time of CVODE, inside what would be a step
    let r = run(&model, &request(0.0, 2.0, Output::IntegratorSteps, 1));
    let (t, x) = (column(&r, "time"), column(&r, "x"));
    let pairs: Vec<usize> = (1..t.len()).filter(|&i| t[i] == t[i - 1]).collect();
    assert_eq!(pairs.len(), 1, "{t:?}");
    let i = pairs[0];
    assert_eq!(t[i], 1.0 + 0.3);
    assert_eq!(x[i - 1], 0.0);
    assert!((x[i] - 1.3).abs() < 1e-9, "{}", x[i]);
    // the steps around it are steps of their own: some step lies before 1.3 and after 1
    assert!(t.iter().any(|s| *s > 1.0 && *s < 1.3), "{t:?}");
}

#[test]
fn a_negative_or_nan_delay_is_an_error() {
    for delay in [(|_, _, _| -0.5) as Fun, |_, _, _| f64::NAN] {
        let mut event = after_one();
        event.delay = Some(delay);
        event.assignments = vec![(X, |_, _, _| 1.0)];
        let model = ramp(vec![event]);
        match simulate(
            &model,
            &model.description(),
            &request(0.0, 2.0, Output::Uniform { steps: 2 }, 1),
        ) {
            Err(EngineError::Integration {
                flag,
                time,
                message,
            }) => {
                assert_eq!(flag, DELAY_FLAG);
                assert!((time - 1.0).abs() < 1e-12, "{time}");
                assert!(
                    message.contains("delay") && message.contains("after_one"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn the_order_of_ties_does_not_depend_on_the_output() {
    let model = ramp(vec![
        recorder_event("one", None, 1),
        recorder_event("two", None, 2),
    ]);
    for seed in 0..16 {
        let uniform = run(
            &model,
            &request(0.0, 2.0, Output::Uniform { steps: 2 }, seed),
        );
        let steps = run(&model, &request(0.0, 2.0, Output::IntegratorSteps, seed));
        assert_eq!(
            column(&uniform, "x").last(),
            column(&steps, "x").last(),
            "seed {seed}"
        );
    }
}

#[test]
fn a_description_of_another_jacobian_is_refused() {
    // two states: the model's pattern is dense (4 entries), the description's diagonal (2)
    let model = EventModel {
        states: vec![("a", 0.0), ("b", 0.0)],
        parameters: vec![],
        rhs: |_, _, _, ydot| ydot.fill(0.0),
        events: vec![],
    };
    let mut desc = model.description();
    desc.jacobian = sbmlsim_abi::JacobianKind::Analytic(sbmlsim_abi::SparsityPattern {
        n: 2,
        col_ptr: vec![0, 1, 2],
        row_idx: vec![0, 1],
    });
    match Engine::new(Box::new(model), desc) {
        Err(EngineError::Request(message)) => assert!(
            message.contains("4 entries of the Jacobian")
                && message.contains("2 entries of the Jacobian"),
            "{message}"
        ),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("accepted"),
    }
}

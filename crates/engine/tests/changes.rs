//! The changes at times, the steady state before a simulation, and the independence of the
//! requests on one engine.

use sbmlsim_abi::{
    Change, IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SteadyState, Tolerance,
};
use sbmlsim_engine::model::{
    Compartmental, Decay, DerivedCompartment, EventModel, Fun, Target, TestEvent,
};
use sbmlsim_engine::{Engine, EngineError, STEADY_STATE_FLAG, simulate};

/// A request from `start` to `end` of the output, every selection, tight tolerances.
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
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

fn change(time: f64, values: &[(&str, f64)]) -> Change {
    Change {
        time,
        values: values.iter().map(|(id, v)| (id.to_string(), *v)).collect(),
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

fn decay(req: &SimulationRequest) -> SimulationResult {
    let model = Decay { k: 0.5 };
    simulate(&model, &model.description(), req).unwrap()
}

fn compartmental(req: &SimulationRequest) -> SimulationResult {
    let model = Compartmental { k: 0.0 };
    simulate(&model, &model.description(), req).unwrap()
}

fn events(model: &EventModel, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
    simulate(model, &model.description(), req)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8 * b.abs().max(1.0)
}

#[test]
fn change_gives_two_rows() {
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 10 });
    req.changes = vec![change(5.0, &[("y", 1.0)])];
    let result = decay(&req);
    assert_eq!(result.n_rows, 12);
    let time = column(&result, "time");
    let y = column(&result, "y");
    assert_eq!(time[5], 5.0);
    assert_eq!(time[6], 5.0);
    assert!(close(y[5], (-2.5f64).exp()), "{}", y[5]);
    assert_eq!(y[6], 1.0);
    // the next segment starts from the state after the change
    assert!(close(y[11], (-2.5f64).exp()), "{}", y[11]);
    assert!(close(y[4], (-2.0f64).exp()), "{}", y[4]);

    // the steps of the integrator: the state before and after the change at its time, the
    // segment before it ends exactly there
    req.output = Output::IntegratorSteps;
    let result = decay(&req);
    let time = column(&result, "time");
    let y = column(&result, "y");
    let at: Vec<usize> = (0..time.len()).filter(|i| time[*i] == 5.0).collect();
    assert_eq!(at.len(), 2, "{time:?}");
    assert!(close(y[at[0]], (-2.5f64).exp()));
    assert_eq!(y[at[1]], 1.0);
    assert_eq!(*time.last().unwrap(), 10.0);
    assert!(time.windows(2).all(|w| w[0] <= w[1]));

    // an output of times without the time of the change has no row of it
    req.output = Output::Times(vec![0.0, 4.0, 6.0, 10.0]);
    let result = decay(&req);
    assert_eq!(result.n_rows, 4);
    let y = column(&result, "y");
    assert!(close(y[2], (-0.5f64).exp()), "{}", y[2]);

    // a change of a parameter: the assignment rules follow it (t_half = ln 2 / k)
    req.output = Output::Uniform { steps: 2 };
    req.changes = vec![change(5.0, &[("k", 1.0)])];
    let result = decay(&req);
    assert_eq!(
        column(&result, "t_half"),
        [2f64.ln() / 0.5, 2f64.ln() / 0.5, 2f64.ln(), 2f64.ln()]
    );
    let y = column(&result, "y");
    assert!(close(y[3], (-2.5f64).exp() * (-5.0f64).exp()), "{}", y[3]);
}

#[test]
fn changes_at_the_start_and_at_the_end() {
    // a change at the start is applied before the output of the start: one row
    let mut req = request(0.0, 2.0, Output::Uniform { steps: 2 });
    req.changes = vec![change(0.0, &[("y", 2.0)])];
    let result = decay(&req);
    assert_eq!(result.n_rows, 3);
    assert_eq!(column(&result, "y")[0], 2.0);
    req.output = Output::IntegratorSteps;
    let result = decay(&req);
    assert_eq!(column(&result, "y")[0], 2.0);
    assert_eq!(
        column(&result, "time")[..2]
            .iter()
            .filter(|t| **t == 0.0)
            .count(),
        1
    );

    // a change at the end: the rows before and after it
    for output in [Output::Uniform { steps: 2 }, Output::IntegratorSteps] {
        let mut req = request(0.0, 2.0, output);
        req.changes = vec![change(2.0, &[("y", 3.0)])];
        let result = decay(&req);
        let (time, y) = (column(&result, "time"), column(&result, "y"));
        let n = result.n_rows;
        assert_eq!(&time[n - 2..], [2.0, 2.0]);
        assert!(close(y[n - 2], (-1.0f64).exp()), "{}", y[n - 2]);
        assert_eq!(y[n - 1], 3.0);
    }
}

#[test]
fn changes_are_simultaneous() {
    // the values of a change are numbers set at once: the order of the pairs does not matter,
    // also of a compartment and a concentration in it (the concentration is one in the new
    // compartment) or an amount in it (not rescaled with its compartment)
    let run = |values: &[(&str, f64)]| {
        let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
        req.changes = vec![change(5.0, values)];
        compartmental(&req)
    };
    let a = run(&[("C", 2.0), ("[S]", 3.0), ("H", 4.0)]);
    let b = run(&[("H", 4.0), ("[S]", 3.0), ("C", 2.0)]);
    assert_eq!(a, b);
    assert_eq!(column(&a, "S")[2], 6.0);
    assert_eq!(column(&a, "[S]")[2], 3.0);
    assert_eq!(column(&a, "H")[2], 4.0);
    let a = run(&[("S", 3.0), ("C", 2.0)]);
    let b = run(&[("C", 2.0), ("S", 3.0)]);
    assert_eq!(a, b);
    assert_eq!(column(&a, "S")[2], 3.0);
    assert_eq!(column(&a, "[S]")[2], 1.5);
    // two changes of one time are one change
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
    req.changes = vec![change(5.0, &[("S", 3.0)]), change(5.0, &[("C", 2.0)])];
    let c = compartmental(&req);
    assert_eq!(a, c);
    assert_eq!(c.n_rows, 4);
}

#[test]
fn compartment_change_keeps_concentration() {
    // probed with libroadrunner 2.11.1 through sbmlsim: the concentration species stored as an
    // amount and the one which stores its concentration (a rate rule) keep their
    // concentrations, the species of an amount (`hasOnlySubstanceUnits`) keeps its amount
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
    req.changes = vec![change(5.0, &[("C", 2.0)])];
    let result = compartmental(&req);
    let row = |i: usize| {
        let k = result.columns.len();
        result.values[i * k + 1..(i + 1) * k].to_vec()
    };
    // S, [S], R, [R], H, [H], C
    assert_eq!(row(1), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(row(2), [2.0, 1.0, 2.0, 1.0, 1.0, 0.5, 2.0]);
    assert_eq!(row(3), [2.0, 1.0, 2.0, 1.0, 1.0, 0.5, 2.0]);
    // the value of a species is its amount (`R`) or its concentration (`[R]`), whatever it
    // stores
    req.changes = vec![change(5.0, &[("C", 2.0), ("R", 3.0)])];
    let result = compartmental(&req);
    assert_eq!(column(&result, "R")[2], 3.0);
    assert_eq!(column(&result, "[R]")[2], 1.5);
    req.changes = vec![change(5.0, &[("[R]", 3.0), ("[H]", 3.0)])];
    let result = compartmental(&req);
    assert_eq!(column(&result, "[R]")[2], 3.0);
    assert_eq!(column(&result, "H")[2], 3.0);
    // with S decaying, the concentration before the change is kept
    let model = Compartmental { k: 0.5 };
    req.changes = vec![change(5.0, &[("C", 4.0)])];
    let result = simulate(&model, &model.description(), &req).unwrap();
    let (s, c) = (column(&result, "S"), column(&result, "[S]"));
    assert_eq!(s[2], s[1] / 1.0 * 4.0);
    assert_eq!(c[2], s[2] / 4.0);
}

/// `y' = b - y`, `y(0) = 0`, `b = 1`, `x = 0`, with events.
fn relax(events: Vec<TestEvent>) -> EventModel {
    EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![("b", 1.0), ("x", 0.0)],
        rhs: |_, y, p, ydot| ydot[0] = p[0] - y[0],
        events,
        discontinuities: vec![],
    }
}

#[test]
fn events_and_changes() {
    // probed with libroadrunner 2.11.1 through sbmlsim (a parameter `x` set to 1 at t = 5):
    // an event the change makes true fires with the change; an event of the time of the change
    // fires after the change (its values are the ones after it); a strict comparison of the
    // time fires right after the output of the change; an execution pending at the time of the
    // change fires after the output of the state after the change
    let x_above: TestEvent = TestEvent {
        assignments: vec![(Target::Param(0), (|_, _, _| 10.0) as Fun)],
        initial_value: false,
        ..TestEvent::new("x_above", |_, _, p| p[1] > 0.5, vec![|_, _, p| p[1] - 0.5])
    };
    let at_five = |strict: bool| TestEvent {
        assignments: vec![(Target::State(0), (|_, _, p| p[1] + 100.0) as Fun)],
        initial_value: false,
        use_values_from_trigger_time: false,
        ..TestEvent::new(
            if strict { "after_five" } else { "at_five" },
            if strict {
                |t, _, _| t > 5.0
            } else {
                |t, _, _| t >= 5.0
            },
            vec![|t, _, _| t - 5.0],
        )
    };
    let delayed = TestEvent {
        assignments: vec![(Target::State(0), (|_, _, p| p[1] + 200.0) as Fun)],
        initial_value: false,
        use_values_from_trigger_time: false,
        delay: Some(|_, _, _| 1.0),
        ..TestEvent::new("delayed", |t, _, _| t >= 4.0, vec![|t, _, _| t - 4.0])
    };
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
    req.changes = vec![change(5.0, &[("x", 1.0)])];

    let result = events(&relax(vec![x_above.clone(), at_five(false)]), &req).unwrap();
    assert_eq!(column(&result, "time"), [0.0, 5.0, 5.0, 10.0]);
    assert_eq!(column(&result, "b"), [1.0, 1.0, 10.0, 10.0]);
    assert_eq!(column(&result, "x"), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(column(&result, "y")[2], 101.0);

    let result = events(&relax(vec![at_five(true)]), &req).unwrap();
    let y = column(&result, "y");
    assert!(y[2] < 1.0, "{y:?}");
    assert!(
        (y[3] - (1.0 + 100.0 * (-5.0f64).exp())).abs() < 1e-6,
        "{y:?}"
    );

    let result = events(&relax(vec![delayed.clone()]), &req).unwrap();
    let y = column(&result, "y");
    assert!(y[2] < 1.0, "{y:?}");
    assert!(
        (y[3] - (1.0 + 200.0 * (-5.0f64).exp())).abs() < 1e-6,
        "{y:?}"
    );
    // in the steps of the integrator: before the change, after it, after the execution
    req.output = Output::IntegratorSteps;
    let result = events(&relax(vec![delayed]), &req).unwrap();
    let (time, y) = (column(&result, "time"), column(&result, "y"));
    let at: Vec<usize> = (0..time.len()).filter(|i| time[*i] == 5.0).collect();
    assert_eq!(at.len(), 3, "{time:?}");
    assert!(
        y[at[0]] < 1.0 && y[at[1]] == y[at[0]] && y[at[2]] == 201.0,
        "{y:?}"
    );

    // a change at the start: the events at the start (the initialization) execute first, then
    // the change and the events it triggers, before the output of the start
    let start_event = TestEvent {
        assignments: vec![(Target::Param(1), (|_, _, _| 7.0) as Fun)],
        initial_value: false,
        ..TestEvent::new("start", |_, _, _| true, vec![])
    };
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
    req.changes = vec![change(0.0, &[("x", 1.0)])];
    let result = events(&relax(vec![start_event, x_above]), &req).unwrap();
    assert_eq!(column(&result, "x"), [1.0, 1.0]);
    assert_eq!(column(&result, "b"), [10.0, 10.0]);
}

#[test]
fn a_delayed_event_which_a_change_triggers_is_skipped() {
    // as sbmlsim's fire_events: an event with a delay which a change makes true is not pending
    // and assigns nothing, at a change after the start and at the start, in both outputs, also
    // when an event the change executes makes it true; its trigger stays true afterwards and
    // the event never executes (y relaxes to b and never becomes 50)
    let x_delayed = TestEvent {
        assignments: vec![(Target::State(0), (|_, _, _| 50.0) as Fun)],
        initial_value: false,
        delay: Some(|_, _, _| 1.0),
        ..TestEvent::new(
            "x_delayed",
            |_, _, p| p[1] > 0.5,
            vec![|_, _, p| p[1] - 0.5],
        )
    };
    let x_above = TestEvent {
        assignments: vec![(Target::Param(0), (|_, _, _| 10.0) as Fun)],
        initial_value: false,
        ..TestEvent::new("x_above", |_, _, p| p[1] > 0.5, vec![|_, _, p| p[1] - 0.5])
    };
    let b_delayed = TestEvent {
        assignments: vec![(Target::State(0), (|_, _, _| 50.0) as Fun)],
        initial_value: false,
        delay: Some(|_, _, _| 1.0),
        ..TestEvent::new(
            "b_delayed",
            |_, _, p| p[0] > 5.0,
            vec![|_, _, p| p[0] - 5.0],
        )
    };
    for (model, b_after) in [
        (relax(vec![x_delayed.clone()]), 1.0),
        (relax(vec![x_above, b_delayed]), 10.0),
    ] {
        for at in [5.0, 0.0] {
            for output in [Output::Uniform { steps: 10 }, Output::IntegratorSteps] {
                let mut req = request(0.0, 10.0, output.clone());
                req.changes = vec![change(at, &[("x", 1.0)])];
                let result = events(&model, &req).unwrap();
                let (time, y, b) = (
                    column(&result, "time"),
                    column(&result, "y"),
                    column(&result, "b"),
                );
                assert!(y.iter().all(|v| *v < 10.0 + 1e-9), "{at} {output:?} {y:?}");
                assert_eq!(*b.last().unwrap(), b_after, "{at} {output:?}");
                // y at the end: the relaxation to b from the change on, without the event
                let start = if at == 5.0 {
                    1.0 - (-5.0f64).exp()
                } else {
                    0.0
                };
                let end = b_after + (start - b_after) * (-(10.0 - at)).exp();
                assert!(close(*y.last().unwrap(), end), "{at} {output:?} {y:?}");
                assert_eq!(*time.last().unwrap(), 10.0);
            }
        }
    }
    // the same event triggered by the pre-initialization (at the start): pending, y = 50 at 1
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
    req.preinit = vec![("x".to_string(), 1.0)];
    let result = events(&relax(vec![x_delayed]), &req).unwrap();
    let y = column(&result, "y");
    assert!(close(y[1], 1.0 + 49.0 * (-4.0f64).exp()), "{y:?}");
}

#[test]
fn a_skipped_delayed_event_executes_after_its_trigger_falls_and_rises_again() {
    // y < 0.5 with a delay of 1 sets x = 7 (true initially, initial value true: no execution at
    // the start); y rises past 0.5 at ln 2; the change at 2 sets y = 0 and makes the trigger
    // true: skipped; y rises past 0.5 again (the trigger falls); the change at 4 sets b = -1,
    // and y falls through 0.5 in the integration (the trigger rises): pending, x = 7 one time
    // unit later
    let y_below = TestEvent {
        assignments: vec![(Target::Param(1), (|_, _, _| 7.0) as Fun)],
        initial_value: true,
        delay: Some(|_, _, _| 1.0),
        ..TestEvent::new("y_below", |_, y, _| y[0] < 0.5, vec![|_, y, _| 0.5 - y[0]])
    };
    let model = relax(vec![y_below]);
    for output in [Output::Uniform { steps: 10 }, Output::IntegratorSteps] {
        let mut req = request(0.0, 10.0, output.clone());
        req.changes = vec![change(2.0, &[("y", 0.0)]), change(4.0, &[("b", -1.0)])];
        let result = events(&model, &req).unwrap();
        let (time, x) = (column(&result, "time"), column(&result, "x"));
        // y(4) = 1 - e^-2, then y = -1 + (2 - e^-2) e^-(t - 4) crosses 0.5 at 4 + ln((2 -
        // e^-2) / 1.5), x = 7 one later
        let cross = 4.0 + ((2.0 - (-2.0f64).exp()) / 1.5).ln();
        for (t, v) in time.iter().zip(&x) {
            if *t < cross + 1.0 - 1e-6 {
                assert_eq!(*v, 0.0, "{output:?} {t}");
            } else if *t > cross + 1.0 + 1e-6 {
                assert_eq!(*v, 7.0, "{output:?} {t}");
            }
        }
        assert_eq!(*x.last().unwrap(), 7.0, "{output:?}");
    }
}

#[test]
fn a_model_without_states_is_changed() {
    let model = EventModel {
        states: vec![],
        parameters: vec![("a", 1.0)],
        rhs: |_, _, _, _| {},
        events: vec![],
        discontinuities: vec![],
    };
    let mut req = request(0.0, 2.0, Output::Uniform { steps: 2 });
    req.changes = vec![change(1.0, &[("a", 2.0)])];
    let result = events(&model, &req).unwrap();
    assert_eq!(column(&result, "time"), [0.0, 1.0, 1.0, 2.0]);
    assert_eq!(column(&result, "a"), [1.0, 1.0, 2.0, 2.0]);
    req.output = Output::IntegratorSteps;
    let result = events(&model, &req).unwrap();
    assert_eq!(column(&result, "time"), [0.0, 1.0, 1.0, 2.0]);
    assert_eq!(column(&result, "a"), [1.0, 1.0, 2.0, 2.0]);
}

#[test]
fn steady_state_then_simulate() {
    // y' = b - y reaches y = b; the simulation starts there at its start
    let model = relax(vec![]);
    let mut req = request(2.0, 4.0, Output::Uniform { steps: 2 });
    req.steady_state = Some(SteadyState::default());
    let result = events(&model, &req).unwrap();
    assert_eq!(column(&result, "time"), [2.0, 3.0, 4.0]);
    for y in column(&result, "y") {
        // |1 - y| <= 1e-8 + 1e-6 y at the steady state
        assert!((1.0 - y).abs() <= 1e-8 + 1e-6 * y + 1e-9, "{y}");
    }
    // the pre-initialization of the steady state, with the one of the request
    req.preinit = vec![("x".to_string(), 5.0)];
    req.steady_state = Some(SteadyState {
        preinit: vec![("b".to_string(), 3.0)],
        ..SteadyState::default()
    });
    let result = events(&model, &req).unwrap();
    assert!(close(column(&result, "y")[0], 3.0) || (column(&result, "y")[0] - 3.0).abs() < 1e-5);
    assert_eq!(column(&result, "x"), [5.0, 5.0, 5.0]);
    assert_eq!(column(&result, "b"), [3.0, 3.0, 3.0]);
    // a change at the start comes after the steady state
    req.changes = vec![change(2.0, &[("y", 0.0)])];
    let result = events(&model, &req).unwrap();
    assert_eq!(column(&result, "y")[0], 0.0);

    // events are active during the steady state: `time >= 5` sets b = b + 1 at 5 (b = 2, the
    // steady state y = 2); the simulation from 0 starts with its trigger false (it was true at
    // the end of the steady state) and executes it again at 5
    let step = TestEvent {
        assignments: vec![(Target::Param(0), (|_, _, p| p[0] + 1.0) as Fun)],
        initial_value: false,
        ..TestEvent::new("step", |t, _, _| t >= 5.0, vec![|t, _, _| t - 5.0])
    };
    let mut req = request(0.0, 10.0, Output::Uniform { steps: 2 });
    req.steady_state = Some(SteadyState::default());
    let result = events(&relax(vec![step]), &req).unwrap();
    assert_eq!(column(&result, "b"), [2.0, 3.0, 3.0]);
    let y = column(&result, "y");
    assert!((y[0] - 2.0).abs() < 1e-5, "{y:?}");
    assert!(close(y[2], 3.0 - (-5.0f64).exp()) || (y[2] - (3.0 - (-5.0f64).exp())).abs() < 1e-5);

    // a model without events: Decay reaches 0 (its rate k y below 1e-8 + 1e-6 y)
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
    req.steady_state = Some(SteadyState::default());
    let y = column(&decay(&req), "y");
    assert!(y[0] * 0.5 <= 1e-8 + 1e-6 * y[0], "{y:?}");
}

#[test]
fn steady_state_not_reached_is_an_error() {
    // y' = 1 has no steady state
    let model = EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![],
        rhs: |_, _, _, ydot| ydot[0] = 1.0,
        events: vec![],
        discontinuities: vec![],
    };
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
    req.steady_state = Some(SteadyState {
        max_time: 50.0,
        ..SteadyState::default()
    });
    match events(&model, &req) {
        Err(EngineError::Integration {
            flag,
            time,
            message,
        }) => {
            assert_eq!(flag, STEADY_STATE_FLAG);
            assert!(flag < -1000, "distinct from the flags of CVODE");
            assert_eq!(time, 50.0);
            assert!(
                message.contains("steady state up to the time 50"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    // also with events
    let model = EventModel {
        events: vec![TestEvent::new(
            "e",
            |t, _, _| t > 3.0,
            vec![|t, _, _| t - 3.0],
        )],
        ..model
    };
    assert!(matches!(
        events(&model, &req),
        Err(EngineError::Integration {
            flag: STEADY_STATE_FLAG,
            ..
        })
    ));
}

#[test]
fn requests_on_one_instance_are_independent() {
    let tie = |id: &'static str, value: f64| -> TestEvent {
        let mut e = TestEvent::new(id, |t, _, _| t >= 1.0, vec![|t, _, _| t - 1.0]);
        e.assignments = vec![(
            Target::Param(1),
            if value > 1.5 {
                (|_, _, p| p[1] * 2.0 + 1.0) as Fun
            } else {
                (|_, _, p| p[1] * 3.0) as Fun
            },
        )];
        e.initial_value = false;
        e
    };
    let delayed = TestEvent {
        delay: Some(|_, _, _| 0.5),
        assignments: vec![(Target::State(0), (|_, y, _| y[0] + 1.0) as Fun)],
        ..TestEvent::new("delayed", |_, y, _| y[0] > 0.5, vec![|_, y, _| y[0] - 0.5])
    };
    let model = relax(vec![tie("a", 1.0), tie("b", 2.0), delayed]);
    let a = request(0.0, 3.0, Output::Uniform { steps: 30 });
    let mut b = request(-1.0, 4.0, Output::IntegratorSteps);
    b.preinit = vec![("x".to_string(), 2.0)];
    b.changes = vec![change(2.0, &[("y", 0.0), ("b", 2.0)])];
    b.steady_state = Some(SteadyState {
        preinit: vec![("b".to_string(), 0.25)],
        ..SteadyState::default()
    });
    b.seed = 7;
    b.settings.stiff = false;

    let fresh = |req: &SimulationRequest| {
        Engine::new(Box::new(model.clone()), model.description())
            .unwrap()
            .simulate(req)
            .unwrap()
    };
    let mut engine = Engine::new(Box::new(model.clone()), model.description()).unwrap();
    let a1 = engine.simulate(&a).unwrap();
    let b1 = engine.simulate(&b).unwrap();
    let a2 = engine.simulate(&a).unwrap();
    let b2 = engine.simulate(&b).unwrap();
    let bits = |r: &SimulationResult| r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a1), bits(&a2));
    assert_eq!(bits(&a1), bits(&fresh(&a)));
    assert_eq!(bits(&b1), bits(&b2));
    assert_eq!(bits(&b1), bits(&fresh(&b)));
    // b executed the tied events during its steady state (x = 2 of its pre-initialization,
    // both values taken at the trigger: 2 * 3 or 2 * 2 + 1, the later one wins) and its change
    let x = column(&b1, "x");
    assert!(x[0] == 6.0 || x[0] == 5.0, "{x:?}");
    assert!(column(&b1, "b").contains(&2.0));
    let (time, y) = (column(&b1, "time"), column(&b1, "y"));
    let after = (0..time.len()).filter(|i| time[*i] == 2.0).nth(1).unwrap();
    assert_eq!(y[after], 0.0);
}

#[test]
fn steady_state_from_time_zero() {
    // y' = (time < 50 ? 1 : 2) - y: integrated from 0 the steady state is 2 (the horizon to 100
    // passes 50), from -1000 it would be 1; sbmlsim integrates it from 0 whatever the start
    // (probed with sbmlsim: y = 1.9999999999999496 at the starts -1000, 0 and 200)
    let model = EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![],
        rhs: |t, y, _, ydot| ydot[0] = if t < 50.0 { 1.0 } else { 2.0 } - y[0],
        events: vec![],
        discontinuities: vec![],
    };
    for start in [-1000.0, 0.0, 200.0] {
        let mut req = request(start, start + 1.0, Output::Uniform { steps: 1 });
        req.steady_state = Some(SteadyState::default());
        let result = events(&model, &req).unwrap();
        assert_eq!(column(&result, "time"), [start, start + 1.0]);
        let y = column(&result, "y");
        assert!((y[0] - 2.0).abs() < 1e-9, "{start}: {y:?}");
    }
    // the events at the time 0 execute before the steady state: `time < 1` (initial value
    // false) is true at 0 and sets b = 3, the steady state is y = 3 for the start 100 too
    let early = TestEvent {
        assignments: vec![(Target::Param(0), (|_, _, _| 3.0) as Fun)],
        initial_value: false,
        ..TestEvent::new("early", |t, _, _| t < 1.0, vec![|t, _, _| 1.0 - t])
    };
    let mut req = request(100.0, 101.0, Output::Uniform { steps: 1 });
    req.steady_state = Some(SteadyState::default());
    let result = events(&relax(vec![early]), &req).unwrap();
    assert_eq!(column(&result, "b"), [3.0, 3.0]);
    assert!((column(&result, "y")[0] - 3.0).abs() < 1e-5);
}

#[test]
fn preinit_takes_the_selections_of_sbmlsim() {
    // probed with libroadrunner 2.11.1 through sbmlsim: `R` is the
    // amount and `[R]` the concentration of a species which stores its concentration, `[S]`
    // the concentration of one which stores its amount, in the compartment of the size the
    // initialization settles on, whatever the order of the values
    let run = |preinit: &[(&str, f64)]| {
        let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
        req.preinit = preinit.iter().map(|(id, v)| (id.to_string(), *v)).collect();
        let result = compartmental(&req);
        let k = result.columns.len();
        result.values[1..k].to_vec()
    };
    // S, [S], R, [R], H, [H], C
    assert_eq!(
        run(&[("C", 2.0), ("R", 3.0)]),
        [1.0, 0.5, 3.0, 1.5, 1.0, 0.5, 2.0]
    );
    assert_eq!(
        run(&[("C", 2.0), ("[R]", 3.0)]),
        [1.0, 0.5, 6.0, 3.0, 1.0, 0.5, 2.0]
    );
    assert_eq!(
        run(&[("C", 2.0), ("S", 3.0)]),
        [3.0, 1.5, 2.0, 1.0, 1.0, 0.5, 2.0]
    );
    assert_eq!(
        run(&[("C", 2.0), ("[S]", 3.0)]),
        [6.0, 3.0, 2.0, 1.0, 1.0, 0.5, 2.0]
    );
    // the order of the values does not matter: a concentration before its compartment is
    // converted with the size the compartment settles on as well (2, not the loaded 1), and so
    // is the amount of a species which stores its concentration
    assert_eq!(
        run(&[("[S]", 3.0), ("C", 2.0)]),
        run(&[("C", 2.0), ("[S]", 3.0)])
    );
    assert_eq!(
        run(&[("R", 3.0), ("C", 2.0)]),
        run(&[("C", 2.0), ("R", 3.0)])
    );
    for order in [[("[S]", 5.0), ("C", 3.0)], [("C", 3.0), ("[S]", 5.0)]] {
        assert_eq!(run(&order), [15.0, 5.0, 3.0, 1.0, 1.0, 1.0 / 3.0, 3.0]);
    }
    // the concentration of an entity which is no species is refused
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
    req.preinit = vec![("[C]".to_string(), 1.0)];
    let model = Compartmental { k: 0.0 };
    assert!(matches!(
        simulate(&model, &model.description(), &req),
        Err(EngineError::Request(m)) if m.contains("[C]")
    ));
}

/// The first row of [`DerivedCompartment`] after the pre-initialization: `A`, `[A]`, `N`,
/// `[N]`, `c`, `V`.
fn derived_compartment(preinit: &[(&str, f64)]) -> Vec<f64> {
    let mut req = request(0.0, 1.0, Output::Uniform { steps: 1 });
    req.preinit = preinit.iter().map(|(id, v)| (id.to_string(), *v)).collect();
    let model = DerivedCompartment;
    let result = simulate(&model, &model.description(), &req).unwrap();
    let k = result.columns.len();
    result.values[1..k].to_vec()
}

/// A pre-initialization holds the concentration of a species over the whole initialization,
/// also when the same pre-initialization changes the size of its compartment through an
/// initial assignment, and whatever the order of the values.
#[test]
fn preinit_holds_a_concentration_across_the_initialization() {
    // probed with libroadrunner 2.11.1 through sbmlsim on the same model: the compartment `c`
    // of the initial assignment `c = V f` with `V = 2 g`, the species `A` of a concentration
    // and `N` of an amount, both of the declared concentration 5 (so `A = N = 5 c`)
    let both = |a: (&str, f64), b: (&str, f64)| {
        let forward = derived_compartment(&[a, b]);
        assert_eq!(forward, derived_compartment(&[b, a]), "{a:?} and {b:?}");
        forward
    };
    // A, [A], N, [N], c, V
    // the size of the conversion is the one the initialization settles on, not the loaded 2
    assert_eq!(
        both(("[A]", 5.0), ("V", 3.0)),
        [15.0, 5.0, 15.0, 5.0, 3.0, 3.0]
    );
    assert_eq!(
        both(("A", 5.0), ("V", 3.0)),
        [5.0, 5.0 / 3.0, 15.0, 5.0, 3.0, 3.0]
    );
    // the chain `c = V f`, `V = 2 g`: a value at either end gives the size 6
    assert_eq!(
        both(("[A]", 5.0), ("f", 3.0)),
        [30.0, 5.0, 30.0, 5.0, 6.0, 2.0]
    );
    assert_eq!(
        both(("[A]", 5.0), ("g", 3.0)),
        [30.0, 5.0, 30.0, 5.0, 6.0, 6.0]
    );
    // a species of an amount (`hasOnlySubstanceUnits`) set by its concentration converts too
    assert_eq!(
        both(("[N]", 5.0), ("f", 3.0)),
        [30.0, 5.0, 30.0, 5.0, 6.0, 2.0]
    );
    // the size set directly, against the initial assignment which derives it
    assert_eq!(
        both(("[A]", 5.0), ("c", 6.0)),
        [30.0, 5.0, 30.0, 5.0, 6.0, 2.0]
    );
    // the cases without a conversion are not touched: an amount is the amount, and a value
    // which changes the size alone leaves the declared concentrations where they are
    assert_eq!(
        both(("A", 5.0), ("f", 3.0)),
        [5.0, 5.0 / 6.0, 30.0, 5.0, 6.0, 2.0]
    );
    assert_eq!(
        both(("N", 5.0), ("f", 3.0)),
        [30.0, 5.0, 5.0, 5.0 / 6.0, 6.0, 2.0]
    );
    assert_eq!(
        derived_compartment(&[("f", 3.0)]),
        [30.0, 5.0, 30.0, 5.0, 6.0, 2.0]
    );
    // and a concentration alone takes the size of the model as loaded
    assert_eq!(
        derived_compartment(&[("[A]", 5.0)]),
        [10.0, 5.0, 10.0, 5.0, 2.0, 2.0]
    );
}

/// Several values which need a size are converted against one state, whatever their order:
/// nothing a conversion writes is a size, which is the invariant the second pass of
/// `initial_values` rests on, and a target whose `compartment` were something other than that
/// of a species would break it.
#[test]
fn preinit_converts_several_concentrations_against_one_state() {
    let (a, n) = (("[A]", 5.0), ("[N]", 7.0));
    // with a third value which changes the size, through the chain `c = V f` or directly
    for third in [("f", 3.0), ("c", 6.0)] {
        for order in [
            [a, n, third],
            [a, third, n],
            [n, a, third],
            [n, third, a],
            [third, a, n],
            [third, n, a],
        ] {
            assert_eq!(
                derived_compartment(&order),
                [30.0, 5.0, 42.0, 7.0, 6.0, 2.0],
                "{order:?}"
            );
        }
    }
    // and without one, against the size of the model as loaded
    for order in [[a, n], [n, a]] {
        assert_eq!(
            derived_compartment(&order),
            [10.0, 5.0, 14.0, 7.0, 2.0, 2.0],
            "{order:?}"
        );
    }
}

/// `y' = -y`, `y(0) = 1`, with an event which never executes (its root function `time - 100`
/// is located, so the run takes the path of the events), or without events.
fn falling(with_event: bool) -> EventModel {
    EventModel {
        states: vec![("y", 1.0)],
        parameters: vec![],
        rhs: |_, y, _, ydot| ydot[0] = -y[0],
        events: if with_event {
            vec![TestEvent::new(
                "never",
                |t, _, _| t > 100.0,
                vec![|t, _, _| t - 100.0],
            )]
        } else {
            vec![]
        },
        discontinuities: vec![],
    }
}

#[test]
fn output_times_close_to_a_change() {
    let before = 5f64.next_down();
    let after = 5f64.next_up();
    assert!(before == 4.999999999999999 && after == 5.000000000000001);
    let e5 = (-5.0f64).exp();
    // an output time too close to a change to integrate to is before it, the time of the
    // change has the rows before and after it
    let mut req = request(0.0, 10.0, Output::Times(vec![0.0, before, 5.0, 10.0]));
    req.changes = vec![change(5.0, &[("y", 7.0)])];
    let mut results = Vec::new();
    for with_event in [false, true] {
        let result = events(&falling(with_event), &req).unwrap();
        assert_eq!(
            column(&result, "time"),
            [0.0, before, 5.0, 5.0, 10.0],
            "{with_event}"
        );
        let y = column(&result, "y");
        assert!(close(y[1], e5) && close(y[2], e5), "{with_event}: {y:?}");
        assert_eq!(y[3], 7.0, "{with_event}");
        assert!(close(y[4], 7.0 * e5), "{with_event}: {y:?}");
        results.push(y);
    }
    // two changes too close to integrate between them: each time has its rows before and
    // after its change
    let mut req = request(0.0, 10.0, Output::Times(vec![0.0, 5.0, after, 10.0]));
    req.changes = vec![change(5.0, &[("y", 7.0)]), change(after, &[("y", 8.0)])];
    for with_event in [false, true] {
        let result = events(&falling(with_event), &req).unwrap();
        assert_eq!(
            column(&result, "time"),
            [0.0, 5.0, 5.0, after, after, 10.0],
            "{with_event}"
        );
        let y = column(&result, "y");
        assert!(close(y[1], e5), "{with_event}: {y:?}");
        assert_eq!(y[2..5], [7.0, 7.0, 8.0], "{with_event}");
        assert!(close(y[5], 8.0 * e5), "{with_event}: {y:?}");
    }
}

#[test]
fn a_failed_request_leaves_the_engine_as_a_fresh_one() {
    // a steady state which is not reached, an integration which fails (one step), then a
    // request with events, a change and a steady state: equal to a fresh engine, bitwise
    let model = EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![("b", 1.0), ("x", 0.0)],
        rhs: |_, y, p, ydot| ydot[0] = p[0] - p[1] * y[0],
        events: vec![TestEvent {
            assignments: vec![(Target::Param(0), (|_, _, p| p[0] + 1.0) as Fun)],
            initial_value: false,
            ..TestEvent::new("step", |t, _, _| t >= 0.5, vec![|t, _, _| t - 0.5])
        }],
        discontinuities: vec![],
    };
    let mut never = request(0.0, 1.0, Output::Uniform { steps: 2 });
    never.steady_state = Some(SteadyState {
        max_time: 20.0,
        ..SteadyState::default()
    });
    let mut one_step = request(0.0, 10.0, Output::Uniform { steps: 2 });
    one_step.settings.maximum_num_steps = 1;
    let mut good = request(0.0, 2.0, Output::IntegratorSteps);
    good.preinit = vec![("x".to_string(), 1.0)];
    good.changes = vec![change(1.0, &[("y", 0.0)])];
    good.steady_state = Some(SteadyState::default());
    let mut engine = Engine::new(Box::new(model.clone()), model.description()).unwrap();
    assert!(matches!(
        engine.simulate(&never),
        Err(EngineError::Integration {
            flag: STEADY_STATE_FLAG,
            ..
        })
    ));
    assert!(matches!(
        engine.simulate(&one_step),
        Err(EngineError::Integration { flag: -1, .. })
    ));
    let after_failures = engine.simulate(&good).unwrap();
    let fresh = Engine::new(Box::new(model.clone()), model.description())
        .unwrap()
        .simulate(&good)
        .unwrap();
    let bits = |r: &SimulationResult| r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&after_failures), bits(&fresh));
}

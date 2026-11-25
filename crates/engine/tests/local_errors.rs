//! The local error estimates of the integrator at the rows of a result
//! ([`sbmlsim_abi::LocalErrors`]): the error test CVODE accepted every step with holds for
//! the estimates and the weights of its row, the weights are the ones of the state the step
//! started from, and a row which no step precedes is `NaN`.

use sbmlsim_abi::{
    IntegratorSettings, LocalErrors, Output, SIMULATION_CONTRACT, SimulationRequest,
    SimulationResult, Tolerance,
};
use sbmlsim_engine::model::{Decay, EventModel, Robertson, Target, TestEvent};
use sbmlsim_engine::simulate;

const RTOL: f64 = 1e-6;
const ATOL: f64 = 1e-10;

/// A request over `[0, end]` with the local errors of the rows.
fn request(end: f64, output: Output, local_errors: bool) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec![],
        settings: IntegratorSettings {
            relative_tolerance: RTOL,
            absolute_tolerance: Tolerance::Scalar(ATOL),
            ..IntegratorSettings::default()
        },
        seed: 0,
        sensitivities: vec![],
        local_errors,
    }
}

/// The local errors of a result.
fn local(result: &SimulationResult) -> &LocalErrors {
    result
        .local_errors
        .as_ref()
        .expect("the result has no local errors")
}

/// The weighted root mean square norm of the estimates of a row, which CVODE accepted the
/// step with when it is at most 1.
fn norm(local: &LocalErrors, row: usize) -> f64 {
    let n = local.states.len();
    let sum: f64 = (0..n)
        .map(|s| (local.errors[row * n + s] * local.weights[row * n + s]).powi(2))
        .sum();
    (sum / n as f64).sqrt()
}

#[test]
fn a_request_without_the_flag_has_none() {
    let model = Robertson;
    let result = simulate(
        &model,
        &model.description(),
        &request(40.0, Output::Uniform { steps: 4 }, false),
    )
    .unwrap();
    assert!(result.local_errors.is_none());
}

#[test]
fn every_row_after_a_step_passed_the_error_test_of_that_step() {
    let model = Robertson;
    for output in [Output::Uniform { steps: 40 }, Output::IntegratorSteps] {
        let result = simulate(&model, &model.description(), &request(40.0, output, true)).unwrap();
        let local = local(&result);
        assert_eq!(local.states, ["y1", "y2", "y3"]);
        assert_eq!(local.errors.len(), result.n_rows * 3);
        assert_eq!(local.weights.len(), result.n_rows * 3);
        // the start: no step precedes it
        assert!(
            local.errors[..3]
                .iter()
                .chain(&local.weights[..3])
                .all(|x| x.is_nan())
        );
        for row in 1..result.n_rows {
            let norm = norm(local, row);
            assert!(norm.is_finite() && norm <= 1.0, "row {row}: {norm}");
            assert!(local.weights[row * 3..][..3].iter().all(|&w| w > 0.0));
        }
        // a step of the error test is not a step of no error
        assert!((1..result.n_rows).any(|row| norm(local, row) > 1e-3));
    }
}

#[test]
fn the_weights_are_the_ones_of_the_state_the_step_started_from() {
    // in the steps of the integrator a row is the end of a step and the row before it its
    // start, whose state CVODE weighs its error test with: `1 / (rtol |y| + atol)`
    let model = Decay { k: 0.7 };
    let result = simulate(
        &model,
        &model.description(),
        &request(5.0, Output::IntegratorSteps, true),
    )
    .unwrap();
    let local = local(&result);
    let width = result.columns.len();
    let y = result
        .columns
        .iter()
        .position(|column| column == "y")
        .expect("the column y");
    assert!(result.n_rows > 10, "{} steps", result.n_rows);
    for row in 1..result.n_rows {
        let before = result.values[(row - 1) * width + y];
        let weight = 1.0 / (RTOL * before.abs() + ATOL);
        assert_eq!(local.weights[row].to_bits(), weight.to_bits(), "row {row}");
    }
}

#[test]
fn a_row_at_an_output_time_has_the_step_which_passed_it() {
    // the rows of an output of times between two steps are interpolated in the last step,
    // whose estimates they carry: two rows in one step carry the same
    let model = Decay { k: 0.7 };
    let result = simulate(
        &model,
        &model.description(),
        &request(5.0, Output::Uniform { steps: 1000 }, true),
    )
    .unwrap();
    let local = local(&result);
    let repeated = (2..result.n_rows)
        .filter(|&row| local.errors[row].to_bits() == local.errors[row - 1].to_bits())
        .count();
    assert!(
        repeated > 500,
        "{repeated} rows share the step of the row before them"
    );
}

#[test]
fn a_model_with_events_carries_them_through_its_events() {
    // `y' = -y` from 1, set to 1 again at `time = 1`
    let mut reset = TestEvent::new("reset", |t, _, _| t > 1.0, vec![|t, _, _| t - 1.0]);
    reset.initial_value = false;
    reset.assignments = vec![(Target::State(0), |_, _, _| 1.0)];
    let model = EventModel {
        states: vec![("y", 1.0)],
        parameters: vec![],
        rhs: |_, y, _, ydot| ydot[0] = -y[0],
        events: vec![reset],
        discontinuities: vec![],
    };
    let result = simulate(
        &model,
        &model.description(),
        &request(5.0, Output::Uniform { steps: 50 }, true),
    )
    .unwrap();
    let local = local(&result);
    assert_eq!(local.states, ["y"]);
    assert_eq!(local.errors.len(), result.n_rows);
    // the event executed: the state is back near 1 after `time = 1`
    assert!(result.values[(result.n_rows - 1) * result.columns.len() + 1] > 0.01);
    for row in 1..result.n_rows {
        let norm = norm(local, row);
        assert!(norm.is_finite() && norm <= 1.0, "row {row}: {norm}");
    }
}

#[test]
fn a_run_of_no_step_is_nan() {
    // a single time is the initial state, which no integration precedes
    let model = Robertson;
    let result = simulate(
        &model,
        &model.description(),
        &request(0.0, Output::Times(vec![0.0, 0.0]), true),
    )
    .unwrap();
    let local = local(&result);
    assert_eq!(result.n_rows, 2);
    assert!(
        local
            .errors
            .iter()
            .chain(&local.weights)
            .all(|x| x.is_nan())
    );
}

//! The forward sensitivities of a simulation: the states, the selections which are no state
//! and the initial values, against the closed form of the decay model.
//!
//! `y' = -k y` with `y(0) = y0` is `y(t) = y0 exp(-k t)`, so
//!
//! - `dy/dk = -t y0 exp(-k t)`, the sensitivity of a state of a parameter of the right hand
//!   side,
//! - `dy/dy0 = exp(-k t)`, which is carried by the initial values of the sensitivities and not
//!   by their right hand side,
//! - `d t_half/dk = -ln(2) / k^2`, the sensitivity of a selection which is no state at all
//!   (`t_half = ln(2) / k` is a constant of the model behind its parameters), and
//!   `d t_half/dy0 = 0`.

use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
};
use sbmlsim_engine::model::{Decay, Derived, EventModel, TestEvent};
use sbmlsim_engine::{Engine, EngineError, simulate};

/// The rate of the data and the initial value of `y`.
const K: f64 = 0.3;

/// A request of the decay model with the sensitivities of the given targets.
fn request(sensitivities: &[&str], stiff: bool) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 10.0,
        output: Output::Uniform { steps: 10 },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec!["time".to_string(), "y".to_string(), "t_half".to_string()],
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            stiff,
            ..IntegratorSettings::default()
        },
        seed: 0,
        sensitivities: sensitivities.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Simulates the decay model with the rate `K`.
fn decay(request: &SimulationRequest) -> Result<SimulationResult, EngineError> {
    let model = Decay { k: K };
    simulate(&model, &model.description(), request)
}

/// The sensitivity of the column `column` of the row `row` to the parameter `parameter`.
fn sensitivity(result: &SimulationResult, row: usize, column: &str, parameter: usize) -> f64 {
    let sensitivities = result
        .sensitivities
        .as_ref()
        .expect("the result has no sensitivities");
    let at = result
        .columns
        .iter()
        .position(|name| name == column)
        .unwrap_or_else(|| panic!("no column {column}"));
    let k = sensitivities.parameters.len();
    sensitivities.values[(row * result.columns.len() + at) * k + parameter]
}

/// The time of a row.
fn time(result: &SimulationResult, row: usize) -> f64 {
    result.values[row * result.columns.len()]
}

#[test]
fn the_sensitivities_of_a_state_are_the_ones_of_the_solution() {
    let result = decay(&request(&["k", "y"], true)).unwrap();

    let sensitivities = result.sensitivities.as_ref().unwrap();
    assert_eq!(sensitivities.parameters, ["k", "y"]);
    assert_eq!(
        sensitivities.values.len(),
        result.n_rows * result.columns.len() * 2
    );
    for row in 0..result.n_rows {
        let t = time(&result, row);
        let decay = (-K * t).exp();
        let (of_k, of_y0) = (
            sensitivity(&result, row, "y", 0),
            sensitivity(&result, row, "y", 1),
        );
        assert!(
            (of_k - (-t * decay)).abs() < 1e-6 * (t * decay).max(1e-3),
            "t = {t}: dy/dk is {of_k} and not {}",
            -t * decay
        );
        assert!(
            (of_y0 - decay).abs() < 1e-6 * decay.max(1e-3),
            "t = {t}: dy/dy0 is {of_y0} and not {decay}"
        );
    }
    // the time of a row is no function of a parameter
    for row in 0..result.n_rows {
        assert_eq!(sensitivity(&result, row, "time", 0), 0.0);
        assert_eq!(sensitivity(&result, row, "time", 1), 0.0);
    }
}

#[test]
fn the_sensitivity_of_a_selection_which_is_no_state_is_the_one_of_its_value() {
    let result = decay(&request(&["k", "y"], true)).unwrap();

    // `t_half = ln(2) / k` is a constant of the model, so its sensitivity is the same in
    // every row and the one of the initial value of `y` is nothing
    let wanted = -2f64.ln() / (K * K);
    for row in 0..result.n_rows {
        let of_k = sensitivity(&result, row, "t_half", 0);
        assert!(
            (of_k / wanted - 1.0).abs() < 1e-5,
            "row {row}: d t_half/dk is {of_k} and not {wanted}"
        );
        assert_eq!(sensitivity(&result, row, "t_half", 1), 0.0);
    }
}

#[test]
fn the_sensitivities_of_a_run_which_is_not_stiff_are_the_same_ones() {
    let stiff = decay(&request(&["k"], true)).unwrap();
    let adams = decay(&request(&["k"], false)).unwrap();

    for row in 0..stiff.n_rows {
        let (one, two) = (
            sensitivity(&stiff, row, "y", 0),
            sensitivity(&adams, row, "y", 0),
        );
        assert!(
            (one - two).abs() < 1e-6 * one.abs().max(1e-3),
            "row {row}: the stiff run says {one} and the non-stiff one {two}"
        );
    }
}

#[test]
fn a_run_without_sensitivities_has_no_table_of_them() {
    let result = decay(&request(&[], true)).unwrap();

    assert!(result.sensitivities.is_none());
}

#[test]
fn a_sensitivity_of_a_target_the_model_has_not_is_refused() {
    let refusal = decay(&request(&["clearance"], true)).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("clearance"), "{message}");
}

#[test]
fn a_sensitivity_of_a_value_which_cannot_be_set_is_refused() {
    // `t_half` is a constant behind the parameters of the model, which a request cannot set,
    // so a sensitivity of it is no sensitivity of the model
    let refusal = decay(&request(&["t_half"], true)).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("t_half"), "{message}");
}

#[test]
fn a_sensitivity_of_the_same_target_twice_is_refused() {
    let refusal = decay(&request(&["k", "k"], true)).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("k"), "{message}");
}

#[test]
fn a_sensitivity_of_a_concentration_is_refused() {
    let refusal = decay(&request(&["[y]"], true)).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("[y]"), "{message}");
}

#[test]
fn a_sensitivity_of_a_run_with_a_change_is_refused() {
    let mut request = request(&["k"], true);
    request.changes = vec![sbmlsim_abi::Change {
        time: 5.0,
        values: vec![("y".to_string(), 2.0)],
        increments: vec![],
        forms: Vec::new(),
    }];

    let refusal = decay(&request).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("change"), "{message}");
}

#[test]
fn a_sensitivity_of_a_model_with_events_is_refused() {
    let model = EventModel {
        states: vec![("y", 0.0)],
        parameters: vec![("k", 1.0)],
        rhs: |_, _, p, ydot| ydot[0] = p[0],
        events: vec![TestEvent::new(
            "after_one",
            |t, _, _| t > 1.0,
            vec![|t, _, _| t - 1.0],
        )],
        discontinuities: vec![],
    };
    let mut request = request(&["k"], true);
    request.selections = vec!["time".to_string()];

    let refusal = simulate(&model, &model.description(), &request).unwrap_err();

    let message = refusal.to_string();
    assert!(message.contains("event"), "{message}");
}

#[test]
fn an_engine_runs_one_sensitivity_after_another() {
    // the memory of the integrator is kept across the simulations of an engine, and CVODES
    // keeps the user data of the sensitivities as it is when they are initialized: the
    // problem of the first simulation must not be the one of the second
    let model = Decay { k: K };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
    let request = request(&["k"], true);

    let first = engine.simulate(&request).unwrap();
    // the second one from a deeper stack, where the problem of the simulation is at another
    // address than the one of the first: a user data which is the problem of the simulation
    // which initialized the sensitivities points at nothing here
    let second = deeper(24, &mut || engine.simulate(&request).unwrap());

    let (first, second) = (first.sensitivities.unwrap(), second.sensitivities.unwrap());
    assert_eq!(
        first.values, second.values,
        "the second run of sensitivities on the engine"
    );
}

/// Runs `what` behind `frames` frames of the stack, each of them a page of its own, so that
/// what it calls does not reuse the stack of the call before it.
fn deeper<T>(frames: usize, what: &mut impl FnMut() -> T) -> T {
    let mut page = [0u64; 512];
    page[frames % page.len()] = frames as u64;
    let out = if frames == 0 {
        what()
    } else {
        deeper(frames - 1, what)
    };
    std::hint::black_box(&page);
    out
}

#[test]
fn an_engine_which_ran_a_sensitivity_runs_a_simulation_after_it() {
    let model = Decay { k: K };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();

    let with = engine.simulate(&request(&["k"], true)).unwrap();
    let without = engine.simulate(&request(&[], true)).unwrap();

    assert!(with.sensitivities.is_some() && without.sensitivities.is_none());
    // and the states of the run of sensitivities are the states of the run without them to
    // the tolerance of the integrator: the staggered corrector solves them by themselves, and
    // what is left is the bookkeeping of CVODES around the solves of the sensitivities
    assert_eq!(with.values.len(), without.values.len());
    for (at, (one, other)) in with.values.iter().zip(&without.values).enumerate() {
        let scale = one.abs().max(other.abs()).max(1e-12);
        assert!(
            (one - other).abs() / scale < 1e-8,
            "the value {at} of the run of sensitivities is {one} and of the run without it {other}"
        );
    }
}

#[test]
fn the_sensitivity_of_a_parameter_an_initial_assignment_reads_is_the_one_of_the_chain() {
    // `y' = -a y` with the initial assignment `a = 2 k`: the right hand side does not read `k`
    // at all, so `df/dk` is nothing and the whole sensitivity of `k` is the one `a` carries,
    // `dy/dk = da/dk * dy/da = 2 * (-t y)`. The initialization of the run is what knows
    // `da/dk`, the same difference the initial values of the sensitivities are taken from.
    let model = Derived { k: K };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
    let mut request = request(&["k"], true);
    request.selections = vec!["time".to_string(), "y".to_string()];

    let result = engine.simulate(&request).unwrap();

    for row in 0..result.n_rows {
        let t = time(&result, row);
        let wanted = -2.0 * t * (-2.0 * K * t).exp();
        let found = sensitivity(&result, row, "y", 0);
        assert!(
            (found - wanted).abs() < 1e-6 * wanted.abs().max(1e-3),
            "dy/dk at {t} is {found} and not {wanted}"
        );
    }
}

#[test]
fn the_sensitivity_of_a_selection_an_initial_assignment_derives_is_the_one_of_the_chain() {
    // the selection `a` of the same model is no state, so its sensitivity is a difference of
    // `model_selections` along what the step of `k` moves the parameters by, `da/dk = 2`; the
    // step of `k` alone would move nothing of `a`, which no assignment rule recomputes
    let model = Derived { k: K };
    let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
    let mut request = request(&["k"], true);
    request.selections = vec!["time".to_string(), "y".to_string(), "a".to_string()];

    let result = engine.simulate(&request).unwrap();

    for row in 0..result.n_rows {
        let found = sensitivity(&result, row, "a", 0);
        assert!(
            (found - 2.0).abs() < 1e-6,
            "da/dk of the row {row} is {found} and not 2"
        );
    }
}

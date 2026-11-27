//! The values of a condition which follow the parameters of a fit.
//!
//! A value of a request of `simulation/1` is a number, so a condition whose value is a
//! parameter the fit estimates could not be a condition of a fit: a number folded when the
//! request was built holds that value where the fit started while the optimizer moves the
//! parameter. Such a value is a [`Derived`] of its condition, a linear form over the
//! parameters which the fit evaluates before every simulation.
//!
//! The decay model `y' = -k y` carries them: the initial value of `y` follows `k`, which is
//! the shape of a PEtab condition that assigns a species an estimated parameter. What the
//! tests hold the fit to is that the value is **not frozen**: the residuals at a point of the
//! search space are the ones of the model simulated with the value that point gives.

use sbmlsim_abi::SimulationRequest;
use sbmlsim_fit::{
    Condition, Curve, Definition, Derived, Parameter, Problem, Scale, Settings, Term,
};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, model, request};

/// The constant and the factor of the derived initial value of `y`, `y = CONSTANT + FACTOR k`.
const CONSTANT: f64 = 0.4;
const FACTOR: f64 = 2.0;

/// The initial value of `y` at a value of `k`.
fn y_of(k: f64) -> f64 {
    CONSTANT + FACTOR * k
}

/// The model at a value of `k`, with the initial value of `y` the derived value gives: the
/// times and the values of `y`, simulated through the same engine and the same request.
fn data(k: f64) -> (Vec<f64>, Vec<f64>) {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let varying = ["k".to_string(), "y".to_string()];
    let prepared = model.prepare(&request(), &varying).unwrap();
    let table = instance
        .simulate_prepared(&prepared, &[k, y_of(k)])
        .unwrap();
    let columns = table.columns.len();
    let time = table.columns.iter().position(|c| c == "time").unwrap();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    (
        (0..table.n_rows)
            .map(|row| table.values[row * columns + time])
            .collect(),
        (0..table.n_rows)
            .map(|row| table.values[row * columns + y])
            .collect(),
    )
}

/// The condition of the request with the derived initial value of `y`.
fn condition() -> Condition {
    Condition {
        request: request(),
        model: None,
        derived: vec![Derived {
            target: "y".to_string(),
            constant: CONSTANT,
            terms: vec![Term {
                parameter: 0,
                factor: FACTOR,
            }],
        }],
    }
}

/// The fit of `k` whose condition carries the derived initial value of `y`.
fn definition(start: f64) -> Definition {
    let (times, values) = data(K);
    Definition {
        jacobian: Default::default(),
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![Parameter {
            id: "k".to_string(),
            start,
            lower: 0.01,
            upper: 10.0,
            scale: Scale::Lin,
        }],
        conditions: vec![condition()],
        curves: vec![Curve {
            condition: 0,
            column: "y".to_string(),
            times,
            values,
            noise: None,
            weight: 1.0,
            after_change: None,
            likelihood: None,
            reduction: None,
        }],
    }
}

fn problem(start: f64) -> Problem {
    Problem::new(definition(start), model(), 1).unwrap()
}

/// The message of a definition the fit refuses.
fn refusal(definition: Definition) -> String {
    match Problem::new(definition, model(), 1) {
        Ok(_) => panic!("the definition was accepted"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn a_derived_value_is_the_data_of_the_fit_at_its_parameter() {
    let problem = problem(K);

    let residuals = problem.residuals(&[K]).unwrap();

    // the data is the model at `K` with the value the derived form gives there
    assert!(!residuals.is_empty());
    assert!(
        residuals.iter().all(|r| *r == 0.0),
        "the residuals at the value of the data are {residuals:?}"
    );
}

#[test]
fn a_derived_value_follows_the_parameter_and_is_not_frozen() {
    let problem = problem(K);
    let other = 0.5;
    // the model at `other`, whose initial value of `y` is the one of `other` and not of `K`
    let (_, expected) = data(other);
    let (_, measured) = data(K);

    let residuals = problem.residuals(&[other]).unwrap();

    let want: Vec<f64> = expected
        .iter()
        .zip(measured.iter())
        .map(|(simulated, value)| simulated - value)
        .collect();
    assert_eq!(residuals, want);
    // a value frozen at the start of the fit would give these instead
    let frozen = simulated_with(other, y_of(K));
    let stale: Vec<f64> = frozen
        .iter()
        .zip(measured.iter())
        .map(|(simulated, value)| simulated - value)
        .collect();
    assert_ne!(residuals, stale);
}

/// The values of `y` of the model at a value of `k` and an initial value of `y`.
fn simulated_with(k: f64, y0: f64) -> Vec<f64> {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let varying = ["k".to_string(), "y".to_string()];
    let prepared = model.prepare(&request(), &varying).unwrap();
    let table = instance.simulate_prepared(&prepared, &[k, y0]).unwrap();
    let columns = table.columns.len();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    (0..table.n_rows)
        .map(|row| table.values[row * columns + y])
        .collect()
}

#[test]
fn a_fit_finds_the_parameter_its_derived_value_follows() {
    let problem = problem(1.5);

    let fit = problem.optimize(&Settings::default()).unwrap();

    assert!(
        (fit.values[0] - K).abs() < 1e-6,
        "the fit ended at {:?}, not at {K}",
        fit.values
    );
}

#[test]
fn a_derived_value_of_a_constant_alone_is_set_on_its_target() {
    let mut definition = definition(K);
    definition.conditions[0].derived[0].terms.clear();
    definition.conditions[0].derived[0].constant = y_of(K);
    let problem = Problem::new(definition, model(), 1).unwrap();

    let residuals = problem.residuals(&[K]).unwrap();

    // the constant is the value the data was simulated at, so the residuals are zero
    assert!(residuals.iter().all(|r| *r == 0.0), "{residuals:?}");
}

#[test]
fn a_derived_value_of_a_target_of_a_parameter_is_refused() {
    let mut definition = definition(K);
    definition.conditions[0].derived[0].target = "k".to_string();

    let refusal = refusal(definition);

    assert!(
        refusal.contains("as a parameter or a derived value already"),
        "{refusal}"
    );
}

#[test]
fn two_derived_values_of_one_target_are_refused() {
    let mut definition = definition(K);
    let value = definition.conditions[0].derived[0].clone();
    definition.conditions[0].derived.push(value);

    let refusal = refusal(definition);

    assert!(
        refusal.contains("as a parameter or a derived value already"),
        "{refusal}"
    );
}

#[test]
fn a_derived_value_which_reads_a_parameter_the_fit_has_not_is_refused() {
    let mut definition = definition(K);
    definition.conditions[0].derived[0].terms[0].parameter = 7;

    let refusal = refusal(definition);

    assert!(
        refusal.contains("reads the parameter 7 of a fit of 1 parameters"),
        "{refusal}"
    );
}

#[test]
fn a_derived_value_of_a_factor_which_is_not_finite_is_refused() {
    let mut definition = definition(K);
    definition.conditions[0].derived[0].terms[0].factor = f64::NAN;

    let refusal = refusal(definition);

    assert!(refusal.contains("which is not finite"), "{refusal}");
}

#[test]
fn a_derived_value_without_a_target_is_refused() {
    let mut definition = definition(K);
    definition.conditions[0].derived[0].target = String::new();

    let refusal = refusal(definition);

    assert!(refusal.contains("without a target"), "{refusal}");
}

#[test]
fn a_condition_of_numbers_alone_carries_no_derived_value() {
    let request: SimulationRequest = request();

    let condition: Condition = request.into();

    assert!(condition.derived.is_empty());
}

//! The jacobian of a fit out of the forward sensitivities of its conditions: one simulation
//! per condition instead of one per parameter and condition, and the derivative of a residual
//! instead of a difference of two of them.
//!
//! The fit is the decay model `y' = -k y` of the fixture, fitted in `k` and the initial value
//! of `y` against the model at `k = 0.3` and `y = 1`.

use sbmlsim_abi::Change;
use sbmlsim_fit::{Definition, JacobianChoice, Problem, Scale, Settings};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{model, pair};

/// The jacobian of the residuals of a problem by a central difference of the residuals
/// themselves, which is accurate to about 1e-9 of itself and is what the sensitivities of a
/// run are checked against.
fn central(problem: &Problem, x: &[f64]) -> Vec<f64> {
    let k = problem.parameters().len();
    let n = problem.n_residuals();
    let mut jacobian = vec![0.0; n * k];
    for column in 0..k {
        let step = 1e-6 * x[column].abs().max(1.0);
        let (mut below, mut above) = (x.to_vec(), x.to_vec());
        below[column] -= step;
        above[column] += step;
        let (below, above) = (
            problem.residuals(&below).unwrap(),
            problem.residuals(&above).unwrap(),
        );
        for row in 0..n {
            jacobian[row * k + column] = (above[row] - below[row]) / (2.0 * step);
        }
    }
    jacobian
}

/// The largest relative difference of two jacobians of the same shape.
fn apart(one: &[f64], other: &[f64]) -> f64 {
    assert_eq!(one.len(), other.len());
    one.iter()
        .zip(other)
        .map(|(a, b)| (a - b).abs() / a.abs().max(b.abs()).max(1e-12))
        .fold(0.0, f64::max)
}

/// The definition of the pair which asks for the jacobian of the sensitivities, which is not
/// the default ([`JacobianChoice::Auto`] takes the differences).
fn sensitive(scale: Scale) -> Definition {
    let mut definition = pair(0.5, 2.0, scale);
    definition.jacobian = JacobianChoice::Sensitivities;
    definition
}

/// The definition of the pair with a change of `y` during the run, which no sensitivity of a
/// run carries.
fn with_a_change(scale: Scale) -> Definition {
    let mut definition = pair(0.5, 2.0, scale);
    definition.conditions[0].request.changes = vec![Change {
        time: 5.0,
        values: vec![("y".to_string(), 0.5)],
        increments: vec![],
        forms: Vec::new(),
    }];
    definition
}

#[test]
fn the_jacobian_of_a_fit_is_one_simulation_per_condition() {
    let problem = Problem::new(sensitive(Scale::Log10), model(), 2).unwrap();
    assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
    let x = problem.start();

    let before = problem.simulations();
    let jacobian = problem.jacobian(&x).unwrap();

    assert_eq!(
        problem.simulations() - before,
        1,
        "the jacobian of two parameters of one condition is not one simulation"
    );
    assert_eq!(jacobian.len(), problem.n_residuals() * 2);
}

#[test]
fn the_jacobian_of_the_sensitivities_is_the_one_of_the_residuals() {
    for scale in [Scale::Lin, Scale::Log10] {
        let problem = Problem::new(sensitive(scale), model(), 2).unwrap();
        let x = problem.start();

        let sensitivities = problem.jacobian(&x).unwrap();

        let apart = apart(&sensitivities, &central(&problem, &x));
        assert!(
            apart < 1e-6,
            "the jacobian of the sensitivities on the {scale:?} scale is {apart} off the one of \
             the residuals"
        );
    }
}

#[test]
fn the_jacobian_of_the_differences_is_the_one_of_the_residuals_as_well() {
    // the finite differences of sbmlsim, which step by 1e-3 of the parameter: the same
    // jacobian to about that step
    let mut definition = pair(0.5, 2.0, Scale::Log10);
    definition.jacobian = JacobianChoice::DifferenceQuotient;
    let problem = Problem::new(definition, model(), 2).unwrap();
    let x = problem.start();

    let before = problem.simulations();
    let differences = problem.jacobian(&x).unwrap();

    assert_eq!(problem.jacobian_of(), JacobianChoice::DifferenceQuotient);
    assert_eq!(
        problem.simulations() - before,
        3,
        "the differences of two parameters of one condition are not three simulations"
    );
    let apart = apart(&differences, &central(&problem, &x));
    assert!(apart < 1e-2, "the differences are {apart} off");
}

#[test]
fn the_fits_of_both_jacobians_find_the_same_optimum() {
    let mut differences = pair(0.5, 2.0, Scale::Log10);
    differences.jacobian = JacobianChoice::DifferenceQuotient;
    let sensitivities = Problem::new(sensitive(Scale::Log10), model(), 2).unwrap();
    let differences = Problem::new(differences, model(), 2).unwrap();

    let one = sensitivities.optimize(&Settings::default()).unwrap();
    let other = differences.optimize(&Settings::default()).unwrap();

    for (at, (value, expected)) in one.values.iter().zip(&other.values).enumerate() {
        assert!(
            (value / expected - 1.0).abs() < 1e-4,
            "the parameter {at} of the fit of the sensitivities is {value} and of the fit of \
             the differences {expected}"
        );
    }
    // the data of the fixture is the model itself, so both fits end at a cost of nothing
    assert!(
        one.cost < 1e-12 && other.cost < 1e-12,
        "the costs of the two fits are {} and {}",
        one.cost,
        other.cost
    );
    // and the fit of the sensitivities paid fewer simulations for the same optimum
    assert!(
        sensitivities.simulations() < differences.simulations(),
        "the fit of the sensitivities ran {} simulations and the fit of the differences {}",
        sensitivities.simulations(),
        differences.simulations()
    );
}

#[test]
fn the_jacobian_a_fit_takes_by_itself_is_the_one_of_the_differences() {
    // the difference quotient of `df/dp` inside a run of sensitivities costs about as much as
    // the integrations the run saves, so a fit which asks for nothing gets the differences
    let problem = Problem::new(pair(0.5, 2.0, Scale::Log10), model(), 2).unwrap();
    let x = problem.start();

    let before = problem.simulations();
    let jacobian = problem.jacobian(&x).unwrap();

    assert_eq!(problem.jacobian_of(), JacobianChoice::DifferenceQuotient);
    assert_eq!(problem.simulations() - before, 3);
    let apart = apart(&jacobian, &central(&problem, &x));
    assert!(
        apart < 1e-2,
        "the jacobian of the differences is {apart} off"
    );
}

#[test]
fn a_fit_which_asks_for_the_sensitivities_of_such_a_condition_is_refused() {
    let mut definition = with_a_change(Scale::Log10);
    definition.jacobian = JacobianChoice::Sensitivities;

    let refusal = Problem::new(definition, model(), 2)
        .unwrap_err()
        .to_string();

    assert!(refusal.contains("the condition 0"), "{refusal}");
    assert!(refusal.contains("sensitivities"), "{refusal}");
}

#[test]
fn a_problem_which_holds_a_parameter_takes_the_sensitivities_of_the_others() {
    let problem = Problem::new(sensitive(Scale::Log10), model(), 2).unwrap();

    let held = problem.fixing(&[(0, 0.3)]).unwrap();
    let x = held.start();
    let jacobian = held.jacobian(&x).unwrap();

    assert_eq!(held.jacobian_of(), JacobianChoice::Sensitivities);
    assert_eq!(jacobian.len(), held.n_residuals());
    let apart = apart(&jacobian, &central(&held, &x));
    assert!(apart < 1e-6, "the jacobian of the held fit is {apart} off");
}

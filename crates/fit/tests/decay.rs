//! A fit of the decay model `y' = -k y`: its residuals, its jacobian and its optimum.

use sbmlsim_fit::{Algorithm, JacobianChoice, Problem, Scale, Settings, Stop};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, definition, model, pair, problem};

#[test]
fn the_residuals_of_the_values_of_the_data_are_zero() {
    let problem = problem(K, Scale::Lin, 4);

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), problem.n_residuals());
    assert_eq!(residuals.len(), 11);
    assert!(
        residuals.iter().all(|value| *value == 0.0),
        "the residuals of the parameters of the data are not zero: {residuals:?}"
    );
}

#[test]
fn the_cost_is_half_the_sum_of_the_squares_of_the_residuals() {
    let problem = problem(0.5, Scale::Lin, 4);
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let expected = 0.5 * residuals.iter().map(|value| value * value).sum::<f64>();

    assert!(expected > 0.0, "the start of the fit is the optimum");
    assert_eq!(problem.cost(&x).unwrap(), expected);
}

#[test]
fn the_jacobian_of_one_batch_is_the_one_of_a_column_at_a_time() {
    // the jacobian of the finite differences, which the fit of a condition with events or
    // changes takes; the one of the sensitivities is `tests/sensitivity.rs`
    let mut definition = definition(0.5, Scale::Log10);
    definition.jacobian = JacobianChoice::DifferenceQuotient;
    let problem = Problem::new(definition, model(), 4).unwrap();
    let x = problem.start();

    let jacobian = problem.jacobian(&x).unwrap();

    // the same forward difference, one column after the other through `residuals`
    let residuals = problem.residuals(&x).unwrap();
    let step = problem.step(0, &x);
    let mut moved = x.clone();
    moved[0] += step;
    let other = problem.residuals(&moved).unwrap();
    assert_eq!(jacobian.len(), residuals.len());
    for (row, (at, ahead)) in residuals.iter().zip(other.iter()).enumerate() {
        assert_eq!(jacobian[row], (ahead - at) / step, "row {row}");
    }
}

#[test]
fn a_fit_which_starts_next_to_the_zero_of_its_search_space_finds_the_same_optimum() {
    // the optimum of a parameter which is 1 on a logarithmic scale is `x = 0`, which a
    // profile of another parameter re-fits from: the radius of the trust region of the
    // optimizer is `||x||`, so a start of 1e-16 would take a first step of 1e-16 and the fit
    // would end where it began
    let problem = Problem::new(pair(0.5, 2.0, Scale::Log10), model(), 2).unwrap();
    let held = problem.fixing(&[(0, 0.267375)]).unwrap();
    let settings = Settings::default();

    let at_zero = held.optimize_from(&settings, &[0.0]).unwrap();
    let next_to_it = held.optimize_from(&settings, &[1e-16]).unwrap();

    assert!(
        (next_to_it.cost / at_zero.cost - 1.0).abs() < 1e-9,
        "the fit from 1e-16 ends at {} and the fit from 0 at {}",
        next_to_it.cost,
        at_zero.cost
    );
    assert!(
        (next_to_it.values[0] / at_zero.values[0] - 1.0).abs() < 1e-6,
        "the fit from 1e-16 finds {} and the fit from 0 {}",
        next_to_it.values[0],
        at_zero.values[0]
    );
}

#[test]
fn the_threads_of_the_batch_do_not_reach_the_residuals() {
    let one = problem(0.5, Scale::Log10, 1);
    let many = problem(0.5, Scale::Log10, 8);

    assert_eq!(
        one.residuals(&one.start()).unwrap(),
        many.residuals(&many.start()).unwrap()
    );
    assert_eq!(
        one.jacobian(&one.start()).unwrap(),
        many.jacobian(&many.start()).unwrap()
    );
}

#[test]
fn the_fit_finds_the_parameter_of_the_data() {
    for scale in [Scale::Lin, Scale::Log10] {
        for algorithm in [Algorithm::TrustRegion, Algorithm::LevenbergMarquardt] {
            let problem = problem(3.0, scale, 4);
            let settings = Settings {
                algorithm,
                ..Settings::default()
            };

            let fit = problem.optimize(&settings).unwrap();

            let what = format!("{algorithm:?} on {scale:?}");
            assert!(
                (fit.values[0] - K).abs() < 1e-6,
                "{what} found k = {} instead of {K} after {} evaluations and {} jacobians, \
                 {}: {:?}",
                fit.values[0],
                fit.evaluations,
                fit.jacobians,
                fit.status,
                fit.residuals
            );
            assert!(fit.cost < 1e-12, "{what} ends at the cost {}", fit.cost);
            assert!(fit.evaluations > 0, "{what} evaluated nothing");
            assert!(fit.jacobians > 0, "{what} took no jacobian");
            assert_eq!(fit.residuals.len(), problem.n_residuals());
            assert_eq!(fit.x.len(), 1);
            assert_eq!(fit.values, problem.values(&fit.x));
        }
    }
}

#[test]
fn a_fit_which_is_stopped_answers_with_the_point_it_reached() {
    let problem = problem(3.0, Scale::Lin, 4);
    let stop = Stop::new();
    stop.stop();

    let fit = problem
        .optimize_with(&Settings::default(), &problem.start(), &stop)
        .unwrap();

    assert_eq!(
        fit.iterations, 0,
        "the stopped fit took a step: {}",
        fit.status
    );
    assert_eq!(fit.values, vec![3.0]);
    assert_eq!(fit.status, "UserRequested");
    assert!(fit.cost > 0.0, "the start of the fit is the optimum");
}

#[test]
fn a_fit_of_a_budget_of_time_stops_at_it() {
    let problem = problem(3.0, Scale::Lin, 4);
    let settings = Settings {
        max_seconds: Some(0.0),
        ..Settings::default()
    };

    let fit = problem.optimize(&settings).unwrap();

    assert_eq!(fit.status, "MaxTime");
    assert_eq!(fit.values, vec![3.0]);
}

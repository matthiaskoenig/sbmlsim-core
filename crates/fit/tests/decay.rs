//! A fit of the decay model `y' = -k y`: its residuals, its jacobian and its optimum.

use sbmlsim_fit::{Algorithm, Scale, Settings};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, problem};

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
    let problem = problem(0.5, Scale::Log10, 4);
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

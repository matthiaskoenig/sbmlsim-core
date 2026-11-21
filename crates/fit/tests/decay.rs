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

/// The definition of a fit whose data no parameter reproduces, so the cost of its optimum
/// is not zero and the relative change of the cost of a step becomes small next to it.
///
/// The data of [`definition`] is the model itself, which a fit reproduces to the last bits:
/// the cost of such a fit falls by orders of magnitude per step and its relative change is
/// about one until it converges, so no tolerance of the cost ever fires on it.
fn inconsistent(start: f64) -> sbmlsim_fit::Definition {
    let mut definition = definition(start, Scale::Log10);
    for (row, value) in definition.curves[0].values.iter_mut().enumerate() {
        *value += if row % 2 == 0 { 0.05 } else { -0.05 };
    }
    definition
}

#[test]
fn a_run_stops_at_the_tolerance_of_its_step() {
    // without it a least squares run has no test of its step and spends its last
    // iterations on the noise of the integrator, which it leaves as `NumericalNoProgress`;
    // `Settings::step_tolerance` says what that costs
    for algorithm in [Algorithm::TrustRegion, Algorithm::LevenbergMarquardt] {
        let problem = problem(3.0, Scale::Log10, 4);
        let ground = problem
            .optimize(&Settings {
                algorithm,
                ..Settings::default()
            })
            .unwrap();

        let fit = problem
            .optimize(&Settings {
                algorithm,
                step_tolerance: Some(1e-2),
                ..Settings::default()
            })
            .unwrap();

        assert_eq!(
            fit.status, "RelativeParamTolerance",
            "{algorithm:?} did not stop at the tolerance of its step"
        );
        assert!(
            fit.evaluations < ground.evaluations,
            "{algorithm:?}: {} evaluations against the {} of a run without the tolerance",
            fit.evaluations,
            ground.evaluations
        );
    }
}

#[test]
fn a_run_stops_at_the_tolerance_of_its_cost() {
    for algorithm in [Algorithm::TrustRegion, Algorithm::LevenbergMarquardt] {
        let problem = Problem::new(inconsistent(3.0), model(), 4).unwrap();
        let ground = problem
            .optimize(&Settings {
                algorithm,
                ..Settings::default()
            })
            .unwrap();

        let fit = problem
            .optimize(&Settings {
                algorithm,
                cost_tolerance: Some(1e-2),
                ..Settings::default()
            })
            .unwrap();

        assert_eq!(
            fit.status, "RelativeCostTolerance",
            "{algorithm:?} did not stop at the tolerance of its cost"
        );
        assert!(
            fit.evaluations < ground.evaluations,
            "{algorithm:?}: {} evaluations against the {} of a run without the tolerance",
            fit.evaluations,
            ground.evaluations
        );
        assert!(
            ground.cost > 0.0,
            "{algorithm:?}: the data of the fit is consistent after all"
        );
    }
}

#[test]
fn the_tolerances_of_a_run_which_converges_leave_its_optimum_where_it_is() {
    // the tolerances a caller gives are the ones of the noise of the integrator, where a
    // run has converged already: turning them on moves no optimum
    for algorithm in [Algorithm::TrustRegion, Algorithm::LevenbergMarquardt] {
        for definition in [definition(3.0, Scale::Log10), inconsistent(3.0)] {
            let problem = Problem::new(definition, model(), 4).unwrap();
            let ground = problem
                .optimize(&Settings {
                    algorithm,
                    ..Settings::default()
                })
                .unwrap();

            let fit = problem
                .optimize(&Settings {
                    algorithm,
                    step_tolerance: Some(1e-8),
                    cost_tolerance: Some(1e-8),
                    ..Settings::default()
                })
                .unwrap();

            assert!(
                (fit.cost - ground.cost).abs() <= 1e-9 * ground.cost.max(1e-12),
                "{algorithm:?}: the cost {} against the {} of a run without the \
                 tolerances",
                fit.cost,
                ground.cost
            );
            assert!(
                fit.evaluations <= ground.evaluations,
                "{algorithm:?}: {} evaluations against the {} of a run without them",
                fit.evaluations,
                ground.evaluations
            );
        }
    }
}

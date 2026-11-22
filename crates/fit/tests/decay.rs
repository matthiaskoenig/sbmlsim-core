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

/// A condition whose change at `t = 5` doubles `y`, so its output has two rows of that
/// time: the state before the change and the state after it.
fn doubled_at_five() -> sbmlsim_abi::SimulationRequest {
    let mut request = fixture::request();
    request.changes = vec![sbmlsim_abi::Change {
        time: 5.0,
        values: vec![],
        increments: vec![("y".to_string(), 1.0)],
    }];
    request
}

/// The `y` of the two rows of the time of the change, the state before it and the one after.
fn around_the_change(k: f64) -> (f64, f64) {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let prepared = model
        .prepare(&doubled_at_five(), &["k".to_string()])
        .unwrap();
    let table = instance.simulate_prepared(&prepared, &[k]).unwrap();
    let columns = table.columns.len();
    let time = table.columns.iter().position(|c| c == "time").unwrap();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    let at: Vec<usize> = (0..table.n_rows)
        .filter(|row| table.values[row * columns + time] == 5.0)
        .collect();
    assert_eq!(
        at.len(),
        2,
        "the change gives the time of its change two rows"
    );
    (
        table.values[at[0] * columns + y],
        table.values[at[1] * columns + y],
    )
}

/// The problem of one measurement at the time of the change, reading the state before it or
/// the one after it, whose value is `value`.
fn at_the_change(value: f64, after: Option<Vec<bool>>) -> Problem {
    let mut definition = definition(K, Scale::Lin);
    definition.conditions = vec![doubled_at_five().into()];
    definition.curves = vec![sbmlsim_fit::Curve {
        condition: 0,
        column: "y".to_string(),
        times: vec![5.0],
        values: vec![value],
        noise: None,
        weight: 1.0,
        after_change: after,
    }];
    Problem::new(definition, model(), 1).unwrap()
}

#[test]
fn a_measurement_at_a_change_reads_the_state_before_it() {
    let (before, after) = around_the_change(K);
    assert_ne!(before, after, "the change has to move the state");

    // the measurement is the state before the change, so its residual is zero
    let problem = at_the_change(before, None);
    assert_eq!(problem.residuals(&problem.start()).unwrap(), vec![0.0]);

    // and a measurement of the state after it is off by the change: the residual is the
    // simulated value, the state before the change, less the measured one
    let problem = at_the_change(after, None);
    assert_eq!(
        problem.residuals(&problem.start()).unwrap(),
        vec![before - after]
    );
}

#[test]
fn a_measurement_whose_flag_is_true_reads_the_state_after_a_change() {
    let (before, after) = around_the_change(K);

    // the measurement is the state after the change, so its residual is zero
    let problem = at_the_change(after, Some(vec![true]));
    assert_eq!(problem.residuals(&problem.start()).unwrap(), vec![0.0]);

    // and a measurement of the state before it is off by the change the other way round
    let problem = at_the_change(before, Some(vec![true]));
    assert_eq!(
        problem.residuals(&problem.start()).unwrap(),
        vec![after - before]
    );
}

#[test]
fn a_flag_of_the_state_after_a_change_which_is_false_reads_the_state_before_it() {
    let (before, _) = around_the_change(K);

    let problem = at_the_change(before, Some(vec![false]));

    assert_eq!(problem.residuals(&problem.start()).unwrap(), vec![0.0]);
}

#[test]
fn a_curve_of_more_flags_than_times_is_refused() {
    let mut definition = definition(K, Scale::Lin);
    definition.conditions = vec![doubled_at_five().into()];
    definition.curves = vec![sbmlsim_fit::Curve {
        condition: 0,
        column: "y".to_string(),
        times: vec![5.0],
        values: vec![1.0],
        noise: None,
        weight: 1.0,
        after_change: Some(vec![true, false]),
    }];

    let error = Problem::new(definition, model(), 1).unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("1 times and 2 flags"),
        "the refusal does not name the flags: {message}"
    );
}

#[test]
fn the_jacobian_of_a_measurement_after_a_change_is_the_one_of_the_row_it_reads() {
    let (before, after) = around_the_change(K);

    // the same measurement read on either side of the change: the residual of each is zero
    // at `K`, so the jacobians differ only by the row the measurement resolved to
    let mut jacobians = Vec::new();
    for flags in [None, Some(vec![true])] {
        let value = if flags.is_none() { before } else { after };
        let mut definition = definition(K, Scale::Lin);
        definition.jacobian = JacobianChoice::DifferenceQuotient;
        definition.conditions = vec![doubled_at_five().into()];
        definition.curves = vec![sbmlsim_fit::Curve {
            condition: 0,
            column: "y".to_string(),
            times: vec![5.0],
            values: vec![value],
            noise: None,
            weight: 1.0,
            after_change: flags,
        }];
        let problem = Problem::new(definition, model(), 1).unwrap();
        let x = problem.start();
        let jacobian = problem.jacobian(&x).unwrap();
        assert_eq!(jacobian.len(), 1);

        // the forward difference of the residual of that very row
        let at = problem.residuals(&x).unwrap()[0];
        let step = problem.step(0, &x);
        let mut moved = x.clone();
        moved[0] += step;
        let ahead = problem.residuals(&moved).unwrap()[0];
        assert_eq!(jacobian[0], (ahead - at) / step);
        jacobians.push(jacobian[0]);
    }

    // the change adds to `y`, so the derivative of the state after it in `k` is not the one
    // of the state before it, i.e. the flag reaches the jacobian and not only the residual
    assert_ne!(
        jacobians[0], jacobians[1],
        "the jacobian is the same on both sides of the change: {jacobians:?}"
    );
}

/// The definition of the decay fit against measurements of 0 with the given noise, whose
/// residual is the simulated value over that noise.
fn over_noise(noise: f64, weight: f64) -> sbmlsim_fit::Definition {
    let mut definition = definition(K, Scale::Lin);
    let points = definition.curves[0].values.len();
    definition.curves[0].values = vec![0.0; points];
    definition.curves[0].noise = Some(vec![noise; points]);
    definition.curves[0].weight = weight;
    definition
}

#[test]
fn the_residual_of_a_measurement_is_its_difference_divided_by_its_noise() {
    // a residual which is multiplied by the reciprocal of its standard deviation rounds
    // once more than one which is divided by it, so it is not the residual of the caller
    // bitwise: `0.9 / 2.5` is `0.36` where `0.9 * (1.0 / 2.5)` is `0.36000000000000004`.
    // The measurements are 0, so the difference of a point is the simulated value itself
    // and a fit of noise 1 reports it.
    let noise = 2.5;
    let simulated = Problem::new(over_noise(1.0, 1.0), model(), 1).unwrap();
    let problem = Problem::new(over_noise(noise, 1.0), model(), 1).unwrap();
    let values = simulated.residuals(&simulated.start()).unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), values.len());
    let reciprocal = 1.0 / noise;
    let mut rounded = 0;
    for (residual, value) in residuals.iter().zip(values.iter()) {
        assert_eq!(
            *residual,
            value / noise,
            "the residual {residual:?} of the value {value:?} is not {:?}, the value \
             divided by its standard deviation",
            value / noise
        );
        if value / noise != value * reciprocal {
            rounded += 1;
        }
    }
    assert!(
        rounded > 0,
        "no point of the fit tells a division by {noise} from a multiplication by its \
         reciprocal, so the test cannot see the difference"
    );
}

#[test]
fn the_weight_of_a_curve_multiplies_the_residuals_it_divided() {
    let problem = Problem::new(over_noise(2.5, 4.0), model(), 1).unwrap();
    let plain = Problem::new(over_noise(2.5, 1.0), model(), 1).unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    let expected = plain.residuals(&plain.start()).unwrap();
    assert_eq!(residuals.len(), expected.len());
    for (got, want) in residuals.iter().zip(expected.iter()) {
        assert_eq!(*got, want * 4.0);
    }
}

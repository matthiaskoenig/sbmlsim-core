//! The curves of a reduction: a value of the whole column of a condition over the time of its
//! simulation, its largest and its smallest value, its mean, its value at a time and a
//! parameter of the non-compartmental analysis of pkpdutils.
//!
//! The model is the decay `y' = -k y` of the fixture, `y = exp(-k t)`, whose reductions are
//! known in closed form: `max(y) = 1` at the start, `min(y) = exp(-10 k)` at the end, the
//! terminal half-life of the analysis is `ln(2) / k`.

use sbmlsim_abi::Output;
use sbmlsim_fit::{
    Curve, CurveRequest, Definition, FitRequest, JacobianChoice, Likelihood, NoiseKind,
    PkReduction, Problem, Reduction, Scale, Settings, problem_of,
};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, model, request};

/// The batch of pkpdutils of one sample of the decay, a bolus of one milligram at the start,
/// as `pkpdutils.nca.engine.batch_dict` of sbmlsim writes it; its time points are replaced.
const BATCH: &str = r#"{
    "contract": "batch/1", "sample_dims": ["_sim"], "sample_shape": [1], "n_time": 1,
    "times": [0.0], "values": [1.0], "sd": null, "se": null, "n": null,
    "nominal_time": null, "lloq": null, "max_doses": 1, "dose_amount": [1.0],
    "dose_time": [0.0], "dose_duration": ["NaN"], "time_unit": "hr", "unit": "mg/l",
    "dose_unit": "mg", "substance": "substance", "substances": null, "route": "iv_bolus",
    "routes": null, "tissue": null, "coords": []
}"#;

/// The default options of the analysis of pkpdutils, as `options_dict` writes them.
const OPTIONS: &str = r#"{
    "kind": "concentration", "auc_method": "linear_log",
    "terminal": {"method": "best_fit", "min_points": 3, "exclude_cmax": true,
        "n_points": null, "points": null, "min_adj_r2": null, "tie_tolerance": 0.0001,
        "relative_warning": 1e-08, "keep_candidates": false, "windows": null},
    "lloq": null, "lloq_relative": null, "blq": "nan", "c0_method": "log_back_extrapolation",
    "extrapolation_warning": 0.2,
    "acceptance": {"r2_adj_min": null, "extrapolation_max": null, "span_min": null,
        "n_points_min": null, "exclude": false},
    "partial_aucs": {}, "tau": null, "tau_tolerance": 0.1, "intervals": true, "units": {},
    "effect_threshold": null, "n_workers": null, "chunk_rows": 5000, "uncertainty": null,
    "n_boot": 1000, "seed": null, "ci_level": 0.95, "bootstrap_spread": "se",
    "bootstrap_distribution": "normal", "delta_step": 0.01
}"#;

/// A parameter of the analysis of the column `y`.
fn pk(parameter: &str) -> Reduction {
    Reduction::Pk(Box::new(PkReduction {
        factor: 1.0,
        time_shift: 0.0,
        changes: Vec::new(),
        parameter: parameter.to_string(),
        batch: serde_json::from_str(BATCH).unwrap(),
        options: serde_json::from_str(OPTIONS).unwrap(),
    }))
}

/// A curve of `y` of a reduction with one measurement of a value.
fn curve(reduction: Reduction, times: Vec<f64>, values: Vec<f64>) -> Curve {
    Curve {
        condition: 0,
        column: "y".to_string(),
        times,
        noise: Some(vec![0.1; values.len()]),
        values,
        weight: 1.0,
        after_change: None,
        likelihood: None,
        reduction: Some(reduction),
    }
}

/// The reductions of the decay at `K`, every one of them a curve.
fn curves() -> Vec<Curve> {
    let k = K;
    vec![
        curve(Reduction::Max { factor: 1.0 }, Vec::new(), vec![1.0]),
        curve(
            Reduction::Min { factor: 2.0 },
            Vec::new(),
            vec![2.0 * (-10.0 * k).exp()],
        ),
        curve(
            Reduction::Mean {
                factor: 1.0,
                time_shift: 0.0,
            },
            Vec::new(),
            vec![(1.0 - (-10.0 * k).exp()) / (10.0 * k)],
        ),
        curve(
            Reduction::At {
                factor: 1.0,
                time_shift: 0.0,
                tolerance: 0.0,
                changes: Vec::new(),
                before: None,
            },
            vec![2.5, 7.0],
            vec![(-2.5 * k).exp(), (-7.0 * k).exp()],
        ),
        curve(pk("thalf"), Vec::new(), vec![std::f64::consts::LN_2 / k]),
    ]
}

/// The fit of `k` against the reductions, on an output of the request.
fn definition(start: f64, output: Output, jacobian: JacobianChoice) -> Definition {
    let mut condition = request();
    condition.output = output;
    Definition {
        jacobian,
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![sbmlsim_fit::Parameter {
            id: "k".to_string(),
            start,
            lower: 0.01,
            upper: 10.0,
            scale: Scale::Log10,
        }],
        conditions: vec![condition.into()],
        curves: curves(),
    }
}

/// The value of `y` of the simulation at `k`, its times and its values.
fn table(k: f64, output: Output) -> (Vec<f64>, Vec<f64>) {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let mut condition = request();
    condition.output = output;
    let prepared = model.prepare(&condition, &["k".to_string()]).unwrap();
    let table = instance.simulate_prepared(&prepared, &[k]).unwrap();
    let columns = table.columns.len();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    (
        (0..table.n_rows)
            .map(|row| table.values[row * columns])
            .collect(),
        (0..table.n_rows)
            .map(|row| table.values[row * columns + y])
            .collect(),
    )
}

#[test]
fn the_residual_of_a_reduction_is_the_reduced_column_against_its_measurement() {
    let k = 0.45;
    let problem = Problem::new(
        definition(k, Output::Uniform { steps: 10 }, JacobianChoice::Auto),
        model(),
        2,
    )
    .unwrap();
    assert_eq!(problem.n_residuals(), 6);

    let residuals = problem.residuals(&problem.start()).unwrap();

    let (times, y) = table(k, Output::Uniform { steps: 10 });
    let max = y.iter().copied().fold(f64::NAN, f64::max);
    let min = y.iter().copied().fold(f64::NAN, f64::min);
    let area: f64 = (1..y.len())
        .map(|i| 0.5 * (y[i] + y[i - 1]) * (times[i] - times[i - 1]))
        .sum();
    let mean = area / (times[times.len() - 1] - times[0]);
    // 2.5 lies halfway between the rows of 2 and 3, 7.0 is a row
    let at = 0.5 * y[2] + 0.5 * y[3];
    let expected = [
        (max - 1.0) / 0.1,
        (2.0 * min - 2.0 * (-10.0 * K).exp()) / 0.1,
        (mean - (1.0 - (-10.0 * K).exp()) / (10.0 * K)) / 0.1,
        (at - (-2.5 * K).exp()) / 0.1,
        (y[7] - (-7.0 * K).exp()) / 0.1,
    ];
    for (row, value) in expected.iter().enumerate() {
        let apart = (residuals[row] - value).abs() / value.abs().max(1e-12);
        assert!(
            apart < 1e-12,
            "the residual {row} is {} and not {value}",
            residuals[row]
        );
    }
    // the half-life of an exponential decay of the analysis is ln(2) / k, up to the
    // integration, which is far below the noise of the curve
    let thalf = residuals[5] * 0.1 + std::f64::consts::LN_2 / K;
    assert!(
        (thalf - std::f64::consts::LN_2 / k).abs() < 1e-6,
        "the half-life is {thalf}"
    );
}

#[test]
fn a_condition_of_reductions_may_output_the_steps_of_the_integrator() {
    let problem = Problem::new(
        definition(K, Output::IntegratorSteps, JacobianChoice::Auto),
        model(),
        2,
    )
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    // the maximum is the start, exactly, and the half-life is the one of the decay
    assert_eq!(residuals[0], 0.0);
    assert!(residuals.iter().all(|value| value.is_finite()));
    assert!(
        residuals[5].abs() < 1e-4,
        "the half-life is off by {}",
        residuals[5]
    );
}

#[test]
fn a_condition_of_the_steps_of_the_integrator_with_a_curve_of_rows_is_refused() {
    let mut definition = definition(K, Output::IntegratorSteps, JacobianChoice::Auto);
    let mut rows = curves().remove(0);
    rows.reduction = None;
    rows.times = vec![0.0];
    definition.curves.push(rows);

    let error = Problem::new(definition, model(), 2)
        .unwrap_err()
        .to_string();

    assert!(error.contains("steps of the integrator"), "{error}");
}

#[test]
fn a_fit_of_reductions_finds_the_parameter_of_its_data() {
    for jacobian in [
        JacobianChoice::DifferenceQuotient,
        JacobianChoice::Sensitivities,
    ] {
        let problem = Problem::new(
            definition(0.5, Output::Uniform { steps: 40 }, jacobian),
            model(),
            2,
        )
        .unwrap();
        assert_eq!(problem.jacobian_of(), jacobian);

        let fit = problem.optimize(&Settings::default()).unwrap();

        assert!(
            (fit.values[0] - K).abs() < 1e-3,
            "{jacobian:?}: the fit found k = {}",
            fit.values[0]
        );
    }
}

/// The jacobian of the residuals by a central difference, accurate to about 1e-9 of itself.
fn central(problem: &Problem, x: &[f64]) -> Vec<f64> {
    let n = problem.n_residuals();
    let step = 1e-6 * x[0].abs().max(1.0);
    let below = problem.residuals(&[x[0] - step]).unwrap();
    let above = problem.residuals(&[x[0] + step]).unwrap();
    (0..n)
        .map(|row| (above[row] - below[row]) / (2.0 * step))
        .collect()
}

#[test]
fn the_jacobian_of_the_sensitivities_of_a_reduction_is_its_derivative() {
    // the reference is the central difference of the residuals on a fine grid, whose rows do
    // not move with the parameter: on the steps of the integrator they do, which a difference
    // of the residuals sees and a sensitivity does not, so the difference of the residuals of
    // those steps is off by the error of the integration (1.3 % at t = 7) and the sensitivity
    // is the better derivative of the two
    let fine = Problem::new(
        definition(
            0.5,
            Output::Uniform { steps: 2000 },
            JacobianChoice::DifferenceQuotient,
        ),
        model(),
        2,
    )
    .unwrap();
    let x = fine.start();
    let expected = central(&fine, &x);
    for (output, tolerance) in [
        (Output::Uniform { steps: 2000 }, 1e-5),
        (Output::IntegratorSteps, 2e-3),
    ] {
        let problem = Problem::new(
            definition(0.5, output.clone(), JacobianChoice::Sensitivities),
            model(),
            2,
        )
        .unwrap();
        assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);

        let jacobian = problem.jacobian(&x).unwrap();

        // the maximum is the start, whose value is no function of `k`
        assert_eq!(jacobian[0], 0.0);
        for row in 1..problem.n_residuals() {
            let apart = (jacobian[row] - expected[row]).abs() / expected[row].abs().max(1e-9);
            assert!(
                apart < tolerance,
                "{output:?}: the row {row} of the jacobian is {} and its difference {}",
                jacobian[row],
                expected[row]
            );
        }
    }
}

#[test]
fn the_likelihood_of_a_reduction_is_the_one_of_its_reduced_value() {
    let mut definition = definition(0.5, Output::Uniform { steps: 10 }, JacobianChoice::Auto);
    for curve in &mut definition.curves {
        curve.likelihood = Some(Likelihood {
            distribution: NoiseKind::Normal,
            noise_column: None,
        });
    }
    let least_squares = Problem::new(
        self::definition(0.5, Output::Uniform { steps: 10 }, JacobianChoice::Auto),
        model(),
        2,
    )
    .unwrap();
    let problem = Problem::new(definition, model(), 2).unwrap();
    let x = problem.start();

    let residuals = problem.residuals(&x).unwrap();
    let data = least_squares.residuals(&x).unwrap();

    // every curve has the rows of its data and then the rows of the scales of its noise
    assert_eq!(residuals.len(), 2 * data.len());
    let mut at = 0;
    let mut of_the_data = Vec::new();
    for size in [1, 1, 1, 2, 1] {
        of_the_data.extend_from_slice(&residuals[at..at + size]);
        at += 2 * size;
    }
    assert_eq!(of_the_data, data);
}

#[test]
fn a_reduction_which_does_not_fit_its_curve_is_refused() {
    let refusal = |change: &dyn Fn(&mut Curve)| {
        let mut definition = definition(K, Output::Uniform { steps: 10 }, JacobianChoice::Auto);
        change(&mut definition.curves[0]);
        Problem::new(definition, model(), 2)
            .unwrap_err()
            .to_string()
    };
    let refused = refusal(&|curve| curve.times = vec![1.0]);
    assert!(refused.contains("has times"), "{refused}");
    let refused = refusal(&|curve| curve.after_change = Some(vec![true]));
    assert!(refused.contains("state after a change"), "{refused}");
    let refused = refusal(&|curve| {
        curve.likelihood = Some(Likelihood {
            distribution: NoiseKind::Normal,
            noise_column: Some("y".to_string()),
        });
        curve.noise = None;
    });
    assert!(refused.contains("noise_column"), "{refused}");
    let refused = refusal(&|curve| {
        curve.reduction = Some(Reduction::At {
            factor: 1.0,
            time_shift: 0.0,
            tolerance: 0.0,
            changes: Vec::new(),
            before: None,
        });
        curve.times = vec![11.0];
    });
    assert!(refused.contains("outside of the simulation"), "{refused}");
    let refused = refusal(&|curve| curve.reduction = Some(pk("no_such_parameter")));
    assert!(refused.contains("does not derive"), "{refused}");
    let refused = refusal(&|curve| curve.reduction = Some(Reduction::Max { factor: f64::NAN }));
    assert!(refused.contains("not finite"), "{refused}");
}

#[test]
fn a_time_shift_moves_the_time_of_an_interpolation() {
    let mut definition = definition(K, Output::Uniform { steps: 10 }, JacobianChoice::Auto);
    definition.curves = vec![curve(
        Reduction::At {
            factor: 1.0,
            time_shift: 100.0,
            tolerance: 0.0,
            changes: Vec::new(),
            before: None,
        },
        vec![102.0],
        vec![(-2.0 * K).exp()],
    )];

    let problem = Problem::new(definition, model(), 1).unwrap();
    let residuals = problem.residuals(&problem.start()).unwrap();

    assert!(residuals[0].abs() < 1e-8, "{residuals:?}");
}

#[test]
fn a_curve_of_the_contract_carries_its_reduction() {
    let curve: CurveRequest = serde_json::from_str(
        r#"{"condition": "c", "column": "y", "values": [1.0], "reduction": {"kind": "max"}}"#,
    )
    .unwrap();
    assert!(curve.times.is_empty());
    assert!(matches!(
        curve.reduction,
        Some(Reduction::Max { factor }) if factor == 1.0
    ));
    let at: CurveRequest = serde_json::from_str(
        r#"{"condition": "c", "column": "y", "times": [1.0], "values": [1.0],
            "reduction": {"kind": "at", "tolerance": 1e-9, "changes": [5.0],
                          "before": [true], "time_shift": -2.0}}"#,
    )
    .unwrap();
    assert!(matches!(at.reduction, Some(Reduction::At { tolerance, .. }) if tolerance == 1e-9));
    let unknown = serde_json::from_str::<CurveRequest>(
        r#"{"condition": "c", "column": "y", "values": [1.0],
            "reduction": {"kind": "max", "window": 2.0}}"#,
    );
    assert!(
        unknown.is_err(),
        "a field a reduction does not know is read"
    );
    let kind = serde_json::from_str::<CurveRequest>(
        r#"{"condition": "c", "column": "y", "values": [1.0], "reduction": {"kind": "median"}}"#,
    );
    assert!(kind.is_err(), "a reduction of no kind is read");
    // a request without the field is a request of every release before
    let plain: CurveRequest = serde_json::from_str(
        r#"{"condition": "c", "column": "y", "times": [1.0], "values": [1.0]}"#,
    )
    .unwrap();
    assert!(plain.reduction.is_none());
    let text = serde_json::to_string(&plain).unwrap();
    assert!(!text.contains("reduction"), "{text}");
}

#[test]
fn a_request_of_a_parameter_of_the_analysis_is_fitted_in_the_engine() {
    let request: FitRequest = serde_json::from_value(serde_json::json!({
        "contract": "fit/1",
        "parameters": [{"id": "k", "start": 0.5, "lower": 0.01, "upper": 10.0, "scale": "log10"}],
        "conditions": [{"id": "c", "request": request()}],
        "curves": [{
            "condition": "c", "column": "y", "values": [std::f64::consts::LN_2 / K],
            "noise": [0.01],
            "reduction": {
                "kind": "pk", "parameter": "thalf",
                "batch": serde_json::from_str::<serde_json::Value>(BATCH).unwrap(),
                "options": serde_json::from_str::<serde_json::Value>(OPTIONS).unwrap()
            }
        }]
    }))
    .unwrap();

    let problem = problem_of(&request, model(), Some(1)).unwrap();
    let fit = problem.optimize(&Settings::default()).unwrap();

    assert!(
        (fit.values[0] - K).abs() < 1e-4,
        "the fit found k = {}",
        fit.values[0]
    );
}

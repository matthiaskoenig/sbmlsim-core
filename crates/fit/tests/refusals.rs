//! What a fit refuses, and what it does with a simulation which fails.
//!
//! A definition which cannot be fitted is refused when the problem is built and not at the
//! first evaluation, so that a fit of many hours does not end on a curve which reads a column
//! its condition never selected. A failed integration is no such error: it is a point of the
//! search space the optimizer may not take, so its residuals are
//! [`sbmlsim_fit::FAILED_RESIDUAL`].

use sbmlsim_abi::{Output, SimulationRequest};
use sbmlsim_fit::{Curve, Definition, FAILED_RESIDUAL, FitError, Parameter, Problem, Scale};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{model, request};

/// The parameter `k` of the decay model, between a tenth and ten.
fn parameter() -> Parameter {
    Parameter {
        id: "k".to_string(),
        start: 0.5,
        lower: 0.1,
        upper: 10.0,
        scale: Scale::Lin,
    }
}

/// A curve of `y` of the first condition, at the output times of its request.
fn curve() -> Curve {
    Curve {
        condition: 0,
        column: "y".to_string(),
        times: vec![0.0, 1.0, 2.0],
        values: vec![1.0, 0.6, 0.4],
        noise: None,
        weight: 1.0,
    }
}

fn definition() -> Definition {
    Definition {
        jacobian: Default::default(),
        parameters: vec![parameter()],
        conditions: vec![request()],
        curves: vec![curve()],
    }
}

/// The problem of a definition, or what it was refused with.
fn refusal(definition: Definition) -> String {
    match Problem::new(definition, model(), 2) {
        Ok(_) => panic!("the fit was not refused"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn a_fit_of_nothing_is_refused() {
    for (what, definition) in [
        (
            "parameter",
            Definition {
                parameters: Vec::new(),
                ..definition()
            },
        ),
        (
            "condition",
            Definition {
                conditions: Vec::new(),
                ..definition()
            },
        ),
        (
            "curve",
            Definition {
                curves: Vec::new(),
                ..definition()
            },
        ),
    ] {
        assert_eq!(refusal(definition), format!("the fit has no {what}"));
    }
}

#[test]
fn a_parameter_which_cannot_be_searched_is_refused() {
    for (fault, parameter) in [
        (
            "has a bound which is not finite",
            Parameter {
                upper: f64::INFINITY,
                ..parameter()
            },
        ),
        (
            "has the bounds [10, 0.1], which are not increasing",
            Parameter {
                lower: 10.0,
                upper: 0.1,
                ..parameter()
            },
        ),
        (
            "starts at 20, outside its bounds [0.1, 10]",
            Parameter {
                start: 20.0,
                ..parameter()
            },
        ),
        (
            "is searched on a logarithmic scale and has the lower bound 0, which is not positive",
            Parameter {
                lower: 0.0,
                scale: Scale::Log10,
                ..parameter()
            },
        ),
    ] {
        assert_eq!(
            refusal(Definition {
                parameters: vec![parameter],
                ..definition()
            }),
            format!("the parameter 'k' {fault}")
        );
    }
}

#[test]
fn a_parameter_the_model_has_not_is_refused() {
    let refusal = refusal(Definition {
        parameters: vec![Parameter {
            id: "clearance".to_string(),
            ..parameter()
        }],
        ..definition()
    });

    assert!(
        refusal.starts_with("the condition 0 cannot be prepared:"),
        "{refusal}"
    );
    assert!(refusal.contains("clearance"), "{refusal}");
}

#[test]
fn a_condition_which_outputs_the_steps_of_the_integrator_is_refused() {
    let refusal = refusal(Definition {
        conditions: vec![SimulationRequest {
            output: Output::IntegratorSteps,
            ..request()
        }],
        ..definition()
    });

    assert!(
        refusal.starts_with("the condition 0 outputs the steps of the integrator"),
        "{refusal}"
    );
}

#[test]
fn a_curve_which_does_not_fit_its_condition_is_refused() {
    let cases = [
        (
            "reads the condition 1, which the fit has not",
            Curve {
                condition: 1,
                ..curve()
            },
        ),
        (
            "reads the column 'z', which its condition does not select",
            Curve {
                column: "z".to_string(),
                ..curve()
            },
        ),
        (
            "measures at t = 0.5, which is no output time of its condition",
            Curve {
                times: vec![0.0, 0.5],
                values: vec![1.0, 0.8],
                ..curve()
            },
        ),
        (
            "has 3 times and 2 values",
            Curve {
                values: vec![1.0, 0.6],
                ..curve()
            },
        ),
        (
            "has no measurement",
            Curve {
                times: Vec::new(),
                values: Vec::new(),
                ..curve()
            },
        ),
        (
            "has 3 values and 1 standard deviations",
            Curve {
                noise: Some(vec![0.1]),
                ..curve()
            },
        ),
        (
            "has the standard deviation 0, which is not positive and finite",
            Curve {
                noise: Some(vec![0.1, 0.0, 0.1]),
                ..curve()
            },
        ),
        (
            "has the weight 0, which is not positive and finite",
            Curve {
                weight: 0.0,
                ..curve()
            },
        ),
    ];
    for (fault, curve) in cases {
        let refusal = refusal(Definition {
            curves: vec![curve],
            ..definition()
        });
        assert!(
            refusal.starts_with(&format!("the curve 0 {fault}")),
            "{refusal} does not say {fault}"
        );
    }
}

#[test]
fn the_residuals_of_a_curve_are_weighted_by_its_noise() {
    let plain = Problem::new(definition(), model(), 2).unwrap();
    let weighted = Problem::new(
        Definition {
            curves: vec![Curve {
                noise: Some(vec![0.5, 0.25, 2.0]),
                weight: 3.0,
                ..curve()
            }],
            ..definition()
        },
        model(),
        2,
    )
    .unwrap();

    let residuals = plain.residuals(&plain.start()).unwrap();
    let scaled = weighted.residuals(&weighted.start()).unwrap();

    for (at, scale) in [3.0 / 0.5, 3.0 / 0.25, 3.0 / 2.0].iter().enumerate() {
        assert_eq!(scaled[at], residuals[at] * scale, "the point {at}");
    }
}

#[test]
fn a_condition_whose_integration_fails_has_the_residuals_of_a_failure() {
    // 60 steps of the integrator reach the end of the condition at the start of the fit
    // (k = 0.5) and not at a larger k, where the decay is faster and the tolerance of the
    // request is as tight
    let mut request = request();
    request.settings.maximum_num_steps = 60;
    let problem = Problem::new(
        Definition {
            conditions: vec![request],
            ..definition()
        },
        model(),
        2,
    )
    .unwrap();

    let residuals = problem.residuals(&[2.0]).unwrap();

    assert_eq!(residuals, vec![FAILED_RESIDUAL; 3]);
    assert_eq!(
        problem.cost(&[2.0]).unwrap(),
        1.5 * FAILED_RESIDUAL * FAILED_RESIDUAL
    );
    // the start of the fit is no failure, so the failure is the point and not the condition
    assert!(
        problem
            .residuals(&problem.start())
            .unwrap()
            .iter()
            .all(|residual| *residual != FAILED_RESIDUAL)
    );
}

#[test]
fn a_condition_which_does_not_integrate_at_the_start_is_refused() {
    // one step of the integrator reaches no output time at all
    let mut request = request();
    request.settings.maximum_num_steps = 1;

    let refusal = refusal(Definition {
        conditions: vec![request],
        ..definition()
    });

    assert!(
        refusal.starts_with("the condition 0 cannot be simulated at the start of the fit:"),
        "{refusal}"
    );
}

#[test]
fn the_residuals_of_a_curve_are_the_ones_of_its_own_condition() {
    // two conditions of the same model, the second one twice as long, and a curve of each:
    // the curve of the second condition measures what the model does there
    let long = SimulationRequest {
        end: 20.0,
        ..request()
    };
    let problem = Problem::new(
        Definition {
            conditions: vec![request(), long],
            curves: vec![
                curve(),
                Curve {
                    condition: 1,
                    times: vec![0.0, 2.0],
                    values: vec![1.0, (-0.5f64 * 2.0).exp()],
                    ..curve()
                },
            ],
            ..definition()
        },
        model(),
        2,
    )
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), 5);
    assert_eq!(problem.n_residuals(), 5);
    // the curve of the second condition is the model itself, to the tolerance of its integrator
    assert!(residuals[3].abs() < 1e-9, "{:?}", residuals);
    assert!(residuals[4].abs() < 1e-9, "{:?}", residuals);
    assert!(residuals[1].abs() > 1e-3, "{:?}", residuals);
}

#[test]
fn a_fit_of_more_threads_than_points_is_no_error() {
    let problem = Problem::new(definition(), model(), 64).unwrap();

    assert_eq!(problem.threads(), 64);
    assert_eq!(problem.residuals(&problem.start()).unwrap().len(), 3);
}

#[test]
fn a_fit_of_no_thread_is_refused() {
    let err = Problem::new(definition(), model(), 0).unwrap_err();

    assert!(
        matches!(err, FitError::Pool(_)),
        "{err} is no refusal of the pool"
    );
}

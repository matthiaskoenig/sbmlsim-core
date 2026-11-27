//! The conditions of the points of a scan: a curve over a dimension and a pinned parameter.
//!
//! A host fits a scan as a condition per point. Two things of a scan are no curve of one
//! condition: a measurement between two points of a dimension, which the host compares with
//! the line between the two conditions at the x of the measurement ([`Dimension`]), and a
//! dimension which sets the target of a parameter of the fit, whose value of the point wins
//! over the parameter ([`Pinned`]).
//!
//! The scan here is the one over the initial value of `y` of the decay model `y' = -k y`,
//! every point a condition whose request sets `y` before the simulation.

use sbmlsim_abi::SimulationRequest;
use sbmlsim_fit::{
    Condition, Curve, Definition, Dimension, FIELDS, FIT_CONTRACT, FitRequest, JacobianChoice,
    Parameter, Pinned, Problem, Reduction, Scale, Settings, problem_of,
};
use serde_json::{Value, json};

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, model, request};

/// The initial values of `y` of the points of the scan, the values of its dimension.
const POINTS: [f64; 3] = [1.0, 2.0, 4.0];

/// The x of the measurements on the dimension: between two points, at a point within the
/// scan and at its last point.
const X: [f64; 5] = [1.5, 2.0, 3.0, 1.25, 4.0];

/// The time of the measurements, the row 5 of the uniform output of the request.
const TIME: f64 = 5.0;

/// The request of the point of an initial value of `y`.
fn point(y: f64) -> SimulationRequest {
    let mut request = request();
    request.preinit = vec![("y".to_string(), y)];
    request
}

/// The value of `y` at [`TIME`] of a point of the scan at a value of `k`.
fn simulated(y: f64, k: f64) -> f64 {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let prepared = model.prepare(&point(y), &["k".to_string()]).unwrap();
    let table = instance.simulate_prepared(&prepared, &[k]).unwrap();
    let columns = table.columns.len();
    let at = table.columns.iter().position(|c| c == "y").unwrap();
    let row = (0..table.n_rows)
        .find(|row| table.values[row * columns] == TIME)
        .unwrap();
    table.values[row * columns + at]
}

/// `numpy.interp` of the values of the points at `x`, as python writes it.
fn interp(x: f64, values: &[f64]) -> f64 {
    let j = POINTS.iter().rposition(|value| *value <= x).unwrap();
    if j == POINTS.len() - 1 || POINTS[j] == x {
        return values[j];
    }
    let slope = (values[j + 1] - values[j]) / (POINTS[j + 1] - POINTS[j]);
    slope * (x - POINTS[j]) + values[j]
}

/// What the interpolation of the points gives at every x of [`X`] at a value of `k`.
fn interpolated(k: f64) -> Vec<f64> {
    let values: Vec<f64> = POINTS.iter().map(|y| simulated(*y, k)).collect();
    X.iter().map(|x| interp(*x, &values)).collect()
}

/// The curve of `y` at [`TIME`] over the dimension of the initial values, whose data is the
/// interpolation at [`K`].
fn curve() -> Curve {
    Curve {
        condition: 0,
        column: "y".to_string(),
        times: vec![TIME; X.len()],
        values: interpolated(K),
        noise: None,
        weight: 1.0,
        after_change: None,
        likelihood: None,
        reduction: None,
        dimension: Some(Dimension {
            conditions: (0..POINTS.len()).collect(),
            values: POINTS.to_vec(),
            x: X.to_vec(),
            reductions: Vec::new(),
        }),
    }
}

/// The fit of `k` over the points of the scan.
fn definition(start: f64, jacobian: JacobianChoice) -> Definition {
    Definition {
        jacobian,
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![Parameter {
            id: "k".to_string(),
            start,
            lower: 0.01,
            upper: 10.0,
            scale: Scale::Log10,
        }],
        conditions: POINTS.iter().map(|y| point(*y).into()).collect(),
        curves: vec![curve()],
    }
}

fn problem(definition: Definition) -> Problem {
    Problem::new(definition, model(), 2).unwrap()
}

/// The message of a definition the fit refuses.
fn refusal(definition: Definition) -> String {
    match Problem::new(definition, model(), 1) {
        Ok(_) => panic!("the definition was accepted"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn a_curve_over_a_dimension_is_the_interpolation_of_its_points() {
    let problem = problem(definition(K, JacobianChoice::Auto));

    for x in [K.log10(), -0.3, -1.0] {
        let residuals = problem.residuals(&[x]).unwrap();
        // the value of `k` of the point, which is no power of ten of a logarithm exactly
        let k = problem.parameter_values(&[x])[0];

        // bitwise the interpolation of the values of the conditions, as python computes it
        let want: Vec<f64> = interpolated(k)
            .iter()
            .zip(interpolated(K).iter())
            .map(|(simulated, value)| simulated - value)
            .collect();
        assert_eq!(residuals, want, "at k = {k}");
        assert!(x != K.log10() || residuals.iter().all(|r| r.abs() < 1e-15));
    }
}

#[test]
fn a_measurement_at_a_point_reads_the_condition_of_the_point_alone() {
    let problem = problem(definition(K, JacobianChoice::Auto));
    let x = [-0.3];
    let k = problem.parameter_values(&x)[0];

    let residuals = problem.residuals(&x).unwrap();

    // x = 2 and x = 4 are the second and the last point
    let values = interpolated(K);
    assert_eq!(residuals[1], simulated(2.0, k) - values[1]);
    assert_eq!(residuals[4], simulated(4.0, k) - values[4]);
}

#[test]
fn the_jacobian_of_a_curve_over_a_dimension_interpolates_the_sensitivities() {
    let problem = problem(definition(0.5, JacobianChoice::Sensitivities));
    assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
    let x = f64::log10(0.5);
    let h = 1e-5;

    let exact = problem.jacobian(&[x]).unwrap();

    // the central difference of the residuals, which the integrator resolves to 1e-10
    let ahead = problem.residuals(&[x + h]).unwrap();
    let behind = problem.residuals(&[x - h]).unwrap();
    assert_eq!(exact.len(), X.len());
    for (at, exact) in exact.iter().enumerate() {
        let difference = (ahead[at] - behind[at]) / (2.0 * h);
        assert!(*exact < 0.0, "{exact}");
        assert!(
            (exact - difference).abs() < 1e-6 * exact.abs(),
            "{exact} against the difference {difference}"
        );
    }
}

#[test]
fn a_fit_over_a_dimension_finds_the_parameter_of_the_data() {
    let problem = problem(definition(1.5, JacobianChoice::Sensitivities));

    let fit = problem.optimize(&Settings::default()).unwrap();

    assert!(
        (fit.values[0] - K).abs() < 1e-6,
        "the fit ended at {:?}, not at {K}",
        fit.values
    );
}

#[test]
fn a_curve_over_a_dimension_interpolates_its_reduction() {
    let mut definition = definition(K, JacobianChoice::Sensitivities);
    let curve = &mut definition.curves[0];
    curve.reduction = Some(Reduction::Max { factor: 1.0 });
    curve.times = Vec::new();
    // the largest value of `y` is its initial value, the value of the dimension itself
    curve.values = X.iter().map(|x| interp(*x, &POINTS)).collect();
    let problem = problem(definition);

    let residuals = problem.residuals(&[f64::log10(0.5)]).unwrap();
    let jacobian = problem.jacobian(&[f64::log10(0.5)]).unwrap();

    assert!(residuals.iter().all(|r| *r == 0.0), "{residuals:?}");
    assert!(jacobian.iter().all(|d| *d == 0.0), "{jacobian:?}");
}

#[test]
fn every_point_of_a_dimension_reads_its_own_reduction() {
    let mut definition = definition(K, JacobianChoice::Sensitivities);
    let curve = &mut definition.curves[0];
    curve.times = Vec::new();
    // the largest value of `y` times a factor of the point, its initial value squared
    let squares: Vec<f64> = POINTS.iter().map(|y| y * y).collect();
    curve.values = X.iter().map(|x| interp(*x, &squares)).collect();
    let dimension = curve.dimension.as_mut().unwrap();
    dimension.reductions = POINTS
        .iter()
        .map(|y| Reduction::Max { factor: *y })
        .collect();
    let problem = problem(definition);

    let residuals = problem.residuals(&[f64::log10(0.5)]).unwrap();

    assert!(residuals.iter().all(|r| *r == 0.0), "{residuals:?}");
}

#[test]
fn a_reduction_of_a_curve_and_of_its_points_is_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    let curve = &mut definition.curves[0];
    curve.times = Vec::new();
    curve.reduction = Some(Reduction::Max { factor: 1.0 });
    curve.dimension.as_mut().unwrap().reductions = vec![Reduction::Max { factor: 1.0 }; 3];

    assert!(refusal(definition).contains("a reduction and the reductions of the points"));
}

#[test]
fn a_curve_over_a_dimension_of_a_likelihood_has_the_rows_of_its_scales() {
    let mut least_squares = definition(K, JacobianChoice::Sensitivities);
    least_squares.curves[0].noise = Some(vec![0.1; X.len()]);
    let mut likelihood = least_squares.clone();
    likelihood.curves[0].likelihood = Some(sbmlsim_fit::Likelihood {
        distribution: sbmlsim_fit::NoiseKind::Normal,
        noise_column: None,
    });
    let x = [f64::log10(0.5)];

    let rows = problem(least_squares).residuals(&x).unwrap();
    let both = problem(likelihood).residuals(&x).unwrap();

    // the rows of the data are the ones of the least squares, the rows of the scales follow
    assert_eq!(both.len(), 2 * X.len());
    assert_eq!(&both[..X.len()], rows.as_slice());
    let scale = (2.0 * (0.1_f64.ln() + sbmlsim_fit::LN_2PI_HALF) + 100.0).sqrt();
    assert!(both[X.len()..].iter().all(|row| *row == scale), "{both:?}");
}

#[test]
fn a_curve_over_a_dimension_of_one_point_is_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    let dimension = definition.curves[0].dimension.as_mut().unwrap();
    dimension.conditions.truncate(1);
    dimension.values.truncate(1);

    assert!(refusal(definition).contains("over 1 points of a dimension"));
}

#[test]
fn values_of_a_dimension_which_do_not_increase_are_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    let dimension = definition.curves[0].dimension.as_mut().unwrap();
    dimension.values = vec![1.0, 4.0, 2.0];

    let refusal = refusal(definition);
    assert!(
        refusal.contains("which do not increase strictly"),
        "{refusal}"
    );
}

#[test]
fn a_measurement_outside_of_the_dimension_is_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    definition.curves[0].dimension.as_mut().unwrap().x[0] = 4.5;

    assert!(refusal(definition).contains("outside of its dimension [1, 4]"));
}

#[test]
fn a_condition_twice_in_a_dimension_is_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    definition.curves[0].dimension.as_mut().unwrap().conditions = vec![0, 1, 1];

    assert!(refusal(definition).contains("the condition 1 twice"));
}

#[test]
fn a_noise_column_over_a_dimension_is_refused() {
    let mut definition = definition(K, JacobianChoice::Auto);
    definition.curves[0].likelihood = Some(sbmlsim_fit::Likelihood {
        distribution: sbmlsim_fit::NoiseKind::Normal,
        noise_column: Some("y".to_string()),
    });

    assert!(refusal(definition).contains("'noise_column' and a dimension"));
}

/// The fit of `k` of the decay at `K` whose second condition pins `k` at `pin`.
fn pinned(pin: f64, jacobian: JacobianChoice) -> Definition {
    let (times, values) = fixture::data(K);
    let condition = |pinned: Vec<Pinned>| Condition {
        request: request(),
        model: None,
        derived: Vec::new(),
        pinned,
    };
    let curve = |condition: usize| Curve {
        condition,
        column: "y".to_string(),
        times: times.clone(),
        values: values.clone(),
        noise: None,
        weight: 1.0,
        after_change: None,
        likelihood: None,
        reduction: None,
        dimension: None,
    };
    Definition {
        jacobian,
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![Parameter {
            id: "k".to_string(),
            start: 0.5,
            lower: 0.01,
            upper: 10.0,
            scale: Scale::Lin,
        }],
        conditions: vec![
            condition(Vec::new()),
            condition(vec![Pinned {
                parameter: 0,
                value: pin,
            }]),
        ],
        curves: vec![curve(0), curve(1)],
    }
}

#[test]
fn a_pinned_parameter_holds_its_target_at_the_value_of_the_condition() {
    let problem = problem(pinned(K, JacobianChoice::Auto));
    let (_, values) = fixture::data(K);
    let n = values.len();

    let residuals = problem.residuals(&[0.5]).unwrap();

    // the first condition follows `k`, the second holds it at `K`, the value of the data
    let (_, at) = fixture::data(0.5);
    let want: Vec<f64> = at.iter().zip(values.iter()).map(|(s, v)| s - v).collect();
    assert_eq!(&residuals[..n], want.as_slice());
    assert!(residuals[n..].iter().all(|r| *r == 0.0), "{residuals:?}");
}

#[test]
fn the_column_of_a_pinned_parameter_is_zero_in_its_condition() {
    for jacobian in [
        JacobianChoice::Sensitivities,
        JacobianChoice::DifferenceQuotient,
    ] {
        let problem = problem(pinned(K, jacobian));
        let n = fixture::data(K).1.len();

        let rows = problem.jacobian(&[0.5]).unwrap();

        assert!(
            rows[1..n].iter().all(|d| *d < 0.0),
            "{jacobian:?}: {rows:?}"
        );
        assert!(
            rows[n..].iter().all(|d| *d == 0.0),
            "{jacobian:?}: {rows:?}"
        );
    }
}

#[test]
fn a_pin_of_a_parameter_the_fit_has_not_is_refused() {
    let mut definition = pinned(K, JacobianChoice::Auto);
    definition.conditions[1].pinned[0].parameter = 3;

    assert!(refusal(definition).contains("pins the parameter 3 of a fit of 1 parameters"));
}

#[test]
fn a_parameter_pinned_twice_is_refused() {
    let mut definition = pinned(K, JacobianChoice::Auto);
    let pin = definition.conditions[1].pinned[0];
    definition.conditions[1].pinned.push(pin);

    assert!(refusal(definition).contains("pins the parameter 0 twice"));
}

#[test]
fn a_pin_which_is_not_finite_is_refused() {
    let mut definition = pinned(f64::NAN, JacobianChoice::Auto);
    definition.conditions[1].pinned[0].value = f64::INFINITY;

    assert!(refusal(definition).contains("which is not finite"));
}

/// The request of the fit over the scan as a host writes it.
fn json() -> Value {
    json!({
        "contract": FIT_CONTRACT,
        "parameters": [{"id": "k", "start": 0.5, "lower": 0.01, "upper": 10.0}],
        "conditions": POINTS.iter().enumerate().map(|(at, y)| json!({
            "id": format!("point {at}"),
            "request": point(*y),
            "pinned": if at == 2 { json!([{"parameter": 0, "value": K}]) } else { Value::Null },
        })).collect::<Vec<_>>(),
        "curves": [{
            "column": "y",
            "times": vec![TIME; X.len()],
            "values": interpolated(K),
            "dimension": {
                "conditions": ["point 0", "point 1", "point 2"],
                "values": POINTS,
                "x": X,
            },
        }],
    })
}

#[test]
fn a_request_carries_a_curve_over_a_dimension_and_a_pin() {
    let request: FitRequest = serde_json::from_value(json()).unwrap();
    let problem = problem_of(&request, model(), Some(1)).unwrap();

    let residuals = problem.residuals(&[0.5]).unwrap();

    // the last point holds `k` at `K`, so its value is the one of the data
    let values: Vec<f64> = [simulated(1.0, 0.5), simulated(2.0, 0.5), simulated(4.0, K)].to_vec();
    let want: Vec<f64> = X
        .iter()
        .zip(interpolated(K).iter())
        .map(|(x, value)| interp(*x, &values) - value)
        .collect();
    assert_eq!(residuals, want);
    assert_eq!(FIELDS, ["curve.dimension", "condition.pinned"]);
}

#[test]
fn a_curve_of_a_condition_and_a_dimension_is_refused() {
    let mut value = json();
    value["curves"][0]["condition"] = json!("point 0");
    let request: FitRequest = serde_json::from_value(value).unwrap();

    let refusal = problem_of(&request, model(), Some(1))
        .unwrap_err()
        .to_string();

    assert!(
        refusal.contains("reads the condition 'point 0' and a dimension"),
        "{refusal}"
    );
}

#[test]
fn a_dimension_of_a_condition_the_fit_has_not_is_refused() {
    let mut value = json();
    value["curves"][0]["dimension"]["conditions"][1] = json!("point 7");
    let request: FitRequest = serde_json::from_value(value).unwrap();

    let refusal = problem_of(&request, model(), Some(1))
        .unwrap_err()
        .to_string();

    assert!(
        refusal.contains("reads the condition 'point 7' it has not"),
        "{refusal}"
    );
}

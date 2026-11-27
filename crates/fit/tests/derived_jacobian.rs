//! The jacobian of a fit whose conditions carry derived values, out of the forward
//! sensitivities of their conditions (sbmlsim-core#136).
//!
//! A derived value is a linear form `constant + Σ factor * p` over the parameters of the fit
//! which is set on its target before every simulation ([`Derived`]). Its target is a varying
//! target of the condition like the target of a parameter, so the run of the sensitivities
//! carries its sensitivity as well, and the column of a parameter is the chain rule: the
//! sensitivity of the target of the parameter, unless the condition pins it, and the
//! sensitivity of the target of every derived value times the factor of its term of the
//! parameter.
//!
//! The model is the one compartment of an oral dose, `gut' = -ka gut`,
//! `cent' = ka gut - ke cent` and the concentration `C = cent / V + s`, whose `a` reads nothing
//! and is the parameter of a reparametrization: a fit of `a` with `ke = KE0 + SLOPE a` is the
//! fit of `ke` with `ke` of that value, so the two jacobians are the same up to the factor of
//! the form. The species `s` of a compartment of 2 adds its concentration to `C`, the target
//! of a derived value as an amount or as a concentration.

use std::sync::Arc;

use sbmlsim_abi::SimulationRequest;
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_fit::{
    Condition, Curve, Definition, Derived, Dimension, Form, JacobianChoice, Likelihood, NoiseKind,
    Parameter, Pinned, Problem, Reduction, Scale, Term, fisher,
};
use sbmlsim_host::LoadedModel;
use serde_json::json;

#[path = "support/fixture.rs"]
mod fixture;

use fixture::runtime;

/// The constant and the factor of `ke = KE0 + SLOPE a` of the reparametrization.
const KE0: f64 = 0.1;
const SLOPE: f64 = 0.5;

/// The value of `a` whose `ke` is the one of the model, `0.15`.
const A: f64 = 0.1;

/// The doses of the points of the scan, the initial values of `gut`.
const DOSES: [f64; 3] = [5.0, 10.0, 20.0];

const SBML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="oral">
    <listOfCompartments>
      <compartment id="body" size="2" constant="true"/>
    </listOfCompartments>
    <listOfSpecies>
      <species id="s" compartment="body" initialConcentration="0.1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
    </listOfSpecies>
    <listOfParameters>
      <parameter id="gut" value="10" constant="false"/>
      <parameter id="cent" value="0" constant="false"/>
      <parameter id="C" value="0" constant="false"/>
      <parameter id="ka" value="1" constant="true"/>
      <parameter id="ke" value="0.15" constant="true"/>
      <parameter id="V" value="10" constant="true"/>
      <parameter id="a" value="0" constant="true"/>
    </listOfParameters>
    <listOfRules>
      <assignmentRule variable="C">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><plus/>
            <apply><divide/><ci>cent</ci><ci>V</ci></apply>
            <ci>s</ci>
          </apply>
        </math>
      </assignmentRule>
      <rateRule variable="gut">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/><apply><minus/><ci>ka</ci></apply><ci>gut</ci></apply>
        </math>
      </rateRule>
      <rateRule variable="cent">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><minus/>
            <apply><times/><ci>ka</ci><ci>gut</ci></apply>
            <apply><times/><ci>ke</ci><ci>cent</ci></apply>
          </apply>
        </math>
      </rateRule>
    </listOfRules>
  </model>
</sbml>
"#;

fn model() -> Arc<LoadedModel> {
    let compiled = compile(SBML, &CompileOptions::default()).unwrap();
    Arc::new(
        runtime()
            .load(&compiled.wasm, &compiled.description)
            .unwrap(),
    )
}

/// The request of the concentration over a day on a grid of half an hour, integrated to a
/// tolerance far below the steps of the central differences it is checked against.
fn request() -> SimulationRequest {
    let mut request: serde_json::Value =
        serde_json::from_str(include_str!("../../host/tests/decay-model/request.json")).unwrap();
    request["end"] = json!(24.0);
    request["output"] = json!({ "uniform": { "steps": 48 } });
    request["selections"] = json!(["time", "C"]);
    request["settings"]["relative_tolerance"] = json!(1e-12);
    request["settings"]["absolute_tolerance"] = json!({ "scalar": 1e-14 });
    request["settings"]["maximum_num_steps"] = json!(100_000);
    serde_json::from_value(request).unwrap()
}

/// The times of the grid of [`request`].
fn times() -> Vec<f64> {
    (0..=48).map(|step| f64::from(step) * 0.5).collect()
}

fn parameter(id: &str, start: f64, lower: f64, upper: f64, scale: Scale) -> Parameter {
    Parameter {
        id: id.to_string(),
        start,
        lower,
        upper,
        scale,
    }
}

/// A derived value of a target, `constant + Σ factor * p` of the terms `(parameter, factor)`.
fn derived(target: &str, constant: f64, terms: &[(usize, f64)]) -> Derived {
    Derived {
        target: target.to_string(),
        constant,
        terms: terms
            .iter()
            .map(|(parameter, factor)| Term {
                parameter: *parameter,
                factor: *factor,
            })
            .collect(),
        form: None,
    }
}

/// The product `factor * Π p` of parameters of the fit, a value which is no linear form of
/// them.
#[derive(Debug)]
struct Product {
    parameters: Vec<usize>,
    factor: f64,
}

impl Form for Product {
    fn parameters(&self) -> &[usize] {
        &self.parameters
    }

    fn value(&self, parameters: &[f64]) -> f64 {
        self.parameters
            .iter()
            .fold(self.factor, |product, k| product * parameters[*k])
    }

    fn gradient(&self, parameters: &[f64]) -> Vec<f64> {
        (0..self.parameters.len())
            .map(|at| {
                self.parameters
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != at)
                    .fold(self.factor, |product, (_, k)| product * parameters[*k])
            })
            .collect()
    }
}

/// A derived value of a target, the linear form of [`derived`] plus `factor * Π p` of the
/// parameters `product`.
fn nonlinear(
    target: &str,
    constant: f64,
    terms: &[(usize, f64)],
    factor: f64,
    product: &[usize],
) -> Derived {
    Derived {
        form: Some(Arc::new(Product {
            parameters: product.to_vec(),
            factor,
        })),
        ..derived(target, constant, terms)
    }
}

fn condition(derived: Vec<Derived>) -> Condition {
    Condition {
        request: request(),
        model: None,
        derived,
        pinned: Vec::new(),
    }
}

/// The curve of the concentration on every row of the grid of a condition, of made up data:
/// the jacobian of a least squares does not read the data.
fn timecourse(condition: usize) -> Curve {
    let times = times();
    Curve {
        condition,
        column: "C".to_string(),
        values: times.iter().map(|t| 0.4 * (-0.1 * t).exp()).collect(),
        noise: Some(vec![0.05; times.len()]),
        times,
        weight: 1.0,
        after_change: None,
        likelihood: None,
        reduction: None,
        dimension: None,
    }
}

/// The curve of the largest concentration of a condition, the cmax of a study.
fn cmax(condition: usize) -> Curve {
    Curve {
        condition,
        column: "C".to_string(),
        times: Vec::new(),
        values: vec![0.6],
        noise: Some(vec![0.05]),
        weight: 1.0,
        after_change: None,
        likelihood: None,
        reduction: Some(Reduction::Max { factor: 1.0 }),
        dimension: None,
    }
}

/// The curve of the concentration of a condition as the likelihood of a normal noise.
fn likelihood(condition: usize) -> Curve {
    Curve {
        likelihood: Some(Likelihood {
            distribution: NoiseKind::Normal,
            noise_column: None,
        }),
        ..timecourse(condition)
    }
}

/// A fit of one condition of the curves of [`timecourse`] and [`cmax`].
fn single(
    parameters: Vec<Parameter>,
    derived: Vec<Derived>,
    jacobian: JacobianChoice,
) -> Definition {
    Definition {
        parameters,
        conditions: vec![condition(derived)],
        curves: vec![timecourse(0), cmax(0)],
        jacobian,
        priors: Vec::new(),
        prior_correlation: None,
    }
}

/// The jacobian of the residuals by a central difference of the residuals themselves, on a
/// step of 1e-5 of the parameter: its truncation is about 1e-10 of itself and the noise of
/// the integrator over the step 1e-7, which is what an exact jacobian is checked against.
fn central(problem: &Problem, x: &[f64]) -> Vec<f64> {
    let k = problem.parameters().len();
    let n = problem.n_residuals();
    let mut jacobian = vec![0.0; n * k];
    for column in 0..k {
        let step = 1e-5 * x[column].abs().max(1.0);
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

/// The largest difference of two jacobians of the same shape, relative to the largest value
/// of the column, so that a residual whose derivative is next to zero does not count as a
/// relative difference of its own.
fn apart(one: &[f64], other: &[f64], k: usize) -> f64 {
    assert_eq!(one.len(), other.len());
    let mut scale = vec![0.0f64; k];
    for (at, value) in one.iter().enumerate() {
        scale[at % k] = scale[at % k].max(value.abs());
    }
    one.iter()
        .zip(other)
        .enumerate()
        .map(|(at, (a, b))| (a - b).abs() / scale[at % k].max(1e-300))
        .fold(0.0, f64::max)
}

/// The jacobian of the sensitivities of a definition at its start, checked to be one
/// simulation per condition, and its distance to the central differences.
fn checked(definition: Definition) -> (Problem, Vec<f64>, f64) {
    let conditions = definition.conditions.len() as u64;
    let problem = Problem::new(definition, model(), 2).unwrap();
    assert_eq!(problem.jacobian_of(), JacobianChoice::Sensitivities);
    let x = problem.start();

    let before = problem.simulations();
    let jacobian = problem.jacobian(&x).unwrap();

    assert_eq!(
        problem.simulations() - before,
        conditions,
        "the jacobian is not one simulation per condition"
    );
    let apart = apart(
        &jacobian,
        &central(&problem, &x),
        problem.parameters().len(),
    );
    (problem, jacobian, apart)
}

#[test]
fn a_volume_which_follows_the_rate_of_absorption_is_the_chain_rule() {
    // `V = 5 ka + 5`, the case of sbmlsim-core#136: the forward differences were 8e-4 off
    for scale in [Scale::Lin, Scale::Log10] {
        let definition = single(
            vec![parameter("ka", 1.0, 0.1, 10.0, scale)],
            vec![derived("V", 5.0, &[(0, 5.0)])],
            JacobianChoice::Sensitivities,
        );

        let (_, _, apart) = checked(definition);

        assert!(apart < 1e-6, "{scale:?}: the jacobian is {apart} off");
    }
}

#[test]
fn a_rate_of_elimination_which_follows_the_rate_of_absorption_is_the_chain_rule() {
    // `ke = 5 ka + 0.15`: the forward differences were 3.4 % off
    for scale in [Scale::Lin, Scale::Log10] {
        let definition = single(
            vec![parameter("ka", 1.0, 0.1, 10.0, scale)],
            vec![derived("ke", 0.15, &[(0, 5.0)])],
            JacobianChoice::Sensitivities,
        );

        let (_, _, apart) = checked(definition);

        assert!(apart < 1e-6, "{scale:?}: the jacobian is {apart} off");
    }
}

#[test]
fn a_derived_value_of_several_parameters_sums_their_terms() {
    // `V = 1 + 2 ka + 30 ke + 1.5 ka`: two terms of one parameter add up
    let definition = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Log10),
            parameter("ke", 0.15, 0.01, 1.0, Scale::Lin),
        ],
        vec![derived("V", 1.0, &[(0, 2.0), (1, 30.0), (0, 1.5)])],
        JacobianChoice::Sensitivities,
    );

    let (_, _, apart) = checked(definition);

    assert!(apart < 1e-6, "the jacobian is {apart} off");
}

#[test]
fn a_derived_value_which_is_no_linear_form_is_the_chain_rule_of_its_form() {
    // `V = 1 + 2 ka + 40 ka ke`, the product of two parameters of two scales, and `ke` of a
    // square of `ka`, `ke = 0.05 + 0.1 ka^2`, which reads one parameter twice
    for scale in [Scale::Lin, Scale::Log10] {
        for (parameters, derived) in [
            (
                vec![
                    parameter("ka", 1.0, 0.1, 10.0, scale),
                    parameter("a", 0.15, 0.01, 1.0, Scale::Lin),
                ],
                vec![nonlinear("V", 1.0, &[(0, 2.0)], 40.0, &[0, 1])],
            ),
            (
                vec![parameter("ka", 1.0, 0.1, 10.0, scale)],
                vec![nonlinear("ke", 0.05, &[], 0.1, &[0, 0])],
            ),
        ] {
            let definition = single(parameters, derived, JacobianChoice::Sensitivities);

            let (_, jacobian, apart) = checked(definition);

            assert!(apart < 1e-6, "{scale:?}: the jacobian is {apart} off");
            assert!(jacobian.iter().any(|d| *d != 0.0), "{jacobian:?}");
        }
    }
}

#[test]
fn a_form_is_evaluated_where_the_optimizer_is() {
    // the value of a form is the one of the parameters on the linear scale: a fit of `ka`
    // with `V = 20 ka^2` simulates the model of `V = 20 ka^2` at every point, as a fit of the
    // differences of the residuals sees
    let form = || vec![nonlinear("V", 0.0, &[], 20.0, &[0, 0])];
    let sensitivities = single(
        vec![parameter("ka", 0.7, 0.1, 10.0, Scale::Log10)],
        form(),
        JacobianChoice::Sensitivities,
    );
    let mut fixed = single(
        vec![parameter("ka", 0.7, 0.1, 10.0, Scale::Log10)],
        Vec::new(),
        JacobianChoice::DifferenceQuotient,
    );
    fixed.conditions[0].request.preinit = vec![("V".to_string(), 20.0 * 0.7 * 0.7)];
    let one = Problem::new(sensitivities, model(), 2).unwrap();
    let other = Problem::new(fixed, model(), 2).unwrap();

    let (a, b) = (
        one.residuals(&one.start()).unwrap(),
        other.residuals(&other.start()).unwrap(),
    );

    for (a, b) in a.iter().zip(&b) {
        assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{a} != {b}");
    }
}

#[test]
fn a_problem_which_holds_a_parameter_of_a_form_reads_its_value() {
    let definition = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Log10),
            parameter("a", 0.15, 0.01, 1.0, Scale::Lin),
        ],
        vec![nonlinear("V", 1.0, &[], 40.0, &[0, 1])],
        JacobianChoice::Sensitivities,
    );
    let problem = Problem::new(definition, model(), 2).unwrap();

    for held in [0, 1] {
        let fixed = problem.fixing(&[(held, [1.5, 0.2][held])]).unwrap();
        let x = fixed.start();

        let jacobian = fixed.jacobian(&x).unwrap();

        let apart = apart(&jacobian, &central(&fixed, &x), 1);
        assert!(apart < 1e-6, "holding {held}: the jacobian is {apart} off");
    }
}

#[test]
fn a_form_which_reads_a_parameter_the_fit_has_not_is_refused() {
    let definition = single(
        vec![parameter("ka", 1.0, 0.1, 10.0, Scale::Lin)],
        vec![nonlinear("V", 1.0, &[], 40.0, &[0, 3])],
        JacobianChoice::Sensitivities,
    );

    let refusal = Problem::new(definition, model(), 2)
        .err()
        .unwrap()
        .to_string();

    assert_eq!(
        refusal,
        "the condition 0 has the derived value 'V', whose form reads the parameter 3 of a fit \
         of 1 parameters"
    );
}

#[test]
fn the_rows_of_a_likelihood_follow_the_derived_values() {
    let mut definition = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Log10),
            parameter("ke", 0.15, 0.01, 1.0, Scale::Log10),
        ],
        vec![derived("V", 5.0, &[(0, 5.0), (1, 10.0)])],
        JacobianChoice::Sensitivities,
    );
    definition.curves = vec![likelihood(0)];

    let (_, _, apart) = checked(definition);

    assert!(apart < 1e-6, "the jacobian is {apart} off");
}

#[test]
fn a_derived_value_reads_a_parameter_its_condition_pins() {
    // the condition holds `ka` at 2, so its column is the one of `V = 5 ka + 5` alone
    let mut definition = single(
        vec![parameter("ka", 1.0, 0.1, 10.0, Scale::Log10)],
        vec![derived("V", 5.0, &[(0, 5.0)])],
        JacobianChoice::Sensitivities,
    );
    definition.conditions[0].pinned = vec![Pinned {
        parameter: 0,
        value: 2.0,
    }];

    let (_, jacobian, apart) = checked(definition);

    assert!(apart < 1e-6, "the jacobian is {apart} off");
    assert!(jacobian.iter().any(|d| *d != 0.0), "{jacobian:?}");
}

#[test]
fn a_problem_which_holds_a_parameter_of_a_derived_value_drops_its_term() {
    let definition = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Log10),
            parameter("ke", 0.15, 0.01, 1.0, Scale::Lin),
        ],
        vec![derived("V", 1.0, &[(0, 2.0), (1, 30.0)])],
        JacobianChoice::Sensitivities,
    );
    let problem = Problem::new(definition, model(), 2).unwrap();

    for held in [0, 1] {
        let fixed = problem.fixing(&[(held, [1.5, 0.2][held])]).unwrap();
        let x = fixed.start();

        let jacobian = fixed.jacobian(&x).unwrap();

        assert_eq!(fixed.jacobian_of(), JacobianChoice::Sensitivities);
        let apart = apart(&jacobian, &central(&fixed, &x), 1);
        assert!(apart < 1e-6, "holding {held}: the jacobian is {apart} off");
    }
}

#[test]
fn a_curve_over_a_dimension_follows_the_derived_values_of_its_points() {
    // the scan over the dose, every point a condition with `V = 5 ka + 5`
    let mut conditions = Vec::new();
    for dose in DOSES {
        let mut condition = condition(vec![derived("V", 5.0, &[(0, 5.0)])]);
        condition.request.preinit = vec![("gut".to_string(), dose)];
        conditions.push(condition);
    }
    let x = [7.5, 10.0, 15.0, 20.0];
    let definition = Definition {
        parameters: vec![parameter("ka", 1.0, 0.1, 10.0, Scale::Log10)],
        conditions,
        curves: vec![Curve {
            condition: 0,
            column: "C".to_string(),
            times: vec![2.0; x.len()],
            values: vec![0.5; x.len()],
            noise: None,
            weight: 1.0,
            after_change: None,
            likelihood: None,
            reduction: None,
            dimension: Some(Dimension {
                conditions: (0..DOSES.len()).collect(),
                values: DOSES.to_vec(),
                x: x.to_vec(),
                reductions: Vec::new(),
            }),
        }],
        jacobian: JacobianChoice::Sensitivities,
        priors: Vec::new(),
        prior_correlation: None,
    };

    let (_, _, apart) = checked(definition);

    assert!(apart < 1e-6, "the jacobian is {apart} off");
}

/// The fit of `ka` and `a` with `ke = KE0 + SLOPE a`, and the same fit of `ka` and `ke`.
fn reparametrized(jacobian: JacobianChoice) -> (Definition, Definition) {
    let derived = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Lin),
            parameter("a", A, -0.1, 1.0, Scale::Lin),
        ],
        vec![derived("ke", KE0, &[(1, SLOPE)])],
        jacobian,
    );
    let direct = single(
        vec![
            parameter("ka", 1.0, 0.1, 10.0, Scale::Lin),
            parameter("ke", KE0 + SLOPE * A, 0.01, 1.0, Scale::Lin),
        ],
        Vec::new(),
        jacobian,
    );
    (derived, direct)
}

#[test]
fn the_fisher_information_is_the_one_of_the_fit_without_the_derived_value() {
    // the column of `a` is SLOPE times the one of `ke`, so the information of the fit of `a`
    // is `Tᵀ I T` of the one of `ke` with `T = diag(1, SLOPE)`, and the standard error of `a`
    // is the one of `ke` over SLOPE
    let (derived, direct) = reparametrized(JacobianChoice::Sensitivities);
    let derived = Problem::new(derived, model(), 2).unwrap();
    let direct = Problem::new(direct, model(), 2).unwrap();
    assert_eq!(derived.jacobian_of(), JacobianChoice::Sensitivities);

    let one = fisher(&derived, &derived.start()).unwrap();
    let other = fisher(&direct, &direct.start()).unwrap();

    let t = [1.0, SLOPE];
    for row in 0..2 {
        for column in 0..2 {
            let want = other.information[row * 2 + column] * t[row] * t[column];
            let got = one.information[row * 2 + column];
            assert!(
                (got - want).abs() < 1e-9 * want.abs(),
                "the information ({row}, {column}) is {got}, the reparametrization {want}"
            );
        }
    }
    let (one, other) = (one.errors.unwrap(), other.errors.unwrap());
    assert!((one[0] / other[0] - 1.0).abs() < 1e-9, "{one:?} {other:?}");
    assert!(
        (one[1] * SLOPE / other[1] - 1.0).abs() < 1e-9,
        "{one:?} {other:?}"
    );
}

#[test]
fn a_fit_with_derived_values_takes_the_jacobian_it_asks_for() {
    for (asked, taken) in [
        (JacobianChoice::Sensitivities, JacobianChoice::Sensitivities),
        (
            JacobianChoice::DifferenceQuotient,
            JacobianChoice::DifferenceQuotient,
        ),
        // what is fastest for the model, the differences today, with or without derived values
        (JacobianChoice::Auto, JacobianChoice::DifferenceQuotient),
    ] {
        let (definition, _) = reparametrized(asked);

        let problem = Problem::new(definition, model(), 1).unwrap();

        assert_eq!(problem.jacobian_of(), taken, "asked for {asked:?}");
    }
}

#[test]
fn a_derived_amount_and_concentration_of_a_species_are_the_chain_rule() {
    // the species lives in a compartment of 2, so its amount and its concentration differ by
    // that factor, which the sensitivity of either target carries
    for target in ["s", "[s]"] {
        let definition = single(
            vec![
                parameter("ka", 1.0, 0.1, 10.0, Scale::Log10),
                parameter("ke", 0.15, 0.01, 1.0, Scale::Lin),
            ],
            vec![derived(target, 0.05, &[(0, 0.1), (1, 0.4)])],
            JacobianChoice::Sensitivities,
        );

        let (_, _, apart) = checked(definition);

        assert!(apart < 1e-6, "{target}: the jacobian is {apart} off");
    }
}

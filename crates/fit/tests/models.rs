//! A fit of several models, whose conditions name the model each of them simulates.
//!
//! The two models are the decay `y' = -f k y` of two factors `f`, compiled from SBML, so a
//! condition which is simulated on the other model of the fit has other residuals: the data
//! of every condition is what its own model simulates at [`fixture::K`].

use std::sync::Arc;

use sbmlsim_abi::SimulationRequest;
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_fit::{Condition, Curve, Definition, FitError, Parameter, Problem, Scale};
use sbmlsim_host::LoadedModel;

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{K, request, runtime};

/// The decay model whose rate is `factor` times `k`.
fn sbml(factor: f64) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="decay">
    <listOfParameters>
      <parameter id="y" value="1" constant="false"/>
      <parameter id="k" value="0.5" constant="true"/>
    </listOfParameters>
    <listOfRules>
      <rateRule variable="y">
        <math xmlns="http://www.w3.org/1998/Math/MathML">
          <apply><times/>
            <apply><minus/><cn>{factor}</cn></apply>
            <ci>k</ci>
            <ci>y</ci>
          </apply>
        </math>
      </rateRule>
    </listOfRules>
  </model>
</sbml>
"#
    )
}

/// The model of a factor, loaded in the runtime of the tests.
fn model(factor: f64) -> Arc<LoadedModel> {
    let compiled = compile(&sbml(factor), &CompileOptions::default()).unwrap();
    Arc::new(
        runtime()
            .load(&compiled.wasm, &compiled.description)
            .unwrap(),
    )
}

/// The slow and the fast model of the fit, by the id their conditions name them by.
fn models() -> Vec<(String, Arc<LoadedModel>)> {
    vec![
        ("slow".to_string(), model(1.0)),
        ("fast".to_string(), model(2.0)),
    ]
}

/// The data of a model at `k`, the time and `y` of its simulation.
fn data(model: &Arc<LoadedModel>, k: f64) -> (Vec<f64>, Vec<f64>) {
    let mut instance = model.instantiate_native().unwrap();
    let prepared = model.prepare(&request(), &["k".to_string()]).unwrap();
    let table = instance.simulate_prepared(&prepared, &[k]).unwrap();
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

/// A condition of the request on the model of an id.
fn condition(id: Option<&str>) -> Condition {
    Condition {
        request: request(),
        model: id.map(str::to_string),
        derived: Vec::new(),
        pinned: Vec::new(),
    }
}

/// The fit of `k` from `start` against the data of both models at [`fixture::K`].
fn definition(start: f64, models: &[(String, Arc<LoadedModel>)]) -> Definition {
    let curves = models
        .iter()
        .enumerate()
        .map(|(at, (_, model))| {
            let (times, values) = data(model, K);
            Curve {
                condition: at,
                column: "y".to_string(),
                times,
                values,
                noise: None,
                weight: 1.0,
                after_change: None,
                likelihood: None,
                reduction: None,
                dimension: None,
            }
        })
        .collect();
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
        conditions: models
            .iter()
            .map(|(id, _)| condition(Some(id.as_str())))
            .collect(),
        curves,
    }
}

#[test]
fn the_residuals_of_a_fit_of_two_models_are_zero_at_the_values_of_their_data() {
    let models = models();
    let problem = Problem::of_models(definition(K, &models), models, 2).unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert_eq!(residuals.len(), 22);
    assert!(
        residuals.iter().all(|value| *value == 0.0),
        "the residuals of the data of the two models are not zero: {residuals:?}"
    );
}

#[test]
fn every_condition_is_simulated_on_the_model_it_names() {
    let models = models();
    let definition = definition(K, &models);
    // the models under each other's id, so every condition gets the other model
    let swapped = vec![
        ("slow".to_string(), models[1].1.clone()),
        ("fast".to_string(), models[0].1.clone()),
    ];
    let problem = Problem::of_models(definition, swapped, 2).unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();

    assert!(
        residuals.iter().any(|value| value.abs() > 1e-3),
        "the conditions do not depend on the model they name: {residuals:?}"
    );
}

#[test]
fn a_fit_of_two_models_finds_the_parameter_of_their_data() {
    let models = models();
    let problem = Problem::of_models(definition(1.0, &models), models, 2).unwrap();

    let fit = problem.optimize(&Default::default()).unwrap();

    assert!(
        (fit.x[0] - K).abs() < 1e-6,
        "the fit of the two models found {} and not {K}",
        fit.x[0]
    );
}

#[test]
fn a_condition_of_a_fit_of_several_models_names_one_of_them() {
    let models = models();
    let mut definition = definition(K, &models);
    definition.conditions[1] = condition(None);

    match Problem::of_models(definition, models, 1) {
        Err(FitError::Condition { at, message }) => {
            assert_eq!(at, 1);
            assert!(
                message.contains("names no model") && message.contains("2 of them"),
                "the message of the condition without a model is '{message}'"
            );
        }
        other => panic!("a condition without a model of a fit of two is {other:?}"),
    }
}

#[test]
fn a_condition_which_names_a_model_the_fit_has_not_is_refused() {
    let models = models();
    let mut definition = definition(K, &models);
    definition.conditions[0] = condition(Some("female"));

    match Problem::of_models(definition, models, 1) {
        Err(FitError::Condition { at, message }) => {
            assert_eq!(at, 0);
            assert!(
                message.contains("'female'") && message.contains("which the fit has not"),
                "the message of the unknown model is '{message}'"
            );
        }
        other => panic!("a condition of an unknown model is {other:?}"),
    }
}

#[test]
fn a_condition_of_a_fit_of_one_model_needs_no_model() {
    let request: SimulationRequest = request();
    let models = models();
    let (times, values) = data(&models[0].1, K);
    let definition = Definition {
        jacobian: Default::default(),
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![Parameter {
            id: "k".to_string(),
            start: K,
            lower: 0.01,
            upper: 10.0,
            scale: Scale::Lin,
        }],
        conditions: vec![request.into()],
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
            dimension: None,
        }],
    };

    let problem = Problem::of_models(
        definition,
        vec![("slow".to_string(), models[0].1.clone())],
        1,
    )
    .unwrap();

    let residuals = problem.residuals(&problem.start()).unwrap();
    assert!(residuals.iter().all(|value| *value == 0.0));
}

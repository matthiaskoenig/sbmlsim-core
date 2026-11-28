//! Driven species on the stepper: the paths of the species set between the advances
//! ([`Stepper::set_parameters`]), the amounts the model exchanged with them against their
//! integrals, and the module of the compiler with the same species driven
//! ([`CompileOptions::driven`]).
//!
//! The stepper advances the native model of the C target of the driven model representation
//! ([`sbmlsim_ir::driven::apply`]); the module of the compiler simulates in the host. They run
//! natively only: the C target and the host are no crates of `wasm32`.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../../c/tests/common/mod.rs"]
mod common;

use sbmlsim_abi::{
    IntegratorSettings, ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest, Tolerance,
};
use sbmlsim_c::{CModel, Library, generate_model};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_engine::{EngineError, Stepper};
use sbmlsim_host::Runtime;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// `X -> C` of the rate `(k * blood) * X`, the amount of `X` times `k`: `X` in `blood`, whose
/// size is the parameter `Vb` (an initial assignment), given by the initial amount `0.3` (so
/// the initial value of its path is an initial evaluation, `0.3 / blood`), `C` in `cell` of
/// size 1. The product `k * blood` reads parameters only, a constant of the optimizer.
fn uptake() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="uptake">
<listOfCompartments><compartment id="blood" constant="true"/><compartment id="cell" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="X" compartment="blood" initialAmount="0.3" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="k" value="0.7" constant="true"/><parameter id="Vb" value="0.15" constant="true"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="blood"><math {MATH}><ci>Vb</ci></math></initialAssignment></listOfInitialAssignments>
<listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math {MATH}><apply><times/><apply><times/><ci>k</ci><ci>blood</ci></apply><ci>X</ci></apply></math></kineticLaw></reaction></listOfReactions>
</model></sbml>"#
    )
}

const K: f64 = 0.7;
const VB: f64 = 0.15;

fn driven(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// The native model of the C target of the model with the species `ids` driven, and its
/// description.
fn native(sbml: &str, ids: &[&str]) -> (CModel, ModelDescription) {
    let doc = sbmlsim_sbml::read(sbml).unwrap();
    let model = sbmlsim_ir::driven::apply(sbmlsim_ir::build(&doc).unwrap(), &driven(ids)).unwrap();
    let source = generate_model(&model).unwrap();
    let library = Library::build(&source.code).unwrap();
    let model = CModel::new(library, &source.description).unwrap();
    (model, source.description)
}

/// The module of the compiler with the species `ids` driven.
fn wasm(sbml: &str, ids: &[&str]) -> CompiledModel {
    let options = CompileOptions {
        parameter_jacobian: false,
        driven: driven(ids),
        ..CompileOptions::default()
    };
    compile(sbml, &options).unwrap()
}

fn settings() -> IntegratorSettings {
    IntegratorSettings {
        relative_tolerance: 1e-10,
        absolute_tolerance: Tolerance::Scalar(1e-14),
        ..IntegratorSettings::default()
    }
}

fn parameter(desc: &ModelDescription, id: &str) -> u32 {
    desc.parameters.iter().position(|p| p.id == id).unwrap() as u32
}

fn state(desc: &ModelDescription, id: &str) -> usize {
    desc.states.iter().position(|s| s.id == id).unwrap()
}

/// The path `(value, slope, t_ref)` of `X` as the values of its parameters.
fn path(desc: &ModelDescription, value: f64, slope: f64, t_ref: f64) -> Vec<(u32, f64)> {
    let x = &desc.driven[0];
    vec![
        (x.value, value),
        (x.slope, slope),
        (desc.t_ref.unwrap(), t_ref),
    ]
}

/// The integral of the path `value + slope (t - t_ref)` from `t0` to `t1`.
fn integral(value: f64, slope: f64, t_ref: f64, t0: f64, t1: f64) -> f64 {
    let primitive = |t: f64| value * t + slope * (t - t_ref).powi(2) / 2.0;
    primitive(t1) - primitive(t0)
}

fn relative(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs()
}

#[test]
fn exchange_matches_closed_form() {
    let (model, desc) = native(&uptake(), &["X"]);
    let ex = state(&desc, "X__exchange");
    let c = state(&desc, "C");
    let mut stepper = Stepper::new(Box::new(model), desc.clone(), settings(), &[]).unwrap();
    // the initial path: the initial amount over the size of blood
    let value = desc.driven[0].value as usize;
    assert_eq!(stepper.parameters()[value], 0.3 / VB);
    stepper
        .set_parameters(&path(&desc, 2.0, -0.5, 0.0))
        .unwrap();
    let y = stepper.state().to_vec();
    assert_eq!((y[c], y[ex]), (0.0, 0.0));
    stepper.advance(&y, 0.0, 1.5).unwrap();

    let expected = -K * VB * (2.0 * 1.5 - 0.5 * 1.5 * 1.5 / 2.0);
    assert_eq!(expected, -K * VB * integral(2.0, -0.5, 0.0, 0.0, 1.5));
    let y = stepper.state();
    assert!(
        relative(y[ex], expected) <= 1e-8,
        "exchange {} for {expected}",
        y[ex]
    );
    // C gains what X gave: the rates of the two are the negatives of each other
    assert_eq!(y[c], -y[ex]);
    assert_eq!(stepper.parameters()[value], 2.0);
}

#[test]
fn parameters_change_between_advances() {
    let (model, desc) = native(&uptake(), &["X"]);
    let ex = state(&desc, "X__exchange");
    let k = parameter(&desc, "k");
    let value = desc.driven[0].value as usize;
    let mut stepper = Stepper::new(Box::new(model), desc.clone(), settings(), &[]).unwrap();

    stepper
        .set_parameters(&path(&desc, 2.0, -0.5, 0.0))
        .unwrap();
    let y = stepper.state().to_vec();
    stepper.advance(&y, 0.0, 1.0).unwrap();
    // another path, referred to the start of the second interval, and another `k`: the
    // constant `k * blood` follows it
    let mut second = path(&desc, 1.2, 0.8, 1.0);
    second.push((k, 0.4));
    stepper.set_parameters(&second).unwrap();
    let y = stepper.state().to_vec();
    stepper.advance(&y, 1.0, 2.5).unwrap();

    let expected =
        -VB * (K * integral(2.0, -0.5, 0.0, 0.0, 1.0) + 0.4 * integral(1.2, 0.8, 1.0, 1.0, 2.5));
    let y = stepper.state();
    assert!(
        relative(y[ex], expected) <= 1e-8,
        "exchange {} for {expected}",
        y[ex]
    );
    assert_eq!(y[state(&desc, "C")], -y[ex]);
    // the initial evaluation `X__value = 0.3 / blood` did not run again
    assert_eq!(stepper.parameters()[value], 1.2);
    assert_eq!(stepper.parameters()[k as usize], 0.4);
}

#[test]
fn set_parameters_refusals_change_nothing() {
    let (model, desc) = native(&uptake(), &["X"]);
    let mut stepper = Stepper::new(Box::new(model), desc.clone(), settings(), &[]).unwrap();
    let before = stepper.parameters().to_vec();
    let value = desc.driven[0].value;
    let n = desc.parameters.len() as u32;
    for values in [
        vec![(value, 2.0), (n, 1.0)],
        vec![(value, 2.0), (desc.driven[0].slope, f64::NAN)],
        vec![(value, f64::INFINITY)],
    ] {
        let error = stepper.set_parameters(&values).unwrap_err();
        assert!(matches!(error, EngineError::Request(_)), "{error:?}");
        assert_eq!(stepper.parameters(), before);
    }
    // the stepper still advances
    let y = stepper.state().to_vec();
    stepper.advance(&y, 0.0, 1.0).unwrap();
}

#[test]
fn set_parameters_refuses_the_target_of_a_rule() {
    let sbml = uptake()
        .replace(
            "</listOfParameters>",
            r#"<parameter id="a" constant="false"/></listOfParameters>"#,
        )
        .replace(
            "</listOfInitialAssignments>",
            &format!(
                r#"</listOfInitialAssignments><listOfRules><assignmentRule variable="a"><math {MATH}><apply><times/><cn>2</cn><ci>k</ci></apply></math></assignmentRule></listOfRules>"#
            ),
        );
    let (model, desc) = native(&sbml, &["X"]);
    let mut stepper = Stepper::new(Box::new(model), desc.clone(), settings(), &[]).unwrap();
    let error = stepper
        .set_parameters(&[(parameter(&desc, "a"), 1.0)])
        .unwrap_err();
    assert!(error.to_string().contains('a'), "{error}");
    assert!(matches!(error, EngineError::Request(_)), "{error:?}");
}

/// The module of the compiler drives as the native model: the description has the driven
/// species, their parameters and selections, and its simulation gives the closed form.
#[test]
fn compiled_module_matches_closed_form() {
    let compiled = wasm(&uptake(), &["X"]);
    let (_, native) = native(&uptake(), &["X"]);
    let desc = &compiled.description;
    assert_eq!(desc.driven, native.driven);
    assert_eq!(desc.t_ref, native.t_ref);
    assert_eq!(desc.states, native.states);
    assert_eq!(desc.parameters, native.parameters);
    for id in ["X__value", "X__slope", "driven__t_ref", "X__exchange"] {
        assert!(
            desc.selections.iter().any(|s| s.id == id),
            "no selection {id}"
        );
    }

    let req = SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 1.5,
        output: Output::Times(vec![0.0, 1.5]),
        // fixed while the initial assignments are evaluated: the initial evaluation of the
        // path does not overwrite it
        preinit: vec![("X__value".into(), 2.0), ("X__slope".into(), -0.5)],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec!["X__exchange".into(), "C".into(), "[X]".into()],
        settings: settings(),
        seed: 1,
    };
    let runtime = Runtime::embedded().unwrap();
    let mut instance = runtime
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(&req).unwrap();
    assert_eq!(result.n_rows, 2);
    let k = result.columns.len();
    let value = |row: usize, id: &str| {
        let column = result.columns.iter().position(|c| c == id).unwrap();
        result.values[row * k + column]
    };
    let expected = -K * VB * integral(2.0, -0.5, 0.0, 0.0, 1.5);
    let exchange = value(1, "X__exchange");
    assert!(
        relative(exchange, expected) <= 1e-8,
        "exchange {exchange} for {expected}"
    );
    assert_eq!(value(1, "C"), -exchange);
    // the concentration of X is its path
    assert_eq!(value(0, "[X]"), 2.0);
    assert_eq!(value(1, "[X]"), 2.0 - 0.5 * 1.5);
}

#[test]
fn driven_spt_model_compiles() {
    let sbml = common::cell_model();
    let ids = ["S_ext", "P_ext"];
    let compiled = wasm(&sbml, &ids);
    let (model, desc) = native(&sbml, &ids);
    assert_eq!(compiled.description.driven, desc.driven);
    assert_eq!(compiled.description.states, desc.states);
    assert!(desc.states.iter().all(|s| !ids.contains(&s.id.as_str())));

    let mut stepper = Stepper::new(Box::new(model), desc.clone(), settings(), &[]).unwrap();
    let s_ext = state(&desc, "S_ext__exchange");
    let p_ext = state(&desc, "P_ext__exchange");
    let mut exchanged = [0.0; 2];
    // one advance per minute, each with the paths of its interval: the substrate in the
    // blood falls, the product rises
    for minute in 0..60 {
        let (t0, t1) = (f64::from(minute), f64::from(minute + 1));
        let s = 1.0 - 0.01 * t0;
        let p = 0.002 * t0;
        stepper
            .set_parameters(&[
                (desc.driven[0].value, s),
                (desc.driven[0].slope, -0.01),
                (desc.driven[1].value, p),
                (desc.driven[1].slope, 0.002),
                (desc.t_ref.unwrap(), t0),
            ])
            .unwrap();
        let mut y = stepper.state().to_vec();
        // the exchange of each interval
        y[s_ext] = 0.0;
        y[p_ext] = 0.0;
        stepper.advance(&y, t0, t1).unwrap();
        let y = stepper.state();
        assert!(y.iter().all(|v| v.is_finite()), "at {t1}: {y:?}");
        exchanged[0] += y[s_ext];
        exchanged[1] += y[p_ext];
    }
    // the cell takes up the substrate
    assert!(exchanged[0] < 0.0, "{exchanged:?}");
}

//! The forward sensitivities of a compiled model out of `model_jac_p`, the analytic `df/dp` of
//! the code generator, against the difference quotient of the right hand side the engine takes
//! without it: the same derivative of the same model.wasm, which the description decides.

use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// A model of the species `S` of the reaction `S -> ` with the rate `kel Vmax S / g`, where
/// `Vmax = 2 w` is an initial assignment, `kel = k / F` an assignment rule whose derivative in
/// every parameter is a constant and `g = Km + q S` one whose derivative in `q` is a state:
/// the sensitivities of `k`, of `w` (which the right hand side reads through `Vmax` alone), of
/// `q` (whose column is the one the derivative of a rule a column before it must not reach)
/// and of `S` itself. The values leave `S` a third of itself at the end of the run.
fn sbml() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="s">
<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="C" initialAmount="10" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters>
  <parameter id="k" value="0.7" constant="true"/>
  <parameter id="F" value="2" constant="true"/>
  <parameter id="w" value="0.35" constant="true"/>
  <parameter id="Km" value="4" constant="true"/>
  <parameter id="Vmax" value="0" constant="true"/>
  <parameter id="kel" value="0" constant="false"/>
  <parameter id="q" value="0.25" constant="true"/>
  <parameter id="g" value="0" constant="false"/>
</listOfParameters>
<listOfInitialAssignments>
  <initialAssignment symbol="Vmax"><math {MATH}><apply><times/><cn>2</cn><ci>w</ci></apply></math></initialAssignment>
</listOfInitialAssignments>
<listOfRules>
  <assignmentRule variable="kel"><math {MATH}><apply><divide/><ci>k</ci><ci>F</ci></apply></math></assignmentRule>
  <assignmentRule variable="g"><math {MATH}><apply><plus/><ci>Km</ci><apply><times/><ci>q</ci><ci>S</ci></apply></apply></math></assignmentRule>
</listOfRules>
<listOfReactions><reaction id="J" reversible="false">
  <listOfReactants><speciesReference species="S" stoichiometry="1" constant="true"/></listOfReactants>
  <kineticLaw><math {MATH}><apply><divide/>
    <apply><times/><ci>kel</ci><ci>Vmax</ci><ci>S</ci></apply>
    <ci>g</ci>
  </apply></math></kineticLaw>
</reaction></listOfReactions>
</model></sbml>"#
    )
}

fn request(stiff: bool) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 20.0,
        output: Output::Uniform { steps: 20 },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec![
            "time".to_string(),
            "S".to_string(),
            "kel".to_string(),
            "g".to_string(),
        ],
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            stiff,
            ..IntegratorSettings::default()
        },
        seed: 0,
        sensitivities: vec![
            "k".to_string(),
            "w".to_string(),
            "q".to_string(),
            "S".to_string(),
        ],
        local_errors: false,
    }
}

/// The sensitivity of the column `column` of the row `row` in the parameter `parameter`.
fn sensitivity(result: &SimulationResult, row: usize, column: &str, parameter: usize) -> f64 {
    let sensitivities = result.sensitivities.as_ref().expect("the sensitivities");
    let at = result
        .columns
        .iter()
        .position(|name| name == column)
        .unwrap_or_else(|| panic!("no column {column}"));
    let k = sensitivities.parameters.len();
    sensitivities.values[(row * result.columns.len() + at) * k + parameter]
}

#[test]
fn the_analytic_df_dp_gives_the_sensitivities_of_the_difference_quotient() {
    let compiled = compile(&sbml(), &CompileOptions::default()).unwrap();
    let analytic = compiled
        .description
        .jacobian_p
        .as_ref()
        .expect("the model carries the Jacobian in its parameters");
    // the column of `k` holds the rate of `S`, the one of `kel` nothing (an assignment rule
    // recomputes it, and what it reads carries its chain)
    let parameter = |id: &str| {
        compiled
            .description
            .parameters
            .iter()
            .position(|e| e.id == id)
            .unwrap()
    };
    assert_eq!(analytic.rows, 1);
    assert_eq!(analytic.rows(parameter("k")), [0]);
    assert_eq!(analytic.rows(parameter("q")), [0]);
    assert_eq!(analytic.rows(parameter("Vmax")), [0]);
    assert!(analytic.rows(parameter("kel")).is_empty());
    // `w` is read by no rate: the initialization derives `Vmax` from it
    assert!(analytic.rows(parameter("w")).is_empty());

    // the same model.wasm without the pattern, whose `df/dp` is the difference quotient
    let mut quotient = compiled.description.clone();
    quotient.jacobian_p = None;
    let runtime = Runtime::embedded().unwrap();
    let mut analytic = runtime
        .load(&compiled.wasm, &compiled.description)
        .unwrap()
        .instantiate_native()
        .unwrap();
    let mut difference = runtime
        .load(&compiled.wasm, &quotient)
        .unwrap()
        .instantiate_native()
        .unwrap();
    let with = analytic.simulate(&request(true)).unwrap();
    let without = difference.simulate(&request(true)).unwrap();

    // the analytic `df/dp` takes no right hand side of a difference quotient at all, and the
    // model without the pattern takes one per parameter and step
    let (with_stats, without_stats) = (analytic.stats(), difference.stats());
    assert!(with_stats.sensitivity_jac_p_evals > 0);
    assert_eq!(with_stats.sensitivity_model_rhs_evals, 0);
    assert_eq!(without_stats.sensitivity_jac_p_evals, 0);
    assert!(without_stats.sensitivity_model_rhs_evals > 0);

    assert_eq!(with.n_rows, without.n_rows);
    for row in 0..with.n_rows {
        for column in ["S", "kel", "g"] {
            for (at, id) in ["k", "w", "q", "S"].iter().enumerate() {
                let (one, other) = (
                    sensitivity(&with, row, column, at),
                    sensitivity(&without, row, column, at),
                );
                assert!(
                    (one - other).abs() < 1e-6 * one.abs().max(other.abs()).max(1e-6),
                    "d {column}/d {id} of the row {row} is {one} of the analytic df/dp and \
                     {other} of the difference quotient"
                );
            }
        }
    }
    // and the sensitivity of `w`, which only the initialization reads, is not nothing
    let of_w = sensitivity(&with, with.n_rows - 1, "S", 1);
    assert!(of_w.abs() > 1e-3, "dS/dw of the last row is {of_w}");
}

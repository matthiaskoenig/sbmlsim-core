//! Request start times and solver work limits on retarded trajectories.

use std::sync::OnceLock;

use sbmlsim_abi::{
    EngineError, IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, Tolerance,
};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::{LoadedModel, Runtime};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const DELAY: &str = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol>"#;
const TIME: &str = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">time</csymbol>"#;

fn load(parameters: &str, rules: &str) -> LoadedModel {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model id="dde"><listOfParameters>{parameters}</listOfParameters><listOfRules>{rules}</listOfRules></model></sbml>"#
    );
    let compiled = compile(&xml, &CompileOptions::default()).unwrap();
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| Runtime::embedded().unwrap())
        .load(&compiled.wasm, &compiled.description)
        .unwrap()
}

fn request(start: f64, end: f64, steps: u32, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_owned(),
        start,
        end,
        output: Output::Uniform { steps },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        seed: 1,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings::default(),
    }
}

#[test]
fn a_request_started_before_zero_reads_its_integrated_negative_time_history() {
    let model = load(
        r#"<parameter id="x" value="0" constant="false"/><parameter id="q" constant="false"/>"#,
        &format!(
            r#"<rateRule variable="x"><math {MATH}><cn>1</cn></math></rateRule>
<assignmentRule variable="q"><math {MATH}><apply>{DELAY}<ci>x</ci><cn>0.25</cn></apply></math></assignmentRule>"#
        ),
    );
    let req = request(-2., 0., 8, &["time", "x", "q"]);
    let verify = |result: sbmlsim_abi::SimulationResult| {
        assert_eq!(result.n_rows, 9);
        for &[t, x, q] in result.values.as_chunks::<3>().0 {
            assert!((x - (t + 2.)).abs() < 1e-8, "t={t} x={x}");
            assert!((q - (t + 1.75).max(0.)).abs() < 1e-8, "t={t} q={q}");
        }
    };
    verify(model.instantiate().unwrap().simulate(&req).unwrap());
    #[cfg(feature = "native")]
    verify(model.instantiate_native().unwrap().simulate(&req).unwrap());
}

#[test]
fn overlapping_history_retries_spend_the_output_intervals_step_budget() {
    let model = load(
        r#"<parameter id="x" value="1" constant="false"/>"#,
        &format!(
            r#"<rateRule variable="x"><math {MATH}><apply>{DELAY}<ci>x</ci><apply><divide/>{TIME}<cn>2</cn></apply></apply></math></rateRule>"#
        ),
    );
    let mut req = request(0., 0.125, 1, &["x"]);
    req.settings.initial_time_step = 0.125;
    req.settings.absolute_tolerance = Tolerance::Scalar(0.1);
    req.settings.relative_tolerance = 1e-4;
    req.settings.maximum_num_steps = 1;
    let verify = |result| {
        assert!(
            matches!(result, Err(EngineError::Integration { flag: -1, .. })),
            "{result:?}"
        )
    };
    verify(model.instantiate().unwrap().simulate(&req));
    #[cfg(feature = "native")]
    verify(model.instantiate_native().unwrap().simulate(&req));
}

#[test]
fn historical_algebraic_state_satisfies_the_constraint_at_query_time() {
    let model = load(
        r#"<parameter id="z" value="0" constant="false"/><parameter id="q" constant="false"/>"#,
        &format!(
            r#"<algebraicRule><math {MATH}><apply><minus/><ci>z</ci><apply><plus/>{TIME}<cn>4</cn></apply></apply></math></algebraicRule>
<assignmentRule variable="q"><math {MATH}><apply>{DELAY}<ci>z</ci><cn>1</cn></apply></math></assignmentRule>"#
        ),
    );
    let req = request(0., 0.5, 2, &["time", "q"]);
    let verify = |result: sbmlsim_abi::SimulationResult| {
        for &[t, q] in result.values.as_chunks::<2>().0 {
            assert!((q - (t + 3.)).abs() < 1e-8, "t={t} q={q}");
        }
    };
    verify(model.instantiate().unwrap().simulate(&req).unwrap());
    #[cfg(feature = "native")]
    verify(model.instantiate_native().unwrap().simulate(&req).unwrap());
}

#[test]
fn historical_assignment_uses_the_coupled_algebraic_solution() {
    let model = load(
        r#"<parameter id="x" value="2000000000000" constant="false"/><parameter id="y" value="2000000000000" constant="false"/>
<parameter id="total" constant="false"/><parameter id="q" constant="false"/>"#,
        &format!(
            r#"<algebraicRule><math {MATH}><apply><times/><cn>0.00000000000001</cn><apply><minus/><apply><times/><apply><divide/><ci>x</ci><cn>1000000000000</cn></apply><apply><power/><apply><divide/><ci>y</ci><cn>1000000000000</cn></apply><cn>2</cn></apply></apply><apply><plus/>{TIME}<cn>8</cn></apply></apply></apply></math></algebraicRule>
<algebraicRule><math {MATH}><apply><minus/><ci>x</ci><ci>y</ci></apply></math></algebraicRule>
<assignmentRule variable="total"><math {MATH}><apply><plus/><ci>x</ci><ci>y</ci></apply></math></assignmentRule>
<assignmentRule variable="q"><math {MATH}><apply>{DELAY}<ci>total</ci><cn>1</cn></apply></math></assignmentRule>"#
        ),
    );
    let req = request(0., 0.5, 2, &["time", "q"]);
    let verify = |result: sbmlsim_abi::SimulationResult| {
        for &[t, q] in result.values.as_chunks::<2>().0 {
            assert!((q / 2e12 - (7. + t).cbrt()).abs() < 1e-10, "t={t} q={q}");
        }
    };
    verify(model.instantiate().unwrap().simulate(&req).unwrap());
    #[cfg(feature = "native")]
    verify(model.instantiate_native().unwrap().simulate(&req).unwrap());
}

#[test]
fn infeasible_historical_algebraic_constraint_reports_a_history_error() {
    let model = load(
        r#"<parameter id="x" value="1" constant="false"/><parameter id="q" constant="false"/>"#,
        &format!(
            r#"<algebraicRule><math {MATH}><apply><minus/><apply><times/><ci>x</ci><ci>x</ci></apply><apply><plus/>{TIME}<cn>1</cn></apply></apply></math></algebraicRule>
<assignmentRule variable="q"><math {MATH}><apply>{DELAY}<ci>x</ci><cn>2</cn></apply></math></assignmentRule>"#
        ),
    );
    let req = request(0., 0.5, 2, &["q"]);
    let verify = |result| {
        assert!(
            matches!(result, Err(EngineError::Integration { flag: -1006, .. })),
            "{result:?}"
        );
    };
    verify(model.instantiate().unwrap().simulate(&req));
    #[cfg(feature = "native")]
    verify(model.instantiate_native().unwrap().simulate(&req));
}

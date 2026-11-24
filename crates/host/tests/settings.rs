//! The settings of CVODE and of IDA which the request carries, and the statistics of the
//! integrator which a simulation leaves behind.

use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::{Instance, Runtime};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

/// Robertson's problem, the stiff chemical kinetics of three states over many decades, as rate
/// rules of three parameters.
fn robertson() -> String {
    let rules = [
        (
            "y1",
            "<apply><plus/><apply><times/><cn>-0.04</cn><ci>y1</ci></apply><apply><times/><cn type=\"e-notation\">1<sep/>4</cn><ci>y2</ci><ci>y3</ci></apply></apply>",
        ),
        (
            "y2",
            "<apply><minus/><apply><minus/><apply><times/><cn>0.04</cn><ci>y1</ci></apply><apply><times/><cn type=\"e-notation\">1<sep/>4</cn><ci>y2</ci><ci>y3</ci></apply></apply><apply><times/><cn type=\"e-notation\">3<sep/>7</cn><ci>y2</ci><ci>y2</ci></apply></apply>",
        ),
        (
            "y3",
            "<apply><times/><cn type=\"e-notation\">3<sep/>7</cn><ci>y2</ci><ci>y2</ci></apply>",
        ),
    ];
    let rules: String = rules
        .iter()
        .map(|(id, math)| {
            format!(r#"<rateRule variable="{id}"><math {MATH}>{math}</math></rateRule>"#)
        })
        .collect();
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="robertson"><listOfParameters><parameter id="y1" value="1" constant="false"/><parameter id="y2" value="0" constant="false"/><parameter id="y3" value="0" constant="false"/></listOfParameters><listOfRules>{rules}</listOfRules></model></sbml>"#
    )
}

/// An exponential decay, which the Adams method of CVODE integrates.
fn decay() -> String {
    let math = "<apply><times/><cn>-1</cn><ci>y</ci></apply>";
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="decay"><listOfParameters><parameter id="y" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="y"><math {MATH}>{math}</math></rateRule></listOfRules></model></sbml>"#
    )
}

/// A differential algebraic model: `y` decays and `a` is `2 y` through an algebraic rule.
fn dae() -> String {
    let rate = "<apply><times/><cn>-1</cn><ci>y</ci></apply>";
    let algebraic = "<apply><minus/><ci>a</ci><apply><times/><cn>2</cn><ci>y</ci></apply></apply>";
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="dae"><listOfParameters><parameter id="y" value="1" constant="false"/><parameter id="a" value="2" constant="false"/></listOfParameters><listOfRules><rateRule variable="y"><math {MATH}>{rate}</math></rateRule><algebraicRule><math {MATH}>{algebraic}</math></algebraicRule></listOfRules></model></sbml>"#
    )
}

fn request(end: f64, steps: u32, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output: Output::Uniform { steps },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings::default(),
        seed: 0,
    }
}

fn loaded(sbml: &str) -> Instance {
    let compiled = compile(sbml, &CompileOptions::default()).unwrap();
    runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
}

#[test]
fn the_maximum_bdf_order_bounds_the_order_of_the_steps() {
    let mut instance = loaded(&robertson());
    let req = request(40.0, 10, &["time", "y1", "y2", "y3"]);
    let free = instance.simulate(&req).unwrap();
    let free_stats = instance.stats().unwrap();
    let mut capped = req.clone();
    capped.settings.maximum_bdf_order = 2;
    let bounded = instance.simulate(&capped).unwrap();
    let stats = instance.stats().unwrap();
    assert!(
        free_stats.last_order > 2,
        "the BDF of CVODE takes an order above 2 for this model: {free_stats:?}"
    );
    assert!(
        stats.last_order <= 2 && stats.current_order <= 2,
        "the capped run stays at the order 2: {stats:?}"
    );
    assert!(
        stats.steps > free_stats.steps,
        "the order of 2 costs steps: {} against {}",
        stats.steps,
        free_stats.steps
    );
    // the same solution, which is the point of a second method of a lower order
    for (a, b) in bounded.values.iter().zip(&free.values) {
        assert!((a - b).abs() <= 1e-4 * b.abs().max(1e-6), "{a} against {b}");
    }
}

#[test]
fn the_maximum_adams_order_bounds_a_run_which_is_not_stiff() {
    let mut instance = loaded(&decay());
    let mut req = request(20.0, 10, &["time", "y"]);
    req.settings.stiff = false;
    req.settings.relative_tolerance = 1e-10;
    let free = instance.simulate(&req).unwrap();
    let free_stats = instance.stats().unwrap();
    let mut capped = req.clone();
    capped.settings.maximum_adams_order = 2;
    instance.simulate(&capped).unwrap();
    let stats = instance.stats().unwrap();
    assert!(free_stats.last_order > 2, "{free_stats:?}");
    assert!(stats.last_order <= 2, "{stats:?}");
    assert!(stats.steps > free_stats.steps, "{stats:?} {free_stats:?}");
    assert_eq!(free.columns.len(), 2);
}

#[test]
fn the_statistics_carry_the_state_of_the_solver() {
    let mut instance = loaded(&robertson());
    let req = request(40.0, 10, &["time", "y1"]);
    instance.simulate(&req).unwrap();
    let stats = instance.stats().unwrap();
    assert!(stats.last_order >= 1, "{stats:?}");
    assert!(stats.actual_initial_time_step > 0.0, "{stats:?}");
    assert!(stats.last_time_step > 0.0, "{stats:?}");
    assert!(stats.current_time_step > 0.0, "{stats:?}");
    // the integrator is at or past the end of the simulation, whose last values it interpolates
    assert!(stats.current_time >= 40.0, "{stats:?}");
    assert_eq!(stats.stability_order_reductions, 0, "{stats:?}");
}

#[test]
fn the_maximum_time_step_bounds_the_steps_of_the_statistics() {
    let mut instance = loaded(&robertson());
    let mut req = request(40.0, 10, &["time", "y1"]);
    req.settings.maximum_time_step = 0.5;
    instance.simulate(&req).unwrap();
    let stats = instance.stats().unwrap();
    assert!(stats.last_time_step <= 0.5 + 1e-12, "{stats:?}");
    assert!(stats.steps >= 80, "{stats:?}");
}

#[test]
fn the_detection_of_the_stability_limit_runs_and_counts() {
    let mut instance = loaded(&robertson());
    let mut req = request(40.0, 10, &["time", "y1", "y2", "y3"]);
    let plain = instance.simulate(&req).unwrap();
    req.settings.stability_limit_detection = true;
    let detected = instance.simulate(&req).unwrap();
    let stats = instance.stats().unwrap();
    // the dominant eigenvalues of Robertson's problem are real: the detection finds nothing
    // and the counter reads the reductions it made
    assert_eq!(stats.stability_order_reductions, 0, "{stats:?}");
    assert_eq!(detected.values, plain.values);
}

#[test]
fn the_error_test_failures_of_one_step_are_bounded() {
    let mut instance = loaded(&robertson());
    let mut req = request(40.0, 10, &["time", "y1"]);
    // a first step far above the one this model takes, which the error test rejects again and
    // again until the integrator gives up
    req.settings.initial_time_step = 10.0;
    req.settings.maximum_error_test_failures = 1;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(
        error.contains("error test") || error.contains("CV_ERR_FAILURE"),
        "{error}"
    );
    // the same step with more failures of the error test allowed than CVODE's own 7
    req.settings.maximum_error_test_failures = 50;
    instance.simulate(&req).unwrap();
}

#[test]
fn the_settings_of_the_nonlinear_solver_are_taken() {
    let mut instance = loaded(&robertson());
    let mut req = request(40.0, 10, &["time", "y1", "y2", "y3"]);
    let plain = instance.simulate(&req).unwrap();
    req.settings.maximum_nonlinear_iterations = 10;
    req.settings.maximum_convergence_failures = 20;
    req.settings.nonlinear_convergence_coefficient = 0.01;
    let strict = instance.simulate(&req).unwrap();
    let stats = instance.stats().unwrap();
    assert!(stats.nonlinear_iterations > 0, "{stats:?}");
    for (a, b) in strict.values.iter().zip(&plain.values) {
        assert!((a - b).abs() <= 1e-4 * b.abs().max(1e-6), "{a} against {b}");
    }
}

#[test]
fn the_minimum_time_step_is_refused_above_the_maximum() {
    let mut instance = loaded(&decay());
    let mut req = request(10.0, 10, &["time", "y"]);
    req.settings.minimum_time_step = 1.0;
    req.settings.maximum_time_step = 0.5;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("minimum_time_step"), "{error}");
    // alone it is taken
    req.settings.maximum_time_step = 0.0;
    req.settings.minimum_time_step = 1e-8;
    instance.simulate(&req).unwrap();
}

#[test]
fn an_order_above_the_method_is_refused() {
    let mut instance = loaded(&decay());
    let mut req = request(10.0, 10, &["time", "y"]);
    req.settings.maximum_bdf_order = 6;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("maximum_bdf_order"), "{error}");
    req.settings.maximum_bdf_order = 0;
    req.settings.maximum_adams_order = 13;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("maximum_adams_order"), "{error}");
}

#[test]
fn the_detection_of_the_stability_limit_is_refused_without_the_bdf_of_cvode() {
    let mut instance = loaded(&decay());
    let mut req = request(10.0, 10, &["time", "y"]);
    req.settings.stiff = false;
    req.settings.stability_limit_detection = true;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("Adams"), "{error}");

    let mut instance = loaded(&dae());
    let mut req = request(10.0, 10, &["time", "y", "a"]);
    req.settings.stability_limit_detection = true;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("IDA"), "{error}");
}

#[test]
fn the_suppression_of_the_algebraic_states_is_of_ida_alone() {
    let mut instance = loaded(&decay());
    let mut req = request(10.0, 10, &["time", "y"]);
    req.settings.suppress_algebraic = true;
    let error = instance.simulate(&req).unwrap_err().to_string();
    assert!(error.contains("suppress_algebraic"), "{error}");

    let mut instance = loaded(&dae());
    let mut req = request(10.0, 5, &["time", "y", "a"]);
    let plain = instance.simulate(&req).unwrap();
    req.settings.suppress_algebraic = true;
    let suppressed = instance.simulate(&req).unwrap();
    // the algebraic state is `2 y` either way, the error test it is left out of or not
    for (a, b) in suppressed.values.iter().zip(&plain.values) {
        assert!((a - b).abs() <= 1e-4 * b.abs().max(1e-6), "{a} against {b}");
    }
    let stats = instance.stats().unwrap();
    assert!(stats.steps > 0, "{stats:?}");
}

#[test]
fn the_order_of_ida_is_bounded_as_well() {
    let mut instance = loaded(&dae());
    let req = request(10.0, 5, &["time", "y", "a"]);
    instance.simulate(&req).unwrap();
    let free = instance.stats().unwrap();
    let mut capped = req.clone();
    capped.settings.maximum_bdf_order = 1;
    instance.simulate(&capped).unwrap();
    let stats = instance.stats().unwrap();
    assert!(free.last_order > 1, "{free:?}");
    assert_eq!(stats.last_order, 1, "{stats:?}");
    assert!(stats.steps > free.steps, "{stats:?} {free:?}");
}

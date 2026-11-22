//! Events and discontinuities which chatter end in bounded time: the bouncing ball, a Zeno
//! model whose bounces accumulate at t = 1.3416, ends with `CV_TOO_MUCH_WORK` in both outputs,
//! a sliding mode in the steps of the integrator likewise, and well separated doses keep
//! starting the budget of steps again.

use std::sync::mpsc;
use std::time::Duration;

use sbmlsim_abi::EngineError;
use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SimulationStats, Tolerance,
};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_host::Runtime;

/// CVODE's flag of too many steps.
const CV_TOO_MUCH_WORK: i32 = -1;

/// The longest a bounded run may take (the bounded runs take well below a second).
const BOUND: Duration = Duration::from_secs(60);

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// The bouncing ball: `x' = v`, `v' = -10`, `x(0) = 1`, and the event `x < 0`: `v = -0.5 v`,
/// `x = 0`. The bounces accumulate at `t = 3 sqrt(1/5) = 1.3416`.
fn ball() -> CompiledModel {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="ball">
<listOfParameters><parameter id="x" value="1" constant="false"/><parameter id="v" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><ci>v</ci></math></rateRule><rateRule variable="v"><math {MATH}><cn>-10</cn></math></rateRule></listOfRules>
<listOfEvents><event id="E" useValuesFromTriggerTime="true"><trigger initialValue="true" persistent="true"><math {MATH}><apply><lt/><ci>x</ci><cn>0</cn></apply></math></trigger><listOfEventAssignments><eventAssignment variable="v"><math {MATH}><apply><times/><cn>-0.5</cn><ci>v</ci></apply></math></eventAssignment><eventAssignment variable="x"><math {MATH}><cn>0</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#
    );
    compile(&xml, &CompileOptions::default()).unwrap()
}

/// The sliding mode `x' = piecewise(-1, x > 0, 1)`, `x(0) = 1`: `x = 1 - t` up to 1, then the
/// condition chatters at noise level.
fn sliding() -> CompiledModel {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="sliding">
<listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><piecewise><piece><cn>-1</cn><apply><gt/><ci>x</ci><cn>0</cn></apply></piece><otherwise><cn>1</cn></otherwise></piecewise></math></rateRule></listOfRules>
</model></sbml>"#
    );
    compile(&xml, &CompileOptions::default()).unwrap()
}

/// The event model of the benchmark: an oral dose every 8 h for a week.
fn repeated_dose() -> CompiledModel {
    let xml = include_str!("../../../bench/models/repeated_dose.xml");
    compile(xml, &CompileOptions::default()).unwrap()
}

fn request(end: f64, output: Output, rtol: f64, atol: f64) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: vec![],
        settings: IntegratorSettings {
            relative_tolerance: rtol,
            absolute_tolerance: Tolerance::Scalar(atol),
            ..IntegratorSettings::default()
        },
        seed: 1,
    }
}

type Outcome = (Result<SimulationResult, EngineError>, SimulationStats);

/// Runs `req` on a new instance in a thread of its own and waits at most [`BOUND`] for it
/// (a run which does not end fails the test instead of hanging it).
fn run_bounded(compiled: &CompiledModel, req: &SimulationRequest) -> Outcome {
    let (compiled, req) = (compiled.clone(), req.clone());
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = Runtime::embedded().unwrap();
        let mut instance = runtime
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap();
        let result = instance.simulate(&req);
        let _ = send.send((result, instance.stats().unwrap()));
    });
    receive
        .recv_timeout(BOUND)
        .unwrap_or_else(|_| panic!("the run did not end within {BOUND:?}"))
}

/// Asserts that the outcome is CVODE's error of too many steps after `after`.
fn assert_too_much_work(outcome: &Outcome, after: f64, before: f64) {
    match &outcome.0 {
        Err(EngineError::Integration { flag, time, .. }) => {
            assert_eq!(*flag, CV_TOO_MUCH_WORK, "{:?}", outcome.0.as_ref().err());
            assert!(*time > after && *time < before, "{time}");
        }
        Err(other) => panic!("{other:?}"),
        Ok(result) => panic!("a result of {} rows", result.n_rows),
    }
}

#[test]
fn the_bouncing_ball_ends_with_too_much_work_in_an_output_of_times() {
    let compiled = ball();
    // up to 1.34 the bounces are well separated: a result
    let req = request(1.34, Output::Uniform { steps: 10 }, 1e-8, 1e-10);
    let (result, _) = run_bounded(&compiled, &req);
    result.unwrap();
    // past the accumulation point: the executions close to the last applied root do not start
    // the budget of steps again
    for (end, steps) in [(1.345, 1), (5.0, 10)] {
        let req = request(end, Output::Uniform { steps }, 1e-8, 1e-10);
        let outcome = run_bounded(&compiled, &req);
        assert_too_much_work(&outcome, 1.3416, 1.3417);
        // measured 20259 steps in every run, the bound is about twice that
        assert!(outcome.1.steps < 40_000, "{:?}", outcome.1);
    }
}

#[test]
fn the_bouncing_ball_ends_with_too_much_work_in_the_steps_of_the_integrator() {
    let compiled = ball();
    for end in [1.345, 5.0] {
        let req = request(end, Output::IntegratorSteps, 1e-8, 1e-10);
        let outcome = run_bounded(&compiled, &req);
        assert_too_much_work(&outcome, 1.3416, 1.3417);
        // measured 20259 steps in every run, the bound is about twice that
        assert!(outcome.1.steps < 40_000, "{:?}", outcome.1);
    }
}

#[test]
fn a_sliding_mode_ends_with_too_much_work_in_the_steps_of_the_integrator() {
    // rtol 1e-6, atol 1e-12 up to 3: CVODE never accepts a step across 0 and creeps up to it
    // in ever smaller steps (in the output of times CV_TOO_MUCH_WORK at 1.00000000157277, as in
    // roadrunner; before the bound of chatter the steps of the integrator ran to the limit of
    // the output, 33.5 million rows)
    let req = request(3.0, Output::IntegratorSteps, 1e-6, 1e-12);
    let outcome = run_bounded(&sliding(), &req);
    assert_too_much_work(&outcome, 1.0, 1.00000001);
    // measured 20025 steps, the bound is about twice that
    assert!(outcome.1.steps < 40_000, "{:?}", outcome.1);
}

#[test]
fn well_separated_doses_start_the_budget_of_steps_again() {
    // one output interval over the week, a budget below the steps of the whole run but above
    // the steps between two doses: every dose starts the budget again
    const MAX_STEPS: u32 = 1000;
    let compiled = repeated_dose();
    let mut req = request(168.0, Output::Uniform { steps: 1 }, 1e-6, 1e-12);
    req.settings.maximum_num_steps = MAX_STEPS;
    let (result, stats) = run_bounded(&compiled, &req);
    let result = result.unwrap();
    assert_eq!(result.n_rows, 2);
    assert!(stats.steps > 3 * u64::from(MAX_STEPS), "{stats:?}");
    // the steps of the integrator: the doses are far apart, no chatter
    let req = request(168.0, Output::IntegratorSteps, 1e-6, 1e-12);
    let (result, _) = run_bounded(&compiled, &req);
    assert!(result.unwrap().n_rows > 100);
}

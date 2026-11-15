//! The root finding of the conditions of `piecewise`: CVODE starts again at the zero of a root
//! function of a discontinuity, compiled models in the chain mathlib -> model -> engine.

use sbmlsim_abi::{
    IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SimulationStats, Tolerance,
};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_host::Runtime;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const TIME: &str =
    r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

/// A model of the state `x` (the rate rule `rate`), the species `S` (an amount, decaying with
/// the rate 1), the parameter `k = 3`, the parameter `n` and `rest` after the rules.
fn model(rate: &str, rest: &str) -> CompiledModel {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="d">
<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="C" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="x" value="0" constant="false"/><parameter id="k" value="3" constant="true"/><parameter id="n" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}>{rate}</math></rateRule></listOfRules>
<listOfReactions><reaction id="J" reversible="false"><listOfReactants><speciesReference species="S" stoichiometry="1" constant="true"/></listOfReactants><kineticLaw><math {MATH}><ci>S</ci></math></kineticLaw></reaction></listOfReactions>
{rest}</model></sbml>"#
    );
    compile(&xml, &CompileOptions::default()).unwrap()
}

fn ap(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

fn cn(x: &str) -> String {
    format!("<cn>{x}</cn>")
}

/// `piecewise(1, c, 0)`.
fn step(c: &str) -> String {
    format!("<piecewise><piece><cn>1</cn>{c}</piece><otherwise><cn>0</cn></otherwise></piecewise>")
}

fn request(end: f64, output: Output) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output,
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: vec!["time".to_string(), "x".to_string(), "n".to_string()],
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: Tolerance::Scalar(1e-14),
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The result and the counters of a request on a new instance.
fn run(compiled: &CompiledModel, req: &SimulationRequest) -> (SimulationResult, SimulationStats) {
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(req).unwrap();
    (result, instance.stats().unwrap())
}

/// The values of column `name`.
fn column(result: &SimulationResult, name: &str) -> Vec<f64> {
    let k = result.columns.len();
    let j = result.columns.iter().position(|c| c == name).unwrap();
    (0..result.n_rows)
        .map(|i| result.values[i * k + j])
        .collect()
}

/// The rows `i` whose time is the one of the row `i + 1`.
fn pairs(times: &[f64]) -> Vec<usize> {
    (0..times.len() - 1)
        .filter(|&i| times[i] == times[i + 1])
        .collect()
}

fn bits(result: &SimulationResult) -> Vec<u64> {
    result.values.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn a_step_of_the_time_in_a_rate_rule_starts_the_integrator_again() {
    // x' = piecewise(1, time > 1.5, 0): x = 0 up to 1.5, then t - 1.5
    let compiled = model(&step(&ap("gt", &[TIME, &cn("1.5")])), "");
    assert_eq!(compiled.description.n_roots, 1);
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    let (result, stats) = run(&compiled, &request(3.0, Output::Uniform { steps: 4 }));
    // the start and the start again at the root
    assert_eq!(stats.integrations, 2, "{stats:?}");
    assert!(stats.root_evals > 0);
    // both pieces are linear, which BDF integrates exactly: the error is the location of the
    // root, which CVODE finds within a hundred units of rounding of the time (and the output at
    // 1.5 is interpolated in the step which crosses the root)
    let x = column(&result, "x");
    assert_eq!(x[..2], [0.0, 0.0]);
    for (x, expected) in x[2..].iter().zip([0.0, 0.75, 1.5]) {
        assert!((x - expected).abs() < 1e-13, "{x} {expected}");
    }
    // the steps of the integrator: two rows of the state at the root, as libroadrunner
    let (steps, _) = run(&compiled, &request(3.0, Output::IntegratorSteps));
    let (times, x) = (column(&steps, "time"), column(&steps, "x"));
    let pairs = pairs(&times);
    assert_eq!(pairs.len(), 1, "{times:?}");
    let i = pairs[0];
    assert!((times[i] - 1.5).abs() < 1e-13, "{}", times[i]);
    assert_eq!(x[i], x[i + 1]);
    assert!(x[i].abs() < 1e-13, "{}", x[i]);
    assert!((x.last().unwrap() - 1.5).abs() < 1e-13);
}

#[test]
fn a_condition_on_a_species() {
    // x' = piecewise(1, S < 0.5, 0) with S = exp(-t): the root at ln 2
    let compiled = model(&step(&ap("lt", &["<ci>S</ci>", &cn("0.5")])), "");
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    let (result, stats) = run(&compiled, &request(2.0, Output::Uniform { steps: 1 }));
    assert_eq!(stats.integrations, 2, "{stats:?}");
    let x = column(&result, "x")[1];
    let expected = 2.0 - 2f64.ln();
    assert!((x - expected).abs() < 1e-9, "{x} {expected}");
}

#[test]
fn a_constant_condition_gives_no_root() {
    // k > 2 and 1 > 2 change only where the integration stops anyway
    let rate = ap(
        "plus",
        &[
            &step(&ap("gt", &["<ci>k</ci>", &cn("2")])),
            &step(&ap("gt", &[&cn("1"), &cn("2")])),
        ],
    );
    let compiled = model(&rate, "");
    assert_eq!(compiled.description.n_roots, 0);
    assert_eq!(compiled.description.n_discontinuity_roots, 0);
    let (result, stats) = run(&compiled, &request(2.0, Output::Uniform { steps: 1 }));
    assert_eq!(stats.integrations, 1);
    assert_eq!(stats.root_evals, 0);
    assert!((column(&result, "x")[1] - 2.0).abs() < 1e-12);
}

#[test]
fn a_boolean_combination_is_one_root_function() {
    // x' = piecewise(1, 1 < time < 2 and S > 0, 0): the roots at 1 and 2 (S > 0 does not change)
    let c = ap(
        "and",
        &[
            &ap("lt", &[&cn("1"), TIME, &cn("2")]),
            &ap("gt", &["<ci>S</ci>", &cn("0")]),
        ],
    );
    let compiled = model(&step(&c), "");
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    let (result, stats) = run(&compiled, &request(3.0, Output::Uniform { steps: 3 }));
    assert_eq!(stats.integrations, 3, "{stats:?}");
    // the outputs at the roots are interpolated in the steps which cross them, within the
    // tolerances
    let x = column(&result, "x");
    assert_eq!(x[0], 0.0);
    for (x, expected) in x[1..].iter().zip([0.0, 1.0, 1.0]) {
        assert!((x - expected).abs() < 1e-9, "{x} {expected}");
    }
    assert_eq!(x[2], x[3], "the rate is 0 after 2");
}

/// An event at `time > 1` which counts in `n`.
fn event_at_one() -> String {
    format!(
        r#"<listOfEvents><event id="E" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}>{}</math></trigger><listOfEventAssignments><eventAssignment variable="n"><math {MATH}>{}</math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        ap("gt", &[TIME, &cn("1")]),
        ap("plus", &["<ci>n</ci>", &cn("1")])
    )
}

#[test]
fn a_discontinuity_at_the_time_of_an_event() {
    // the event at time > 1 and x' = piecewise(1, time > 1, 0): one stop, one start again
    let compiled = model(&step(&ap("gt", &[TIME, &cn("1")])), &event_at_one());
    assert_eq!(compiled.description.n_roots, 2);
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    // (an absolute tolerance of 1e-14 fails the error test of CVODE in the steps of the
    // integrator, as before the root finding of the discontinuities: the event executes at a
    // step right before 1, within the probe of its trigger, and CVODE starts again from there
    // into the jump of the rate at 1)
    let request = |end: f64, output: Output| {
        let mut req = request(end, output);
        req.settings.absolute_tolerance = Tolerance::Scalar(1e-12);
        req
    };
    let (result, stats) = run(&compiled, &request(2.0, Output::Uniform { steps: 2 }));
    assert_eq!(stats.integrations, 2, "{stats:?}");
    assert_eq!(column(&result, "n"), [0.0, 0.0, 1.0]);
    let x = column(&result, "x");
    assert!((x[2] - 1.0).abs() < 1e-11, "{x:?}");
    // the steps: the two rows of the event, no more
    let (steps, _) = run(&compiled, &request(2.0, Output::IntegratorSteps));
    let (times, n) = (column(&steps, "time"), column(&steps, "n"));
    let pairs = pairs(&times);
    assert_eq!(pairs.len(), 1, "{times:?}");
    let i = pairs[0];
    assert!((times[i] - 1.0).abs() < 1e-11, "{}", times[i]);
    assert_eq!((n[i], n[i + 1]), (0.0, 1.0));
}

/// Two events of equal priority at the time 2, which set `n` to 1 and to 2; the seed draws
/// their order.
fn tie_at_two() -> String {
    let event = |id: &str, value: &str| {
        format!(
            r#"<event id="{id}" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}>{}</math></trigger><priority><math {MATH}><cn>1</cn></math></priority><listOfEventAssignments><eventAssignment variable="n"><math {MATH}><cn>{value}</cn></math></eventAssignment></listOfEventAssignments></event>"#,
            ap("geq", &[TIME, &cn("2")])
        )
    };
    format!(
        "<listOfEvents>{}{}</listOfEvents>",
        event("A", "1"),
        event("B", "2")
    )
}

#[test]
fn a_discontinuity_draws_nothing_from_the_order_of_ties() {
    // the same ties with and without the discontinuities at 0.5 and 1.5 before them: every seed
    // gives the same order
    let plain = model(&cn("1"), &tie_at_two());
    let c = ap(
        "or",
        &[
            &ap("lt", &[TIME, &cn("0.5")]),
            &ap("gt", &[TIME, &cn("1.5")]),
        ],
    );
    let stepped = model(&step(&c), &tie_at_two());
    assert_eq!(stepped.description.n_discontinuity_roots, 1);
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..16 {
        let mut req = request(3.0, Output::Uniform { steps: 3 });
        req.seed = seed;
        let (a, _) = run(&plain, &req);
        let (b, stats) = run(&stepped, &req);
        assert!(stats.integrations >= 3, "{stats:?}");
        let n = column(&a, "n")[3];
        assert_eq!(n, column(&b, "n")[3], "seed {seed}");
        seen.insert(n.to_bits());
    }
    assert_eq!(seen.len(), 2, "both orders occur");
}

#[test]
fn requests_on_one_instance_give_the_bits_of_a_fresh_instance() {
    let compiled = model(&step(&ap("gt", &[TIME, &cn("1.5")])), &event_at_one());
    let a = request(3.0, Output::Uniform { steps: 6 });
    let mut b = request(4.0, Output::IntegratorSteps);
    b.preinit = vec![("k".to_string(), 5.0)];
    b.changes = vec![sbmlsim_abi::Change {
        time: 2.5,
        values: vec![("x".to_string(), 1.0)],
    }];
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let a1 = instance.simulate(&a).unwrap();
    let b1 = instance.simulate(&b).unwrap();
    let a2 = instance.simulate(&a).unwrap();
    let b2 = instance.simulate(&b).unwrap();
    assert_eq!(bits(&a1), bits(&a2));
    assert_eq!(bits(&a1), bits(&run(&compiled, &a).0));
    assert_eq!(bits(&b1), bits(&b2));
    assert_eq!(bits(&b1), bits(&run(&compiled, &b).0));
}

#[test]
fn the_steady_state_starts_the_integrator_again_at_a_discontinuity() {
    // x' = piecewise(1, time < 0.5, -x): x rises to 0.5, then decays to its steady state 0
    let rate = format!(
        "<piecewise><piece><cn>1</cn>{}</piece><otherwise><apply><minus/><ci>x</ci></apply></otherwise></piecewise>",
        ap("lt", &[TIME, &cn("0.5")])
    );
    let compiled = model(&rate, "");
    let mut req = request(1.0, Output::Uniform { steps: 1 });
    req.steady_state = Some(sbmlsim_abi::SteadyState::default());
    let (result, stats) = run(&compiled, &req);
    // the steady state: a start per horizon and the start again at 0.5, then the simulation
    // and its start again at 0.5; `x' = -x` from the value which gives the same x from 0.5 on
    // reaches the steady state at the same horizon without the starts again
    let (_, plain) = run(
        &model(&ap("minus", &["<ci>x</ci>"]), ""),
        &SimulationRequest {
            preinit: vec![("x".to_string(), 0.5 * 0.5f64.exp())],
            ..req.clone()
        },
    );
    assert_eq!(
        stats.integrations,
        plain.integrations + 2,
        "{stats:?} {plain:?}"
    );
    let x = column(&result, "x");
    assert!(x[0].abs() < 1e-6, "{x:?}");
}

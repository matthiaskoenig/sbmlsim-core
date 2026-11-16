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
    let (result, stats) = run(&compiled, &request(2.0, Output::Uniform { steps: 2 }));
    assert_eq!(stats.integrations, 2, "{stats:?}");
    assert_eq!(column(&result, "n"), [0.0, 0.0, 1.0]);
    let x = column(&result, "x");
    assert!((x[2] - 1.0).abs() < 1e-11, "{x:?}");
    // the steps: the event executes once, at its root at 1 (its two rows; a step evaluates the
    // triggers at its time only), and the discontinuity has its two rows of one
    // state there or right after it. At atol 1e-14 CVODE failed with the flag -3 when the event
    // executed at a step within the probe of its trigger, 4.6e-13 before 1, and CVODE started
    // again from there into the jump of the rate at 1
    let (steps, _) = run(&compiled, &request(2.0, Output::IntegratorSteps));
    let (times, n, x) = (
        column(&steps, "time"),
        column(&steps, "n"),
        column(&steps, "x"),
    );
    let pairs = pairs(&times);
    assert!(!pairs.is_empty() && pairs.len() <= 2, "{times:?}");
    let event: Vec<usize> = pairs
        .iter()
        .copied()
        .filter(|&i| n[i] != n[i + 1])
        .collect();
    assert_eq!(event.len(), 1, "{times:?}");
    assert_eq!((n[event[0]], n[event[0] + 1]), (0.0, 1.0));
    for i in pairs {
        assert!((times[i] - 1.0).abs() < 1e-11, "{}", times[i]);
        assert_eq!(x[i], x[i + 1]);
    }
    assert_eq!(*n.last().unwrap(), 1.0);
    assert!((x.last().unwrap() - 1.0).abs() < 1e-11);
}

#[test]
fn an_event_at_its_threshold_at_a_root_executes_once_at_a_tight_tolerance() {
    // the root of time > 1 is located exactly at 1, where the strict trigger is false; CVODE,
    // started again there with a first step below the rounding of the time, returns the zero at
    // 1 again, which was a fall and a rise of the trigger in the probe each time (the event
    // executed 1000 times, EVENT_LIMIT_FLAG)
    for discontinuity in [true, false] {
        let rate = if discontinuity {
            step(&ap("gt", &[TIME, &cn("1")]))
        } else {
            cn("0")
        };
        let compiled = model(&rate, &event_at_one());
        for output in [Output::Uniform { steps: 2 }, Output::IntegratorSteps] {
            let mut req = request(2.0, output.clone());
            req.settings.absolute_tolerance = Tolerance::Scalar(1e-16);
            let (result, stats) = run(&compiled, &req);
            let n = column(&result, "n");
            assert_eq!(*n.last().unwrap(), 1.0, "{output:?} {discontinuity}");
            assert!(
                stats.integrations <= 3,
                "{output:?} {discontinuity} {stats:?}"
            );
        }
    }
}

/// The model of the sliding mode `x' = piecewise(-1, x > 0, 1)`, `x(0) = 1`: `x = 1 - t` up to
/// 1, then 0 (the condition chatters at noise level).
fn sliding() -> CompiledModel {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="sliding">
<listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><piecewise><piece><cn>-1</cn>{}</piece><otherwise><cn>1</cn></otherwise></piecewise></math></rateRule></listOfRules>
</model></sbml>"#,
        ap("gt", &["<ci>x</ci>", &cn("0")])
    );
    compile(&xml, &CompileOptions::default()).unwrap()
}

#[test]
fn a_sliding_mode_ends_as_in_roadrunner() {
    let compiled = sliding();
    let sliding_request = |end: f64, rtol: f64, atol: f64| {
        let mut req = request(end, Output::Uniform { steps: 30 });
        req.selections = vec!["time".to_string(), "x".to_string()];
        req.settings.relative_tolerance = rtol;
        req.settings.absolute_tolerance = Tolerance::Scalar(atol);
        req
    };
    // rtol 1e-6, atol 1e-12: CVODE never accepts a step across 0 and creeps up to it; roadrunner
    // 2.12.0.dev683 fails with CV_TOO_MUCH_WORK at 1.00000000157277, the engine likewise
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    match instance.simulate(&sliding_request(3.0, 1e-6, 1e-12)) {
        Err(sbmlsim_abi::EngineError::Integration { flag, time, .. }) => {
            assert_eq!(flag, -1);
            assert!((time - 1.00000000157277).abs() < 1e-11, "{time}");
        }
        other => panic!("{other:?}"),
    }
    // rtol 1e-3, atol 1e-4 up to 2: the condition chatters (the engine starts again at about
    // 3000 roots, its count of roots close together ignoring others; roadrunner restarts at
    // none of them, its trajectory chatters otherwise); both are within the absolute tolerance
    // of the exact solution at every output time (roadrunner 6.7e-6, the engine 5.5e-6) and of
    // each other (1.1e-5), in a bounded number of steps
    let (result, stats) = run(&compiled, &sliding_request(2.0, 1e-3, 1e-4));
    assert!(stats.integrations > 100, "{stats:?}");
    assert!(stats.steps <= 30 * 20000, "{stats:?}");
    for (t, x) in column(&result, "time").iter().zip(column(&result, "x")) {
        let exact = (1.0 - t).max(0.0);
        assert!((x - exact).abs() < 1e-4, "{t} {x}");
    }
}

#[test]
fn a_guard_at_noise_level_agrees_with_roadrunner() {
    // the rate piecewise(0, S < 0, k * S) of a decay, whose state falls below 0 within the
    // absolute tolerance once: roadrunner and the engine restart CVODE once (roadrunner
    // 2.12.0.dev683: 1 restart, dense and KLU). Against the solution exp(-t), in units of the
    // criterion of the agreement, 1e-12 + 1e-7 |c|, roadrunner at rtol 1e-10 and atol 1e-12 has
    // the largest excess 1.67732 (dense) and 1.67745 (KLU) at the output 11.67; the engine
    // passes the agreement (at most 1 % above roadrunner's excess); in the steps of the integrator
    // (roadrunner dense: 464 rows, largest excess 1.90231) likewise
    const ROADRUNNER_EXCESS: f64 = 1.6773194722192153;
    const ROADRUNNER_STEPS_EXCESS: f64 = 1.9023145427845396;
    let law = format!(
        "<piecewise><piece><cn>0</cn>{}</piece><otherwise>{}</otherwise></piecewise>",
        ap("lt", &["<ci>S</ci>", &cn("0")]),
        ap("times", &["<ci>k</ci>", "<ci>S</ci>"])
    );
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="guard">
<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="C" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="k" value="1" constant="true"/></listOfParameters>
<listOfReactions><reaction id="J" reversible="false"><listOfReactants><speciesReference species="S" stoichiometry="1" constant="true"/></listOfReactants><kineticLaw><math {MATH}>{law}</math></kineticLaw></reaction></listOfReactions>
</model></sbml>"#
    );
    let compiled = compile(&xml, &CompileOptions::default()).unwrap();
    for output in [Output::Uniform { steps: 30 }, Output::IntegratorSteps] {
        let mut req = request(50.0, output.clone());
        req.selections = vec!["time".to_string(), "S".to_string()];
        req.settings.absolute_tolerance = Tolerance::Scalar(1e-12);
        let (result, stats) = run(&compiled, &req);
        assert_eq!(stats.integrations, 2, "{output:?} {stats:?}");
        let excess = column(&result, "time")
            .iter()
            .zip(column(&result, "S"))
            .map(|(t, s)| {
                let c = (-t).exp();
                (s - c).abs() / (1e-12 + 1e-7 * c)
            })
            .fold(0.0, f64::max);
        let roadrunner = match output {
            Output::IntegratorSteps => ROADRUNNER_STEPS_EXCESS,
            _ => ROADRUNNER_EXCESS,
        };
        assert!(excess <= 1.01 * roadrunner, "{output:?} {excess}");
    }
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

#[test]
fn well_separated_switches_do_not_share_the_budget_of_steps() {
    // x' = piecewise(1, sin(time) > 0, 0) over about 16 periods, output only at the start and
    // the end: a restart at a root far from the last one starts the budget of steps again (as
    // libroadrunner's count of steps of one call of CVODE), so maximum_num_steps bounds the
    // steps between two such roots and not the steps of the whole interval; x(100) = 16 pi
    let rate = step(&ap("gt", &[&ap("sin", &[TIME]), &cn("0")]));
    let compiled = model(&rate, "");
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    // (a step no longer than 0.5, so no step crosses two roots, whose signs would cancel)
    let mut req = request(100.0, Output::Uniform { steps: 1 });
    req.settings.maximum_num_steps = MAX_STEPS;
    req.settings.maximum_time_step = 0.5;
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let result = instance.simulate(&req).unwrap();
    let stats = instance.stats().unwrap();
    assert!(stats.steps > u64::from(MAX_STEPS), "{stats:?}");
    // the start and a start again at each of the 31 roots k pi
    assert_eq!(stats.integrations, 32, "{stats:?}");
    let x = column(&result, "x")[1];
    let exact = 16.0 * std::f64::consts::PI;
    assert!((x - exact).abs() <= 1e-12 + 1e-7 * exact, "{x} {exact}");
}

/// The steps between two roots of `well_separated_switches_do_not_share_the_budget_of_steps`
/// are fewer, the steps of the whole interval more.
const MAX_STEPS: u32 = 200;

#[test]
#[cfg(feature = "native")]
fn a_wide_xor_preserves_root_crossings_in_native_and_wasm_engines() {
    // Twelve sign changes must all stop the integrator. The integral of the alternating
    // unit/zero rate at integer times is floor(t / 2), with half a unit at each midpoint.
    let operands = (1..=12)
        .map(|k| ap("gt", &[TIME, &cn(&k.to_string())]))
        .collect::<Vec<_>>();
    let condition = ap(
        "xor",
        &operands.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let compiled = model(&step(&condition), "");
    assert_eq!(compiled.description.n_discontinuity_roots, 1);
    let req = request(13.0, Output::Uniform { steps: 26 });
    let (wasm, stats) = run(&compiled, &req);
    assert_eq!(stats.integrations, 13, "{stats:?}");
    let mut native = runtime()
        .instantiate_native(&compiled.wasm, &compiled.description)
        .unwrap();
    let native_result = native.simulate(&req).unwrap();
    assert_eq!(native.stats().integrations, 13);
    for result in [&wasm, &native_result] {
        for (t, x) in column(result, "time").into_iter().zip(column(result, "x")) {
            let expected = (t / 2.0).floor() + (t % 2.0 - 1.0).max(0.0);
            assert!(
                (x - expected).abs() < 1e-7,
                "t={t}, x={x}, expected={expected}"
            );
        }
    }
    for (a, b) in wasm.values.iter().zip(&native_result.values) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }
}

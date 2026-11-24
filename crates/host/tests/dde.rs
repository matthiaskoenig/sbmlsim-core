//! Delay semantics through the public SBML compiler and simulation requests.

use std::sync::OnceLock;

use sbmlsim_abi::{
    Change, IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    Tolerance,
};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::{LoadedModel, Runtime};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const TIME: &str = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">time</csymbol>"#;
const DELAY: &str = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol>"#;
const RATE_OF: &str = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/rateOf">rateOf</csymbol>"#;

fn load(body: &str) -> LoadedModel {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="dde">{body}</model></sbml>"#
    );
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    RUNTIME
        .get_or_init(|| Runtime::embedded().unwrap())
        .load(&compiled.wasm, &compiled.description)
        .unwrap()
}

fn request(end: f64, steps: u32, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_owned(),
        start: 0.,
        end,
        output: Output::Uniform { steps },
        selections: selections.iter().map(|s| s.to_string()).collect(),
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        seed: 0,
        settings: IntegratorSettings {
            relative_tolerance: 1e-9,
            absolute_tolerance: Tolerance::Scalar(1e-11),
            ..Default::default()
        },
    }
}

fn each_host(model: &LoadedModel, req: &SimulationRequest, check: impl Fn(&SimulationResult)) {
    check(&model.instantiate().unwrap().simulate(req).unwrap());
    #[cfg(feature = "native")]
    check(&model.instantiate_native().unwrap().simulate(req).unwrap());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-6 * expected.abs().max(1.),
        "expected {expected}, got {actual}"
    );
}

#[test]
fn initial_assignment_history_nested_delay_and_time_dependent_lag() {
    let model = load(&format!(
        r#"<listOfParameters>
<parameter id="x" constant="false"/><parameter id="nested" constant="false"/>
<parameter id="half" constant="false"/><parameter id="now" constant="false"/>
<parameter id="slope" constant="false"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="x"><math {MATH}><apply><plus/><cn>1</cn>{TIME}</apply></math></initialAssignment></listOfInitialAssignments>
<listOfRules><rateRule variable="x"><math {MATH}><cn>2</cn></math></rateRule>
<assignmentRule variable="nested"><math {MATH}><apply>{DELAY}<apply>{DELAY}<ci>x</ci><cn>0.5</cn></apply><cn>0.5</cn></apply></math></assignmentRule>
<assignmentRule variable="half"><math {MATH}><apply>{DELAY}<ci>x</ci><apply><divide/>{TIME}<cn>2</cn></apply></apply></math></assignmentRule>
<assignmentRule variable="now"><math {MATH}><apply>{DELAY}<ci>x</ci><cn>0</cn></apply></math></assignmentRule>
<assignmentRule variable="slope"><math {MATH}><apply>{DELAY}<apply>{RATE_OF}<ci>x</ci></apply><cn>1</cn></apply></math></assignmentRule>
</listOfRules>"#
    ));
    let req = request(2., 8, &["time", "nested", "half", "now", "slope"]);
    each_host(&model, &req, |result| {
        assert_eq!(result.n_rows, 9);
        for &[t, nested, half, now, slope] in result.values.as_chunks::<5>().0 {
            close(nested, if t < 1. { t } else { 1. + 2. * (t - 1.) });
            close(half, 1. + t);
            close(now, 1. + 2. * t);
            close(slope, if t < 1. { 0. } else { 2. });
        }
    });
}

#[test]
fn delayed_feedback_uses_the_integrated_trajectory_between_output_times() {
    let model = load(&format!(
        r#"<listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply>{DELAY}<ci>x</ci><cn>1</cn></apply></math></rateRule></listOfRules>"#
    ));
    let req = request(3., 2, &["time", "x"]);
    each_host(&model, &req, |result| {
        assert_eq!(result.n_rows, 3);
        for &[t, x] in result.values.as_chunks::<2>().0 {
            close(
                x,
                1. + t + (t - 1.).max(0.).powi(2) / 2. + (t - 2.).max(0.).powi(3) / 6.,
            );
        }
    });
}

#[test]
fn delayed_species_reads_concentration_in_the_historical_compartment() {
    let model = load(&format!(
        r#"<listOfCompartments><compartment id="V" size="1" constant="false"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="V" initialAmount="6" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="previous" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="V"><math {MATH}><cn>1</cn></math></rateRule>
<assignmentRule variable="previous"><math {MATH}><apply>{DELAY}<ci>S</ci><cn>0.5</cn></apply></math></assignmentRule></listOfRules>"#
    ));
    let req = request(2., 8, &["time", "previous"]);
    each_host(&model, &req, |result| {
        assert_eq!(result.n_rows, 9);
        for &[t, previous] in result.values.as_chunks::<2>().0 {
            close(previous, 6. / (1. + (t - 0.5).max(0.)));
        }
    });
}

#[test]
fn historical_parameter_values_survive_events_and_request_changes_on_prepared_replay() {
    let model = load(&format!(
        r#"<listOfParameters><parameter id="p" value="2" constant="false"/>
<parameter id="previous" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="previous"><math {MATH}><apply>{DELAY}<ci>p</ci><cn>0.5</cn></apply></math></assignmentRule></listOfRules>
<listOfEvents><event useValuesFromTriggerTime="true"><trigger persistent="true" initialValue="false"><math {MATH}><apply><geq/>{TIME}<cn>1</cn></apply></math></trigger>
<listOfEventAssignments><eventAssignment variable="p"><math {MATH}><cn>12</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#
    ));
    let mut req = request(3., 12, &["time", "previous"]);
    req.changes.push(Change {
        time: 2.,
        values: vec![("p".into(), 20.)],
        increments: vec![],
        forms: Vec::new(),
    });
    let plan = model.prepare(&req, &["p".into()]).unwrap();
    let verify = |result: SimulationResult, initial| {
        assert_eq!(result.n_rows, 14); // The request returns both sides of the change at t=2.
        for &[t, previous] in result.values.as_chunks::<2>().0 {
            close(
                previous,
                if t < 1.5 {
                    initial
                } else if t < 2.5 {
                    12.
                } else {
                    20.
                },
            );
        }
    };
    let mut wasm = model.instantiate().unwrap();
    #[cfg(feature = "native")]
    let mut native = model.instantiate_native().unwrap();
    for initial in [2., 7., 2.] {
        verify(wasm.simulate_prepared(&plan, &[initial]).unwrap(), initial);
        #[cfg(feature = "native")]
        verify(
            native.simulate_prepared(&plan, &[initial]).unwrap(),
            initial,
        );
    }
}

#[test]
fn state_dependent_lag_can_grow_continuously_from_zero() {
    // lag'=1/2 and lag(0)=0 give x'(t)=x(t/2). The Taylor coefficients satisfy
    // a[0]=1 and a[n+1]=a[n]/((n+1)*2^n).
    let model = load(&format!(
        r#"<listOfParameters>
<parameter id="x" value="1" constant="false"/><parameter id="lag" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply>{DELAY}<ci>x</ci><ci>lag</ci></apply></math></rateRule>
<rateRule variable="lag"><math {MATH}><cn>0.5</cn></math></rateRule></listOfRules>"#
    ));
    let mut req = request(4., 8, &["time", "x"]);
    for tolerance in [1e-6, 1e-9] {
        req.settings.relative_tolerance = tolerance;
        req.settings.absolute_tolerance = Tolerance::Scalar(tolerance * 0.01);
        each_host(&model, &req, |result| {
            assert_eq!(result.n_rows, 9);
            for &[t, x] in result.values.as_chunks::<2>().0 {
                let mut expected = 1.;
                let mut term = 1.;
                for n in 0..20 {
                    term *= t / (f64::from(n + 1) * 2_f64.powi(n));
                    expected += term;
                }
                assert!(
                    (x - expected).abs() <= 40. * tolerance * expected,
                    "t={t} tolerance={tolerance}: {x} vs {expected}"
                );
            }
        });
    }
}

#[test]
fn a_delayed_pulse_reaches_the_rhs_and_trigger_between_sparse_outputs() {
    for (lag, area) in [
        ("<cn>1</cn>".to_owned(), 1.0),
        (
            format!("<apply><plus/><cn>1</cn><apply><times/><cn>0.1</cn>{TIME}</apply></apply>"),
            1.0 / 0.9,
        ),
    ] {
        let previous = format!("<apply>{DELAY}<ci>p</ci>{lag}</apply>");
        let model = load(&format!(
            r#"<listOfParameters><parameter id="p" value="0" constant="false"/>
<parameter id="x" value="0" constant="false"/><parameter id="seen" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply><times/><cn>100000</cn>{previous}</apply></math></rateRule></listOfRules>
<listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}><apply><gt/>{previous}<cn>0.5</cn></apply></math></trigger>
<listOfEventAssignments><eventAssignment variable="seen"><math {MATH}><cn>1</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#
        ));
        for steps in [1, 40] {
            let mut req = request(2., steps, &["x", "seen"]);
            for (time, value) in [(0.25, 1.), (0.25001, 0.)] {
                req.changes.push(Change {
                    time,
                    values: vec![("p".into(), value)],
                    increments: vec![],
                    forms: Vec::new(),
                });
            }
            each_host(&model, &req, |result| {
                let tail = &result.values[result.values.len() - 2..];
                close(tail[0], area);
                assert_eq!(tail[1], 1.);
            });
        }
    }
}

#[test]
fn initial_selections_and_concentration_preinit_apply_to_the_history() {
    let model = load(&format!(
        r#"<listOfCompartments><compartment id="V" size="2" constant="true"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="V" initialConcentration="3" hasOnlySubstanceUnits="false" boundaryCondition="true" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="previous" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="previous"><math {MATH}><apply>{DELAY}<ci>S</ci><cn>1</cn></apply></math></assignmentRule></listOfRules>"#
    ));
    let i = model
        .description()
        .selections
        .iter()
        .position(|s| s.id == "previous")
        .unwrap();
    close(model.initial_selections().unwrap()[i], 3.);
    let mut req = request(0., 1, &["previous"]);
    req.output = Output::Times(vec![0.]);
    req.preinit = vec![("[S]".into(), 5.)];
    each_host(&model, &req, |result| assert_eq!(result.values, [0., 5.]));
}

#[test]
fn steady_state_supplies_the_history_at_simulation_start() {
    let model = load(&format!(
        r#"<listOfParameters><parameter id="x" value="0" constant="false"/>
<parameter id="previous" constant="false"/></listOfParameters><listOfRules>
<rateRule variable="x"><math {MATH}><apply><minus/><cn>1</cn><ci>x</ci></apply></math></rateRule>
<assignmentRule variable="previous"><math {MATH}><apply>{DELAY}<ci>x</ci><cn>1</cn></apply></math></assignmentRule></listOfRules>"#
    ));
    let mut req = request(1., 2, &["x", "previous"]);
    req.steady_state = Some(Default::default());
    each_host(&model, &req, |result| {
        assert_eq!(result.n_rows, 3);
        for &[_, x, previous] in result.values.as_chunks::<3>().0 {
            close(x, 1.);
            close(previous, 1.);
        }
    });
}

#[test]
fn invalid_durations_and_exhausted_step_budgets_leave_the_instance_reusable() {
    for (rule, lag) in [
        ("assignmentRule", "<ci>k</ci>".to_owned()),
        (
            "rateRule",
            format!("<apply><divide/><cn>1</cn><apply><plus/><ci>k</ci>{TIME}</apply></apply>"),
        ),
    ] {
        let model = load(&format!(
            r#"<listOfParameters><parameter id="p" value="1" constant="false"/>
<parameter id="k" value="1" constant="false"/><parameter id="x" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="p"><math {MATH}><ci>p</ci></math></rateRule>
<{rule} variable="x"><math {MATH}><apply>{DELAY}<ci>p</ci>{lag}</apply></math></{rule}></listOfRules>"#
        ));
        let req = request(2., 2, &["p"]);
        let mut invalid = req.clone();
        invalid.preinit = vec![("k".into(), if rule == "rateRule" { 0. } else { -1. })];
        let mut budget = req.clone();
        budget.settings.maximum_num_steps = 1;
        let check_error = |error: sbmlsim_abi::EngineError| {
            assert!(error.to_string().contains("delay duration"), "{error}")
        };
        macro_rules! check_instance {
            ($instance:expr) => {{
                let mut instance = $instance.unwrap();
                check_error(instance.simulate(&invalid).unwrap_err());
                assert!(matches!(
                    instance.simulate(&budget),
                    Err(sbmlsim_abi::EngineError::Integration { flag: -1, .. })
                ));
                let result = instance.simulate(&req).unwrap();
                close(*result.values.last().unwrap(), 2_f64.exp());
            }};
        }
        check_instance!(model.instantiate());
        #[cfg(feature = "native")]
        check_instance!(model.instantiate_native());
    }
}

#[test]
fn independent_initial_assignments_with_nested_history_use_only_their_dependencies() {
    let mut parameters = String::from(r#"<parameter id="a" constant="true"/>"#);
    let mut initial = format!(
        r#"<initialAssignment symbol="a"><math {MATH}><apply><plus/><cn>64</cn>{TIME}</apply></math></initialAssignment>"#
    );
    for i in 1..=24 {
        parameters.push_str(&format!(r#"<parameter id="b{i}" constant="true"/>"#));
        initial.push_str(&format!(r#"<initialAssignment symbol="b{i}"><math {MATH}><apply>{DELAY}<apply>{DELAY}<ci>a</ci><cn>1</cn></apply><cn>{i}</cn></apply></math></initialAssignment>"#));
    }
    let model = load(&format!(
        "<listOfParameters>{parameters}</listOfParameters><listOfInitialAssignments>{initial}</listOfInitialAssignments>"
    ));
    let mut req = request(0., 1, &["b1", "b12", "b24"]);
    req.output = Output::Times(vec![0.]);
    each_host(&model, &req, |result| {
        assert_eq!(result.values, [0., 62., 51., 39.])
    });
}

#[test]
fn a_short_delay_in_a_constant_rhs_finishes_within_the_step_budget() {
    let model = load(&format!(
        r#"<listOfParameters>
<parameter id="p" value="3" constant="true"/><parameter id="x" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply>{DELAY}<ci>p</ci><cn>0.000000001</cn></apply></math></rateRule></listOfRules>"#
    ));
    let mut req = request(1., 1, &["time", "x"]);
    req.settings.maximum_num_steps = 200;
    each_host(&model, &req, |result| {
        assert_eq!(result.n_rows, 2);
        close(result.values[3], 3.);
    });
}

#[test]
fn a_reversing_history_query_rearms_the_epoch_it_just_crossed() {
    // q(t) = t - lag(t) = 1 + (t-2)(2.01-t) crosses the same parameter
    // jump forward at 2 and backward at 2.01. The lag stays strictly positive.
    let lag = format!(
        "<apply><plus/><apply><times/>{TIME}{TIME}</apply><apply><times/><cn>-3.01</cn>{TIME}</apply><cn>3.02</cn></apply>"
    );
    let model = load(&format!(
        r#"<listOfParameters><parameter id="p" value="0" constant="false"/>
<parameter id="x" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="x"><math {MATH}><apply>{DELAY}<ci>p</ci>{lag}</apply></math></rateRule></listOfRules>"#
    ));
    let mut req = request(2.1, 1, &["x"]);
    req.output = Output::IntegratorSteps;
    // Separate the two crossings so the test isolates rearming, rather than
    // the documented limit of two unbracketed roots in one solver step.
    req.settings.maximum_time_step = 0.006;
    req.changes = vec![Change {
        time: 1.,
        values: vec![("p".into(), 1.)],
        increments: vec![],
        forms: Vec::new(),
    }];
    each_host(&model, &req, |result| {
        close(*result.values.last().unwrap(), 0.01);
        let rows = result.values.as_chunks::<2>().0;
        for crossing in [2., 2.01] {
            assert!(
                rows.windows(2).any(|pair| {
                    pair[0][0] == pair[1][0] && (pair[0][0] - crossing).abs() < 1e-8
                }),
                "missing delayed discontinuity at {crossing}"
            );
        }
    });
}

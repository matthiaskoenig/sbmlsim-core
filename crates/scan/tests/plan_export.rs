//! The plans of a scan, `plan/1`: the plans of every fixture are the plans the planner runs
//! (every point of a scan is the plan of its simulation and model with the values of the point
//! applied), and the JSON reads back as the plans, every number by its bits.

mod common;

use std::path::{Path, PathBuf};

use sbmlsim_scan::{
    Assigned, CompiledPlans, ModelInfo, PLAN_CONTRACT, Plan, PlanRequest, Scan, ScanErrorKind,
    compile_plans,
};
use serde::Deserialize;

/// Every fixture of the planner.
const CASES: [&str; 25] = [
    "custom_observable",
    "events_steps",
    "events_times",
    "failing_point",
    "failing_point_raise",
    "fold_linear_form",
    "fold_nonlinear",
    "formula_reductions",
    "keep_selections",
    "multiple_dosing",
    "negative_start",
    "pk_oral",
    "points_block",
    "preinit",
    "presimulation_steady_state",
    "scan_2d_values",
    "scan_at",
    "scan_simulations",
    "steady_state_output",
    "steps_plain",
    "time_grid",
    "time_shift",
    "times_output",
    "time_unit_hr_quantities",
    "tutorial_scan",
];

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/scan")
}

#[derive(Deserialize)]
struct File {
    model: String,
}

/// The text of the request of a fixture and the SBML of its model.
fn fixture(case: &str) -> (String, String) {
    let text = std::fs::read_to_string(testdata().join(format!("{case}.json"))).unwrap();
    let file: File = serde_json::from_str(&text).unwrap();
    let request = serde_json::to_string(&common::fixture_request(case)).unwrap();
    (request, common::sbml(&file.model))
}

/// The plan of a point: the plan of its simulation and model with the values of the point, as a
/// host applies them (`Plan::with_values` of the values without a time, then every time).
fn point_plan(compiled: &CompiledPlans, scan: &Scan, index: usize, model: &ModelInfo) -> Plan {
    let expanded = scan.expanded();
    let position = expanded.position(index);
    let entry = &compiled.plans[expanded.plan_of(&position)];
    let mut plan = entry.plan.plan().unwrap();
    let mut timed: Vec<(f64, Vec<(String, f64)>)> = Vec::new();
    let mut values = Vec::new();
    for (d, of_dimension) in entry.values.iter().enumerate() {
        for target in of_dimension {
            let value = (target.target.clone(), target.values[position[d]]);
            match entry.at[d] {
                None => values.push(value),
                Some(at) => match timed.iter_mut().find(|(t, _)| *t == at) {
                    Some((_, v)) => v.push(value),
                    None => timed.push((at, vec![value])),
                },
            }
        }
    }
    plan = plan.with_values(&values, None, model).unwrap();
    timed.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (at, values) in timed {
        plan = plan.with_values(&values, Some(at), model).unwrap();
    }
    plan
}

/// The plans of a plan with every number by its bits, as JSON.
fn bits(plan: &Plan) -> String {
    serde_json::to_string(&sbmlsim_scan::PlanDocument::from(plan)).unwrap()
}

#[test]
fn the_plans_of_every_fixture_are_the_plans_the_planner_runs() {
    for case in CASES {
        let (text, sbml) = fixture(case);
        let description = common::compile(&sbml).description;
        let model = ModelInfo::new(&description).unwrap();
        let request: PlanRequest = serde_json::from_str(&text).unwrap();
        let compiled = compile_plans(&request, Some(description.clone()), Vec::new()).unwrap();
        assert_eq!(compiled.contract, PLAN_CONTRACT);
        let scan = Scan::open(serde_json::from_str(&text).unwrap(), description).unwrap();
        let expanded = scan.expanded();
        assert_eq!(compiled.plans.len(), expanded.plans.len(), "{case}");
        for (entry, plan) in compiled.plans.iter().zip(&expanded.plans) {
            assert_eq!(bits(&entry.plan.plan().unwrap()), bits(plan), "{case}");
            assert_eq!(entry.model, None, "{case}");
        }
        for index in 0..expanded.size() {
            assert_eq!(
                bits(&point_plan(&compiled, &scan, index, &model)),
                bits(&scan.point_plan(index).unwrap()),
                "{case}: the point {index}"
            );
        }
    }
}

#[test]
fn the_json_of_the_plans_reads_back_by_bits() {
    for case in CASES {
        let (text, sbml) = fixture(case);
        let description = common::compile(&sbml).description;
        let request: PlanRequest = serde_json::from_str(&text).unwrap();
        let compiled = compile_plans(&request, Some(description), Vec::new()).unwrap();
        let json = serde_json::to_string(&compiled).unwrap();
        let back: CompiledPlans = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json, "{case}");
    }
}

#[test]
fn non_finite_numbers_and_formulas_read_back() {
    let sbml = common::sbml("scan_fold.xml");
    let description = common::compile(&sbml).description;
    let simulation = r#"{"type": "Simulation", "time_unit": null, "start": 0.0, "end": 10.0, "preinit_changes": {"k": null}, "changes": [{"times": [5.0], "values": {"kb": 2.5, "C": "k * 2"}}], "presimulation": null, "times": [0.0, 10.0, "inf"], "steps": null, "time_shift": 0.0}"#;
    let values = common::values_dimension("rate", r#"{"kb": [1.0, "inf", "-inf"]}"#, "{}", 3);
    let text = format!(
        r#"{{"contract": "scan/1", "scan": {}}}"#,
        common::scan(simulation, &[values])
    );
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let compiled = compile_plans(&request, Some(description), Vec::new()).unwrap();
    let plan = compiled.plans[0].plan.plan().unwrap();
    assert!(plan.preinit[0].number().unwrap().is_nan());
    let event = &plan.events[0];
    assert_eq!(event.assignments[0].number(), Some(2.5));
    assert_eq!(
        event.assignments[1].value,
        Assigned::Formula("k * 2".to_string())
    );
    assert!(plan.steady_state_output.is_some());
    assert_eq!(compiled.plans[0].at, [None]);
    let json = serde_json::to_string(&compiled).unwrap();
    assert!(json.contains(r#""preinit":[{"target":"k","kind":"parameter","value":null}]"#));
    assert!(json.contains(r#""values":[1.0,"inf","-inf"]"#), "{json}");
    assert!(json.contains(r#""formula":"k * 2""#), "{json}");
    assert!(json.contains(r#""at":[null]"#), "{json}");
    let back: CompiledPlans = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    assert!(
        back.plans[0].plan.plan().unwrap().preinit[0]
            .number()
            .unwrap()
            .is_nan()
    );
}

#[test]
fn a_request_needs_only_its_scan() {
    let description = common::compile(&common::sbml("scan_fold.xml")).description;
    let text = format!(
        r#"{{"contract": "scan/1", "scan": {}}}"#,
        common::scan(&common::simulation(10.0, 2), &[])
    );
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let compiled = compile_plans(&request, Some(description), Vec::new()).unwrap();
    assert_eq!(compiled.plans[0].plan.times, [0.0, 5.0, 10.0]);
}

#[test]
fn the_changes_of_the_model_of_the_run_are_in_its_plans() {
    let description = common::compile(&common::sbml("scan_fold.xml")).description;
    let text = format!(
        r#"{{"contract": "scan/1", "scan": {}, "changes": {{"k": 3.0}}}}"#,
        common::scan(&common::simulation(10.0, 2), &[])
    );
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let compiled = compile_plans(&request, Some(description), Vec::new()).unwrap();
    let preinit = &compiled.plans[0].plan.preinit;
    assert_eq!(preinit.len(), 1);
    assert_eq!(preinit[0].target, "k");
    assert_eq!(preinit[0].value, Some(3.0));
}

#[test]
fn the_plans_of_a_dimension_of_models_name_their_models() {
    let sbml = common::sbml("scan_fold.xml");
    let description = common::compile(&sbml).description;
    let dimension = common::models_dimension("model", &["fast", "slow"]);
    let values = common::values_dimension("rate", r#"{"k": [1.0, 2.0]}"#, "{}", 2);
    let text = format!(
        r#"{{"contract": "scan/1", "scan": {}, "model_changes": {{"slow": {{"k": 3.0}}}}}}"#,
        common::scan(&common::simulation(10.0, 2), &[dimension, values])
    );
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let models = vec![
        ("fast".to_string(), description.clone()),
        ("slow".to_string(), description.clone()),
    ];
    let compiled = compile_plans(&request, None, models).unwrap();
    let keys: Vec<Option<&str>> = compiled.plans.iter().map(|p| p.model.as_deref()).collect();
    assert_eq!(keys, [Some("fast"), Some("slow")]);
    assert!(compiled.plans[0].plan.preinit.is_empty());
    assert_eq!(compiled.plans[1].plan.preinit[0].value, Some(3.0));
    assert!(compiled.plans[0].values[0].is_empty());
    assert_eq!(compiled.plans[1].values[1][0].values, [1.0, 2.0]);
    // the model of the run and a dimension of models exclude each other
    let err = compile_plans(&request, Some(description), Vec::new()).unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Request);
    assert!(
        err.message.contains("replaces the model of the run"),
        "{err}"
    );
}

#[test]
fn a_simulation_which_does_not_fit_the_model_is_an_error() {
    let description = common::compile(&common::sbml("scan_fold.xml")).description;
    let simulation = r#"{"type": "Simulation", "time_unit": null, "start": 0.0, "end": 10.0, "preinit_changes": {"nothing": 1.0}, "changes": [], "presimulation": null, "times": null, "steps": null, "time_shift": 0.0}"#;
    let text = format!(
        r#"{{"contract": "scan/1", "scan": {}}}"#,
        common::scan(simulation, &[])
    );
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let err = compile_plans(&request, Some(description), Vec::new()).unwrap_err();
    assert_eq!(
        err.message,
        "The simulation: 'nothing' is not a parameter, a compartment or a species of the model."
    );
}

#[test]
fn a_value_which_is_not_finite_is_a_number_and_no_formula() {
    let model = common::compile(&common::sbml("scan_fold.xml"));
    let simulation = r#"{"type": "Simulation", "time_unit": null, "start": 0.0, "end": 10.0, "preinit_changes": {"k": "inf", "kb": "NaN"}, "changes": [{"times": [5.0], "values": {"k": "-inf", "kb": null, "C": "k * 2"}}], "presimulation": null, "times": null, "steps": 2, "time_shift": 0.0}"#;
    let scan = common::scan(simulation, &[]);
    let check = |plan: &Plan| {
        let number = |a: &sbmlsim_scan::Assignment| match a.value {
            Assigned::Number(x) => x,
            Assigned::Formula(ref f) => panic!("'{}' is the formula '{f}'", a.target),
        };
        assert_eq!(number(&plan.preinit[0]), f64::INFINITY);
        assert!(number(&plan.preinit[1]).is_nan());
        let event = &plan.events[0].assignments;
        assert_eq!(number(&event[0]), f64::NEG_INFINITY);
        assert!(number(&event[1]).is_nan());
        assert_eq!(event[2].value, Assigned::Formula("k * 2".to_string()));
    };
    // the plans of a host
    let text = format!(r#"{{"contract": "scan/1", "scan": {scan}}}"#);
    let request: PlanRequest = serde_json::from_str(&text).unwrap();
    let compiled = compile_plans(&request, Some(model.description.clone()), Vec::new()).unwrap();
    check(&compiled.plans[0].plan.plan().unwrap());
    // the plans of a scan of the engine
    let request = common::request("failing_point_raise", &scan, &[]);
    let opened = Scan::open(request, model.description).unwrap();
    check(&opened.point_plan(0).unwrap());
}

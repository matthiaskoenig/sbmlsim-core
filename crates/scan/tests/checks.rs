//! The checks of sbmlsim's `Simulator.run` which need no simulation, with sbmlsim's messages:
//! a coordinate never hides and is never hidden by another name of the result
//! (`_check_coordinates`), the lists of names in python's words, the grid of times of
//! `time_magnitudes`, and the first failed point in the order of the scan across the plans of
//! its simulations (`_run_serial`).

mod common;

use common::{Engine, compile, request, sbml, scan, simulation, values_dimension};
use sbmlsim_scan::{Scan, ScanError, ScanErrorKind};
use serde_json::json;

/// The DEMO model of sbmlsim, the model of the fixture `scan_2d_values`.
const DEMO: &str = "Koenig_demo_14.xml";

/// The error of opening a scan of the DEMO model, or `None`; the result keeps `[c__A]` and
/// `Vmax_bA`, so the targets `Km_A` and `Vmax_bB` are no outputs.
fn demo_error(dimensions: &[String], on_error: &str) -> Option<ScanError> {
    let model = compile(&sbml(DEMO));
    let request = request(
        "scan_2d_values",
        &scan(&simulation(1.0, 2), dimensions),
        &[
            ("on_error", json!(on_error)),
            ("keep", json!(["[c__A]", "Vmax_bA"])),
        ],
    );
    Scan::open(request, model.description).err()
}

/// The covariates which would hide or be hidden by a name of the result, the cases of
/// sbmlsim's `test_a_coordinate_never_hides_or_is_hidden_by_a_name` on the DEMO model, whose
/// `Vmax_bA` is an output and `Km_A` and `Vmax_bB` are targets.
fn hiding_cases() -> Vec<(&'static str, Vec<String>, String)> {
    let hidden = |name: &str, dimension: &str, what: &str| {
        format!(
            "The coordinate '{name}' of the dimension '{dimension}' would hide or be hidden by \
             {what} of the same name; rename the covariate."
        )
    };
    let d = |coordinates: &str| values_dimension("d", r#"{"Km_A": [1, 1]}"#, coordinates, 2);
    let mut cases = vec![
        (
            "a selection",
            vec![d(r#"{"Vmax_bA": [1, 1]}"#)],
            hidden("Vmax_bA", "d", "an output of the result"),
        ),
        (
            "a dimension id",
            vec![
                d(r#"{"f": [1, 1]}"#),
                values_dimension("f", r#"{"Vmax_bA": [1, 1]}"#, "{}", 2),
            ],
            hidden("f", "d", "a dimension id"),
        ),
        (
            "a changed target",
            vec![
                d("{}"),
                values_dimension("f", r#"{"Vmax_bB": [1, 1]}"#, r#"{"Km_A": [1, 1]}"#, 2),
            ],
            hidden("Km_A", "f", "a changed target"),
        ),
        (
            "a coordinate",
            vec![
                d(r#"{"w": [1, 1]}"#),
                values_dimension("f", r#"{"Vmax_bB": [1, 1]}"#, r#"{"w": [1, 1]}"#, 2),
            ],
            hidden("w", "f", "a coordinate of the dimension 'd'"),
        ),
    ];
    for reserved in ["time", "status", "_point", "_index", "statistic"] {
        cases.push((
            reserved,
            vec![d(&format!(r#"{{"{reserved}": [1, 1]}}"#))],
            hidden(reserved, "d", "a name of the result"),
        ));
    }
    cases
}

#[test]
fn a_coordinate_never_hides_or_is_hidden_by_a_name() {
    for on_error in ["raise", "flag"] {
        for (case, dimensions, message) in hiding_cases() {
            let err = demo_error(&dimensions, on_error)
                .unwrap_or_else(|| panic!("{case} ({on_error}): no error"));
            assert_eq!(err.kind, ScanErrorKind::Request, "{case}");
            assert_eq!(err.message, message, "{case} ({on_error})");
        }
    }
}

#[test]
fn a_coordinate_cannot_hide_a_qualified_target() {
    // `Vmax_bA` is a selection, so the dimension `rate` names its values `rate.Vmax_bA`
    let dimensions = [values_dimension(
        "rate",
        r#"{"Vmax_bA": [1, 2]}"#,
        r#"{"rate.Vmax_bA": [3, 4]}"#,
        2,
    )];
    let err = demo_error(&dimensions, "flag").expect("an error");
    assert_eq!(
        err.message,
        "The coordinate 'rate.Vmax_bA' of the dimension 'rate' would hide or be hidden by a \
         changed target of the same name; rename the covariate."
    );
}

#[test]
fn a_dimension_cannot_hide_a_qualified_target() {
    let dimensions = [
        values_dimension("rate", r#"{"Vmax_bA": [1, 2]}"#, "{}", 2),
        values_dimension("rate.Vmax_bA", r#"{"Km_A": [1, 2]}"#, "{}", 2),
    ];
    let err = demo_error(&dimensions, "flag").expect("an error");
    assert_eq!(
        err.message,
        "The coordinate 'rate.Vmax_bA' of the changed target 'Vmax_bA' would hide a dimension or \
         an output of the same name; rename the dimension 'rate'."
    );
}

#[test]
fn a_covariate_which_hides_nothing_is_a_coordinate() {
    let model = compile(&sbml(DEMO));
    let dimensions = [values_dimension(
        "d",
        r#"{"Km_A": [1, 2]}"#,
        r#"{"weight": [70, 80]}"#,
        2,
    )];
    let request = request(
        "scan_2d_values",
        &scan(&simulation(1.0, 2), &dimensions),
        &[],
    );
    let scan = Scan::open(request, model.description.clone()).unwrap();
    let result = Engine::of(&model).result(scan).unwrap();
    let weight = result.coords.iter().find(|c| c.name == "weight").unwrap();
    assert_eq!(weight.dims, ["d"]);
}

/// The model of the fold fixtures with a parameter `status`, a name of the result.
fn status_model() -> String {
    sbml("scan_fold.xml").replace(
        r#"<parameter id="k" value="0.2" constant="true"/>"#,
        r#"<parameter id="k" value="0.2" constant="true"/>
      <parameter id="status" value="1" constant="true"/>"#,
    )
}

#[test]
fn a_selection_which_the_result_reserves_is_an_error() {
    let model = compile(&status_model());
    let mut selections = common::fixture_request("failing_point_raise")["selections"].clone();
    selections.as_array_mut().unwrap().push(json!("status"));
    let request = request(
        "failing_point_raise",
        &scan(&simulation(1.0, 2), &[]),
        &[("selections", selections), ("on_error", json!("flag"))],
    );
    let err = Scan::open(request, model.description).unwrap_err();
    assert_eq!(
        err.message,
        "The selections ['status'] are names of the result (['_index', '_point', 'statistic', \
         'status', 'time']), select other entities, see `SBMLModel.set_selections`."
    );
}

#[test]
fn a_dimension_named_as_a_selection_is_an_error() {
    let model = compile(&sbml(DEMO));
    let dimensions = [values_dimension("Vmax_bB", r#"{"Km_A": [1]}"#, "{}", 1)];
    let request = request(
        "scan_2d_values",
        &scan(&simulation(1.0, 2), &dimensions),
        &[],
    );
    let err = Scan::open(request, model.description).unwrap_err();
    assert_eq!(
        err.message,
        "The dimension ids ['Vmax_bB'] are selections of the model or observables of the run: a \
         dimension and a variable of the result share no name, choose other ids."
    );
}

#[test]
fn reserved_dimension_ids_are_listed_as_python_lists_them() {
    let dimensions = [values_dimension("status", r#"{"Km_A": [1]}"#, "{}", 1)];
    let err = demo_error(&dimensions, "raise").expect("an error");
    assert_eq!(
        err.message,
        "The dimension ids ['status'] are names of the result (['_index', '_point', \
         'statistic', 'status', 'time']), choose other ids."
    );
}

#[test]
fn a_grid_of_times_with_a_unit_needs_a_time_unit() {
    let unitless = compile(&sbml("scan_fold.xml").replace(
        r#"id="scan_fold">"#,
        r#"id="scan_fold" timeUnits="dimensionless">"#,
    ));
    let open = |model: &common::Model, time: serde_json::Value| {
        let request = request(
            "failing_point_raise",
            &scan(&simulation(1.0, 2), &[]),
            &[("time", time)],
        );
        Scan::open(request, model.description.clone())
    };
    let err = open(&unitless, json!({"value": [0, 1], "unit": "second"})).unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Units);
    assert_eq!(
        err.message,
        "The model has no unit of time, give the times as numbers, not in 'second'."
    );
    // a dimensionless quantity is numbers
    let grid = open(&unitless, json!({"value": [0, 1], "unit": "dimensionless"})).unwrap();
    drop(grid);
    let seconds = compile(&sbml("scan_fold.xml"));
    let err = open(&seconds, json!({"value": [0, 1], "unit": "milligram"})).unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Units);
    assert_eq!(
        err.message,
        "The times in 'milligram' cannot be converted to the unit of time 'second' of the model."
    );
}

#[test]
fn the_first_failed_point_in_scan_order_raises() {
    // B' = kb B^2 with B(0) = 1 blows up at 1/kb: kb = 2 within the simulation `b` (end 1) and
    // kb = 5 within both; the chunk of `a` holds the points 0, 2, 4 and fails at 4, the one of
    // `b` starts before it and fails at 3, the first failed point in the order of the scan
    let model = compile(&sbml("scan_fold.xml"));
    let simulations = format!(
        r#"{{"id": "sim", "kind": "simulations", "labels": ["a", "b"], "values": {{}}, "simulations": {{"a": {}, "b": {}}}, "models": {{}}, "at": null, "design": null, "coordinates": {{}}}}"#,
        simulation(0.3, 3),
        simulation(1.0, 4)
    );
    let dimensions = [
        values_dimension("rate", r#"{"kb": [0.1, 2.0, 5.0]}"#, "{}", 3),
        simulations,
    ];
    let request = request(
        "failing_point_raise",
        &scan(&simulation(1.0, 2), &dimensions),
        &[],
    );
    let mut scan = Scan::open(request, model.description.clone()).unwrap();
    let (outcome, _) = Engine::of(&model).run(&mut scan);
    let err = outcome.unwrap_err();
    assert_eq!(err.kind, ScanErrorKind::Point);
    assert_eq!(err.point, Some(3));
    assert!(
        err.message
            .starts_with("The point rate=1, kb=2.0, sim=b of the scan failed: RuntimeError:"),
        "{}",
        err.message
    );
    assert_eq!(scan.finished().unwrap_err(), err);
}

/// A unit the registry refuses keeps its reason in the message, here the power 0.5 of `sqrt`
/// of a concentration, which pint reads and the registry, whose powers are integers, does not.
#[test]
fn a_unit_which_cannot_be_converted_names_the_reason() {
    let model = compile(&sbml(DEMO));
    let request = request(
        "formula_reductions",
        &scan(&simulation(1.0, 2), &[]),
        &[(
            "observables",
            json!([{"type": "Formula", "id": "root", "formula": "sqrt([e__A])", "unit": "mmol/l"}]),
        )],
    );
    let error = Scan::open(request, model.description).err().unwrap();
    assert_eq!(error.kind, ScanErrorKind::Units);
    assert_eq!(
        error.message,
        "The formula of the observable 'root' has the unit 'mole ** 0.5 / meter ** 1.5', which \
         cannot be converted into its unit 'millimole / liter': the unit 'mole ** 0.5 / meter \
         ** 1.5' has the power 0.5, which is no integer: the powers of units are integers."
    );
}

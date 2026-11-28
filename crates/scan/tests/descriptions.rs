//! What the planner reads of a description `model/2`: a description of a compiler before the
//! planner is refused, and a unit the description names by the id of its unit definition (a
//! factor which is no prefix) is read with the unit definitions of the description.

mod common;

use common::{Engine, compile, request, sbml, scan, variable};
use sbmlsim_scan::{Scan, ScanErrorKind};

/// The fold model with the parameter `k` in the unit `u37`, 3.7 seconds, which pint has no
/// unit of.
fn fold_in_u37() -> String {
    sbml("scan_fold.xml")
        .replace(
            "<listOfCompartments>",
            r#"<listOfUnitDefinitions>
      <unitDefinition id="u37">
        <listOfUnits>
          <unit kind="second" exponent="1" scale="0" multiplier="3.7"/>
        </listOfUnits>
      </unitDefinition>
    </listOfUnitDefinitions>
    <listOfCompartments>"#,
        )
        .replace(r#"id="k" value="0.2""#, r#"id="k" value="0.2" units="u37""#)
}

/// A scan of the fold model whose pre-initialization sets `k` to `value`.
fn preinit_request(value: &str) -> sbmlsim_scan::ScanRequest {
    let simulation = format!(
        r#"{{"type": "Simulation", "time_unit": null, "start": 0.0, "end": 1.0, "preinit_changes": {{"k": {value}}}, "changes": [], "presimulation": null, "times": null, "steps": 2, "time_shift": 0.0}}"#
    );
    request("failing_point_raise", &scan(&simulation, &[]), &[])
}

#[test]
fn a_description_without_the_time_unit_is_refused() {
    let mut description = compile(&sbml("scan_fold.xml")).description;
    description.time_unit = None;
    description.compiler = "sbmlsim-compiler 0.42.0".to_string();
    let error = Scan::open(preinit_request("0.3"), description).unwrap_err();
    assert_eq!(error.kind, ScanErrorKind::Request);
    assert_eq!(
        error.message,
        "The description of the model 'scan_fold' was written by sbmlsim-compiler 0.42.0, which \
         writes no time unit, initial assignments or time dependence of the model, which a scan \
         plans with: compile the model again with the current sbmlsim-core."
    );
}

#[test]
fn a_quantity_is_converted_into_a_unit_of_a_unit_definition() {
    let model = compile(&fold_in_u37());
    assert_eq!(model.description.units["k"], "u37");
    assert_eq!(model.description.unit_definitions["u37"], "(3.7*s)");
    let scan = Scan::open(
        preinit_request(r#"{"value": 7.4, "unit": "second"}"#),
        model.description.clone(),
    )
    .unwrap();
    let result = Engine::of(&model).result(scan).unwrap();
    for k in variable(&result, "k") {
        assert!((k - 2.0).abs() < 1e-12, "{k}");
    }
}

//! A scan through the interface of a page: opened against a compiled model, asked for its
//! rounds and told answers which do not fit, every failure the JSON of a `ScanError`. The
//! fixtures end to end, with the engine of a page answering every round, are
//! `js/test/scan.test.mjs`.
#![cfg(feature = "scan")]

use std::path::Path;

use sbmlsim_wasm::scan;
use serde_json::Value;

fn testdata() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/scan")
}

/// The request of a fixture as the text of its JSON, its mappings in their order, and the
/// description of its model compiled again.
fn fixture(case: &str) -> (String, String) {
    #[derive(serde::Deserialize)]
    struct File {
        request: sbmlsim_scan::ScanRequest,
        model: String,
    }
    let text = std::fs::read_to_string(testdata().join(format!("{case}.json"))).unwrap();
    // the request read from the text and written again, its mappings in their order
    let file: File = serde_json::from_str(&text).unwrap();
    let request = serde_json::to_string(&file.request).unwrap();
    let sbml = std::fs::read_to_string(testdata().join("models").join(&file.model)).unwrap();
    let compiled = sbmlsim_wasm::compile_model(&sbml, &sbmlsim_wasm::ModelOptions::default()).unwrap();
    (request, compiled.description)
}

/// The error of a call which failed.
fn error(answer: Result<(String, Vec<u8>), sbmlsim_scan::ScanError>) -> sbmlsim_scan::ScanError {
    answer.expect_err("the call fails")
}

#[test]
fn a_scan_asks_its_rounds_and_refuses_answers_which_do_not_fit() {
    let (request, description) = fixture("scan_2d_values");
    let (text, values) = scan::open(request.as_bytes(), description.as_bytes(), &[]).unwrap();
    assert!(values.is_empty());
    let opened: Value = serde_json::from_str(&text).unwrap();
    let handle = opened["handle"].as_u64().unwrap() as usize;
    let points = opened["points"].as_u64().unwrap();
    assert!(points > 1);

    let (text, _) = scan::ask(handle).unwrap();
    let round: Value = serde_json::from_str(&text).unwrap();
    let requests = round["simulate"].as_array().unwrap();
    assert_eq!(requests[0]["contract"], "simulation/1");
    assert_eq!(round["progress"], serde_json::json!([0, points]));
    assert_eq!(round["model"], Value::Null);

    // no answers for the requests of the round
    let err = error(scan::tell(handle, br#"{"results": []}"#, &[]));
    assert_eq!(err.kind, sbmlsim_scan::ScanErrorKind::Request);
    // values which are no whole number of doubles
    let err = error(scan::tell(handle, br#"{"results": []}"#, &[0; 3]));
    assert!(
        err.message.contains("no whole number of f64"),
        "{}",
        err.message
    );
    // a table whose values are missing
    let err = error(scan::tell(
        handle,
        br#"{"results": [{"columns": ["time"], "n_rows": 2}]}"#,
        &1.0f64.to_le_bytes(),
    ));
    assert!(err.message.contains("values end"), "{}", err.message);

    // a scan which is not done gives no result and stays open
    let err = error(scan::result(handle));
    assert!(err.message.contains("not done"), "{}", err.message);
    let (again, _) = scan::ask(handle).unwrap();
    assert_eq!(again, text);

    scan::close(handle);
    let err = error(scan::ask(handle));
    assert_eq!(err.kind, sbmlsim_scan::ScanErrorKind::Request);
    assert!(err.message.contains("no open scan"), "{}", err.message);
}

#[test]
fn a_request_which_does_not_fit_the_model_is_refused_with_its_kind() {
    let (request, description) = fixture("steps_plain");
    let mut value: Value = serde_json::from_str(&request).unwrap();
    value["scan"]["simulation"]["preinit_changes"] = serde_json::json!({"no_such_entity": 1.0});
    let err = error(scan::open(
        value.to_string().as_bytes(),
        description.as_bytes(),
        &[],
    ));
    assert_eq!(err.kind, sbmlsim_scan::ScanErrorKind::Request);
    assert!(err.message.contains("no_such_entity"), "{}", err.message);
    let err = error(scan::open(b"{}", description.as_bytes(), &[]));
    assert!(
        err.message.contains("no request of scan/1"),
        "{}",
        err.message
    );
}

#[test]
fn a_round_of_a_dimension_of_models_names_its_model() {
    let (request, description) = fixture("scan_2d_values");
    let mut value: Value = serde_json::from_str(&request).unwrap();
    value["scan"]["dimensions"] = serde_json::json!([{
        "id": "model", "kind": "models", "labels": ["a", "b"], "values": {},
        "simulations": {}, "models": {"a": "<sbml/>", "b": "<sbml/>"}, "at": null,
        "design": null, "coordinates": {}
    }]);
    let models = format!(r#"{{"b": {description}, "a": {description}}}"#);
    let (text, _) = scan::open(value.to_string().as_bytes(), &[], models.as_bytes()).unwrap();
    let opened: Value = serde_json::from_str(&text).unwrap();
    let handle = opened["handle"].as_u64().unwrap() as usize;
    let (text, _) = scan::ask(handle).unwrap();
    let round: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(round["model"], "a");
    assert_eq!(round["finished"], serde_json::json!([]));
    scan::close(handle);
    // the dimension of models replaces the model of the run
    let err = error(scan::open(
        value.to_string().as_bytes(),
        description.as_bytes(),
        models.as_bytes(),
    ));
    assert!(
        err.message.contains("replaces the model"),
        "{}",
        err.message
    );
}

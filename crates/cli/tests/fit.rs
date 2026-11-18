//! `sbmlsim-core fit` on the decay model of `crates/host/tests/decay-model`.
//!
//! The data of the fit is what the model itself simulates at `k = 0.3`, so the fit has to
//! find that value again from a start of three.

#![cfg(feature = "native")]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_host::Runtime;
use serde_json::{Value, json};

/// The model directory of the decay model.
fn model_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../host/tests/decay-model")
}

/// Runs the command line with `args`.
fn run(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .args(args)
        .output()
        .expect("the command runs")
}

/// The data of the fit: the time and `y` of the model at `k = 0.3`.
fn data() -> (Vec<f64>, Vec<f64>) {
    let dir = model_dir();
    let desc: ModelDescription =
        serde_json::from_slice(&fs::read(dir.join("model.json")).unwrap()).unwrap();
    let wasm = fs::read(dir.join("model.wasm")).unwrap();
    let mut request: SimulationRequest =
        serde_json::from_slice(&fs::read(dir.join("request.json")).unwrap()).unwrap();
    request.preinit = vec![("k".to_string(), 0.3)];
    let mut instance = Runtime::embedded()
        .unwrap()
        .instantiate(&wasm, &desc)
        .unwrap();
    let table = instance.simulate(&request).unwrap();
    let columns = table.columns.len();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    (
        (0..table.n_rows)
            .map(|row| table.values[row * columns])
            .collect(),
        (0..table.n_rows)
            .map(|row| table.values[row * columns + y])
            .collect(),
    )
}

/// The request of the fit of `k` as a file, and its path.
fn request_file(name: &str, change: impl FnOnce(&mut Value)) -> PathBuf {
    let dir = model_dir();
    let request: Value =
        serde_json::from_slice(&fs::read(dir.join("request.json")).unwrap()).unwrap();
    let (times, values) = data();
    let mut fit = json!({
        "contract": "fit/1",
        "parameters": [
            {"id": "k", "start": 3.0, "lower": 0.01, "upper": 10.0, "scale": "log10"}
        ],
        "conditions": [{"id": "decay", "request": request}],
        "curves": [{
            "id": "y of the decay",
            "condition": "decay",
            "column": "y",
            "times": times,
            "values": values,
        }],
        "optimizer": {"max_iterations": 50},
    });
    change(&mut fit);
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::write(&path, serde_json::to_vec_pretty(&fit).unwrap()).unwrap();
    path
}

/// What the answer of a fit says about the parameter `k`.
fn fitted(answer: &Value) -> f64 {
    assert_eq!(answer["contract"], "fit/1");
    assert_eq!(answer["parameters"][0]["id"], "k");
    answer["parameters"][0]["value"].as_f64().unwrap()
}

#[test]
fn fit_writes_the_answer_of_the_fit() {
    let request = request_file("decay-fit.json", |_| {});
    let answer_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("decay-fit-answer.json");

    let out = run(&[
        OsStr::new("fit"),
        model_dir().as_os_str(),
        request.as_os_str(),
        OsStr::new("--output"),
        answer_path.as_os_str(),
        OsStr::new("--threads"),
        OsStr::new("2"),
    ]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer: Value = serde_json::from_slice(&fs::read(&answer_path).unwrap()).unwrap();
    assert!(
        (fitted(&answer) - 0.3).abs() < 1e-6,
        "the fit found k = {}, {}",
        fitted(&answer),
        answer["status"]
    );
    assert!(answer["cost"].as_f64().unwrap() < 1e-12);
    assert_eq!(answer["residuals"].as_array().unwrap().len(), 11);
    // the command says where the answer went and what it found
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("decay-fit-answer.json"), "{said}");
    assert!(said.contains("k"), "{said}");
}

#[test]
fn the_answer_of_a_fit_goes_to_stdout_without_an_output() {
    let request = request_file("decay-fit-stdout.json", |fit| {
        fit["optimizer"] = json!({
            "algorithm": {"differential_evolution": {"seed": 3, "population": 8}},
            "max_iterations": 20,
        });
    });

    let out = run(&[
        OsStr::new("fit"),
        model_dir().as_os_str(),
        request.as_os_str(),
    ]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer: Value = serde_json::from_slice(&out.stdout).expect("the answer is JSON");
    assert!(
        (fitted(&answer) - 0.3).abs() < 1e-6,
        "the evolution found k = {}",
        fitted(&answer)
    );
}

#[test]
fn a_fit_of_another_contract_is_reported() {
    let request = request_file("decay-fit-contract.json", |fit| {
        fit["contract"] = json!("fit/2");
    });

    let out = run(&[
        OsStr::new("fit"),
        model_dir().as_os_str(),
        request.as_os_str(),
    ]);

    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("fit/2") && said.contains("fit/1"), "{said}");
}

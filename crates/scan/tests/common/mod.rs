//! What the tests of the checks and of the dimensions of models share: the models of the
//! fixtures compiled, requests written from the options of a fixture, and scans whose rounds
//! the engine answers, as a host does.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sbmlsim_abi::ModelDescription;
use sbmlsim_scan::{Ask, PointError, Scan, ScanError, ScanRequest, ScanResult};
use serde_json::Value as Json;

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/scan")
}

/// The SBML of a model of the fixtures.
pub fn sbml(name: &str) -> String {
    std::fs::read_to_string(testdata().join("models").join(name)).unwrap()
}

/// A compiled model.
pub struct Model {
    pub wasm: Vec<u8>,
    pub description: ModelDescription,
}

/// Compiles an SBML.
pub fn compile(sbml: &str) -> Model {
    let compiled = sbmlsim_compiler::compile(sbml, &Default::default()).unwrap();
    Model {
        wasm: compiled.wasm,
        description: compiled.description,
    }
}

/// The request of a fixture as JSON, whose `scan` and options a test replaces.
pub fn fixture_request(case: &str) -> Json {
    let text = std::fs::read_to_string(testdata().join(format!("{case}.json"))).unwrap();
    let file: Json = serde_json::from_str(&text).unwrap();
    file["request"].clone()
}

/// A request of the options of a fixture (its settings and selections) with the scan of a JSON
/// text, read from the text so its mappings keep their order.
pub fn request(case: &str, scan: &str, options: &[(&str, Json)]) -> ScanRequest {
    let mut base = fixture_request(case);
    for (key, value) in options {
        base[*key] = value.clone();
    }
    base["scan"] = Json::Null;
    let text = serde_json::to_string(&base).unwrap().replacen(
        "\"scan\":null",
        &format!("\"scan\":{scan}"),
        1,
    );
    serde_json::from_str(&text).unwrap()
}

/// A simulation of `Simulation.to_dict()`, ending at `end` with `steps` steps.
pub fn simulation(end: f64, steps: u64) -> String {
    format!(
        r#"{{"type": "Simulation", "time_unit": null, "start": 0.0, "end": {end:?}, "preinit_changes": {{}}, "changes": [], "presimulation": null, "times": null, "steps": {steps}, "time_shift": 0.0}}"#
    )
}

/// A dimension of values of `Dimension.to_dict()`: the labels `0..n`, the values and the
/// coordinates as JSON texts of their mappings.
pub fn values_dimension(id: &str, values: &str, coordinates: &str, n: usize) -> String {
    let labels: Vec<String> = (0..n).map(|k| k.to_string()).collect();
    format!(
        r#"{{"id": "{id}", "kind": "values", "labels": [{}], "values": {values}, "simulations": {{}}, "models": {{}}, "at": null, "design": null, "coordinates": {coordinates}}}"#,
        labels.join(", ")
    )
}

/// A dimension of models of `Dimension.to_dict()`, the labels its keys.
pub fn models_dimension(id: &str, keys: &[&str]) -> String {
    let labels: Vec<String> = keys.iter().map(|k| format!("\"{k}\"")).collect();
    let models: Vec<String> = keys
        .iter()
        .map(|k| format!("\"{k}\": \"<sbml/>\""))
        .collect();
    format!(
        r#"{{"id": "{id}", "kind": "models", "labels": [{}], "values": {{}}, "simulations": {{}}, "models": {{{}}}, "at": null, "design": null, "coordinates": {{}}}}"#,
        labels.join(", "),
        models.join(", ")
    )
}

/// A scan of `Scan.to_dict()`.
pub fn scan(simulation: &str, dimensions: &[String]) -> String {
    format!(
        r#"{{"type": "Scan", "simulation": {simulation}, "dimensions": [{}]}}"#,
        dimensions.join(", ")
    )
}

/// The engine of the models of a scan, an instance per model by its key (`None` the model of
/// the run).
pub struct Engine {
    _runtime: sbmlsim_host::Runtime,
    instances: BTreeMap<Option<String>, sbmlsim_host::Instance>,
}

impl Engine {
    /// The engine of the model of a run.
    pub fn of(model: &Model) -> Engine {
        Engine::of_models(Some(model), &[])
    }

    /// The engine of the model of a run and of the models of a dimension by their keys.
    pub fn of_models(model: Option<&Model>, models: &[(&str, &Model)]) -> Engine {
        let runtime = sbmlsim_host::Runtime::embedded().unwrap();
        let mut instances = BTreeMap::new();
        let instance = |m: &Model| {
            runtime
                .load(&m.wasm, &m.description)
                .unwrap()
                .instantiate()
                .unwrap()
        };
        if let Some(model) = model {
            instances.insert(None, instance(model));
        }
        for (key, m) in models {
            instances.insert(Some((*key).to_string()), instance(m));
        }
        Engine {
            _runtime: runtime,
            instances,
        }
    }

    /// The answers of the engine of a model to the requests of a round.
    pub fn answers(
        &mut self,
        model: &Option<String>,
        requests: &[sbmlsim_abi::SimulationRequest],
    ) -> Vec<Result<sbmlsim_abi::SimulationResult, PointError>> {
        let instance = self
            .instances
            .get_mut(model)
            .unwrap_or_else(|| panic!("a round of the unknown model {model:?}"));
        requests
            .iter()
            .map(|r| instance.simulate(r).map_err(PointError::from))
            .collect()
    }

    /// Runs a scan to its result, the model of every round of simulations recorded.
    pub fn run(&mut self, scan: &mut Scan) -> (Result<(), ScanError>, Vec<Option<String>>) {
        let mut models = Vec::new();
        loop {
            match scan.ask() {
                Ask::Simulate { model, requests } => {
                    let answers = self.answers(&model, &requests);
                    models.push(model);
                    if let Err(err) = scan.tell(answers) {
                        return (Err(err), models);
                    }
                }
                Ask::Observe(_) => panic!("no observable of the host is asked"),
                Ask::Done => return (scan.finished(), models),
            }
        }
    }

    /// Runs a scan to its result.
    pub fn result(&mut self, mut scan: Scan) -> Result<ScanResult, ScanError> {
        self.run(&mut scan).0?;
        scan.result()
    }
}

/// The values of a variable of a result.
pub fn variable<'r>(result: &'r ScanResult, name: &str) -> &'r [f64] {
    &result
        .variables
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no variable '{name}'"))
        .values
}

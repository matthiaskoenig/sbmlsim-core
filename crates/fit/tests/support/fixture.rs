//! The decay model `y' = -k y` of `crates/host/tests/decay-model`, which the tests of the
//! host, of the pool and of the command line simulate as well, and the fit of its parameter.
//!
//! The data of a fit is what the model itself simulates at a value of `k`, through the same
//! engine and the same request, so the residuals at that value are zero bitwise on a linear
//! scale and a fit which starts somewhere else has to find it again.

#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_fit::{Curve, Definition, Parameter, Problem, Scale};
use sbmlsim_host::{LoadedModel, Runtime};

/// The value of `k` the data of a fit is simulated at.
pub const K: f64 = 0.3;

pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

pub fn model() -> Arc<LoadedModel> {
    let wasm = wat::parse_str(include_str!("../../../host/tests/decay.wat")).unwrap();
    let desc: ModelDescription =
        serde_json::from_str(include_str!("../../../host/tests/decay-model/model.json")).unwrap();
    Arc::new(runtime().load(&wasm, &desc).unwrap())
}

pub fn request() -> SimulationRequest {
    serde_json::from_str(include_str!("../../../host/tests/decay-model/request.json")).unwrap()
}

/// The data of a fit: the time and `y` of the model at `k`, at the times of the request.
pub fn data(k: f64) -> (Vec<f64>, Vec<f64>) {
    let model = model();
    let mut instance = model.instantiate_native().unwrap();
    let prepared = model.prepare(&request(), &["k".to_string()]).unwrap();
    let table = instance.simulate_prepared(&prepared, &[k]).unwrap();
    let columns = table.columns.len();
    let time = table.columns.iter().position(|c| c == "time").unwrap();
    let y = table.columns.iter().position(|c| c == "y").unwrap();
    (
        (0..table.n_rows)
            .map(|row| table.values[row * columns + time])
            .collect(),
        (0..table.n_rows)
            .map(|row| table.values[row * columns + y])
            .collect(),
    )
}

/// The definition of the fit of `k`, starting at `start` on a scale.
pub fn definition(start: f64, scale: Scale) -> Definition {
    let (times, values) = data(K);
    Definition {
        parameters: vec![Parameter {
            id: "k".to_string(),
            start,
            lower: 0.01,
            upper: 10.0,
            scale,
        }],
        conditions: vec![request()],
        curves: vec![Curve {
            condition: 0,
            column: "y".to_string(),
            times,
            values,
            noise: None,
            weight: 1.0,
        }],
    }
}

/// The problem of the fit of `k` on the threads of a pool.
pub fn problem(start: f64, scale: Scale, threads: usize) -> Problem {
    Problem::new(definition(start, scale), model(), threads).unwrap()
}

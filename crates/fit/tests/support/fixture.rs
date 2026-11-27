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
        jacobian: Default::default(),
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![Parameter {
            id: "k".to_string(),
            start,
            lower: 0.01,
            upper: 10.0,
            scale,
        }],
        conditions: vec![request().into()],
        curves: vec![Curve {
            condition: 0,
            column: "y".to_string(),
            times,
            values,
            noise: None,
            weight: 1.0,
            after_change: None,
            likelihood: None,
            reduction: None,
            dimension: None,
        }],
    }
}

/// The problem of the fit of `k` on the threads of a pool.
pub fn problem(start: f64, scale: Scale, threads: usize) -> Problem {
    Problem::new(definition(start, scale), model(), threads).unwrap()
}

/// The standard deviation of the measurements of the fit of a pair, which is what a
/// confidence interval of its parameters is measured against: data of this many per cent is
/// what a study of a drug reports.
pub const NOISE: f64 = 0.02;

/// The definition of the fit of `k` and of the initial value of `y`, whose data is the model
/// at `k = K` and `y = 1`, so that a fit of both has to find that pair. Every measurement
/// carries the standard deviation [`NOISE`].
pub fn pair(k: f64, y: f64, scale: Scale) -> Definition {
    let (times, values) = data(K);
    let noise = Some(vec![NOISE; values.len()]);
    Definition {
        jacobian: Default::default(),
        priors: Vec::new(),
        prior_correlation: None,
        parameters: vec![
            Parameter {
                id: "k".to_string(),
                start: k,
                lower: 0.01,
                upper: 10.0,
                scale,
            },
            Parameter {
                id: "y".to_string(),
                start: y,
                lower: 0.1,
                upper: 10.0,
                scale,
            },
        ],
        conditions: vec![request().into()],
        curves: vec![Curve {
            condition: 0,
            column: "y".to_string(),
            times,
            values,
            noise,
            weight: 1.0,
            after_change: None,
            likelihood: None,
            reduction: None,
            dimension: None,
        }],
    }
}

/// The fit of `k` and `y` against one measurement at the start, which tells `y` and says
/// nothing about `k`: the one case of a parameter the data does not determine.
pub fn one_point(scale: Scale) -> Definition {
    let mut definition = pair(0.5, 2.0, scale);
    definition.curves[0].times = vec![0.0];
    definition.curves[0].values = vec![1.0];
    definition.curves[0].noise = Some(vec![NOISE]);
    definition
}

/// Asserts that a run of the chains of a piece of work took less than the chains themselves,
/// which is what it takes when they run one after the other.
///
/// A chain is work whose steps wait for each other, a side of a profile or a run of a start,
/// and the seconds of the chains are the ones their own steps report. A run of them next to
/// each other takes the longest chain of them and a run of them one after the other all of
/// them, so the assertion is that at least half of what the shorter ones add is gone. That
/// holds whatever the machine of the test is: a pool of fewer threads than chains stretches
/// the seconds of every chain which shares a thread as much as it stretches the whole run.
pub fn next_to_each_other(took: f64, chains: &[f64], what: &str) {
    let all: f64 = chains.iter().sum();
    let longest = chains.iter().copied().fold(0.0, f64::max);
    assert!(
        took < all - 0.5 * (all - longest),
        "{what} took {took:.4} s, its {} chains {all:.4} s of the threads \
         and the longest of them {longest:.4} s",
        chains.len()
    );
}

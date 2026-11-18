//! A scan of one parameter on a pool of threads: `cargo run --example a_scan` in a directory
//! with a `model.xml` which has a parameter `k`.

use std::sync::Arc;

use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest};
use sbmlsim_batch::{WorkerPool, thread_count};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

fn main() -> anyhow::Result<()> {
    let sbml = std::fs::read_to_string("model.xml")?;
    let compiled = compile(&sbml, &CompileOptions::default())?;

    let runtime = Runtime::embedded()?;
    let model = Arc::new(runtime.load(&compiled.wasm, &compiled.description)?);

    let request = SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: 10.0,
        output: Output::Uniform { steps: 100 },
        selections: vec!["time".to_string(), "[S]".to_string()],
        settings: IntegratorSettings::default(),
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        seed: 0,
    };
    // the request is validated and resolved once; a point of the scan is the value of `k`
    let prepared = model.prepare(&request, &["k".to_string()])?;
    let points: Vec<Vec<f64>> = (1..=64).map(|step| vec![f64::from(step) * 0.1]).collect();

    let mut pool = WorkerPool::new(model, thread_count(None)?);
    pool.ensure(points.len())?;
    let workers = pool.workers().expect("the workers of a pool which grew");
    for (point, outcome) in points
        .iter()
        .zip(workers.simulate_prepared(&prepared, &points))
    {
        match outcome {
            Ok(table) => println!("k = {}: {:?}", point[0], table.values.last()),
            // a point which fails leaves the points around it their results
            Err(failure) => println!("k = {}: {failure}", point[0]),
        }
    }
    Ok(())
}

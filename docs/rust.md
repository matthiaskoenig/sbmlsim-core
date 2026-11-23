# Rust

Rust is the language sbmlsim-core is written in, so it is also the binding with nothing in between: the compiler (`sbmlsim-compiler`) turns SBML into `model.wasm` and its description, and the host (`sbmlsim-host`) links that model with `engine.wasm` and `mathlib.wasm` in wasmtime and simulates it. The [Python](python.md) and [JavaScript](javascript.md) bindings are these two crates behind an extension module and an ES module.

The crates are built from this repository and version together with it, so depend on them by git. `sbmlsim-sbml`, the reader of SBML, is the one exception: it holds nothing but its own sources and is published on [crates.io](https://crates.io/crates/sbmlsim-sbml), so a project which reads SBML without simulating it needs none of the rest.

```toml
[dependencies]
sbmlsim-abi = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.24.0" }
sbmlsim-compiler = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.24.0" }
sbmlsim-host = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.24.0" }
sbmlsim-batch = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.24.0" }
anyhow = "1"

# the reader alone, from crates.io
sbmlsim-sbml = "0.21"
```

`sbmlsim-abi` carries the two contracts as types, which the compiler and the host both speak.

Building the host compiles `engine.wasm` from source, which needs the wasm32-wasip1 target and a WASI sysroot; [Installation](installation.md) has the toolchain.

## A first simulation

`Runtime::embedded()` takes the `engine.wasm` and `mathlib.wasm` built into the crate, from wasmtime's compilation cache or compiling them on a miss. A `Runtime` and a `LoadedModel` are shared between threads; an `Instance` is one simulation at a time, so give each thread its own.

```rust
use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

fn main() -> anyhow::Result<()> {
    let sbml = std::fs::read_to_string("model.xml")?;
    let compiled = compile(&sbml, &CompileOptions::default())?;

    let runtime = Runtime::embedded()?;
    let model = runtime.load(&compiled.wasm, &compiled.description)?;
    let mut instance = model.instantiate()?;

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
        steady_state_after: None,
        before_events: false,
        seed: 0,
        sensitivities: vec![],
    };
    let result = instance.simulate(&request)?;

    println!("{}", result.columns.join(","));
    for row in result.values.chunks(result.columns.len()) {
        println!("{row:?}");
    }
    Ok(())
}
```

This is `crates/host/examples/first_simulation.rs`, which `cargo clippy --all-targets` builds, so the example above cannot drift from the API. Run it with `cargo run --example first_simulation`.

`result.values` is row major with one column per selection, the same layout the other two bindings return. `instance.stats()` gives the counters of the integrator (steps, right hand side evaluations, Jacobian evaluations, nonlinear iterations, error test failures), which are the same in every host for the same request.

## A batch of simulations

A scan, the points of a finite difference Jacobian and the population of an optimizer are the same work: many simulations of one model which do not depend on each other. `sbmlsim-batch` runs them on a pool of threads, each on an engine instance of its own, made at the first point of the thread and reused by every point after it, so the results of a batch are those of a serial run bitwise, whatever the threads are, and the cost of an instance (0.33 ms for a model of 700 kB of description) is paid once per thread instead of once per point.

```rust
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
        steady_state_after: None,
        before_events: false,
        seed: 0,
        sensitivities: vec![],
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
```

This is `crates/batch/examples/a_scan.rs`, which `cargo clippy --all-targets` builds, so the example above cannot drift from the API either. Run it with `cargo run --example a_scan`.

`Workers::new` is a pool for one batch and `WorkerPool` keeps one across batches, grown to the points of a batch and never shrunk, which is what a caller of many small batches needs. `Workers::map` is the batch of any work which is more than a simulation: the whole work of a point runs on the thread of the point, so a caller which computes the residuals of an objective or the rows of a table does that there as well, and gives one `Outcome` per point in the order of the points. A point which fails, and a point which panics, fails alone: `Outcome` is a result per point, so a batch is never ended by one of its points. `Workers::of_models` is a pool of several models, whose thread holds an instance of every model its points met, and `Workers::map_at` routes a point to its model, which is what a fit of several models runs on.

The pool is the one the [Python](python.md) bindings run `simulate_many` and `batch_executor` on.

## The pieces

| crate | what it is |
|---|---|
| `sbmlsim-abi` | the contracts `model/2` and `simulation/1` as Rust types |
| `sbmlsim-compiler` | SBML to `model.wasm` and `model.json` (over `sbml`, `ir` and `codegen`) |
| `sbmlsim-engine` | the integrator, compiled to `engine.wasm` (CVODE, KLU) |
| `sbmlsim-host` | the wasmtime host: `Runtime`, `LoadedModel`, `Instance` |
| `sbmlsim-batch` | the pool of a batch of simulations: a thread per point, an instance per thread |
| `sbmlsim-csolvers` | CVODE and SuiteSparse, built from source for the engine and for the native host |
| `sbmlsim-cli` | the command line `sbmlsim-core` |
| `sbmlsim-sbml` | the reader of SBML core, every level and version, on crates.io |

`Runtime::new(engine_wasm, mathlib_wasm)` takes other builds of the two, `Runtime::with_max_memory` and `Runtime::with_execution_timeout` bound an instance, and `Instance::wasi_calls` counts the calls into the host, which is zero while a model loads and simulates. The exports of `engine.wasm` are documented in `crates/engine/src/exports.rs`.

## Without WebAssembly

`NativeInstance` is the same engine compiled for the machine instead of to wasm, the host the [benchmarks](benchmarks.md) call `native` and the one the python wheel runs. It answers the same requests; the model is still the `model.wasm` wasmtime runs, so results can differ from `engine.wasm` in the last bits (see [Python](python.md#the-engine-of-the-wheel)).

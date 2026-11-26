# Rust

Rust is the language sbmlsim-core is written in, so it is also the binding with nothing in between: the compiler (`sbmlsim-compiler`) turns SBML into `model.wasm` and its description, and the host (`sbmlsim-host`) links that model with `engine.wasm` and `mathlib.wasm` in wasmtime and simulates it. The [Python](python.md) and [JavaScript](javascript.md) bindings are these two crates behind an extension module and an ES module.

The crates are built from this repository and version together with it, so depend on them by git. `sbmlsim-sbml`, the reader of SBML, is the one exception: it holds nothing but its own sources and is published on [crates.io](https://crates.io/crates/sbmlsim-sbml), so a project which reads SBML without simulating it needs none of the rest.

```toml
[dependencies]
sbmlsim-abi = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.29.0" }
sbmlsim-compiler = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.29.0" }
sbmlsim-host = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.29.0" }
sbmlsim-batch = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.29.0" }
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

Set `CompileOptions { parameter_jacobian: false, ..CompileOptions::default() }` for ordinary
simulations that do not need analytic parameter derivatives. This omits their compilation and
cached constants; forward sensitivities retain numerical parameter derivatives. The default is
`true` (0.26.0).

This is `crates/host/examples/first_simulation.rs`, which `cargo clippy --all-targets` builds, so the example above cannot drift from the API. Run it with `cargo run --example first_simulation`.

`result.values` is row major with one column per selection, the same layout the other two bindings return. `instance.stats()` gives the statistics of the integrator (steps, right hand side evaluations, Jacobian evaluations, nonlinear iterations, error test failures, and the orders, steps and internal time the solver ended at), which are the same in every host for the same request.

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
| `sbmlsim-c` | the C target: the model as native code for the engine, compiled with the C compiler of the machine |
| `sbmlsim-host` | the wasmtime host: `Runtime`, `LoadedModel`, `Instance` |
| `sbmlsim-batch` | the pool of a batch of simulations: a thread per point, an instance per thread |
| `sbmlsim-chaos` | the basis of a polynomial chaos expansion: its multi-indices, its design matrix and its surrogate |
| `sbmlsim-csolvers` | CVODE and SuiteSparse, built from source for the engine and for the native host |
| `sbmlsim-cli` | the command line `sbmlsim-core` |
| `sbmlsim-sbml` | the reader of SBML core, every level and version, on crates.io |

`Runtime::new(engine_wasm, mathlib_wasm)` takes other builds of the two, `Runtime::with_max_memory` and `Runtime::with_execution_timeout` bound an instance, and `Instance::wasi_calls` counts the calls into the host, which is zero while a model loads and simulates. The exports of `engine.wasm` are documented in `crates/engine/src/exports.rs`.

## Without WebAssembly

`NativeInstance` is the same engine compiled for the machine instead of to wasm, the host the [benchmarks](benchmarks.md) call `native` and the one the python wheel runs. It answers the same requests; the model is still the `model.wasm` wasmtime runs, so results can differ from `engine.wasm` in the last bits (see [Python](python.md#the-engine-of-the-wheel)). The [C target](#the-c-target) runs without WebAssembly at all.

## The C target

`sbmlsim-c` runs a model without WebAssembly at all: the engine compiled for the machine with the model as native code. `generate` translates the SBML into C, the functions of the contract `model/2` (`defaults`, `init`, `rhs`, `jac`, `roots`, `trigger`, `priority`, `delay`, `event_values`, `event_apply`, `selections`, `assign`) with a prefix of the model (`sbmlsim_model_<id>_`), each a pure function of `t`, pointers to `y` and `p` and its outputs, without globals, and the description the WebAssembly target gives the model. `Library::build` compiles the C into a shared library and loads it, and `CModel` is its functions as the `ModelFns` of `sbmlsim_engine::Engine`; a clone of a `CModel` is a reference count, one per engine.

```rust
use sbmlsim_c::{CModel, Library, generate};
use sbmlsim_engine::Engine;

let source = generate(&std::fs::read_to_string("model.xml")?)?;
let library = Library::build(&source.code)?;
let model = CModel::new(library, &source.description)?;
let mut engine = Engine::new(Box::new(model), source.description.clone())?;
let result = engine.simulate(&request)?;
```

The compiler is `$CC`, else `cc`, with `-O3 -march=native -ffp-contract=off -fno-fast-math -fPIC -shared`: the processor of the machine, but no fused multiply adds and no fast math, so every operation is the IEEE operation the C writes, in the order of `model.wasm`. The libraries are cached in `$SBMLSIM_C_CACHE`, else `$XDG_CACHE_HOME/sbmlsim/c`, else `~/.cache/sbmlsim/c`, as `<key>.so` (`.dylib` on macOS) next to their source `<key>.c`; the key is the SHA-256 of the flags, the `--version` line of the compiler, the macros it predefines under the flags (`cc -dM -E` with them: the features `-march=native` selects, which make the key stricter than the version alone, so a cache shared by machines of different processors keeps a library for each and never loads one built for another processor) and the source. A library is compiled into a temporary file and renamed into place, so the processes of a parallel run which build the same model never load a partial one, and a library a process has loaded is shared by its engines. `CError` names what failed: `Unsupported { feature }`, `NoCompiler { command }`, `Compiler { command, stderr }`, `Load` and `Model`.

The C reads the math functions of `<math.h>` and not those of mathlib, so its right hand side and Jacobian agree with those of `model.wasm` to the last bits of the math library (within `1e-13` relative in the tests) and a simulation within the tolerances of the integration, not bitwise. The cell model of `testdata/models` (7 states, 8 reactions) takes 0.23 ms a simulation of 1440 min on the native engine with its C, against 0.59 ms on `engine.wasm` in wasmtime; its C compiles in 0.12 s once, after which the cache loads it.

Refused (`CError::Unsupported` with the feature): a model which reads its history (`delay`), and a model without an analytic Jacobian (`factorial` of a state), a known limitation: the engine would take the colored difference quotient of its right hand side, which needs no `jac`, but the C target asks for the analytic Jacobian of every model it compiles. Of the SBML Test Suite (`sbmlsim-core testsuite testdata/cases --model-code c`, [Test suite](testsuite.md)) every case of the baseline passes but the 53 delay models it refuses, pinned in `testdata/testsuite_refused_c.json`: 00937-00943, 00981-00985, 01142, 01173-01174, 01176, 01318-01320, 01400-01401, 01403-01404, 01406-01407, 01409-01419, 01454, 01480-01481, 01518-01524, 01534-01538 and 01592-01593. No model of the suite lacks an analytic Jacobian but those.

## A stepper

`sbmlsim_engine::Stepper` is an instance of a model which is advanced from a given state, one interval at a time, for a caller which couples the model to something else and changes its state between two intervals (operator splitting: the transport of species between many instances of a cell model). `Stepper::new` initializes the model as a simulation does, with `changes` as the pre-initialization values of a request (`S` the amount of a species, `[S]` its concentration, any other id its value), and creates its integrator memory once; the linear solver is the one of the settings (`Auto`: dense up to `KLU_THRESHOLD` states, KLU above). The description is an `Arc`, which the steppers of one model share.

```rust
use sbmlsim_engine::Stepper;

let mut stepper = Stepper::new(Box::new(model), description, settings, &[("k".into(), 0.5)])?;
let mut y = stepper.state().to_vec();
for k in 0..1440 {
    y[2] = 0.0; // what the caller changes
    stepper.advance(&y, k as f64, (k + 1) as f64)?;
    y.copy_from_slice(stepper.state());
}
```

`advance(y, t0, t1)` starts the integrator again from `y` at `t0` and integrates to `t1` as a simulation does, with the events and the discontinuities of the interval: the triggers keep their values from one advance to the next, an execution of an event with a delay stays pending across the end of an interval, the parameters an event assigns stay assigned, and an event whose trigger the changed state makes true executes at the start of the interval. The state after an advance is `state()`, the state after the events at `t1`, as the last row of a simulation is. Its first step is the step the integrator ended the advance before with, its step control is its own and depends on no other instance. `StepStats` counts the steps of the advance and the attempts of a step the integrator rejected. A refused call (a state of the wrong length or not finite, an interval which is not finite or ends before it starts) changes nothing; an advance which fails after that leaves the stepper failed, as its events and parameters may have changed on the way. A model with delays is refused.

`memory_bytes` estimates the memory of an instance: for the cell model of `testdata/models` (7 states) 19.0 kB after the first advance, within 1% of the resident memory of 10000 steppers, most of it the objects of SUNDIALS (each of its vectors carries its own table of operations, 0.6 kB for 56 bytes of values). An advance of that model at the tolerances 1e-10 and 1e-12 takes 10 to 13 µs for 1 min (17 steps), 15 to 21 µs for 10 min and 32 to 42 µs for 60 min on one core, and a stepper 8 to 11 µs to create (`cargo test --release -p sbmlsim-engine --test stepper -- --ignored --nocapture`).

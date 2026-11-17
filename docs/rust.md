# Rust

Rust is the language sbmlsim-core is written in, so it is also the binding with nothing in between: the compiler (`sbmlsim-compiler`) turns SBML into `model.wasm` and its description, and the host (`sbmlsim-host`) links that model with `engine.wasm` and `mathlib.wasm` in wasmtime and simulates it. The [Python](python.md) and [JavaScript](javascript.md) bindings are these two crates behind an extension module and an ES module.

The crates are not published on crates.io; they are built from this repository and version together with it. Depend on them by git:

```toml
[dependencies]
sbmlsim-abi = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.9.0" }
sbmlsim-compiler = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.9.0" }
sbmlsim-host = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.9.0" }
anyhow = "1"
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
        seed: 0,
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

## The pieces

| crate | what it is |
|---|---|
| `sbmlsim-abi` | the contracts `model/2` and `simulation/1` as Rust types |
| `sbmlsim-compiler` | SBML to `model.wasm` and `model.json` (over `sbml`, `ir` and `codegen`) |
| `sbmlsim-engine` | the integrator, compiled to `engine.wasm` (CVODE, KLU) |
| `sbmlsim-host` | the wasmtime host: `Runtime`, `LoadedModel`, `Instance` |
| `sbmlsim-csolvers` | CVODE and SuiteSparse, built from source for the engine and for the native host |
| `sbmlsim-cli` | the command line `sbmlsim-core` |

`Runtime::new(engine_wasm, mathlib_wasm)` takes other builds of the two, `Runtime::with_max_memory` and `Runtime::with_execution_timeout` bound an instance, and `Instance::wasi_calls` counts the calls into the host, which is zero while a model loads and simulates. The exports of `engine.wasm` are documented in `crates/engine/src/exports.rs`.

## Without WebAssembly

`NativeInstance` is the same engine compiled for the machine instead of to wasm, the host the [benchmarks](benchmarks.md) call `native` and the one the python wheel runs. It answers the same requests; the model is still the `model.wasm` wasmtime runs, so results can differ from `engine.wasm` in the last bits (see [Python](python.md#the-engine-of-the-wheel)).

# Rust

Rust is the language sbmlsim-core is written in, so it is also the binding with nothing in between: the compiler (`sbmlsim-compiler`) turns SBML into `model.wasm` and its description, and the host (`sbmlsim-host`) links that model with `engine.wasm` and `mathlib.wasm` in wasmtime and simulates it. The [Python](python.md) and [JavaScript](javascript.md) bindings are these two crates behind an extension module and an ES module.

The crates are built from this repository and version together with it, so depend on them by git. `sbmlsim-sbml`, the reader of SBML, is the one exception: it holds nothing but its own sources and is published on [crates.io](https://crates.io/crates/sbmlsim-sbml), so a project which reads SBML without simulating it needs none of the rest.

```toml
[dependencies]
sbmlsim-abi = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.33.0" }
sbmlsim-compiler = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.33.0" }
sbmlsim-host = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.33.0" }
sbmlsim-batch = { git = "https://github.com/matthiaskoenig/sbmlsim-core", tag = "v0.33.0" }
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
| `sbmlsim-c` | the C target: the model as native code for the engine, compiled with the C compiler of the machine; and the QFunction target: the model with a Rosenbrock integrator as a libCEED QFunction |
| `sbmlsim-host` | the wasmtime host: `Runtime`, `LoadedModel`, `Instance` |
| `sbmlsim-batch` | the pool of a batch of simulations: a thread per point, an instance per thread |
| `sbmlsim-chaos` | the basis of a polynomial chaos expansion: its multi-indices, its design matrix and its surrogate |
| `sbmlsim-thin` | the thinning of a line of a figure to the points which can be seen, Ramer-Douglas-Peucker, the core of `sbmlsim_core.chart.thinning` |
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

`sbmlsim_engine::Stepper` is an instance of a model which is advanced from a given state, one interval at a time, for a caller which couples the model to something else and changes its state between two intervals (operator splitting: the transport of species between many instances of a cell model). `Stepper::new` initializes the model as a simulation does, with `changes` as the pre-initialization values of a request (`S` the amount of a species, `[S]` its concentration, any other id its value), and creates its integrator memory once; the linear solver is the one of the settings (`Auto`: dense up to `KLU_THRESHOLD` states, KLU above). The description is an `Arc`, which the steppers of one model share. `state()` and `parameters()` (without the constants behind them) are the initial values after `new`, which a caller can also hand to the [QFunction target](#the-qfunction-target).

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

`advance(y, t0, t1)` starts the integrator again from `y` at `t0` and integrates to `t1` as a simulation does, with the events and the discontinuities of the interval: the triggers keep their values from one advance to the next, an execution of an event with a delay stays pending across the end of an interval, the parameters an event assigns stay assigned, and an event whose trigger the changed state makes true executes at the start of the interval. The state after an advance is `state()`, the state after the events at `t1`, as the last row of a simulation is. Its first step is the one of a simulation (the estimate of CVODE or IDA, or the initial step of the settings), its step control is its own and depends on no other instance; starting with the step the advance before ended with was measured worse (a step of a higher order fails the error test of the first order the integrator starts again with, which doubled the rejected steps and saved no time). `StepStats` counts the steps of the advance and the attempts of a step the integrator rejected. A refused call (a state of the wrong length or not finite, an interval which is not finite or ends before it starts) changes nothing; an advance which fails after that leaves the stepper failed, as its events and parameters may have changed on the way. A model with delays is refused.

`memory_bytes` estimates the memory of an instance: for the cell model of `testdata/models` (7 states) 18.9 kB after the first advance, within 1% of the resident memory of 10000 steppers, most of it the objects of SUNDIALS (each of its vectors carries its own table of operations, 0.6 kB for 56 bytes of values). An advance of that model at the tolerances 1e-10 and 1e-12 takes 7.7 µs for 1 min (16.5 steps), 13.6 µs for 10 min and 30.5 µs for 60 min on one core, and a stepper 7 µs to create (`cargo test --release -p sbmlsim-engine --test stepper -- --ignored --nocapture`).

## The QFunction target

`sbmlsim_c::generate_qfunction` writes a model with a Rosenbrock integrator as one [libCEED](https://libceed.org) QFunction, which advances any number of points (the cells of a tissue, one per element of a mesh), each by step attempts until it reaches the end of the interval or fails, at most a number of attempts per application, on libCEED's CPU backends now and, as its source is plain C99 for its JIT, on its GPU backends later. Its source (`QSource::code`) includes `<ceed/types.h>` and `<math.h>` (the latter guarded by `CEED_RUNNING_JIT_PASS`, as libCEED asks), then the constants, the right hand side, the dense Jacobian and, for a model whose rates depend on the time, their derivative in the time, from the programs of the emitter of the C target as `CEED_QFUNCTION_HELPER` functions, then the integrator, written once for every model in `crates/c/include/sbmlsim_rosenbrock.h`, and the QFunction `QSource::function` (`sbmlsim_model_<id>_qfunction`) of libCEED's interior signature. No allocation, no call outside the model and `<math.h>`, no function pointer: every work array of a point has a size the model fixes at compile time.

| | field | values per point |
|---|---|---|
| input | `state` | the states, `QSource::state_ids` |
| input | `parameters` | the parameters, `QSource::parameter_ids` (the QFunction computes the constants behind them) |
| input | `control` | `t`, `h`, `done`, `failed` (`CONTROL_T`, `CONTROL_H`, `CONTROL_DONE`, `CONTROL_FAILED`) |
| input | `counts` | the accepted steps and the rejected attempts so far |
| output | `state_out`, `control_out`, `counts_out` | as their inputs |

The context is `struct SbmlsimRosenbrockContext { double t1; double rtol; double atol; double h_min; double attempts; }` (`RosenbrockContext` in Rust), in the time unit of the model. An application makes step attempts for every point which is neither done nor failed, until it is done or failed but at most `attempts` (a whole number, at least 1), and copies the others, so the sequence of steps of a point depends on nothing but the point: not on the other points, how many a call gets or in which order, the backend, the number of processes or the attempts per application. The caller applies the QFunction until every point is done or failed, the outputs becoming the inputs: with as many attempts as a point takes (on a CPU) once, which reads and writes the fields of the points once per interval; with one attempt per application (a call of bounded work) as often as the point which takes the most attempts (`counts` is an input as well so that it can be cumulative, which an output of libCEED cannot be). A point is done when its `t` reaches `t1`, and failed when the step it would take next is below `h_min` (or no longer moves `t`); it keeps the state of its last accepted step. Its `h` carries over to the next interval; a point without one (`h <= 0`) starts with the estimate of Hairer and Wanner (`HINIT`, one explicit Euler step).

The method is **RODAS4** (Hairer and Wanner, *Solving Ordinary Differential Equations II*, section VI.4, with the coefficients and the stage formulation of their `rodas.f`): order 4 with 6 stages and an embedded solution of order 3, both stiffly accurate and L-stable, so its error estimate stays sound on the stiff components a cell model has, and one LU decomposition with partial pivoting of `1/(h gamma) I - J` per step. ROS3P is not L-stable, and RODAS3 is of order 3 with an embedded solution of order 2, which take more steps at the tolerances of a tissue (1e-6); the order 4 of RODAS4 is measured (`order_of_the_method`: 4.00 on a linear equation with a rate of the time). Like other Rosenbrock methods of its kind it loses order on the Prothero-Robinson problem `y' = -k (y - g(t)) + g'(t)` of a large `k h`. The step control is the one of `rodas.f` without its memory across steps (a point carries only `t` and `h`): the error is the root mean square of the embedded estimate scaled by `atol + rtol max(|y|, |y_new|)`, a step is accepted at an error of at most 1 with every new state finite, the next step is `h / max(1/6, min(5, err^(1/4) / 0.9))`, a singular matrix halves it, and a step which reaches `t1` within `1.0001 h` ends exactly there. After such a last step the step a point keeps is the one before the shortening, divided by the factor where the error asks for a smaller one (or the step of the error where that is larger): a short last step, the usual case where every interval starts again, does not shrink the step of the next interval below `h_min`.

```rust
use sbmlsim_c::{Library, Points, RosenbrockContext, generate_qfunction, run_points};

let q = generate_qfunction(&sbml)?;
let library = Library::build(&q.host_code())?;
// states and parameters point by point, e.g. those of a Stepper::new per point
let mut points = Points::new(&q, &states, &parameters)?;
for k in 0..24 {
    points.start(60.0 * k as f64);
    // as many attempts per application as a point takes: one application per interval
    let context = RosenbrockContext { t1: 60.0 * (k + 1) as f64, rtol: 1e-6, atol: 1e-9, h_min: 1e-10, attempts: 1e6 };
    run_points(&library, &q, &context, &mut points, 8)?;
}
```

The source compiles under libCEED unchanged: a CPU backend calls the function the compiled library exports as `QSource::host_symbol()` (the `CEED_QFUNCTION` itself is static, as libCEED defines it), a JIT backend reads the source and the function `QSource::function`. `host_code()` is the source with the definitions of `<ceed/types.h>` it reads in place of the include (`CEED_QFUNCTION`, `CEED_QFUNCTION_HELPER`, `CeedInt`, `CeedScalar`, as libCEED v1.1.2 defines them), which `Library::build` compiles without libCEED, with the flags and the cache of the C target. `run_points` and `apply_points` call that library as libCEED calls a QFunction, every field `[size][Q]` for `Q` points per call (a block), until every point is done, or once. The source compiles under `-std=c99 -pedantic -Wall -Wextra -Werror` with both (`sources_compile_without_warnings`, with libCEED's own headers where `SBMLSIM_LIBCEED_INCLUDE` names them). Inside the JIT pass, where libCEED drops `<math.h>` and NVRTC and HIPRTC define no `INFINITY` and `NAN`, the source defines the two as `__builtin_huge_val()` and `__builtin_nan("")` for the math of the model; the integrator reads neither. `sources_compile_in_the_jit_pass` compiles it as a JIT backend sees it: C++17 with `CEED_RUNNING_JIT_PASS`, the macros of libCEED's `cuda-jit.h`, the math functions of the device and no system header.

On the cell model of `testdata/models` with its 50 parameter sets, 24 intervals of 60 min at 1e-6 and 1e-9, every state at the end of every interval is within `2e-6 |y| + 2e-9` of a stepper at 1e-12 and 1e-14 (the largest difference half of it), with 7.8 steps and 0.16 rejected attempts per point and interval; blocks of 1, 7 and 64 points, in order and shuffled, with 1, 3 or as many step attempts per application as a point takes, give bitwise the same states, controls and counts. One core of an Intel i7-12700 (`cargo run --release -p sbmlsim-c --example qfunction_performance`, 1000 points, a point-step being one interval of 60 min of one point, both paths at 1e-6 and 1e-9):

| path | point-steps/s | µs/point-step | steps/point-step | rejected/point-step |
|---|---:|---:|---:|---:|
| QFunction, RODAS4, block 1, as many attempts as a point takes | 182735 | 5.47 | 7.76 | 0.156 |
| QFunction, RODAS4, block 8, one attempt per application | 167410 | 5.97 | 7.76 | 0.156 |
| QFunction, RODAS4, block 8, as many attempts as a point takes | 181925 | 5.50 | 7.76 | 0.156 |
| Stepper, CVODE BDF, `CModel` | 90623 | 11.03 | 27.61 | 1.771 |

A point is a few dozen numbers (its states, parameters, control and counts) against the 19 kB of a stepper. The arithmetic of a point is scalar: the pivoting and the step control of each point take their own branches, so the compiler does not vectorize across the points of a block.

Refused (`CError::Unsupported`, naming the feature): events, delays, algebraic rules, a model without states and a model without an analytic Jacobian; the engine (a `Stepper` per point) integrates those. The discontinuities of the math (a `piecewise` of a state) need no root search: the step control steps across them, at the cost of rejected attempts there.

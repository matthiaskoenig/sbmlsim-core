# sbmlsim-core

sbmlsim-core is an SBML to WebAssembly compiler and a CVODE/KLU simulation engine, written in Rust. It compiles an SBML Level 3 Version 2 core model into a WebAssembly module and simulates it with the same `engine.wasm` in wasmtime, in Node and in browsers, with bitwise identical results on every host.

It is the simulation core of the web apps of [sbmlsim](https://matthiaskoenig.github.io/sbmlsim/): the semantics of a simulation are those of sbmlsim (PEtab v2), the integrator and the linear solvers those of libroadrunner (CVODE of SUNDIALS, KLU of SuiteSparse).

See it run in the [glimepiride app](https://matthiaskoenig.github.io/sbmlsim-web/examples/glimepiride/) of the sbmlsim-web documentation: the whole-body PBPK model of glimepiride, its panels drawn by sbmlsim-web and every simulation run in the browser of the reader on a pool of workers ([the panel endpoint](javascript.md#the-panel-endpoint)).

## The program

A model passes through three parts, joined by two contracts which are defined in the crate `sbmlsim-abi` (`crates/abi`):

```text
model.xml  --compiler-->  model.wasm + model.json   (model/2)
                                 |
                                 v
request    --engine.wasm + mathlib.wasm in a host-->  result   (simulation/1)
```

**The compiler.** `crates/compiler`, built on `sbml` (the reader of SBML L3V2 core and its MathML), `ir` (the model representation: the states and parameters in the order of libroadrunner's vectors, the rules in their order of dependency, the events, the analytic Jacobian) and `codegen` (the WebAssembly module). It turns an SBML file into `model.wasm`, with an analytic sparse Jacobian, and `model.json`, its description (`model/1`). The compiler does no file system or OS access, so it also builds for `wasm32-unknown-unknown` and can run in a browser. See [Compiling a model](compiling.md).

**The engine.** `crates/engine`, built into `engine.wasm` (wasm32-wasip1): Rust with CVODE 7.9 and KLU (`crates/csolvers`). It runs a request of `simulation/1` on a compiled model: the pre-initialization, changes at times, events, a steady state before the start, and the dense or the KLU linear solver (KLU from 16 states, `KLU_THRESHOLD`). `mathlib.wasm` (`crates/mathlib`) is the one implementation of every math function a model calls, so a result does not depend on the math of a host. See [Simulating](simulating.md).

**The hosts.** They link `mathlib.wasm`, a `model.wasm` and `engine.wasm` into one instance, so every call between them is a call from wasm to wasm: wasmtime (`crates/host`, the command line `sbmlsim-core` of `crates/cli`) and JavaScript (`js/loader.mjs`, for Node and browsers). On top of the JavaScript loader, `js/pool.mjs` runs simulations in a pool of Web Workers and `js/panels.mjs` answers the routes of the panel service of sbmlsim inside a page, so an app of interactive panels is a static page. See [JavaScript](javascript.md).

## Contracts

- `model/2`: the description and the exports of a compiled model (`model.json`, `model.wasm`); `model/1` was the contract of 0.1.0, a model compiled by 0.1.0 is compiled again.
- `simulation/1`: the request and the result of a simulation.

Both are documented on [Simulating](simulating.md) and defined in `crates/abi`.

## What it is measured against

- The SBML Test Suite: 1581 of 1789 cases pass, every case which sbmlsim passes among them; the 208 failures are refused at compilation with a reason. See [Test suite and benchmark](testsuite.md).
- roadrunner: no model of the benchmark takes more than 1.4x the time of roadrunner native, dense or with KLU, in wasmtime, Node or Chrome (the PBPK models 0.57 to 0.83 of it, Chen with KLU 0.16 to 0.19), and every model agrees with roadrunner at tight tolerances.
- sbmlsim: the known differences to roadrunner and to sbmlsim are listed on [Semantics](semantics.md).

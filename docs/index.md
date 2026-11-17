# sbmlsim-core

[![ci](https://github.com/matthiaskoenig/sbmlsim-core/actions/workflows/ci.yml/badge.svg)](https://github.com/matthiaskoenig/sbmlsim-core/actions/workflows/ci.yml)
[![docs](https://github.com/matthiaskoenig/sbmlsim-core/actions/workflows/docs.yml/badge.svg)](https://matthiaskoenig.github.io/sbmlsim-core/)
[![PyPI](https://img.shields.io/pypi/v/sbmlsim-core.svg?logo=pypi&logoColor=white&label=PyPI)](https://pypi.org/project/sbmlsim-core/)
[![npm](https://img.shields.io/npm/v/sbmlsim-core.svg?logo=npm&logoColor=white&label=npm)](https://www.npmjs.com/package/sbmlsim-core)
[![python](https://img.shields.io/pypi/pyversions/sbmlsim-core.svg?logo=python&logoColor=white&label=python)](https://pypi.org/project/sbmlsim-core/)
[![license](https://img.shields.io/badge/license-MIT%20AND%20BSD--3--Clause%20AND%20LGPL--2.1--or--later-blue.svg)](licenses.md)

sbmlsim-core is an SBML to WebAssembly compiler and a CVODE/IDA simulation engine with KLU, written in Rust. It compiles an SBML Level 3 Version 2 core model into a WebAssembly module and simulates it with the same `engine.wasm` in wasmtime, in Node and in browsers, with bitwise identical results on every host.

It is the simulation core of the web apps of [sbmlsim](https://matthiaskoenig.github.io/sbmlsim/): the semantics of a simulation are those of sbmlsim (PEtab v2), the integrator and the linear solvers those of libroadrunner (CVODE of SUNDIALS, KLU of SuiteSparse).

Try it here: the [live repressilator](javascript.md#a-live-simulation-in-this-page) on the JavaScript page is compiled and integrated in your browser as you move its slider, by the same engine the command line and the python wheel run.

See it run in the [glimepiride app](https://matthiaskoenig.github.io/sbmlsim-web/examples/glimepiride/) of the sbmlsim-web documentation: the whole-body PBPK model of glimepiride, its panels drawn by sbmlsim-web and every simulation run in the browser of the reader on a pool of workers ([the panel endpoint](javascript.md#the-panel-endpoint)).

## The program

A model passes through three parts, joined by two contracts which are defined in the crate `sbmlsim-abi` (`crates/abi`):

<svg class="core-diagram" viewBox="0 16 840 350" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="A model passes the compiler into model.wasm and model.json, the contract model/2. A host links mathlib.wasm, model.wasm and engine.wasm into one WebAssembly instance, where the engine calls the model and the model calls mathlib without leaving WebAssembly. A request of the contract simulation/1 goes in and a result comes out. The hosts are wasmtime with the command line, Node, a browser, and the native engine of the python wheel.">
  <defs>
    <marker id="cd-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor"/>
    </marker>
    <marker id="cd-arrow-accent" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--cd-accent)"/>
    </marker>
  </defs>
  <!-- the compiler -->
  <g class="cd-step">
    <rect x="16" y="30" width="128" height="46" rx="6"/>
    <text x="80" y="58">model.xml</text>
  </g>
  <line x1="150" y1="53" x2="196" y2="53" marker-end="url(#cd-arrow)"/>
  <g class="cd-step cd-active">
    <rect x="202" y="30" width="138" height="46" rx="6"/>
    <text x="271" y="58">compiler</text>
  </g>
  <line x1="346" y1="53" x2="392" y2="53" marker-end="url(#cd-arrow)"/>
  <g class="cd-artifact">
    <rect x="398" y="24" width="156" height="26" rx="4"/>
    <text x="476" y="42">model.wasm</text>
    <rect x="398" y="56" width="156" height="26" rx="4"/>
    <text x="476" y="74">model.json</text>
  </g>
  <text class="cd-contract" x="572" y="58">model/2</text>
  <!-- the request and the result -->
  <g class="cd-step">
    <rect x="16" y="196" width="128" height="46" rx="6"/>
    <text x="80" y="217">request</text>
    <text class="cd-sub" x="80" y="233">simulation/1</text>
  </g>
  <line x1="150" y1="219" x2="196" y2="219" marker-end="url(#cd-arrow)"/>
  <line x1="694" y1="219" x2="740" y2="219" marker-end="url(#cd-arrow)"/>
  <g class="cd-step">
    <rect x="746" y="196" width="80" height="46" rx="6"/>
    <text x="786" y="217">result</text>
    <text class="cd-sub" x="786" y="233">simulation/1</text>
  </g>
  <!-- the instance -->
  <g class="cd-instance">
    <rect x="202" y="140" width="488" height="158" rx="10"/>
    <text class="cd-title" x="446" y="162">one WebAssembly instance, one linear memory</text>
  </g>
  <g class="cd-module">
    <rect x="226" y="186" width="136" height="54" rx="6"/>
    <text x="294" y="211">engine.wasm</text>
    <text class="cd-sub" x="294" y="227">CVODE, IDA, KLU</text>
    <rect x="394" y="186" width="136" height="54" rx="6"/>
    <text x="462" y="211">model.wasm</text>
    <text class="cd-sub" x="462" y="227">rhs, Jacobian</text>
    <rect x="546" y="186" width="124" height="54" rx="6"/>
    <text x="608" y="211">mathlib.wasm</text>
    <text class="cd-sub" x="608" y="227">exp, pow, ...</text>
  </g>
  <line class="cd-call" x1="368" y1="213" x2="388" y2="213" marker-end="url(#cd-arrow-accent)"/>
  <line class="cd-call" x1="536" y1="213" x2="540" y2="213" marker-end="url(#cd-arrow-accent)"/>
  <text class="cd-note" x="446" y="276">every call wasm to wasm, the host is not entered</text>
  <!-- the hosts -->
  <text class="cd-label" x="16" y="344">hosts</text>
  <g class="cd-host">
    <rect x="76" y="326" width="152" height="30" rx="15"/>
    <text x="152" y="346">wasmtime, the CLI</text>
    <rect x="240" y="326" width="80" height="30" rx="15"/>
    <text x="280" y="346">Node</text>
    <rect x="332" y="326" width="100" height="30" rx="15"/>
    <text x="382" y="346">browser</text>
    <rect x="444" y="326" width="196" height="30" rx="15"/>
    <text x="542" y="346">native, the python wheel</text>
  </g>
  <line x1="446" y1="310" x2="446" y2="322" marker-end="url(#cd-arrow)"/>
</svg>
**The compiler.** `crates/compiler`, built on `sbml` (the reader of SBML L3V2 core and its MathML), `ir` (the model representation: the states and parameters in the order of libroadrunner's vectors, the rules in their order of dependency, the events, the analytic Jacobian) and `codegen` (the WebAssembly module). It turns an SBML file into `model.wasm`, with an analytic sparse Jacobian, and `model.json`, its description (`model/1`). The compiler does no file system or OS access, so it also builds for `wasm32-unknown-unknown` and can run in a browser. See [Compiling a model](compiling.md).

**The engine.** `crates/engine`, built into `engine.wasm` (wasm32-wasip1): Rust with CVODE and IDA 7.9 and KLU (`crates/csolvers`). It runs a request of `simulation/1` on a compiled model: the pre-initialization, changes at times, events, a steady state before the start, and the dense or the KLU linear solver (KLU above 12 states, `KLU_THRESHOLD`). `mathlib.wasm` (`crates/mathlib`) is the one implementation of every math function a model calls, so a result does not depend on the math of a host. See [Simulating](simulating.md).

**The hosts.** They link `mathlib.wasm`, a `model.wasm` and `engine.wasm` into one instance, so every call between them is a call from wasm to wasm: wasmtime (`crates/host`, the command line `sbmlsim-core` of `crates/cli`) and JavaScript (`js/loader.mjs`, for Node and browsers). On top of the JavaScript loader, `js/pool.mjs` runs simulations in a pool of Web Workers and `js/panels.mjs` answers the routes of the panel service of sbmlsim inside a page, so an app of interactive panels is a static page. See [JavaScript](javascript.md).

## Contracts

- `model/2`: the description and the exports of a compiled model (`model.json`, `model.wasm`); `model/1` was the contract of 0.1.0, a model compiled by 0.1.0 is compiled again.
- `simulation/1`: the request and the result of a simulation.

Both are documented on [Simulating](simulating.md) and defined in `crates/abi`.

## What it is measured against

- The SBML Test Suite: 1701 of 1789 cases pass, including 120 algebraic-rule cases. The remaining 88 cases are refused during preparation or compilation. See [Test suite](testsuite.md).
- roadrunner: in the published benchmark before DAE support, no model takes more than 1.4x the time of roadrunner native, dense or with KLU, in wasmtime, Node or Chrome (the PBPK models 0.57 to 0.83 of it, Chen with KLU 0.16 to 0.19), and every model agrees with roadrunner at tight tolerances.
- sbmlsim: the known differences to roadrunner and to sbmlsim are listed on [Semantics](semantics.md).

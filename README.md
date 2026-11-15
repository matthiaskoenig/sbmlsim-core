# sbmlsim-core

An SBML to WebAssembly compiler and a CVODE/KLU simulation engine, written in Rust. The two contracts `model/1` (the description and the exports of a compiled model) and `simulation/1` (the request and the result of a simulation) are defined in `crates/abi`.

Documentation: https://matthiaskoenig.github.io/sbmlsim-core/ (built from `docs/` with Zensical: `uvx --from zensical==0.1.0 zensical build --clean`).

## What the core is

- The compiler (`crates/compiler`, built on `sbml`, `ir` and `codegen`) turns an SBML Level 3 Version 2 core model (flat, no packages) into `model.wasm`, with analytic sparse Jacobians, and `model.json`, its description.
- The engine (`crates/engine`, `engine.wasm`, wasm32-wasip1) is Rust with CVODE 7.9 and KLU. It runs a request on a compiled model: changes, events, steady states, dense or KLU linear solver (`KLU_THRESHOLD` 16 states).
- The hosts run the same `engine.wasm`, `mathlib.wasm` and `model.wasm` with bitwise identical results: wasmtime (`crates/host`, the command line `sbmlsim-core`) and Node and browsers (`js/loader.mjs`).
- It is the simulation core of sbmlsim's web apps; the semantics are those of sbmlsim (PEtab v2), the integrator and the solvers those of roadrunner.

## Contracts

- `model/1`: the description and the exports of a compiled model (`model.json`, `model.wasm`).
- `simulation/1`: the request and the result of a simulation.

Both are defined in `crates/abi`.

## Semantics and known differences

The semantics follow sbmlsim's simulation core (`changes`, `events`, root finding of the conditions of `piecewise`). All known differences to roadrunner and to sbmlsim are documented with the engine (`crates/engine/src/lib.rs`, "Known differences to libroadrunner", and the modules `changes` and `events`) and in `bench/RESULTS.md` ("Known differences to roadrunner"). The ones a user meets first are listed below.

One behavior follows sbmlsim rather than SBML, so it is no difference to sbmlsim: an event with a delay whose trigger a change of the request makes true (also through the events the change executes) does not fire. Its trigger stays armed, so the event fires only after the trigger falls and rises again during the integration. A trigger which the pre-initialization makes true is scheduled as usual.

- Events which accumulate end with an error of the integration (`CV_TOO_MUCH_WORK`, flag -1): the executions of events within the relative tolerance (as a distance in time) of the last root share the budget of `maximum_num_steps` steps towards one output time, and in the output of the steps of the integrator `maximum_num_steps` consecutive stops within the resolution of the time (1e-9 of the magnitude of the times) end the run. The bouncing ball (`x' = v`, `v' = -10`, the event `x < 0`: `v = -0.5 v`, `x = 0`) fails at its accumulation point t = 1.3416 within milliseconds; roadrunner lets the ball fall through and continues.
- Roots which chatter share that budget too, so a model which roadrunner finishes after millions of steps (a sliding mode at a loose tolerance) may end with `CV_TOO_MUCH_WORK` here.
- Three differences to sbmlsim's roadrunner executor: a change which sets a state exactly to the threshold of a strict comparison triggers its event right after the change (as sbmlsim's executors of COPASI and AMICI; roadrunner's CVODE ignores a root which is 0 at the start of a segment); the variable of an assignment rule without math can be changed (the rule is ignored, as roadrunner does; sbmlsim refuses to change it); an entity named `time` is set as an entity by a change (sbmlsim sets the clock of roadrunner).
- A limitation shared with roadrunner: CVODE finds a root where a root function changes its sign across a step, so two roots of one function within one step cancel and are missed. A condition which switches faster than the steps needs `maximum_time_step`, e.g. `piecewise(1, sin(time) > 0, 0)` over many periods with a `maximum_time_step` of 0.5.

## Requirements of the hosts

`engine.wasm` uses fixed-width SIMD (WebAssembly SIMD, the C solvers are compiled with `-msimd128`). The results do not depend on it, they are the same bits as without, but a host without SIMD cannot instantiate `engine.wasm`. wasmtime, Node and the current browsers (Chrome 91, Firefox 89, Safari 16.4 and later) support it. The memory of an instance grows to at most 2 GiB.

## Toolchain

User-level installs only, no system packages.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup target add wasm32-wasip1 wasm32-unknown-unknown
cargo install wasmtime-cli --version 50.0.1 --locked
# wasi-sdk 34 (the version of CI and of the build scripts): unpack
# https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz to ~/opt/wasi-sdk
export WASI_SDK_PATH=$HOME/opt/wasi-sdk
```

## Checks

clippy runs with `-D warnings` and the default lints; the pedantic lints (`clippy::pedantic`) are planned for later and not enabled yet.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy -p sbmlsim-csolvers --all-targets --target wasm32-wasip1 -- -D warnings
cargo clippy -p sbmlsim-engine --all-targets --target wasm32-wasip1 -- -D warnings
cargo clippy -p sbmlsim-engine --lib --features exports --target wasm32-wasip1 -- -D warnings
cargo test
cargo build -p sbmlsim-compiler --target wasm32-unknown-unknown
node --test js/test
```

## Build of the wasm modules

`cargo build` builds `engine.wasm` (wasm32-wasip1, C solvers compiled with wasi-sdk, `WASI_SDK_PATH`) and `mathlib.wasm` (wasm32-unknown-unknown) in the build script of `sbmlsim-host`, which embeds them; `SBMLSIM_CORE_WASM_DIR` names a directory with prebuilt ones. `sbmlsim-core runtime -o DIR` writes them out for the JavaScript host, and `sbmlsim-core compile` writes `model.wasm` and `model.json` of a model.

## Compile

The command line compiles an SBML Level 3 Version 2 file into a model directory, `model.wasm` and its description `model.json`:

```bash
cargo run -p sbmlsim-cli -- compile crates/host/tests/decay.xml -o decay-model
```

## Simulate

`sbmlsim-host` runs a compiled model with `engine.wasm` and `mathlib.wasm` in wasmtime; it embeds both, built by its build script with `WASI_SDK_PATH` (or copied from `SBMLSIM_CORE_WASM_DIR`, a directory with a prebuilt `engine.wasm` and `mathlib.wasm`). The command line simulates a model directory (`model.wasm` and `model.json`) with a request of `simulation/1` and writes the result as CSV:

```bash
cargo run -p sbmlsim-cli -- simulate crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

## JavaScript

`js/loader.mjs` runs the same `engine.wasm`, `mathlib.wasm` and `model.wasm` in a JavaScript host (Node, browsers) with the bits of wasmtime; it has no dependencies and uses no API of Node. `sbmlsim-core runtime -o js/runtime` writes the `engine.wasm` and `mathlib.wasm` of the build, and `js/simulate.mjs` writes the CSV of `sbmlsim-core simulate` byte for byte:

```bash
cargo run -p sbmlsim-cli -- runtime -o js/runtime
node js/simulate.mjs crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

```js
import { createRuntime } from "./js/loader.mjs";
const runtime = await createRuntime({ engine, mathlib }); // WebAssembly.Modules or their bytes
const instance = await runtime.instantiate(modelWasm, modelJson);
const { columns, nRows, values } = instance.simulate(request); // values: a Float64Array, row major
```

The host is published to npm as [`sbmlsim-core`](https://www.npmjs.com/package/sbmlsim-core) (`npm install sbmlsim-core`, the runtime of the release included; `js/README.md`), from a tag `vX.Y.Z` by `.github/workflows/publish.yml`. `js/pool.mjs` runs simulations in a pool of Web Workers (worker threads in Node, `js/worker.mjs`) with the bits of the main thread, and `js/panels.mjs` (`createPanelEndpoint`) answers the routes of the panel service of sbmlsim (`python -m sbmlsim.serve`) inside a page from an exported bundle (`panelbundle/1`), so an app of panels is a static page; see the [JavaScript page](https://matthiaskoenig.github.io/sbmlsim-core/javascript/) of the documentation.

The tests (`node --test js/test`) build the command line (`SBMLSIM_CORE_CLI` names a built one) and compare every passing case of the prepared SBML Test Suite with the wasmtime host (`SBMLSIM_CORE_EXPORT` names the output of `sbmlsim-core testsuite <cases> --export <dir>`).

## Test suite

`sbmlsim-core testsuite testdata/cases` compiles and simulates the cases of the SBML Test Suite (prepared with `uv run scripts/prepare_testsuite.py --out testdata/cases`) and compares them with the baseline `testdata/testsuite_baseline.json`; `--write-baseline` writes the baseline from a run. In every mode (analytic and difference Jacobian, optimizer on and off) each passing case is simulated again to check that it repeats bitwise.

Measured in this repository: 1581 of 1789 cases pass, every case which sbmlsim passes among them (the 208 failures are the cases sbmlsim does not pass either). The 208 expected failures are all refused at compilation (`not_compiled`) with a reason in the baseline:

| reason | cases |
|---|---:|
| algebraic rule is not supported | 123 |
| delay csymbol is not supported | 50 |
| not converted: fast reaction (libsbml conversion refuses a fast reaction) | 35 |

## Benchmark

`bench/models.toml` lists the models of the benchmark against roadrunner; `bench/RESULTS.md` has the method and the results. A model with `comp`, `distrib` or of another level is prepared first (`scripts/prepare_model.py`: comp flattened, the uncertainties of distrib stripped, a distribution function in a math refused, converted to L3V2):

```bash
uv run scripts/prepare_model.py --bench bench/models.toml        # into target/bench-models/
taskset -c 2 sbmlsim-core bench bench/models.toml --json target/bench/engine.json --export target/bench/export
taskset -c 2 node js/bench.mjs target/bench/export --json target/bench/node.json
uv run scripts/roadrunner_reference.py bench/models.toml --json target/bench/roadrunner.json
uv run scripts/agreement.py                                        # the engine at rtol 1e-10 against roadrunner at 1e-12 (dense, KLU)
```

The KLU baseline of roadrunner is a `roadrunner-sparsedq` build (libroadrunner with KLU and its colored sparse difference quotient) when `SBMLSIM_CORE_SPARSEDQ_BUILD` names its `site-packages` directory (or `--sparse-build`); without it `roadrunner_reference.py` takes the KLU of the libroadrunner 2.12 of its environment (KLU with the sparse difference quotient by default). `js/bench.mjs` reads `engine.wasm` from `SBMLSIM_CORE_RUNTIME`, else `js/runtime` (`sbmlsim-core runtime -o js/runtime`).

The counters of the integrator of the last simulation are `Instance::stats` (wasmtime) and `instance.stats()` (`js/loader.mjs`).

No model of the benchmark takes more than 2x the time of roadrunner native, dense or with KLU, in wasmtime, Node or Chrome (the largest 1.60, the event model in wasmtime), and every model agrees with roadrunner at tight tolerances. The engine locates the discontinuities of the math as roadrunner does: CVODE stops where the branch of a `piecewise` of a rule, a kinetic law or a trigger changes and starts again from the state there (the root functions after those of the triggers in `model_roots`, `n_discontinuity_roots` in `model.json`).

## License

The code of sbmlsim-core is MIT licensed, see `LICENSE`. `engine.wasm` statically links SUNDIALS 7.9.0 (BSD-3-Clause) and, from SuiteSparse 7.14.1, KLU and BTF (LGPL-2.1-or-later) and AMD, COLAMD and SuiteSparse_config (BSD-3-Clause), built from the submodules in `third_party`. So `engine.wasm` and the packages which build or embed it (`sbmlsim-csolvers`, `sbmlsim-host`, `sbmlsim-cli` and the JavaScript package with its runtime) are under `MIT AND BSD-3-Clause AND LGPL-2.1-or-later`. `THIRD_PARTY.md` has the versions, the commits of the submodules, the license texts and how to rebuild `engine.wasm` with a modified KLU or BTF; `sbmlsim-core runtime -o DIR` writes it next to `engine.wasm` and `mathlib.wasm`, and `sbmlsim_host::THIRD_PARTY_NOTICES` is its text.

# sbmlsim-core

An SBML to WebAssembly compiler and a CVODE/KLU simulation engine, written in Rust. The two contracts `model/1` (the description and the exports of a compiled model) and `simulation/1` (the request and the result of a simulation) are defined in `crates/abi`.

## Toolchain

User-level installs only, no system packages.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup target add wasm32-wasip1 wasm32-unknown-unknown
cargo install wasmtime-cli
# wasi-sdk: unpack the latest release from https://github.com/WebAssembly/wasi-sdk/releases to ~/opt/wasi-sdk
export WASI_SDK_PATH=$HOME/opt/wasi-sdk
```

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build -p sbmlsim-compiler --target wasm32-unknown-unknown
node --test js/test
```

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

The tests (`node --test js/test`) build the command line (`SBMLSIM_CORE_CLI` names a built one) and compare every passing case of the prepared SBML Test Suite with the wasmtime host (`SBMLSIM_CORE_EXPORT` names the output of `sbmlsim-core testsuite <cases> --export <dir>`).

## Benchmark

`bench/models.toml` lists the models of the benchmark against roadrunner; `bench/RESULTS.md` has the method and the results. A model with `comp`, `distrib` or of another level is prepared first (`scripts/prepare_model.py`: comp flattened, the uncertainties of distrib stripped, a distribution function in a math refused, converted to L3V2):

```bash
uv run scripts/prepare_model.py --bench bench/models.toml        # into target/bench-models/
taskset -c 2 sbmlsim-core bench bench/models.toml --json target/bench/engine.json --export target/bench/export
taskset -c 2 node js/bench.mjs target/bench/export --json target/bench/node.json
uv run scripts/roadrunner_reference.py bench/models.toml --json target/bench/roadrunner.json
uv run scripts/agreement.py                                        # the engine at rtol 1e-10 against roadrunner at 1e-12 (dense, KLU)
```

The counters of the integrator of the last simulation are `Instance::stats` (wasmtime) and `instance.stats()` (`js/loader.mjs`).

## License

MIT, see `LICENSE`.

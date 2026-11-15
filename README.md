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
cargo build -p sbmlsim-abi --target wasm32-unknown-unknown
```

## Simulate

`sbmlsim-host` runs a compiled model with `engine.wasm` and `mathlib.wasm` in wasmtime; it embeds both, built by its build script with `WASI_SDK_PATH` (or copied from `SBMLSIM_CORE_WASM_DIR`, a directory with a prebuilt `engine.wasm` and `mathlib.wasm`). The command line simulates a model directory (`model.wasm` and `model.json`) with a request of `simulation/1` and writes the result as CSV:

```bash
cargo run -p sbmlsim-cli -- simulate crates/host/tests/decay-model crates/host/tests/decay-model/request.json [-o out.csv]
```

## License

MIT, see `LICENSE`.

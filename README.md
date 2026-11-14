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

## License

MIT, see `LICENSE`.

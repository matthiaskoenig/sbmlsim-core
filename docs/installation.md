# Installation

sbmlsim-core is built from source. The crates are not published, and the JavaScript package (`js/`) is private: a page uses the files of `js/` and the `engine.wasm` and `mathlib.wasm` of a build.

## Toolchain

User-level installs only, no system packages. The Rust toolchain is the one of `rust-toolchain.toml` (1.100.0 with the targets `wasm32-wasip1` and `wasm32-unknown-unknown`), which rustup installs when it is first used in the repository.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup target add wasm32-wasip1 wasm32-unknown-unknown
cargo install wasmtime-cli --version 50.0.1 --locked
# wasi-sdk 34 (the version of CI and of the build scripts): unpack
# https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz to ~/opt/wasi-sdk
export WASI_SDK_PATH=$HOME/opt/wasi-sdk
```

The C solvers are git submodules (`third_party/sundials`, `third_party/SuiteSparse`), so the repository is cloned with them:

```bash
git clone --recurse-submodules https://github.com/matthiaskoenig/sbmlsim-core
```

Node 20 or newer runs the JavaScript host and its tests (CI uses Node 24). [uv](https://docs.astral.sh/uv/) runs the Python scripts (`scripts/`), which prepare models and the SBML Test Suite and compute the references of roadrunner.

## Building

`cargo build` builds `engine.wasm` (wasm32-wasip1, the C solvers compiled with the clang of wasi-sdk, `WASI_SDK_PATH`) and `mathlib.wasm` (wasm32-unknown-unknown) in the build script of `sbmlsim-host`, which embeds them; `SBMLSIM_CORE_WASM_DIR` names a directory with prebuilt ones instead.

```bash
export PATH=$HOME/.cargo/bin:$PATH WASI_SDK_PATH=$HOME/opt/wasi-sdk
cargo build --release -p sbmlsim-cli          # the command line target/release/sbmlsim-core
./target/release/sbmlsim-core runtime -o js/runtime
```

`sbmlsim-core runtime -o DIR` writes the `engine.wasm` and `mathlib.wasm` of the build and the notices of the libraries of the engine (`THIRD_PARTY.md`) into a directory; `js/runtime` is where the JavaScript host looks for them by default (gitignored).

## Requirements of the hosts

`engine.wasm` uses fixed-width SIMD (WebAssembly SIMD, the C solvers are compiled with `-msimd128`). The results do not depend on it, they are the same bits as without, but a host without SIMD cannot instantiate `engine.wasm`. wasmtime, Node and the current browsers (Chrome 91, Firefox 89, Safari 16.4 and later) support it. The memory of an instance grows to at most 2 GiB.

The default Rust host and CLI use a minimal Wasmtime build and do not link the native C solvers. The embedded WebAssembly engine still contains its solvers. Its WASI adapter captures stdout and stderr locally (up to 1 MiB each), exposes an empty environment, and rejects process exit, clocks, randomness, filesystem and socket operations.

Build with `cargo build -p sbmlsim-cli --features native` to enable `bench --host native`. Native comparison tests run with `cargo test -p sbmlsim-host --features native`. `EngineError` is defined in `sbmlsim-abi` and reexported by the engine and host.

## Checks

clippy runs with `-D warnings` and the default lints.

```bash
cargo fmt --check
cargo check -p sbmlsim-host -p sbmlsim-cli --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy -p sbmlsim-csolvers --all-targets --target wasm32-wasip1 -- -D warnings
cargo clippy -p sbmlsim-engine --all-targets --target wasm32-wasip1 -- -D warnings
cargo clippy -p sbmlsim-engine --lib --features exports --target wasm32-wasip1 -- -D warnings
cargo test --all-features
cargo build -p sbmlsim-compiler --target wasm32-unknown-unknown
node --test js/test
```

The JavaScript tests build the command line (`SBMLSIM_CORE_CLI` names a built one instead) and compare every passing case of the prepared SBML Test Suite with the wasmtime host (`SBMLSIM_CORE_EXPORT` names the output of `sbmlsim-core testsuite <cases> --export <dir>`); without the prepared cases those tests are skipped.

## Documentation

These pages are built with [Zensical](https://zensical.org/) from `docs/` and `zensical.toml` into the gitignored `site/`:

```bash
uvx --from zensical==0.1.0 zensical build --clean
uvx --from zensical==0.1.0 zensical serve      # a preview on http://localhost:8000
```

The workflow `docs` builds them on every push and pull request to `develop` and publishes them from `develop` to [GitHub Pages](https://matthiaskoenig.github.io/sbmlsim-core/).

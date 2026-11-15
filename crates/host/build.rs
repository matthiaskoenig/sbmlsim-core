//! Provides `engine.wasm` and `mathlib.wasm` in `OUT_DIR`, which `src/lib.rs` embeds.
//!
//! With `SBMLSIM_CORE_WASM_DIR` both are copied from that directory. Otherwise they are built
//! by a nested cargo, in a target directory of their own (`wasm-build` in the target directory
//! of the workspace, kept between builds): the engine as a `cdylib` for `wasm32-wasip1` with an
//! imported (and exported) memory, which needs `WASI_SDK_PATH` for the C of `sbmlsim-csolvers`,
//! and mathlib for `wasm32-unknown-unknown`, both with the release profile.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The variable of a directory with a prebuilt `engine.wasm` and `mathlib.wasm`.
const WASM_DIR: &str = "SBMLSIM_CORE_WASM_DIR";

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    println!("cargo:rerun-if-env-changed={WASM_DIR}");
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(dir) = env::var_os(WASM_DIR) {
        let dir = PathBuf::from(dir);
        for name in ["engine.wasm", "mathlib.wasm"] {
            let source = dir.join(name);
            println!("cargo:rerun-if-changed={}", source.display());
            std::fs::copy(&source, out.join(name)).unwrap_or_else(|e| {
                panic!(
                    "{WASM_DIR} is set, but {} cannot be read: {e}",
                    source.display()
                )
            });
        }
        return;
    }
    if env::var_os("WASI_SDK_PATH").is_none() {
        panic!(
            "sbmlsim-host embeds engine.wasm and mathlib.wasm: set WASI_SDK_PATH (the wasi-sdk \
             with which the engine is built for wasm32-wasip1) or {WASM_DIR} (a directory with \
             a prebuilt engine.wasm and mathlib.wasm)"
        );
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.join("../..");
    for crate_dir in ["abi", "engine", "csolvers", "mathlib"] {
        let dir = workspace.join("crates").join(crate_dir);
        for part in ["src", "Cargo.toml", "build.rs"] {
            if dir.join(part).exists() {
                println!("cargo:rerun-if-changed={}", dir.join(part).display());
            }
        }
    }
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("Cargo.lock").display()
    );
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"))
        .join("wasm-build");
    let engine = build(
        &workspace,
        &target_dir,
        &[
            "rustc",
            "-p",
            "sbmlsim-engine",
            "--lib",
            "--crate-type",
            "cdylib",
            "--target",
            "wasm32-wasip1",
            "--release",
            "--locked",
            "--features",
            "exports",
        ],
        &[
            "--",
            "-C",
            "link-arg=--import-memory",
            "-C",
            "link-arg=--export-memory",
        ],
        "wasm32-wasip1/release/sbmlsim_engine.wasm",
    );
    let mathlib = build(
        &workspace,
        &target_dir,
        &[
            "build",
            "-p",
            "sbmlsim-mathlib",
            "--lib",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--locked",
        ],
        &[],
        "wasm32-unknown-unknown/release/sbmlsim_mathlib.wasm",
    );
    std::fs::copy(engine, out.join("engine.wasm")).unwrap();
    std::fs::copy(mathlib, out.join("mathlib.wasm")).unwrap();
}

/// Runs a nested cargo with `args`, the target directory and `rest`, and gives the path of
/// `artifact` in the target directory.
fn build(
    workspace: &Path,
    target_dir: &Path,
    args: &[&str],
    rest: &[&str],
    artifact: &str,
) -> PathBuf {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(workspace)
        .args(args)
        .arg("--target-dir")
        .arg(target_dir)
        .args(rest);
    // the flags, the target and the wrapper (clippy) of the outer build are not the ones of the
    // wasm build
    for variable in [
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_RUSTFLAGS",
    ] {
        command.env_remove(variable);
    }
    // cargo reads the stdout of a build script, the nested cargo writes to stderr
    let status = command
        .stdout(std::io::stderr())
        .status()
        .unwrap_or_else(|e| panic!("cargo cannot run: {e}"));
    if !status.success() {
        panic!(
            "building {artifact} failed ({status}); it needs WASI_SDK_PATH (the wasi-sdk of the \
             engine) and the targets of rust-toolchain.toml, or {WASM_DIR} with a prebuilt \
             engine.wasm and mathlib.wasm"
        );
    }
    target_dir.join(artifact)
}

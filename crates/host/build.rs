//! Provides `engine.wasm`, `mathlib.wasm`, `compiler.wasm` and `plot.wasm` in `OUT_DIR`, which
//! `src/lib.rs` embeds.
//!
//! With `SBMLSIM_CORE_WASM_DIR` the four are copied from that directory. Otherwise they are
//! built by a nested cargo, in a target directory of their own (`wasm-build` in the target
//! directory of the workspace, kept between builds): the engine as a `cdylib` for
//! `wasm32-wasip1` with an imported (and exported) memory, which needs `WASI_SDK_PATH` for the
//! C of `sbmlsim-csolvers`, mathlib for `wasm32-unknown-unknown` with the release profile, and
//! `compiler.wasm` (`sbmlsim-wasm`, the compiler in a browser) for `wasm32-unknown-unknown`
//! with the profile `wasm`, which is the release profile for size, and `plot.wasm`
//! (`sbmlsim-plot-wasm`, the renderer of the figures in a browser) for `wasm32-wasip1` with the
//! profile `wasm` (its crate documents why not for `wasm32-unknown-unknown`).
//!
//! `engine.wasm` gets a custom section `sbmlsim.engine` (`sbmlsim_abi::ENGINE_SECTION`, when it
//! has none) with the JSON `{"memory_minimum_pages":n,"output_format":k}`: the minimum of the
//! memory it imports as `env.memory`, which a JavaScript host creates before it instantiates
//! the engine and has no reflection of the type of an import of a compiled `WebAssembly.Module`
//! (`js/loader.mjs` reads the section with `WebAssembly.Module.customSections`), and the version
//! of its output (`sbmlsim_abi::ENGINE_OUTPUT_FORMAT`), which both hosts check.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use sbmlsim_abi::{ENGINE_OUTPUT_FORMAT, ENGINE_SECTION};

/// The variable of a directory with a prebuilt `engine.wasm`, `mathlib.wasm`, `compiler.wasm`
/// and `plot.wasm`.
const WASM_DIR: &str = "SBMLSIM_CORE_WASM_DIR";

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    println!("cargo:rerun-if-env-changed={WASM_DIR}");
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(dir) = env::var_os(WASM_DIR) {
        let dir = PathBuf::from(dir);
        for name in ["engine.wasm", "mathlib.wasm", "compiler.wasm", "plot.wasm"] {
            let source = dir.join(name);
            println!("cargo:rerun-if-changed={}", source.display());
            std::fs::copy(&source, out.join(name)).unwrap_or_else(|e| {
                panic!(
                    "{WASM_DIR} is set, but {} cannot be read: {e}",
                    source.display()
                )
            });
        }
        add_engine_section(&out.join("engine.wasm"));
        return;
    }
    if env::var_os("WASI_SDK_PATH").is_none() {
        panic!(
            "sbmlsim-host embeds engine.wasm, mathlib.wasm, compiler.wasm and plot.wasm: set \
             WASI_SDK_PATH (the wasi-sdk with which the engine is built for wasm32-wasip1) or \
             {WASM_DIR} (a directory with the four prebuilt)"
        );
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.join("../..");
    for crate_dir in [
        "abi",
        "engine",
        "csolvers",
        "mathlib",
        "sbml",
        "ir",
        "codegen",
        "compiler",
        "wasm",
        "document",
        "reader",
        "ode",
        "plot",
        "plot-wasm",
    ] {
        let dir = workspace.join("crates").join(crate_dir);
        for part in ["src", "csrc", "templates", "Cargo.toml", "build.rs"] {
            if dir.join(part).exists() {
                println!("cargo:rerun-if-changed={}", dir.join(part).display());
            }
        }
    }
    // The outer build must notice C/header changes too, otherwise the nested build is
    // never invoked and the host can embed an older solver than its native dependency.
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("third_party").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("rust-toolchain.toml").display()
    );
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
    let compiler = build(
        &workspace,
        &target_dir,
        &[
            "build",
            "-p",
            "sbmlsim-wasm",
            "--lib",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "wasm",
            "--locked",
        ],
        &[],
        "wasm32-unknown-unknown/wasm/sbmlsim_wasm.wasm",
    );
    let plot = build(
        &workspace,
        &target_dir,
        &[
            "build",
            "-p",
            "sbmlsim-plot-wasm",
            "--lib",
            "--target",
            "wasm32-wasip1",
            "--profile",
            "wasm",
            "--locked",
        ],
        &[],
        "wasm32-wasip1/wasm/sbmlsim_plot_wasm.wasm",
    );
    std::fs::copy(engine, out.join("engine.wasm")).unwrap();
    std::fs::copy(mathlib, out.join("mathlib.wasm")).unwrap();
    std::fs::copy(compiler, out.join("compiler.wasm")).unwrap();
    std::fs::copy(plot, out.join("plot.wasm")).unwrap();
    add_engine_section(&out.join("engine.wasm"));
}

/// Reads an unsigned LEB128 at `*at`.
fn leb(bytes: &[u8], at: &mut usize) -> u64 {
    let (mut value, mut shift) = (0u64, 0);
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

/// Appends the unsigned LEB128 of `value`.
fn push_leb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A name of the module at `*at`.
fn name<'a>(bytes: &'a [u8], at: &mut usize) -> &'a [u8] {
    let len = leb(bytes, at) as usize;
    let name = &bytes[*at..*at + len];
    *at += len;
    name
}

/// Adds the section [`ENGINE_SECTION`] to the module at `path` unless it has one.
fn add_engine_section(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0", "engine.wasm is no module");
    let mut minimum = None;
    let mut at = 8;
    while at < bytes.len() {
        let id = bytes[at];
        at += 1;
        let size = leb(&bytes, &mut at) as usize;
        let end = at + size;
        match id {
            0 if name(&bytes, &mut at.clone()) == ENGINE_SECTION.as_bytes() => return,
            // the import section: the memory `env.memory`
            2 => {
                let mut p = at;
                for _ in 0..leb(&bytes, &mut p) {
                    let module = name(&bytes, &mut p).to_vec();
                    let field = name(&bytes, &mut p).to_vec();
                    let kind = bytes[p];
                    p += 1;
                    match kind {
                        0 => {
                            leb(&bytes, &mut p);
                        }
                        1 => {
                            p += 1;
                            let flags = leb(&bytes, &mut p);
                            leb(&bytes, &mut p);
                            if flags & 1 == 1 {
                                leb(&bytes, &mut p);
                            }
                        }
                        2 => {
                            let flags = leb(&bytes, &mut p);
                            let min = leb(&bytes, &mut p);
                            if flags & 1 == 1 {
                                leb(&bytes, &mut p);
                            }
                            if module == b"env" && field == b"memory" {
                                minimum = Some(min);
                            }
                        }
                        3 => p += 2,
                        other => panic!("engine.wasm imports an extern of the kind {other}"),
                    }
                }
            }
            _ => {}
        }
        at = end;
    }
    let minimum = minimum.expect("engine.wasm imports env.memory");
    let payload =
        format!(r#"{{"memory_minimum_pages":{minimum},"output_format":{ENGINE_OUTPUT_FORMAT}}}"#);
    let mut section = Vec::new();
    push_leb(&mut section, ENGINE_SECTION.len() as u64);
    section.extend_from_slice(ENGINE_SECTION.as_bytes());
    section.extend_from_slice(payload.as_bytes());
    bytes.push(0);
    push_leb(&mut bytes, section.len() as u64);
    bytes.extend_from_slice(&section);
    std::fs::write(path, bytes).unwrap();
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
    // the C flags of the environment never reach the C of the engine (`sbmlsim-csolvers`
    // ignores them as well): a stray `-ffast-math` would change the bits of `engine.wasm`
    for (key, _) in env::vars_os() {
        if key.to_str().is_some_and(|key| {
            key == "CFLAGS"
                || key.starts_with("CFLAGS_")
                || key == "TARGET_CFLAGS"
                || key == "HOST_CFLAGS"
        }) {
            command.env_remove(key);
        }
    }
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
             engine.wasm, mathlib.wasm, compiler.wasm and plot.wasm"
        );
    }
    target_dir.join(artifact)
}

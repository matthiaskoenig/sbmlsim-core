//! The command line of sbmlsim-core.
//!
//! `sbmlsim-core compile <model.xml> -o <dir>` compiles an SBML file into the directory `dir`
//! (created when missing): `model.wasm` and `model.json`, the description of the model.
//!
//! `sbmlsim-core simulate <model-dir> <request.json> [-o out.csv]` simulates a compiled model
//! (the directory holds `model.wasm` and `model.json`) with a request of the contract
//! `simulation/1`, in the wasmtime host with the embedded `engine.wasm` and `mathlib.wasm`, and
//! writes the result as CSV, to `out.csv` or else to stdout. Errors go to stderr with the exit
//! code 1; a usage error has the exit code 2.
//!
//! The CSV has a header of the columns (ids of SBML, which hold no comma or quote) and a line
//! per row; every number is written in the shortest decimal which reads back as the same
//! double (Rust's `{}` of `f64`, which writes no exponent: `0.5`, `1`, `0.000001`), and the
//! values which are not finite as `NaN`, `inf` and `-inf` (`sbmlsim_cli::csv`, which
//! `js/simulate.mjs` writes byte for byte alike).
//!
//! `sbmlsim-core bench <models.toml> [--json FILE]` measures the compilation, the instantiation
//! and the simulations of the models of a benchmark file in wasmtime (`sbmlsim_cli::bench`).
//!
//! `sbmlsim-core runtime -o <dir>` writes the `engine.wasm` and `mathlib.wasm` of this build
//! (the ones `simulate` embeds), which the JavaScript loader `js/loader.mjs` runs, and next to
//! them `THIRD_PARTY.md`, the notices of the libraries `engine.wasm` links statically.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use sbmlsim_abi::{ModelDescription, SimulationRequest};
use sbmlsim_cli::bench::{self, BenchArgs};
use sbmlsim_cli::csv::to_csv;
use sbmlsim_cli::testsuite::{self, TestsuiteArgs};
use sbmlsim_compiler::CompileOptions;
use sbmlsim_host::Runtime;

/// The command line of sbmlsim-core.
#[derive(Parser)]
#[command(name = "sbmlsim-core", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The commands.
#[derive(Subcommand)]
enum Command {
    /// Compiles an SBML file into `model.wasm` and `model.json`.
    Compile {
        /// The SBML file.
        model: PathBuf,
        /// The directory to write `model.wasm` and `model.json` into.
        #[arg(short, long)]
        output: PathBuf,
        /// Without the optimizer (the same results, a larger and slower module).
        #[arg(long)]
        no_optimize: bool,
    },
    /// Simulates a compiled model with a request and writes the result as CSV.
    Simulate {
        /// The directory of the model, with `model.wasm` and `model.json`.
        model_dir: PathBuf,
        /// The request, a JSON file of the contract `simulation/1`.
        request: PathBuf,
        /// The CSV file to write (stdout without).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Execution deadline in milliseconds; 0 disables it for trusted long runs.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
    /// Runs the prepared cases of the SBML Test Suite and compares them with the baseline.
    Testsuite(TestsuiteArgs),
    /// Measures the models of a benchmark file (`bench/models.toml`) in wasmtime.
    Bench(BenchArgs),
    /// Writes the `engine.wasm` and `mathlib.wasm` of this build and the notices of the
    /// libraries of the engine (`THIRD_PARTY.md`) into a directory.
    Runtime {
        /// The directory (created when missing).
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Compile {
            model,
            output,
            no_optimize,
        } => compile(&model, &output, !no_optimize).map(|()| true),
        Command::Simulate {
            model_dir,
            request,
            output,
            timeout_ms,
        } => simulate(&model_dir, &request, output.as_deref(), timeout_ms).map(|()| true),
        Command::Testsuite(args) => testsuite::run(&args),
        Command::Bench(args) => bench::run(&args),
        Command::Runtime { output } => testsuite::write_runtime(&output).map(|()| true),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a JSON file.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("cannot read {}", path.display()))
}

/// The command `compile`, with the optimizer when `optimize`.
fn compile(model: &Path, output: &Path, optimize: bool) -> anyhow::Result<()> {
    let sbml =
        fs::read_to_string(model).with_context(|| format!("cannot read {}", model.display()))?;
    let options = CompileOptions {
        optimize,
        ..CompileOptions::default()
    };
    let compiled = sbmlsim_compiler::compile(&sbml, &options)
        .with_context(|| format!("cannot compile {}", model.display()))?;
    fs::create_dir_all(output).with_context(|| format!("cannot create {}", output.display()))?;
    let write = |name: &str, bytes: &[u8]| {
        let path = output.join(name);
        fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))
    };
    write("model.wasm", &compiled.wasm)?;
    write(
        "model.json",
        &testsuite::description_json(&compiled.description)?,
    )
}

/// The command `simulate`.
fn simulate(
    model_dir: &Path,
    request: &Path,
    output: Option<&Path>,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let wasm_path = model_dir.join("model.wasm");
    let wasm =
        fs::read(&wasm_path).with_context(|| format!("cannot read {}", wasm_path.display()))?;
    let desc: ModelDescription = read_json(&model_dir.join("model.json"))?;
    let req: SimulationRequest = read_json(request)?;
    let mut instance = Runtime::embedded()?
        .with_execution_timeout(
            (timeout_ms != 0).then(|| std::time::Duration::from_millis(timeout_ms)),
        )
        .instantiate(&wasm, &desc)?;
    let csv = to_csv(&instance.simulate(&req)?);
    match output {
        Some(path) => {
            fs::write(path, csv).with_context(|| format!("cannot write {}", path.display()))?
        }
        None => std::io::stdout()
            .lock()
            .write_all(csv.as_bytes())
            .context("cannot write to stdout")?,
    }
    Ok(())
}

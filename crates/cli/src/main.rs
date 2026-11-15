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
//! values which are not finite as `NaN`, `inf` and `-inf`.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use sbmlsim_abi::{ModelDescription, SimulationRequest, SimulationResult};
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
    },
    /// Runs the prepared cases of the SBML Test Suite and compares them with the baseline.
    Testsuite(TestsuiteArgs),
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
        } => simulate(&model_dir, &request, output.as_deref()).map(|()| true),
        Command::Testsuite(args) => testsuite::run(&args),
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
    let mut json = serde_json::to_vec_pretty(&compiled.description)?;
    json.push(b'\n');
    write("model.json", &json)
}

/// The command `simulate`.
fn simulate(model_dir: &Path, request: &Path, output: Option<&Path>) -> anyhow::Result<()> {
    let wasm_path = model_dir.join("model.wasm");
    let wasm =
        fs::read(&wasm_path).with_context(|| format!("cannot read {}", wasm_path.display()))?;
    let desc: ModelDescription = read_json(&model_dir.join("model.json"))?;
    let req: SimulationRequest = read_json(request)?;
    let mut instance = Runtime::embedded()?.instantiate(&wasm, &desc)?;
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

/// The CSV of a result.
fn to_csv(result: &SimulationResult) -> String {
    let mut csv = result.columns.join(",");
    csv.push('\n');
    let k = result.columns.len().max(1);
    for row in result.values.chunks(k) {
        for (j, value) in row.iter().enumerate() {
            if j > 0 {
                csv.push(',');
            }
            write!(csv, "{value}").unwrap();
        }
        csv.push('\n');
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_back_bitwise() {
        let values = vec![
            0.0,
            -0.0,
            1.0,
            0.1,
            1.0 / 3.0,
            1e-300,
            1.7976931348623157e308,
            5e-324,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let result = SimulationResult {
            columns: vec!["time".to_string()],
            n_rows: values.len(),
            values: values.clone(),
        };
        let csv = to_csv(&result);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "time");
        assert_eq!(&lines[1..5], ["0", "-0", "1", "0.1"]);
        assert_eq!(&lines[9..], ["NaN", "inf", "-inf"]);
        for (line, value) in lines[1..].iter().zip(&values) {
            let read: f64 = line.parse().unwrap();
            assert!(read.to_bits() == value.to_bits() || (read.is_nan() && value.is_nan()));
        }
    }
}

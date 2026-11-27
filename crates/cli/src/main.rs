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
//! `sbmlsim-core petab <problem.yaml> [--simulations FILE] [--fit]` (with the feature `native`)
//! evaluates a PEtab v2 problem at the nominal values of its parameter table, writes its
//! simulation table and fits it (`sbmlsim_petab`).
//!
//! `sbmlsim-core bench <models.toml> [--json FILE]` measures the compilation, the instantiation
//! and the simulations of the models of a benchmark file in wasmtime (`sbmlsim_cli::bench`).
//!
//! `sbmlsim-core runtime -o <dir>` writes the `engine.wasm` and `mathlib.wasm` of this build
//! (the ones `simulate` embeds), which the JavaScript loader `js/loader.mjs` runs,
//! `compiler.wasm`, the compiler of `js/compile.mjs` which compiles a document in a page,
//! `plot.wasm`, the renderer of the figures in a page (`crates/plot-wasm`), and next to them
//! `THIRD_PARTY.md`, the notices of the libraries `engine.wasm` links statically.

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
        /// Omit analytic parameter derivatives and their cached constants.
        #[arg(long)]
        no_parameter_jacobian: bool,
    },
    /// Export standalone Python, Diffrax, Julia, R, Markdown, LaTeX or Typst from SBML.
    Export {
        /// The trusted SBML file, with local comp companion documents.
        model: PathBuf,
        /// The source format.
        #[arg(long, value_parser=["python","diffrax","julia","r","markdown","latex","typst"], default_value="python")]
        format: String,
        /// The file to write (stdout without).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit equation functions without a simulation driver.
        #[arg(long)]
        no_simulator: bool,
        /// Emit a document fragment instead of a standalone document.
        #[arg(long)]
        fragment: bool,
        /// Mathematical symbols use source ids or unambiguous names.
        #[arg(long, value_parser=["id","name"], default_value="id")]
        symbols: String,
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
    /// Fits the parameters of a compiled model to data with a request of the contract `fit/1`.
    #[cfg(feature = "native")]
    Fit {
        /// The directory of the model, with `model.wasm` and `model.json`.
        model_dir: PathBuf,
        /// The fit, a JSON file of the contract `fit/1`.
        request: PathBuf,
        /// The JSON file to write the answer into (stdout without).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// The threads of the batch of an evaluation; the ones of the request, else the CPUs
        /// of the machine.
        #[arg(short, long)]
        threads: Option<usize>,
    },
    /// Evaluates a PEtab v2 problem at the nominal values of its parameters, or fits it.
    #[cfg(feature = "native")]
    Petab {
        /// The YAML file of the problem.
        problem: PathBuf,
        /// The TSV file to write the simulation table of PEtab into.
        #[arg(short, long)]
        simulations: Option<PathBuf>,
        /// Fits the estimated parameters from their nominal values (trust region reflective).
        #[arg(long)]
        fit: bool,
        /// The relative tolerance of the integrator; the one of the problem, else `1e-8`.
        #[arg(long)]
        relative_tolerance: Option<f64>,
        /// The absolute tolerance of the integrator; the one of the problem, else `1e-12`.
        #[arg(long)]
        absolute_tolerance: Option<f64>,
        /// The threads of a fit; the CPUs of the machine without.
        #[arg(short, long)]
        threads: Option<usize>,
    },
    /// Runs the prepared cases of the SBML Test Suite and compares them with the baseline.
    Testsuite(TestsuiteArgs),
    /// Measures the models of a benchmark file (`bench/models.toml`) in wasmtime.
    Bench(BenchArgs),
    /// Writes the `engine.wasm`, `mathlib.wasm`, `compiler.wasm` and `plot.wasm` of this build
    /// and the notices of the libraries of the engine (`THIRD_PARTY.md`) into a directory.
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
            no_parameter_jacobian,
        } => compile(&model, &output, !no_optimize, !no_parameter_jacobian).map(|()| true),
        Command::Export {
            model,
            format,
            output,
            no_simulator,
            fragment,
            symbols,
        } => export_ode(
            &model,
            &format,
            output.as_deref(),
            !no_simulator,
            !fragment,
            &symbols,
        )
        .map(|()| true),
        Command::Simulate {
            model_dir,
            request,
            output,
            timeout_ms,
        } => simulate(&model_dir, &request, output.as_deref(), timeout_ms).map(|()| true),
        #[cfg(feature = "native")]
        Command::Fit {
            model_dir,
            request,
            output,
            threads,
        } => fit(&model_dir, &request, output.as_deref(), threads).map(|()| true),
        #[cfg(feature = "native")]
        Command::Petab {
            problem,
            simulations,
            fit,
            relative_tolerance,
            absolute_tolerance,
            threads,
        } => petab(
            &problem,
            simulations.as_deref(),
            fit,
            (relative_tolerance, absolute_tolerance),
            threads,
        )
        .map(|()| true),
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
fn compile(
    model: &Path,
    output: &Path,
    optimize: bool,
    parameter_jacobian: bool,
) -> anyhow::Result<()> {
    let sbml =
        fs::read_to_string(model).with_context(|| format!("cannot read {}", model.display()))?;
    let options = CompileOptions {
        optimize,
        parameter_jacobian,
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

/// The command `fit`: the fit of the request against the model of the directory, and its
/// answer as JSON.
#[cfg(feature = "native")]
fn fit(
    model_dir: &Path,
    request: &Path,
    output: Option<&Path>,
    threads: Option<usize>,
) -> anyhow::Result<()> {
    let wasm_path = model_dir.join("model.wasm");
    let wasm =
        fs::read(&wasm_path).with_context(|| format!("cannot read {}", wasm_path.display()))?;
    let desc: ModelDescription = read_json(&model_dir.join("model.json"))?;
    let fit: sbmlsim_fit::FitRequest = read_json(request)?;
    let model = std::sync::Arc::new(Runtime::embedded()?.load(&wasm, &desc)?);
    let answer = sbmlsim_fit::fit(&fit, model, threads)?;
    let json = serde_json::to_string_pretty(&answer).context("cannot write the answer")?;
    match output {
        Some(path) => {
            fs::write(path, &json).with_context(|| format!("cannot write {}", path.display()))?;
            let found: Vec<String> = answer
                .parameters
                .iter()
                .map(|parameter| {
                    let said = format!("{} = {:.6}", parameter.id, parameter.value);
                    // the interval of the parameter, when the fit profiled it
                    match answer
                        .profiles
                        .iter()
                        .find(|profile| profile.id == parameter.id)
                    {
                        None => said,
                        Some(profile) => match (profile.lower, profile.upper) {
                            (Some(lower), Some(upper)) => {
                                format!("{said} [{lower:.6}, {upper:.6}]")
                            }
                            _ => format!("{said} (not identifiable)"),
                        },
                    }
                })
                .collect();
            println!(
                "{} after {} evaluations and {} simulations ({}) in {:.3} s, cost {:.6e} \
                 ({}): {}",
                found.join(", "),
                answer.evaluations,
                answer.simulations,
                match answer.jacobian {
                    sbmlsim_fit::JacobianChoice::DifferenceQuotient => "finite differences",
                    _ => "sensitivities",
                },
                answer.seconds,
                answer.cost,
                answer.status,
                path.display()
            );
        }
        None => std::io::stdout()
            .lock()
            .write_all(json.as_bytes())
            .context("cannot write to stdout")?,
    }
    Ok(())
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

/// Generate a standalone file through the same reader and equation kernel as the bindings.
fn export_ode(
    model: &Path,
    format: &str,
    output: Option<&Path>,
    simulator: bool,
    standalone: bool,
    symbols: &str,
) -> anyhow::Result<()> {
    let system = sbmlsim_ode::OdeSystem::read_path(model, 10000).map_err(anyhow::Error::msg)?;
    let options = sbmlsim_ode::RenderOptions {
        simulator,
        standalone,
        symbols: symbols.into(),
        source: model.file_name().map(|s| s.to_string_lossy().into_owned()),
    };
    let export = system
        .render(format, &options)
        .map_err(anyhow::Error::msg)?;
    match output {
        Some(path) => std::fs::write(path, export.content)?,
        None => std::io::stdout()
            .lock()
            .write_all(export.content.as_bytes())?,
    }
    Ok(())
}

/// The command `petab`: the evaluation of a problem at its nominal values, and its fit.
#[cfg(feature = "native")]
fn petab(
    problem: &Path,
    simulations: Option<&Path>,
    fit: bool,
    (relative, absolute): (Option<f64>, Option<f64>),
    threads: Option<usize>,
) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let read = sbmlsim_petab::PetabProblem::from_path(problem)?;
    let mut options = sbmlsim_petab::Options::default();
    if relative.is_some() || absolute.is_some() {
        let mut settings = sbmlsim_abi::IntegratorSettings {
            relative_tolerance: 1e-8,
            ..sbmlsim_abi::IntegratorSettings::default()
        };
        if let Some(relative) = relative {
            settings.relative_tolerance = relative;
        }
        if let Some(absolute) = absolute {
            settings.absolute_tolerance = sbmlsim_abi::Tolerance::Scalar(absolute);
        }
        options.integrator = Some(settings);
    }
    let plan = sbmlsim_petab::Plan::new(&read, &options)?;
    let executor = sbmlsim_petab::Executor::new(plan)?;
    let loaded = started.elapsed().as_secs_f64();
    let x = executor.plan().nominal();
    let evaluation = executor.evaluate(&x);
    for (simulation, message) in &evaluation.failures {
        eprintln!("simulation {simulation}: {message}");
    }
    println!(
        "{} measurements, {} simulations of {} models: llh {:e}, chi2 {:e}, log prior {:e} \
         (read and compiled in {loaded:.3} s, evaluated in {:.3} s)",
        executor.plan().measurements.len(),
        executor.plan().simulations.len(),
        executor.plan().models.len(),
        evaluation.llh,
        evaluation.chi2,
        evaluation.log_prior,
        started.elapsed().as_secs_f64() - loaded,
    );
    if let Some(path) = simulations {
        fs::write(path, executor.plan().simulation_table(&evaluation))
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if fit {
        let threads = threads.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        });
        let answer = executor.fit(&sbmlsim_fit::Settings::default(), threads)?;
        let names: Vec<String> = executor
            .plan()
            .estimated()
            .into_iter()
            .zip(&answer.x)
            .map(|(k, value)| format!("{} = {value:.6}", executor.plan().parameters[k].id))
            .collect();
        println!(
            "{} after {} evaluations in {:.3} s, objective {:.10e} ({})",
            names.join(", "),
            answer.fit.evaluations,
            answer.fit.seconds,
            answer.objective,
            answer.fit.status
        );
    }
    Ok(())
}

//! The benchmark of the command line: `sbmlsim-core bench <models.toml> [--json FILE]
//! [--host wasmtime|native]`.
//!
//! `--host native` measures the native engine instead of `engine.wasm` in wasmtime:
//! `sbmlsim-engine` compiled for the machine, with the model in wasmtime
//! (`Runtime::instantiate_native`); its results are not bitwise those of `engine.wasm` (the C
//! library of the host in the step sizes of CVODE).
//!
//! Every model of the benchmark file (`bench/models.toml`, prepared with
//! `scripts/prepare_model.py --bench`) is compiled (the median of `--compile-reps` compilations
//! of its SBML into `model.wasm`), instantiated in the wasmtime host (the median of as many
//! instantiations: the compilation of `model.wasm` by wasmtime, the instances and the load of
//! the description; `engine.wasm` and `mathlib.wasm` are compiled once) and simulated in every
//! configuration of the linear solver (dense, KLU) and the Jacobian (analytic, difference
//! quotient), with the output of uniform times and, for a model with `steps = true`, of the
//! steps of the integrator. A measurement of a configuration is the median of `--reps` timed
//! simulations after `--warmups` untimed ones, each the wall time of `Instance::simulate` (the
//! request to JSON, the simulation in the engine and the copy of the result); the
//! configurations take turns within a round, and the time of a configuration is the median of
//! its medians over `--rounds` rounds. The counters of the integrator are the ones of the last
//! simulation of a configuration, and every timed simulation must give the bits of the first.
//!
//! The absolute tolerance of a model is scaled per state as roadrunner scales it, from the
//! file `<sbml>.atol.json` next to the prepared SBML (the tolerance of every state of
//! roadrunner's state vector by its id), else it is the scalar of the model.
//!
//! `--export DIR` writes, per model, `model.wasm`, `model.json` and the request of every
//! configuration, and `bench.json`, the list of the models and their configurations, which
//! `js/bench.mjs` (Node) and `bench/web/` (Chrome) run with the same requests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, bail};
use clap::Args;
use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, LinearSolver, ModelDescription, Output,
    SIMULATION_CONTRACT, SimulationRequest, SimulationStats, Tolerance,
};
use sbmlsim_compiler::CompileOptions;
use sbmlsim_host::{Instance, NativeInstance, Runtime};
use serde::{Deserialize, Serialize};

/// The arguments of `bench`.
#[derive(Args)]
pub struct BenchArgs {
    /// The benchmark file (`bench/models.toml`).
    pub models: PathBuf,
    /// Write the results as JSON.
    #[arg(long)]
    pub json: Option<PathBuf>,
    /// Only these models (comma separated names).
    #[arg(long)]
    pub only: Option<String>,
    /// The rounds of the configurations.
    #[arg(long, default_value_t = 3)]
    pub rounds: usize,
    /// The untimed simulations of a configuration before the timed ones, per round.
    #[arg(long, default_value_t = 3)]
    pub warmups: usize,
    /// The timed simulations of a configuration per round.
    #[arg(long, default_value_t = 20)]
    pub reps: usize,
    /// The compilations and instantiations of a model.
    #[arg(long, default_value_t = 5)]
    pub compile_reps: usize,
    /// The linear solvers (comma separated: dense, klu).
    #[arg(long, default_value = "dense,klu")]
    pub solvers: String,
    /// The Jacobians (comma separated: analytic, difference).
    #[arg(long, default_value = "analytic,difference")]
    pub jacobians: String,
    /// Write the models and the requests for the JavaScript hosts into this directory.
    #[arg(long)]
    pub export: Option<PathBuf>,
    /// The engine: `wasmtime` (`engine.wasm`) or `native` (the engine compiled for the machine,
    /// the model in wasmtime).
    #[arg(long, default_value = "wasmtime")]
    pub host: String,
}

/// A loaded model of the engine of `--host`.
enum Loaded {
    Wasm(Box<Instance>),
    Native(Box<NativeInstance>),
}

impl Loaded {
    fn simulate(
        &mut self,
        req: &SimulationRequest,
    ) -> anyhow::Result<sbmlsim_abi::SimulationResult> {
        Ok(match self {
            Loaded::Wasm(i) => i.simulate(req)?,
            Loaded::Native(i) => i.simulate(req)?,
        })
    }

    fn stats(&mut self) -> anyhow::Result<SimulationStats> {
        Ok(match self {
            Loaded::Wasm(i) => i.stats()?,
            Loaded::Native(i) => i.stats(),
        })
    }
}

/// A model of the benchmark file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchModel {
    /// The source the SBML was prepared from (`scripts/prepare_model.py`).
    pub source: String,
    /// The prepared SBML, relative to the benchmark file.
    pub sbml: PathBuf,
    /// The end time (from 0).
    pub end: f64,
    /// The uniform output points.
    pub points: u32,
    /// Values set before the initialization.
    #[serde(default)]
    pub preinit: Vec<(String, f64)>,
    /// The relative tolerance.
    pub relative_tolerance: f64,
    /// The absolute tolerance, scaled per state as roadrunner does.
    pub absolute_tolerance: f64,
    /// Whether the output of the steps of the integrator is measured too.
    #[serde(default)]
    pub steps: bool,
}

/// The benchmark file: the models in their order.
#[derive(Debug, Deserialize)]
struct BenchFile {
    /// The checkouts of the sources (read by `scripts/prepare_model.py`).
    #[serde(default)]
    #[allow(dead_code)]
    checkouts: BTreeMap<String, String>,
    /// The models.
    models: toml::Table,
}

/// Reads the models of a benchmark file in their order, with their SBML paths resolved.
pub fn read_models(path: &Path) -> anyhow::Result<Vec<(String, BenchModel)>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let file: BenchFile =
        toml::from_str(&text).with_context(|| format!("cannot read {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    file.models
        .into_iter()
        .map(|(name, value)| {
            let mut model: BenchModel = value
                .try_into()
                .with_context(|| format!("model {name} of {}", path.display()))?;
            model.sbml = base.join(&model.sbml);
            Ok((name, model))
        })
        .collect()
}

/// A configuration of a simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Config {
    solver: &'static str,
    jacobian: &'static str,
    output: &'static str,
}

impl Config {
    /// The name of the configuration, the name of its request file.
    fn name(&self) -> String {
        format!("{}_{}_{}", self.solver, self.jacobian, self.output)
    }
}

/// The result of a configuration.
#[derive(Debug, Clone, Serialize)]
struct ConfigResult {
    #[serde(flatten)]
    config: Config,
    /// The median over the rounds of the medians of the timed simulations, seconds.
    median_s: f64,
    /// The median of every round.
    round_medians_s: Vec<f64>,
    /// The rows of the result.
    n_rows: usize,
    /// Whether every timed simulation gave the bits of the first.
    repeat_bitwise: bool,
    /// The counters of the last simulation.
    stats: SimulationStats,
}

/// The result of a model.
#[derive(Debug, Clone, Serialize)]
struct ModelResult {
    name: String,
    source: String,
    states: usize,
    jacobian_entries: usize,
    events: usize,
    roots: u32,
    /// The size of `model.wasm` and of `model.json`.
    wasm_bytes: usize,
    json_bytes: usize,
    /// Whether the absolute tolerance is the vector of roadrunner (else the scalar).
    atol_per_state: bool,
    /// The median compilation of the SBML, seconds.
    compile_s: f64,
    /// The median instantiation in wasmtime (compile `model.wasm`, instances, load), seconds.
    instantiate_s: f64,
    /// The median of the calls of the right hand side through the host
    /// (`Runtime::model_functions`, the copies of the host included: an upper bound of one
    /// call inside the engine), seconds.
    rhs_call_s: f64,
    configs: Vec<ConfigResult>,
}

/// The results of the benchmark.
#[derive(Debug, Serialize)]
struct BenchResult {
    host: &'static str,
    models_file: String,
    rounds: usize,
    warmups: usize,
    reps: usize,
    models: Vec<ModelResult>,
}

/// The manifest of `--export`, which the JavaScript hosts read.
#[derive(Debug, Serialize)]
struct Manifest {
    rounds: usize,
    warmups: usize,
    reps: usize,
    models: Vec<ManifestModel>,
}

/// A model of the manifest: its directory (with `model.wasm`, `model.json` and the requests)
/// and its configurations.
#[derive(Debug, Serialize)]
struct ManifestModel {
    name: String,
    dir: String,
    configs: Vec<ManifestConfig>,
}

/// A configuration of the manifest and its request file.
#[derive(Debug, Serialize)]
struct ManifestConfig {
    #[serde(flatten)]
    config: Config,
    request: String,
}

/// The median of a list of times.
fn median(times: &[f64]) -> f64 {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    match n {
        0 => f64::NAN,
        _ if n % 2 == 1 => sorted[n / 2],
        _ => 0.5 * (sorted[n / 2 - 1] + sorted[n / 2]),
    }
}

/// Splits a comma separated list.
fn list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A comma separated list of `flag` whose every value is one of `known` (and which is not
/// empty), else an error naming the values.
fn checked_list(flag: &str, text: &str, known: &[&str]) -> anyhow::Result<Vec<String>> {
    let values = list(text);
    if values.is_empty() {
        bail!(
            "{flag} names nothing (one or more of: {})",
            known.join(", ")
        );
    }
    if let Some(unknown) = values.iter().find(|v| !known.contains(&v.as_str())) {
        bail!(
            "{flag}: unknown value {unknown} (one or more of: {})",
            known.join(", ")
        );
    }
    Ok(values)
}

/// The absolute tolerance of a model: the vector of roadrunner in the order of the states of
/// the description (from `<sbml>.atol.json`), else the scalar.
fn absolute_tolerance(
    model: &BenchModel,
    desc: &ModelDescription,
) -> anyhow::Result<(Tolerance, bool)> {
    let path = model.sbml.with_extension("atol.json");
    if !path.exists() {
        return Ok((Tolerance::Scalar(model.absolute_tolerance), false));
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let by_id: BTreeMap<String, f64> =
        serde_json::from_str(&text).with_context(|| format!("cannot read {}", path.display()))?;
    let vector = desc
        .states
        .iter()
        .map(|state| {
            by_id.get(&state.id).copied().with_context(|| {
                format!(
                    "{} has no tolerance of the state {} of the model",
                    path.display(),
                    state.id
                )
            })
        })
        .collect::<anyhow::Result<Vec<f64>>>()?;
    if vector.len() != by_id.len() {
        bail!(
            "{} has {} states, the model {}",
            path.display(),
            by_id.len(),
            vector.len()
        );
    }
    Ok((Tolerance::PerState(vector), true))
}

/// The request of a configuration.
fn request(
    model: &BenchModel,
    desc: &ModelDescription,
    atol: &Tolerance,
    config: &Config,
) -> SimulationRequest {
    let output = match config.output {
        "steps" => Output::IntegratorSteps,
        _ => Output::Uniform {
            steps: model.points.saturating_sub(1).max(1),
        },
    };
    let selections = std::iter::once("time".to_string())
        .chain(desc.states.iter().map(|s| s.id.clone()))
        .collect();
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end: model.end,
        output,
        preinit: model.preinit.clone(),
        changes: vec![],
        steady_state: None,
        selections,
        settings: IntegratorSettings {
            relative_tolerance: model.relative_tolerance,
            absolute_tolerance: atol.clone(),
            maximum_num_steps: 20000,
            stiff: true,
            linear_solver: match config.solver {
                "klu" => LinearSolver::Klu,
                _ => LinearSolver::Dense,
            },
            jacobian: match config.jacobian {
                "difference" => JacobianChoice::DifferenceQuotient,
                _ => JacobianChoice::Analytic,
            },
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The median time of `reps` calls of `f`, seconds.
fn time_median(reps: usize, mut f: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<f64> {
    let mut times = Vec::with_capacity(reps);
    for _ in 0..reps.max(1) {
        let start = Instant::now();
        f()?;
        times.push(start.elapsed().as_secs_f64());
    }
    Ok(median(&times))
}

/// The bits of the values of a result.
fn bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}

/// One round of a configuration: the median of the timed simulations, the rows, whether they
/// repeat bitwise and the counters.
fn measure(
    instance: &mut Loaded,
    req: &SimulationRequest,
    warmups: usize,
    reps: usize,
) -> anyhow::Result<(f64, usize, bool, SimulationStats)> {
    let mut first = None;
    for _ in 0..warmups {
        instance.simulate(req)?;
    }
    let mut times = Vec::with_capacity(reps);
    let mut bitwise = true;
    let mut n_rows = 0;
    for _ in 0..reps.max(1) {
        let start = Instant::now();
        let result = instance.simulate(req)?;
        times.push(start.elapsed().as_secs_f64());
        n_rows = result.n_rows;
        let b = bits(&result.values);
        match &first {
            None => first = Some(b),
            Some(f) => bitwise &= *f == b,
        }
    }
    Ok((median(&times), n_rows, bitwise, instance.stats()?))
}

/// Runs the benchmark; `Ok(true)` when every simulation repeated bitwise.
pub fn run(args: &BenchArgs) -> anyhow::Result<bool> {
    let models = read_models(&args.models)?;
    let names: Vec<&str> = models.iter().map(|(n, _)| n.as_str()).collect();
    let only = args
        .only
        .as_deref()
        .map(|o| checked_list("--only", o, &names))
        .transpose()?;
    let solvers = checked_list("--solvers", &args.solvers, &["dense", "klu"])?;
    let jacobians = checked_list("--jacobians", &args.jacobians, &["analytic", "difference"])?;
    let host: &'static str = match args.host.as_str() {
        "wasmtime" => "wasmtime",
        "native" => "native",
        other => bail!("--host: unknown value {other} (wasmtime or native)"),
    };
    let runtime = Runtime::embedded()?;
    let load = |wasm: &[u8], desc: &ModelDescription| -> anyhow::Result<Loaded> {
        Ok(match host {
            "native" => Loaded::Native(Box::new(runtime.instantiate_native(wasm, desc)?)),
            _ => Loaded::Wasm(Box::new(runtime.instantiate(wasm, desc)?)),
        })
    };
    let mut results = Vec::new();
    let mut manifest = Manifest {
        rounds: args.rounds,
        warmups: args.warmups,
        reps: args.reps,
        models: Vec::new(),
    };
    for (name, model) in &models {
        if only.as_ref().is_some_and(|o| !o.contains(name)) {
            continue;
        }
        let sbml = fs::read_to_string(&model.sbml).with_context(|| {
            format!(
                "cannot read {} (prepare it: uv run scripts/prepare_model.py --bench {})",
                model.sbml.display(),
                args.models.display()
            )
        })?;
        let options = CompileOptions::default();
        let compiled = sbmlsim_compiler::compile(&sbml, &options)
            .with_context(|| format!("cannot compile {name}"))?;
        let compile_s = time_median(args.compile_reps, || {
            sbmlsim_compiler::compile(&sbml, &options)?;
            Ok(())
        })?;
        let desc = compiled.description.clone();
        let instantiate_s = time_median(args.compile_reps, || {
            load(&compiled.wasm, &desc)?;
            Ok(())
        })?;
        let json = crate::testsuite::description_json(&desc)?;
        let (atol, atol_per_state) = absolute_tolerance(model, &desc)?;
        // the right hand side at the initial state of the description
        let rhs_call_s = {
            let mut functions = runtime.model_functions(&compiled.wasm, &desc)?;
            let y: Vec<f64> = desc.states.iter().map(|s| s.initial).collect();
            let p: Vec<f64> = desc.parameters.iter().map(|s| s.initial).collect();
            let calls = 2000;
            let mut samples = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                for _ in 0..calls {
                    functions.rhs(0.0, &y, &p)?;
                }
                samples.push(start.elapsed().as_secs_f64() / calls as f64);
            }
            median(&samples)
        };
        let mut configs = Vec::new();
        let outputs: &[&'static str] = if model.steps {
            &["times", "steps"]
        } else {
            &["times"]
        };
        for output in outputs {
            for solver in ["dense", "klu"] {
                for jacobian in ["analytic", "difference"] {
                    if solvers.iter().any(|s| s == solver)
                        && jacobians.iter().any(|j| j == jacobian)
                    {
                        configs.push(Config {
                            solver,
                            jacobian,
                            output,
                        });
                    }
                }
            }
        }
        let requests: Vec<SimulationRequest> = configs
            .iter()
            .map(|c| request(model, &desc, &atol, c))
            .collect();
        if let Some(dir) = &args.export {
            let model_dir = dir.join(name);
            fs::create_dir_all(&model_dir)
                .with_context(|| format!("cannot create {}", model_dir.display()))?;
            fs::write(model_dir.join("model.wasm"), &compiled.wasm)?;
            fs::write(model_dir.join("model.json"), &json)?;
            let mut entries = Vec::new();
            for (config, req) in configs.iter().zip(&requests) {
                let file = format!("{}.json", config.name());
                fs::write(model_dir.join(&file), serde_json::to_vec(req)?)?;
                entries.push(ManifestConfig {
                    config: *config,
                    request: file,
                });
            }
            manifest.models.push(ManifestModel {
                name: name.clone(),
                dir: name.clone(),
                configs: entries,
            });
        }
        let mut instance = load(&compiled.wasm, &desc)?;
        let mut rounds: Vec<Vec<f64>> = vec![Vec::new(); configs.len()];
        let mut last = vec![(0usize, true, SimulationStats::default()); configs.len()];
        for round in 0..args.rounds.max(1) {
            for (k, req) in requests.iter().enumerate() {
                let (median_s, n_rows, bitwise, stats) =
                    measure(&mut instance, req, args.warmups, args.reps)
                        .with_context(|| format!("{name} {}", configs[k].name()))?;
                rounds[k].push(median_s);
                last[k] = (n_rows, last[k].1 && bitwise, stats);
            }
            eprintln!(
                "{name}: round {}/{}: {}",
                round + 1,
                args.rounds.max(1),
                configs
                    .iter()
                    .zip(&rounds)
                    .map(|(c, r)| format!("{} {:.4} ms", c.name(), r[round] * 1e3))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let configs = configs
            .iter()
            .zip(rounds)
            .zip(last)
            .map(
                |((config, round_medians_s), (n_rows, repeat_bitwise, stats))| ConfigResult {
                    config: *config,
                    median_s: median(&round_medians_s),
                    round_medians_s,
                    n_rows,
                    repeat_bitwise,
                    stats,
                },
            )
            .collect();
        results.push(ModelResult {
            name: name.clone(),
            source: model.source.clone(),
            states: desc.states.len(),
            jacobian_entries: desc.jacobian.pattern().nnz(),
            events: desc.events.len(),
            roots: desc.n_roots,
            wasm_bytes: compiled.wasm.len(),
            json_bytes: json.len(),
            atol_per_state,
            compile_s,
            instantiate_s,
            rhs_call_s,
            configs,
        });
    }
    print_table(&results);
    let all_bitwise = results
        .iter()
        .all(|m| m.configs.iter().all(|c| c.repeat_bitwise));
    let result = BenchResult {
        host,
        models_file: args.models.display().to_string(),
        rounds: args.rounds,
        warmups: args.warmups,
        reps: args.reps,
        models: results,
    };
    if let Some(path) = &args.json {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(path, serde_json::to_vec_pretty(&result)?)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if let Some(dir) = &args.export {
        fs::write(
            dir.join("bench.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
    }
    Ok(all_bitwise)
}

/// Prints the results as a table.
fn print_table(results: &[ModelResult]) {
    println!(
        "{:<14} {:>6} {:>7} {:>10} {:>12} {:>6} {:>10} {:>11} {:>7} {:>8} {:>6} {:>7}",
        "model",
        "states",
        "solver",
        "jacobian",
        "output",
        "rows",
        "median ms",
        "compile ms",
        "inst ms",
        "steps",
        "jac",
        "rhs"
    );
    for m in results {
        for c in &m.configs {
            println!(
                "{:<14} {:>6} {:>7} {:>10} {:>12} {:>6} {:>10.4} {:>11.2} {:>7.2} {:>8} {:>6} {:>7}",
                m.name,
                m.states,
                c.config.solver,
                c.config.jacobian,
                c.config.output,
                c.n_rows,
                c.median_s * 1e3,
                m.compile_s * 1e3,
                m.instantiate_s * 1e3,
                c.stats.steps,
                c.stats.jacobian_evals,
                c.stats.all_rhs_evals(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_benchmark_file_is_read_in_its_order() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/models.toml");
        let models = read_models(&path).unwrap();
        let names: Vec<&str> = models.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "hctz",
                "midazolam",
                "repeated_dose",
                "mp_core",
                "chen",
                "mp_flat"
            ]
        );
        for (_, model) in &models {
            assert_eq!(model.points, 201);
            assert_eq!(model.relative_tolerance, 1e-6);
            assert_eq!(model.absolute_tolerance, 1e-12);
            assert!(model.sbml.starts_with(path.parent().unwrap()));
        }
    }

    #[test]
    fn unknown_values_of_a_list_are_refused() {
        let known = ["dense", "klu"];
        assert_eq!(
            checked_list("--solvers", "klu, dense", &known).unwrap(),
            ["klu", "dense"]
        );
        let error = checked_list("--solvers", "dense,KLU", &known).unwrap_err();
        assert!(error.to_string().contains("unknown value KLU"), "{error}");
        let error = checked_list("--solvers", "dense,kluu", &known).unwrap_err();
        assert!(error.to_string().contains("unknown value kluu"), "{error}");
        assert!(checked_list("--solvers", " , ", &known).is_err());
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }
}

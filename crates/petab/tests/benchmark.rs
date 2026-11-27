//! The PEtab benchmark collection in Rust: every problem of the pinned commit of sbmlsim, read,
//! compiled and evaluated at its nominal values, against `tests/data/benchmark_baseline.json`.
//!
//! The problems are the ones sbmlsim downloads and converts to PEtab v2
//! (`uv run scripts/petab_benchmark.py download`, which runs the download of sbmlsim,
//! `sbmlsim.fit.petab_v2.benchmark`) into `~/.cache/sbmlsim/petab-benchmark/<commit>`, or the
//! directory `SBMLSIM_BENCHMARK_PATH` names. Without them the test passes with a message, as the
//! PEtab test suite does. The collection has every problem in PEtab v1, which sbmlsim converts
//! with `petab.v2.petab1to2` of python, so the converted problems are the input here, as the
//! converted cases of PEtab v1 are for the test suite.
//!
//! A problem is evaluated as sbmlsim's runner of the collection evaluates it in the core: the
//! integrator at the tolerances `1e-8` (relative) and `1e-10` (absolute), tighter for the
//! problems of `PROBLEM_SETTINGS`, a measurement at a change reading the state after it. The
//! simulation of every measurement is compared with the simulations of the collection
//! (`simulations.tsv`, converted to PEtab v2 with the problem) at `1e-3` absolute and relative,
//! the log-likelihood with the one of AMICI (`amici.yaml`) at `1e-3` absolute and `1e-6`
//! relative, both by the criterion of the SBML Test Suite, `|c - u| <= atol + rtol |c|`.
//!
//! The test fails when a problem which the baseline does not list fails, when a problem which
//! it lists passes and when a problem fails otherwise than the baseline says: a change which
//! fixes a problem updates the baseline
//! (`SBMLSIM_PETAB_BENCHMARK_BASELINE=write cargo test --release -p sbmlsim-petab --test
//! benchmark -- --nocapture`), which keeps the reasons of the problems which still fail as they
//! did.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_host::Runtime;
use sbmlsim_petab::{DataAtChange, Executor, Options, PetabProblem, Plan};
use serde::{Deserialize, Serialize};

/// The commit of the collection, the `BENCHMARK_COMMIT` of sbmlsim.
const COMMIT: &str = "c44d5ede9bf979db5aa8ae4e94fc31fba9776675";

/// The problems which are not run, the `EXCLUDED_PROBLEMS` of sbmlsim.
const EXCLUDED: [(&str, &str); 1] = [(
    "Froehlich_CellSystems2018",
    "9169 experiments with a pre-equilibration each take longer than the 600 s a problem gets",
)];

/// The tolerances of the integrator, relative and absolute, the `BENCHMARK_SETTINGS` of sbmlsim.
const TOLERANCES: (f64, f64) = (1e-8, 1e-10);

/// The problems which need a tighter integrator, the `PROBLEM_SETTINGS` of sbmlsim:
/// `EarlyEffector` of `Crauste_CellSystems2017` grows towards a blow up in finite time.
const PROBLEM_TOLERANCES: [(&str, (f64, f64)); 1] = [("Crauste_CellSystems2017", (1e-12, 1e-14))];

/// The absolute and relative tolerance of a simulation.
const SIMULATION_TOLERANCE: f64 = 1e-3;

/// The absolute and the relative tolerance of the log-likelihood.
const LLH_TOLERANCE: (f64, f64) = (1e-3, 1e-6);

/// The outcome of a problem, the `BenchmarkStatus` of sbmlsim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    /// The simulations and the log-likelihood agree.
    Pass,
    /// A simulation or the log-likelihood is outside of its tolerance.
    Tolerance,
    /// The problem was not converted to PEtab v2.
    Conversion,
    /// The problem cannot be read, compiled or simulated.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Expected {
    status: Status,
    reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Baseline {
    suite_commit: String,
    n_cases: usize,
    n_passed: usize,
    expected_failures: BTreeMap<String, Expected>,
}

/// The outcome of a problem with what it took.
struct Outcome {
    status: Status,
    message: String,
    llh: Option<f64>,
    /// Seconds to read the files, to plan and compile, to simulate and evaluate.
    timings: [f64; 3],
}

impl Outcome {
    fn failed(status: Status, message: impl Into<String>, timings: [f64; 3]) -> Outcome {
        Outcome {
            status,
            message: message.into(),
            llh: None,
            timings,
        }
    }
}

fn collection() -> Option<PathBuf> {
    let path = match std::env::var("SBMLSIM_BENCHMARK_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .ok()?
            .join("sbmlsim")
            .join("petab-benchmark")
            .join(COMMIT),
    };
    (path.join("problems").is_dir() && path.join("v2").is_dir()).then_some(path)
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("benchmark_baseline.json")
}

/// A number of a table, `inf`, `-inf` and `nan` included.
fn number(text: &str) -> Option<f64> {
    match text.trim().to_ascii_lowercase().as_str() {
        "inf" | "+inf" | "infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        "nan" | "" => Some(f64::NAN),
        other => other.parse().ok(),
    }
}

/// The criterion of the SBML Test Suite: both finite and `|c - u| <= atol + rtol |c|`.
fn within(expected: f64, value: f64, absolute: f64, relative: f64) -> bool {
    expected.is_finite()
        && value.is_finite()
        && (expected - value).abs() <= absolute + relative * expected.abs()
}

/// The log-likelihoods of AMICI by the name of the problem.
fn references(path: &Path) -> HashMap<String, f64> {
    let Ok(text) = std::fs::read_to_string(path.join("amici.yaml")) else {
        return HashMap::new();
    };
    let content: serde_norway::Value = serde_norway::from_str(&text).expect("amici.yaml");
    let mut references = HashMap::new();
    if let serde_norway::Value::Mapping(entries) = content {
        for (name, entry) in entries {
            let llh = match entry.get("llh") {
                Some(serde_norway::Value::Number(llh)) => llh.as_f64(),
                Some(serde_norway::Value::String(llh)) => number(llh),
                _ => None,
            };
            if let (Some(name), Some(llh)) = (name.as_str(), llh) {
                references.insert(name.to_string(), llh);
            }
        }
    }
    references
}

/// The key of the simulation of a measurement: the observable, the experiment (empty without
/// one) and the time.
type Key = (String, String, f64);

/// A table of the simulations of the collection.
struct Simulations {
    keys: Vec<Key>,
    values: Vec<f64>,
}

fn read_simulations(path: &Path) -> Result<Simulations, String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header: Vec<&str> = lines
        .next()
        .ok_or("an empty table of simulations")?
        .split('\t')
        .map(str::trim)
        .collect();
    let column = |name: &str| header.iter().position(|c| *c == name);
    let observable = column("observableId").ok_or("no observableId")?;
    let time = column("time").ok_or("no time")?;
    let simulation = column("simulation").ok_or("no simulation")?;
    let experiment = column("experimentId");
    let mut keys = Vec::new();
    let mut values = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
        let cell = |k: usize| cells.get(k).copied().unwrap_or("");
        keys.push((
            cell(observable).to_string(),
            experiment.map(cell).unwrap_or("").to_string(),
            number(cell(time)).ok_or("a time which is no number")?,
        ));
        values.push(number(cell(simulation)).ok_or("a simulation which is no number")?);
    }
    Ok(Simulations { keys, values })
}

/// Whether two columns of ids agree row by row, up to a renaming: the columns have no id in
/// common and every id of `other` stands for one id of `ids`, e.g. an observable whose
/// simulations are written per parameters and conditions (`Isensee_JCB2018`).
fn same_or_renamed(ids: &[&str], other: &[&str]) -> bool {
    if ids == other {
        return true;
    }
    let mine: HashSet<&str> = ids.iter().copied().collect();
    if other.iter().any(|id| mine.contains(id)) {
        return false;
    }
    let mut original: HashMap<&str, &str> = HashMap::new();
    ids.iter()
        .zip(other)
        .all(|(a, b)| original.entry(b).or_insert(a) == a)
}

/// `numpy.allclose` of two times, equal infinities and `NaN`s included.
fn close(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-8 + 1e-5 * b.abs()
}

/// Whether the simulations are row by row the ones of the measurements, up to a renaming of
/// the observables and of the experiments after the simulations were written.
fn in_order(measurements: &[Key], expected: &[Key]) -> bool {
    if measurements.len() != expected.len() {
        return false;
    }
    if !measurements
        .iter()
        .zip(expected)
        .all(|(m, e)| close(m.2, e.2))
    {
        return false;
    }
    let column = |keys: &[Key], k: usize| -> Vec<String> {
        keys.iter()
            .map(|key| if k == 0 { key.0.clone() } else { key.1.clone() })
            .collect()
    };
    (0..2).all(|k| {
        let (ids, other) = (column(measurements, k), column(expected, k));
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let other: Vec<&str> = other.iter().map(String::as_str).collect();
        same_or_renamed(&ids, &other)
    })
}

/// The observables of the simulations renamed to the ones of the measurements: an id which is
/// no observable of the measurements is the observable whose id it starts or ends with, if
/// there is exactly one (`Raia_CancerResearch2011`, `Isensee_JCB2018`).
fn with_observables_of(measurements: &[Key], expected: &[Key]) -> Vec<Key> {
    let observables: HashSet<&str> = measurements.iter().map(|key| key.0.as_str()).collect();
    let mut renamed: HashMap<String, String> = HashMap::new();
    for key in expected {
        if observables.contains(key.0.as_str()) || renamed.contains_key(&key.0) {
            continue;
        }
        let candidates: Vec<&str> = observables
            .iter()
            .copied()
            .filter(|o| key.0.starts_with(o) || o.ends_with(key.0.as_str()))
            .collect();
        if let [only] = candidates.as_slice() {
            renamed.insert(key.0.clone(), (*only).to_string());
        }
    }
    expected
        .iter()
        .map(|key| {
            let observable = renamed
                .get(&key.0)
                .cloned()
                .unwrap_or_else(|| key.0.clone());
            (observable, key.1.clone(), key.2)
        })
        .collect()
}

/// The simulation of the collection of every measurement which has one: the rows of the
/// measurements and their simulations, see `_match` of sbmlsim.
fn matched(measurements: &[Key], expected: &Simulations) -> (Vec<usize>, Vec<f64>) {
    if in_order(measurements, &expected.keys) {
        return ((0..measurements.len()).collect(), expected.values.clone());
    }
    let keys = with_observables_of(measurements, &expected.keys);
    let mut available: HashMap<(String, String, u64), VecDeque<f64>> = HashMap::new();
    for (key, value) in keys.into_iter().zip(&expected.values) {
        available
            .entry((key.0, key.1, key.2.to_bits()))
            .or_default()
            .push_back(*value);
    }
    let mut rows = Vec::new();
    let mut values = Vec::new();
    for (k, key) in measurements.iter().enumerate() {
        if let Some(value) = available
            .get_mut(&(key.0.clone(), key.1.clone(), key.2.to_bits()))
            .and_then(VecDeque::pop_front)
        {
            rows.push(k);
            values.push(value);
        }
    }
    (rows, values)
}

/// Compares the simulation of every measurement with the collection: the failures.
fn compare(directory: &Path, plan: &Plan, simulated: &[f64]) -> Result<Vec<String>, String> {
    let path = directory.join("simulations.tsv");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let expected = read_simulations(&path)?;
    let measurements: Vec<Key> = plan
        .measurements
        .iter()
        .map(|m| {
            (
                m.observable.clone(),
                m.experiment.clone().unwrap_or_default(),
                m.time,
            )
        })
        .collect();
    let (rows, references) = matched(&measurements, &expected);
    let mut failures = Vec::new();
    if rows.len() < simulated.len() {
        failures.push(format!(
            "'{}' of the '{}' measurements have no simulation in the collection",
            simulated.len() - rows.len(),
            simulated.len()
        ));
    }
    let mut failed: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (&row, &reference) in rows.iter().zip(&references) {
        let value = simulated[row];
        let agree = within(reference, value, SIMULATION_TOLERANCE, SIMULATION_TOLERANCE)
            || (value.is_nan() && reference.is_nan());
        if !agree {
            let m = &measurements[row];
            *failed.entry((m.0.clone(), m.1.clone())).or_default() += 1;
        }
    }
    if !failed.is_empty() {
        let count: usize = failed.values().sum();
        let which: Vec<String> = failed
            .iter()
            .map(|((observable, experiment), n)| {
                let experiment = if experiment.is_empty() {
                    "no experiment"
                } else {
                    experiment
                };
                format!("{observable} in {experiment} ({n})")
            })
            .collect();
        failures.push(format!(
            "'{count}' of '{}' simulations differ: {}",
            rows.len(),
            which.join(", ")
        ));
    }
    Ok(failures)
}

/// Reads, compiles, evaluates and compares a problem.
fn run_problem(path: &Path, name: &str, reference: Option<f64>, runtime: &Runtime) -> Outcome {
    let mut timings = [0.0; 3];
    let directory = path.join("v2").join(name);
    if !directory.is_dir() {
        let error = path.join("v2").join(format!("{name}.error"));
        let message = std::fs::read_to_string(error)
            .map(|text| text.trim().to_string())
            .unwrap_or_else(|_| "the problem was not converted to PEtab v2".to_string());
        return Outcome::failed(Status::Conversion, message, timings);
    }
    let mut yaml: Vec<PathBuf> = std::fs::read_dir(&directory)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|e| e == "yaml"))
                .collect()
        })
        .unwrap_or_default();
    yaml.sort();
    let Some(yaml) = yaml.first() else {
        return Outcome::failed(Status::Error, "the problem has no YAML file", timings);
    };

    let start = Instant::now();
    let problem = match PetabProblem::from_path(yaml) {
        Ok(problem) => problem,
        Err(err) => return Outcome::failed(Status::Error, format!("read: {err}"), timings),
    };
    timings[0] = start.elapsed().as_secs_f64();

    let start = Instant::now();
    let (relative, absolute) = PROBLEM_TOLERANCES
        .iter()
        .find(|(problem, _)| *problem == name)
        .map_or(TOLERANCES, |(_, tolerances)| *tolerances);
    let options = Options {
        integrator: Some(IntegratorSettings {
            relative_tolerance: relative,
            absolute_tolerance: Tolerance::Scalar(absolute),
            ..IntegratorSettings::default()
        }),
        data_at_change: Some(DataAtChange::After),
        ..Options::default()
    };
    let plan = match Plan::new(&problem, &options) {
        Ok(plan) => plan,
        Err(err) => return Outcome::failed(Status::Error, format!("plan: {err}"), timings),
    };
    let executor = match Executor::with_runtime(plan, runtime) {
        Ok(executor) => executor,
        Err(err) => return Outcome::failed(Status::Error, format!("compile: {err}"), timings),
    };
    timings[1] = start.elapsed().as_secs_f64();

    let start = Instant::now();
    let x = executor.plan().nominal();
    let evaluation = executor.evaluate(&x);
    timings[2] = start.elapsed().as_secs_f64();
    if !evaluation.failures.is_empty() {
        let messages: Vec<String> = evaluation
            .failures
            .iter()
            .map(|(simulation, message)| format!("simulation {simulation}: {message}"))
            .collect();
        return Outcome::failed(Status::Error, messages.join("; "), timings);
    }

    let mut failures = match compare(&directory, executor.plan(), &evaluation.simulated) {
        Ok(failures) => failures,
        Err(err) => return Outcome::failed(Status::Error, err, timings),
    };
    if let Some(reference) = reference
        && !within(reference, evaluation.llh, LLH_TOLERANCE.0, LLH_TOLERANCE.1)
    {
        failures.push(format!(
            "llh {} differs from the llh {reference} of AMICI",
            evaluation.llh
        ));
    }
    Outcome {
        status: if failures.is_empty() {
            Status::Pass
        } else {
            Status::Tolerance
        },
        message: failures.join("; "),
        llh: Some(evaluation.llh),
        timings,
    }
}

#[test]
fn the_petab_benchmark_collection_passes_its_baseline() {
    let Some(path) = collection() else {
        eprintln!(
            "the PEtab benchmark collection {COMMIT} is not downloaded; \
             `uv run scripts/petab_benchmark.py download` fetches and converts it"
        );
        return;
    };
    let references = references(&path);
    let mut names: Vec<String> = std::fs::read_dir(path.join("problems"))
        .expect("the problems of the collection")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|problem| problem.join("v1").is_dir() || problem.join("v2").is_dir())
        .filter_map(|problem| {
            problem
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .filter(|name| EXCLUDED.iter().all(|(excluded, _)| excluded != name))
        .collect();
    names.sort();

    let runtime = Runtime::embedded().expect("the embedded engine");
    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    eprintln!(
        "{:<34} {:>9} {:>9} {:>9}  outcome",
        "problem", "read s", "compile s", "eval s"
    );
    for name in &names {
        let outcome = run_problem(&path, name, references.get(name).copied(), &runtime);
        let llh = outcome
            .llh
            .map_or(String::new(), |llh| format!(" llh {llh:.10}"));
        eprintln!(
            "{name:<34} {:>9.3} {:>9.3} {:>9.3}  {:?}{llh} {}",
            outcome.timings[0],
            outcome.timings[1],
            outcome.timings[2],
            outcome.status,
            outcome.message
        );
        outcomes.insert(name.clone(), outcome);
    }
    let totals: Vec<f64> = (0..3)
        .map(|k| outcomes.values().map(|o| o.timings[k]).sum())
        .collect();
    let passed = outcomes
        .values()
        .filter(|outcome| outcome.status == Status::Pass)
        .count();
    eprintln!(
        "the PEtab benchmark collection: {passed} of {} problems pass; read {:.2} s, \
         compile {:.2} s, evaluate {:.2} s",
        outcomes.len(),
        totals[0],
        totals[1],
        totals[2]
    );

    let previous: Option<Baseline> = std::fs::read_to_string(baseline_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    if std::env::var("SBMLSIM_PETAB_BENCHMARK_BASELINE").as_deref() == Ok("write") {
        let expected_failures = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.status != Status::Pass)
            .map(|(name, outcome)| {
                // a problem which fails as it did keeps the reason of the baseline
                let reason = previous
                    .as_ref()
                    .and_then(|b| b.expected_failures.get(name))
                    .filter(|e| e.status == outcome.status)
                    .map_or_else(|| outcome.message.clone(), |e| e.reason.clone());
                (
                    name.clone(),
                    Expected {
                        status: outcome.status,
                        reason,
                    },
                )
            })
            .collect();
        let baseline = Baseline {
            suite_commit: COMMIT.to_string(),
            n_cases: outcomes.len(),
            n_passed: passed,
            expected_failures,
        };
        let text = serde_json::to_string_pretty(&baseline).expect("the baseline as JSON");
        std::fs::write(baseline_path(), text + "\n").expect("the baseline is written");
        return;
    }
    let baseline = previous.expect("the baseline of the collection");
    assert_eq!(
        baseline.suite_commit, COMMIT,
        "the baseline is of another commit"
    );
    let mut differences = Vec::new();
    for (name, outcome) in &outcomes {
        match (baseline.expected_failures.get(name), outcome.status) {
            (None, Status::Pass) => {}
            (None, status) => differences.push(format!(
                "{name} is {status:?} and not in the baseline: {}",
                outcome.message
            )),
            (Some(_), Status::Pass) => {
                differences.push(format!("{name} passes, remove it from the baseline"));
            }
            (Some(expected), status) if expected.status != status => differences.push(format!(
                "{name} is {status:?}, the baseline says {:?}: {}",
                expected.status, outcome.message
            )),
            (Some(_), _) => {}
        }
    }
    for name in baseline.expected_failures.keys() {
        if !outcomes.contains_key(name) {
            differences.push(format!(
                "{name} of the baseline is no problem of the collection"
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "the collection differs from its baseline:\n{}",
        differences.join("\n")
    );
    assert_eq!(
        (baseline.n_cases, baseline.n_passed),
        (outcomes.len(), passed),
        "the counts of the baseline"
    );
}

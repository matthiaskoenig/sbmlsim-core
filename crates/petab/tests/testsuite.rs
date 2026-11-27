//! The PEtab test suite in Rust: every case of a model and every case of the math of the
//! pinned commit of sbmlsim, against `tests/data/petab_baseline.json`.
//!
//! The cases are the ones sbmlsim downloads (`python scripts/petab_testsuite.py download` of
//! sbmlsim, or `python scripts/petab_testsuite.py download` of this repository for the cases
//! of PEtab v2 alone) into `~/.cache/sbmlsim/petab-test-suite/<commit>`, or the directory
//! `petabtests/cases` of a checkout which `SBMLSIM_PETAB_SUITE_PATH` names. Without them the
//! test passes with a message, as the test suite of SBML does.
//!
//! A case is read, planned and evaluated at the nominal values of its parameter table with
//! the tolerances of the integrator of sbmlsim's run of the suite (`1e-12`), and its
//! simulations, `chi2`, `llh`, log priors and log posterior are compared with its solution at
//! the tolerances the solution gives. The test fails when a case which the baseline does not
//! list fails, and when a case which it lists passes: a change which fixes a case updates the
//! baseline (`SBMLSIM_PETAB_BASELINE=write cargo test -p sbmlsim-petab --test testsuite`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_host::Runtime;
use sbmlsim_petab::math::{Formula, parse};
use sbmlsim_petab::{Executor, Options, PetabProblem, Plan};
use serde::{Deserialize, Serialize};

/// The commit of the suite, the `PETAB_SUITE_COMMIT` of sbmlsim.
const COMMIT: &str = "d09985e7bb03280b461599a24337137533d931e2";

/// The tolerance of a math case, the `MATH_TOLERANCE` of sbmlsim.
const MATH_TOLERANCE: f64 = 1e-12;

/// The groups of cases, by their directory below the cases of the suite.
const GROUPS: [(&str, &str); 2] = [
    ("sbml", "v2.0.0/sbml"),
    ("v1/sbml", "v1.0.0-converted/sbml"),
];

#[derive(Debug, Serialize, Deserialize)]
struct Baseline {
    suite_commit: String,
    n_cases: usize,
    n_passed: usize,
    expected_failures: BTreeMap<String, String>,
}

fn suite() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SBMLSIM_PETAB_SUITE_PATH") {
        return Some(PathBuf::from(path));
    }
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok()?;
    let path = cache.join("sbmlsim").join("petab-test-suite").join(COMMIT);
    path.join("v2.0.0").is_dir().then_some(path)
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("petab_baseline.json")
}

/// A number of a solution or of a table, `inf` and `-inf` included.
fn number(value: &serde_norway::Value) -> Option<f64> {
    match value {
        serde_norway::Value::Number(number) => number.as_f64(),
        serde_norway::Value::String(text) => text_number(text),
        _ => None,
    }
}

fn text_number(text: &str) -> Option<f64> {
    match text.trim().to_ascii_lowercase().as_str() {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        other => other.parse().ok(),
    }
}

/// The difference of a value to its reference, zero for equal infinities.
fn difference(value: f64, expected: f64) -> f64 {
    if value == expected {
        0.0
    } else {
        (value - expected).abs()
    }
}

/// Runs a case of a model: `Ok` when it passes, the reason when it does not.
fn run_case(directory: &Path, runtime: &Runtime) -> Result<(), String> {
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("no case")?;
    if directory.join("conversion.error").is_file() {
        let reason =
            std::fs::read_to_string(directory.join("conversion.error")).unwrap_or_default();
        return Err(format!("not converted: {}", reason.trim()));
    }
    let solution: serde_norway::Value = serde_norway::from_str(
        &std::fs::read_to_string(directory.join(format!("_{id}_solution.yaml")))
            .map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())?;
    let field = |name: &str| solution.get(name).and_then(number);

    let problem = PetabProblem::from_path(directory.join(format!("_{id}.yaml")))
        .map_err(|err| format!("read: {err}"))?;
    let settings = IntegratorSettings {
        relative_tolerance: 1e-12,
        absolute_tolerance: Tolerance::Scalar(1e-12),
        ..IntegratorSettings::default()
    };
    let options = Options {
        integrator: Some(settings),
        ..Options::default()
    };
    let plan = Plan::new(&problem, &options).map_err(|err| format!("plan: {err}"))?;
    let executor =
        Executor::with_runtime(plan, runtime).map_err(|err| format!("compile: {err}"))?;
    let x = executor.plan().nominal();
    let evaluation = executor.evaluate(&x);
    if let Some((simulation, message)) = evaluation.failures.first() {
        return Err(format!("simulation {simulation}: {message}"));
    }

    let mut failed = Vec::new();
    let mut compare = |name: String, value: f64, expected: f64, tolerance: f64| {
        let delta = difference(value, expected);
        // a difference of `NaN` is a failure as well
        if delta.is_nan() || delta > tolerance {
            failed.push(format!(
                "{name}: {value} != {expected} (difference {delta:.3e} > {tolerance:.0e})"
            ));
        }
    };
    for (name, value) in [("llh", evaluation.llh), ("chi2", evaluation.chi2)] {
        let expected = field(name).ok_or(format!("the solution has no {name}"))?;
        let tolerance = field(&format!("tol_{name}")).unwrap_or(0.0);
        compare(name.to_string(), value, expected, tolerance);
    }
    if let Some(serde_norway::Value::Mapping(priors)) = solution.get("log_prior") {
        let tolerance = field("tol_log_prior").unwrap_or(0.0);
        let mine: HashMap<&str, f64> = evaluation
            .log_priors
            .iter()
            .map(|(id, value)| (id.as_str(), *value))
            .collect();
        for (id, expected) in priors {
            let id = id.as_str().unwrap_or_default();
            let value = *mine.get(id).ok_or(format!(
                "the log prior of '{id}' is no parameter of the fit"
            ))?;
            compare(
                format!("log_prior {id}"),
                value,
                number(expected).unwrap_or(f64::NAN),
                tolerance,
            );
        }
    }
    if let Some(expected) = field("unnorm_log_posterior") {
        let tolerance = field("tol_unnorm_log_posterior").unwrap_or(0.0);
        compare(
            "unnorm_log_posterior".to_string(),
            evaluation.unnorm_log_posterior(),
            expected,
            tolerance,
        );
    }

    // the simulations, each matched with an unmatched measurement of its observable,
    // experiment and time
    let tolerance = field("tol_simulations").unwrap_or(0.0);
    let files = match solution.get("simulation_files") {
        Some(serde_norway::Value::Sequence(files)) => files.clone(),
        _ => Vec::new(),
    };
    let plan = executor.plan();
    let mut used = vec![false; plan.measurements.len()];
    let mut rows = 0;
    for file in files {
        let name = file.as_str().ok_or("a simulation file is no name")?;
        let text = std::fs::read_to_string(directory.join(name)).map_err(|err| err.to_string())?;
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header: Vec<&str> = lines
            .next()
            .ok_or("an empty simulation file")?
            .split('\t')
            .collect();
        let column = |name: &str| header.iter().position(|c| c.trim() == name);
        let (observable, time, simulation) = (
            column("observableId").ok_or("no observableId")?,
            column("time").ok_or("no time")?,
            column("simulation").ok_or("no simulation")?,
        );
        let experiment = column("experimentId");
        for line in lines {
            rows += 1;
            let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
            let cell = |k: usize| cells.get(k).copied().unwrap_or("");
            let at = text_number(cell(time)).ok_or("a time which is no number")?;
            let experiment = experiment.map(cell).filter(|e| !e.is_empty());
            let k = (0..plan.measurements.len())
                .find(|&k| {
                    let m = &plan.measurements[k];
                    !used[k]
                        && m.observable == cell(observable)
                        && m.experiment.as_deref() == experiment
                        && m.time == at
                })
                .ok_or(format!(
                    "the simulation of '{}' at {at} is of no measurement",
                    cell(observable)
                ))?;
            used[k] = true;
            let expected =
                text_number(cell(simulation)).ok_or("a simulation which is no number")?;
            compare(
                format!("simulation {} at {at}", cell(observable)),
                evaluation.simulated[k],
                expected,
                tolerance,
            );
        }
    }
    if rows != plan.measurements.len() {
        return Err(format!(
            "the solution has {rows} simulations of the {} measurements",
            plan.measurements.len()
        ));
    }
    // the fit of the engine: its cost is the negative log posterior at its start, plus the
    // offsets of its rows; a problem it cannot carry is a gap of the fit and no failure
    match executor.problem(1) {
        Ok(problem) => {
            let start = problem.start();
            let values = problem.values(&start);
            let cost = problem.cost(&start).map_err(|err| format!("fit: {err}"))?;
            let offset = executor
                .cost_offset()
                .map_err(|err| format!("fit: {err}"))?;
            let evaluation = executor.evaluate(&values);
            // the maximum likelihood of a problem without priors, the posterior of one with
            let posterior = if executor.plan().has_priors() {
                evaluation.unnorm_log_posterior()
            } else {
                evaluation.llh
            };
            let delta = difference(cost - offset, -posterior);
            if delta.is_nan() || delta > 1e-6 * posterior.abs().max(1.0) {
                failed.push(format!(
                    "fit: the cost {cost} - {offset} is not the negative log posterior {}",
                    -posterior
                ));
            }
        }
        Err(err) => eprintln!("  {id}: no fit of the engine: {err}"),
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Runs a case of the math: `Ok` when the expression has its value.
fn run_math(expression: &str, expected: &serde_norway::Value) -> Result<(), String> {
    let node = parse(expression).map_err(|err| err.to_string())?;
    let formula = Formula::new(&node)?;
    let values: Vec<f64> = (0..formula.symbols().len())
        .map(|k| 1.3 + 0.7 * k as f64)
        .collect();
    let value = formula.evaluate(&values, 0.0);
    let expected = match number(expected) {
        Some(expected) => expected,
        None => {
            let text = match expected {
                serde_norway::Value::String(text) => text.clone(),
                other => format!("{other:?}"),
            };
            let reference = Formula::new(&parse(&text).map_err(|err| err.to_string())?)?;
            let by_symbol: HashMap<&str, f64> = formula
                .symbols()
                .iter()
                .map(String::as_str)
                .zip(values.iter().copied())
                .collect();
            let read: Vec<f64> = reference
                .symbols()
                .iter()
                .map(|symbol| by_symbol.get(symbol.as_str()).copied().unwrap_or(f64::NAN))
                .collect();
            reference.evaluate(&read, 0.0)
        }
    };
    let close =
        value == expected || (value - expected).abs() <= MATH_TOLERANCE * expected.abs().max(1.0);
    if close {
        Ok(())
    } else {
        Err(format!("'{expression}' is {value}, expected {expected}"))
    }
}

#[test]
fn the_petab_test_suite_passes_its_baseline() {
    let Some(suite) = suite() else {
        eprintln!(
            "the PEtab test suite {COMMIT} is not downloaded; \
             `python scripts/petab_testsuite.py download` fetches it"
        );
        return;
    };
    let runtime = Runtime::embedded().expect("the embedded engine");
    let mut outcomes: BTreeMap<String, Result<(), String>> = BTreeMap::new();
    let mut groups_run = Vec::new();
    for (group, directory) in GROUPS {
        let directory = suite.join(directory);
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        groups_run.push(group);
        let mut cases: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.chars().all(|c| c.is_ascii_digit()))
            })
            .collect();
        cases.sort();
        for case in cases {
            let id = case
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            outcomes.insert(format!("{group}/{id}"), run_case(&case, &runtime));
        }
    }
    let math = suite.join("v2.0.0/math/math_tests.yaml");
    if let Ok(text) = std::fs::read_to_string(&math) {
        groups_run.push("math");
        let content: serde_norway::Value = serde_norway::from_str(&text).expect("the math cases");
        let cases = content
            .get("cases")
            .and_then(|cases| cases.as_sequence())
            .expect("a list of cases");
        for (index, case) in cases.iter().enumerate() {
            let expression = match case.get("expression") {
                Some(serde_norway::Value::String(text)) => text.clone(),
                Some(serde_norway::Value::Number(number)) => number.to_string(),
                Some(other) => format!("{other:?}"),
                None => continue,
            };
            let expected = case.get("expected").cloned().unwrap_or_default();
            outcomes.insert(format!("math/{index:03}"), run_math(&expression, &expected));
        }
    }

    let passed = outcomes.values().filter(|outcome| outcome.is_ok()).count();
    for group in &groups_run {
        let prefix = format!("{group}/");
        let all = outcomes
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .count();
        let ok = outcomes
            .iter()
            .filter(|(key, outcome)| key.starts_with(&prefix) && outcome.is_ok())
            .count();
        eprintln!("{group}: {ok} of {all} cases pass");
    }
    eprintln!(
        "the PEtab test suite: {passed} of {} cases pass",
        outcomes.len()
    );
    let failures: BTreeMap<String, String> = outcomes
        .iter()
        .filter_map(|(key, outcome)| {
            outcome
                .as_ref()
                .err()
                .map(|reason| (key.clone(), reason.clone()))
        })
        .collect();
    for (key, reason) in &failures {
        eprintln!("  {key}: {reason}");
    }

    if std::env::var("SBMLSIM_PETAB_BASELINE").as_deref() == Ok("write") {
        let baseline = Baseline {
            suite_commit: COMMIT.to_string(),
            n_cases: outcomes.len(),
            n_passed: passed,
            expected_failures: failures,
        };
        let text = serde_json::to_string_pretty(&baseline).expect("the baseline as JSON");
        std::fs::write(baseline_path(), text + "\n").expect("the baseline is written");
        return;
    }
    let baseline: Baseline = serde_json::from_str(
        &std::fs::read_to_string(baseline_path()).expect("the baseline of the suite"),
    )
    .expect("the baseline is JSON");
    assert_eq!(
        baseline.suite_commit, COMMIT,
        "the baseline is of another commit"
    );
    let ran = |key: &str| {
        groups_run
            .iter()
            .any(|group| key.starts_with(&format!("{group}/")))
    };
    let regressions: Vec<&String> = failures
        .keys()
        .filter(|key| !baseline.expected_failures.contains_key(*key))
        .collect();
    let fixed: Vec<&String> = baseline
        .expected_failures
        .keys()
        .filter(|key| ran(key) && outcomes.get(*key).is_some_and(Result::is_ok))
        .collect();
    assert!(
        regressions.is_empty() && fixed.is_empty(),
        "cases which fail and are not in the baseline: {regressions:?}; cases of the baseline \
         which pass, remove them from it: {fixed:?}"
    );
}

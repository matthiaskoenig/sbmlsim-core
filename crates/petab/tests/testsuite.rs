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
//!
//! The fit of every case is held to the negative log posterior of its evaluation, but for the
//! cases of [`FIT_REFUSED`], whose posterior has no largest value; the cases of the priors,
//! 0024 and 0025, are fitted as well, with that prior bounded away from zero.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_fit::{JacobianChoice, Problem, Settings};
use sbmlsim_host::Runtime;
use sbmlsim_petab::math::{Formula, parse};
use sbmlsim_petab::{Executor, InMemory, Options, PetabProblem, Plan};
use serde::{Deserialize, Serialize};

/// The commit of the suite, the `PETAB_SUITE_COMMIT` of sbmlsim.
const COMMIT: &str = "d09985e7bb03280b461599a24337137533d931e2";

/// The tolerance of a math case, the `MATH_TOLERANCE` of sbmlsim.
const MATH_TOLERANCE: f64 = 1e-12;

/// The cases whose fit is refused, with the reason: the `log-laplace(3, 5)` prior of
/// `p_log_laplace` at the lower bound 0, whose density `x^(-0.8)` is not bounded there, so the
/// posterior has no largest value and a fit no optimum.
const FIT_REFUSED: [(&str, &str); 2] = [
    (
        "0024",
        "the log-laplace prior of 'p_log_laplace' has a density which is not bounded",
    ),
    (
        "0025",
        "the log-laplace prior of 'p_log_laplace' has a density which is not bounded",
    ),
];

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
    // offsets of its rows; the fit of every case is carried, but for the ones whose
    // posterior has no optimum
    let refused = FIT_REFUSED
        .iter()
        .find(|(case, _)| directory.ends_with(Path::new("v2.0.0/sbml").join(case)))
        .map(|(_, reason)| *reason);
    match executor.problem(1) {
        Ok(_) if refused.is_some() => {
            failed.push(format!(
                "fit: carried, and it is refused with '{}'",
                refused.unwrap_or_default()
            ));
        }
        Err(err) if refused.is_some_and(|reason| err.to_string().contains(reason)) => {}
        Err(err) => failed.push(format!("fit: {err}")),
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

/// The options of the runs of the suite: the tolerances of sbmlsim's run.
fn options() -> Options {
    Options {
        integrator: Some(IntegratorSettings {
            relative_tolerance: 1e-12,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        }),
        ..Options::default()
    }
}

/// The files of a case, the lower bound of `p_log_laplace` raised from 0 to 1, where the
/// density of its prior is bounded.
fn bounded_case(directory: &Path) -> InMemory {
    let mut files = InMemory::new();
    for entry in std::fs::read_dir(directory).expect("the case") {
        let path = entry.expect("a file of the case").path();
        if path.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            files.insert(name.to_string(), std::fs::read(&path).expect("a file"));
        }
    }
    let table = String::from_utf8(files["_parameters.tsv"].clone()).expect("a table");
    assert!(table.contains("p_log_laplace\t0.0\t"), "{table}");
    files.insert(
        "_parameters.tsv".to_string(),
        table
            .replace("p_log_laplace\t0.0\t", "p_log_laplace\t1.0\t")
            .into_bytes(),
    );
    files
}

/// The gradient of the cost of a problem, `J' r`, and its central difference.
fn gradients(problem: &Problem, x: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let k = x.len();
    let residuals = problem.residuals(x).expect("the residuals");
    let jacobian = problem.jacobian(x).expect("the jacobian");
    let gradient = (0..k)
        .map(|column| {
            residuals
                .iter()
                .enumerate()
                .map(|(row, r)| r * jacobian[row * k + column])
                .sum()
        })
        .collect();
    let central = (0..k)
        .map(|column| {
            let step = 1e-6 * x[column].abs().max(1.0);
            let (mut below, mut above) = (x.to_vec(), x.to_vec());
            below[column] -= step;
            above[column] += step;
            (problem.cost(&above).expect("a cost") - problem.cost(&below).expect("a cost"))
                / (2.0 * step)
        })
        .collect();
    (gradient, central)
}

#[test]
fn the_cases_of_every_prior_are_fitted() {
    // every kind of prior of PEtab v2, truncated at finite bounds (0024) and at bounds which
    // are not finite (0025); no data informs a parameter of a prior, so the optimum of the
    // posterior is the mode of every prior in its bounds, and `p1`, measured once at 1, is 1
    let Some(suite) = suite() else {
        eprintln!("the PEtab test suite {COMMIT} is not downloaded");
        return;
    };
    let runtime = Runtime::embedded().expect("the embedded engine");
    let modes: HashMap<&str, f64> = [
        ("p_normal", 4.0),
        ("p_log_normal", 1f64.exp()),
        ("p_cauchy", 3.0),
        ("p_chisquare", 2.0),
        ("p_exponential", 0.0),
        ("p_gamma", 10.0),
        ("p_laplace", 3.0),
        ("p_log_laplace", 1.0),
        ("p_log_uniform", 3.0),
        ("p_rayleigh", 3.0),
        ("p1", 1.0),
    ]
    .into_iter()
    .collect();
    for case in ["0024", "0025"] {
        let directory = suite.join("v2.0.0/sbml").join(case);
        let yaml = format!("_{case}.yaml");
        let problem = PetabProblem::read(&yaml, &bounded_case(&directory)).expect("the case");
        let executor =
            Executor::with_runtime(Plan::new(&problem, &options()).expect("a plan"), &runtime)
                .expect("the models");
        let plan = executor.plan();
        let ids: Vec<&str> = plan
            .estimated()
            .into_iter()
            .map(|k| plan.parameters[k].id.as_str())
            .collect();

        // the cost is the negative log posterior and its gradient the one of the jacobian of
        // the sensitivities, whose rows of the priors are analytic, at points of the box
        let mut definition = executor.definition(&plan.nominal()).expect("a definition");
        definition.jacobian = JacobianChoice::Sensitivities;
        let problem = executor.problem_of(definition, 1).expect("a problem");
        let offset = executor.cost_offset().expect("the offset");
        let (lower, upper) = problem.bounds();
        let start = problem.start();
        for shift in [0.0, -0.3, 0.15, 0.4] {
            let x: Vec<f64> = start
                .iter()
                .zip(lower.iter().zip(&upper))
                .map(|(x, (low, high))| {
                    let moved = x + shift * x.abs().max(0.5);
                    if low.is_finite() && high.is_finite() {
                        let margin = 0.02 * (high - low);
                        moved.clamp(low + margin, high - margin)
                    } else if low.is_finite() {
                        moved.max(low + 0.1)
                    } else {
                        moved
                    }
                })
                .collect();
            let evaluation = executor.evaluate(&problem.values(&x));
            let posterior = evaluation.unnorm_log_posterior();
            let cost = problem.cost(&x).expect("a cost") - offset;
            assert!(
                (cost + posterior).abs() <= 1e-9 * posterior.abs().max(1.0),
                "{case} at {x:?}: the cost {cost} is not the negative log posterior {}",
                -posterior
            );
            let (gradient, central) = gradients(&problem, &x);
            for (k, (g, c)) in gradient.iter().zip(&central).enumerate() {
                assert!(
                    (g - c).abs() <= 1e-5 * c.abs().max(1.0),
                    "{case} at {x:?}: the gradient of '{}' is {g} and its central difference {c}",
                    ids[k]
                );
            }
        }

        // the fit of every prior alone, the others held at their start, finds its mode, on
        // the analytic rows of the priors, which the kink of a Laplace prior at its mode needs
        let full = &problem;
        let settings = Settings {
            max_iterations: 500,
            ..Settings::default()
        };
        for (k, id) in ids.iter().enumerate() {
            let held: Vec<(usize, f64)> = (0..ids.len())
                .filter(|other| *other != k)
                .map(|other| (other, full.parameters()[other].start))
                .collect();
            let one = full.fixing(&held).expect("a problem of one parameter");
            let fit = one.optimize(&settings).expect("a fit");
            let value = fit.values[0];
            match modes.get(id) {
                // the cost of the optimum is the one of the mode, and so is its value, to the
                // flatness of a mode at a bound where the slope of the density is zero, e.g.
                // the one of `gamma(3, 5)` at 10
                Some(mode) => {
                    let at_mode = one
                        .cost(&[one.parameters()[0].scale.to_search(*mode)])
                        .expect("the cost of the mode");
                    assert!(
                        (value - mode).abs() <= 1e-3 * mode.max(1.0) && fit.cost <= at_mode + 1e-6,
                        "{case}: '{id}' is fitted to {value} of the cost {} and its mode is \
                         {mode} of the cost {at_mode} ({})",
                        fit.cost,
                        fit.status
                    );
                }
                // the uniform prior between 2 and 8 has no mode, and the fit stays at its
                // start
                None => assert!((2.0..=8.0).contains(&value), "{case}: '{id}' is {value}"),
            }
        }

        // and the fit of all of them, whose objective is the negative log posterior at its
        // optimum; the kink of the Laplace prior at its mode slows the joint fit, which ends
        // close to the modes
        let answer = executor.fit(&settings, 1).expect("a fit");
        let posterior = executor.evaluate(&answer.x).unnorm_log_posterior();
        assert!(
            (answer.objective + posterior).abs() <= 1e-9 * posterior.abs().max(1.0),
            "{case}: the objective {} is not the negative log posterior {}",
            answer.objective,
            -posterior
        );
        for (id, value) in ids.iter().zip(&answer.x) {
            if let Some(mode) = modes.get(id) {
                assert!(
                    (value - mode).abs() <= 1e-2 * mode.max(1.0),
                    "{case}: '{id}' is fitted to {value} and its mode is {mode} ({})",
                    answer.fit.status
                );
            }
        }
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

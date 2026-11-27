//! `sbmlsim-core testsuite <cases-dir>`: runs the prepared cases of the SBML Test Suite.
//!
//! A case is a directory made by `scripts/prepare_testsuite.py`: `model.xml` (SBML L3V2, `comp`
//! flattened), `case.json` (the simulation, the selections, the tolerances and the tags, or
//! `not_converted` with a message) and `expected.csv`. The runner compiles every model, runs it
//! with the integrator at the absolute and relative tolerance `1e-12` and compares the result
//! with the expected values by the rule of the suite: a value `u` agrees with the expected `c` if
//! `|c - u| <= abs_tol + rel_tol * |c|`; a `NaN` agrees with a `NaN` and an infinity with the
//! same infinity. The columns are matched by position, `time` is the first one and is not
//! compared (as in sbmlsim). Every failure is a [`Status`], none is raised.
//!
//! `--jacobian` is the Jacobian the models are compiled with (`sbmlsim_compiler::CompileOptions`):
//! `auto` (the default) the analytic one where the model has one, else the colored difference
//! quotient; `analytic` the analytic one, a model without one is not compiled; `difference` the
//! colored difference quotient for every model. The summary counts the Jacobians of the compiled
//! models. `--no-optimize` compiles them without the optimizer, which gives the same bits.
//!
//! The statuses are compared with the baseline (`testdata/testsuite_baseline.json`, the format of
//! `sbmlsim.testsuite.baseline`) in both directions: a case which fails and is not listed is a
//! regression, a case which is listed and passes is a baseline which is out of date; a listed
//! case with another status and, in a run of all cases, a listed case which was not run are
//! differences as well. The command exits with 0 only without differences.
//!
//! `--model-code c` simulates every case on the native engine with the model code of the C target
//! (`sbmlsim_c`, a build with the feature `native`) instead of `engine.wasm` with `model.wasm`:
//! the same comparison, repeat checks and baseline, the steady state of a fresh instance being
//! a fresh engine. A model the C target refuses (a delay, no analytic Jacobian) is
//! [`Status::Refused`] with the feature. The cases it refuses are pinned with their features
//! in `testdata/testsuite_refused_c.json` ([`Refusals`], `--refused`): a pinned refusal is no
//! difference, a case refused which is not pinned (or for another feature) and a pinned case
//! which is no longer refused are, so the set changes only deliberately (`--write-baseline
//! --model-code c` writes it from a run of every case).
//!
//! `--comp` runs the cases of the package `comp` from their composed models instead
//! (`comp/model.xml`, with the documents of its external model definitions beside it, which
//! `scripts/prepare_testsuite.py` writes next to the model libSBML flattened): the model is
//! flattened by the reader of the core (`sbmlsim_sbml::flatten_comp`), and the cases are
//! compared with the same baseline, as a run of some cases.
//!
//! `--export DIR` writes every case which passes as the inputs and the outputs of the wasmtime
//! host, which `js/test/same_bits.test.mjs` runs in Node and compares byte for byte:
//! `DIR/engine.wasm` and `DIR/mathlib.wasm` (the ones of the run), and per case
//! `DIR/cases/<case>/`: `model.wasm`, `model.json`, `request.json` and `wasmtime.csv` (the CSV
//! of `sbmlsim_cli::csv` of the result of the case), `wasmtime.f64` (its values as `f64`, little
//! endian) and, when the simulation with a steady state before it succeeds, `steady.json` and
//! `steady.csv`.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use rayon::prelude::*;
use sbmlsim_abi::ModelDescription;
use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, JacobianKind, Output, SIMULATION_CONTRACT,
    SimulationRequest, SimulationResult, SteadyState, Tolerance,
};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_host::{Instance, Runtime};

use crate::csv::to_csv;
use serde::{Deserialize, Serialize};

/// The absolute and the relative tolerance of the integrator in a run of the suite.
pub const INTEGRATOR_TOLERANCE: f64 = 1e-12;

/// The seed of the random generator which orders simultaneous events.
pub const EVENT_SEED: u64 = 1;

/// The reason of a case in the baseline which nobody wrote down.
pub const MISSING_REASON: &str = "reason missing";

/// The longest message of a result in characters.
const MAX_MESSAGE: usize = 300;

/// How many differences the console lists of each kind.
const LISTED_DIFFERENCES: usize = 40;

/// The outcome of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The results are within the tolerances (and the repeat checks agree).
    Pass,
    /// The results are outside the tolerances.
    Tolerance,
    /// The model was not converted or not compiled.
    NotCompiled,
    /// The instantiation or the simulation failed.
    SimulationError,
    /// A column or a row of the expected results is missing from the simulation.
    MissingVariable,
    /// A repeated simulation does not equal the first (see [`repeat_check`]).
    Repeat,
    /// The C target refuses the model (`--model-code c`), the message names the feature.
    Refused,
}

impl Status {
    /// Every status, in the order of the report.
    pub const ALL: [Status; 7] = [
        Status::Pass,
        Status::Tolerance,
        Status::NotCompiled,
        Status::SimulationError,
        Status::MissingVariable,
        Status::Repeat,
        Status::Refused,
    ];

    /// The name of the status in the baseline and in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Tolerance => "tolerance",
            Status::NotCompiled => "not_compiled",
            Status::SimulationError => "simulation_error",
            Status::MissingVariable => "missing_variable",
            Status::Repeat => "repeat",
            Status::Refused => "refused",
        }
    }
}

/// The result of a case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    /// The identifier of the case, e.g. `00001`.
    pub cid: String,
    /// The outcome.
    pub status: Status,
    /// What failed, empty for a case which passes.
    pub message: String,
    /// The largest absolute difference to the expected values, where something was compared.
    pub max_difference: Option<f64>,
    /// The Jacobian of the compiled model, `analytic` or `colored_difference`, absent for a
    /// model which was not compiled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jacobian: Option<String>,
}

impl CaseResult {
    fn new(cid: &str, status: Status, message: impl Into<String>) -> Self {
        let mut message: String = message.into();
        if let Some((end, _)) = message.char_indices().nth(MAX_MESSAGE) {
            message.truncate(end);
        }
        CaseResult {
            cid: cid.to_string(),
            status,
            message,
            max_difference: None,
            jacobian: None,
        }
    }
}

/// The `case.json` of a case.
#[derive(Debug, Clone, Deserialize)]
pub struct CaseSettings {
    /// The identifier of the case.
    pub cid: String,
    /// The start time.
    pub start: f64,
    /// The duration.
    pub duration: f64,
    /// The number of steps, so the results have `steps + 1` rows.
    pub steps: u32,
    /// The selections, `time` first.
    pub selections: Vec<String>,
    /// The absolute tolerance of the comparison.
    pub absolute_tolerance: f64,
    /// The relative tolerance of the comparison.
    pub relative_tolerance: f64,
    /// The test tags of the case.
    #[serde(default)]
    pub test_tags: Vec<String>,
    /// Set when the model was not converted by the preparation: the message.
    #[serde(default)]
    pub not_converted: Option<String>,
    /// The commit of the suite the case was prepared from.
    #[serde(default)]
    pub suite_commit: String,
}

/// A table of numbers, row major, `time` in the first column.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The number of columns.
    pub n_cols: usize,
    /// `n_rows * n_cols` values.
    pub values: Vec<f64>,
}

impl Table {
    /// Reads `expected.csv`: a header line and the rows; `NaN`, `Inf` and `-Inf` are values.
    pub fn from_csv(text: &str) -> anyhow::Result<Table> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header = lines.next().context("the results are empty")?;
        let n_cols = header.split(',').count();
        let mut values = Vec::new();
        for (i, line) in lines.enumerate() {
            let before = values.len();
            for field in line.split(',') {
                let field = field.trim();
                values.push(
                    field
                        .parse::<f64>()
                        .with_context(|| format!("row {}: '{field}' is no number", i + 1))?,
                );
            }
            anyhow::ensure!(
                values.len() - before == n_cols,
                "row {} has {} columns, the header {n_cols}",
                i + 1,
                values.len() - before
            );
        }
        Ok(Table { n_cols, values })
    }

    fn n_rows(&self) -> usize {
        self.values.len() / self.n_cols.max(1)
    }
}

/// Whether the observed value agrees with the expected one under the rule of the suite.
fn within(expected: f64, observed: f64, abs_tol: f64, rel_tol: f64) -> bool {
    if expected.is_nan() {
        return observed.is_nan();
    }
    if expected.is_infinite() {
        return expected == observed;
    }
    (expected - observed).abs() <= abs_tol + rel_tol * expected.abs()
}

/// Compares a result with the expected values, the columns by position.
///
/// `time` (the first column) is not compared. Returns the largest absolute difference of the
/// finite values, or the failure with the status `tolerance` (the worst point is named) or
/// `missing_variable` (a column or a row is missing).
pub fn compare(
    expected: &Table,
    observed: &SimulationResult,
    abs_tol: f64,
    rel_tol: f64,
) -> Result<f64, CaseResult> {
    let fail = |status, message: String, max| CaseResult {
        cid: String::new(),
        status,
        message,
        max_difference: max,
        jacobian: None,
    };
    let k = observed.columns.len();
    if k < expected.n_cols {
        let missing = expected.n_cols - k;
        return Err(fail(
            Status::MissingVariable,
            format!("{missing} of {} columns are not simulated", expected.n_cols),
            None,
        ));
    }
    let n_rows = expected.n_rows();
    if observed.n_rows != n_rows || observed.values.len() != observed.n_rows * k {
        return Err(fail(
            Status::MissingVariable,
            format!(
                "{} rows are simulated, {n_rows} expected (point count)",
                observed.n_rows
            ),
            None,
        ));
    }
    let (mut n_points, mut n_bad) = (0usize, 0usize);
    let mut max_difference: f64 = 0.0;
    let mut worst: Option<(f64, usize, usize)> = None;
    for row in 0..n_rows {
        for col in 1..expected.n_cols {
            let c = expected.values[row * expected.n_cols + col];
            let u = observed.values[row * k + col];
            n_points += 1;
            if within(c, u, abs_tol, rel_tol) {
                if c.is_finite() && u.is_finite() {
                    max_difference = max_difference.max((c - u).abs());
                }
                continue;
            }
            n_bad += 1;
            let excess = if c.is_finite() && u.is_finite() {
                (c - u).abs() - (abs_tol + rel_tol * c.abs())
            } else {
                f64::INFINITY
            };
            if c.is_finite() && u.is_finite() {
                max_difference = max_difference.max((c - u).abs());
            }
            if worst.is_none_or(|(w, _, _)| excess > w) {
                worst = Some((excess, row, col));
            }
        }
    }
    match worst {
        None => Ok(max_difference),
        Some((_, row, col)) => {
            let c = expected.values[row * expected.n_cols + col];
            let u = observed.values[row * k + col];
            let time = observed.values[row * k];
            Err(fail(
                Status::Tolerance,
                format!(
                    "{n_bad} of {n_points} points outside the tolerances, worst '{}' at t={time:.6e}: expected {c:e}, simulated {u:e}",
                    observed.columns[col]
                ),
                Some(max_difference),
            ))
        }
    }
}

/// The message when the expected results do not have `time` and one column per selection.
pub fn check_columns(expected: &Table, settings: &CaseSettings) -> Option<String> {
    (expected.n_cols != settings.selections.len()).then(|| {
        format!(
            "the expected results have {} columns, the case {} selections",
            expected.n_cols,
            settings.selections.len()
        )
    })
}

/// Checks that every case was prepared from the suite commit of the baseline.
pub fn check_commits(dirs: &[PathBuf], baseline_commit: &str) -> anyhow::Result<()> {
    for dir in dirs {
        let bytes = fs::read(dir.join("case.json"))
            .with_context(|| format!("cannot read {}", dir.join("case.json").display()))?;
        let settings: CaseSettings = serde_json::from_slice(&bytes)
            .with_context(|| format!("cannot read {}", dir.join("case.json").display()))?;
        anyhow::ensure!(
            settings.suite_commit == baseline_commit,
            "the case {} was prepared from the suite commit '{}', the baseline records '{}'; \
             prepare the cases again (scripts/prepare_testsuite.py) or update the baseline deliberately",
            dir.display(),
            settings.suite_commit,
            baseline_commit
        );
    }
    Ok(())
}

/// The test tag of the cases whose simultaneous events execute in a random order.
pub const RANDOM_EVENT_EXECUTION: &str = "RandomEventExecution";

/// The request of a case for `fraction` of its duration with as many of its steps (rounded half
/// to even, as Python's `round` in sbmlsim's `case_simulation`, at least one), with a steady
/// state before it of at most the duration of the case for `steady`.
pub fn case_request_of(settings: &CaseSettings, fraction: f64, steady: bool) -> SimulationRequest {
    let mut req = case_request(settings);
    if fraction != 1.0 {
        req.end = settings.start + settings.duration * fraction;
        req.output = Output::Uniform {
            steps: ((settings.steps as f64 * fraction).round_ties_even() as u32).max(1),
        };
    }
    if steady {
        req.steady_state = Some(SteadyState {
            max_time: settings.duration,
            ..SteadyState::default()
        });
    }
    req
}

/// How a later simulation differs from the first, bitwise (`NaN` equal to `NaN`), `None` for
/// equal results.
pub fn repeat_difference(
    first: &SimulationResult,
    second: &SimulationResult,
    what: &str,
) -> Option<String> {
    if first.columns != second.columns
        || first.n_rows != second.n_rows
        || first.values.len() != second.values.len()
    {
        return Some(format!(
            "{what} has {} rows of {} columns and not {} rows of {} columns of the first",
            second.n_rows,
            second.columns.len(),
            first.n_rows,
            first.columns.len()
        ));
    }
    let differs = |a: f64, b: f64| a.to_bits() != b.to_bits() && !(a.is_nan() && b.is_nan());
    let k = first.columns.len().max(1);
    let mut n = 0usize;
    let mut worst: Option<(f64, usize)> = None;
    for (i, (a, b)) in first.values.iter().zip(&second.values).enumerate() {
        if differs(*a, *b) {
            n += 1;
            let d = (a - b).abs();
            let d = if d.is_nan() { f64::INFINITY } else { d };
            if worst.is_none_or(|(w, _)| d > w) {
                worst = Some((d, i));
            }
        }
    }
    let (d, i) = worst?;
    let (row, col) = (i / k, i % k);
    Some(format!(
        "{what} differs from the first in {n} values, at most {d:.3e} in '{}' at the time {} \
         ({:?} and {:?})",
        first.columns[col],
        first.values[row * k],
        first.values[i],
        second.values[i]
    ))
}

/// Whether two results of a case of random event execution agree within the tolerances of
/// the case (the rule of the suite, `a` taken as the expected one).
fn within_case(a: &SimulationResult, b: &SimulationResult, settings: &CaseSettings) -> bool {
    a.columns == b.columns
        && a.values.len() == b.values.len()
        && a.values.iter().zip(&b.values).all(|(x, y)| {
            within(
                *x,
                *y,
                settings.absolute_tolerance,
                settings.relative_tolerance,
            )
        })
}

/// The repeat checks of a case which agrees with its expected results, as sbmlsim's
/// `testsuite/runner.py` `repeat_check`, on the instance which simulated it first:
///
/// 1. the same simulation again;
/// 2. a simulation of half the duration, then the simulation of the case;
/// 3. the simulation with a steady state before it (at most the duration of the case), on
///    this instance and on a fresh one; both may fail, then with the same error.
///
/// Each must equal the first (3: the one of the fresh instance), bitwise. Every request has
/// the seed [`EVENT_SEED`], so a case of random event execution ([`RANDOM_EVENT_EXECUTION`])
/// draws the same order; as in sbmlsim it is compared with its expected results in 2 and
/// within the tolerances of the case in 3 instead. Returns the message of a difference.
pub fn repeat_check(
    runtime: &Runtime,
    compiled: &CompiledModel,
    instance: &mut Instance,
    settings: &CaseSettings,
    expected: &Table,
    first: &SimulationResult,
) -> Option<String> {
    let fresh = || -> Result<Box<dyn Simulator>, String> {
        let instance = runtime
            .instantiate(&compiled.wasm, &compiled.description)
            .map_err(|e| format!("{e:#}"))?;
        Ok(Box::new(instance))
    };
    repeat_check_with(instance, settings, expected, first, &fresh)
}

/// What simulates the requests of a case: an [`Instance`] of the wasmtime host, or the native
/// engine with the model of the C target (`--model-code c`).
pub trait Simulator {
    /// The result of a request, or the message of its failure.
    fn simulate_case(&mut self, req: &SimulationRequest) -> Result<SimulationResult, String>;
}

impl Simulator for Instance {
    fn simulate_case(&mut self, req: &SimulationRequest) -> Result<SimulationResult, String> {
        self.simulate(req).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "native")]
impl Simulator for sbmlsim_engine::Engine {
    fn simulate_case(&mut self, req: &SimulationRequest) -> Result<SimulationResult, String> {
        self.simulate(req).map_err(|e| e.to_string())
    }
}

/// [`repeat_check`] of any [`Simulator`]; `fresh` makes a fresh one of the same model.
pub fn repeat_check_with(
    instance: &mut dyn Simulator,
    settings: &CaseSettings,
    expected: &Table,
    first: &SimulationResult,
    fresh: &dyn Fn() -> Result<Box<dyn Simulator>, String>,
) -> Option<String> {
    let random = settings
        .test_tags
        .iter()
        .any(|t| t == RANDOM_EVENT_EXECUTION);
    let request = case_request(settings);
    let simulate = |instance: &mut dyn Simulator, req: &SimulationRequest, what: &str| {
        instance
            .simulate_case(req)
            .map_err(|e| format!("{what} failed: {e}"))
    };

    let what = "the second simulation of the instance";
    let second = match simulate(instance, &request, what) {
        Ok(second) => second,
        Err(message) => return Some(message),
    };
    if let Some(difference) = repeat_difference(first, &second, what) {
        return Some(difference);
    }

    let half = case_request_of(settings, 0.5, false);
    if let Err(message) = simulate(instance, &half, "a simulation of half the duration") {
        return Some(message);
    }
    let what = "a simulation of the instance after a shorter one";
    let third = match simulate(instance, &request, what) {
        Ok(third) => third,
        Err(message) => return Some(message),
    };
    if let Some(difference) = repeat_difference(first, &third, what) {
        let agrees = compare(
            expected,
            &third,
            settings.absolute_tolerance,
            settings.relative_tolerance,
        )
        .is_ok();
        if !(random && agrees) {
            return Some(difference);
        }
    }

    let steady = case_request_of(settings, 1.0, true);
    let what = "a steady state after the simulations of the instance";
    let reference = match fresh() {
        Ok(mut fresh) => fresh.simulate_case(&steady),
        Err(error) => return Some(format!("a fresh instance failed: {error}")),
    };
    let observed = instance.simulate_case(&steady);
    match (reference, observed) {
        (Ok(reference), Ok(observed)) => {
            let difference = repeat_difference(&reference, &observed, what)?;
            if random && within_case(&reference, &observed, settings) {
                return None;
            }
            Some(difference)
        }
        (Err(reference), Err(observed)) if reference == observed => None,
        (reference, observed) => {
            let describe = |r: &Result<SimulationResult, String>| match r {
                Ok(_) => "results".to_string(),
                Err(message) => message.clone(),
            };
            Some(format!(
                "{what} gives {}, a steady state of a fresh instance {}",
                describe(&observed),
                describe(&reference)
            ))
        }
    }
}

/// The request of a case.
pub fn case_request(settings: &CaseSettings) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_string(),
        start: settings.start,
        end: settings.start + settings.duration,
        output: Output::Uniform {
            steps: settings.steps,
        },
        preinit: Vec::new(),
        changes: Vec::new(),
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: settings.selections.clone(),
        settings: IntegratorSettings {
            relative_tolerance: INTEGRATOR_TOLERANCE,
            absolute_tolerance: Tolerance::Scalar(INTEGRATOR_TOLERANCE),
            ..IntegratorSettings::default()
        },
        seed: EVENT_SEED,
    }
}

/// Runs the case in a directory; every failure is the status of the result.
/// The model of a case which is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseModel {
    /// `model.xml`, which libSBML flattened.
    #[default]
    Flattened,
    /// `comp/model.xml`, the composed model of a case of the package `comp`, which the reader
    /// of the core flattens with the documents of its external model definitions in `comp/`.
    Comp,
}

pub fn run_case(runtime: &Runtime, dir: &Path) -> CaseResult {
    run_case_with(runtime, dir, &CompileOptions::default())
}

/// Runs the case in a directory with a model compiled with `options`.
pub fn run_case_with(runtime: &Runtime, dir: &Path, options: &CompileOptions) -> CaseResult {
    run_case_exported(runtime, dir, options, None, CaseModel::Flattened)
}

/// [`run_case_with`], which writes a case which passes into `export` (`--export`).
pub fn run_case_exported(
    runtime: &Runtime,
    dir: &Path,
    options: &CompileOptions,
    export: Option<&Path>,
    model: CaseModel,
) -> CaseResult {
    guarded(dir, |cid| {
        run_case_inner(runtime, dir, cid, options, export, model)
    })
}

/// Runs the case in a directory on the native engine with the model of the C target
/// (`--model-code c`); a model the C target refuses is [`Status::Refused`].
#[cfg(feature = "native")]
pub fn run_case_c(dir: &Path) -> CaseResult {
    run_case_c_of(dir, CaseModel::Flattened)
}

/// [`run_case_c`] of a model of the case.
#[cfg(feature = "native")]
pub fn run_case_c_of(dir: &Path, model: CaseModel) -> CaseResult {
    guarded(dir, |cid| run_c(dir, cid, model))
}

/// `run` of the case in `dir` with its identifier; a panic is a simulation error.
fn guarded(dir: &Path, run: impl FnOnce(&str) -> CaseResult) -> CaseResult {
    let cid = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match catch_unwind(AssertUnwindSafe(|| run(&cid))) {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            CaseResult::new(&cid, Status::SimulationError, format!("panic: {message}"))
        }
    }
}

fn run_case_inner(
    runtime: &Runtime,
    dir: &Path,
    cid: &str,
    options: &CompileOptions,
    export: Option<&Path>,
    model: CaseModel,
) -> CaseResult {
    let compiled = OnceCell::new();
    let result = run_compiled(runtime, dir, cid, options, &compiled, export, model);
    CaseResult {
        jacobian: compiled.get().map(|c: &CompiledModel| {
            match c.description.jacobian {
                JacobianKind::Analytic(_) => "analytic",
                JacobianKind::ColoredDifference { .. } => "colored_difference",
            }
            .to_string()
        }),
        ..result
    }
}

fn run_compiled(
    runtime: &Runtime,
    dir: &Path,
    cid: &str,
    options: &CompileOptions,
    cell: &OnceCell<CompiledModel>,
    export: Option<&Path>,
    model: CaseModel,
) -> CaseResult {
    let (settings, sbml) = match read_case(dir, cid, model) {
        Ok(case) => case,
        Err(result) => return result,
    };
    let compiled = match compile(&sbml, options) {
        Ok(compiled) => cell.get_or_init(|| compiled),
        Err(error) => return CaseResult::new(cid, Status::NotCompiled, error.to_string()),
    };
    let mut instance = match runtime.instantiate(&compiled.wasm, &compiled.description) {
        Ok(instance) => instance,
        Err(error) => {
            return CaseResult::new(cid, Status::SimulationError, format!("{error:#}"));
        }
    };
    let fresh = || -> Result<Box<dyn Simulator>, String> {
        let instance = runtime
            .instantiate(&compiled.wasm, &compiled.description)
            .map_err(|e| format!("{e:#}"))?;
        Ok(Box::new(instance))
    };
    simulate_case(
        dir,
        cid,
        &settings,
        &mut instance,
        &fresh,
        |observed| match export {
            Some(out) => export_case(runtime, compiled, &settings, observed, out, cid),
            None => Ok(()),
        },
    )
}

/// The case of the C target: its model compiled to C, loaded, and simulated on the native
/// engine.
#[cfg(feature = "native")]
fn run_c(dir: &Path, cid: &str, model: CaseModel) -> CaseResult {
    use sbmlsim_c::CError;
    let (settings, sbml) = match read_case(dir, cid, model) {
        Ok(case) => case,
        Err(result) => return result,
    };
    let source = match sbmlsim_c::generate(&sbml) {
        Ok(source) => source,
        Err(error @ CError::Unsupported { .. }) => {
            return CaseResult::new(cid, Status::Refused, error.to_string());
        }
        Err(error) => return CaseResult::new(cid, Status::NotCompiled, error.to_string()),
    };
    // every model the C target compiles has the analytic Jacobian
    let result = simulate_c(dir, cid, &settings, &source);
    CaseResult {
        jacobian: Some("analytic".to_string()),
        ..result
    }
}

/// [`run_c`] of the C of a case.
#[cfg(feature = "native")]
fn simulate_c(
    dir: &Path,
    cid: &str,
    settings: &CaseSettings,
    source: &sbmlsim_c::CSource,
) -> CaseResult {
    use sbmlsim_c::{CModel, Library};
    let model = match Library::build(&source.code)
        .and_then(|library| CModel::new(library, &source.description))
    {
        Ok(model) => model,
        Err(error) => return CaseResult::new(cid, Status::SimulationError, error.to_string()),
    };
    let engine = || {
        sbmlsim_engine::Engine::new(Box::new(model.clone()), source.description.clone())
            .map_err(|e| e.to_string())
    };
    let mut first = match engine() {
        Ok(engine) => engine,
        Err(message) => return CaseResult::new(cid, Status::SimulationError, message),
    };
    let fresh = || -> Result<Box<dyn Simulator>, String> { Ok(Box::new(engine()?)) };
    simulate_case(dir, cid, settings, &mut first, &fresh, |_| Ok(()))
}

/// The settings and the SBML of the case in `dir`, or the result of a case which cannot be
/// read or was not converted.
fn read_case(
    dir: &Path,
    cid: &str,
    model: CaseModel,
) -> Result<(CaseSettings, String), CaseResult> {
    let settings: CaseSettings = fs::read(dir.join("case.json"))
        .map_err(anyhow::Error::from)
        .and_then(|b| serde_json::from_slice(&b).map_err(anyhow::Error::from))
        .map_err(|error| {
            CaseResult::new(cid, Status::SimulationError, format!("case.json: {error}"))
        })?;
    if let Some(message) = &settings.not_converted {
        return Err(CaseResult::new(
            cid,
            Status::NotCompiled,
            format!("not converted: {message}"),
        ));
    }
    let sbml = match model {
        CaseModel::Flattened => fs::read_to_string(dir.join("model.xml")).map_err(|error| {
            CaseResult::new(cid, Status::SimulationError, format!("model.xml: {error}"))
        })?,
        CaseModel::Comp => {
            let comp = dir.join(COMP_DIR);
            let text = fs::read_to_string(comp.join("model.xml")).map_err(|error| {
                CaseResult::new(
                    cid,
                    Status::SimulationError,
                    format!("{COMP_DIR}/model.xml: {error}"),
                )
            })?;
            // the external documents are the files beside the model
            let sources = |source: &str| {
                fs::read_to_string(comp.join(source)).map_err(|error| error.to_string())
            };
            sbmlsim_sbml::flatten_comp(&text, &sources)
                .map_err(|error| CaseResult::new(cid, Status::NotCompiled, error.to_string()))?
        }
    };
    Ok((settings, sbml))
}

/// Simulates a case on `instance` and compares the result with the expected results, then
/// the repeat checks ([`repeat_check_with`], `fresh` a fresh simulator of the model); `passed`
/// runs for a case which passes (the export), its failure a simulation error.
fn simulate_case(
    dir: &Path,
    cid: &str,
    settings: &CaseSettings,
    instance: &mut dyn Simulator,
    fresh: &dyn Fn() -> Result<Box<dyn Simulator>, String>,
    passed: impl FnOnce(&SimulationResult) -> anyhow::Result<()>,
) -> CaseResult {
    let observed = match instance.simulate_case(&case_request(settings)) {
        Ok(observed) => observed,
        Err(message) => return CaseResult::new(cid, Status::SimulationError, message),
    };
    let expected = match fs::read_to_string(dir.join("expected.csv"))
        .map_err(anyhow::Error::from)
        .and_then(|text| Table::from_csv(&text))
    {
        Ok(expected) => expected,
        Err(error) => {
            return CaseResult::new(
                cid,
                Status::SimulationError,
                format!("expected.csv: {error:#}"),
            );
        }
    };
    if let Some(message) = check_columns(&expected, settings) {
        return CaseResult::new(cid, Status::MissingVariable, message);
    }
    match compare(
        &expected,
        &observed,
        settings.absolute_tolerance,
        settings.relative_tolerance,
    ) {
        Err(mut failure) => {
            failure.cid = cid.to_string();
            CaseResult::new(cid, failure.status, failure.message)
                .with_difference(failure.max_difference)
        }
        Ok(max_difference) => {
            match repeat_check_with(instance, settings, &expected, &observed, fresh) {
                Some(message) => CaseResult::new(cid, Status::Repeat, message),
                None => match passed(&observed) {
                    Err(error) => CaseResult::new(
                        cid,
                        Status::SimulationError,
                        format!("the export failed: {error:#}"),
                    ),
                    Ok(()) => {
                        CaseResult::new(cid, Status::Pass, "").with_difference(Some(max_difference))
                    }
                },
            }
        }
    }
}

/// The JSON of a description as `sbmlsim-core compile` writes `model.json`.
pub fn description_json(desc: &ModelDescription) -> anyhow::Result<Vec<u8>> {
    let mut json = serde_json::to_vec_pretty(desc)?;
    json.push(b'\n');
    Ok(json)
}

/// Writes the `engine.wasm`, `mathlib.wasm`, `compiler.wasm` and `plot.wasm` of this build into
/// `dir`, and next to them `THIRD_PARTY.md`, the notices of the libraries `engine.wasm` links
/// statically (a redistributed `engine.wasm` carries them).
///
/// `compiler.wasm` is the compiler of a JavaScript host (`js/compile.mjs`), which compiles an
/// SBML document in a page into the `model.wasm` the engine runs, and `plot.wasm` the renderer
/// of the figures (`figure/1`) in a page, the SVG of the native renderer byte for byte.
pub fn write_runtime(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    for (name, bytes) in [
        ("engine.wasm", sbmlsim_host::ENGINE_WASM),
        ("mathlib.wasm", sbmlsim_host::MATHLIB_WASM),
        ("compiler.wasm", sbmlsim_host::COMPILER_WASM),
        ("plot.wasm", sbmlsim_host::PLOT_WASM),
        (
            "THIRD_PARTY.md",
            sbmlsim_host::THIRD_PARTY_NOTICES.as_bytes(),
        ),
    ] {
        let path = dir.join(name);
        fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}

/// Writes a case which passes into `out/cases/<cid>` (see the module).
fn export_case(
    runtime: &Runtime,
    compiled: &CompiledModel,
    settings: &CaseSettings,
    observed: &SimulationResult,
    out: &Path,
    cid: &str,
) -> anyhow::Result<()> {
    let dir = out.join("cases").join(cid);
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))
    };
    write("model.wasm", &compiled.wasm)?;
    write("model.json", &description_json(&compiled.description)?)?;
    write(
        "request.json",
        &serde_json::to_vec_pretty(&case_request(settings))?,
    )?;
    write("wasmtime.csv", to_csv(observed).as_bytes())?;
    let values: Vec<u8> = observed
        .values
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    write("wasmtime.f64", &values)?;
    let steady = case_request_of(settings, 1.0, true);
    let mut fresh = runtime.instantiate(&compiled.wasm, &compiled.description)?;
    if let Ok(result) = fresh.simulate(&steady) {
        write("steady.json", &serde_json::to_vec_pretty(&steady)?)?;
        write("steady.csv", to_csv(&result).as_bytes())?;
    }
    Ok(())
}

impl CaseResult {
    fn with_difference(mut self, max_difference: Option<f64>) -> Self {
        self.max_difference = max_difference;
        self
    }
}

/// A case of the baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expected {
    /// The status the case has.
    pub status: String,
    /// Why it does not pass.
    pub reason: String,
}

/// The baseline of the suite: the cases which do not pass, with their reasons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// The commit of the suite.
    pub suite_commit: String,
    /// The number of cases.
    pub n_cases: usize,
    /// The number of cases which pass.
    pub n_passed: usize,
    /// The cases which do not pass, by identifier.
    pub expected_failures: BTreeMap<String, Expected>,
}

/// A case whose outcome is not the recorded one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Difference {
    /// The identifier of the case.
    pub cid: String,
    /// The recorded status (`pass` for a case which is not listed).
    pub expected: String,
    /// The observed status, `not run` for a listed case which was not run.
    pub observed: String,
    /// The recorded reason of a listed case, else the message of the result.
    pub reason: String,
}

/// The cases the C target refuses (`--model-code c`), with the features it names, as a run of
/// every case writes them (`testdata/testsuite_refused_c.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refusals {
    /// The commit of the suite.
    pub suite_commit: String,
    /// The number of cases refused.
    pub n_refused: usize,
    /// The message of every case refused, by identifier.
    pub refused: BTreeMap<String, String>,
}

/// The refusals of a full run.
pub fn refusals_of(results: &[CaseResult], suite_commit: &str) -> Refusals {
    let refused: BTreeMap<String, String> = results
        .iter()
        .filter(|r| r.status == Status::Refused)
        .map(|r| (r.cid.clone(), r.message.clone()))
        .collect();
    Refusals {
        suite_commit: suite_commit.to_string(),
        n_refused: refused.len(),
        refused,
    }
}

/// The cases which do not have the outcome the baseline records.
///
/// `full` is a run of every case: a listed case which was not run is a difference then.
pub fn differences(results: &[CaseResult], baseline: &Baseline, full: bool) -> Vec<Difference> {
    differences_with(results, baseline, None, full)
}

/// [`differences`] of a run of the C target, whose pinned refusals are `refusals`: a case
/// pinned is expected `refused` with its message, before what the baseline says of it; any
/// other refusal is a difference as any other failure.
pub fn differences_with(
    results: &[CaseResult],
    baseline: &Baseline,
    refusals: Option<&Refusals>,
    full: bool,
) -> Vec<Difference> {
    let pinned = |cid: &str| refusals.and_then(|r| r.refused.get(cid));
    let mut found = Vec::new();
    for result in results {
        if let Some(reason) = pinned(&result.cid) {
            if result.status != Status::Refused || &result.message != reason {
                found.push(Difference {
                    cid: result.cid.clone(),
                    expected: Status::Refused.as_str().to_string(),
                    observed: result.status.as_str().to_string(),
                    reason: if result.status == Status::Refused {
                        format!("refused with '{}', pinned with '{reason}'", result.message)
                    } else {
                        reason.clone()
                    },
                });
            }
            continue;
        }
        let recorded = baseline.expected_failures.get(&result.cid);
        let expected = recorded.map_or("pass", |e| e.status.as_str());
        if result.status.as_str() != expected {
            found.push(Difference {
                cid: result.cid.clone(),
                expected: expected.to_string(),
                observed: result.status.as_str().to_string(),
                reason: recorded.map_or_else(|| result.message.clone(), |e| e.reason.clone()),
            });
        }
    }
    if full {
        for (cid, reason) in refusals.iter().flat_map(|r| &r.refused) {
            if !results.iter().any(|r| &r.cid == cid) {
                found.push(Difference {
                    cid: cid.clone(),
                    expected: Status::Refused.as_str().to_string(),
                    observed: "not run".to_string(),
                    reason: reason.clone(),
                });
            }
        }
        for (cid, recorded) in &baseline.expected_failures {
            if pinned(cid).is_some() {
                continue;
            }
            if !results.iter().any(|r| &r.cid == cid) {
                found.push(Difference {
                    cid: cid.clone(),
                    expected: recorded.status.clone(),
                    observed: "not run".to_string(),
                    reason: recorded.reason.clone(),
                });
            }
        }
    }
    found.sort_by(|a, b| a.cid.cmp(&b.cid));
    found
}

/// The baseline of a full run: the cases which do not pass, with the message of the result as
/// the reason.
pub fn baseline_of(results: &[CaseResult], suite_commit: &str) -> Baseline {
    let expected_failures: BTreeMap<String, Expected> = results
        .iter()
        .filter(|r| r.status != Status::Pass)
        .map(|r| {
            let reason = if r.message.is_empty() {
                MISSING_REASON
            } else {
                &r.message
            };
            (
                r.cid.clone(),
                Expected {
                    status: r.status.as_str().to_string(),
                    reason: reason.to_string(),
                },
            )
        })
        .collect();
    Baseline {
        suite_commit: suite_commit.to_string(),
        n_cases: results.len(),
        n_passed: results.len() - expected_failures.len(),
        expected_failures,
    }
}

/// A selection of cases: `00001-00100`, `00007`, or a comma separated list of both.
pub fn parse_selection(text: &str) -> anyhow::Result<Vec<(u32, u32)>> {
    text.split(',')
        .map(|part| {
            let part = part.trim();
            let (a, b) = part.split_once('-').unwrap_or((part, part));
            let a: u32 = a
                .trim()
                .parse()
                .with_context(|| format!("'{part}' is no case range"))?;
            let b: u32 = b
                .trim()
                .parse()
                .with_context(|| format!("'{part}' is no case range"))?;
            anyhow::ensure!(a <= b, "'{part}' is an empty range");
            Ok((a, b))
        })
        .collect()
}

/// The arguments of `sbmlsim-core testsuite`.
#[derive(Args, Debug)]
pub struct TestsuiteArgs {
    /// The directory of the prepared cases (`scripts/prepare_testsuite.py`).
    pub cases_dir: PathBuf,
    /// The baseline of the expected failures.
    #[arg(long, default_value = "testdata/testsuite_baseline.json")]
    pub baseline: PathBuf,
    /// Only these cases, e.g. `00001-00100` or `00007,00010-00020`.
    #[arg(long)]
    pub cases: Option<String>,
    /// The number of threads (every core without).
    #[arg(long)]
    pub jobs: Option<usize>,
    /// Write the baseline from this run (of every case) instead of comparing with it.
    #[arg(long)]
    pub write_baseline: bool,
    /// Write the results of every case as JSON.
    #[arg(long)]
    pub json: Option<PathBuf>,
    /// The Jacobian the models are compiled with.
    #[arg(long, value_enum, default_value_t = JacobianArg::Auto)]
    pub jacobian: JacobianArg,
    /// Compile the models without the optimizer (`CompileOptions.optimize` false).
    #[arg(long)]
    pub no_optimize: bool,
    /// Write every case which passes into this directory (see the module).
    #[arg(long)]
    pub export: Option<PathBuf>,
    /// The model code: `model.wasm` on `engine.wasm`, or the C target on the native engine
    /// (a build with the feature `native`).
    #[arg(long, value_enum, default_value_t = ModelCode::Wasm)]
    pub model_code: ModelCode,
    /// The cases the C target refuses (`--model-code c`), which `--write-baseline` writes
    /// with it instead of the baseline.
    #[arg(long, default_value = "testdata/testsuite_refused_c.json")]
    pub refused: PathBuf,
    /// Run the cases of the package `comp` from their composed models (`comp/model.xml`),
    /// which the reader of the core flattens, instead of the models libSBML flattened.
    #[arg(long)]
    pub comp: bool,
}

/// The values of `--model-code`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCode {
    /// `model.wasm` on `engine.wasm` in wasmtime.
    Wasm,
    /// The C of the C target, compiled with the C compiler of the machine, on the native
    /// engine.
    C,
}

/// The values of `--jacobian`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JacobianArg {
    /// The analytic Jacobian where the model has one, else the colored difference quotient.
    Auto,
    /// The analytic Jacobian; a model without one is not compiled.
    Analytic,
    /// The colored difference quotient.
    Difference,
}

impl JacobianArg {
    /// The choice of the compiler.
    pub fn choice(self) -> JacobianChoice {
        match self {
            JacobianArg::Auto => JacobianChoice::Auto,
            JacobianArg::Analytic => JacobianChoice::Analytic,
            JacobianArg::Difference => JacobianChoice::DifferenceQuotient,
        }
    }
}

/// The directory of a case with its composed model and the documents of its external model
/// definitions, see [`CaseModel::Comp`].
pub const COMP_DIR: &str = "comp";

/// The case directories of a directory, in the order of their names, filtered by `selection`,
/// and with `comp` the ones with a composed model alone.
fn case_dirs(
    dir: &Path,
    selection: Option<&[(u32, u32)]>,
    comp: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if !path.join("case.json").is_file()
            || (comp && !path.join(COMP_DIR).join("model.xml").is_file())
        {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(ranges) = selection {
            let Ok(n) = name.parse::<u32>() else { continue };
            if !ranges.iter().any(|&(a, b)| (a..=b).contains(&n)) {
                continue;
            }
        }
        dirs.push(path);
    }
    dirs.sort();
    Ok(dirs)
}

/// The command `testsuite`; `Ok(true)` when the run has no differences.
pub fn run(args: &TestsuiteArgs) -> anyhow::Result<bool> {
    let selection = args.cases.as_deref().map(parse_selection).transpose()?;
    anyhow::ensure!(
        !(args.write_baseline && (selection.is_some() || args.comp)),
        "--write-baseline needs a run of every case, without --cases and --comp"
    );
    let model = if args.comp {
        CaseModel::Comp
    } else {
        CaseModel::Flattened
    };
    if args.model_code == ModelCode::C {
        anyhow::ensure!(
            cfg!(feature = "native"),
            "--model-code c requires a build with --features native"
        );
        anyhow::ensure!(
            args.export.is_none(),
            "--export is of model.wasm, not of --model-code c"
        );
        anyhow::ensure!(
            args.jacobian == JacobianArg::Auto && !args.no_optimize,
            "--model-code c compiles with the analytic Jacobian and the optimizer"
        );
    }
    let dirs = case_dirs(&args.cases_dir, selection.as_deref(), args.comp)?;
    anyhow::ensure!(
        !dirs.is_empty(),
        "no case in {}, run scripts/prepare_testsuite.py",
        args.cases_dir.display()
    );
    // the cases must be those of the suite commit of the baseline, before anything runs
    let baseline: Option<Baseline> = if args.write_baseline {
        None
    } else {
        let baseline: Baseline =
            serde_json::from_slice(&fs::read(&args.baseline).with_context(|| {
                format!("cannot read the baseline {}", args.baseline.display())
            })?)
            .with_context(|| format!("cannot read the baseline {}", args.baseline.display()))?;
        check_commits(&dirs, &baseline.suite_commit)?;
        Some(baseline)
    };
    let refusals: Option<Refusals> = if args.write_baseline || args.model_code == ModelCode::Wasm {
        None
    } else {
        let what = || format!("cannot read the refusals {}", args.refused.display());
        let refusals: Refusals =
            serde_json::from_slice(&fs::read(&args.refused).with_context(what)?)
                .with_context(what)?;
        check_commits(&dirs, &refusals.suite_commit)?;
        Some(refusals)
    };
    let runtime = match args.model_code {
        ModelCode::Wasm => Some(Runtime::embedded()?),
        ModelCode::C => None,
    };
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(jobs) = args.jobs {
        anyhow::ensure!(jobs > 0, "--jobs must be positive");
        builder = builder.num_threads(jobs);
    }
    let pool = builder.build()?;
    // rayon keeps the order of the directories
    let options = CompileOptions {
        jacobian: args.jacobian.choice(),
        optimize: !args.no_optimize,
        ..CompileOptions::default()
    };
    if let Some(out) = &args.export {
        write_runtime(out)?;
    }
    let results: Vec<CaseResult> = pool.install(|| {
        dirs.par_iter()
            .map(|dir| match &runtime {
                Some(runtime) => {
                    run_case_exported(runtime, dir, &options, args.export.as_deref(), model)
                }
                #[cfg(feature = "native")]
                None => run_case_c_of(dir, model),
                #[cfg(not(feature = "native"))]
                None => unreachable!("--model-code c is refused without the feature native"),
            })
            .collect()
    });

    let mut summary = String::new();
    writeln!(summary, "{} cases", results.len())?;
    for status in Status::ALL {
        let n = results.iter().filter(|r| r.status == status).count();
        if n > 0 {
            writeln!(summary, "  {:<17}{n}", status.as_str())?;
        }
    }
    let jacobians = |kind: &str| {
        results
            .iter()
            .filter(|r| r.jacobian.as_deref() == Some(kind))
            .count()
    };
    writeln!(
        summary,
        "jacobians of the compiled models: {} analytic, {} colored difference",
        jacobians("analytic"),
        jacobians("colored_difference")
    )?;
    let refused: Vec<&CaseResult> = results
        .iter()
        .filter(|r| r.status == Status::Refused)
        .collect();
    if !refused.is_empty() {
        writeln!(summary, "refused by the C target: {}", refused.len())?;
        for r in &refused {
            writeln!(summary, "  {}: {}", r.cid, r.message)?;
        }
    }
    print!("{summary}");

    if args.write_baseline {
        let first = dirs
            .first()
            .map(|d| fs::read(d.join("case.json")))
            .transpose()?
            .map(|b| serde_json::from_slice::<CaseSettings>(&b))
            .transpose()?
            .map(|s| s.suite_commit)
            .unwrap_or_default();
        check_commits(&dirs, &first)?;
        if args.model_code == ModelCode::C {
            let refusals = refusals_of(&results, &first);
            fs::write(
                &args.refused,
                serde_json::to_string_pretty(&refusals)? + "\n",
            )
            .with_context(|| format!("cannot write {}", args.refused.display()))?;
            println!(
                "refusals written to {}: {} of {} cases refused",
                args.refused.display(),
                refusals.n_refused,
                results.len()
            );
            write_json(args, &first, &results, &[])?;
            return Ok(true);
        }
        let baseline = baseline_of(&results, &first);
        fs::write(
            &args.baseline,
            serde_json::to_string_pretty(&baseline)? + "\n",
        )
        .with_context(|| format!("cannot write {}", args.baseline.display()))?;
        println!(
            "baseline written to {}: {} of {} cases pass",
            args.baseline.display(),
            baseline.n_passed,
            baseline.n_cases
        );
        write_json(args, &first, &results, &[])?;
        return Ok(true);
    }

    let baseline = baseline.expect("read above");
    let full = selection.is_none() && !args.comp;
    let found = differences_with(&results, &baseline, refusals.as_ref(), full);
    report_differences(&found);
    write_json(args, &baseline.suite_commit, &results, &found)?;
    Ok(found.is_empty())
}

/// Selects the differences of a kind.
type Selector = fn(&Difference) -> bool;

/// Lists the differences by kind on the console.
fn report_differences(found: &[Difference]) {
    if found.is_empty() {
        println!("no difference to the baseline");
        return;
    }
    let kinds: [(&str, Selector); 3] = [
        ("regressions (not listed, fail)", |d| d.expected == "pass"),
        ("listed and pass (the baseline is out of date)", |d| {
            d.observed == "pass"
        }),
        ("other differences", |d| {
            d.expected != "pass" && d.observed != "pass"
        }),
    ];
    for (title, select) in kinds {
        let of_kind: Vec<&Difference> = found.iter().filter(|d| select(d)).collect();
        if of_kind.is_empty() {
            continue;
        }
        println!("{title}: {}", of_kind.len());
        for d in of_kind.iter().take(LISTED_DIFFERENCES) {
            println!(
                "  {}: '{}' -> '{}': {}",
                d.cid, d.expected, d.observed, d.reason
            );
        }
        if of_kind.len() > LISTED_DIFFERENCES {
            println!(
                "  ... and {} more (--json lists every case)",
                of_kind.len() - LISTED_DIFFERENCES
            );
        }
    }
    println!("{} differences to the baseline", found.len());
}

/// Writes the JSON of the results.
fn write_json(
    args: &TestsuiteArgs,
    commit: &str,
    results: &[CaseResult],
    found: &[Difference],
) -> anyhow::Result<()> {
    let Some(path) = &args.json else {
        return Ok(());
    };
    let counts: BTreeMap<&str, usize> = Status::ALL
        .iter()
        .map(|s| {
            (
                s.as_str(),
                results.iter().filter(|r| r.status == *s).count(),
            )
        })
        .collect();
    let value = serde_json::json!({
        "suite_commit": commit,
        "n_cases": results.len(),
        "counts": counts,
        "results": results,
        "differences": found,
    });
    fs::write(path, serde_json::to_string_pretty(&value)? + "\n")
        .with_context(|| format!("cannot write {}", path.display()))
}

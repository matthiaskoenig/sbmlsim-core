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
}

impl Status {
    /// Every status, in the order of the report.
    pub const ALL: [Status; 6] = [
        Status::Pass,
        Status::Tolerance,
        Status::NotCompiled,
        Status::SimulationError,
        Status::MissingVariable,
        Status::Repeat,
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
    let random = settings
        .test_tags
        .iter()
        .any(|t| t == RANDOM_EVENT_EXECUTION);
    let request = case_request(settings);
    let simulate = |instance: &mut Instance, req: &SimulationRequest, what: &str| {
        instance
            .simulate(req)
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
    let reference = match runtime.instantiate(&compiled.wasm, &compiled.description) {
        Ok(mut fresh) => fresh.simulate(&steady).map_err(|e| e.to_string()),
        Err(error) => return Some(format!("a fresh instance failed: {error:#}")),
    };
    let observed = instance.simulate(&steady).map_err(|e| e.to_string());
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
pub fn run_case(runtime: &Runtime, dir: &Path) -> CaseResult {
    run_case_with(runtime, dir, &CompileOptions::default())
}

/// Runs the case in a directory with a model compiled with `options`.
pub fn run_case_with(runtime: &Runtime, dir: &Path, options: &CompileOptions) -> CaseResult {
    run_case_exported(runtime, dir, options, None)
}

/// [`run_case_with`], which writes a case which passes into `export` (`--export`).
pub fn run_case_exported(
    runtime: &Runtime,
    dir: &Path,
    options: &CompileOptions,
    export: Option<&Path>,
) -> CaseResult {
    let cid = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match catch_unwind(AssertUnwindSafe(|| {
        run_case_inner(runtime, dir, &cid, options, export)
    })) {
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
) -> CaseResult {
    let compiled = OnceCell::new();
    let result = run_compiled(runtime, dir, cid, options, &compiled, export);
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
) -> CaseResult {
    let settings: CaseSettings = match fs::read(dir.join("case.json"))
        .map_err(anyhow::Error::from)
        .and_then(|b| serde_json::from_slice(&b).map_err(anyhow::Error::from))
    {
        Ok(settings) => settings,
        Err(error) => {
            return CaseResult::new(cid, Status::SimulationError, format!("case.json: {error}"));
        }
    };
    if let Some(message) = &settings.not_converted {
        return CaseResult::new(
            cid,
            Status::NotCompiled,
            format!("not converted: {message}"),
        );
    }
    let sbml = match fs::read_to_string(dir.join("model.xml")) {
        Ok(sbml) => sbml,
        Err(error) => {
            return CaseResult::new(cid, Status::SimulationError, format!("model.xml: {error}"));
        }
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
    let observed = match instance.simulate(&case_request(&settings)) {
        Ok(observed) => observed,
        Err(error) => return CaseResult::new(cid, Status::SimulationError, error.to_string()),
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
    if let Some(message) = check_columns(&expected, &settings) {
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
        Ok(max_difference) => match repeat_check(
            runtime,
            compiled,
            &mut instance,
            &settings,
            &expected,
            &observed,
        ) {
            Some(message) => CaseResult::new(cid, Status::Repeat, message),
            None => {
                if let Some(out) = export
                    && let Err(error) =
                        export_case(runtime, compiled, &settings, &observed, out, cid)
                {
                    return CaseResult::new(
                        cid,
                        Status::SimulationError,
                        format!("the export failed: {error:#}"),
                    );
                }
                CaseResult::new(cid, Status::Pass, "").with_difference(Some(max_difference))
            }
        },
    }
}

/// The JSON of a description as `sbmlsim-core compile` writes `model.json`.
pub fn description_json(desc: &ModelDescription) -> anyhow::Result<Vec<u8>> {
    let mut json = serde_json::to_vec_pretty(desc)?;
    json.push(b'\n');
    Ok(json)
}

/// Writes the `engine.wasm`, `mathlib.wasm` and `compiler.wasm` of this build into `dir`, and
/// next to them `THIRD_PARTY.md`, the notices of the libraries `engine.wasm` links statically
/// (a redistributed `engine.wasm` carries them).
///
/// `compiler.wasm` is the compiler of a JavaScript host (`js/compile.mjs`), which compiles an
/// SBML document in a page into the `model.wasm` the engine runs.
pub fn write_runtime(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    for (name, bytes) in [
        ("engine.wasm", sbmlsim_host::ENGINE_WASM),
        ("mathlib.wasm", sbmlsim_host::MATHLIB_WASM),
        ("compiler.wasm", sbmlsim_host::COMPILER_WASM),
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

/// The cases which do not have the outcome the baseline records.
///
/// `full` is a run of every case: a listed case which was not run is a difference then.
pub fn differences(results: &[CaseResult], baseline: &Baseline, full: bool) -> Vec<Difference> {
    let mut found = Vec::new();
    for result in results {
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
        for (cid, recorded) in &baseline.expected_failures {
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

/// The case directories of a directory, in the order of their names, filtered by `selection`.
fn case_dirs(dir: &Path, selection: Option<&[(u32, u32)]>) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if !path.join("case.json").is_file() {
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
        !(args.write_baseline && selection.is_some()),
        "--write-baseline needs a run of every case, without --cases"
    );
    let dirs = case_dirs(&args.cases_dir, selection.as_deref())?;
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
    let runtime = Runtime::embedded()?;
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
    };
    if let Some(out) = &args.export {
        write_runtime(out)?;
    }
    let results: Vec<CaseResult> = pool.install(|| {
        dirs.par_iter()
            .map(|dir| run_case_exported(&runtime, dir, &options, args.export.as_deref()))
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
    let found = differences(&results, &baseline, selection.is_none());
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

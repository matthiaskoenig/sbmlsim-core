//! PEtab v2 in a browser: the problem read and planned in `compiler.wasm`, its simulations run
//! by the engine of the host, its likelihood evaluated in `compiler.wasm` again.
//!
//! The engine of a page is `engine.wasm` under the loader of JavaScript, which simulates a
//! request of `simulation/1`; everything else of a problem is Rust of `sbmlsim-petab`, so a
//! page evaluates a problem as the command line and python do:
//!
//! 1. [`open`] reads a problem from the text of its files and plans it, and gives a handle and
//!    a summary of it (its models, its estimated parameters with their nominal values and
//!    bounds, its simulations and measurements),
//! 2. [`model`] compiles a model of the plan, the observables of the problem added to it,
//! 3. [`requests`] gives the request of every simulation at the values of the estimated
//!    parameters, with the index of its model, which the host simulates,
//! 4. [`evaluate`] reads the results of those requests and gives the evaluation of PEtab: the
//!    log-likelihood, `chi2`, the log priors, the simulated value of every measurement and the
//!    simulation table; the values of the results come as the bytes of their `f64`s, the
//!    `Float64Array` of the loader, so they cross exactly, `NaN` and the infinities included,
//!    and are not written into JSON and parsed again,
//! 5. [`close`] drops the problem.
//!
//! A fit of the problem runs the same way, ask/tell ([`sbmlsim_fit::Asking`]): the optimizer is
//! the one of the command line, in this module, and the page simulates what it asks for.
//!
//! 1. [`fit_open`] starts a fit of an open problem with the settings of `sbmlsim-fit`,
//! 2. [`fit_ask`] answers the requests of the simulations the optimizer needs next, each with
//!    the index of its model, or the fit once it is done,
//! 3. [`fit_tell`] takes their results, as [`evaluate`] takes them,
//! 4. [`fit_stop`] stops it at its next step, and [`fit_close`] drops it.
//!
//! The C ABI of the module carries the same calls as `petab_open`, `petab_model`,
//! `petab_requests`, `petab_evaluate`, `petab_close`, `petab_fit_open`, `petab_fit_ask`,
//! `petab_fit_tell`, `petab_fit_stop` and `petab_fit_close`, every text as UTF-8 JSON in the
//! buffers of [`crate::allocate`].

use std::cell::RefCell;
use std::collections::HashMap;

#[cfg(feature = "fit")]
use sbmlsim_abi::EngineError;
use sbmlsim_abi::{IntegratorSettings, SimulationResult, Tolerance};
#[cfg(feature = "fit")]
use sbmlsim_fit::{Ask, Asking, Opening, Settings};
use sbmlsim_petab::{DataAtChange, ExtensionPolicy, Options, PetabProblem, Plan};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Compiled, answer, bytes, give, put_word};

thread_local! {
    /// The open problems, by their handle, their index.
    static PROBLEMS: RefCell<Vec<Option<Plan>>> = const { RefCell::new(Vec::new()) };
    /// The fits of the open problems, by their handle, their index.
    #[cfg(feature = "fit")]
    static FITS: RefCell<Vec<Option<RunningFit>>> = const { RefCell::new(Vec::new()) };
}

/// A fit of a problem: its optimizer and what its cost carries on top of the objective of
/// PEtab ([`Plan::cost_offset`]).
#[cfg(feature = "fit")]
struct RunningFit {
    asking: Asking,
    offset: f64,
}

/// A problem a page opens: the name of the YAML file and the text of every file by its name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenRequest {
    yaml: String,
    files: HashMap<String, String>,
    #[serde(default)]
    options: OpenOptions,
}

/// The options of [`open`], those of `sbmlsim_core.petab.Problem.from_files`.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct OpenOptions {
    relative_tolerance: Option<f64>,
    absolute_tolerance: Option<f64>,
    data_at_change: Option<String>,
    extension: Option<String>,
}

/// A request of a simulation of the plan and the index of its model.
#[derive(Serialize)]
struct PlannedRequest {
    model: usize,
    request: sbmlsim_abi::SimulationRequest,
}

/// The result of a simulation as the host gives it: the shape of the table of `simulation/1`,
/// whose values are in the buffer of the values, or the message of its failure.
#[derive(Deserialize)]
#[serde(untagged)]
enum HostResult {
    Table {
        columns: Vec<String>,
        n_rows: usize,
    },
    Failure {
        error: String,
        /// The kind of the error of the engine, `integration` for an integration which
        /// failed: a fit reads a failed integration as a point it may not take, and any other
        /// failure as its own.
        #[serde(default)]
        kind: Option<String>,
    },
}

/// What [`evaluate`] reads: the values of the estimated parameters (their nominal values when
/// absent) and the result of every request of [`requests`] at them, in their order.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluateRequest {
    #[serde(default)]
    x: Option<Vec<f64>>,
    results: Vec<HostResult>,
}

/// A number of an answer: JSON has no `NaN` and no infinities (`serde_json` writes `null` for
/// them), so they are the strings `"NaN"`, `"Infinity"` and `"-Infinity"`, which `Number` of
/// JavaScript reads back; a log-likelihood of a failed simulation is `NaN`, a log prior
/// outside of the bounds `-inf`.
fn number(value: f64) -> Value {
    if value.is_finite() {
        Value::from(value)
    } else if value.is_nan() {
        Value::from("NaN")
    } else if value > 0.0 {
        Value::from("Infinity")
    } else {
        Value::from("-Infinity")
    }
}

/// The JSON of an evaluation: the fields of [`sbmlsim_petab::Evaluation`], every number by
/// [`number`], with the unnormalized log posterior and the simulation table.
fn evaluation_answer(plan: &Plan, evaluation: &sbmlsim_petab::Evaluation) -> Value {
    let numbers = |values: &[f64]| Value::Array(values.iter().copied().map(number).collect());
    json!({
        "llh": number(evaluation.llh),
        "chi2": number(evaluation.chi2),
        "log_prior": number(evaluation.log_prior),
        "log_priors": evaluation
            .log_priors
            .iter()
            .map(|(id, value)| json!([id, number(*value)]))
            .collect::<Vec<_>>(),
        "unnorm_log_posterior": number(evaluation.unnorm_log_posterior()),
        "simulated": numbers(&evaluation.simulated),
        "sigma": numbers(&evaluation.sigma),
        "failures": evaluation.failures,
        "simulation_table": plan.simulation_table(evaluation),
    })
}

fn with_plan<T>(handle: usize, work: impl FnOnce(&Plan) -> Result<T, String>) -> Result<T, String> {
    PROBLEMS.with(|problems| {
        let problems = problems.borrow();
        let plan = problems
            .get(handle)
            .and_then(Option::as_ref)
            .ok_or_else(|| format!("there is no open problem {handle}"))?;
        work(plan)
    })
}

fn point(plan: &Plan, x: Option<Vec<f64>>) -> Result<Vec<f64>, String> {
    let wanted = plan.estimated().len();
    match x {
        None => Ok(plan.nominal()),
        Some(x) if x.len() == wanted => Ok(x),
        Some(x) => Err(format!(
            "the problem estimates {wanted} parameters and the point has {} values",
            x.len()
        )),
    }
}

/// Reads and plans a problem from the JSON `{"yaml": name, "files": {name: text}, "options":
/// {...}}` and gives the JSON of its summary, whose `handle` the other calls take: its
/// `models`, its estimated `parameters` with their `nominal` values and their `lower` and
/// `upper` bounds (an infinite bound as the string of `number`) and the numbers of its
/// `simulations` and `measurements`.
///
/// # Errors
///
/// A request which is no such JSON and a problem which is not read or planned.
pub fn open(request: &[u8]) -> Result<String, String> {
    let request: OpenRequest = serde_json::from_slice(request).map_err(|err| err.to_string())?;
    let files: HashMap<String, Vec<u8>> = request
        .files
        .into_iter()
        .map(|(name, text)| (name, text.into_bytes()))
        .collect();
    let mut options = Options::default();
    let o = request.options;
    if o.relative_tolerance.is_some() || o.absolute_tolerance.is_some() {
        let mut settings = IntegratorSettings {
            relative_tolerance: 1e-8,
            ..IntegratorSettings::default()
        };
        if let Some(relative) = o.relative_tolerance {
            settings.relative_tolerance = relative;
        }
        if let Some(absolute) = o.absolute_tolerance {
            settings.absolute_tolerance = Tolerance::Scalar(absolute);
        }
        options.integrator = Some(settings);
    }
    options.data_at_change = match o.data_at_change.as_deref() {
        None => None,
        Some("after") => Some(DataAtChange::After),
        Some("before") => Some(DataAtChange::Before),
        Some(other) => {
            return Err(format!(
                "data_at_change is 'after' or 'before', not '{other}'"
            ));
        }
    };
    options.extension = match o.extension.as_deref() {
        None | Some("read") => ExtensionPolicy::Read,
        Some("ignore") => ExtensionPolicy::Ignore,
        Some(other) => return Err(format!("extension is 'read' or 'ignore', not '{other}'")),
    };
    let problem = PetabProblem::read(&request.yaml, &files).map_err(|err| err.to_string())?;
    let plan = Plan::new(&problem, &options).map_err(|err| err.to_string())?;
    let estimated = plan.estimated();
    let numbers = |values: Vec<f64>| Value::Array(values.into_iter().map(number).collect());
    let mut summary = json!({
        "models": plan.models.iter().map(|model| model.id.clone()).collect::<Vec<_>>(),
        "parameters": estimated
            .iter()
            .map(|k| plan.parameters[*k].id.clone())
            .collect::<Vec<_>>(),
        "nominal": numbers(plan.nominal()),
        "lower": numbers(estimated.iter().map(|k| plan.parameters[*k].lower).collect()),
        "upper": numbers(estimated.iter().map(|k| plan.parameters[*k].upper).collect()),
        "simulations": plan.simulations.len(),
        "measurements": plan.measurements.len(),
    });
    summary["handle"] = Value::from(PROBLEMS.with(|problems| {
        let mut problems = problems.borrow_mut();
        problems.push(Some(plan));
        problems.len() - 1
    }));
    Ok(summary.to_string())
}

/// Compiles the model `k` of an open problem, with the observables the plan added to it.
///
/// # Errors
///
/// A handle of no open problem, a model it has not and the refusal of the compiler.
pub fn model(handle: usize, k: usize) -> Result<Compiled, String> {
    with_plan(handle, |plan| {
        if k >= plan.models.len() {
            return Err(format!("the problem has {} models", plan.models.len()));
        }
        let compiled = plan.compile(k).map_err(|err| err.to_string())?;
        let description =
            serde_json::to_string(&compiled.description).map_err(|err| err.to_string())?;
        Ok(Compiled {
            description,
            wasm: compiled.wasm,
        })
    })
}

/// The JSON of the requests of every simulation of an open problem at the values of its
/// estimated parameters (a JSON array, `null` or nothing for the nominal values), each with
/// the index of its model.
///
/// # Errors
///
/// A handle of no open problem and values which are not one per estimated parameter.
pub fn requests(handle: usize, x: &[u8]) -> Result<String, String> {
    with_plan(handle, |plan| {
        let x: Option<Vec<f64>> = if x.is_empty() {
            None
        } else {
            serde_json::from_slice(x).map_err(|err| err.to_string())?
        };
        let x = point(plan, x)?;
        let requests: Vec<PlannedRequest> = (0..plan.simulations.len())
            .map(|simulation| PlannedRequest {
                model: plan.simulations[simulation].model,
                request: plan.request(simulation, &x),
            })
            .collect();
        serde_json::to_string(&requests).map_err(|err| err.to_string())
    })
}

/// The JSON of the evaluation of an open problem from the results of its requests (`llh`,
/// `chi2`, `log_prior`, `log_priors` as pairs of the id and the value, `unnorm_log_posterior`,
/// `simulated` and `sigma` per measurement, `failures` as pairs of the simulation and the
/// message and `simulation_table`, the numbers which are not finite as strings): the JSON
/// `{"x": [...], "results": [...]}` of the shape `{"columns": [...], "n_rows": n}` of a result
/// of `simulation/1` or `{"error": message}` per request, and the values of the tables of the
/// results one after the other, each row major, as the little-endian bytes of their `f64`s.
///
/// # Errors
///
/// A handle of no open problem, an input which is no such JSON, results which are not one
/// per simulation and values which are not those of the tables.
pub fn evaluate(handle: usize, input: &[u8], values: &[u8]) -> Result<String, String> {
    with_plan(handle, |plan| {
        let input: EvaluateRequest =
            serde_json::from_slice(input).map_err(|err| err.to_string())?;
        let x = point(plan, input.x)?;
        if input.results.len() != plan.simulations.len() {
            return Err(format!(
                "the problem has {} simulations and the input {} results",
                plan.simulations.len(),
                input.results.len()
            ));
        }
        let results: Vec<Result<SimulationResult, String>> = tables(input.results, values)?
            .into_iter()
            .map(|result| result.map_err(|(error, _)| error))
            .collect();
        let evaluation = plan.evaluate(&x, &results);
        Ok(evaluation_answer(plan, &evaluation).to_string())
    })
}

/// A result of a host: its table, or the message and the kind of its failure.
type Told = Result<SimulationResult, (String, Option<String>)>;

/// The results of a host with the values of their tables, the little-endian bytes of their
/// `f64`s one table after the other.
fn tables(results: Vec<HostResult>, values: &[u8]) -> Result<Vec<Told>, String> {
    let (chunks, rest) = values.as_chunks::<8>();
    if !rest.is_empty() {
        return Err(format!(
            "the values are {} bytes, no whole number of f64",
            values.len()
        ));
    }
    let mut values = chunks.iter().map(|bytes| f64::from_le_bytes(*bytes));
    let mut tables = Vec::with_capacity(results.len());
    for result in results {
        tables.push(match result {
            HostResult::Table { columns, n_rows } => {
                let n = n_rows
                    .checked_mul(columns.len())
                    .ok_or("a table of more values than an address")?;
                let table: Vec<f64> = values.by_ref().take(n).collect();
                if table.len() != n {
                    return Err("the values end before the tables of the results".to_string());
                }
                Ok(SimulationResult {
                    columns,
                    n_rows,
                    values: table,
                    sensitivities: None,
                    local_errors: None,
                })
            }
            HostResult::Failure { error, kind } => Err((error, kind)),
        });
    }
    if values.next().is_some() {
        return Err("the values are more than the tables of the results".to_string());
    }
    Ok(tables)
}

/// What [`fit_open`] reads: the settings of the optimizer of `sbmlsim-fit` (the command line
/// fits with their defaults) and the values of the estimated parameters it starts from, their
/// nominal values when absent.
#[cfg(feature = "fit")]
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FitRequest {
    settings: Settings,
    start: Option<Vec<f64>>,
}

/// What [`fit_tell`] reads: the result of every simulation of the last [`fit_ask`], in its
/// order, as [`evaluate`] reads the results of the requests of a problem.
#[cfg(feature = "fit")]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TellRequest {
    results: Vec<HostResult>,
}

#[cfg(feature = "fit")]
fn with_fit<T>(
    fit: usize,
    work: impl FnOnce(&mut RunningFit) -> Result<T, String>,
) -> Result<T, String> {
    FITS.with(|fits| {
        let mut fits = fits.borrow_mut();
        let running = fits
            .get_mut(fit)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("there is no open fit {fit}"))?;
        work(running)
    })
}

/// Starts a fit of an open problem from the JSON `{"settings": {...}, "start": [...]}` (both
/// optional; the settings of `sbmlsim_fit::Settings`, whose `max_seconds` the host measures)
/// and gives the JSON `{"fit": handle, "offset": number}`: the handle the other calls of a
/// fit take and what its cost carries on top of the objective of PEtab.
///
/// The fit is the one of `Executor::fit` of the command line: the same definition, the same
/// optimizer with the same tolerances where the settings give none (`Plan::fit_settings`) and,
/// when the host simulates with `engine.wasm`, the same evaluations.
///
/// # Errors
///
/// A handle of no open problem, an input which is no such JSON, a start which is not one value
/// per estimated parameter, and a problem whose fit the engine does not carry.
#[cfg(feature = "fit")]
pub fn fit_open(handle: usize, input: &[u8]) -> Result<String, String> {
    let request: FitRequest = if input.is_empty() {
        FitRequest::default()
    } else {
        serde_json::from_slice(input).map_err(|err| err.to_string())?
    };
    let (asking, offset) = with_plan(handle, |plan| {
        let start = point(plan, request.start)?;
        let definition = plan.definition(&start).map_err(|err| err.to_string())?;
        let offset = plan.cost_offset().map_err(|err| err.to_string())?;
        let models: Vec<String> = plan.models.iter().map(|model| model.id.clone()).collect();
        let opening = Opening::new(definition, &models).map_err(|err| err.to_string())?;
        Ok((
            Asking::open(opening, plan.fit_settings(&request.settings)),
            offset,
        ))
    })?;
    let fit = FITS.with(|fits| {
        let mut fits = fits.borrow_mut();
        fits.push(Some(RunningFit { asking, offset }));
        fits.len() - 1
    });
    Ok(json!({ "fit": fit, "offset": number(offset) }).to_string())
}

/// What a fit needs next, as JSON: `{"simulate": [{"model": k, "request": {...}}, ...]}`, the
/// requests of `simulation/1` of the simulations its optimizer needs, which [`fit_tell`]
/// answers in their order, or `{"done": {...}}`, the fit: the values of the estimated
/// parameters at the optimum (`x`), the `objective` of PEtab there (the negative log posterior,
/// the negative log-likelihood of a problem without priors), the `cost` of `sbmlsim-fit`, the
/// `iterations`, the `evaluations` and the `jacobians` the optimizer took and its `status`.
///
/// # Errors
///
/// A handle of no open fit and a failure of the optimizer.
#[cfg(feature = "fit")]
pub fn fit_ask(fit: usize) -> Result<String, String> {
    with_fit(fit, |running| {
        match running.asking.ask().map_err(|err| err.to_string())? {
            Ask::Simulate(simulations) => {
                let requests: Vec<PlannedRequest> = simulations
                    .into_iter()
                    .map(|simulation| PlannedRequest {
                        model: simulation.model,
                        request: simulation.request,
                    })
                    .collect();
                Ok(json!({ "simulate": requests }).to_string())
            }
            Ask::Done(fit) => Ok(json!({
                "done": {
                    "x": fit.values.iter().copied().map(number).collect::<Vec<_>>(),
                    "objective": number(fit.cost - running.offset),
                    "cost": number(fit.cost),
                    "iterations": fit.iterations,
                    "evaluations": fit.evaluations,
                    "jacobians": fit.jacobians,
                    "status": fit.status,
                }
            })
            .to_string()),
        }
    })
}

/// Takes the results of the simulations of the last [`fit_ask`]: the JSON `{"results": [...]}`
/// of the shape of a table or `{"error": message, "kind": kind}` per simulation and the values
/// of the tables as the bytes of [`evaluate`]. An integration which failed (`kind`
/// `integration`) is a point the fit may not take; any other failure ends the fit with an
/// error.
///
/// # Errors
///
/// A handle of no open fit, an input which is no such JSON, results which are not one per
/// simulation asked for, values which are not those of the tables, and a failure of the
/// engine which is no failed integration.
#[cfg(feature = "fit")]
pub fn fit_tell(fit: usize, input: &[u8], values: &[u8]) -> Result<String, String> {
    let input: TellRequest = serde_json::from_slice(input).map_err(|err| err.to_string())?;
    let results = tables(input.results, values)?
        .into_iter()
        .map(|result| {
            result.map_err(|(message, kind)| match kind.as_deref() {
                Some("integration") => EngineError::Integration {
                    flag: 0,
                    time: f64::NAN,
                    message,
                },
                Some("request") => EngineError::Request(message),
                Some("unsupported") => EngineError::Unsupported(message),
                _ => EngineError::Internal(message),
            })
        })
        .collect();
    with_fit(fit, |running| {
        running
            .asking
            .tell(results)
            .map_err(|err| err.to_string())?;
        Ok(String::new())
    })
}

/// Stops a fit at its next step: its next ask gives the best point it reached.
#[cfg(feature = "fit")]
pub fn fit_stop(fit: usize) {
    let _ = with_fit(fit, |running| {
        running.asking.stop();
        Ok(())
    });
}

/// Drops a fit; a handle of none is ignored.
#[cfg(feature = "fit")]
pub fn fit_close(fit: usize) {
    FITS.with(|fits| {
        if let Some(slot) = fits.borrow_mut().get_mut(fit) {
            *slot = None;
        }
    });
}

/// Drops an open problem; a handle of none is ignored.
pub fn close(handle: usize) {
    PROBLEMS.with(|problems| {
        if let Some(slot) = problems.borrow_mut().get_mut(handle) {
            *slot = None;
        }
    });
}

/// [`open`] of the C ABI: writes the summary, or the message of a refusal, to the two words at
/// `out` and gives 1, or 0 for a refusal.
///
/// # Safety
///
/// `pointer` is a buffer of [`crate::allocate`] of `len` bytes, `out` two words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_open(pointer: *const u8, len: usize, out: *mut usize) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(open(bytes(pointer, len)), out) }
}

/// [`model`] of the C ABI, with the four words of [`crate::compile`] at `out`.
///
/// # Safety
///
/// `out` is four words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_model(handle: usize, k: usize, out: *mut usize) -> u32 {
    match model(handle, k) {
        Ok(compiled) => {
            // SAFETY: the caller gives four words of its own memory
            unsafe {
                give(compiled.description.into_bytes(), out);
                give(compiled.wasm, out.add(2));
            }
            1
        }
        Err(message) => {
            // SAFETY: the caller gives four words of its own memory
            unsafe {
                give(message.into_bytes(), out);
                put_word(out, 2, 0);
                put_word(out, 3, 0);
            }
            0
        }
    }
}

/// [`requests`] of the C ABI, with the two words of [`petab_open`] at `out`.
///
/// # Safety
///
/// As [`petab_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_requests(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(requests(handle, bytes(pointer, len)), out) }
}

/// [`evaluate`] of the C ABI: the JSON at `pointer`, the bytes of the values at `values`,
/// with the two words of [`petab_open`] at `out`.
///
/// # Safety
///
/// `pointer` and `values` are buffers of [`crate::allocate`] of `len` and `values_len` bytes
/// (or null and 0), `out` two words of the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_evaluate(
    handle: usize,
    pointer: *const u8,
    len: usize,
    values: *const u8,
    values_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe {
        answer(
            evaluate(handle, bytes(pointer, len), bytes(values, values_len)),
            out,
        )
    }
}

/// [`close`] of the C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn petab_close(handle: usize) {
    close(handle);
}

/// [`fit_open`] of the C ABI, with the two words of [`petab_open`] at `out`.
///
/// # Safety
///
/// As [`petab_open`].
#[cfg(feature = "fit")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_fit_open(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(fit_open(handle, bytes(pointer, len)), out) }
}

/// [`fit_ask`] of the C ABI, with the two words of [`petab_open`] at `out`.
///
/// # Safety
///
/// `out` is two words of the caller.
#[cfg(feature = "fit")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_fit_ask(fit: usize, out: *mut usize) -> u32 {
    // SAFETY: the buffer is the caller's, as documented
    unsafe { answer(fit_ask(fit), out) }
}

/// [`fit_tell`] of the C ABI, with the buffers of [`petab_evaluate`].
///
/// # Safety
///
/// As [`petab_evaluate`].
#[cfg(feature = "fit")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_fit_tell(
    fit: usize,
    pointer: *const u8,
    len: usize,
    values: *const u8,
    values_len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe {
        answer(
            fit_tell(fit, bytes(pointer, len), bytes(values, values_len)),
            out,
        )
    }
}

/// [`fit_stop`] of the C ABI.
#[cfg(feature = "fit")]
#[unsafe(no_mangle)]
pub extern "C" fn petab_fit_stop(fit: usize) {
    fit_stop(fit);
}

/// [`fit_close`] of the C ABI.
#[cfg(feature = "fit")]
#[unsafe(no_mangle)]
pub extern "C" fn petab_fit_close(fit: usize) {
    fit_close(fit);
}

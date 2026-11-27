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
//!    simulation table,
//! 5. [`close`] drops the problem.
//!
//! The C ABI of the module carries the same calls as `petab_open`, `petab_model`,
//! `petab_requests`, `petab_evaluate` and `petab_close`, every text as UTF-8 JSON in the
//! buffers of [`crate::allocate`].

use std::cell::RefCell;
use std::collections::HashMap;

use sbmlsim_abi::{IntegratorSettings, SimulationResult, Tolerance};
use sbmlsim_petab::{DataAtChange, ExtensionPolicy, Options, PetabProblem, Plan};
use serde::{Deserialize, Serialize};

use crate::{Compiled, give};

thread_local! {
    /// The open problems, by their handle, their index.
    static PROBLEMS: RefCell<Vec<Option<Plan>>> = const { RefCell::new(Vec::new()) };
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

/// What [`open`] says about a problem.
#[derive(Serialize)]
struct Summary {
    handle: usize,
    models: Vec<String>,
    parameters: Vec<String>,
    nominal: Vec<f64>,
    lower: Vec<f64>,
    upper: Vec<f64>,
    simulations: usize,
    measurements: usize,
}

/// A request of a simulation of the plan and the index of its model.
#[derive(Serialize)]
struct PlannedRequest {
    model: usize,
    request: sbmlsim_abi::SimulationRequest,
}

/// The result of a simulation as the host gives it: the table of `simulation/1`, or the
/// message of its failure.
#[derive(Deserialize)]
#[serde(untagged)]
enum HostResult {
    Table(SimulationResult),
    Failure { error: String },
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

#[derive(Serialize)]
struct EvaluationAnswer {
    #[serde(flatten)]
    evaluation: sbmlsim_petab::Evaluation,
    unnorm_log_posterior: f64,
    simulation_table: String,
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
/// {...}}` and gives the JSON of its summary, whose `handle` the other calls take.
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
    let parameter = |k: &usize| &plan.parameters[*k];
    let mut summary = Summary {
        handle: 0,
        models: plan.models.iter().map(|model| model.id.clone()).collect(),
        parameters: estimated.iter().map(|k| parameter(k).id.clone()).collect(),
        nominal: plan.nominal(),
        lower: estimated.iter().map(|k| parameter(k).lower).collect(),
        upper: estimated.iter().map(|k| parameter(k).upper).collect(),
        simulations: plan.simulations.len(),
        measurements: plan.measurements.len(),
    };
    summary.handle = PROBLEMS.with(|problems| {
        let mut problems = problems.borrow_mut();
        problems.push(Some(plan));
        problems.len() - 1
    });
    serde_json::to_string(&summary).map_err(|err| err.to_string())
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

/// The JSON of the evaluation of an open problem from the results of its requests, the JSON
/// `{"x": [...], "results": [...]}` of a result of `simulation/1` or `{"error": message}` per
/// request.
///
/// # Errors
///
/// A handle of no open problem, an input which is no such JSON and results which are not one
/// per simulation.
pub fn evaluate(handle: usize, input: &[u8]) -> Result<String, String> {
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
        let results: Vec<Result<SimulationResult, String>> = input
            .results
            .into_iter()
            .map(|result| match result {
                HostResult::Table(table) => Ok(table),
                HostResult::Failure { error } => Err(error),
            })
            .collect();
        let evaluation = plan.evaluate(&x, &results);
        let answer = EvaluationAnswer {
            unnorm_log_posterior: evaluation.unnorm_log_posterior(),
            simulation_table: plan.simulation_table(&evaluation),
            evaluation,
        };
        serde_json::to_string(&answer).map_err(|err| err.to_string())
    })
}

/// Drops an open problem; a handle of none is ignored.
pub fn close(handle: usize) {
    PROBLEMS.with(|problems| {
        if let Some(slot) = problems.borrow_mut().get_mut(handle) {
            *slot = None;
        }
    });
}

/// The bytes of a buffer of the caller.
///
/// # Safety
///
/// `pointer` is a buffer of [`crate::allocate`] of `len` bytes, or null.
unsafe fn bytes<'a>(pointer: *const u8, len: usize) -> &'a [u8] {
    if pointer.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller gives a buffer of `allocate` of exactly this length
        unsafe { std::slice::from_raw_parts(pointer, len) }
    }
}

/// Writes a text answer into the two words at `out`: 1 with the answer, 0 with the message.
///
/// # Safety
///
/// `out` points at two words of the caller.
unsafe fn answer(result: Result<String, String>, out: *mut usize) -> u32 {
    let (success, text) = match result {
        Ok(text) => (1, text),
        Err(message) => (0, message),
    };
    // SAFETY: the caller gives two words of its own memory
    unsafe { give(text.into_bytes(), out) };
    success
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
                *out.add(2) = 0;
                *out.add(3) = 0;
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

/// [`evaluate`] of the C ABI, with the two words of [`petab_open`] at `out`.
///
/// # Safety
///
/// As [`petab_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn petab_evaluate(
    handle: usize,
    pointer: *const u8,
    len: usize,
    out: *mut usize,
) -> u32 {
    // SAFETY: the buffers are the caller's, as documented
    unsafe { answer(evaluate(handle, bytes(pointer, len)), out) }
}

/// [`close`] of the C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn petab_close(handle: usize) {
    close(handle);
}

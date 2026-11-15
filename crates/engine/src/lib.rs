//! The engine of sbmlsim-core: integrates a compiled model with CVODE.
//!
//! A compiled model satisfies [`ModelFns`] and is described by a [`ModelDescription`]; a
//! [`SimulationRequest`] gives the times, the pre-initialization and the settings of the
//! integrator, and the answer is a [`SimulationResult`], a table of the time and the selections.
//!
//! - [`model`]: the functions of a compiled model and the test models.
//! - `cvode`: the wrapper of CVODE, created once per configuration and re-initialized per
//!   segment.
//! - `segments`: the integration of a segment and the rows of its output, without and with
//!   events.
//! - `events`: the triggers, the pending executions and the execution of the events at one
//!   time.
//! - `exports` (wasm32 and the feature `exports`): the exports of `engine.wasm` and the model it
//!   imports.
//!
//! - `changes`: the changes at times and how they are applied.
//! - `steady`: the steady state before a simulation.
//!
//! The initialization is `defaults`, then the pre-initialization values of the request and of
//! its steady state (fixed for `init`, in the selections of sbmlsim as the changes), then `init`
//! at the time 0 ([`INIT_TIME`], as sbmlsim); then the steady state, if the request has one
//! (integrated from the time 0); the integration runs in absolute time from the start, in
//! segments between the changes.

mod changes;
mod cvode;
mod events;
#[cfg(all(target_arch = "wasm32", feature = "exports"))]
mod exports;
pub mod model;
mod segments;
mod steady;

use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, MODEL_CONTRACT,
    ModelDescription, ModelSizes, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SparsityPattern, Tolerance, check_contract,
};

pub use cvode::KLU_THRESHOLD;

/// The most `f64` values a result may hold (rows times columns, 1 GiB).
///
/// An output of times or of uniform steps which exceeds it is refused before any allocation as
/// [`EngineError::Request`]; an output of the steps of the integrator which reaches it stops
/// with [`EngineError::Integration`] and the flag [`OUTPUT_LIMIT_FLAG`].
pub const MAX_OUTPUT_VALUES: usize = 1 << 27;

/// The flag of the [`EngineError::Integration`] of an output which exceeds
/// [`MAX_OUTPUT_VALUES`]: an engine code, distinct from the flags of CVODE (`-1` to `-99`).
pub const OUTPUT_LIMIT_FLAG: i32 = -1000;

/// The flag of the [`EngineError::Integration`] of a run of the integrator steps which stalls
/// (a step does not advance the time): an engine code, distinct from CVODE's and from
/// [`OUTPUT_LIMIT_FLAG`].
pub const STALL_FLAG: i32 = -1001;

/// The flag of the [`EngineError::Integration`] of an event which executes more than
/// [`MAX_EXECUTIONS_PER_EVENT`] times at one time (events which trigger each other in a cycle):
/// an engine code, distinct from CVODE's and the other engine flags.
pub const EVENT_LIMIT_FLAG: i32 = -1002;

/// The flag of the [`EngineError::Integration`] of an event whose delay is negative or NaN when
/// it is triggered: an engine code, distinct from CVODE's and the other engine flags.
pub const DELAY_FLAG: i32 = -1003;

/// The time of the initialization (`model_init`): 0, whatever the start of the simulation, as
/// sbmlsim initializes roadrunner at its time 0 (an initial assignment which reads `time` sees
/// 0 also for a simulation from 5 or -3, with and without a steady state).
pub const INIT_TIME: f64 = 0.0;

/// The flag of the [`EngineError::Integration`] of a steady state which is not reached by its
/// `max_time`: an engine code, distinct from CVODE's and the other engine flags.
pub const STEADY_STATE_FLAG: i32 = -1004;

pub use events::MAX_EXECUTIONS_PER_EVENT;
pub use model::ModelFns;

use changes::{Changes, Target};
use cvode::{Config, Cvode, Difference, Jacobian, Linear, Method, Problem};
use events::Events;
use segments::{Column, Recorder, SegmentOutput};

/// The error of a simulation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// The request or the description is invalid.
    #[error("invalid request: {0}")]
    Request(String),
    /// CVODE failed: its return flag, the time it reached and the message of its last error.
    #[error("integration failed at t = {time} (flag {flag}): {message}")]
    Integration {
        /// The return flag of CVODE.
        flag: i32,
        /// The time CVODE reached.
        time: f64,
        /// The message of the last error SUNDIALS reported.
        message: String,
    },
    /// The engine does not simulate this yet.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The engine failed in itself: a panic of `engine.wasm`, a trap of its instance (also one
    /// of the model it calls) or a failure of the host around it. The host does not use such an
    /// instance again; the native engine never gives this error.
    #[error("internal error of the engine: {0}")]
    Internal(String),
}

/// A loaded model with the CVODE memory of its last simulation, which the next simulation of
/// the same configuration (number of states, method, linear solver) re-initializes.
pub struct Engine {
    model: Box<dyn ModelFns>,
    desc: ModelDescription,
    cvode: Option<Cvode>,
}

impl Engine {
    /// Loads a model with its description; checks the contract and the pattern of the Jacobian.
    pub fn new(model: Box<dyn ModelFns>, desc: ModelDescription) -> Result<Engine, EngineError> {
        check_description(&desc)?;
        check_sizes(&*model, &desc)?;
        Ok(Engine {
            model,
            desc,
            cvode: None,
        })
    }

    /// The description of the model.
    pub fn description(&self) -> &ModelDescription {
        &self.desc
    }

    /// Simulates the model; the result does not depend on the simulations before.
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        run(&*self.model, &self.desc, &mut self.cvode, req)
    }
}

/// Simulates a model once, with a CVODE memory of its own.
pub fn simulate(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    req: &SimulationRequest,
) -> Result<SimulationResult, EngineError> {
    check_description(desc)?;
    check_sizes(model, desc)?;
    run(model, desc, &mut None, req)
}

/// Checks that the sizes of a model (`model_sizes`, `model_event_sizes`) are the ones of its
/// description, so every slice the engine passes, sized by the description, is as long as the
/// model reads and writes: a description of another model would make the model read and write
/// past its vectors.
fn check_sizes(model: &dyn ModelFns, desc: &ModelDescription) -> Result<(), EngineError> {
    let (sizes, described) = (model.sizes(), ModelSizes::of(desc));
    if sizes != described {
        return Err(request(format!(
            "the description of model {} is not the one of its model: the model has {sizes}, \
             the description {described}",
            desc.id
        )));
    }
    Ok(())
}

/// The error of an invalid request.
fn request(message: impl Into<String>) -> EngineError {
    EngineError::Request(message.into())
}

/// The pattern of the Jacobian of a description.
fn pattern_of(desc: &ModelDescription) -> &SparsityPattern {
    desc.jacobian.pattern()
}

/// Checks the contract of a description, the shape of the pattern of its Jacobian and, for a
/// difference quotient, that the columns of a color share no row (else the difference quotient
/// would mix the entries of two columns).
fn check_description(desc: &ModelDescription) -> Result<(), EngineError> {
    check_contract(&desc.contract, MODEL_CONTRACT).map_err(|e| request(e.to_string()))?;
    let n = desc.states.len();
    let pattern = pattern_of(desc);
    if !pattern.is_csc_of(n) {
        return Err(request(format!(
            "the Jacobian pattern of model {} is not a CSC pattern of {n} states",
            desc.id
        )));
    }
    if let JacobianKind::ColoredDifference { colors, .. } = &desc.jacobian {
        pattern.check_colors(colors).map_err(|e| {
            request(format!(
                "the colors of the Jacobian of model {} are no coloring of its pattern: {e}",
                desc.id
            ))
        })?;
    }
    // the selection indices are exactly 0..k, each once: the engine sizes the output of the
    // model by them and a gap or a repeat would write past it or leave a column unset
    let mut seen = vec![false; desc.selections.len()];
    for selection in &desc.selections {
        match seen.get_mut(selection.index as usize) {
            Some(slot) if !*slot => *slot = true,
            _ => {
                return Err(request(format!(
                    "the selection indices of model {} are not 0..{} without gaps or repeats \
                     (selection {} has index {})",
                    desc.id,
                    desc.selections.len(),
                    selection.id,
                    selection.index
                )));
            }
        }
    }
    Ok(())
}

/// The linear solver of a stiff simulation (a non-stiff one has none).
///
/// The Jacobian is the analytic one of a model which has one, else (and for a request of the
/// difference quotient) the sparse difference quotient over the colors of the pattern; the
/// request of the analytic Jacobian of a model without one is refused. Either is used with the
/// solver of the settings, `Auto` being KLU above [`KLU_THRESHOLD`] states.
fn linear_of(
    desc: &ModelDescription,
    settings: &IntegratorSettings,
) -> Result<Linear, EngineError> {
    let jacobian = match (&desc.jacobian, settings.jacobian) {
        (_, JacobianChoice::DifferenceQuotient) => Jacobian::Difference,
        (JacobianKind::ColoredDifference { reason, .. }, JacobianChoice::Analytic) => {
            return Err(request(format!(
                "model {} has no analytic Jacobian: {reason}",
                desc.id
            )));
        }
        (JacobianKind::ColoredDifference { .. }, JacobianChoice::Auto) => Jacobian::Difference,
        (JacobianKind::Analytic(_), _) => Jacobian::Analytic,
    };
    let klu = Linear::Klu {
        nnz: pattern_of(desc).nnz(),
        jacobian,
    };
    Ok(match settings.linear_solver {
        LinearSolver::Dense => Linear::Dense { jacobian },
        LinearSolver::Klu => klu,
        LinearSolver::Auto if desc.states.len() > KLU_THRESHOLD => klu,
        LinearSolver::Auto => Linear::Dense { jacobian },
    })
}

/// The sparse difference quotient of a configuration of `linear`, `None` for another source
/// of the Jacobian: over the colors of the description, or the greedy coloring of the pattern
/// of an analytic Jacobian.
fn difference_of(desc: &ModelDescription, linear: Linear) -> Option<Difference> {
    let (Linear::Dense {
        jacobian: Jacobian::Difference,
    }
    | Linear::Klu {
        jacobian: Jacobian::Difference,
        ..
    }) = linear
    else {
        return None;
    };
    let n = desc.states.len();
    Some(match &desc.jacobian {
        JacobianKind::ColoredDifference { colors, .. } => Difference::new(n, colors),
        JacobianKind::Analytic(pattern) => Difference::new(n, &pattern.greedy_colors()),
    })
}

/// Checks the settings of the integrator for a model of `n` states.
///
/// CVODE refuses negative tolerances; a tolerance of 0 is allowed (an absolute tolerance of 0
/// is the pure relative error), but a relative tolerance of 0 needs every absolute tolerance
/// above 0, else the error weights vanish where the state does and no step could be accepted.
fn check_settings(settings: &IntegratorSettings, n: usize) -> Result<(), EngineError> {
    let valid = |x: f64| x.is_finite() && x >= 0.0;
    let rtol = settings.relative_tolerance;
    if !valid(rtol) {
        return Err(request(format!("relative tolerance {rtol}")));
    }
    let atols: &[f64] = match &settings.absolute_tolerance {
        Tolerance::Scalar(atol) => std::slice::from_ref(atol),
        Tolerance::PerState(atol) => {
            if atol.len() != n {
                return Err(request(format!(
                    "absolute tolerances {atol:?} for {n} states"
                )));
            }
            atol
        }
    };
    if let Some(atol) = atols.iter().find(|a| !valid(**a)) {
        return Err(request(format!("absolute tolerance {atol}")));
    }
    if rtol == 0.0 && atols.contains(&0.0) {
        return Err(request(
            "relative tolerance 0 needs absolute tolerances above 0".to_string(),
        ));
    }
    for (name, value) in [
        ("maximum time step", settings.maximum_time_step),
        ("initial time step", settings.initial_time_step),
    ] {
        if !valid(value) {
            return Err(request(format!("{name} {value}")));
        }
    }
    if settings.maximum_num_steps == 0 {
        return Err(request("maximum number of steps 0"));
    }
    Ok(())
}

/// The times of an output of times or of uniform steps from `start` to `end`; the result of
/// `n_columns` columns must not exceed [`MAX_OUTPUT_VALUES`] (checked before any allocation).
fn output_times(
    output: &Output,
    start: f64,
    end: f64,
    n_columns: usize,
) -> Result<Vec<f64>, EngineError> {
    let too_large = |rows: u128| {
        request(format!(
            "an output of {rows} rows of {n_columns} columns exceeds the limit of \
             {MAX_OUTPUT_VALUES} values"
        ))
    };
    let exceeds = |rows: u128| rows * n_columns as u128 > MAX_OUTPUT_VALUES as u128;
    match output {
        Output::Times(times) => {
            if exceeds(times.len() as u128) {
                return Err(too_large(times.len() as u128));
            }
            let inside = times.iter().all(|t| *t >= start && *t <= end);
            let ordered = times.windows(2).all(|w| w[0] <= w[1]);
            if !inside || !ordered {
                return Err(request(format!(
                    "output times {times:?} are not non-decreasing within [{start}, {end}]"
                )));
            }
            Ok(times.clone())
        }
        Output::Uniform { steps: 0 } => Err(request("uniform output of 0 steps")),
        Output::Uniform { steps } => {
            let rows = *steps as u128 + 1;
            if exceeds(rows) {
                return Err(too_large(rows));
            }
            let steps = *steps as usize;
            let h = (end - start) / steps as f64;
            let mut times: Vec<f64> = (0..steps).map(|i| start + i as f64 * h).collect();
            times.push(end);
            Ok(times)
        }
        Output::IntegratorSteps => Ok(Vec::new()),
    }
}

/// The columns of a request: the time, then the requested selections (a leading `time` of the
/// request is the time column, not repeated), or the time and every selection of the model.
fn columns_of(
    desc: &ModelDescription,
    req: &SimulationRequest,
) -> Result<(Vec<String>, Vec<Column>), EngineError> {
    let selection = |id: &str| -> Result<Column, EngineError> {
        match desc.selections.iter().find(|s| s.id == id) {
            Some(s) => Ok(Column::Selection(s.index as usize)),
            None if id == "time" => Ok(Column::Time),
            None => Err(request(format!("model {} has no selection {id}", desc.id))),
        }
    };
    // the first column is always the time of the row (as sbmlsim's `output_columns`): a leading
    // `time` of the request is that column, else it is prepended; a later `time` is a copy of
    // it, unless the model has an entity `time` (SBML test case 01820), whose value it is
    let rest: Vec<String> = match req.selections.split_first() {
        None => desc.selections.iter().map(|s| s.id.clone()).collect(),
        Some((first, rest)) if first == "time" => rest.to_vec(),
        Some(_) => req.selections.clone(),
    };
    let columns = std::iter::once(Ok(Column::Time))
        .chain(rest.iter().map(|id| selection(id)))
        .collect::<Result<_, _>>()?;
    let names: Vec<String> = std::iter::once("time".to_string()).chain(rest).collect();
    Ok((names, columns))
}

/// Resolves the pre-initialization values, in the selections of sbmlsim as the changes (`S`
/// the amount of a species, `[S]` its concentration, see `changes`).
fn resolve_preinit(
    desc: &ModelDescription,
    values: &[(String, f64)],
) -> Result<Vec<(Target, f64)>, EngineError> {
    changes::resolve_values(desc, values, "in the pre-initialization")
}

/// Checks the steady state of a request.
fn check_steady_state(
    desc: &ModelDescription,
    steady: &sbmlsim_abi::SteadyState,
) -> Result<(), EngineError> {
    resolve_preinit(desc, &steady.preinit)?;
    let valid = |x: f64| x.is_finite() && x >= 0.0;
    if !valid(steady.absolute_tolerance) || !valid(steady.relative_tolerance) {
        return Err(request(format!(
            "steady state tolerances {} and {}",
            steady.absolute_tolerance, steady.relative_tolerance
        )));
    }
    if !(steady.max_time.is_finite() && steady.max_time > 0.0) {
        return Err(request(format!(
            "steady state maximum time {}",
            steady.max_time
        )));
    }
    Ok(())
}

/// The parameters and the state at the start: `defaults`, the pre-initialization values (fixed
/// for `init`), `init` at [`INIT_TIME`].
///
/// A value which needs the size of a compartment (the amount of a species which stores its
/// concentration, the concentration of a species which stores its amount) is converted with
/// the size of the model as loaded (`defaults` and `init`), or with the value an earlier
/// pre-initialization value of the request gave the compartment, as sbmlsim's `setValue` of
/// the selections on the reset model, in the order of the values.
fn initial_values(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    preinit: &[(Target, f64)],
) -> (Vec<f64>, Vec<f64>) {
    let (np, ns) = (desc.parameters.len(), desc.states.len());
    let mut p = vec![0.0; np];
    let mut y = vec![0.0; ns];
    model.defaults(&mut p, &mut y);
    let mut fixed = vec![0u8; np + ns];
    // the model as loaded, with the values set so far, for the sizes of the conversions
    let mut loaded = preinit
        .iter()
        .any(|(target, _)| target.compartment().is_some())
        .then(|| {
            let (mut lp, mut ly) = (p.clone(), y.clone());
            model.init(INIT_TIME, &mut lp, &mut ly, &fixed);
            (lp, ly)
        });
    let get = |p: &[f64], y: &[f64], i: usize| if i < np { p[i] } else { y[i - np] };
    let set = |p: &mut [f64], y: &mut [f64], i: usize, v: f64| {
        if i < np {
            p[i] = v;
        } else {
            y[i - np] = v;
        }
    };
    for &(target, value) in preinit {
        let stored = match (&loaded, target.compartment()) {
            (Some((lp, ly)), Some(c)) => target.stored(value, get(lp, ly, c)),
            _ => value,
        };
        let index = target.index();
        set(&mut p, &mut y, index, stored);
        if let Some((lp, ly)) = &mut loaded {
            set(lp, ly, index, stored);
        }
        fixed[index] = 1;
    }
    model.init(INIT_TIME, &mut p, &mut y, &fixed);
    (p, y)
}

/// The simulation [`simulate`] and [`Engine::simulate`] share; `cache` is the CVODE memory of
/// the last simulation, replaced when the configuration differs.
fn run(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    cache: &mut Option<Cvode>,
    req: &SimulationRequest,
) -> Result<SimulationResult, EngineError> {
    // 1. the request is validated before any feature is refused and before any integration
    check_contract(&req.contract, SIMULATION_CONTRACT).map_err(|e| request(e.to_string()))?;
    let (start, end) = (req.start, req.end);
    if !start.is_finite() || !end.is_finite() || end < start || !(end - start).is_finite() {
        return Err(request(format!(
            "simulation from {start} to {end}: the interval is not finite or is negative"
        )));
    }
    let n = desc.states.len();
    check_settings(&req.settings, n)?;
    let (names, columns) = columns_of(desc, req)?;
    let times = output_times(&req.output, start, end, names.len())?;
    let changes = Changes::new(desc, &req.changes, start, end)?;
    // the model is initialized once, with the pre-initialization values of the request and of
    // its steady state together (sbmlsim refuses a target in both)
    let mut preinit = req.preinit.clone();
    if let Some(steady) = &req.steady_state {
        check_steady_state(desc, steady)?;
        preinit.extend(steady.preinit.iter().cloned());
    }
    let preinit = resolve_preinit(desc, &preinit)?;
    // 2. the initialization
    let (p, mut y0) = initial_values(model, desc, &preinit);
    let with_events = !desc.events.is_empty() || desc.n_roots > 0;

    // a model without states but with events integrates one state of the rate 0, so CVODE
    // locates the zeros of its root functions
    let n_cvode = if with_events { n.max(1) } else { n };
    let mut pattern = None;
    let mut difference = None;
    let cvode = if n_cvode == 0 {
        None
    } else {
        let method = if req.settings.stiff {
            // the one state of the rate 0 of a model without states has no pattern
            let linear = if n == 0 {
                Linear::Dense {
                    jacobian: Jacobian::Internal,
                }
            } else {
                linear_of(desc, &req.settings)?
            };
            if n > 0 {
                pattern = Some(pattern_of(desc));
                difference = difference_of(desc, linear);
            }
            Method::Bdf(linear)
        } else {
            Method::Adams
        };
        let config = Config { n: n_cvode, method };
        if cache.as_ref().map(Cvode::config) != Some(config) {
            *cache = None;
            *cache = Some(Cvode::new(config)?);
        }
        cache.as_mut()
    };
    let settings = match (&req.settings.absolute_tolerance, n_cvode > n) {
        // the state of the rate 0 of a model without states gets the tolerance of no state
        (Tolerance::PerState(_), true) => IntegratorSettings {
            absolute_tolerance: Tolerance::Scalar(1.0),
            ..req.settings.clone()
        },
        _ => req.settings.clone(),
    };
    let mut problem = Problem {
        n: n_cvode,
        n_model: n,
        n_roots: desc.n_roots as usize,
        model,
        p,
        pattern,
        jac_values: vec![0.0; pattern.map_or(0, |p| p.row_idx.len())],
        difference,
        mem: std::ptr::null_mut(),
    };
    let n_selections = desc
        .selections
        .iter()
        .map(|s| s.index as usize + 1)
        .max()
        .unwrap_or(0);
    let mut recorder = Recorder::new(model, columns, n_selections, MAX_OUTPUT_VALUES);
    // the size of an output of times is known (and checked against the cap): one allocation,
    // not a growth by doubling, whose peak would be twice the result; the steps of the
    // integrator have no known count and grow by doubling up to the cap
    if !matches!(req.output, Output::IntegratorSteps) {
        recorder.reserve(times.len());
    }
    let output = match req.output {
        Output::IntegratorSteps => SegmentOutput::Steps,
        _ => SegmentOutput::Times(&times),
    };
    // the state of CVODE: the model reads its first `n` values
    y0.resize(n_cvode, 0.0);
    match cvode {
        Some(cvode) if with_events => {
            let mut events = Events::new(model, desc, req.seed, start, end);
            if let Some(steady) = &req.steady_state {
                steady::steady_state(
                    Some(&mut *cvode),
                    &mut problem,
                    &settings,
                    start,
                    end,
                    &mut y0,
                    steady,
                    Some(&mut events),
                    &desc.id,
                )?;
            }
            segments::integrate_events(
                cvode,
                &mut problem,
                &settings,
                start,
                &y0,
                end,
                output,
                &mut recorder,
                &mut events,
                &changes,
            )?;
        }
        mut cvode => {
            if let Some(steady) = &req.steady_state {
                steady::steady_state(
                    cvode.as_deref_mut(),
                    &mut problem,
                    &settings,
                    start,
                    end,
                    &mut y0,
                    steady,
                    None,
                    &desc.id,
                )?;
            }
            segments::integrate(
                cvode,
                &mut problem,
                &settings,
                start,
                &y0,
                end,
                output,
                &mut recorder,
                &changes,
            )?
        }
    }
    Ok(SimulationResult {
        columns: names,
        n_rows: recorder.n_rows,
        values: recorder.values,
    })
}

#[cfg(test)]
mod tests {
    use sbmlsim_abi::{Entity, EntityKind};

    use super::*;
    use crate::model::Decay;

    /// A description of `n` states with a diagonal Jacobian pattern.
    fn diagonal(n: usize) -> ModelDescription {
        let mut desc = Decay { k: 1.0 }.description();
        desc.states = (0..n)
            .map(|i| Entity {
                id: format!("y{i}"),
                kind: EntityKind::Parameter,
                initial: 0.0,
                settable: true,
            })
            .collect();
        desc.jacobian = JacobianKind::Analytic(SparsityPattern {
            n: n as u32,
            col_ptr: (0..=n as u32).collect(),
            row_idx: (0..n as u32).collect(),
        });
        desc
    }

    #[test]
    fn a_later_time_is_the_entity_time_of_a_model_which_has_one() {
        let req = |selections: &[&str]| SimulationRequest {
            contract: SIMULATION_CONTRACT.to_string(),
            start: 0.0,
            end: 1.0,
            output: Output::Uniform { steps: 1 },
            preinit: vec![],
            changes: vec![],
            steady_state: None,
            selections: selections.iter().map(|s| s.to_string()).collect(),
            settings: IntegratorSettings::default(),
            seed: 0,
        };
        let model = Decay { k: 0.5 };
        let mut desc = model.description();
        let columns = |desc: &ModelDescription, selections: &[&str]| {
            columns_of(desc, &req(selections)).unwrap().1
        };
        // without an entity time a later time is the time of the row
        assert!(matches!(
            columns(&desc, &["time", "y", "time"])[..],
            [Column::Time, Column::Selection(0), Column::Time]
        ));
        // with one, it is its value; the first column is still the time of the row
        desc.selections[1].id = "time".to_string();
        assert!(matches!(
            columns(&desc, &["time", "y", "time"])[..],
            [Column::Time, Column::Selection(0), Column::Selection(1)]
        ));
        let result = simulate(&model, &desc, &req(&["time", "time"])).unwrap();
        assert_eq!(result.columns, ["time", "time"]);
        assert_eq!(result.values, [0.0, 2f64.ln() / 0.5, 1.0, 2f64.ln() / 0.5]);
    }

    #[test]
    fn auto_chooses_klu_above_the_threshold() {
        let settings = IntegratorSettings::default();
        assert_eq!(settings.linear_solver, LinearSolver::Auto);
        let at = diagonal(KLU_THRESHOLD);
        let above = diagonal(KLU_THRESHOLD + 1);
        check_description(&at).unwrap();
        check_description(&above).unwrap();
        assert!(matches!(
            linear_of(&at, &settings),
            Ok(Linear::Dense {
                jacobian: Jacobian::Analytic
            })
        ));
        assert!(matches!(
            linear_of(&above, &settings),
            Ok(Linear::Klu { nnz, jacobian: Jacobian::Analytic }) if nnz == KLU_THRESHOLD + 1
        ));
        // an explicit choice wins over the size
        let dense = IntegratorSettings {
            linear_solver: LinearSolver::Dense,
            ..settings.clone()
        };
        assert!(matches!(
            linear_of(&above, &dense),
            Ok(Linear::Dense {
                jacobian: Jacobian::Analytic
            })
        ));
        let klu = IntegratorSettings {
            linear_solver: LinearSolver::Klu,
            ..settings
        };
        assert!(matches!(linear_of(&at, &klu), Ok(Linear::Klu { .. })));
    }

    #[test]
    fn every_jacobian_works_with_every_linear_solver() {
        // R4 ends: a model without an analytic Jacobian takes KLU as well, with the sparse
        // difference quotient
        let settings = IntegratorSettings::default();
        let colored = |n: usize| {
            let mut desc = diagonal(n);
            desc.jacobian = JacobianKind::ColoredDifference {
                pattern: pattern_of(&desc).clone(),
                colors: vec![0; n],
                reason: "factorial has no derivative (the rate of y0)".to_string(),
            };
            check_description(&desc).unwrap();
            desc
        };
        let difference = Jacobian::Difference;
        assert_eq!(
            linear_of(&colored(KLU_THRESHOLD), &settings).unwrap(),
            Linear::Dense {
                jacobian: difference
            }
        );
        assert_eq!(
            linear_of(&colored(KLU_THRESHOLD + 1), &settings).unwrap(),
            Linear::Klu {
                nnz: KLU_THRESHOLD + 1,
                jacobian: difference
            }
        );
        // R16: the difference quotient of an analytic model is the sparse difference quotient,
        // the analytic Jacobian of a model without one is refused with the reason
        let dq = IntegratorSettings {
            jacobian: JacobianChoice::DifferenceQuotient,
            ..settings.clone()
        };
        assert_eq!(
            linear_of(&diagonal(KLU_THRESHOLD + 1), &dq).unwrap(),
            Linear::Klu {
                nnz: KLU_THRESHOLD + 1,
                jacobian: difference
            }
        );
        let analytic = IntegratorSettings {
            jacobian: JacobianChoice::Analytic,
            ..settings
        };
        match linear_of(&colored(2), &analytic) {
            Err(EngineError::Request(m)) => {
                assert!(
                    m.contains("no analytic Jacobian") && m.contains("factorial"),
                    "{m}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn colors_which_share_a_row_are_refused() {
        // two columns of one color which share a row would mix their differences
        let mut desc = diagonal(2);
        desc.jacobian = JacobianKind::ColoredDifference {
            pattern: SparsityPattern {
                n: 2,
                col_ptr: vec![0, 1, 2],
                row_idx: vec![0, 0],
            },
            colors: vec![0, 0],
            reason: String::new(),
        };
        match check_description(&desc) {
            Err(EngineError::Request(m)) => {
                assert!(m.contains("share the row 0"), "{m}")
            }
            other => panic!("{other:?}"),
        }
        let JacobianKind::ColoredDifference { colors, .. } = &mut desc.jacobian else {
            unreachable!()
        };
        *colors = vec![0, 1];
        check_description(&desc).unwrap();
    }
}

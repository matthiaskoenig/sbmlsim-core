//! The engine of sbmlsim-core: integrates a compiled model with CVODE.
//!
//! A compiled model satisfies [`ModelFns`] and is described by a [`ModelDescription`]; a
//! [`SimulationRequest`] gives the times, the pre-initialization and the settings of the
//! integrator, and the answer is a [`SimulationResult`], a table of the time and the selections.
//!
//! - [`model`]: the functions of a compiled model and the test models.
//! - `cvode`: the wrapper of CVODE, created once per configuration and re-initialized per
//!   segment.
//! - `segments`: the integration of a segment and the rows of its output.
//!
//! The initialization is `defaults`, then the pre-initialization values (fixed for `init`), then
//! `init` at the start time; the integration runs in absolute time from the start. Events,
//! changes and steady states come with later tasks and are [`EngineError::Unsupported`] until
//! then.

mod cvode;
pub mod model;
mod segments;

use sbmlsim_abi::{
    Change, IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, MODEL_CONTRACT,
    ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
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
pub use model::ModelFns;

use cvode::{Config, Cvode, Linear, Method, Problem};
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
    run(model, desc, &mut None, req)
}

/// The error of an invalid request.
fn request(message: impl Into<String>) -> EngineError {
    EngineError::Request(message.into())
}

/// The pattern of the Jacobian of a description.
fn pattern_of(desc: &ModelDescription) -> &SparsityPattern {
    match &desc.jacobian {
        JacobianKind::Analytic(pattern) => pattern,
        JacobianKind::ColoredDifference { pattern, .. } => pattern,
    }
}

/// Checks the contract of a description and the shape of the pattern of its Jacobian.
fn check_description(desc: &ModelDescription) -> Result<(), EngineError> {
    check_contract(&desc.contract, MODEL_CONTRACT).map_err(|e| request(e.to_string()))?;
    let n = desc.states.len();
    let pattern = pattern_of(desc);
    let nnz = pattern.row_idx.len();
    let shaped = pattern.n as usize == n
        && pattern.col_ptr.len() == n + 1
        && pattern.col_ptr[0] == 0
        && pattern.col_ptr.windows(2).all(|w| w[0] <= w[1])
        && pattern.col_ptr[n] as usize == nnz
        && pattern.row_idx.iter().all(|i| (*i as usize) < n);
    if !shaped {
        return Err(request(format!(
            "the Jacobian pattern of model {} is not a CSC pattern of {n} states",
            desc.id
        )));
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
/// A model whose Jacobian is not analytic, or a request for the difference quotient, integrates
/// with the dense solver and the difference quotient of CVODE (the colored sparse difference
/// quotient comes later); an analytic Jacobian is used with the solver of the settings, `Auto`
/// being KLU above [`KLU_THRESHOLD`] states.
fn linear_of(
    desc: &ModelDescription,
    settings: &IntegratorSettings,
) -> Result<Linear, EngineError> {
    match (&desc.jacobian, settings.jacobian) {
        (_, JacobianChoice::DifferenceQuotient) => Ok(Linear::Dense { analytic: false }),
        (JacobianKind::ColoredDifference { .. }, JacobianChoice::Analytic) => Err(request(
            format!("model {} has no analytic Jacobian", desc.id),
        )),
        (JacobianKind::ColoredDifference { .. }, JacobianChoice::Auto) => {
            Ok(Linear::Dense { analytic: false })
        }
        (JacobianKind::Analytic(pattern), _) => {
            let klu = Linear::Klu {
                nnz: pattern.row_idx.len(),
            };
            Ok(match settings.linear_solver {
                LinearSolver::Dense => Linear::Dense { analytic: true },
                LinearSolver::Klu => klu,
                LinearSolver::Auto if desc.states.len() > KLU_THRESHOLD => klu,
                LinearSolver::Auto => Linear::Dense { analytic: true },
            })
        }
    }
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
        if id == "time" {
            return Ok(Column::Time);
        }
        desc.selections
            .iter()
            .find(|s| s.id == id)
            .map(|s| Column::Selection(s.index as usize))
            .ok_or_else(|| request(format!("model {} has no selection {id}", desc.id)))
    };
    // the first column is always the time of the row (as sbmlsim's `output_columns`): a leading
    // `time` of the request is that column, else it is prepended; a later `time` is a copy of it
    let rest: Vec<String> = match req.selections.split_first() {
        None => desc.selections.iter().map(|s| s.id.clone()).collect(),
        Some((first, rest)) if first == "time" => rest.to_vec(),
        Some(_) => req.selections.clone(),
    };
    let names: Vec<String> = std::iter::once("time".to_string()).chain(rest).collect();
    let columns = names
        .iter()
        .map(|id| selection(id))
        .collect::<Result<_, _>>()?;
    Ok((names, columns))
}

/// Resolves pre-initialization values to the entity position (the parameters, then the states)
/// and the value; an unknown id, an entity which is not settable, a repeated id and a value
/// which is not finite are [`EngineError::Request`] naming the id.
fn resolve_preinit(
    desc: &ModelDescription,
    values: &[(String, f64)],
) -> Result<Vec<(usize, f64)>, EngineError> {
    let mut resolved: Vec<(usize, f64)> = Vec::with_capacity(values.len());
    for (id, value) in values {
        let (index, entity) = desc
            .parameters
            .iter()
            .chain(&desc.states)
            .enumerate()
            .find(|(_, e)| e.id == *id)
            .ok_or_else(|| request(format!("model {} has no entity {id}", desc.id)))?;
        if !entity.settable {
            return Err(request(format!(
                "{id} of model {} is not settable",
                desc.id
            )));
        }
        if !value.is_finite() {
            return Err(request(format!("the value {value} of {id} is not finite")));
        }
        if resolved.iter().any(|(i, _)| *i == index) {
            return Err(request(format!("{id} is set more than once")));
        }
        resolved.push((index, *value));
    }
    Ok(resolved)
}

/// Checks the changes: finite values in `(start, end]` in non-decreasing order of the time.
fn check_changes(changes: &[Change], start: f64, end: f64) -> Result<(), EngineError> {
    let mut previous = start;
    for change in changes {
        let t = change.time;
        if !t.is_finite() || t <= start || t > end || t < previous {
            return Err(request(format!(
                "change at t = {t} is not after the start {start}, within the end {end} and in \
                 order of the time"
            )));
        }
        if let Some((id, value)) = change.values.iter().find(|(_, v)| !v.is_finite()) {
            return Err(request(format!(
                "the value {value} of {id} at t = {t} is not finite"
            )));
        }
        previous = t;
    }
    Ok(())
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
/// for `init`), `init`.
fn initial_values(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    preinit: &[(usize, f64)],
    start: f64,
) -> (Vec<f64>, Vec<f64>) {
    let (np, ns) = (desc.parameters.len(), desc.states.len());
    let mut p = vec![0.0; np];
    let mut y = vec![0.0; ns];
    model.defaults(&mut p, &mut y);
    let mut fixed = vec![0u8; np + ns];
    for &(index, value) in preinit {
        if index < np {
            p[index] = value;
        } else {
            y[index - np] = value;
        }
        fixed[index] = 1;
    }
    model.init(start, &mut p, &mut y, &fixed);
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
    if !start.is_finite() || !end.is_finite() || end < start {
        return Err(request(format!("simulation from {start} to {end}")));
    }
    let n = desc.states.len();
    check_settings(&req.settings, n)?;
    let (names, columns) = columns_of(desc, req)?;
    let times = output_times(&req.output, start, end, names.len())?;
    check_changes(&req.changes, start, end)?;
    if let Some(steady) = &req.steady_state {
        check_steady_state(desc, steady)?;
    }
    let preinit = resolve_preinit(desc, &req.preinit)?;
    // 2. the features of later tasks
    if !req.changes.is_empty() {
        return Err(EngineError::Unsupported("changes".into()));
    }
    if req.steady_state.is_some() {
        return Err(EngineError::Unsupported("steady states".into()));
    }
    if !desc.events.is_empty() || model.n_roots() > 0 {
        return Err(EngineError::Unsupported("events".into()));
    }
    let (p, y0) = initial_values(model, desc, &preinit, start);

    let mut pattern = None;
    let cvode = if n == 0 {
        None
    } else {
        let method = if req.settings.stiff {
            let linear = linear_of(desc, &req.settings)?;
            if !matches!(linear, Linear::Dense { analytic: false }) {
                pattern = Some(pattern_of(desc));
            }
            Method::Bdf(linear)
        } else {
            Method::Adams
        };
        let config = Config { n, method };
        if cache.as_ref().map(Cvode::config) != Some(config) {
            *cache = None;
            *cache = Some(Cvode::new(config)?);
        }
        cache.as_mut()
    };
    let mut problem = Problem {
        n,
        model,
        p: &p,
        pattern,
        jac_values: vec![0.0; pattern.map_or(0, |p| p.row_idx.len())],
    };
    let n_selections = desc
        .selections
        .iter()
        .map(|s| s.index as usize + 1)
        .max()
        .unwrap_or(0);
    let mut recorder = Recorder::new(model, columns, n_selections, MAX_OUTPUT_VALUES);
    let output = match req.output {
        Output::IntegratorSteps => SegmentOutput::Steps,
        _ => SegmentOutput::Times(&times),
    };
    segments::integrate(
        cvode,
        &mut problem,
        &req.settings,
        start,
        &y0,
        end,
        output,
        &mut recorder,
    )?;
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
    fn auto_chooses_klu_above_the_threshold() {
        let settings = IntegratorSettings::default();
        assert_eq!(settings.linear_solver, LinearSolver::Auto);
        let at = diagonal(KLU_THRESHOLD);
        let above = diagonal(KLU_THRESHOLD + 1);
        check_description(&at).unwrap();
        check_description(&above).unwrap();
        assert!(matches!(
            linear_of(&at, &settings),
            Ok(Linear::Dense { analytic: true })
        ));
        assert!(matches!(
            linear_of(&above, &settings),
            Ok(Linear::Klu { nnz }) if nnz == KLU_THRESHOLD + 1
        ));
        // an explicit choice wins over the size
        let dense = IntegratorSettings {
            linear_solver: LinearSolver::Dense,
            ..settings.clone()
        };
        assert!(matches!(
            linear_of(&above, &dense),
            Ok(Linear::Dense { analytic: true })
        ));
        let klu = IntegratorSettings {
            linear_solver: LinearSolver::Klu,
            ..settings
        };
        assert!(matches!(linear_of(&at, &klu), Ok(Linear::Klu { .. })));
    }
}

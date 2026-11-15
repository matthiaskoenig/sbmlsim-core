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
    IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, MODEL_CONTRACT,
    ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SparsityPattern, Tolerance, check_contract,
};

pub use cvode::KLU_THRESHOLD;
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
fn check_settings(settings: &IntegratorSettings, n: usize) -> Result<(), EngineError> {
    let valid = |x: f64| x.is_finite() && x >= 0.0;
    if !valid(settings.relative_tolerance) {
        return Err(request(format!(
            "relative tolerance {}",
            settings.relative_tolerance
        )));
    }
    match &settings.absolute_tolerance {
        Tolerance::Scalar(atol) if !valid(*atol) => {
            return Err(request(format!("absolute tolerance {atol}")));
        }
        Tolerance::PerState(atol) if atol.len() != n || !atol.iter().all(|a| valid(*a)) => {
            return Err(request(format!(
                "absolute tolerances {atol:?} for {n} states"
            )));
        }
        _ => {}
    }
    for (name, value) in [
        ("maximum time step", settings.maximum_time_step),
        ("initial time step", settings.initial_time_step),
    ] {
        if !valid(value) {
            return Err(request(format!("{name} {value}")));
        }
    }
    Ok(())
}

/// The times of an output of times or of uniform steps from `start` to `end`.
fn output_times(output: &Output, start: f64, end: f64) -> Result<Vec<f64>, EngineError> {
    match output {
        Output::Times(times) => {
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
            let steps = *steps as usize;
            let h = (end - start) / steps as f64;
            let mut times: Vec<f64> = (0..steps).map(|i| start + i as f64 * h).collect();
            times.push(end);
            Ok(times)
        }
        Output::IntegratorSteps => Ok(Vec::new()),
    }
}

/// The columns of a request: the requested ones, or the time and every selection of the model.
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
    let names: Vec<String> = if req.selections.is_empty() {
        std::iter::once("time".to_string())
            .chain(desc.selections.iter().map(|s| s.id.clone()))
            .collect()
    } else {
        req.selections.clone()
    };
    let columns = names
        .iter()
        .map(|id| selection(id))
        .collect::<Result<_, _>>()?;
    Ok((names, columns))
}

/// The parameters and the state at the start: `defaults`, the pre-initialization values (fixed
/// for `init`), `init`.
fn initial_values(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    req: &SimulationRequest,
) -> Result<(Vec<f64>, Vec<f64>), EngineError> {
    let (np, ns) = (desc.parameters.len(), desc.states.len());
    let mut p = vec![0.0; np];
    let mut y = vec![0.0; ns];
    model.defaults(&mut p, &mut y);
    let mut fixed = vec![0u8; np + ns];
    for (id, value) in &req.preinit {
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
        if index < np {
            p[index] = *value;
        } else {
            y[index - np] = *value;
        }
        fixed[index] = 1;
    }
    model.init(req.start, &mut p, &mut y, &fixed);
    Ok((p, y))
}

/// The simulation [`simulate`] and [`Engine::simulate`] share; `cache` is the CVODE memory of
/// the last simulation, replaced when the configuration differs.
fn run(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    cache: &mut Option<Cvode>,
    req: &SimulationRequest,
) -> Result<SimulationResult, EngineError> {
    check_contract(&req.contract, SIMULATION_CONTRACT).map_err(|e| request(e.to_string()))?;
    if !req.changes.is_empty() {
        return Err(EngineError::Unsupported("changes".into()));
    }
    if req.steady_state.is_some() {
        return Err(EngineError::Unsupported("steady states".into()));
    }
    if !desc.events.is_empty() || model.n_roots() > 0 {
        return Err(EngineError::Unsupported("events".into()));
    }
    let (start, end) = (req.start, req.end);
    if !start.is_finite() || !end.is_finite() || end < start {
        return Err(request(format!("simulation from {start} to {end}")));
    }
    let n = desc.states.len();
    check_settings(&req.settings, n)?;
    let times = output_times(&req.output, start, end)?;
    let (names, columns) = columns_of(desc, req)?;
    let (p, y0) = initial_values(model, desc, req)?;

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
    let mut recorder = Recorder::new(model, columns, n_selections);
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

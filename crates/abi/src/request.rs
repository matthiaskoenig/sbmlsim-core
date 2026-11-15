//! The request and the result of a simulation, contract `simulation/1`.

use serde::{Deserialize, Serialize};

/// A simulation of a compiled model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRequest {
    /// The contract, `SIMULATION_CONTRACT`.
    pub contract: String,
    /// The start time (negative times are allowed).
    pub start: f64,
    /// The end time.
    pub end: f64,
    /// The times of the output.
    pub output: Output,
    /// Values set before the initialization, as `(id, value)`; the ids are selections, as the
    /// ones of a [`Change`]: `S` is the amount of a species, `[S]` its concentration (converted
    /// with the size of its compartment as loaded, or as an earlier value of the
    /// pre-initialization sets it), any other id the value of its entity.
    pub preinit: Vec<(String, f64)>,
    /// Changes at absolute times in `[start, end]`, in non-decreasing order of the time.
    pub changes: Vec<Change>,
    /// A steady state to reach before the start.
    pub steady_state: Option<SteadyState>,
    /// The ids of the selections to write as columns, after the column `time`.
    ///
    /// The first column of a result is always `time`, the time of the row: a leading `"time"`
    /// here is that column (not repeated), otherwise it is prepended; a later `"time"` is another
    /// copy of the time of the row. Empty is `time` followed by every selection of the model.
    pub selections: Vec<String>,
    /// The settings of the integrator.
    pub settings: IntegratorSettings,
    /// The seed of the random generator which orders simultaneous events.
    pub seed: u64,
}

/// The times of the output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// The given times.
    Times(Vec<f64>),
    /// `steps` equal intervals from the start to the end.
    Uniform {
        /// The number of intervals.
        steps: u32,
    },
    /// The steps of the integrator.
    IntegratorSteps,
}

/// Values set at a time, as sbmlsim's `Change` (PEtab v2).
///
/// The values are set at once (the changes of one time are one change): a compartment keeps
/// the concentrations of the species of a concentration in it (not of an amount, not of an
/// assignment rule, not of one the change sets), and the assignment rules follow. The id of a
/// value is a selection: `S` is the amount of a species, `[S]` its concentration, any other id
/// the value of its entity. A change at the start is applied after the initialization (and
/// after the steady state) and before the output of the start; a change after the start ends a
/// segment of the integration, and the output has the state before it and the state after it
/// at its time (two rows: in the steps of the integrator always, in an output of times when
/// its time is one of them). The events a change triggers execute with it; an event at its
/// time executes after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    /// The absolute time of the change.
    pub time: f64,
    /// The `(id, value)` pairs, set simultaneously.
    pub values: Vec<(String, f64)>,
}

/// A steady state which is reached before the start, as sbmlsim's `SteadyState`.
///
/// The model is initialized once, with the pre-initialization of the request and of the steady
/// state together (an id in both is refused), and integrated from the time 0 (whatever the
/// start of the simulation, as sbmlsim), events active, until every state has
/// `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|` at the end of a horizon (1, 10,
/// 100, ... time units), at most `max_time`; the simulation starts from that state at its
/// start. A steady state which is not reached is an error of the integration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyState {
    /// Values set before the initialization, as `(id, value)`, as the pre-initialization of the
    /// request.
    pub preinit: Vec<(String, f64)>,
    /// The absolute tolerance of the steady state criterion.
    pub absolute_tolerance: f64,
    /// The relative tolerance of the steady state criterion.
    pub relative_tolerance: f64,
    /// The longest time integrated to reach it.
    pub max_time: f64,
}

impl Default for SteadyState {
    fn default() -> Self {
        Self {
            preinit: Vec::new(),
            absolute_tolerance: 1e-8,
            relative_tolerance: 1e-6,
            max_time: 1e8,
        }
    }
}

/// The absolute tolerance of the integrator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tolerance {
    /// One tolerance for every state.
    Scalar(f64),
    /// One tolerance per state, in the order of the state vector.
    PerState(Vec<f64>),
}

/// The linear solver of the integrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearSolver {
    /// Chosen from the model.
    Auto,
    /// Dense.
    Dense,
    /// Sparse (KLU).
    Klu,
}

/// How the integrator gets the Jacobian.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JacobianChoice {
    /// Chosen from the model.
    Auto,
    /// The analytic one of the model (refused for a model without one).
    Analytic,
    /// The sparse difference quotient over the colors of the columns of the pattern, also for a
    /// model with an analytic Jacobian.
    DifferenceQuotient,
}

/// The settings of the integrator, the defaults are the ones of roadrunner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegratorSettings {
    /// The relative tolerance.
    pub relative_tolerance: f64,
    /// The absolute tolerance.
    pub absolute_tolerance: Tolerance,
    /// The largest number of steps of one call.
    pub maximum_num_steps: u32,
    /// The largest step in time; `0.0` is not set (CVODE chooses).
    pub maximum_time_step: f64,
    /// The first step in time; `0.0` is not set (CVODE chooses).
    pub initial_time_step: f64,
    /// Stiff (BDF) or non-stiff (Adams).
    pub stiff: bool,
    /// The linear solver.
    pub linear_solver: LinearSolver,
    /// The source of the Jacobian.
    pub jacobian: JacobianChoice,
}

impl Default for IntegratorSettings {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-6,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            maximum_num_steps: 20000,
            maximum_time_step: 0.0,
            initial_time_step: 0.0,
            stiff: true,
            linear_solver: LinearSolver::Auto,
            jacobian: JacobianChoice::Auto,
        }
    }
}

/// The result of a simulation: a table in row major order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationResult {
    /// The names of the columns, `time` first.
    pub columns: Vec<String>,
    /// The number of rows.
    pub n_rows: usize,
    /// `n_rows * columns.len()` values, row major.
    pub values: Vec<f64>,
}

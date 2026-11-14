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
    /// Values set before the initialization, as `(id, value)`.
    pub preinit: Vec<(String, f64)>,
    /// Changes at absolute times after the start.
    pub changes: Vec<Change>,
    /// A steady state to reach before the start.
    pub steady_state: Option<SteadyState>,
    /// The ids of the selections to write as columns.
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

/// Values set at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    /// The absolute time of the change.
    pub time: f64,
    /// The `(id, value)` pairs, evaluated simultaneously.
    pub values: Vec<(String, f64)>,
}

/// A steady state which is reached before the start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyState {
    /// Values set before the steady state, as `(id, value)`.
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
    /// The analytic one of the model.
    Analytic,
    /// Difference quotients.
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

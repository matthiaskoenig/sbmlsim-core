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
    /// with the size its compartment has once the initial assignments are evaluated, so a
    /// pre-initialization holds the concentration of a species across the initialization, also
    /// when it changes that size, directly or through an initial assignment), any other id the
    /// value of its entity. The order of the values does not matter.
    pub preinit: Vec<(String, f64)>,
    /// Changes at absolute times in `[start, end]`, in non-decreasing order of the time.
    pub changes: Vec<Change>,
    /// A steady state to reach before the start.
    pub steady_state: Option<SteadyState>,
    /// A steady state to reach after the end, whose row the output carries last, at the time
    /// `inf`; none by default.
    ///
    /// The model continues from the state the timecourse ended in, with the triggers of its
    /// events as it left them, so an event which executed during the timecourse does not
    /// execute again at the same threshold. This is a measurement at steady state (PEtab v2's
    /// time `inf`), where [`SimulationRequest::steady_state`] is the pre-equilibration before
    /// the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steady_state_after: Option<SteadyStateAfter>,
    /// Whether the output at the end is the state **before** the events of the end time
    /// execute; `false` by default, which is the state after them.
    ///
    /// The caller which folds the value of a change needs the state the change is applied to,
    /// which is the one before the events of its time: it asks for the segment which ends
    /// there with this flag, as libroadrunner evaluates the triggers and applies a change
    /// before it executes the events of that time.
    ///
    /// Only the end is affected: an output time inside the interval sees the state after the
    /// events of its time, as without the flag, and the events of the end are simply not
    /// executed, the simulation ending there.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub before_events: bool,
    /// The ids of the selections to write as columns, after the column `time`.
    ///
    /// The first column of a result is always `time`, the time of the row: a leading `"time"`
    /// here is that column (not repeated), otherwise it is prepended; a later `"time"` is another
    /// copy of the time of the row. Empty is `time` followed by every selection of the model.
    pub selections: Vec<String>,
    /// The settings of the integrator.
    pub settings: IntegratorSettings,
    /// The seed of the random generator which orders simultaneous events; 1 when it is absent
    /// ([`DEFAULT_SEED`]).
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// The ids whose forward sensitivities the run computes, in the order of the parameters of
    /// [`Sensitivities`]; none of them when it is empty, which is the default.
    ///
    /// An id is a target of a pre-initialization, without the concentration form: a parameter
    /// of the model, the amount of a species, the size of a compartment. The sensitivity of a
    /// state is the one of its initial value.
    ///
    /// The forward sensitivities of a run are the derivatives of its columns in those values,
    /// which CVODES integrates along with the states: one run of `k` sensitivities answers
    /// what `k + 1` runs of a finite difference answer, and to the tolerance of the
    /// integration instead of the step of the difference.
    ///
    /// A run which cannot carry them is refused ([`crate::SimulationResult::sensitivities`]
    /// says which): a model of events, of algebraic rules or of delays, and a request of
    /// changes or of a steady state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitivities: Vec<String>,
}

/// The seed of a request without one.
pub const DEFAULT_SEED: u64 = 1;

/// [`DEFAULT_SEED`], for serde.
fn default_seed() -> u64 {
    DEFAULT_SEED
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
///
/// An increment adds its value to the value of its id in the state before the change (`S` the
/// amount, `[S]` the concentration, as a value reads them), simultaneously with the values: a
/// repeated dose (`PODOSE + d`), as sbmlsim's formula `"PODOSE + d"` of a change, which reads
/// the state before the change. An id is either set or incremented by one change.
///
/// A form is the general case of an increment: `constant + Σ factor * value(id)` over the
/// values before the change, set on its target as a value is, simultaneously with the values
/// and the increments (an increment is the form of its own id with the factor 1). It carries a
/// value which follows the state, e.g. the formula `S - 0.5 * R` of a change, which a number
/// would hold where the request was built. An id is either set, incremented or formed by one
/// change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    /// The absolute time of the change.
    pub time: f64,
    /// The `(id, value)` pairs, set simultaneously.
    pub values: Vec<(String, f64)>,
    /// The `(id, increment)` pairs, added to the values before the change; none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub increments: Vec<(String, f64)>,
    /// The forms, each evaluated from the state before the change; none by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<Form>,
}

/// A value of a change which the engine evaluates from its own state:
/// `constant + Σ factor * value(selection)`, read before the change.
///
/// The terms are summed after the constant in the order of the request, left to right, so a
/// result is reproducible from the request alone. An id may repeat in `terms` and its
/// contributions add; empty `terms` make the form a constant. The `target` must be settable
/// (and is set, incremented or formed once by a change), while a term need only be readable,
/// so it may name an entity an assignment rule computes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    /// The id of the entity the value is set on, a settable selection.
    pub target: String,
    /// The constant term.
    pub constant: f64,
    /// The `(id, factor)` terms over the values before the change.
    pub terms: Vec<(String, f64)>,
}

/// A steady state which is reached after the end of the simulation, as sbmlsim's measurement
/// at the time `inf`.
///
/// The model continues from the state the timecourse ended in, with the triggers of its events
/// as it left them and every pending execution moved to [`SteadyStateAfter::from`], and is
/// integrated from `from` over horizons which grow by a factor of ten, as the pre-equilibration
/// is, until every state has `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|` at the
/// end of a horizon, at most `max_time`. Its row is the last of the output, at the time `inf`.
///
/// A steady state which is not reached is an error of the integration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyStateAfter {
    /// The time the integration to the steady state starts at; 0 by default.
    ///
    /// libroadrunner continues at the time the timecourse ended at for a model whose math reads
    /// the time and at 0 for every other one, which sbmlsim passes here, so the two agree.
    #[serde(default)]
    pub from: f64,
    /// The absolute tolerance of the steady state criterion.
    pub absolute_tolerance: f64,
    /// The relative tolerance of the steady state criterion.
    pub relative_tolerance: f64,
    /// The longest time integrated to reach it.
    pub max_time: f64,
}

impl Default for SteadyStateAfter {
    fn default() -> Self {
        Self {
            from: 0.0,
            absolute_tolerance: 1e-8,
            relative_tolerance: 1e-6,
            max_time: 1e8,
        }
    }
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
    /// Matrix-free, preconditioned GMRES for ODE models.
    Gmres,
}

/// How the integrator gets the Jacobian for a direct solve or the GMRES preconditioner.
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
    /// The largest number of steps of one call of the integrator; in an output of times (and in a horizon
    /// of a steady state) the steps towards one output time, summed over the returns of the integrator at
    /// roots without an execution of an event and its restarts at the roots of discontinuities.
    pub maximum_num_steps: u32,
    /// The largest step in time; `0.0` is not set (the integrator chooses).
    pub maximum_time_step: f64,
    /// The first step in time; `0.0` is not set (the integrator chooses).
    pub initial_time_step: f64,
    /// CVODE BDF (`true`) or Adams (`false`); DAE models always use IDA BDF.
    pub stiff: bool,
    /// The linear solver.
    pub linear_solver: LinearSolver,
    /// The source of the Jacobian.
    pub jacobian: JacobianChoice,
    /// The largest order of the BDF, `1` to `5`; `0` is not set (CVODE and IDA take 5). It is
    /// the order of IDA as well, which integrates a DAE model with the BDF whatever `stiff`
    /// says.
    #[serde(default)]
    pub maximum_bdf_order: u8,
    /// The largest order of the Adams method, `1` to `12`; `0` is not set (CVODE takes 12).
    /// An ODE model of `stiff` false is integrated with it, a DAE model never.
    #[serde(default)]
    pub maximum_adams_order: u8,
    /// The smallest step in time; `0.0` is not set (the integrator chooses).
    #[serde(default)]
    pub minimum_time_step: f64,
    /// The largest number of error test failures of one step before the integration fails;
    /// `0` is not set (CVODE takes 7 and IDA 10, of which the engine sets 30 for a DAE
    /// model, whose consistent initial conditions cost failures of their own).
    #[serde(default)]
    pub maximum_error_test_failures: u32,
    /// The largest number of iterations of the nonlinear solver of one step; `0` is not set
    /// (CVODE and IDA take 3, 4 for the fixed point iteration of Adams).
    #[serde(default)]
    pub maximum_nonlinear_iterations: u32,
    /// The largest number of convergence failures of the nonlinear solver of one step;
    /// `0` is not set (CVODE takes 10 and IDA 10).
    #[serde(default)]
    pub maximum_convergence_failures: u32,
    /// The safety factor of the convergence test of the nonlinear solver; `0.0` is not set
    /// (CVODE takes 0.1 and IDA 0.33).
    #[serde(default)]
    pub nonlinear_convergence_coefficient: f64,
    /// Whether CVODE detects the stability limit of the BDF of an order above 2 and lowers the
    /// order, for a model of a dominant eigenvalue near the imaginary axis. It is a setting of
    /// the BDF, which `stiff` false and a DAE model have none of.
    #[serde(default)]
    pub stability_limit_detection: bool,
    /// Whether IDA leaves the algebraic states out of its error test, for a DAE model whose
    /// algebraic states are not of interest. It is a setting of IDA, which an ODE model has
    /// none of; the engine turns it on of its own at an algebraic jump, where the error of the
    /// algebraic states does not go to zero with the step.
    #[serde(default)]
    pub suppress_algebraic: bool,
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
            maximum_bdf_order: 0,
            maximum_adams_order: 0,
            minimum_time_step: 0.0,
            maximum_error_test_failures: 0,
            maximum_nonlinear_iterations: 0,
            maximum_convergence_failures: 0,
            nonlinear_convergence_coefficient: 0.0,
            stability_limit_detection: false,
            suppress_algebraic: false,
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
    /// The forward sensitivities of the columns, when the request asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivities: Option<Sensitivities>,
}

/// The forward sensitivities of the columns of a result.
///
/// The derivative of every column of every row in every parameter of the request, in the
/// order of the columns of the result and of the parameters of the request: the sensitivity
/// of the column `c` of the row `r` to the parameter `j` is
/// `values[(r * columns.len() + c) * parameters.len() + j]`. The column `time` is no function
/// of a parameter, so its sensitivities are 0.
///
/// The sensitivity of a column which is a state is the one CVODES integrated. The one of a
/// column which is not (a concentration, an assignment rule, a constant of the model) is the
/// derivative of its value along the sensitivity of the states and of the parameter, a
/// difference of one evaluation of the selections of the model, which is exact where the
/// column is a state and of the order of the square root of the rounding otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sensitivities {
    /// The ids of the parameters, the `sensitivities` of the request.
    pub parameters: Vec<String>,
    /// `n_rows * columns.len() * parameters.len()` values, row major.
    pub values: Vec<f64>,
}

/// The counters of the last simulation of an engine: diagnostics of the work of the
/// integrator, no part of a result (`engine_stats_ptr` of `engine.wasm`, `stats()` of the hosts).
///
/// The counters of CVODE or IDA are summed over every (re)initialization (including IDA
/// consistency projections) of the simulation (the
/// steady state, the segments between the changes and the events). The calls of the right hand
/// side or residual are counted where they are made: by the integrator (its nonlinear iteration, its error
/// estimates, its own difference quotient), by the sparse difference quotient, by the probes of
/// the triggers of the events and by the criterion of the steady state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationStats {
    /// The steps of the integrator.
    pub steps: u64,
    /// The calls of the right hand side or residual by the integrator.
    pub rhs_evals: u64,
    /// The calls of the Jacobian function by the integrator (the analytic Jacobian or the sparse
    /// difference quotient; CVODE's own difference quotient of a model without states is not
    /// counted).
    pub jacobian_evals: u64,
    /// The calls of the right hand side by the sparse difference quotient (one per color of a
    /// Jacobian, also of the fallback of the analytic one).
    pub jacobian_rhs_evals: u64,
    /// The calls of the analytic Jacobian with an entry which is not finite, which the sparse
    /// difference quotient replaced.
    pub jacobian_fallbacks: u64,
    /// The setups of the linear solver, including GMRES preconditioner setups.
    pub linear_setups: u64,
    /// The failures of the local error test.
    pub error_test_failures: u64,
    /// The iterations of the nonlinear solver.
    pub nonlinear_iterations: u64,
    /// The convergence failures of the nonlinear solver.
    pub nonlinear_convergence_failures: u64,
    /// The calls of the root functions by the integrator.
    pub root_evals: u64,
    /// The (re)initializations of the integrator and IDA consistency projections.
    pub integrations: u64,
    /// The calls of the right hand side by the probes of the triggers right after a time.
    pub probe_rhs_evals: u64,
    /// The calls of the right hand side by the criterion of the steady state.
    pub steady_state_rhs_evals: u64,
    /// The iterations of GMRES.
    #[serde(default)]
    pub linear_iterations: u64,
    /// The convergence failures of GMRES.
    #[serde(default)]
    pub linear_convergence_failures: u64,
    /// The Jacobian-vector products evaluated by CVODE for GMRES.
    #[serde(default)]
    pub jacobian_vector_evals: u64,
    /// The preconditioner evaluations that refresh the Jacobian approximation.
    #[serde(default)]
    pub preconditioner_evals: u64,
    /// The applications of the GMRES preconditioner.
    #[serde(default)]
    pub preconditioner_solves: u64,
    /// The calls of the right hand side of the forward sensitivities by CVODES, one per
    /// nonlinear iteration of the sensitivities of a step.
    #[serde(default)]
    pub sensitivity_rhs_evals: u64,
    /// The calls of the right hand side of the model by the difference quotient of `df/dp`,
    /// one per parameter of the sensitivities and call of their right hand side.
    #[serde(default)]
    pub sensitivity_model_rhs_evals: u64,
    /// The calls of `model_jac_p`, the analytic `df/dp` of a model which carries it: one per
    /// column of the chain of a parameter of the sensitivities and call of their right hand
    /// side, where the difference quotient of `df/dp` would take one right hand side.
    #[serde(default)]
    pub sensitivity_jac_p_evals: u64,
    /// The reductions of the order of the BDF by the detection of the stability limit, 0 where
    /// `stability_limit_detection` is off (`CVodeGetNumStabLimOrderReds`).
    #[serde(default)]
    pub stability_order_reductions: u64,
    /// The order of the last step of the integrator, 0 before the first one.
    #[serde(default)]
    pub last_order: u64,
    /// The order the integrator would take for the next step, 0 before the first one.
    #[serde(default)]
    pub current_order: u64,
    /// The first step the integrator actually took, which is `initial_time_step` where the
    /// request gave one.
    #[serde(default)]
    pub actual_initial_time_step: f64,
    /// The last step in time the integrator took.
    #[serde(default)]
    pub last_time_step: f64,
    /// The step in time the integrator would take for the next step.
    #[serde(default)]
    pub current_time_step: f64,
    /// The internal time of the integrator, which is at or behind the end of the simulation
    /// (the values of the end are interpolated).
    #[serde(default)]
    pub current_time: f64,
}

impl SimulationStats {
    /// The number of statistics, the `u64` words of `engine_stats_ptr`. The words of the
    /// counters are the counts; the words of the four times and steps, which are no counts,
    /// are the bits of their `f64` (`f64::to_bits`), so that one block carries both.
    pub const WORDS: usize = 28;

    /// The number of the leading words of [`SimulationStats::words`] which are counts. The
    /// `WORDS - COUNTS` words after them are the bits of an `f64`, the times and steps.
    pub const COUNTS: usize = 24;

    /// The names of the statistics in the order of [`SimulationStats::words`] (the fields).
    pub const NAMES: [&'static str; Self::WORDS] = [
        "steps",
        "rhs_evals",
        "jacobian_evals",
        "jacobian_rhs_evals",
        "jacobian_fallbacks",
        "linear_setups",
        "error_test_failures",
        "nonlinear_iterations",
        "nonlinear_convergence_failures",
        "root_evals",
        "integrations",
        "probe_rhs_evals",
        "steady_state_rhs_evals",
        "linear_iterations",
        "linear_convergence_failures",
        "jacobian_vector_evals",
        "preconditioner_evals",
        "preconditioner_solves",
        "sensitivity_rhs_evals",
        "sensitivity_model_rhs_evals",
        "sensitivity_jac_p_evals",
        "stability_order_reductions",
        "last_order",
        "current_order",
        "actual_initial_time_step",
        "last_time_step",
        "current_time_step",
        "current_time",
    ];

    /// The statistics in the order of the fields, the times and steps as the bits of their
    /// `f64`.
    pub fn words(&self) -> [u64; Self::WORDS] {
        [
            self.steps,
            self.rhs_evals,
            self.jacobian_evals,
            self.jacobian_rhs_evals,
            self.jacobian_fallbacks,
            self.linear_setups,
            self.error_test_failures,
            self.nonlinear_iterations,
            self.nonlinear_convergence_failures,
            self.root_evals,
            self.integrations,
            self.probe_rhs_evals,
            self.steady_state_rhs_evals,
            self.linear_iterations,
            self.linear_convergence_failures,
            self.jacobian_vector_evals,
            self.preconditioner_evals,
            self.preconditioner_solves,
            self.sensitivity_rhs_evals,
            self.sensitivity_model_rhs_evals,
            self.sensitivity_jac_p_evals,
            self.stability_order_reductions,
            self.last_order,
            self.current_order,
            self.actual_initial_time_step.to_bits(),
            self.last_time_step.to_bits(),
            self.current_time_step.to_bits(),
            self.current_time.to_bits(),
        ]
    }

    /// The statistics of [`SimulationStats::words`].
    pub fn from_words(w: [u64; Self::WORDS]) -> SimulationStats {
        SimulationStats {
            steps: w[0],
            rhs_evals: w[1],
            jacobian_evals: w[2],
            jacobian_rhs_evals: w[3],
            jacobian_fallbacks: w[4],
            linear_setups: w[5],
            error_test_failures: w[6],
            nonlinear_iterations: w[7],
            nonlinear_convergence_failures: w[8],
            root_evals: w[9],
            integrations: w[10],
            probe_rhs_evals: w[11],
            steady_state_rhs_evals: w[12],
            linear_iterations: w[13],
            linear_convergence_failures: w[14],
            jacobian_vector_evals: w[15],
            preconditioner_evals: w[16],
            preconditioner_solves: w[17],
            sensitivity_rhs_evals: w[18],
            sensitivity_model_rhs_evals: w[19],
            sensitivity_jac_p_evals: w[20],
            stability_order_reductions: w[21],
            last_order: w[22],
            current_order: w[23],
            actual_initial_time_step: f64::from_bits(w[24]),
            last_time_step: f64::from_bits(w[25]),
            current_time_step: f64::from_bits(w[26]),
            current_time: f64::from_bits(w[27]),
        }
    }

    /// Every call of the right hand side.
    pub fn all_rhs_evals(&self) -> u64 {
        self.rhs_evals
            + self.jacobian_rhs_evals
            + self.probe_rhs_evals
            + self.steady_state_rhs_evals
    }
}

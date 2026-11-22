//! The engine of sbmlsim-core integrates ODEs with CVODE and differential-algebraic systems with IDA.
//!
//! A compiled model satisfies [`ModelFns`] and is described by a [`ModelDescription`]; a
//! [`SimulationRequest`] gives the times, the pre-initialization and the settings of the
//! integrator, and the answer is a [`SimulationResult`], a table of the time and the selections.
//!
//! - [`model`]: the functions of a compiled model and the test models.
//! - `solver`: the SUNDIALS integrator, created once per configuration and re-initialized per
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
//! - `roots`: the guard of the restarts at the roots of the discontinuities of the math.
//!
//! The initialization is `defaults`, then the pre-initialization values of the request and of
//! its steady state (fixed for `init`, in the selections of sbmlsim as the changes), then `init`
//! at the time 0 ([`INIT_TIME`], as sbmlsim), the values and `init` in two passes when one of
//! them needs the size of a compartment (`initial_values`); then the steady state, if the
//! request has one
//! (integrated from the time 0); the integration runs in absolute time from the start, in
//! segments between the changes.
//!
//! # Known differences to libroadrunner
//!
//! Besides the known differences to sbmlsim's executors (`changes`, `events`), which are
//! libroadrunner's as well where sbmlsim keeps them:
//!
//! - The roots of the conditions of `piecewise` are located with continuous root functions of
//!   their operands (`a - b`, `min` and `max` for `and` and `or`), libroadrunner's with the
//!   booleans of the branches (`1` or `-1`); both stop where the selected branch changes. The
//!   kinetic laws with local parameters have them too, libroadrunner's none (a slip of its
//!   generator).
//! - The steps towards one output time are bounded by `maximum_num_steps` across the returns of
//!   CVODE at roots and its restarts at the roots of discontinuities (`roots`);
//!   libroadrunner counts the steps of one call of CVODE, which every return at a root ends,
//!   so a model whose roots chatter may run far longer there (the sliding mode
//!   `x' = piecewise(-1, x > 0, 1)` at rtol 1e-3, atol 1e-6: 28.6 million steps in
//!   libroadrunner, `CV_TOO_MUCH_WORK` within 20000 steps of one output interval here).
//! - The executions of events within the relative tolerance (as a distance in time) of the last
//!   applied root share the budget of steps towards one output time (`roots`), so
//!   events which accumulate end in `CV_TOO_MUCH_WORK`: the bouncing ball (`x' = v`,
//!   `v' = -10`, the event `x < 0`: `v = -0.5 v`, `x = 0`), whose bounces accumulate at
//!   t = 1.3416, fails there within `maximum_num_steps` steps; libroadrunner lets the ball fall
//!   through (a bounce it does not locate any more) and continues.
//! - The output of the steps of the integrator ends with `CV_TOO_MUCH_WORK` after
//!   `maximum_num_steps` consecutive stops (steps, roots, stop times) within the resolution of
//!   the time, 1e-9 times the magnitude of the times of the simulation (`roots`): the
//!   bouncing ball at its accumulation point, a sliding mode which creeps up to its switch
//!   (`x' = piecewise(-1, x > 0, 1)` at rtol 1e-6, atol 1e-12, at t = 1.0000000016).
//!   libroadrunner has no such bound (the ball falls through; the sliding mode runs to its
//!   `max_output_rows`). A chatter whose stops are further apart than the resolution is bounded
//!   by the limit of the output only.
//! - An event with a delay whose trigger a change of the request makes true (also through the
//!   events the change executes) is skipped, as sbmlsim skips it (`events`): it is not pending,
//!   assigns nothing, and its trigger is armed, so it executes again only after the trigger
//!   falls and rises again. SBML would schedule it.
//! - libroadrunner's count of roots close together (`strikes`) ignores the events of an ignored
//!   root as well; here it decides the restarts at the roots of discontinuities only, and the
//!   events execute whatever it counts.
//! - The output of the steps of the integrator has the two rows of a root (of an event, or a
//!   root of a discontinuity) at its exact time; libroadrunner writes them at `t - tol` and
//!   `t + tol` (`tol` its root tolerance, `100 * 32 * eps * (|t| + |h|)`), which sbmlsim moves
//!   to the time half way between them for a model with events.

mod changes;
mod delay;
mod events;
#[cfg(all(target_arch = "wasm32", feature = "exports"))]
mod exports;
mod initial;
pub mod model;
mod roots;
mod segments;
mod solver;
mod steady;

use sbmlsim_abi::{
    IntegratorSettings, JacobianChoice, JacobianKind, LinearSolver, MODEL_CONTRACT,
    ModelDescription, ModelSizes, Output, SIMULATION_CONTRACT, SimulationRequest, SimulationResult,
    SimulationStats, SparsityPattern, Tolerance, check_contract,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub use solver::KLU_THRESHOLD;

/// The most `f64` values a result may hold (rows times columns, 512 MiB).
///
/// An output of times or of uniform steps which exceeds it is refused before any allocation as
/// [`EngineError::Request`]; an output of the steps of the integrator which reaches it stops
/// with [`EngineError::Integration`] and the flag [`OUTPUT_LIMIT_FLAG`].
///
/// Why 512 MiB: the result is recorded in place of the output of `engine.wasm` (no second
/// copy), but the steps of the integrator grow it by doubling, and in the linear memory of
/// wasm32 the high water mark of that growth is about twice the result (the old block, the new
/// one, and the freed blocks below, which the allocator cannot reuse for the larger one;
/// measured: 1153 MiB of memory at a result of 576 MiB). The memory of an instance grows to at
/// most 2 GiB (the `maximum` of the hosts, `MAX_MEMORY_BYTES` of `sbmlsim-host` and
/// `js/loader.mjs`), so the largest result of 512 MiB leaves room for the model, CVODE and the
/// request (measured: 1153 MiB at the limit of the steps of the integrator, 683 MiB at the
/// largest output of times).
pub const MAX_OUTPUT_VALUES: usize = 1 << 26;

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

/// A delayed-event queue or assignment snapshot exceeded its resource policy.
pub const PENDING_EVENT_LIMIT_FLAG: i32 = -1005;

/// Bounds event storage independently of numerical step settings.
#[derive(Debug, Clone, Copy)]
pub struct EventLimits {
    /// Maximum pending executions, including delayed persistent events.
    pub pending_events: usize,
    /// Maximum retained assignment values, and maximum values of one execution.
    pub assignment_values: usize,
}

impl Default for EventLimits {
    fn default() -> Self {
        Self {
            pending_events: 65_536,
            assignment_values: 1 << 20,
        }
    }
}

pub use events::MAX_EXECUTIONS_PER_EVENT;
pub use model::ModelFns;

use changes::{Changes, Target};
use events::Events;
use segments::{Column, Recorder, SegmentOutput};
use solver::{Config, Counts, Difference, Integrator, Jacobian, Linear, Method, Problem};

pub use sbmlsim_abi::EngineError;

/// A loaded model with the integrator memory of its last simulation, which the next simulation of
/// the same configuration (number of states, method, linear solver) re-initializes.
pub struct Engine {
    model: Box<dyn ModelFns>,
    desc: ModelDescription,
    cache: SolverCache,
    stats: SimulationStats,
    metadata: Metadata,
    failed: bool,
    identity: Arc<()>,
    scratch: Scratch,
}

/// An immutable request resolved for one loaded engine. Clones share the plan.
#[derive(Clone)]
pub struct PreparedRequest {
    identity: Arc<()>,
    plan: Arc<Plan>,
}

impl PreparedRequest {
    /// Number of finite values required by each execution, in the varying ID order.
    pub fn n_values(&self) -> usize {
        self.plan.varying.len()
    }
}

struct Plan {
    req: RunSettings,
    names: Vec<String>,
    columns: Vec<Column>,
    times: Vec<f64>,
    changes: Changes,
    preinit: Vec<(Target, f64)>,
    varying: Vec<usize>,
    config: Option<Config>,
    mask: Vec<u8>,
    /// The targets of the forward sensitivities of the run, resolved, with their ids.
    sensitivities: Vec<(String, Target)>,
}

struct RunSettings {
    start: f64,
    end: f64,
    steps: bool,
    settings: IntegratorSettings,
    steady_state: Option<sbmlsim_abi::SteadyState>,
    steady_state_after: Option<sbmlsim_abi::SteadyStateAfter>,
    before_events: bool,
    seed: u64,
}

#[derive(Default)]
struct Scratch {
    p: Vec<f64>,
    y: Vec<f64>,
    fixed: Vec<u8>,
    jac_values: Vec<f64>,
    preinit: Vec<(Target, f64)>,
    selections: Vec<f64>,
    steady_rates: Vec<f64>,
    events: events::Scratch,
    segments: segments::Scratch,
}

/// Lookups and finite-difference workspace reused by simulations of this model.
struct Metadata {
    changes: changes::Metadata,
    selections: BTreeMap<String, usize>,
    difference: Option<Difference>,
    event_limits: EventLimits,
    /// The most values a result may hold, see [`Engine::with_max_output_values`].
    max_output_values: usize,
}

impl Metadata {
    fn new(desc: &ModelDescription) -> Self {
        let mut selections = BTreeMap::new();
        for selection in &desc.selections {
            selections
                .entry(selection.id.clone())
                .or_insert(selection.index as usize);
        }
        Self {
            changes: changes::Metadata::new(desc),
            selections,
            difference: None,
            event_limits: EventLimits::default(),
            max_output_values: MAX_OUTPUT_VALUES,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "model panicked without a message".to_string())
}

impl Engine {
    /// Loads a model with its description; checks the contract and the pattern of the Jacobian.
    pub fn new(model: Box<dyn ModelFns>, desc: ModelDescription) -> Result<Engine, EngineError> {
        check_description(&desc)?;
        check_sizes(&*model, &desc)?;
        Ok(Engine {
            metadata: Metadata::new(&desc),
            model,
            cache: SolverCache::new(&desc),
            desc,
            stats: SimulationStats::default(),
            failed: false,
            identity: Arc::new(()),
            scratch: Scratch::default(),
        })
    }

    /// The description of the model.
    pub fn description(&self) -> &ModelDescription {
        &self.desc
    }

    /// Initial selection values in selection-index order, before any event executes.
    pub fn initial_values(&mut self) -> Result<Vec<f64>, EngineError> {
        self.ensure_ready()?;
        let model = &*self.model;
        let mut initialize = || {
            if let SolverCache::Delayed(cache) = &mut self.cache {
                return cache.initial_values(model, &self.desc, &mut self.scratch);
            }
            initial_values(model, &self.desc, &[], &mut self.scratch)?;
            if self.desc.n_algebraic > 0 {
                initial::consistent(
                    model,
                    &self.desc,
                    &[],
                    &mut self.scratch,
                    &IntegratorSettings::default(),
                )?;
            }
            let n = self.desc.selections.len();
            let mut out = vec![0.0; n];
            model.selections(
                INIT_TIME,
                &self.scratch.y,
                &self.scratch.p,
                &vec![1; n],
                &mut out,
            );
            model.check_error()?;
            Ok(out)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut initialize))
            .unwrap_or_else(|error| Err(EngineError::Internal(panic_message(&*error))));
        #[cfg(target_arch = "wasm32")]
        let outcome = initialize();
        if matches!(outcome, Err(EngineError::Internal(_))) {
            self.failed = true;
        }
        outcome
    }

    /// Overrides this engine's event resource policy. Zero refuses storage; it does not
    /// mean unlimited. The WASM engine uses the bounded default policy.
    pub fn with_event_limits(mut self, limits: EventLimits) -> Self {
        self.metadata.event_limits = limits;
        self
    }

    /// Bounds the values of a result below [`MAX_OUTPUT_VALUES`], which is the bound of an
    /// engine which is not told another one.
    ///
    /// A result above the bound is the [`EngineError::Request`] of a result which does not fit
    /// into the memory of the engine, raised before the values are allocated. It is how a host
    /// which runs this engine natively gives a result the memory limit of an instance, which
    /// the memory of a guest gives an instance of `engine.wasm`
    /// ([`crate::MAX_OUTPUT_VALUES`] is the bound of the largest guest).
    pub fn with_max_output_values(mut self, values: usize) -> Self {
        self.metadata.max_output_values = values.min(MAX_OUTPUT_VALUES);
        self
    }

    /// Simulates the model; the result does not depend on the simulations before.
    pub fn simulate(&mut self, req: &SimulationRequest) -> Result<SimulationResult, EngineError> {
        self.simulate_with_prefix(req, &|_| 0)
            .map(Recorded::into_result)
    }

    /// The counters of the last simulation (also of one which failed; all 0 for one refused
    /// before its integration).
    pub fn stats(&self) -> SimulationStats {
        self.stats
    }

    /// [`Engine::simulate`] with `prefix(columns)` slots before the values of the result, which
    /// the caller fills (the header of the output of `engine.wasm`).
    #[cfg_attr(not(feature = "exports"), allow(dead_code))]
    pub(crate) fn simulate_with_prefix(
        &mut self,
        req: &SimulationRequest,
        prefix: &dyn Fn(&[String]) -> usize,
    ) -> Result<Recorded, EngineError> {
        self.stats = SimulationStats::default();
        self.ensure_ready()?;
        let plan = prepare(&self.desc, &self.metadata, req, &[])?;
        let rows = self.execute_plan(&plan, &[], prefix)?;
        Ok(rows.with_columns(plan.names))
    }

    /// Validates and resolves a request once for repeated executions on this engine.
    ///
    /// Each varying ID must name a distinct settable entity. It overrides an existing
    /// pre-initialization slot for that entity, preserving the slot's order and quantity,
    /// or appends a slot when absent. Every execution must supply one finite value per ID.
    /// The returned plan and its clones can only be executed by this engine.
    pub fn prepare(
        &self,
        request: &SimulationRequest,
        varying: &[String],
    ) -> Result<PreparedRequest, EngineError> {
        self.ensure_ready()?;
        Ok(PreparedRequest {
            identity: self.identity.clone(),
            plan: Arc::new(prepare(&self.desc, &self.metadata, request, varying)?),
        })
    }

    /// Executes a prepared request with values in the varying ID order.
    /// Each result owns its output independently of other executions.
    pub fn simulate_prepared(
        &mut self,
        prepared: &PreparedRequest,
        values: &[f64],
    ) -> Result<SimulationResult, EngineError> {
        self.simulate_prepared_with_prefix(prepared, values, &|_| 0)
            .map(Recorded::into_result)
    }

    #[cfg_attr(not(feature = "exports"), allow(dead_code))]
    pub(crate) fn simulate_prepared_with_prefix(
        &mut self,
        prepared: &PreparedRequest,
        values: &[f64],
        prefix: &dyn Fn(&[String]) -> usize,
    ) -> Result<Recorded, EngineError> {
        self.stats = SimulationStats::default();
        if !Arc::ptr_eq(&self.identity, &prepared.identity) {
            return Err(request("prepared request belongs to another engine"));
        }
        if values.len() != prepared.n_values() || values.iter().any(|value| !value.is_finite()) {
            return Err(request(format!(
                "prepared request requires {} finite values",
                prepared.n_values()
            )));
        }
        let rows = self.execute_plan(&prepared.plan, values, prefix)?;
        Ok(rows.with_columns(prepared.plan.names.clone()))
    }

    fn ensure_ready(&self) -> Result<(), EngineError> {
        if self.failed {
            return Err(EngineError::Internal(
                "the instance failed before and is not used again".into(),
            ));
        }
        Ok(())
    }

    fn execute_plan(
        &mut self,
        plan: &Plan,
        values: &[f64],
        prefix: &dyn Fn(&[String]) -> usize,
    ) -> Result<RecordedValues, EngineError> {
        self.ensure_ready()?;
        let mut execute = || {
            run(
                &*self.model,
                &self.desc,
                &mut self.metadata,
                &mut self.cache,
                plan,
                values,
                &mut self.scratch,
                prefix,
                &mut self.stats,
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut execute))
            .unwrap_or_else(|error| Err(EngineError::Internal(panic_message(&*error))));
        #[cfg(target_arch = "wasm32")]
        let outcome = execute();
        let outcome = self.model.check_error().and(outcome);
        if matches!(outcome, Err(EngineError::Internal(_))) {
            self.failed = true;
        }
        outcome
    }
}

/// A result with slots before its values ([`Engine::simulate_with_prefix`]).
pub(crate) struct Recorded {
    /// The names of the columns.
    pub columns: Vec<String>,
    /// The number of rows.
    pub n_rows: usize,
    /// The slots of the prefix, then the values of the rows, row major.
    pub values: Vec<f64>,
    /// The number of slots before the values.
    pub prefix: usize,
    /// The forward sensitivities of the columns, for a run which asked for them.
    pub sensitivities: Option<sbmlsim_abi::Sensitivities>,
}

/// Recorded rows before their caller transfers or clones the resolved column names.
struct RecordedValues {
    n_rows: usize,
    values: Vec<f64>,
    prefix: usize,
    sensitivities: Option<sbmlsim_abi::Sensitivities>,
}

impl RecordedValues {
    fn from_recorder(mut recorder: Recorder<'_>, prefix: usize) -> Self {
        Self {
            n_rows: recorder.n_rows,
            values: std::mem::take(&mut recorder.values),
            prefix,
            sensitivities: recorder.sensitivities.take().map(|sensitivities| {
                sbmlsim_abi::Sensitivities {
                    parameters: sensitivities.parameters,
                    values: sensitivities.values,
                }
            }),
        }
    }

    fn with_columns(self, columns: Vec<String>) -> Recorded {
        Recorded {
            columns,
            n_rows: self.n_rows,
            values: self.values,
            prefix: self.prefix,
            sensitivities: self.sensitivities,
        }
    }
}

impl Recorded {
    /// The result, without the prefix.
    fn into_result(self) -> SimulationResult {
        let mut values = self.values;
        if self.prefix > 0 {
            values.drain(..self.prefix);
        }
        SimulationResult {
            columns: self.columns,
            n_rows: self.n_rows,
            values,
            sensitivities: self.sensitivities,
        }
    }
}

/// Simulates a model once, with an integrator memory of its own.
pub fn simulate(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    req: &SimulationRequest,
) -> Result<SimulationResult, EngineError> {
    check_description(desc)?;
    check_sizes(model, desc)?;
    let mut stats = SimulationStats::default();
    let mut metadata = Metadata::new(desc);
    let plan = prepare(desc, &metadata, req, &[])?;
    let result = run(
        model,
        desc,
        &mut metadata,
        &mut SolverCache::new(desc),
        &plan,
        &[],
        &mut Scratch::default(),
        &|_| 0,
        &mut stats,
    );
    model
        .check_error()
        .and(result)
        .map(|rows| rows.with_columns(plan.names).into_result())
}

/// Checks that the sizes of a model (`model_sizes`, `model_event_sizes`) are the ones of its
/// description, so every slice the engine passes, sized by the description, is as long as the
/// model reads and writes: a description of another model would make the model read and write
/// past its vectors.
fn check_sizes(model: &dyn ModelFns, desc: &ModelDescription) -> Result<(), EngineError> {
    let (sizes, described) = (model.sizes(), ModelSizes::of(desc));
    model.check_error()?;
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
    if desc.n_delays > 0 && desc.n_constants != 1 {
        return Err(request("a delay model requires one reserved history slot"));
    }
    check_contract(&desc.contract, MODEL_CONTRACT).map_err(|e| request(e.to_string()))?;
    let n = desc.states.len();
    if desc.n_algebraic as usize > n {
        return Err(request("more algebraic equations than states"));
    }
    let pattern = pattern_of(desc);
    if !pattern.is_csc_of(n) {
        return Err(request(format!(
            "the Jacobian pattern of model {} is not a CSC pattern of {n} states",
            desc.id
        )));
    }
    if let Some(parameters) = &desc.jacobian_p
        && !parameters.is_csc_of(n, desc.parameters.len())
    {
        return Err(request(format!(
            "the pattern of the Jacobian of model {} in its parameters is not a CSC pattern \
             of {n} states and {} parameters",
            desc.id,
            desc.parameters.len()
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
    if desc.n_discontinuity_roots > desc.n_roots {
        return Err(request(format!(
            "model {} has {} root functions of discontinuities of {} root functions",
            desc.id, desc.n_discontinuity_roots, desc.n_roots
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
/// The Jacobian is the analytic one of a model which has one, else (and for a request of the
/// difference quotient) the sparse difference quotient over the colors of the pattern; the
/// request of the analytic Jacobian of a model without one is refused. Either is used with the
/// solver of the settings, `Auto` being KLU above [`KLU_THRESHOLD`] states.
fn linear_of(
    desc: &ModelDescription,
    settings: &IntegratorSettings,
) -> Result<Linear, EngineError> {
    if settings.linear_solver == LinearSolver::Gmres {
        gmres_applies(desc, settings)?;
    }
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
        LinearSolver::Gmres => Linear::Gmres {
            nnz: pattern_of(desc).nnz(),
            jacobian,
        },
        LinearSolver::Auto if desc.states.len() > KLU_THRESHOLD => klu,
        LinearSolver::Auto => Linear::Dense { jacobian },
    })
}

/// Refuses a request of GMRES for a model or a method which has no preconditioned Newton
/// iteration of CVODE BDF, naming the field of the request and what to set instead.
fn gmres_applies(
    desc: &ModelDescription,
    settings: &IntegratorSettings,
) -> Result<(), EngineError> {
    if desc.n_algebraic > 0 {
        return Err(request(format!(
            "settings.linear_solver \"gmres\" is a solver of CVODE, and model {} has {} \
             algebraic rule(s), which IDA integrates with a direct solver; set \
             settings.linear_solver \"auto\", \"dense\" or \"klu\"",
            desc.id, desc.n_algebraic
        )));
    }
    if !settings.stiff {
        return Err(request(
            "settings.linear_solver \"gmres\" needs the Newton iteration of CVODE BDF, which \
             settings.stiff false replaces with the fixed point iteration of Adams; set \
             settings.stiff true, or settings.linear_solver \"auto\"",
        ));
    }
    Ok(())
}

/// The sparse difference quotient of a stiff configuration with a pattern: the Jacobian of
/// [`Jacobian::Difference`], the fallback of [`Jacobian::Analytic`] for a call with an entry
/// which is not finite; over the colors of the description, or the greedy coloring of the
/// pattern of an analytic Jacobian.
fn difference_of(desc: &ModelDescription) -> Difference {
    let n = desc.states.len();
    match &desc.jacobian {
        JacobianKind::ColoredDifference { colors, .. } => Difference::new(n, colors),
        JacobianKind::Analytic(pattern) => Difference::new(n, &pattern.greedy_colors()),
    }
}

/// Checks the settings of the integrator for a model of `n` states.
///
/// CVODE refuses negative tolerances; a tolerance of 0 is allowed (an absolute tolerance of 0
/// is the pure relative error), but a relative tolerance of 0 needs every absolute tolerance
/// above 0, else the error weights vanish where the state does and no step could be accepted.
fn check_settings(settings: &IntegratorSettings, n: usize) -> Result<(), EngineError> {
    if settings.linear_solver == LinearSolver::Gmres && !settings.stiff {
        return Err(request(
            "settings.linear_solver \"gmres\" needs the Newton iteration of CVODE BDF, which \
             settings.stiff false replaces with the fixed point iteration of Adams; set \
             settings.stiff true, or settings.linear_solver \"auto\"",
        ));
    }
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
/// `n_columns` columns must not exceed `max_values`, the bound of the engine
/// ([`Engine::with_max_output_values`], [`MAX_OUTPUT_VALUES`] without another one), checked
/// before any allocation.
fn output_times(
    output: &Output,
    start: f64,
    end: f64,
    n_columns: usize,
    max_values: usize,
) -> Result<Vec<f64>, EngineError> {
    let too_large = |rows: u128| {
        request(format!(
            "an output of {rows} rows of {n_columns} columns exceeds the limit of \
             {max_values} values and does not fit into the memory of the engine"
        ))
    };
    let exceeds = |rows: u128| rows * n_columns as u128 > max_values as u128;
    let no_memory = |rows: usize| {
        request(format!(
            "{rows} output times do not fit into the memory of the engine"
        ))
    };
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
            let mut copy = Vec::new();
            copy.try_reserve_exact(times.len())
                .map_err(|_| no_memory(times.len()))?;
            copy.extend_from_slice(times);
            Ok(copy)
        }
        Output::Uniform { steps: 0 } => Err(request("uniform output of 0 steps")),
        Output::Uniform { steps } => {
            let rows = *steps as u128 + 1;
            if exceeds(rows) {
                return Err(too_large(rows));
            }
            let steps = *steps as usize;
            let h = (end - start) / steps as f64;
            let mut times = Vec::new();
            times
                .try_reserve_exact(steps + 1)
                .map_err(|_| no_memory(steps + 1))?;
            times.extend((0..steps).map(|i| start + i as f64 * h));
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
    selections: &BTreeMap<String, usize>,
) -> Result<(Vec<String>, Vec<Column>), EngineError> {
    let selection = |id: &str| -> Result<Column, EngineError> {
        match selections.get(id) {
            Some(&index) => Ok(Column::Selection(index)),
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
    metadata: &changes::Metadata,
    values: &[(String, f64)],
) -> Result<Vec<(Target, f64)>, EngineError> {
    changes::resolve_values(metadata, values, "in the pre-initialization")
}

/// Checks the steady state of a request.
fn check_steady_state(
    metadata: &changes::Metadata,
    steady: &sbmlsim_abi::SteadyState,
) -> Result<(), EngineError> {
    resolve_preinit(metadata, &steady.preinit)?;
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

/// The parameters and the state at the start: `defaults`, then the pre-initialization values
/// (fixed for `init`) and `init` at [`INIT_TIME`] in two passes, one pass when no value needs
/// the size of a compartment.
///
/// A value which needs the size of a compartment (the amount of a species which stores its
/// concentration, the concentration of a species which stores its amount) is converted with
/// the size the initial assignments settle on, so a pre-initialization holds the concentration
/// of a species over the whole initialization, as sbmlsim and libroadrunner do. The values are
/// therefore set in two passes: the ones which need no conversion first, then `init`, which
/// derives the sizes with them, then the converted ones, then `init` again, so whatever an
/// initial assignment derives from those species settles too. A conversion cannot change a
/// size (only a species is converted), so the result does not depend on the order of the
/// values of the request.
///
/// The size of a conversion is the one the **first** evaluation settles on. That is the size
/// of the initialized model unless an initial assignment of that size reads a species a value
/// converts, which is circular: the second evaluation then moves the size the first one
/// converted against, and there is no fixed point to take instead (libroadrunner's `setValue`
/// defines none either, it reads whichever size the model carries at the time).
fn initial_values(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    preinit: &[(Target, f64)],
    scratch: &mut Scratch,
) -> Result<(), EngineError> {
    let (np, ns) = (desc.parameters.len(), desc.states.len());
    // the parameters, then the constants of the model, which `init` computes
    let (p, y) = (&mut scratch.p, &mut scratch.y);
    p.clear();
    p.resize(np + desc.n_constants as usize, 0.0);
    y.clear();
    y.resize(ns, 0.0);
    model.defaults(p, y);
    model.check_error()?;
    let fixed = &mut scratch.fixed;
    fixed.clear();
    fixed.resize(np + ns, 0);
    let get = |p: &[f64], y: &[f64], i: usize| if i < np { p[i] } else { y[i - np] };
    let set = |p: &mut [f64], y: &mut [f64], i: usize, v: f64| {
        if i < np {
            p[i] = v;
        } else {
            y[i - np] = v;
        }
    };
    // a value which is no concentration is set before the initialization, so the
    // initial assignments see it and the sizes of the compartments afterwards are
    // the ones of the initialized model
    for &(target, value) in preinit {
        if target.compartment().is_none() {
            let index = target.index();
            set(p, y, index, value);
            fixed[index] = 1;
        }
    }
    model.init(INIT_TIME, p, y, fixed);
    model.check_error()?;
    // a concentration is converted with the size its compartment has once the
    // initial assignments are evaluated, so the pre-initialization holds the
    // concentration across the initialization, also when it changes that size,
    // directly or through an initial assignment, and the order of the values of
    // the request does not matter
    let mut converted = false;
    for &(target, value) in preinit {
        if let Some(c) = target.compartment() {
            let size = get(p, y, c);
            let index = target.index();
            set(p, y, index, target.stored(value, size));
            fixed[index] = 1;
            converted = true;
        }
    }
    if converted {
        model.init(INIT_TIME, p, y, fixed);
        model.check_error()?;
    }
    Ok(())
}

fn prepare(
    desc: &ModelDescription,
    metadata: &Metadata,
    req: &SimulationRequest,
    varying: &[String],
) -> Result<Plan, EngineError> {
    check_contract(&req.contract, SIMULATION_CONTRACT).map_err(|e| request(e.to_string()))?;
    let (start, end) = (req.start, req.end);
    if !start.is_finite() || !end.is_finite() || end < start || !(end - start).is_finite() {
        return Err(request(format!(
            "simulation from {start} to {end}: the interval is not finite or is negative"
        )));
    }
    let n = desc.states.len();
    check_settings(&req.settings, n)?;
    let (names, columns) = columns_of(desc, req, &metadata.selections)?;
    let times = output_times(
        &req.output,
        start,
        end,
        names.len(),
        metadata.max_output_values,
    )?;
    let changes = Changes::with_metadata(&metadata.changes, &req.changes, start, end)?;
    // the model is initialized once, with the pre-initialization values of the request and of
    // its steady state together (sbmlsim refuses a target in both)
    let mut preinit = req.preinit.clone();
    if let Some(steady) = &req.steady_state {
        check_steady_state(&metadata.changes, steady)?;
        preinit.extend(steady.preinit.iter().cloned());
    }
    let preinit = resolve_preinit(&metadata.changes, &preinit)?;
    let targets = resolve_preinit(
        &metadata.changes,
        &varying
            .iter()
            .map(|id| (id.clone(), 0.0))
            .collect::<Vec<_>>(),
    )?;
    let mut preinit = preinit;
    let mut slots = Vec::with_capacity(targets.len());
    for (target, _) in targets {
        // Match the entity, retaining the template's quantity and position.
        let slot = match preinit
            .iter()
            .position(|(existing, _)| existing.index() == target.index())
        {
            Some(slot) => slot,
            None => {
                preinit.push((target, 0.0));
                preinit.len() - 1
            }
        };
        slots.push(slot);
    }
    let sensitivities = sensitivities_of(desc, metadata, req)?;
    let with_events = !desc.events.is_empty() || desc.n_roots > 0;
    let n_cvode = if with_events { n.max(1) } else { n };
    let config = if n_cvode == 0 {
        None
    } else {
        Some(Config {
            n_sens: sensitivities.len(),
            n: n_cvode,
            method: if desc.n_algebraic > 0 {
                Method::Ida {
                    linear: linear_of(desc, &req.settings)?,
                    n_algebraic: desc.n_algebraic as usize,
                }
            } else if req.settings.stiff {
                Method::Bdf(if n == 0 {
                    Linear::Dense {
                        jacobian: Jacobian::Internal,
                    }
                } else {
                    linear_of(desc, &req.settings)?
                })
            } else {
                Method::Adams
            },
        })
    };
    let mut req = RunSettings {
        start,
        end,
        steps: matches!(req.output, Output::IntegratorSteps),
        settings: req.settings.clone(),
        steady_state: req
            .steady_state
            .as_ref()
            .map(|steady| sbmlsim_abi::SteadyState {
                preinit: Vec::new(),
                absolute_tolerance: steady.absolute_tolerance,
                relative_tolerance: steady.relative_tolerance,
                max_time: steady.max_time,
            }),
        steady_state_after: req.steady_state_after.clone(),
        before_events: req.before_events,
        seed: req.seed,
    };
    if n_cvode > n && matches!(req.settings.absolute_tolerance, Tolerance::PerState(_)) {
        req.settings.absolute_tolerance = Tolerance::Scalar(1.0);
    }
    let n_selections = desc
        .selections
        .iter()
        .map(|s| s.index as usize + 1)
        .max()
        .unwrap_or(0);
    let mut mask = vec![0; n_selections];
    for column in &columns {
        if let Column::Selection(index) = column {
            mask[*index] = 1;
        }
    }
    Ok(Plan {
        req,
        names,
        columns,
        times,
        changes,
        preinit,
        varying: slots,
        config,
        mask,
        sensitivities,
    })
}

/// The targets of the forward sensitivities of a request, resolved, and what a run which
/// cannot carry them is refused with.
///
/// The sensitivities of a run are the derivatives of its columns in the values of those
/// targets at the start of the run. What they do not carry yet is refused here and not
/// answered wrongly: a value which jumps during a run (a change, an event, a discontinuity of
/// the math), a state which an algebraic rule or a delay determines, a steady state before
/// the start, and the iterative linear solver, whose callbacks read the memory of CVODE as
/// their user data instead of the problem.
fn sensitivities_of(
    desc: &ModelDescription,
    metadata: &Metadata,
    req: &SimulationRequest,
) -> Result<Vec<(String, Target)>, EngineError> {
    if req.sensitivities.is_empty() {
        return Ok(Vec::new());
    }
    let refuse = |what: &str| {
        Err(EngineError::Unsupported(format!(
            "the forward sensitivities of a simulation of model {} do not carry {what}",
            desc.id
        )))
    };
    if !desc.events.is_empty() {
        return refuse("the events of a model");
    }
    if desc.n_roots > 0 {
        return refuse("the discontinuities of the math of a model");
    }
    if desc.n_algebraic > 0 {
        return refuse("the algebraic rules of a model");
    }
    if desc.n_delays > 0 {
        return refuse("the delays of a model");
    }
    if !req.changes.is_empty() {
        return refuse("the changes of a request");
    }
    if req.steady_state.is_some() {
        return refuse("the steady state of a request");
    }
    if req.settings.linear_solver == LinearSolver::Gmres {
        return refuse("the iterative linear solver");
    }
    let values: Vec<(String, f64)> = req
        .sensitivities
        .iter()
        .map(|id| (id.clone(), 0.0))
        .collect();
    let resolved = changes::resolve_values(&metadata.changes, &values, "in the sensitivities")?;
    Ok(req
        .sensitivities
        .iter()
        .cloned()
        .zip(resolved.into_iter().map(|(target, _)| target))
        .collect())
}

/// The value a target has in the parameters and the state of a model, which the step of its
/// difference is relative to: the value of its entity, converted with the size of its
/// compartment for an amount or a concentration of a species which stores the other one.
fn target_value(target: &Target, desc: &ModelDescription, p: &[f64], y: &[f64]) -> f64 {
    let n_params = desc.parameters.len();
    let value = |index: usize| {
        if index < n_params {
            p[index]
        } else {
            y[index - n_params]
        }
    };
    let stored = value(target.index());
    match target.compartment() {
        None => stored,
        Some(compartment) => target.read(stored, value(compartment)),
    }
}

/// The forward sensitivities of a run, before its integration: the initial values `dy0/dp` of
/// the sensitivities of the states, what their right hand side needs and the table the rows
/// are recorded into.
///
/// `dy0/dp` is a difference of the initialization of the model, one per target, with that
/// target fixed at a step above the value it has: a target which the initial assignments of
/// the model read moves the state at the start, a target which is a state moves it directly,
/// and a parameter which neither reads moves nothing of it. The step is the one the right
/// hand side of the sensitivities and the columns of the rows use as well, so what the
/// derivative of a run is, is one definition and not three.
fn initial_sensitivities(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    plan: &Plan,
    scratch: &mut Scratch,
) -> Result<(segments::Sensitivities, solver::Sensitivity), EngineError> {
    let n = desc.states.len();
    let n_p = scratch.p.len();
    let k = plan.sensitivities.len();
    let mut states = vec![0.0; k * n];
    let mut deltas = vec![0.0; k * n_p];
    let mut steps = Vec::with_capacity(k);
    let mut perturbed = Scratch::default();
    for (at, (_, target)) in plan.sensitivities.iter().enumerate() {
        let base = target_value(target, desc, &scratch.p, &scratch.y);
        // the square root of the rounding, relative to the value: the truncation of the
        // difference and its rounding are then both about 1e-8 of the derivative
        let step = f64::EPSILON.sqrt() * base.abs().max(1.0);
        let index = target.index();
        steps.push(step);
        let mut preinit = plan.preinit.clone();
        for (slot, _) in plan.varying.iter().zip(0..) {
            // the values of a prepared request are already in `scratch.preinit`
            preinit[*slot].1 = scratch.preinit[*slot].1;
        }
        match preinit
            .iter_mut()
            .find(|(existing, _)| existing.index() == index)
        {
            Some(slot) => slot.1 = base + step,
            None => preinit.push((*target, base + step)),
        }
        initial_values(model, desc, &preinit, &mut perturbed)?;
        model.check_error()?;
        for state in 0..n {
            states[at * n + state] = (perturbed.y[state] - scratch.y[state]) / step;
        }
        // what the step of the target moves the parameters by, the target itself and the
        // values the initial assignments of the model derive from it: the right hand side and
        // the selections read those and not the target
        for value in 0..n_p {
            deltas[at * n_p + value] = perturbed.p[value] - scratch.p[value];
        }
    }
    let n_selections = scratch.selections.len().max(
        desc.selections
            .iter()
            .map(|selection| selection.index as usize + 1)
            .max()
            .unwrap_or(0),
    );
    let rows = segments::Sensitivities::new(
        plan.sensitivities
            .iter()
            .map(|(id, _)| id.clone())
            .collect(),
        deltas.clone(),
        steps.clone(),
        states,
        n_p,
        n_selections,
    );
    Ok((
        rows,
        solver::Sensitivity::new(deltas, steps, desc.jacobian_p.as_ref(), n),
    ))
}

/// A loaded model keeps its solver family across requests. Matching the cache once
/// selects direct CVODE or IDA calls throughout the simulation.
enum SolverCache {
    Ode(Option<Integrator<false>>),
    Dae(Option<Integrator<true>>),
    Delayed(Box<delay::Cache>),
}

impl SolverCache {
    fn new(desc: &ModelDescription) -> Self {
        if desc.n_delays > 0 {
            Self::Delayed(Box::new(delay::Cache::new(desc)))
        } else if desc.n_algebraic == 0 {
            Self::Ode(None)
        } else {
            Self::Dae(None)
        }
    }
}

/// The simulation [`simulate`] and [`Engine::simulate`] share; `cache` is the integrator memory of
/// the last simulation, replaced when the configuration differs; the result has
/// `prefix(columns)` slots of 0 before its values; `stats` gets the counters of the simulation
/// (0 for a request refused before the integration).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn run(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    metadata: &mut Metadata,
    cache: &mut SolverCache,
    plan: &Plan,
    values: &[f64],
    scratch: &mut Scratch,
    prefix: &dyn Fn(&[String]) -> usize,
    stats: &mut SimulationStats,
) -> Result<RecordedValues, EngineError> {
    match cache {
        SolverCache::Ode(cache) => run_model(
            model,
            desc,
            metadata,
            cache,
            plan,
            values,
            scratch,
            prefix,
            stats,
            delay::NoHistory,
        ),
        SolverCache::Dae(cache) => run_model(
            model,
            desc,
            metadata,
            cache,
            plan,
            values,
            scratch,
            prefix,
            stats,
            delay::NoHistory,
        ),
        SolverCache::Delayed(cache) => {
            cache.run(model, desc, metadata, plan, values, scratch, prefix, stats)
        }
    }
}

fn record_point(
    model: &dyn ModelFns,
    plan: &Plan,
    scratch: &mut Scratch,
    max_output_values: usize,
    prefix: usize,
    sensitivities: Option<segments::Sensitivities>,
) -> Result<RecordedValues, EngineError> {
    if let Some(config) = &plan.config {
        solver::check_index_range(config)?;
    }
    let mut recorder = Recorder::with_scratch(
        model,
        &plan.columns,
        &plan.mask,
        &mut scratch.selections,
        max_output_values,
        prefix,
    )?;
    recorder.sensitivities = sensitivities;
    let times = if plan.req.steps {
        std::slice::from_ref(&plan.req.start)
    } else {
        recorder.reserve(plan.times.len())?;
        &plan.times
    };
    for &time in times {
        recorder.row(time, &scratch.y, &scratch.p)?;
    }
    Ok(RecordedValues::from_recorder(recorder, prefix))
}

#[allow(clippy::too_many_arguments)]
fn run_model<const DAE: bool, H: delay::Mode>(
    model: &dyn ModelFns,
    desc: &ModelDescription,
    metadata: &mut Metadata,
    cache: &mut Option<Integrator<DAE, H>>,
    plan: &Plan,
    values: &[f64],
    scratch: &mut Scratch,
    prefix: &dyn Fn(&[String]) -> usize,
    stats: &mut SimulationStats,
    history: H,
) -> Result<RecordedValues, EngineError> {
    *stats = SimulationStats::default();
    let req = &plan.req;
    let (start, end) = (req.start, req.end);
    let n = desc.states.len();
    let (names, columns, times, changes) = (&plan.names, &plan.columns, &plan.times, &plan.changes);
    scratch.preinit.clone_from(&plan.preinit);
    for (&slot, &value) in plan.varying.iter().zip(values) {
        scratch.preinit[slot].1 = value;
    }
    let preinit = std::mem::take(&mut scratch.preinit);
    let initialized = initial_values(model, desc, &preinit, scratch);
    let mut init_callbacks = Counts::default();
    let mut projection_counts = solver::SolverCounts::default();
    let initialized = initialized.and_then(|()| {
        if DAE {
            (init_callbacks, projection_counts) =
                initial::consistent(model, desc, &preinit, scratch, &req.settings)?;
        }
        Ok(())
    });
    scratch.preinit = preinit;
    initialized?;
    // the forward sensitivities of the run, whose initial values are a difference of the
    // initialization, before the parameters and the state are moved into the problem
    let sensitivities = if plan.sensitivities.is_empty() {
        None
    } else {
        Some(initial_sensitivities(model, desc, plan, scratch)?)
    };
    // 2. the initialization

    let with_events = !desc.events.is_empty() || desc.n_roots > 0;

    // At a single time without events, changes or a steady-state calculation, the
    // requested rows read only the initialized ODE state.
    if !DAE
        && start == end
        && !with_events
        && req.steady_state.is_none()
        && changes.time(0).is_none()
    {
        return record_point(
            model,
            plan,
            scratch,
            metadata.max_output_values,
            prefix(names),
            sensitivities.map(|(rows, _)| rows),
        );
    }

    // a model without states but with events integrates one state of the rate 0, so CVODE
    // locates the zeros of its root functions
    let n_cvode = if with_events { n.max(1) } else { n };
    let mut pattern = None;
    let mut difference = None;
    let mut analytic = false;
    let cvode = if n_cvode == 0 {
        None
    } else {
        let config = plan.config.expect("nonempty integration");
        if sensitivities.is_some() && n > 0 && matches!(config.method, Method::Adams) {
            // the right hand side of the sensitivities applies `df/dy` to them, which the
            // non-stiff method asks for nowhere else
            pattern = Some(pattern_of(desc));
            difference = Some(
                metadata
                    .difference
                    .get_or_insert_with(|| difference_of(desc)),
            );
            analytic = matches!(desc.jacobian, JacobianKind::Analytic(_))
                && req.settings.jacobian != JacobianChoice::DifferenceQuotient;
        }
        if let Method::Bdf(linear) | Method::Ida { linear, .. } = config.method
            && n > 0
        {
            pattern = Some(pattern_of(desc));
            difference = Some(
                metadata
                    .difference
                    .get_or_insert_with(|| difference_of(desc)),
            );
            analytic = matches!(
                linear,
                Linear::Dense {
                    jacobian: Jacobian::Analytic
                } | Linear::Klu {
                    jacobian: Jacobian::Analytic,
                    ..
                } | Linear::Gmres {
                    jacobian: Jacobian::Analytic,
                    ..
                }
            );
        }
        if cache.as_ref().map(Integrator::config) != Some(config) {
            *cache = None;
            *cache = Some(Integrator::new(config)?);
        }
        let cvode = cache.as_mut();
        if let Some(cvode) = cvode {
            cvode.history = history.clone();
            // the counters of the memory start with this simulation
            cvode.begin();
            Some(cvode)
        } else {
            None
        }
    };
    let settings = &req.settings;
    let prefix = prefix(names);
    let mut recorder = Recorder::with_scratch(
        model,
        columns,
        &plan.mask,
        &mut scratch.selections,
        metadata.max_output_values,
        prefix,
    )?;
    let (sensitivities, sensitivity) = match sensitivities {
        None => (None, None),
        Some((rows, sensitivity)) => (Some(rows), Some(sensitivity)),
    };
    recorder.sensitivities = sensitivities;
    if !req.steps {
        recorder.reserve(times.len())?;
    }

    scratch.jac_values.clear();
    scratch
        .jac_values
        .resize(pattern.map_or(0, |p| p.row_idx.len()), 0.0);
    let mut y0 = std::mem::take(&mut scratch.y);
    let mut problem = Problem {
        n: n_cvode,
        n_model: n,
        n_algebraic: desc.n_algebraic as usize,
        interpolation: if DAE && desc.n_discontinuity_roots > 0 {
            Some(Box::new(solver::Constraints::new(model, desc, settings)?))
        } else {
            None
        },
        n_roots: desc.n_roots as usize,
        n_discontinuity_roots: desc.n_discontinuity_roots as usize,
        model,
        p: std::mem::take(&mut scratch.p),
        pattern,
        jac_values: std::mem::take(&mut scratch.jac_values),
        jacobian_p: desc.jacobian_p.as_ref(),
        analytic,
        difference,
        mem: std::ptr::null_mut(),
        krylov: std::ptr::null_mut(),
        sensitivity,
        counts: init_callbacks,
        callback_error: None,
    };
    let output = if req.steps {
        SegmentOutput::Steps
    } else {
        SegmentOutput::Times(times)
    };
    // the state of CVODE: the model reads its first `n` values
    y0.resize(n_cvode, 0.0);
    let mut constraints = if DAE {
        Some(Box::new(solver::Constraints::new(model, desc, settings)?))
    } else {
        None
    };
    let mut probe_rhs = 0;
    let (outcome, mut cvode_counts) = match cvode {
        Some(cvode) if with_events => {
            let mut events = Events::with_scratch(
                model,
                desc,
                req.seed,
                start,
                end,
                metadata.event_limits,
                &mut scratch.events,
            );
            events.constraints = constraints.take();
            let mut integrate = || {
                if let Some(steady) = &req.steady_state {
                    steady::steady_state(
                        Some(&mut *cvode),
                        &mut problem,
                        settings,
                        start,
                        end,
                        &mut y0,
                        steady,
                        Some(&mut events),
                        &desc.id,
                        &mut scratch.steady_rates,
                        None,
                    )?;
                    if H::ENABLED {
                        let mut state = history.shared().unwrap().borrow_mut();
                        state.rebase(start);
                        state.sync(model, &mut problem.p)?;
                    }
                }
                if DAE && let Some(projection) = &mut events.constraints {
                    projection.project(start, &mut y0, &problem.p)?;
                }
                segments::integrate_events(
                    cvode,
                    &mut problem,
                    settings,
                    start,
                    &mut y0,
                    end,
                    output,
                    &mut recorder,
                    &mut events,
                    changes,
                    &mut scratch.segments,
                    req.before_events,
                )?;
                // the steady state after the end continues from the state the timecourse
                // ended in, with the triggers of the events as it left them
                if let Some(after) = &req.steady_state_after {
                    steady::steady_state_after(
                        Some(&mut *cvode),
                        &mut problem,
                        settings,
                        end,
                        &mut y0,
                        after,
                        Some(&mut events),
                        &desc.id,
                        &mut scratch.steady_rates,
                        None,
                    )?;
                    recorder.row(f64::INFINITY, &y0[..problem.n_model], &problem.p)?;
                }
                Ok(())
            };
            let outcome = integrate();
            probe_rhs = events.probe_rhs();
            constraints = events.constraints.take();
            (outcome, cvode.counts())
        }
        mut cvode => {
            let mut integrate = || {
                if let Some(steady) = &req.steady_state {
                    steady::steady_state(
                        cvode.as_deref_mut(),
                        &mut problem,
                        settings,
                        start,
                        end,
                        &mut y0,
                        steady,
                        None,
                        &desc.id,
                        &mut scratch.steady_rates,
                        constraints.as_deref_mut(),
                    )?;
                    if H::ENABLED {
                        let mut state = history.shared().unwrap().borrow_mut();
                        state.rebase(start);
                        state.sync(model, &mut problem.p)?;
                    }
                }
                if DAE && let Some(projection) = &mut constraints {
                    projection.project(start, &mut y0, &problem.p)?;
                }
                segments::integrate(
                    cvode.as_deref_mut(),
                    &mut problem,
                    settings,
                    start,
                    &mut y0,
                    end,
                    output,
                    &mut recorder,
                    changes,
                    constraints.as_deref_mut(),
                )?;
                // a model without events has no trigger to carry, the steady state after the
                // end simply continues from the state the timecourse ended in
                if let Some(after) = &req.steady_state_after {
                    steady::steady_state_after(
                        cvode.as_deref_mut(),
                        &mut problem,
                        settings,
                        end,
                        &mut y0,
                        after,
                        None,
                        &desc.id,
                        &mut scratch.steady_rates,
                        constraints.as_deref_mut(),
                    )?;
                    recorder.row(f64::INFINITY, &y0[..problem.n_model], &problem.p)?;
                }
                Ok(())
            };
            let outcome = integrate();
            let counts = cvode.as_deref().map(Integrator::counts).unwrap_or_default();
            (outcome, counts)
        }
    };
    if DAE {
        for projection in [constraints.as_ref(), problem.interpolation.as_ref()]
            .into_iter()
            .flatten()
        {
            let (callbacks, counts) = projection.counts();
            problem.counts = problem.counts.add(callbacks);
            projection_counts = projection_counts.add(counts);
        }
        cvode_counts = cvode_counts.add(projection_counts);
    }
    *stats = solver::stats(&problem.counts, &cvode_counts, probe_rhs);
    scratch.p = problem.p;
    scratch.jac_values = problem.jac_values;
    scratch.y = y0;
    outcome?;
    Ok(RecordedValues::from_recorder(recorder, prefix))
}

#[cfg(test)]
mod tests {
    use sbmlsim_abi::{Entity, EntityKind};

    use super::*;
    use crate::model::Decay;

    #[test]
    fn repeated_runs_retain_the_initialization_and_recording_workspaces() {
        let model = crate::model::Compartmental { k: 0.5 };
        let mut engine = Engine::new(Box::new(model), model.description()).unwrap();
        let req = SimulationRequest {
            contract: SIMULATION_CONTRACT.into(),
            start: 0.0,
            end: 1.0,
            output: Output::Uniform { steps: 4 },
            preinit: vec![("[S]".into(), 2.0)],
            changes: vec![],
            steady_state: None,
            steady_state_after: None,
            before_events: false,
            selections: vec![],
            settings: IntegratorSettings::default(),
            seed: 0,
            sensitivities: vec![],
        };
        let plan = engine.prepare(&req, &["[S]".into()]).unwrap();
        engine.simulate_prepared(&plan, &[2.0]).unwrap();
        let pointers = |scratch: &Scratch| {
            (
                scratch.p.as_ptr(),
                scratch.y.as_ptr(),
                scratch.fixed.as_ptr(),
                scratch.jac_values.as_ptr(),
                scratch.selections.as_ptr(),
                scratch.preinit.as_ptr(),
            )
        };
        let before = pointers(&engine.scratch);
        engine.simulate_prepared(&plan, &[5.0]).unwrap();
        assert_eq!(before, pointers(&engine.scratch));
        assert!(engine.simulate_prepared(&plan, &[f64::NAN]).is_err());
        engine.simulate_prepared(&plan, &[2.0]).unwrap();
        assert_eq!(before, pointers(&engine.scratch));
    }

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
            steady_state_after: None,
            before_events: false,
            selections: selections.iter().map(|s| s.to_string()).collect(),
            settings: IntegratorSettings::default(),
            seed: 0,
            sensitivities: vec![],
        };
        let model = Decay { k: 0.5 };
        let mut desc = model.description();
        let columns = |desc: &ModelDescription, selections: &[&str]| {
            columns_of(desc, &req(selections), &Metadata::new(desc).selections)
                .unwrap()
                .1
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
        // a model without an analytic Jacobian takes KLU as well, with the sparse
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
        // the difference quotient of an analytic model is the sparse difference quotient,
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

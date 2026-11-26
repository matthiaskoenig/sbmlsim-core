//! [`Stepper`]: an instance of the engine which is advanced from a given state, one interval at
//! a time.
//!
//! A simulation integrates a model from its initial state over the whole time of a request. A
//! caller which couples the model to something else (operator splitting: the transport of a
//! species between many instances of a cell model) advances it over one interval at a time
//! instead, and changes its state between two intervals. The stepper is that instance: the
//! model is initialized once, as a simulation initializes it, its integrator memory is created
//! once and started again for every interval, from the state the caller gives.
//!
//! An advance integrates as a simulation does ([`segments::integrate_events`] for a model with
//! events or discontinuities, [`segments::integrate`] without), with the events and the
//! discontinuities of the interval as a simulation handles them, and the state of the events
//! carries from one interval to the next ([`Events::resume`]): the triggers keep their values,
//! a pending execution (of an event with a delay) stays pending across the end of an interval,
//! and an event whose trigger the changed state makes true at the start of an interval executes
//! there. The parameters an event assigns stay assigned. The state after an advance is the state
//! after the events at its end, as the last row of a simulation is.

use std::sync::Arc;

use sbmlsim_abi::{DEFAULT_SEED, IntegratorSettings, ModelDescription};

use crate::changes::{self, Changes};
use crate::events::{self, Events};
use crate::model::ModelFns;
use crate::segments::{self, Recorder, SegmentOutput};
use crate::solver::{ConstraintWorkspace, Constraints, Difference, Integrator, Problem};
use crate::{
    EngineError, EventLimits, INIT_TIME, Scratch, check_description, check_settings, check_sizes,
    configure, initial, initial_values, jacobian_of, n_integrated, request, resolve_preinit,
};

/// The counters of the integrator of one [`Stepper::advance`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StepStats {
    /// The steps the integrator took.
    pub steps: u64,
    /// The attempts of a step the integrator rejected: the failures of its local error test
    /// and of the convergence of its nonlinear solver, after each of which it tries the step
    /// again with a smaller size.
    pub rejected: u64,
}

/// The integrator of a stepper, of the family of its model.
enum Solver {
    /// A model without states, events and discontinuities: there is nothing to integrate.
    None,
    Ode(Integrator<false>),
    /// IDA for a model of algebraic states, with the memories of the projection of a state on
    /// the algebraic equations and of the interpolation across a discontinuity
    /// ([`Constraints`]), which the stepper keeps from one advance to the next as a simulation
    /// keeps them from one run to the next.
    Dae(Integrator<true>, Box<Projections>),
}

#[derive(Default)]
struct Projections {
    projection: Option<ConstraintWorkspace>,
    interpolation: Option<ConstraintWorkspace>,
}

/// An instance of a model which is advanced from a given state over one interval at a time,
/// see the module documentation.
///
/// Its step control is its own and depends on no other instance: every advance starts the
/// integrator with the first step of a simulation, the estimate of CVODE (or IDA), or the
/// initial step of the settings. (Starting with the step the advance before ended with was
/// measured worse: a step of a higher order fails the error test of the first order, at which
/// the integrator starts again, so it doubled the rejected steps and saved no time.)
pub struct Stepper {
    model: Box<dyn ModelFns>,
    desc: Arc<ModelDescription>,
    /// The settings of the integration, as a simulation applies them ([`configure`]).
    settings: IntegratorSettings,
    solver: Solver,
    /// The parameters and the constants behind them, which the events change.
    p: Vec<f64>,
    /// The state after [`Stepper::new`], then after the last advance.
    y: Vec<f64>,
    /// The state of the integrator during an advance ([`n_integrated`] values).
    work: Vec<f64>,
    jac_values: Vec<f64>,
    difference: Option<Difference>,
    /// The state of the events which one advance leaves to the next one.
    events: events::Scratch,
    segments: segments::Scratch,
    /// The selections of the recorder, which records none (empty).
    selections: Vec<f64>,
    /// No changes: the caller changes the state between two advances.
    changes: Changes,
    limits: EventLimits,
    failed: bool,
}

impl Stepper {
    /// Loads a model with its description, initializes it and creates its integrator memory.
    ///
    /// The initial state is the one of a simulation: the defaults of the model, the `changes`
    /// (as the pre-initialization values of a request: `S` the amount of a species, `[S]` its
    /// concentration, any other id the value of its entity, fixed while the initial
    /// assignments are evaluated) and the initial assignments, then the consistent algebraic
    /// states of a model of algebraic rules. The linear solver is the one of the settings
    /// (`LinearSolver::Auto`: dense up to [`crate::KLU_THRESHOLD`] states, KLU above).
    ///
    /// The description can be shared by the steppers of one model (an `Arc`). A model with
    /// delays is refused: its history would not follow a state the caller changes.
    pub fn new(
        model: Box<dyn ModelFns>,
        desc: impl Into<Arc<ModelDescription>>,
        settings: IntegratorSettings,
        changes: &[(String, f64)],
    ) -> Result<Stepper, EngineError> {
        let desc: Arc<ModelDescription> = desc.into();
        check_description(&desc)?;
        check_sizes(&*model, &desc)?;
        if desc.n_delays > 0 {
            return Err(request(format!(
                "model {} has delays, whose history a stepper does not keep",
                desc.id
            )));
        }
        let dae = desc.n_algebraic > 0;
        check_settings(&settings, desc.states.len(), dae)?;
        let mut settings = settings;
        let config = configure(&desc, &mut settings, 0)?;
        let metadata = changes::Metadata::new(&desc);
        let preinit = resolve_preinit(&metadata, changes)?;
        let no_changes = Changes::with_metadata(&metadata, &[], INIT_TIME, INIT_TIME)?;

        let mut scratch = Scratch::default();
        let mut initialize = || {
            initial_values(&*model, &desc, &preinit, &mut scratch)?;
            if dae {
                initial::consistent(&*model, &desc, &preinit, &mut scratch, &settings)?;
            }
            Ok(())
        };
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut initialize))
            .unwrap_or_else(|error| Err(EngineError::Internal(crate::panic_message(&*error))));
        #[cfg(target_arch = "wasm32")]
        let outcome = initialize();
        model.check_error().and(outcome)?;

        let solver = match config {
            None => Solver::None,
            Some(config) if dae => Solver::Dae(Integrator::new(config)?, Box::default()),
            Some(config) => Solver::Ode(Integrator::new(config)?),
        };
        let mut work = Vec::with_capacity(n_integrated(&desc));
        work.extend_from_slice(&scratch.y);
        Ok(Stepper {
            model,
            settings,
            solver,
            p: scratch.p,
            y: scratch.y,
            work,
            jac_values: Vec::new(),
            difference: None,
            events: events::Scratch::default(),
            segments: segments::Scratch::default(),
            selections: Vec::new(),
            changes: no_changes,
            limits: EventLimits::default(),
            failed: false,
            desc,
        })
    }

    /// The description of the model.
    pub fn description(&self) -> &ModelDescription {
        &self.desc
    }

    /// The state: the initial state after [`Stepper::new`], then the state after the last
    /// advance, in the order of the states of the description.
    pub fn state(&self) -> &[f64] {
        &self.y
    }

    /// Integrates the model from the state `y` at `t0` to `t1`, with the events and the
    /// discontinuities of the interval as a simulation handles them (see the module
    /// documentation); the state at `t1` is [`Stepper::state`] afterwards.
    ///
    /// The integrator starts again from `y` at `t0` (a projection on the algebraic equations
    /// first, for a model of algebraic rules), with the first step of a simulation. `t0` need
    /// not be where the advance before ended.
    ///
    /// A state of the wrong length or with a value which is not finite, and an interval which
    /// is not finite or ends before it starts, are refused ([`EngineError::Request`]) and change
    /// nothing. An advance which fails after that leaves the stepper failed: the events and
    /// the parameters may have changed on the way, so it refuses every advance after it
    /// ([`EngineError::Internal`]).
    pub fn advance(&mut self, y: &[f64], t0: f64, t1: f64) -> Result<StepStats, EngineError> {
        if self.failed {
            return Err(EngineError::Internal(
                "the stepper failed before and is not advanced again".into(),
            ));
        }
        let n = self.desc.states.len();
        if y.len() != n {
            return Err(request(format!(
                "a state of {} values for model {} of {n} states",
                y.len(),
                self.desc.id
            )));
        }
        if let Some(k) = y.iter().position(|value| !value.is_finite()) {
            return Err(request(format!(
                "the state {} of model {} is {}",
                self.desc.states[k].id, self.desc.id, y[k]
            )));
        }
        if !t0.is_finite() || !t1.is_finite() || t1 < t0 || !(t1 - t0).is_finite() {
            return Err(request(format!(
                "an advance from {t0} to {t1}: the interval is not finite or is negative"
            )));
        }
        let mut advance = || self.integrate(y, t0, t1);
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut advance))
            .unwrap_or_else(|error| Err(EngineError::Internal(crate::panic_message(&*error))));
        #[cfg(target_arch = "wasm32")]
        let outcome = advance();
        let outcome = self.model.check_error().and(outcome);
        if outcome.is_err() {
            self.failed = true;
        }
        outcome
    }

    /// An estimate of the memory of this instance in bytes: the stepper and its vectors, the
    /// integrator memory with the objects of SUNDIALS (complete after the first advance, see
    /// [`Integrator::memory_bytes`]), and the model object. The description, which the
    /// steppers of one model can share, and the code of the model are not counted. For the cell
    /// model of the tests (7 states) it is within 1% of the resident memory of 10000 steppers.
    pub fn memory_bytes(&self) -> usize {
        let f = std::mem::size_of::<f64>();
        let mut bytes = std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.model)
            + (self.p.capacity()
                + self.y.capacity()
                + self.work.capacity()
                + self.jac_values.capacity()
                + self.selections.capacity())
                * f
            + self.events.memory_bytes()
            + self.segments.memory_bytes()
            + self.difference.as_ref().map_or(0, Difference::memory_bytes);
        bytes += match &self.solver {
            Solver::None => 0,
            Solver::Ode(cvode) => cvode.memory_bytes(),
            Solver::Dae(ida, projections) => {
                ida.memory_bytes()
                    + std::mem::size_of::<Projections>()
                    + [&projections.projection, &projections.interpolation]
                        .into_iter()
                        .flatten()
                        .map(ConstraintWorkspace::memory_bytes)
                        .sum::<usize>()
            }
        };
        bytes
    }

    /// The advance of [`Stepper::advance`] after its checks.
    fn integrate(&mut self, y: &[f64], t0: f64, t1: f64) -> Result<StepStats, EngineError> {
        let n = self.desc.states.len();
        self.work.clear();
        self.work.extend_from_slice(y);
        self.work.resize(n_integrated(&self.desc), 0.0);
        let parts = Parts {
            model: &*self.model,
            desc: &self.desc,
            settings: &self.settings,
            p: &mut self.p,
            work: &mut self.work,
            jac_values: &mut self.jac_values,
            difference: &mut self.difference,
            events: &mut self.events,
            segments: &mut self.segments,
            selections: &mut self.selections,
            changes: &self.changes,
            limits: self.limits,
        };
        let stats = match &mut self.solver {
            Solver::None => parts.advance::<false>(None, None, t0, t1)?,
            Solver::Ode(cvode) => parts.advance(Some(cvode), None, t0, t1)?,
            Solver::Dae(ida, projections) => parts.advance(Some(ida), Some(projections), t0, t1)?,
        };
        self.y.copy_from_slice(&self.work[..n]);
        Ok(stats)
    }
}

/// What an advance borrows of its stepper besides the integrator.
struct Parts<'a> {
    model: &'a dyn ModelFns,
    desc: &'a ModelDescription,
    settings: &'a IntegratorSettings,
    p: &'a mut Vec<f64>,
    work: &'a mut Vec<f64>,
    jac_values: &'a mut Vec<f64>,
    difference: &'a mut Option<Difference>,
    events: &'a mut events::Scratch,
    segments: &'a mut segments::Scratch,
    selections: &'a mut Vec<f64>,
    changes: &'a Changes,
    limits: EventLimits,
}

impl Parts<'_> {
    /// Integrates `work` from `t0` to `t1` as `run_model` integrates a run without a steady
    /// state and without output: the problem of the run, the projection of the start of a
    /// model of algebraic rules, and the integration with or without events.
    fn advance<const DAE: bool>(
        self,
        mut cvode: Option<&mut Integrator<DAE>>,
        projections: Option<&mut Box<Projections>>,
        t0: f64,
        t1: f64,
    ) -> Result<StepStats, EngineError> {
        let Parts {
            model,
            desc,
            settings,
            p,
            work,
            jac_values,
            difference,
            events,
            segments,
            selections,
            changes,
            limits,
        } = self;
        let with_events = !desc.events.is_empty() || desc.n_roots > 0;
        let (pattern, difference, analytic) = match cvode.as_deref() {
            Some(cvode) => jacobian_of(desc, &cvode.config(), settings, false, difference),
            None => (None, None, false),
        };
        if let Some(cvode) = cvode.as_deref_mut() {
            // the counters of the memory start with this advance
            cvode.begin();
        }
        jac_values.resize(pattern.map_or(0, |pattern| pattern.row_idx.len()), 0.0);
        let mut projections = projections.map(|projections| &mut **projections);
        let interpolation = match projections.as_deref_mut() {
            Some(projections) if DAE && desc.n_discontinuity_roots > 0 => Some(Box::new(
                Constraints::cached(model, desc, settings, projections.interpolation.take())?,
            )),
            _ => None,
        };
        let mut problem = Problem {
            interpolation,
            pattern,
            analytic,
            difference,
            ..Problem::new(
                model,
                desc,
                work.len(),
                std::mem::take(p),
                std::mem::take(jac_values),
            )
        };
        let mut constraints = match projections.as_deref_mut() {
            Some(projections) if DAE => Some(Box::new(Constraints::cached(
                model,
                desc,
                settings,
                projections.projection.take(),
            )?)),
            _ => None,
        };
        // the state is the only output: a recorder of no columns (which reads no selection of
        // the model), of one row at the end
        let mut recorder = Recorder::with_scratch(model, &[], &[], selections, usize::MAX, 0)?;
        let end = [t1];
        let output = SegmentOutput::Times(&end);
        let (outcome, counts) = match cvode {
            Some(cvode) if with_events => {
                let mut events =
                    Events::<DAE>::resume(model, desc, DEFAULT_SEED, t0, t1, limits, events);
                events.constraints = constraints.take();
                let mut integrate = || {
                    if DAE && let Some(projection) = &mut events.constraints {
                        projection.project(t0, work, &problem.p)?;
                    }
                    segments::integrate_events(
                        cvode,
                        &mut problem,
                        settings,
                        t0,
                        work,
                        t1,
                        output,
                        &mut recorder,
                        &mut events,
                        changes,
                        segments,
                        false,
                    )
                };
                let outcome = integrate();
                constraints = events.constraints.take();
                (outcome, Some(cvode.counts()))
            }
            mut cvode => {
                let mut integrate = || {
                    if DAE && let Some(projection) = &mut constraints {
                        projection.project(t0, work, &problem.p)?;
                    }
                    segments::integrate(
                        cvode.as_deref_mut(),
                        &mut problem,
                        settings,
                        t0,
                        work,
                        t1,
                        output,
                        &mut recorder,
                        changes,
                        constraints.as_deref_mut(),
                    )
                };
                let outcome = integrate();
                (outcome, cvode.as_deref().map(Integrator::counts))
            }
        };
        drop(recorder);
        if let Some(projections) = projections {
            projections.projection = constraints.map(|projection| (*projection).into_workspace());
            projections.interpolation = problem
                .interpolation
                .take()
                .map(|projection| (*projection).into_workspace());
        }
        *p = problem.p;
        *jac_values = problem.jac_values;
        outcome?;
        let Some(counts) = counts else {
            return Ok(StepStats::default());
        };
        Ok(StepStats {
            steps: counts.steps,
            rejected: counts.error_test_failures + counts.nonlinear_convergence_failures,
        })
    }
}

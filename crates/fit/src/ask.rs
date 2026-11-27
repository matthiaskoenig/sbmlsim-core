//! The fit of a host which simulates itself: the optimizers of a fit driven ask/tell.
//!
//! A page has no thread of its own to block on a simulation: its engine runs on the workers of
//! JavaScript, which answer later. So the optimizer cannot call the objective, it asks for it:
//! [`Asking::ask`] runs the optimizer until it needs an evaluation the host has not given yet
//! and answers the simulations of that evaluation ([`Simulation`]), the host simulates them on
//! its workers, next to each other, and hands the tables back ([`Asking::tell`]). The next ask
//! continues the optimizer with those evaluations, until it gives the fit ([`Ask::Done`]).
//!
//! What the optimizer asks for is what [`Problem::optimize`] evaluates on the batch, so the
//! simulations and their order of a step are the ones of the batch, and the optimizer is the
//! same:
//!
//! - the residuals of a point are one simulation per condition,
//! - the jacobian of a point is `k + 1` such evaluations, the point and every column of the
//!   finite differences ([`Problem::step`]); a table a host hands in carries no sensitivities,
//! - a generation of a differential evolution is one evaluation per member, which are asked
//!   for together, since the members of a generation do not depend on each other.
//!
//! An optimizer of basin runs a step in one call and cannot wait inside it, so a step which
//! misses an evaluation is abandoned and run again once the host told it. The trust region,
//! whose solver clones, runs again from the state before the step; the Levenberg-Marquardt and
//! the evolution, whose solvers do not, run again from their start, which repeats the
//! arithmetic of the optimizer and none of the simulations: every evaluation it took is kept.
//!
//! The loop of a step is the one of the executor of basin, which [`Problem::optimize`] runs:
//! the iteration limit, then the convergence of the solver, then the step. The executor itself
//! is not used, as its limit of time reads a clock a module of `wasm32-unknown-unknown` has
//! not; a host stops a fit by its own clock ([`Asking::stop`]), and a fit which is stopped
//! takes the steps whose evaluations the host has told and ends before the first which needs
//! a simulation more, so that a stop wastes none of them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use basin::{
    ApplicationStop, BoxBounds, ConstraintDescription, ConstraintSet, ConstraintSolveError,
    Constraints, CostFunction, CountsMirror, DenseMatrix, EvalCounts, ExecutionLimit, InitialState,
    Jacobian, PointState, PopulationProgress, Problem as Wrapped, Residual, Solver, SolverStep,
    State, Termination, TerminationReport, TerminationStage,
};
use sbmlsim_abi::{EngineError, SimulationResult};

use crate::optimize::{evolution, levenberg_marquardt, trust_region};
use crate::problem::{FitError, Opening, Problem, Simulation};
use crate::{Algorithm, Fit, Settings};

/// What a fit asks of its host.
#[derive(Debug)]
pub enum Ask {
    /// Simulate these and tell their results, in this order ([`Asking::tell`]).
    Simulate(Vec<Simulation>),
    /// The fit is done.
    Done(Fit),
}

/// A point of the search space as a key: the bits of its values.
type Key = Vec<u64>;

fn key(x: &[f64]) -> Key {
    x.iter().map(|value| value.to_bits()).collect()
}

/// An evaluation an optimizer asked for and the host has not given yet.
#[derive(Clone, Debug, PartialEq)]
enum Wanted {
    /// The residuals of a point.
    Residuals(Vec<f64>),
    /// The jacobian of a point.
    Jacobian(Vec<f64>),
    /// The cost of a point, of a member of an evolution, whose residuals are not kept.
    Cost(Vec<f64>),
}

/// The evaluations of a run: the ones the host gave and the ones it is asked for.
#[derive(Debug, Default)]
struct Evaluations {
    residuals: HashMap<Key, Vec<f64>>,
    jacobians: HashMap<Key, Vec<f64>>,
    costs: HashMap<Key, f64>,
    /// The evaluations an attempt of the optimizer missed, in the order it asked for them.
    missed: Vec<Wanted>,
}

impl Evaluations {
    fn miss(&mut self, wanted: Wanted) {
        if !self.missed.contains(&wanted) {
            self.missed.push(wanted);
        }
    }
}

/// The evaluations of a run, shared by the problem of every attempt of its optimizer.
type Shared = Arc<Mutex<Evaluations>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Evaluations> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An evaluation the host has not given yet: the attempt of the step is abandoned.
#[derive(Debug)]
struct Pending;

impl std::fmt::Display for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an evaluation the host has not given yet")
    }
}

/// The problem of an attempt of the optimizer: the evaluations of the host, and a miss for the
/// ones it has not given.
struct Told {
    problem: Arc<Problem>,
    evaluations: Shared,
    lower: Vec<f64>,
    upper: Vec<f64>,
}

impl Told {
    fn new(problem: &Arc<Problem>, evaluations: &Shared) -> Told {
        let (lower, upper) = problem.bounds();
        Told {
            problem: Arc::clone(problem),
            evaluations: Arc::clone(evaluations),
            lower,
            upper,
        }
    }
}

impl Residual for Told {
    type Param = Vec<f64>;
    type Output = Vec<f64>;
    type Error = Pending;

    fn residual(&self, x: &Vec<f64>) -> Result<Vec<f64>, Pending> {
        let mut evaluations = lock(&self.evaluations);
        if let Some(residuals) = evaluations.residuals.get(&key(x)) {
            return Ok(residuals.clone());
        }
        evaluations.miss(Wanted::Residuals(x.clone()));
        Err(Pending)
    }
}

impl Jacobian for Told {
    type Jacobian = DenseMatrix;

    fn jacobian(&self, x: &Vec<f64>) -> Result<DenseMatrix, Pending> {
        let mut evaluations = lock(&self.evaluations);
        if let Some(jacobian) = evaluations.jacobians.get(&key(x)) {
            return Ok(DenseMatrix::from_row_slice(
                self.problem.n_residuals(),
                self.problem.parameters().len(),
                jacobian,
            ));
        }
        evaluations.miss(Wanted::Jacobian(x.clone()));
        Err(Pending)
    }

    /// The residuals and the jacobian of a point, asked for together: the residuals are the
    /// first of the `k + 1` evaluations of the jacobian, so they cost no simulation of their
    /// own and the step waits for the host once.
    fn residual_and_jacobian(&self, x: &Vec<f64>) -> Result<(Vec<f64>, DenseMatrix), Pending> {
        let k = key(x);
        {
            let mut evaluations = lock(&self.evaluations);
            if !(evaluations.residuals.contains_key(&k) && evaluations.jacobians.contains_key(&k)) {
                evaluations.miss(Wanted::Residuals(x.clone()));
                evaluations.miss(Wanted::Jacobian(x.clone()));
                return Err(Pending);
            }
        }
        Ok((self.residual(x)?, self.jacobian(x)?))
    }
}

impl CostFunction for Told {
    type Param = Vec<f64>;
    type Output = f64;
    type Error = Pending;

    /// The cost of a point, which a generation of an evolution asks for member by member: a
    /// member the host has not given is noted and costs nothing here, so that the generation
    /// goes on and asks for every member at once; the step is abandoned after it.
    fn cost(&self, x: &Vec<f64>) -> Result<f64, Pending> {
        let mut evaluations = lock(&self.evaluations);
        if let Some(cost) = evaluations.costs.get(&key(x)) {
            return Ok(*cost);
        }
        evaluations.miss(Wanted::Cost(x.clone()));
        Ok(f64::INFINITY)
    }
}

impl Constraints for Told {
    type Set = ConstraintSet<BoxBounds<Vec<f64>>>;

    fn constraints(&self) -> <Self::Set as ConstraintDescription>::Borrowed<'_> {
        ConstraintSet::new().with_bounds(BoxBounds::new(Some(&self.lower), Some(&self.upper)))
    }
}

/// The error of the optimizer of an attempt: a miss, or a failure of the optimizer.
fn missed(error: ConstraintSolveError<Pending>) -> Result<(), FitError> {
    match error {
        ConstraintSolveError::Evaluation(Pending) => Ok(()),
        ConstraintSolveError::InvalidConstraints(error) => Err(FitError::Optimizer(format!(
            "refused the bounds of the fit: {error}"
        ))),
        other => Err(FitError::Optimizer(format!("failed: {other}"))),
    }
}

/// Where an optimizer ended: its point, its iterations, its evaluations and why it stopped.
struct Ended {
    x: Vec<f64>,
    iterations: u64,
    counts: EvalCounts,
    status: String,
}

/// The progress of an attempt of an optimizer.
enum Progress {
    /// It misses evaluations, which [`Evaluations::missed`] holds.
    Missing,
    /// It ended.
    Ended(Ended),
}

/// An optimizer of basin run step by step on the evaluations of a host.
trait Run {
    /// Runs the optimizer as far as the evaluations of the host reach.
    fn advance(
        &mut self,
        problem: &Arc<Problem>,
        evaluations: &Shared,
        stop: bool,
    ) -> Result<Progress, FitError>;
}

/// The solver and the state of an optimizer after a completed step, with its evaluations.
struct Snapshot<So, S> {
    solver: So,
    state: S,
    counts: EvalCounts,
}

/// An optimizer of basin: a solver, the state it starts from, how its result reads and, for a
/// solver which clones, the snapshot of its last completed step.
struct Stepped<So, S> {
    /// The solver and the state of the start of the optimizer.
    make: Box<dyn Fn() -> (So, S)>,
    /// The solver which clones, for a snapshot after every step, or `None` for a solver which
    /// runs again from its start.
    clone: Option<fn(&So) -> So>,
    /// The point, the iterations and the evaluations of a state.
    read: fn(&S) -> (Vec<f64>, u64, EvalCounts),
    max_iterations: u64,
    snapshot: Option<Snapshot<So, S>>,
}

impl<So, S> Stepped<So, S>
where
    S: State + CountsMirror + Clone,
    So: Solver<Told, S, Error = ConstraintSolveError<Pending>>,
{
    /// A step which misses evaluations: the host is asked for them, or, for a fit which is
    /// stopped, the optimizer ends at the state before the step.
    fn missing(&self, before: Option<S>, evaluations: &Shared) -> Progress {
        match before {
            None => Progress::Missing,
            Some(state) => {
                lock(evaluations).missed.clear();
                let termination =
                    Termination::Application(ApplicationStop::new("the fit was stopped"));
                self.ended(&state, termination, TerminationStage::Boundary)
            }
        }
    }

    /// The end of the optimizer at a state, with the termination of its report.
    fn ended(
        &self,
        state: &S,
        termination: Termination<S::Float>,
        stage: TerminationStage,
    ) -> Progress {
        let (x, iterations, counts) = (self.read)(state);
        let report = TerminationReport::new(termination, stage, iterations);
        Progress::Ended(Ended {
            x,
            iterations,
            counts,
            status: format!("{:?}", report.code()),
        })
    }
}

impl<So, S> Run for Stepped<So, S>
where
    S: State + CountsMirror + Clone,
    So: Solver<Told, S, Error = ConstraintSolveError<Pending>>,
{
    fn advance(
        &mut self,
        problem: &Arc<Problem>,
        evaluations: &Shared,
        stop: bool,
    ) -> Result<Progress, FitError> {
        let mut wrapped = Wrapped::new(Told::new(problem, evaluations));
        // the start of the attempt: the snapshot of the last completed step, or the start of
        // the optimizer, initialized as `Executor::into_stepper` initializes it
        let (mut solver, mut state) = match (&self.snapshot, self.clone) {
            (Some(snapshot), Some(clone)) => {
                *wrapped.counts_mut() = snapshot.counts;
                (clone(&snapshot.solver), snapshot.state.clone())
            }
            _ => {
                let (mut solver, mut state) = (self.make)();
                state.reset_best();
                solver.reset_convergence();
                let mut state = match solver.init(&mut wrapped, state) {
                    Ok(state) => state,
                    Err(error) => {
                        missed(error)?;
                        return Ok(Progress::Missing);
                    }
                };
                if !lock(evaluations).missed.is_empty() {
                    return Ok(Progress::Missing);
                }
                state.mirror(wrapped.counts());
                state.update_best();
                (solver, state)
            }
        };
        loop {
            // the boundary of a step, as `step_once` of the executor of basin checks it: the
            // iteration limit, then the convergence of the solver
            if state.iter() >= self.max_iterations {
                let limit = ExecutionLimit::Iterations {
                    observed: state.iter(),
                    limit: self.max_iterations,
                };
                return Ok(self.ended(
                    &state,
                    Termination::Limit(vec![limit]),
                    TerminationStage::Boundary,
                ));
            }
            if let Some(termination) = solver.check_convergence(&wrapped, &state) {
                return Ok(self.ended(&state, termination, TerminationStage::Boundary));
            }
            // a fit which is stopped takes the steps the host has told and ends at the first
            // which needs a simulation more, at the boundary before it
            let before = stop.then(|| state.clone());
            let step: SolverStep<S> = match solver.next_iter(&mut wrapped, state) {
                Ok(step) => step,
                Err(error) => {
                    missed(error)?;
                    return Ok(self.missing(before, evaluations));
                }
            };
            if !lock(evaluations).missed.is_empty() {
                return Ok(self.missing(before, evaluations));
            }
            let (mut next, completed, termination) = (step.state, step.completed, step.termination);
            next.mirror(wrapped.counts());
            if completed {
                next.increment_iter();
            }
            next.update_best();
            if let Some(termination) = termination {
                return Ok(self.ended(&next, termination, TerminationStage::Step { completed }));
            }
            if let Some(clone) = self.clone {
                // a step which completed needs none of the evaluations before it again
                let (x, _, _) = (self.read)(&next);
                let at = key(&x);
                let mut told = lock(evaluations);
                told.residuals.retain(|k, _| *k == at);
                told.jacobians.retain(|k, _| *k == at);
                drop(told);
                self.snapshot = Some(Snapshot {
                    solver: clone(&solver),
                    state: next.clone(),
                    counts: *wrapped.counts(),
                });
            }
            state = next;
        }
    }
}

/// The trust region of the settings from a point, which clones.
fn trust_region_run(settings: &Settings, from: &[f64], problem: &Problem) -> Box<dyn Run> {
    let (lower, upper) = problem.bounds();
    let start = from.to_vec();
    let settings = settings.clone();
    let solver = trust_region!(&settings, &start, &lower, &upper);
    Box::new(Stepped {
        make: Box::new(move || {
            let solver = solver.clone();
            let state: PointState<Vec<f64>, f64> = solver.seed(&start);
            (solver, state)
        }),
        clone: Some(Clone::clone),
        read: |state: &PointState<Vec<f64>, f64>| {
            (state.param().clone(), state.iter(), *state.counts())
        },
        max_iterations: settings.max_iterations,
        snapshot: None,
    })
}

/// The Levenberg-Marquardt of the settings from a point, which runs again from its start.
fn levenberg_marquardt_run(settings: &Settings, from: &[f64]) -> Box<dyn Run> {
    let start = from.to_vec();
    let settings = settings.clone();
    let max_iterations = settings.max_iterations;
    Box::new(Stepped {
        make: Box::new(move || {
            let solver = levenberg_marquardt!(&settings);
            let state = solver.seed(&start);
            (solver, state)
        }),
        clone: None,
        read: |state: &PointState<Vec<f64>, f64>| {
            (state.param().clone(), state.iter(), *state.counts())
        },
        max_iterations,
        snapshot: None,
    })
}

/// The differential evolution of the settings, which runs again from its start.
fn evolution_run(
    settings: &Settings,
    evolution: &crate::Evolution,
    problem: &Arc<Problem>,
) -> Result<Box<dyn Run>, FitError> {
    // the population and the bounds are checked once, where the batch checks them
    self::evolution(evolution, problem)?;
    let evolution = *evolution;
    let problem = Arc::clone(problem);
    Ok(Box::new(Stepped {
        make: Box::new(move || {
            let solver = self::evolution(&evolution, &problem).expect("the evolution was checked");
            (solver, PopulationProgress::empty())
        }),
        clone: None,
        read: |state: &PopulationProgress<Vec<f64>, f64>| {
            (state.best_param().clone(), state.iter(), *state.counts())
        },
        max_iterations: settings.max_iterations,
        snapshot: None,
    }))
}

/// The phase of a fit.
enum Phase {
    /// The tables of the conditions at the start are asked for, against which the definition
    /// is resolved.
    Opening(Box<Opening>),
    /// An optimizer runs.
    Running,
    /// The optimizer ended at a point whose residuals are asked for.
    Ending,
    /// The fit is done.
    Done,
}

/// A fit whose simulations a host runs: the optimizers of [`Problem::optimize`], driven
/// ask/tell.
///
/// ```text
/// let mut fit = Asking::open(opening, settings)?;   // or Asking::new(problem, settings, from)
/// loop {
///     match fit.ask()? {
///         Ask::Simulate(simulations) => fit.tell(simulate(simulations))?,
///         Ask::Done(fit) => break fit,
///     }
/// }
/// ```
///
/// The fit is the one [`Problem::optimize_from`] gives on the batch, to the bit when the host
/// simulates as the engine of the batch does: the same optimizer, the same evaluations in the
/// same order, the same counts of iterations and evaluations; its time is the host's to
/// measure, [`Fit::seconds`] is zero.
pub struct Asking {
    settings: Settings,
    phase: Phase,
    problem: Option<Arc<Problem>>,
    from: Option<Vec<f64>>,
    evaluations: Shared,
    run: Option<Box<dyn Run>>,
    /// Whether the run is the polish of an evolution.
    polishing: bool,
    /// The end of the evolution before its polish.
    evolved: Option<Ended>,
    ended: Option<Ended>,
    /// The simulations of the last ask, which the next tell answers.
    asked: Vec<Simulation>,
    /// The values of the varying targets of each of them, as the key of its output.
    asked_at: Vec<Key>,
    /// The evaluations those simulations are for.
    wanted: Vec<Wanted>,
    stopped: bool,
}

impl std::fmt::Debug for Asking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asking")
            .field("asked", &self.asked.len())
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl Asking {
    /// A fit of a definition which is still to be resolved: its first ask is the simulations of
    /// its conditions at the start ([`Opening::simulations`]), and it starts from the start of
    /// its parameters.
    #[must_use]
    pub fn open(opening: Opening, settings: Settings) -> Asking {
        Asking::with(Phase::Opening(Box::new(opening)), None, None, settings)
    }

    /// A fit of a problem resolved on the tables of a host ([`Opening::resolve`]), from a point
    /// of the search space.
    ///
    /// # Errors
    ///
    /// A point which is not one value per parameter of the problem.
    pub fn new(problem: Problem, settings: Settings, from: &[f64]) -> Result<Asking, FitError> {
        if from.len() != problem.parameters().len() {
            return Err(FitError::Optimizer(format!(
                "was started at {} values of the {} parameters of the fit",
                from.len(),
                problem.parameters().len()
            )));
        }
        Ok(Asking::with(
            Phase::Running,
            Some(Arc::new(problem)),
            Some(from.to_vec()),
            settings,
        ))
    }

    fn with(
        phase: Phase,
        problem: Option<Arc<Problem>>,
        from: Option<Vec<f64>>,
        settings: Settings,
    ) -> Asking {
        Asking {
            settings,
            phase,
            problem,
            from,
            evaluations: Shared::default(),
            run: None,
            polishing: false,
            evolved: None,
            ended: None,
            asked: Vec::new(),
            asked_at: Vec::new(),
            wanted: Vec::new(),
            stopped: false,
        }
    }

    /// The problem of the fit, once its definition is resolved.
    #[must_use]
    pub fn problem(&self) -> Option<&Problem> {
        self.problem.as_deref()
    }

    /// Stops the fit: it takes the steps whose simulations were told and ends before the first
    /// which needs a simulation more, at the point it reached, as a [`crate::Stop`] stops a fit
    /// on the batch; the ask after a stop may still ask for the residuals of that point.
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// What the fit needs next: the simulations of the evaluations its optimizer misses, or the
    /// fit itself.
    ///
    /// # Errors
    ///
    /// A failure of the optimizer, and an ask before the simulations of the last one were told.
    pub fn ask(&mut self) -> Result<Ask, FitError> {
        if !self.asked.is_empty() {
            return Ok(Ask::Simulate(self.asked.clone()));
        }
        if let Phase::Opening(opening) = &self.phase {
            self.asked = opening.simulations();
            return Ok(Ask::Simulate(self.asked.clone()));
        }
        let problem = Arc::clone(self.problem.as_ref().expect("a resolved problem"));
        loop {
            match self.phase {
                Phase::Opening(_) => unreachable!("answered above"),
                Phase::Done => {
                    return Err(FitError::Optimizer("is done, and asks nothing".to_string()));
                }
                Phase::Ending => {
                    let ended = self.ended.as_ref().expect("an end");
                    let x = ended.x.clone();
                    if let Some(residuals) = lock(&self.evaluations).residuals.get(&key(&x)) {
                        let fit = self.fit(&problem, residuals.clone());
                        self.phase = Phase::Done;
                        return Ok(Ask::Done(fit));
                    }
                    lock(&self.evaluations).miss(Wanted::Residuals(x));
                    return Ok(self.simulations(&problem));
                }
                Phase::Running => {}
            }
            if self.run.is_none() {
                let from = self.from.clone().unwrap_or_else(|| problem.start());
                self.run = Some(match self.settings.algorithm {
                    Algorithm::TrustRegion => trust_region_run(&self.settings, &from, &problem),
                    Algorithm::LevenbergMarquardt => levenberg_marquardt_run(&self.settings, &from),
                    Algorithm::DifferentialEvolution(evolution) => {
                        evolution_run(&self.settings, &evolution, &problem)?
                    }
                });
            }
            let run = self.run.as_mut().expect("a run");
            lock(&self.evaluations).missed.clear();
            match run.advance(&problem, &self.evaluations, self.stopped)? {
                Progress::Missing => return Ok(self.simulations(&problem)),
                Progress::Ended(ended) => self.finish(ended),
            }
        }
    }

    /// The end of a run: the polish of an evolution, or the end of the fit.
    fn finish(&mut self, ended: Ended) {
        self.run = None;
        if let Algorithm::DifferentialEvolution(evolution) = self.settings.algorithm
            && evolution.polish
            && !self.polishing
        {
            // a least squares optimizer from the best member the evolution found
            self.polishing = true;
            self.from = Some(ended.x.clone());
            self.settings.algorithm = Algorithm::TrustRegion;
            self.evolved = Some(ended);
            return;
        }
        let ended = match self.evolved.take() {
            Some(evolved) => {
                let mut counts = evolved.counts;
                counts.cost_evals += ended.counts.residual_evals.max(ended.counts.cost_evals);
                counts.residual_evals += ended.counts.residual_evals.max(ended.counts.cost_evals);
                counts.jacobian_evals += ended.counts.jacobian_evals;
                Ended {
                    x: ended.x,
                    iterations: evolved.iterations + ended.iterations,
                    counts,
                    status: format!("{}, polish {}", evolved.status, ended.status),
                }
            }
            None => ended,
        };
        self.ended = Some(ended);
        self.phase = Phase::Ending;
    }

    /// The fit at the end of its optimizer, with the residuals there.
    fn fit(&self, problem: &Problem, residuals: Vec<f64>) -> Fit {
        let ended = self.ended.as_ref().expect("an end");
        Fit {
            values: problem.parameter_values(&ended.x),
            cost: 0.5 * residuals.iter().map(|value| value * value).sum::<f64>(),
            residuals,
            x: ended.x.clone(),
            iterations: ended.iterations,
            evaluations: ended.counts.residual_evals.max(ended.counts.cost_evals),
            jacobians: ended.counts.jacobian_evals,
            status: ended.status.clone(),
            seconds: 0.0,
        }
    }

    /// The simulations of the evaluations the optimizer missed, each once.
    fn simulations(&mut self, problem: &Problem) -> Ask {
        let wanted = std::mem::take(&mut lock(&self.evaluations).missed);
        let mut seen: HashMap<(usize, Key), ()> = HashMap::new();
        let mut asked = Vec::new();
        let mut asked_at = Vec::new();
        for want in &wanted {
            let points = match want {
                Wanted::Residuals(x) | Wanted::Cost(x) => vec![problem.values(x)],
                Wanted::Jacobian(x) => problem.differences(x).1,
            };
            for values in &points {
                let at = key(values);
                for condition in 0..problem.n_conditions() {
                    if seen.insert((condition, at.clone()), ()).is_none() {
                        asked.push(problem.simulation(condition, values));
                        asked_at.push(at.clone());
                    }
                }
            }
        }
        self.wanted = wanted;
        self.asked = asked;
        self.asked_at = asked_at;
        Ask::Simulate(self.asked.clone())
    }

    /// Takes the results of the simulations of the last ask, in its order: the table of a
    /// simulation, or the error of the engine which failed it.
    ///
    /// A failed integration ([`EngineError::Integration`]) is a point the fit may not take,
    /// whose residuals are [`crate::FAILED_RESIDUAL`], as on the batch; any other failure of the
    /// engine is an error of the fit, as on the batch.
    ///
    /// # Errors
    ///
    /// Results which are not one per simulation of the last ask, a failure of the engine which
    /// is no failed integration, a table at the start which does not resolve the definition and
    /// a table which is not the one of its condition.
    pub fn tell(
        &mut self,
        results: Vec<Result<SimulationResult, EngineError>>,
    ) -> Result<(), FitError> {
        if results.len() != self.asked.len() {
            return Err(FitError::Request(format!(
                "asked for {} simulations and was told {} results",
                self.asked.len(),
                results.len()
            )));
        }
        let asked = std::mem::take(&mut self.asked);
        let asked_at = std::mem::take(&mut self.asked_at);
        if let Phase::Opening(_) = self.phase {
            let Phase::Opening(opening) = std::mem::replace(&mut self.phase, Phase::Running) else {
                unreachable!("matched above")
            };
            let mut tables = Vec::with_capacity(results.len());
            for (simulation, result) in asked.iter().zip(results) {
                tables.push(result.map_err(|err| FitError::Condition {
                    at: simulation.condition,
                    message: format!("cannot be simulated at the start of the fit: {err}"),
                })?);
            }
            self.problem = Some(Arc::new(opening.resolve(&tables)?));
            return Ok(());
        }
        let problem = Arc::clone(self.problem.as_ref().expect("a resolved problem"));
        // the output of every simulation, by its condition and its values
        let mut outputs: HashMap<(usize, Key), Option<Vec<f64>>> = HashMap::new();
        for ((simulation, at), result) in asked.iter().zip(asked_at).zip(results) {
            let output = match result {
                Ok(table) => Some(problem.output_of(simulation.condition, &table)?),
                Err(EngineError::Integration { .. }) => None,
                Err(err) => return Err(FitError::Engine(err)),
            };
            outputs.insert((simulation.condition, at), output);
        }
        let evaluation = |values: &[f64]| -> Result<Vec<f64>, FitError> {
            let k = key(values);
            let mut of = Vec::with_capacity(problem.n_conditions());
            for condition in 0..problem.n_conditions() {
                of.push(
                    outputs
                        .get(&(condition, k.clone()))
                        .cloned()
                        .ok_or_else(|| {
                            FitError::Request("was not told a simulation it asked for".to_string())
                        })?,
                );
            }
            Ok(problem.residuals_of(values, &of))
        };
        let mut evaluations = lock(&self.evaluations);
        for want in std::mem::take(&mut self.wanted) {
            match want {
                Wanted::Residuals(x) => {
                    let residuals = evaluation(&problem.values(&x))?;
                    evaluations.residuals.insert(key(&x), residuals);
                }
                Wanted::Cost(x) => {
                    let residuals = evaluation(&problem.values(&x))?;
                    let cost = 0.5 * residuals.iter().map(|value| value * value).sum::<f64>();
                    evaluations.costs.insert(key(&x), cost);
                }
                Wanted::Jacobian(x) => {
                    let (steps, moved) = problem.differences(&x);
                    let residuals = moved
                        .iter()
                        .map(|values| evaluation(values))
                        .collect::<Result<Vec<_>, _>>()?;
                    let jacobian = problem.difference_quotient(&steps, &residuals);
                    evaluations.jacobians.insert(key(&x), jacobian);
                }
            }
        }
        Ok(())
    }
}

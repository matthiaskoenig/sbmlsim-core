//! The planner of a scan of sbmlsim-core: a simulation and its dimensions, the request
//! `scan/1` ([`ScanRequest`]), planned into the requests `simulation/1` of the engine as
//! sbmlsim plans them (`simulator/plan.py`, `simulator/executor_core.py`), so a host of Python
//! or JavaScript runs a scan of sbmlsim from the description of the compiled model alone.
//!
//! [`Scan`] is a state machine which simulates nothing itself: [`Scan::ask`] gives the next
//! round of requests (a batch of at most [`CORE_MAX_CHUNK`] points, a chunk of the points of one
//! simulation and one model in C order of the scan, as sbmlsim's run of the engine core cuts
//! them), the host simulates them on the model the round names and [`Scan::tell`] takes the
//! output or the error of every request, until `ask` says [`Ask::Done`]; [`Scan::result`] is
//! then the result of the scan, `scanresult/1`.
//!
//! ```text
//! let mut scan = Scan::open(request, description)?;
//! loop {
//!     match scan.ask() {
//!         Ask::Simulate { model, requests } => scan.tell(simulate(model, requests))?,
//!         Ask::Observe(points) => scan.tell_observed(observe(points))?,
//!         Ask::Done => break,
//!     }
//! }
//! let result = scan.result()?;
//! ```
//!
//! The requests are the ones of sbmlsim bitwise (`tests/fixtures.rs` compares them with the
//! oracle fixtures `testdata/scan`): the times and values in the units of the model (the
//! factors of `sbmlsim-units`, which are pint's), the output times merged within the resolution
//! of the time, the values of the dimensions applied as `Plan.with_values` applies them, the
//! formulas before the initialization folded, the steady states before the start and after the
//! end, and the selections of the run.
//!
//! A formula at a change which reads the state of the model is folded into a number by
//! simulating the segment which ends at the change: a chunk asks a round per such change of its
//! longest protocol, the k-th segment of every point which still folds as one batch, before the
//! round of the points themselves, as sbmlsim's `requests_of` batches them. The segment of a
//! model with events ends at the state before the events of the time of the change
//! (`before_events`), which is the state sbmlsim's executor of roadrunner applies a change to.
//! A segment which fails fails its point alone.
//!
//! What the model needs to be planned comes from its description `model/2`: the units of its
//! entities and of its time, the kinds of its entities, which ones a change may set, their
//! values as loaded, its states, which parameters an initial assignment computes, whether it
//! has events and whether it depends on the time of its clock.
//!
//! The observables of the run (`Formula`, `PK`, `Custom`, sbmlsim's `Observable.to_dict()`)
//! are evaluated on the native output of every point once the points of a chunk ran, before
//! any interpolation: a formula with the operations of the engine and its reductions `max`,
//! `min`, `mean` and `at` over the time of the point, a PK observable with the kernel of
//! pkpdutils, and a `Custom` observable, a function of the host, in a round of its own
//! ([`Ask::Observe`]: the finite time points of every point and the columns the functions
//! read, answered by [`Scan::tell_observed`]). A point fails exactly when its own simulation
//! or its own observables fail.
//!
//! [`Scan::result`] assembles the result `scanresult/1` ([`ScanResult`], see its module): the
//! kept timecourses on the grid of `time` (interpolated with the rules of sbmlsim's
//! `grid_weights`), on the output times every simulation shares, or ragged over `_point`, the
//! values per simulation, the coordinates of the dimensions, `status` and the errors of the
//! failed points of `on_error: flag`, and the block of the points of a run of `points`.
//!
//! # The request `scan/1`
//!
//! [`ScanRequest`]: `contract` (`scan/1`), `scan` (sbmlsim's `Scan.to_dict()`: the simulation
//! and the dimensions), `observables` (`Observable.to_dict()` of each), `keep`, `time` (a grid,
//! numbers in the time unit of the model or `{"value": [...], "unit": u}`), `on_error`
//! (`raise` or `flag`), `points` (`[start, stop)` of the flat indices of a block), `settings`
//! (the integrator settings of `simulation/1`), `changes` (the changes of the model of the
//! run), `model_settings` and `model_changes` (the settings and the changes of the models of a
//! dimension of models by their key) and `selections` (the selections of the model).
//!
//! The changes of a model (sbmlsim's `AbstractModel.changes`, `changes` of the model of the run
//! and `model_changes` of the models of a dimension of models) are the defaults of the
//! pre-initialization of every simulation of the model
//! ([`SimulationDefinition::with_preinit_defaults`]).
//!
//! # Dimensions of models
//!
//! A dimension of models gives every point the model of its key, which replaces the model of
//! the run: the host compiles every model and opens the scan with their descriptions by their
//! keys ([`Scan::open_with_models`]), and every round of simulations names the model of its
//! requests ([`Ask::Simulate`]). The models are checked against the first one as sbmlsim's
//! `_check_model` does, and the result has the units of the first model.

mod assemble;
mod derive;
mod error;
mod expand;
mod fold;
mod formula;
mod host_observables;
mod model;
mod observables;
mod pk;
mod plan;
mod plan_export;
mod reduce;
mod request;
mod requests;
mod result;
mod text;
mod units;

use std::ops::Range;

use sbmlsim_abi::{IntegratorSettings, ModelDescription, SimulationRequest, SimulationResult};

use crate::assemble::{PointOutput, interpolated};
use crate::fold::Fold;
use crate::observables::{Graph, Observation, PointRun};
use crate::reduce::Val;
use crate::requests::{base_request, output_columns as columns_of};

pub use error::{PointError, PointErrorKind, STEADY_STATE_FLAG, ScanError, ScanErrorKind};
pub use expand::{CORE_MAX_CHUNK, Expanded, PointValues, RESERVED};
pub use host_observables::{HostError, HostObservables, HostObserved, HostPoint};
pub use model::{ModelInfo, TargetKind};
pub use plan::{
    Assigned, Assignment, OutputMode, Plan, PlanEvent, STEADY_STATE, SteadyStatePlan, TIME_RTOL,
    compile_simulation, linspace, merge_times, time_tolerance,
};
pub use plan_export::{
    AssignmentDocument, CompiledPlan, CompiledPlans, EventDocument, PLAN_CONTRACT, PlanDocument,
    PlanRequest, SteadyStateDocument, TargetValues, compile_plans,
};
pub use request::{
    ChangeDefinition, DimensionDefinition, DimensionKind, Dose, Num, Observable, OnError, Ordered,
    SCAN_CONTRACT, ScanDefinition, ScanRequest, SimulationDefinition, SteadyStateDefinition, Time,
    Value, Values,
};
pub use requests::{output_columns, run_selections};
pub use result::{
    Block, Coord, CoordValues, FailedPoint, Layout, MAX_ERRORS, RESULT_CONTRACT, ScanResult, Size,
    Variable,
};

/// What the scan asks of its host next.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Simulate the requests, one batch, on a model, and tell the output or the error of every
    /// one of them in their order ([`Scan::tell`]).
    Simulate {
        /// The key of the model of the requests in the dimension of models, `None` for the
        /// model of the run.
        model: Option<String>,
        /// The requests.
        requests: Vec<SimulationRequest>,
    },
    /// Compute the observables of the host of the points and tell their values
    /// ([`Scan::tell_observed`]).
    Observe(HostObservables),
    /// Nothing: the scan is done, or failed ([`Scan::result`] says which).
    Done,
}

/// What a round of the scan simulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundKind {
    /// The segments of the folds of the points of the chunk.
    Segments,
    /// The points of the chunk.
    Points,
}

/// The round the scan waits for.
#[derive(Debug, Clone)]
struct Pending {
    kind: RoundKind,
    /// The position in the chunk of the point of every request, in their order.
    positions: Vec<usize>,
    /// The requests.
    requests: Vec<SimulationRequest>,
    /// The model of the requests, its index in the models of the scan.
    model: usize,
}

/// The fold of a point of the chunk which runs, or the failure of the point in words.
type PointFold = Result<Fold, String>;

/// A scan planned into the requests of the engine, see the crate.
#[derive(Debug, Clone)]
pub struct Scan {
    request: ScanRequest,
    /// The model of the run, or the model of every point of the dimension of models in its
    /// order.
    models: Vec<ModelInfo>,
    /// The key of every model in the dimension of models, `None` for the model of the run.
    keys: Vec<Option<String>>,
    expanded: Expanded,
    selections: Vec<String>,
    block: Range<usize>,
    chunks: Vec<Vec<usize>>,
    /// The chunk the next round of [`Scan::ask`] is of.
    next: usize,
    /// The folds of the points of that chunk, once its first round was asked.
    folds: Option<Vec<PointFold>>,
    /// The plan of every point of that chunk, in its order.
    plans: Vec<Plan>,
    pending: Option<Pending>,
    /// The observables of the run.
    graph: Graph,
    /// The times of a grid of the result, `None` for the ragged layout.
    grid: Option<Vec<f64>>,
    /// Whether the timecourses are interpolated onto the grid.
    interpolate: bool,
    /// The evaluation of the observables of the chunk which ran, and the failures of its
    /// points before it (by their flat index).
    observing: Option<(Observation, Vec<(usize, String)>)>,
    /// The outcome of every point of the block, by its position in the block.
    outcomes: Vec<Option<Result<PointOutput, String>>>,
    /// The first failed point in the order of the scan of a scan of `on_error: raise` and its
    /// failure, which fails the scan once no chunk which starts before it is left to run.
    first_failure: Option<(usize, String)>,
    failed: Option<ScanError>,
}

impl Scan {
    /// Plans a scan of a model, the model of the run: [`Scan::open_with_models`] without a
    /// dimension of models.
    ///
    /// # Errors
    ///
    /// See [`Scan::open_with_models`].
    pub fn open(request: ScanRequest, description: ModelDescription) -> Result<Scan, ScanError> {
        Scan::open_with_models(request, Some(description), Vec::new())
    }

    /// Plans a scan of its models: the model of the run (`description`), or, for a scan with a
    /// dimension of models, the model of every key of that dimension (`models`, the key and
    /// the description of its compiled model), which replaces the model of the run, as
    /// sbmlsim's `Simulator.run(None, scan)`.
    ///
    /// Every check which does not need a simulation runs here, so a request which does not fit
    /// a model fails before a point runs: the plan of every simulation on every model, the
    /// values and times of the dimensions, the selections, the request of the first point of
    /// every simulation and model (a formula the planner cannot fold fails every point alike),
    /// the models of a dimension against the first one (every selection, every dose target of
    /// a PK observable and their units), and the names of the result (a dimension, a
    /// coordinate, an observable and an output share no name).
    ///
    /// # Errors
    ///
    /// A request which does not fit itself or its models, or which the planner does not plan,
    /// see [`ScanErrorKind`].
    pub fn open_with_models(
        request: ScanRequest,
        description: Option<ModelDescription>,
        models: Vec<(String, ModelDescription)>,
    ) -> Result<Scan, ScanError> {
        if request.contract != SCAN_CONTRACT {
            return Err(ScanError::request(format!(
                "The contract of the request is '{}', not '{SCAN_CONTRACT}'.",
                request.contract
            )));
        }
        // the scan is checked once, here, before its models are resolved from its dimension of
        // models, as sbmlsim's `Scan` checks itself before a run reads it
        expand::check_scan(&request.scan)?;
        let settings: Vec<&str> = request.model_settings.keys().collect();
        let (infos, keys, labels) = resolve_models(
            &request.scan,
            &settings,
            &request.changes,
            &request.model_changes,
            description,
            models,
        )?;
        let changes = changes_of(&keys, &request.changes, &request.model_changes);
        let expanded = Expanded::new(&request.scan, &infos, &labels, &changes)?;
        let mut first_points = Vec::with_capacity(expanded.plans.len());
        for key in 0..expanded.plans.len() {
            let m = expanded.model_of_plan(key);
            let plan = expanded.point_plan(&expanded.first_point(key), &infos)?;
            // a formula which cannot be folded fails every point of its plan alike; the
            // selections do not matter to the check
            base_request(&plan, &infos[m], &[], settings_of(&request, &keys[m]))?;
            first_points.push(plan);
        }
        let graph = observables::compile(
            &request.observables,
            &request.selections,
            request.keep.as_deref(),
            &infos[0],
            &first_points,
        )?;
        let selections = columns_of(&graph.selections);
        let mut read: Vec<String> = selections.clone();
        read.extend(graph.doses());
        for (model, label) in infos.iter().zip(&labels).skip(1) {
            check_model(model, label, &infos[0], &read)?;
        }
        let mut clash: Vec<&str> = request
            .scan
            .dimensions
            .iter()
            .map(|d| d.id.as_str())
            .filter(|id| graph.outputs.iter().any(|s| s == id))
            .collect();
        clash.sort_unstable();
        if !clash.is_empty() {
            return Err(ScanError::request(format!(
                "The dimension ids {} are selections of the model or observables of the run: a \
                 dimension and a variable of the result share no name, choose other ids.",
                text::list(&clash)
            )));
        }
        let ids: Vec<&str> = request.observables.iter().map(Observable::id).collect();
        let mut changed: Vec<&str> = request
            .scan
            .dimensions
            .iter()
            .flat_map(|d| d.values.keys())
            .filter(|t| ids.contains(t))
            .collect();
        changed.sort_unstable();
        changed.dedup();
        if !changed.is_empty() {
            return Err(ScanError::request(format!(
                "The observables {} are targets the scan changes; choose other ids.",
                text::list(&changed)
            )));
        }
        let mut same: Vec<&str> = request
            .scan
            .dimensions
            .iter()
            .map(|d| d.id.as_str())
            .filter(|d| ids.contains(d))
            .collect();
        same.sort_unstable();
        if !same.is_empty() {
            return Err(ScanError::request(format!(
                "The observables {} have the id of a dimension of the scan; choose other ids.",
                text::list(&same)
            )));
        }
        check_coordinates(&request.scan.dimensions, &graph.outputs)?;
        let (grid, interpolate) = assemble::grid(request.time.as_ref(), &expanded, &infos[0])?;
        let block = block(request.points, expanded.size())?;
        let chunks = expanded.chunks(&block, CORE_MAX_CHUNK);
        Ok(Scan {
            outcomes: vec![None; block.len()],
            request,
            models: infos,
            keys,
            expanded,
            selections,
            block,
            chunks,
            next: 0,
            folds: None,
            plans: Vec::new(),
            pending: None,
            graph,
            grid,
            interpolate,
            observing: None,
            first_failure: None,
            failed: None,
        })
    }

    /// The request of the scan.
    pub fn request(&self) -> &ScanRequest {
        &self.request
    }

    /// The scan compiled against the model: the plan of every simulation and the values of the
    /// dimensions in the units of the model.
    pub fn expanded(&self) -> &Expanded {
        &self.expanded
    }

    /// The selections of the requests, `time` first.
    pub fn selections(&self) -> &[String] {
        &self.selections
    }

    /// The flat indices of the points which run.
    pub fn block(&self) -> Range<usize> {
        self.block.clone()
    }

    /// The plan of a point of the scan, by its flat index.
    ///
    /// # Errors
    ///
    /// An index outside of the scan.
    pub fn point_plan(&self, index: usize) -> Result<Plan, ScanError> {
        if index >= self.expanded.size() {
            return Err(ScanError::request(format!(
                "The point {index} is outside of the {} points of the scan.",
                self.expanded.size()
            )));
        }
        self.expanded
            .point_plan(&self.expanded.position(index), &self.models)
    }

    /// The model of the chunk of the next round, its index in the models of the scan.
    fn chunk_model(&self) -> usize {
        self.expanded
            .model_of(&self.expanded.position(self.chunks[self.next][0]))
    }

    /// The keys of the models of a dimension of models whose points have all run: no chunk of
    /// them is left from the chunk of the next (or the pending) round on, so a host may drop
    /// what it holds of them, e.g. the executor of their requests. Empty without a dimension of
    /// models; a model whose chunks are all left behind is listed until the scan is done.
    pub fn finished_models(&self) -> Vec<String> {
        let mut left = vec![false; self.models.len()];
        for chunk in self.chunks.iter().skip(self.next) {
            left[self.expanded.model_of(&self.expanded.position(chunk[0]))] = true;
        }
        self.keys
            .iter()
            .zip(left)
            .filter_map(|(key, left)| key.clone().filter(|_| !left))
            .collect()
    }

    /// The next round of the scan, see the crate. A round which was asked and not told is asked
    /// again; a scan which failed asks nothing.
    ///
    /// A chunk asks the rounds of the segments of its folds first, every point which still has
    /// a formula of the model to fold in each, then the round of its points. A round of
    /// simulations names the model of its requests ([`Ask::Simulate`]): the key of its model in
    /// the dimension of models, `None` for the model of the run; the chunks are in the order of
    /// their first point, so the rounds of the models of a dimension of models may alternate.
    pub fn ask(&mut self) -> Ask {
        loop {
            if self.failed.is_some() {
                return Ask::Done;
            }
            if let Some(question) = self.observing.as_ref().and_then(|(o, _)| o.question()) {
                return Ask::Observe(question.clone());
            }
            if let Some(pending) = &self.pending {
                return Ask::Simulate {
                    model: self.keys[pending.model].clone(),
                    requests: pending.requests.clone(),
                };
            }
            if self.next >= self.chunks.len() {
                return Ask::Done;
            }
            if self.folds.is_none() {
                match self.chunk_folds() {
                    Ok((plans, folds)) => {
                        self.plans = plans;
                        self.folds = Some(folds);
                    }
                    // a definition which does not fit the plan fails the scan, not the point;
                    // the checks of `open` leave none, so this is the guard of a planner error
                    Err(err) => {
                        self.failed = Some(err);
                        return Ask::Done;
                    }
                }
            }
            let m = self.chunk_model();
            let model = &self.models[m];
            let folds = self
                .folds
                .as_mut()
                .expect("the folds of the chunk are made");
            let mut positions = Vec::new();
            let mut requests = Vec::new();
            for (position, fold) in folds.iter_mut().enumerate() {
                if let Ok(fold) = fold
                    && let Some(segment) = fold.step(model)
                {
                    positions.push(position);
                    requests.push(segment);
                }
            }
            let kind = if requests.is_empty() {
                for (position, fold) in folds.iter().enumerate() {
                    if let Ok(fold) = fold {
                        positions.push(position);
                        requests.push(fold.request());
                    }
                }
                RoundKind::Points
            } else {
                RoundKind::Segments
            };
            if requests.is_empty() {
                // every point of the chunk failed in its fold: the engine is asked nothing
                if let Err(err) = self.points_ran(Vec::new()) {
                    self.failed = Some(err);
                }
                continue;
            }
            self.pending = Some(Pending {
                kind,
                positions,
                requests: requests.clone(),
                model: m,
            });
            return Ask::Simulate {
                model: self.keys[m].clone(),
                requests,
            };
        }
    }

    /// The plans and the folds of the points of the chunk of the next round, in their order.
    fn chunk_folds(&self) -> Result<(Vec<Plan>, Vec<PointFold>), ScanError> {
        let m = self.chunk_model();
        let settings = settings_of(&self.request, &self.keys[m]);
        let mut plans = Vec::new();
        let mut folds = Vec::new();
        for &index in &self.chunks[self.next] {
            let plan = self
                .expanded
                .point_plan(&self.expanded.position(index), &self.models)?;
            let request = base_request(&plan, &self.models[m], &self.selections, settings)?;
            folds.push(Ok(Fold::new(plan.clone(), request)?));
            plans.push(plan);
        }
        Ok((plans, folds))
    }

    /// Takes the output or the error of every request of the round [`Scan::ask`] gave, in the
    /// order of the requests.
    ///
    /// A point which failed, in the segment of its fold or in its own simulation, is recorded
    /// with the failure in the words of sbmlsim; with `on_error: raise` the scan fails, once
    /// the points of the chunk ran, with the first failed point of the chunk in the order of
    /// the scan ([`ScanErrorKind::Point`], the message naming its labels and values), and a
    /// point the engine does not support fails the scan whatever `on_error` says.
    ///
    /// # Errors
    ///
    /// No round asked, another number of answers than requests, or the failure of the scan.
    pub fn tell(
        &mut self,
        answers: Vec<Result<SimulationResult, PointError>>,
    ) -> Result<(), ScanError> {
        if let Some(err) = &self.failed {
            return Err(err.clone());
        }
        let Some(pending) = self.pending.take() else {
            return Err(ScanError::request(
                "The scan asked for no simulation: ask() before tell().",
            ));
        };
        if answers.len() != pending.requests.len() {
            let n = pending.requests.len();
            self.pending = Some(pending);
            return Err(ScanError::request(format!(
                "The scan asked for {n} simulations and was told {} answers.",
                answers.len()
            )));
        }
        let result = match pending.kind {
            RoundKind::Segments => self.take_segments(&pending.positions, pending.model, answers),
            RoundKind::Points => self.take_points(&pending.positions, pending.model, answers),
        };
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }

    /// Folds the answers of a round of segments into the folds of their points.
    fn take_segments(
        &mut self,
        positions: &[usize],
        m: usize,
        answers: Vec<Result<SimulationResult, PointError>>,
    ) -> Result<(), ScanError> {
        let model = &self.models[m];
        let folds = self.folds.as_mut().expect("a round of segments has folds");
        for (&position, answer) in positions.iter().zip(answers) {
            let fold = &mut folds[position];
            let Ok(running) = fold else {
                unreachable!("a failed fold asks for no segment");
            };
            let taken = match answer {
                Ok(output) => running.take(&output, model),
                Err(error) => Err(error::point_failure(&error, &model.label)?),
            };
            if let Err(text) = taken {
                *fold = Err(text);
            }
        }
        Ok(())
    }

    /// Records the answers of the round of the points of the chunk, evaluates its observables
    /// and finishes the chunk once they are evaluated.
    fn take_points(
        &mut self,
        positions: &[usize],
        m: usize,
        answers: Vec<Result<SimulationResult, PointError>>,
    ) -> Result<(), ScanError> {
        let mut outcomes = Vec::with_capacity(answers.len());
        for (&position, answer) in positions.iter().zip(answers) {
            let outcome = match answer {
                Ok(output) => Ok(output),
                Err(error) => Err(error::point_failure(&error, &self.models[m].label)?),
            };
            outcomes.push((position, outcome));
        }
        self.points_ran(outcomes)
    }

    /// The points of the chunk ran: without observables their outputs are its outcomes,
    /// with observables they are evaluated first.
    fn points_ran(
        &mut self,
        outcomes: Vec<(usize, Result<SimulationResult, String>)>,
    ) -> Result<(), ScanError> {
        let chunk = self.chunks[self.next].clone();
        let mut failures: Vec<(usize, String)> = self
            .folds
            .take()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .filter_map(|(position, fold)| fold.err().map(|text| (chunk[position], text)))
            .collect();
        let mut ran: Vec<PointRun> = Vec::new();
        let mut done: Vec<(usize, Result<PointOutput, String>)> = Vec::new();
        for (position, outcome) in outcomes {
            let index = chunk[position];
            let plan = self.plans[position].clone();
            match outcome {
                Err(text) => failures.push((index, text)),
                Ok(output) => {
                    let (time, mut columns) = self.columns_of(&output, &plan);
                    drop(output);
                    if self.graph.observes() {
                        ran.push(PointRun {
                            index,
                            plan,
                            time,
                            values: columns
                                .into_iter()
                                .map(|(name, xs)| (name, Val::Series(xs)))
                                .collect(),
                            failed: None,
                        });
                    } else {
                        let mut rows = vec![time];
                        for name in self.graph.timecourses() {
                            rows.push(columns.remove(&name).unwrap_or_default());
                        }
                        done.push((index, self.output_of(rows, Vec::new(), &plan)));
                    }
                }
            }
        }
        if !self.graph.observes() {
            done.extend(failures.into_iter().map(|(i, t)| (i, Err(t))));
            return self.finish_chunk(done);
        }
        let mut observation = Observation::new(ran);
        if observation.advance(&self.graph) {
            return self.observed(observation, failures);
        }
        self.observing = Some((observation, failures));
        Ok(())
    }

    /// The time and the columns of the output of a point, the time shifted, sbmlsim's
    /// `output_time`.
    fn columns_of(
        &self,
        output: &SimulationResult,
        plan: &Plan,
    ) -> (Vec<f64>, std::collections::HashMap<String, Vec<f64>>) {
        let n = output.columns.len();
        let column = |j: usize| -> Vec<f64> {
            (0..output.n_rows)
                .map(|r| output.values[r * n + j])
                .collect()
        };
        let mut time = column(0);
        if plan.time_shift != 0.0 {
            for t in &mut time {
                *t += plan.time_shift;
            }
        }
        let mut columns = std::collections::HashMap::new();
        for (j, name) in output.columns.iter().enumerate().skip(1) {
            columns.entry(name.clone()).or_insert_with(|| column(j));
        }
        (time, columns)
    }

    /// The output of a point: its rows (the time and the kept timecourses) on the grid of the
    /// run where it interpolates, and its values per simulation; a point whose output times are
    /// not the ones of the grid the simulations share fails.
    fn output_of(
        &self,
        rows: Vec<Vec<f64>>,
        scalars: Vec<f64>,
        plan: &Plan,
    ) -> Result<PointOutput, String> {
        let rows = match (&self.grid, self.interpolate) {
            (Some(grid), true) => interpolated(rows, grid, plan),
            (Some(grid), false) if rows[0].len() != grid.len() => {
                return Err(format!(
                    "RuntimeError: The output of the point has {} rows, not the {} output times \
                     the simulations of the scan share.",
                    rows[0].len(),
                    grid.len()
                ));
            }
            _ => rows,
        };
        Ok(PointOutput { rows, scalars })
    }

    /// The observables of the chunk are evaluated: the outputs of its points.
    fn observed(
        &mut self,
        observation: Observation,
        failures: Vec<(usize, String)>,
    ) -> Result<(), ScanError> {
        let timecourses = self.graph.timecourses();
        let scalars = self.graph.scalars();
        let mut done: Vec<(usize, Result<PointOutput, String>)> =
            failures.into_iter().map(|(i, t)| (i, Err(t))).collect();
        for point in observation.points {
            if let Some(text) = point.failed {
                done.push((point.index, Err(text)));
                continue;
            }
            let n = point.time.len();
            let mut rows = vec![point.time.clone()];
            for name in &timecourses {
                rows.push(match point.values.get(name) {
                    Some(Val::Series(xs)) => xs.clone(),
                    Some(Val::Scalar(x)) => vec![*x; n],
                    None => vec![f64::NAN; n],
                });
            }
            let values = scalars
                .iter()
                .map(|name| match point.values.get(name) {
                    Some(Val::Scalar(x)) => *x,
                    _ => f64::NAN,
                })
                .collect();
            let output = self.output_of(rows, values, &point.plan);
            done.push((point.index, output));
        }
        self.finish_chunk(done)
    }

    /// Records the outcome of every point of the chunk and goes on to the next chunk.
    ///
    /// With `on_error: raise` a chunk with a failed point records the first of them, and the
    /// scan fails with the first failed point in the order of the scan once no chunk which
    /// starts before it is left: the chunks are in the order of their first point, and one of
    /// another simulation or model which starts before the failed point may hold a point before
    /// it which fails as well, as sbmlsim's `_run_serial` runs them.
    fn finish_chunk(
        &mut self,
        outcomes: Vec<(usize, Result<PointOutput, String>)>,
    ) -> Result<(), ScanError> {
        let first = outcomes
            .iter()
            .filter_map(|(index, outcome)| outcome.as_ref().err().map(|text| (*index, text)))
            .min_by_key(|(index, _)| *index);
        match first {
            Some((index, text)) if self.request.on_error == OnError::Raise => {
                if self
                    .first_failure
                    .as_ref()
                    .is_none_or(|(failed, _)| index < *failed)
                {
                    self.first_failure = Some((index, text.clone()));
                }
            }
            // under `raise` the outcomes of a chunk with a failed point are dropped on purpose:
            // the scan fails, and gives no result of them
            _ => {
                for (index, outcome) in outcomes {
                    self.outcomes[index - self.block.start] = Some(outcome);
                }
            }
        }
        self.plans.clear();
        self.next += 1;
        if let Some((index, text)) = &self.first_failure {
            let before = self.chunks.get(self.next).is_some_and(|c| c[0] < *index);
            if !before {
                return Err(self.point_error(*index, text));
            }
        }
        Ok(())
    }

    /// Takes the values of the observables of the host which [`Scan::ask`] asked for, in the
    /// order of its points; a point whose function failed (an entry of
    /// [`HostObserved::errors`]) fails.
    ///
    /// # Errors
    ///
    /// No observation asked, an answer which does not fit the question, or the failure of the
    /// scan.
    pub fn tell_observed(&mut self, values: HostObserved) -> Result<(), ScanError> {
        if let Some(err) = &self.failed {
            return Err(err.clone());
        }
        let Some((mut observation, failures)) = self.observing.take() else {
            return Err(ScanError::request(
                "The scan asked for no observables: ask() before tell_observed().",
            ));
        };
        if let Err(err) = observation.take(&self.graph, values) {
            self.observing = Some((observation, failures));
            return Err(err);
        }
        if !observation.advance(&self.graph) {
            self.observing = Some((observation, failures));
            return Ok(());
        }
        let result = self.observed(observation, failures);
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }

    /// The points of the block which ran (or failed) and the points of the block: a host
    /// reports its progress with them after every round it told.
    pub fn progress(&self) -> (usize, usize) {
        let done = self.outcomes.iter().filter(|o| o.is_some()).count();
        (done, self.outcomes.len())
    }

    /// Whether [`Scan::result`] assembles a result: `Ok` once the scan is done, the failure of
    /// a scan which failed, or the error of a scan which is not done yet. A host checks it
    /// before it gives the scan up to [`Scan::result`], which takes the scan.
    ///
    /// # Errors
    ///
    /// The failure of the scan, or a scan which is not done.
    pub fn finished(&self) -> Result<(), ScanError> {
        if let Some(err) = &self.failed {
            return Err(err.clone());
        }
        if self.pending.is_some() || self.observing.is_some() || self.next < self.chunks.len() {
            return Err(ScanError::request(
                "The scan is not done: ask() and tell() until ask() says it is done.",
            ));
        }
        Ok(())
    }

    /// The result of the scan once [`Scan::ask`] says it is done.
    ///
    /// # Errors
    ///
    /// The failure of the scan, or a scan which is not done.
    pub fn result(self) -> Result<ScanResult, ScanError> {
        self.finished()?;
        let failed: Vec<result::FailedPoint> = self
            .block
            .clone()
            .zip(&self.outcomes)
            .filter_map(|(index, outcome)| match outcome {
                Some(Err(message)) => Some(result::FailedPoint {
                    index,
                    point: self.point_text(index),
                    message: message.clone(),
                }),
                _ => None,
            })
            .take(MAX_ERRORS)
            .collect();
        let outputs: Vec<Option<PointOutput>> = self
            .outcomes
            .into_iter()
            .map(|o| o.and_then(Result::ok))
            .collect();
        Ok(assemble::assemble(
            &assemble::Assembly {
                request: &self.request,
                model: &self.models[0],
                expanded: &self.expanded,
                graph: &self.graph,
                grid: self.grid.as_deref(),
                block: self.block.clone(),
                errors: failed,
            },
            outputs,
        ))
    }

    /// The error of a failed point with its labels and values, sbmlsim's `_Compiled.error`.
    fn point_error(&self, index: usize, message: &str) -> ScanError {
        let text = self.point_text(index);
        let place = if text.is_empty() {
            "The simulation".to_string()
        } else {
            format!("The point {text} of the scan")
        };
        ScanError::point(index, format!("{place} failed: {message}"))
    }

    /// The labels and the values of a point, sbmlsim's `_Compiled.point_text`.
    pub(crate) fn point_text(&self, index: usize) -> String {
        let position = self.expanded.position(index);
        let mut parts = Vec::new();
        for (dimension, &k) in self.request.scan.dimensions.iter().zip(&position) {
            parts.push(format!(
                "{}={}",
                dimension.id,
                text::label(&dimension.labels[k])
            ));
            for (target, values) in dimension.values.iter() {
                let unit = values.unit().map(|u| format!(" {u}")).unwrap_or_default();
                let value = text::number(values.magnitudes()[k]);
                parts.push(format!("{target}={value}{unit}"));
            }
        }
        parts.join(", ")
    }
}

/// The models of a scan, their keys (`None` for the model of the run) and their labels in
/// messages (`""` for the model of the run).
pub(crate) type Models = (Vec<ModelInfo>, Vec<Option<String>>, Vec<String>);

/// The changes of every model of a scan by its key, the defaults of the pre-initialization of
/// its simulations (empty for the model of the run, whose changes the host applies).
pub(crate) fn changes_of(
    keys: &[Option<String>],
    changes: &Ordered<Value>,
    model_changes: &Ordered<Ordered<Value>>,
) -> Vec<Ordered<Value>> {
    keys.iter()
        .map(|key| match key.as_deref() {
            None => changes.clone(),
            Some(key) => model_changes.get(key).cloned().unwrap_or_default(),
        })
        .collect()
}

/// The models of a scan with their keys and their labels in messages, sbmlsim's
/// `Simulator._models`: the model of the run (no key, no label), or the model of every key of
/// the dimension of models in the order of its points (labelled by the labels of the
/// dimension), which replaces the model of the run.
///
/// # Errors
///
/// Neither a model of the run nor a dimension of models, or both; models of the host which are
/// not the ones of the dimension; settings (the keys of `model_settings`) or changes of a model
/// which is none of the dimension; changes of the model of the run (`changes`) with a
/// dimension of models, which replaces it.
pub(crate) fn resolve_models(
    scan: &ScanDefinition,
    model_settings: &[&str],
    changes: &Ordered<Value>,
    model_changes: &Ordered<Ordered<Value>>,
    description: Option<ModelDescription>,
    mut models: Vec<(String, ModelDescription)>,
) -> Result<Models, ScanError> {
    let dimension = scan
        .dimensions
        .iter()
        .find(|d| d.kind == DimensionKind::Models);
    let given = |models: &[(String, ModelDescription)]| -> Vec<String> {
        models.iter().map(|(key, _)| key.clone()).collect()
    };
    let resolved = match dimension {
        None => {
            if !models.is_empty() {
                return Err(ScanError::request(format!(
                    "The scan has no dimension of models, the host gave the models {}.",
                    text::list(&given(&models))
                )));
            }
            let Some(description) = description else {
                return Err(ScanError::request(
                    "The run needs a model: give the model of the run or a dimension of models.",
                ));
            };
            (
                vec![ModelInfo::new(&description)?],
                vec![None],
                vec![String::new()],
            )
        }
        Some(dimension) => {
            if description.is_some() {
                return Err(ScanError::request(format!(
                    "The dimension of models '{}' replaces the model of the run: give `None` as \
                     the model.",
                    dimension.id
                )));
            }
            if !changes.is_empty() {
                return Err(ScanError::request(format!(
                    "The dimension of models '{}' replaces the model of the run: give the \
                     changes of its models as `model_changes`, not `changes`.",
                    dimension.id
                )));
            }
            let wanted: Vec<&str> = dimension.models.keys().collect();
            let gave = given(&models);
            let mut infos = Vec::with_capacity(wanted.len());
            for key in &wanted {
                if let Some(k) = models.iter().position(|(k, _)| k == key) {
                    infos.push(ModelInfo::new(&models.swap_remove(k).1)?);
                }
            }
            if infos.len() != wanted.len() || !models.is_empty() {
                return Err(ScanError::request(format!(
                    "The dimension of models '{}' has the models {}, the host gave the models \
                     {}.",
                    dimension.id,
                    text::list(&wanted),
                    text::list(&gave)
                )));
            }
            (
                infos,
                wanted.iter().map(|key| Some((*key).to_string())).collect(),
                dimension.labels.iter().map(text::label).collect(),
            )
        }
    };
    let keys = &resolved.1;
    for (what, mut unknown) in [
        ("settings", model_settings.to_vec()),
        ("changes", model_changes.keys().collect::<Vec<_>>()),
    ] {
        unknown.retain(|key| !keys.iter().flatten().any(|k| k == key));
        unknown.sort_unstable();
        if !unknown.is_empty() {
            return Err(ScanError::request(format!(
                "The {what} of the models {} are of no model of the dimension of models.",
                text::list(&unknown)
            )));
        }
    }
    Ok(resolved)
}

/// The settings of the requests of a model by its key: its own `model_settings`, else the
/// `settings` of the request.
fn settings_of<'r>(request: &'r ScanRequest, key: &Option<String>) -> &'r IntegratorSettings {
    key.as_deref()
        .and_then(|key| request.model_settings.get(key))
        .unwrap_or(&request.settings)
}

/// Checks that a model of a dimension of models has the selections and the units of the first
/// one, sbmlsim's `_check_model`: `selections` are the selections of the requests (`time`
/// first) and the dose targets of the PK observables.
///
/// # Errors
///
/// A selection the model has not, or another unit of a selection.
fn check_model(
    model: &ModelInfo,
    label: &str,
    first: &ModelInfo,
    selections: &[String],
) -> Result<(), ScanError> {
    let mut names: Vec<&str> = Vec::new();
    for s in selections {
        if !names.contains(&s.as_str()) {
            names.push(s);
        }
    }
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|s| !model.has_selection(s))
        .collect();
    if !missing.is_empty() {
        return Err(ScanError::request(format!(
            "The model '{label}' has not every selection of the first model of the dimension, it \
             has no selection {}.",
            text::list(&missing)
        )));
    }
    for name in names {
        let unit = model.unit(name).unwrap_or("");
        let expected = first.unit(name).unwrap_or("");
        if unit != expected {
            return Err(ScanError::units(format!(
                "The models of the dimension have different units of '{name}': '{expected}' and \
                 '{unit}' of the model '{label}'."
            )));
        }
    }
    Ok(())
}

/// Checks that no coordinate hides or is hidden by another name of the result, sbmlsim's
/// `_check_coordinates`: a changed target which is an output or a name of the result is the
/// coordinate `<dimension>.<target>`, which must not be a dimension id or an output, and a
/// covariate must not be a name of the result, an output, a dimension id, a changed target or
/// a covariate of another dimension.
///
/// # Errors
///
/// A coordinate of the name of another one.
fn check_coordinates(
    dimensions: &[DimensionDefinition],
    outputs: &[String],
) -> Result<(), ScanError> {
    let ids: Vec<&str> = dimensions.iter().map(|d| d.id.as_str()).collect();
    let output = |name: &str| outputs.iter().any(|o| o == name);
    let mut taken: Vec<String> = Vec::new();
    for dimension in dimensions {
        for target in dimension.values.keys() {
            taken.push(target.to_string());
            if output(target) || RESERVED.contains(&target) {
                let qualified = format!("{}.{target}", dimension.id);
                if ids.contains(&qualified.as_str()) || output(&qualified) {
                    return Err(ScanError::request(format!(
                        "The coordinate '{qualified}' of the changed target '{target}' would hide \
                         a dimension or an output of the same name; rename the dimension '{}'.",
                        dimension.id
                    )));
                }
                taken.push(qualified);
            }
        }
    }
    let mut owners: Vec<(&str, &str)> = Vec::new();
    for dimension in dimensions {
        for name in dimension.coordinates.keys() {
            let what = if RESERVED.contains(&name) {
                "a name of the result".to_string()
            } else if output(name) {
                "an output of the result".to_string()
            } else if ids.contains(&name) {
                "a dimension id".to_string()
            } else if taken.iter().any(|t| t == name) {
                "a changed target".to_string()
            } else if let Some((_, owner)) = owners.iter().find(|(n, _)| *n == name) {
                format!("a coordinate of the dimension '{owner}'")
            } else {
                owners.push((name, &dimension.id));
                continue;
            };
            return Err(ScanError::request(format!(
                "The coordinate '{name}' of the dimension '{}' would hide or be hidden by {what} \
                 of the same name; rename the covariate.",
                dimension.id
            )));
        }
    }
    Ok(())
}

/// The block of the points of a run, every point of the scan without one: sbmlsim's `_block`.
fn block(points: Option<[usize; 2]>, size: usize) -> Result<Range<usize>, ScanError> {
    let Some([start, stop]) = points else {
        return Ok(0..size);
    };
    if stop <= start {
        return Err(ScanError::request(format!(
            "The points range({start}, {stop}) of a run are empty."
        )));
    }
    if stop > size {
        return Err(ScanError::request(format!(
            "The points range({start}, {stop}) of a run lie in the points range(0, {size}) of \
             the scan."
        )));
    }
    Ok(start..stop)
}

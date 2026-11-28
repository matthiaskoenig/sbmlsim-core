//! The fold of the changes of a plan into the numbers of its request, sbmlsim's `_Fold` of
//! `executor_core.py` (`_segment_request`, `_segment_output`, `_state_of`, `_resume_values`,
//! `_change_values`, `probe_before_events`).
//!
//! A formula at a change is evaluated by sbmlsim's executor of roadrunner with the state of the
//! model at the time of the change, which a request of the engine carries no formula of: the
//! planner asks the engine for the values the formula reads over the segment which ends at the
//! change, with a request of its own, and sends the number. The changes are folded in the
//! order of their times, so the request of a segment carries the changes before it, already
//! folded, and resumes from the state of the segment before it.
//!
//! The fold is a step at a time, so the folds of the points of a chunk run as one batch per
//! step: [`Fold::step`] gives the request of the next segment, which the host simulates,
//! [`Fold::take`] folds the change that segment was simulated for, and [`Fold::request`] is the
//! request of the plan once `step` gives nothing. A change of numbers and of formulas of the
//! time alone is folded by `step` itself, so a plan without a formula of the model asks for no
//! segment.
//!
//! A linear formula is folded as every other one: the run of a scan of sbmlsim writes no
//! `forms` of `simulation/1`, which only its fit asks for.

use std::collections::{BTreeSet, HashMap, HashSet};

use sbmlsim_abi::{Change, Output, SimulationRequest, SimulationResult};

use crate::error::ScanError;
use crate::formula::CompiledFormula;
use crate::model::ModelInfo;
use crate::plan::{Assigned, OutputMode, Plan, PlanEvent, time_tolerance};
use crate::requests::output_columns;

/// The changes of one plan, folded into the numbers of its request, see the module.
#[derive(Debug, Clone)]
pub(crate) struct Fold {
    plan: Plan,
    /// The request of the plan without its changes.
    request: SimulationRequest,
    /// The compiled formula of every assignment of every event, `None` for a number.
    formulas: Vec<Vec<Option<CompiledFormula>>>,
    /// The symbols of the model every change reads, sorted, by the position of its event:
    /// the state is read only where a formula reads it.
    reads: Vec<Vec<String>>,
    /// The changes which are folded, in their order.
    changes: Vec<Change>,
    /// The event which is folded next, the one a pending segment ends at.
    index: usize,
    /// The time of the last segment and the state after its change, which the next segment
    /// resumes from.
    resume: Option<(f64, Vec<(String, f64)>)>,
    /// The selections of the pending segment.
    columns: Vec<String>,
}

impl Fold {
    /// The fold of a plan whose request without changes is `request`.
    ///
    /// # Errors
    ///
    /// A formula which is no math of PEtab.
    pub fn new(plan: Plan, request: SimulationRequest) -> Result<Fold, ScanError> {
        let mut formulas = Vec::with_capacity(plan.events.len());
        let mut reads = Vec::with_capacity(plan.events.len());
        for event in &plan.events {
            let mut compiled = Vec::with_capacity(event.assignments.len());
            let mut symbols = BTreeSet::new();
            for a in &event.assignments {
                match &a.value {
                    Assigned::Number(_) => compiled.push(None),
                    Assigned::Formula(text) => {
                        let formula = CompiledFormula::new(text).map_err(ScanError::request)?;
                        symbols.extend(formula.symbols().iter().cloned());
                        compiled.push(Some(formula));
                    }
                }
            }
            formulas.push(compiled);
            reads.push(symbols.into_iter().collect());
        }
        Ok(Fold {
            plan,
            request,
            formulas,
            reads,
            changes: Vec::new(),
            index: 0,
            resume: None,
            columns: Vec::new(),
        })
    }

    /// The request of the next segment of the fold, `None` once every change is folded.
    ///
    /// Every change until the next one which reads the model is folded here.
    pub fn step(&mut self, model: &ModelInfo) -> Option<SimulationRequest> {
        while self.index < self.plan.events.len() {
            let event = &self.plan.events[self.index];
            if !self.reads[self.index].is_empty() {
                let mut selections: BTreeSet<&str> =
                    self.reads[self.index].iter().map(String::as_str).collect();
                if let Some(targets) = self.resumed(model) {
                    selections.extend(targets.iter().map(String::as_str));
                }
                let selections: Vec<String> = selections.into_iter().map(String::from).collect();
                self.columns = output_columns(&selections);
                return Some(self.segment(event.time, model));
            }
            let values = change_values(event, &self.formulas[self.index], &HashMap::new());
            self.changes.push(Change {
                time: event.time,
                values,
                ..Change::default()
            });
            self.index += 1;
        }
        None
    }

    /// Folds the change the segment of [`Fold::step`] was simulated for.
    ///
    /// # Errors
    ///
    /// An output without the state at the change, the failure of the point in words.
    pub fn take(&mut self, output: &SimulationResult, model: &ModelInfo) -> Result<(), String> {
        let event = &self.plan.events[self.index];
        let start = self.resume.as_ref().map_or(self.plan.start, |r| r.0);
        let state = state_of(output, event.time, start)?;
        for symbol in &self.columns {
            if !state.contains_key(symbol.as_str()) {
                return Err(format!(
                    "KeyError: The output of the segment which ends at the change at the time \
                     {} has no column '{symbol}'.",
                    crate::text::number(event.time)
                ));
            }
        }
        let values = change_values(event, &self.formulas[self.index], &state);
        if let Some(targets) = self.resumed(model) {
            self.resume = Some((event.time, resume_values(targets, model, &state, &values)));
        }
        self.changes.push(Change {
            time: event.time,
            values,
            ..Change::default()
        });
        self.index += 1;
        Ok(())
    }

    /// The request of the plan, its changes folded; call it once [`Fold::step`] gives nothing.
    pub fn request(&self) -> SimulationRequest {
        debug_assert!(self.index == self.plan.events.len(), "a fold is done");
        SimulationRequest {
            changes: self.changes.clone(),
            ..self.request.clone()
        }
    }

    /// The targets the next segment resumes from, `None` when no later change reads the state
    /// or the model has a state the targets do not cover.
    fn resumed<'m>(&self, model: &'m ModelInfo) -> Option<&'m [String]> {
        if self.reads[self.index + 1..].iter().any(|r| !r.is_empty()) {
            model.resume_targets()
        } else {
            None
        }
    }

    /// The request which reads the state of the model at the time of a change, sbmlsim's
    /// `_segment_request`.
    ///
    /// The segment ends at the change and carries no steady state after the end. Its output
    /// at the end is the state before the events of that time for a model with events
    /// (`before_events`), the state sbmlsim's executor of roadrunner applies a change to. The
    /// first segment starts at the start of the plan with its pre-initialization and its
    /// presimulation; a later one resumes from the state after the change before it, which a
    /// change at its start sets, keeps the pre-initialization of the plan and of its steady
    /// state (so every value which is no state is the one of the plan) and runs no
    /// presimulation.
    fn segment(&self, time: f64, model: &ModelInfo) -> SimulationRequest {
        let start = self.resume.as_ref().map_or(self.plan.start, |r| r.0);
        let mut segment = SimulationRequest {
            start,
            end: time,
            output: segment_output(&self.plan, start, time),
            selections: self.columns.clone(),
            before_events: model.has_events(),
            steady_state_after: None,
            ..self.request.clone()
        };
        match &self.resume {
            None => segment.changes = self.changes.clone(),
            Some((at, state)) => {
                if let Some(steady) = segment.steady_state.take() {
                    segment.preinit.extend(steady.preinit);
                }
                segment.changes = std::iter::once(Change {
                    time: *at,
                    values: state.clone(),
                    ..Change::default()
                })
                .chain(self.changes.iter().filter(|c| c.time > *at).cloned())
                .collect();
            }
        }
        segment
    }
}

/// The output of the segment of a change from a start to its time, sbmlsim's
/// `_segment_output`: the steps of the integrator, or the output times of the plan inside the
/// segment and its end, an output time within the resolution of the time of the change left
/// out.
fn segment_output(plan: &Plan, start: f64, time: f64) -> Output {
    match plan.output {
        OutputMode::Integrator => Output::IntegratorSteps,
        OutputMode::Times => {
            let tolerance = time_tolerance(plan.start, plan.end);
            let mut times: Vec<f64> = plan
                .times
                .iter()
                .copied()
                .filter(|&t| start <= t && t < time - tolerance)
                .collect();
            times.push(time);
            Output::Times(times)
        }
    }
}

/// The state at the time of a change, sbmlsim's `_state_of`: the last row of the output of the
/// segment, the first for a change at the start of the segment, by column (a later column of
/// the same name wins, as a dictionary of python keeps it).
fn state_of(
    output: &SimulationResult,
    time: f64,
    start: f64,
) -> Result<HashMap<&str, f64>, String> {
    let width = output.columns.len();
    if output.n_rows == 0 || output.values.len() != output.n_rows * width {
        return Err(format!(
            "IndexError: The output of the segment which ends at the change at the time {} has \
             no row of its state.",
            crate::text::number(time)
        ));
    }
    let row = if time > start { output.n_rows - 1 } else { 0 };
    let values = &output.values[row * width..(row + 1) * width];
    Ok(output
        .columns
        .iter()
        .map(String::as_str)
        .zip(values.iter().copied())
        .collect())
}

/// The values of a change, sbmlsim's `_change_values`: a number as it is, a formula evaluated
/// with the state before the change and its time.
fn change_values(
    event: &PlanEvent,
    formulas: &[Option<CompiledFormula>],
    state: &HashMap<&str, f64>,
) -> Vec<(String, f64)> {
    event
        .assignments
        .iter()
        .zip(formulas)
        .map(|(a, formula)| {
            let value = match (&a.value, formula) {
                (Assigned::Number(x), _) => *x,
                (Assigned::Formula(_), Some(formula)) => {
                    let values: Vec<f64> = formula
                        .symbols()
                        .iter()
                        .map(|s| state[s.as_str()])
                        .collect();
                    formula.evaluate(&values, event.time)
                }
                (Assigned::Formula(_), None) => unreachable!("every formula is compiled"),
            };
            (a.target.clone(), value)
        })
        .collect()
}

/// The values the next segment resumes from, sbmlsim's `_resume_values`: the state at the time
/// of the change of every target the change does not set (matched by its entity, so a change
/// of `[S]` replaces the amount `S`), then the values of the change. A target whose value is
/// not finite is left out where it is no state: its value is one of the model, which the
/// initialization of the segment computes again from the pre-initialization it keeps.
fn resume_values(
    targets: &[String],
    model: &ModelInfo,
    state: &HashMap<&str, f64>,
    values: &[(String, f64)],
) -> Vec<(String, f64)> {
    let assigned: HashSet<&str> = values.iter().map(|(t, _)| ModelInfo::entity(t)).collect();
    targets
        .iter()
        .filter(|t| !assigned.contains(ModelInfo::entity(t)))
        .filter_map(|t| {
            let value = state[t.as_str()];
            (model.is_state(t) || value.is_finite()).then(|| (t.clone(), value))
        })
        .chain(values.iter().cloned())
        .collect()
}

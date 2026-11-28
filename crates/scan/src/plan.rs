//! A simulation compiled against a model, the plan of its requests: sbmlsim's
//! `simulator/plan.py` (`compile_simulation`, `merge_times`, `Plan.with_values`).
//!
//! Every time is in the time unit of the model and every quantity in the unit of its target,
//! the kind of every target is resolved, the changes are merged into one event per time and
//! the output times are built. A formula stays text; the requests fold it.

use crate::error::ScanError;
use crate::formula::CompiledFormula;
use crate::model::{ModelInfo, TargetKind};
use crate::request::{SimulationDefinition, SteadyStateDefinition, Time, Value};
use crate::text;
use crate::units::{Converter, in_model_unit};

/// Output times closer than `TIME_RTOL` times the time scale are one output time.
pub const TIME_RTOL: f64 = 1e-9;

/// The tolerances of the steady state after the end of a simulation, the output at the time
/// `inf`: sbmlsim's default `SteadyState()`.
pub const STEADY_STATE: SteadyStatePlan = SteadyStatePlan {
    preinit: Vec::new(),
    absolute_tolerance: 1e-8,
    relative_tolerance: 1e-6,
    max_time: 1e8,
};

/// The resolution of the time of a simulation, `TIME_RTOL * max(|start|, |end|, end - start)`:
/// two times which differ by less are one time.
pub fn time_tolerance(start: f64, end: f64) -> f64 {
    TIME_RTOL * start.abs().max(end.abs()).max(end - start)
}

/// Merges output times which differ by less than the resolution of the time, sbmlsim's
/// `merge_times`: a cluster of times whose consecutive distances are within it becomes its
/// smallest time inside `[start, end]`, else its smallest time; a time at `inf` stays one of
/// its own.
///
/// Returns the merged, sorted times and, for every given time, the index of its merged time.
pub fn merge_times(times: &[f64], start: f64, end: f64) -> (Vec<f64>, Vec<usize>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let tolerance = time_tolerance(start, end);
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|&a, &b| times[a].total_cmp(&times[b]));
    let ordered: Vec<f64> = order.iter().map(|&k| times[k]).collect();
    let mut merged = Vec::new();
    let mut cluster_of = vec![0; times.len()];
    let mut first = 0;
    while first < ordered.len() {
        let mut last = first;
        while last + 1 < ordered.len() {
            let gap = ordered[last + 1] - ordered[last];
            // inf - inf is NaN, which is no gap: the times at inf are one cluster
            let gap = if gap.is_nan() { 0.0 } else { gap };
            if gap > tolerance {
                break;
            }
            last += 1;
        }
        let cluster = &ordered[first..=last];
        let inside = cluster.iter().find(|t| start <= **t && **t <= end);
        merged.push(*inside.unwrap_or(&cluster[0]));
        for &k in &order[first..=last] {
            cluster_of[k] = merged.len() - 1;
        }
        first = last + 1;
    }
    (merged, cluster_of)
}

/// The points of `numpy.linspace(start, end, steps + 1)`, operation for operation: `k * step
/// + start` with `step = (end - start) / steps`, the last point `end`.
pub fn linspace(start: f64, end: f64, steps: u64) -> Vec<f64> {
    let div = steps as f64;
    let delta = end - start;
    let step = delta / div;
    let mut points: Vec<f64> = (0..=steps)
        .map(|k| {
            let k = k as f64;
            if step == 0.0 {
                k / div * delta + start
            } else {
                k * step + start
            }
        })
        .collect();
    if let Some(last) = points.last_mut() {
        *last = end;
    }
    points
}

/// The value of one target, a number in the unit of the model or a formula.
#[derive(Debug, Clone, PartialEq)]
pub enum Assigned {
    /// A number.
    Number(f64),
    /// A formula of the math of PEtab.
    Formula(String),
}

/// The value of one target.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// The target, `S`, `[S]` or the id of a parameter or a compartment.
    pub target: String,
    /// The kind of the target.
    pub kind: TargetKind,
    /// The value.
    pub value: Assigned,
}

impl Assignment {
    /// The number of the assignment, `None` for a formula.
    pub fn number(&self) -> Option<f64> {
        match self.value {
            Assigned::Number(x) => Some(x),
            Assigned::Formula(_) => None,
        }
    }
}

/// The assignments at one time.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanEvent {
    /// The time in the time unit of the model.
    pub time: f64,
    /// The assignments, evaluated together and then applied.
    pub assignments: Vec<Assignment>,
}

/// The output of a simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// The steps of the integrator.
    Integrator,
    /// Exact output times.
    Times,
}

/// A pre-equilibration of a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SteadyStatePlan {
    /// The pre-initialization assignments of the steady state.
    pub preinit: Vec<Assignment>,
    /// The absolute tolerance of the rates of change.
    pub absolute_tolerance: f64,
    /// The relative tolerance of the rates of change.
    pub relative_tolerance: f64,
    /// The longest pre-equilibration, in the time unit of the model.
    pub max_time: f64,
}

/// A simulation compiled against a model, see the module.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The start, in the time unit of the model.
    pub start: f64,
    /// The end.
    pub end: f64,
    /// The assignments before the initialization.
    pub preinit: Vec<Assignment>,
    /// The assignments at times in `[start, end]`, sorted by time.
    pub events: Vec<PlanEvent>,
    /// The pre-equilibration.
    pub steady_state: Option<SteadyStatePlan>,
    /// The output.
    pub output: OutputMode,
    /// The output times of [`OutputMode::Times`], empty otherwise.
    pub times: Vec<f64>,
    /// Added to the time of the result.
    pub time_shift: f64,
    /// The steady state after the end, the output at the time `inf`.
    pub steady_state_output: Option<SteadyStatePlan>,
}

impl Plan {
    /// The targets the plan assigns, before the initialization, in the steady state or at a
    /// time.
    pub fn assigns(&self, target: &str) -> bool {
        let has = |assignments: &[Assignment]| assignments.iter().any(|a| a.target == target);
        has(&self.preinit)
            || self.steady_state.as_ref().is_some_and(|s| has(&s.preinit))
            || self.events.iter().any(|e| has(&e.assignments))
    }

    /// The times of the changes after the start, sorted.
    pub fn change_times(&self) -> Vec<f64> {
        self.events
            .iter()
            .map(|e| e.time)
            .filter(|t| *t > self.start)
            .collect()
    }

    /// The plan with other values of targets, sbmlsim's `Plan.with_values`.
    ///
    /// A value replaces the assignment of its target wherever the plan has one (before the
    /// initialization, in the steady state and at every time) and is added to the assignments
    /// before the initialization of a target the plan does not set. With `at` the values are a
    /// change at that time instead, joining the event of that time, where they replace the
    /// assignment of a target it has, or a new event.
    ///
    /// # Errors
    ///
    /// A target which is no target of the model, or `at` outside of the simulation.
    pub fn with_values(
        &self,
        values: &[(String, f64)],
        at: Option<f64>,
        model: &ModelInfo,
    ) -> Result<Plan, ScanError> {
        if values.is_empty() {
            return Ok(self.clone());
        }
        if let Some(at) = at {
            return self.with_change(values, at, model);
        }
        let replaced = |assignments: &[Assignment]| -> Vec<Assignment> {
            assignments
                .iter()
                .map(|a| match values.iter().find(|(t, _)| *t == a.target) {
                    Some((_, v)) => Assignment {
                        target: a.target.clone(),
                        kind: a.kind,
                        value: Assigned::Number(*v),
                    },
                    None => a.clone(),
                })
                .collect()
        };
        let mut plan = self.clone();
        plan.events = self
            .events
            .iter()
            .map(|e| PlanEvent {
                time: e.time,
                assignments: replaced(&e.assignments),
            })
            .collect();
        if let Some(steady) = &mut plan.steady_state {
            steady.preinit = replaced(&steady.preinit);
        }
        plan.preinit = replaced(&self.preinit);
        for (target, value) in values {
            if self.assigns(target) {
                continue;
            }
            plan.preinit.push(Assignment {
                target: target.clone(),
                kind: model.kind(target)?,
                value: Assigned::Number(*value),
            });
        }
        Ok(plan)
    }

    /// The plan with values as a change at a time, sbmlsim's `Plan._with_change`.
    fn with_change(
        &self,
        values: &[(String, f64)],
        at: f64,
        model: &ModelInfo,
    ) -> Result<Plan, ScanError> {
        if !(self.start <= at && at <= self.end) {
            let mut targets: Vec<&str> = values.iter().map(|(t, _)| t.as_str()).collect();
            targets.sort_unstable();
            return Err(ScanError::request(format!(
                "The change of {} at the time {} is outside of the simulation [{}, {}] in the \
                 time unit of the model.",
                text::list(&targets),
                text::number(at),
                text::number(self.start),
                text::number(self.end)
            )));
        }
        let added = values
            .iter()
            .map(|(target, value)| {
                Ok(Assignment {
                    target: target.clone(),
                    kind: model.kind(target)?,
                    value: Assigned::Number(*value),
                })
            })
            .collect::<Result<Vec<_>, ScanError>>()?;
        let mut plan = self.clone();
        let mut merged = false;
        for event in &mut plan.events {
            if event.time == at {
                event
                    .assignments
                    .retain(|a| !values.iter().any(|(t, _)| *t == a.target));
                event.assignments.extend(added.iter().cloned());
                merged = true;
            }
        }
        if !merged {
            plan.events.push(PlanEvent {
                time: at,
                assignments: added,
            });
            // a stable sort, as python's
            plan.events.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        Ok(plan)
    }

    /// The times of the result, an output time at a change after the start twice, shifted by
    /// `time_shift`, with `inf` last for the steady state after the end; `None` for the steps
    /// of the integrator. `changes` are the times of further changes, of a dimension with `at`.
    pub fn output_times(&self, changes: &[f64]) -> Option<Vec<f64>> {
        if self.output != OutputMode::Times {
            return None;
        }
        let mut at = self.change_times();
        at.extend(changes.iter().copied().filter(|t| *t > self.start));
        let mut times = Vec::new();
        for &time in &self.times {
            times.push(time + self.time_shift);
            if at.contains(&time) {
                times.push(time + self.time_shift);
            }
        }
        if self.steady_state_output.is_some() {
            times.push(f64::INFINITY);
        }
        Some(times)
    }
}

/// Converts the values of a simulation into the units of a model.
struct Compiler<'m> {
    model: &'m ModelInfo,
    convert: Converter<'m>,
}

impl Compiler<'_> {
    /// The assignment of a value to a target, sbmlsim's `_Converter.assignment`.
    fn assignment(&self, target: &str, value: &Value) -> Result<Assignment, ScanError> {
        let kind = self.model.kind(target)?;
        let value = match value.plain() {
            Value::Formula(formula) => {
                self.check_formula(target, formula)?;
                Assigned::Formula(formula.clone())
            }
            Value::Quantity { value, unit } => {
                Assigned::Number(in_model_unit(self.model, target, *value, unit)?)
            }
            Value::Number(x) => Assigned::Number(*x),
            Value::Uncertain { .. } => unreachable!("the plain value is no uncertain value"),
        };
        Ok(Assignment {
            target: target.to_string(),
            kind,
            value,
        })
    }

    /// Checks that every symbol of a formula is the time or an entity of the model.
    fn check_formula(&self, target: &str, formula: &str) -> Result<(), ScanError> {
        let compiled = CompiledFormula::new(formula).map_err(|err| {
            ScanError::request(format!(
                "The formula '{formula}' of '{target}' is no math of PEtab: {err}"
            ))
        })?;
        for symbol in compiled.symbols() {
            if !self.model.is_entity(symbol) {
                return Err(ScanError::request(format!(
                    "The formula '{formula}' of '{target}' reads '{symbol}', which is neither \
                     the time nor an entity of the model."
                )));
            }
        }
        Ok(())
    }

    /// The assignments of changes before the initialization, whose formulas read parameters
    /// only: nothing else has a value before the model is initialized (PEtab v2).
    fn preinit(
        &self,
        changes: &crate::request::Ordered<Value>,
    ) -> Result<Vec<Assignment>, ScanError> {
        let assignments = changes
            .iter()
            .map(|(target, value)| self.assignment(target, value))
            .collect::<Result<Vec<_>, _>>()?;
        for a in &assignments {
            let Assigned::Formula(formula) = &a.value else {
                continue;
            };
            let compiled = CompiledFormula::new(formula).map_err(ScanError::request)?;
            let time = compiled.reads_time().then(|| "time".to_string());
            for symbol in compiled.symbols().iter().cloned().chain(time) {
                if !self.model.is_parameter(&symbol) {
                    return Err(ScanError::request(format!(
                        "The formula '{formula}' of '{}' before the initialization reads \
                         '{symbol}', which is not a parameter: nothing else has a value before \
                         the model is initialized.",
                        a.target
                    )));
                }
            }
        }
        Ok(assignments)
    }

    fn steady_state(
        &self,
        steady: Option<&SteadyStateDefinition>,
    ) -> Result<Option<SteadyStatePlan>, ScanError> {
        let Some(steady) = steady else {
            return Ok(None);
        };
        Ok(Some(SteadyStatePlan {
            preinit: self.preinit(&steady.preinit_changes)?,
            absolute_tolerance: steady.absolute_tolerance,
            relative_tolerance: steady.relative_tolerance,
            max_time: steady.max_time,
        }))
    }
}

/// Compiles a simulation against a model, sbmlsim's `compile_simulation`.
///
/// # Errors
///
/// A time or a value which cannot be converted, a target which is no target of the model, a
/// change or an output time outside of the simulation, or two values of one target at one
/// time.
pub fn compile_simulation(
    simulation: &SimulationDefinition,
    model: &ModelInfo,
) -> Result<Plan, ScanError> {
    if simulation.kind != "Simulation" {
        return Err(ScanError::request(format!(
            "A simulation has the type 'Simulation', not '{}'.",
            simulation.kind
        )));
    }
    let convert = Converter::new(model, simulation.time_unit.as_deref())?;
    let start = convert.time(&simulation.start)?;
    let end = convert.time(&simulation.end)?;
    if start.partial_cmp(&end) != Some(std::cmp::Ordering::Less) {
        return Err(ScanError::request(format!(
            "The start '{}' of the simulation is not before its end '{}'.",
            simulation.start, simulation.end
        )));
    }
    if simulation.times.is_some() && simulation.steps.is_some() {
        return Err(ScanError::request(
            "A simulation has exact output 'times' or an equidistant grid of 'steps', not both.",
        ));
    }
    let compiler = Compiler { model, convert };

    let mut by_time: Vec<(f64, Vec<Assignment>)> = Vec::new();
    for change in &simulation.changes {
        if change.values.is_empty() || change.times.is_empty() {
            return Err(ScanError::request(
                "A change has at least one time and at least one value.",
            ));
        }
        for time in &change.times {
            let t = compiler.convert.time(time)?;
            if !(start <= t && t <= end) {
                let mut targets: Vec<&str> = change.values.keys().collect();
                targets.sort_unstable();
                return Err(ScanError::request(format!(
                    "The change of {} at '{time}' is outside of the simulation [{}, {}].",
                    text::list(&targets),
                    simulation.start,
                    simulation.end
                )));
            }
            let position = match by_time.iter().position(|(time, _)| *time == t) {
                Some(position) => position,
                None => {
                    by_time.push((t, Vec::new()));
                    by_time.len() - 1
                }
            };
            for (target, value) in change.values.iter() {
                if by_time[position].1.iter().any(|a| a.target == target) {
                    return Err(ScanError::request(format!(
                        "Two values of '{target}' at the time '{time}' ({} in the time unit of \
                         the model): a target is set once at a time.",
                        text::number(t)
                    )));
                }
                let assignment = compiler.assignment(target, value)?;
                by_time[position].1.push(assignment);
            }
        }
    }
    by_time.sort_by(|a, b| a.0.total_cmp(&b.0));
    let events = by_time
        .into_iter()
        .map(|(time, assignments)| PlanEvent { time, assignments })
        .collect();

    let mut output = OutputMode::Integrator;
    let mut times = Vec::new();
    let mut steady_state_output = None;
    if let Some(steps) = simulation.steps {
        if steps < 1 {
            return Err(ScanError::request(format!(
                "The 'steps' must be at least 1, not {steps}."
            )));
        }
        output = OutputMode::Times;
        times = linspace(start, end, steps);
    } else if let Some(given) = &simulation.times {
        output = OutputMode::Times;
        // the times once each, as python's set of them: `-0.0` is `0.0`
        let mut converted: Vec<f64> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for time in given {
            let t = compiler.convert.time(time)?;
            if seen.insert(if t == 0.0 { 0 } else { t.to_bits() }) {
                converted.push(t);
            }
        }
        if let Some(k) = converted.iter().position(|t| *t == f64::INFINITY) {
            // the steady state after the end
            converted.remove(k);
            steady_state_output = Some(STEADY_STATE);
        }
        converted.sort_by(f64::total_cmp);
        let (merged, _) = merge_times(&converted, start, end);
        let outside: Vec<String> = merged
            .iter()
            .filter(|t| !(start <= **t && **t <= end))
            .map(|t| text::number(*t))
            .collect();
        if !outside.is_empty() {
            return Err(ScanError::request(format!(
                "The output times [{}] are outside of the simulation [{}, {}] in the time unit \
                 of the model.",
                outside.join(", "),
                text::number(start),
                text::number(end)
            )));
        }
        times = merged;
    }

    Ok(Plan {
        start,
        end,
        preinit: compiler.preinit(&simulation.preinit_changes)?,
        events,
        steady_state: compiler.steady_state(simulation.presimulation.as_ref())?,
        output,
        times,
        time_shift: compiler.convert.time(&simulation.time_shift)?,
        steady_state_output,
    })
}

/// A time of a simulation in the time unit of the model, sbmlsim's `model_time`.
///
/// # Errors
///
/// A time which cannot be converted.
pub fn model_time(
    simulation: &SimulationDefinition,
    time: &Time,
    model: &ModelInfo,
) -> Result<f64, ScanError> {
    Converter::new(model, simulation.time_unit.as_deref())?.time(time)
}

/// The pre-initialization assignments of a plan and of its steady state.
pub fn preinit_targets(plan: &Plan) -> Vec<&Assignment> {
    let steady = plan.steady_state.iter().flat_map(|s| s.preinit.iter());
    plan.preinit.iter().chain(steady).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_within_the_resolution_merge() {
        let (merged, index) = merge_times(&[0.0, 5.0, 5.0 + 1e-12, 10.0], 0.0, 10.0);
        assert_eq!(merged, [0.0, 5.0, 10.0]);
        assert_eq!(index, [0, 1, 1, 2]);
        // a cluster at the start keeps the time inside the simulation
        let (merged, _) = merge_times(&[-1e-12, 0.0, 1.0], 0.0, 1.0);
        assert_eq!(merged, [0.0, 1.0]);
        let (merged, _) = merge_times(&[1.0, f64::INFINITY, f64::INFINITY], 0.0, 1.0);
        assert_eq!(merged, [1.0, f64::INFINITY]);
    }

    #[test]
    fn linspace_is_the_one_of_numpy() {
        assert_eq!(linspace(0.0, 1.0, 10)[3], 3.0 * 0.1);
        assert_eq!(
            linspace(0.0, 1.0, 3),
            [0.0, 1.0 / 3.0, 2.0 * (1.0 / 3.0), 1.0]
        );
    }
}

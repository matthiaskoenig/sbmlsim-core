//! The integration in segments, the time intervals between the changes, and the rows of the
//! output.

use sbmlsim_abi::IntegratorSettings;
use sbmlsim_csolvers::ffi::{CV_NORMAL, CV_ONE_STEP, CV_ROOT_RETURN, CV_TSTOP_RETURN};
use std::borrow::Cow;

use crate::changes::Changes;
use crate::events::{END_RTOL, Events, Probe, time_scale};
use crate::model::ModelFns;
use crate::roots::RootGuard;
use crate::solver::{Integrator, Problem};
use crate::{EngineError, OUTPUT_LIMIT_FLAG, STALL_FLAG};

/// A column of the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    /// The time.
    Time,
    /// The selection at this position of the output of [`ModelFns::selections`].
    Selection(usize),
}

/// The rows of a result: every row is the time and the selections of the model
/// ([`ModelFns::selections`], which computes the assignment rules it reads itself and reads the
/// state and the parameters only, so the output does not change the integration).
/// The forward sensitivities of the columns of a result while it is recorded.
///
/// The sensitivities of the states come from CVODES, the ones of a column from the values of
/// the selections of the model at the state and the parameters of the row moved along the
/// sensitivity: for a column which is a state that difference is the sensitivity itself, for
/// one which is not (a concentration, an assignment rule, a constant of the model) it is the
/// derivative of its value in the parameter.
pub(crate) struct Sensitivities {
    /// The ids of the parameters, in the order of the sensitivities.
    pub parameters: Vec<String>,
    /// For every sensitivity, what its step moves the whole parameter vector by, which is the
    /// difference of two initializations (`k * n_p`, row major): the step of the target and
    /// the values the initial assignments of the model derive from it.
    pub deltas: Vec<f64>,
    /// The step of the difference of every sensitivity, relative to its value at the start.
    pub steps: Vec<f64>,
    /// The sensitivities of the states at the row which is recorded next, `k * n` row major.
    pub states: Vec<f64>,
    /// The sensitivities of the columns, row major: the parameter of the column of the row.
    pub values: Vec<f64>,
    /// The state, the parameters and the selections of the difference.
    y: Vec<f64>,
    p: Vec<f64>,
    selections: Vec<f64>,
}

impl Sensitivities {
    /// The sensitivities of the targets of `parameters`, whose initial values are `states`
    /// (`k * n` row major) and whose steps move the parameters by `deltas` (`k * n_p`, row
    /// major), of a model of `n` states, `n_p` values of `p` and `n_selections` selections.
    pub fn new(
        parameters: Vec<String>,
        deltas: Vec<f64>,
        steps: Vec<f64>,
        states: Vec<f64>,
        n_p: usize,
        n_selections: usize,
    ) -> Sensitivities {
        let n = if parameters.is_empty() {
            0
        } else {
            states.len() / parameters.len()
        };
        Sensitivities {
            parameters,
            deltas,
            steps,
            states,
            values: Vec::new(),
            y: vec![0.0; n],
            p: vec![0.0; n_p],
            selections: vec![0.0; n_selections],
        }
    }

    /// The number of sensitivities per value of a row.
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Records the sensitivities of the columns of one row.
    fn row(&mut self, at: At<'_>) -> Result<(), EngineError> {
        let At {
            model,
            columns,
            mask,
            t,
            y,
            p,
            values,
        } = at;
        let k = self.parameters.len();
        let n = self.y.len();
        let at = self.values.len();
        self.values.resize(at + columns.len() * k, 0.0);
        for parameter in 0..k {
            // the state and the parameters of the row, moved along the sensitivity of this
            // parameter by the step of its difference
            let step = self.steps[parameter];
            for (state, value) in self.y.iter_mut().enumerate() {
                *value = y[state] + step * self.states[parameter * n + state];
            }
            let n_p = self.p.len();
            self.p.copy_from_slice(&p[..n_p]);
            for (slot, delta) in self
                .p
                .iter_mut()
                .zip(&self.deltas[parameter * n_p..(parameter + 1) * n_p])
            {
                *slot += delta;
            }
            // the constants of the model behind its parameters are the ones of the moved
            // parameter, which `assign` recomputes (it writes the targets of the assignment
            // rules into the copy of the state as well)
            model.assign(t, &mut self.y, &mut self.p);
            model.selections(t, &self.y, &self.p, mask, &mut self.selections);
            model.check_error()?;
            for (column, slot) in columns.iter().enumerate() {
                let difference = match slot {
                    Column::Time => 0.0,
                    Column::Selection(index) => (self.selections[*index] - values[*index]) / step,
                };
                self.values[at + column * k + parameter] = difference;
            }
        }
        Ok(())
    }
}

/// The row the sensitivities of a recorder are recorded at: what the model is asked with and
/// the values of the selections of the row, which the difference of every parameter is taken
/// against.
struct At<'a> {
    model: &'a dyn ModelFns,
    columns: &'a [Column],
    mask: &'a [u8],
    t: f64,
    y: &'a [f64],
    p: &'a [f64],
    values: &'a [f64],
}

/// The local error estimates of the integrator at the rows of a result while it is recorded.
pub(crate) struct LocalErrors {
    /// The ids of the states.
    states: Vec<String>,
    /// The estimates of the last step of the integrator, `NaN` before the first one, which
    /// the caller fills after every return of the integrator ([`Recorder::last_step`]).
    errors: Vec<f64>,
    /// The weights of the error test of that step.
    weights: Vec<f64>,
    /// The estimates of the rows, row major.
    error_rows: Vec<f64>,
    /// The weights of the rows, row major.
    weight_rows: Vec<f64>,
}

impl LocalErrors {
    /// The local errors of the states of these ids, before the first step.
    pub fn new(states: Vec<String>) -> LocalErrors {
        let n = states.len();
        LocalErrors {
            states,
            errors: vec![f64::NAN; n],
            weights: vec![f64::NAN; n],
            error_rows: Vec::new(),
            weight_rows: Vec::new(),
        }
    }

    /// The values of a row: an estimate and a weight per state.
    fn len(&self) -> usize {
        2 * self.states.len()
    }

    /// Records the estimates of one row.
    fn row(&mut self) -> Result<(), EngineError> {
        let n = self.states.len();
        let rows = self.error_rows.len() + self.weight_rows.len();
        for (table, values) in [
            (&mut self.error_rows, &self.errors),
            (&mut self.weight_rows, &self.weights),
        ] {
            table
                .try_reserve(n)
                .map_err(|_| out_of_memory(rows + 2 * n))?;
            table.extend_from_slice(values);
        }
        Ok(())
    }

    /// The estimates of the rows as a part of a result.
    pub fn into_rows(self) -> sbmlsim_abi::LocalErrors {
        sbmlsim_abi::LocalErrors {
            states: self.states,
            errors: self.error_rows,
            weights: self.weight_rows,
        }
    }
}

pub(crate) struct Recorder<'a> {
    model: &'a dyn ModelFns,
    columns: Cow<'a, [Column]>,
    /// A byte per selection of the model, 1 for the selections of a column.
    mask: Cow<'a, [u8]>,
    selections: Vec<f64>,
    selection_scratch: Option<&'a mut Vec<f64>>,
    /// The values of the rows, row major, after the `prefix` slots which the caller fills (the
    /// header of the output of `engine.wasm`, which so needs no second copy of the result).
    pub values: Vec<f64>,
    /// The number of slots before the first row.
    prefix: usize,
    /// The number of rows.
    pub n_rows: usize,
    /// The most values the result may hold (the prefix not counted).
    cap: usize,
    /// The forward sensitivities of the columns, for a run which asked for them.
    pub sensitivities: Option<Sensitivities>,
    /// The local error estimates of the integrator at the rows, for a run which asked for them.
    pub local_errors: Option<LocalErrors>,
}

/// The [`EngineError::Request`] of a result which does not fit into the memory of the engine.
fn out_of_memory(values: usize) -> EngineError {
    EngineError::Request(format!(
        "the result of {values} values does not fit into the memory of the engine"
    ))
}

impl Drop for Recorder<'_> {
    fn drop(&mut self) {
        if let Some(scratch) = self.selection_scratch.take() {
            *scratch = std::mem::take(&mut self.selections);
        }
    }
}

impl<'a> Recorder<'a> {
    /// A recorder of the columns with `prefix` slots of 0 before the first row;
    /// `n_selections` is the length of the output of [`ModelFns::selections`].
    #[cfg(test)]
    pub fn new(
        model: &'a dyn ModelFns,
        columns: Vec<Column>,
        n_selections: usize,
        cap: usize,
        prefix: usize,
    ) -> Result<Self, EngineError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(prefix)
            .map_err(|_| out_of_memory(prefix))?;
        values.resize(prefix, 0.0);
        let mut mask = vec![0u8; n_selections];
        for column in &columns {
            if let Column::Selection(index) = column {
                mask[*index] = 1;
            }
        }
        Ok(Recorder {
            model,
            columns: Cow::Owned(columns),
            mask: Cow::Owned(mask),
            selections: vec![0.0; n_selections],
            selection_scratch: None,
            values,
            prefix,
            n_rows: 0,
            cap,
            sensitivities: None,
            local_errors: None,
        })
    }

    pub fn with_scratch(
        model: &'a dyn ModelFns,
        columns: &'a [Column],
        mask: &'a [u8],
        selections: &'a mut Vec<f64>,
        cap: usize,
        prefix: usize,
    ) -> Result<Self, EngineError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(prefix)
            .map_err(|_| out_of_memory(prefix))?;
        values.resize(prefix, 0.0);
        selections.clear();
        selections.resize(mask.len(), 0.0);
        Ok(Self {
            model,
            columns: Cow::Borrowed(columns),
            mask: Cow::Borrowed(mask),
            selections: std::mem::take(selections),
            selection_scratch: Some(selections),
            values,
            prefix,
            n_rows: 0,
            cap,
            sensitivities: None,
            local_errors: None,
        })
    }

    /// Reserves the values of `rows` more rows, in one allocation; a result which does not fit
    /// into the memory is an [`EngineError::Request`] (not an abort of the allocator).
    pub fn reserve(&mut self, rows: usize) -> Result<(), EngineError> {
        let n = rows.saturating_mul(self.columns.len());
        self.values
            .try_reserve_exact(n)
            .map_err(|_| out_of_memory(self.values.len() - self.prefix + n))
    }

    /// Makes room for `k` more values: by doubling (the steps of the integrator), but never
    /// beyond the prefix and the cap, so the growth of a result at the cap is never twice the cap
    /// (which, while the old block is alive, would not fit into the memory and be an error of
    /// the memory instead of the limit of the output). A growth which finds no memory is an
    /// error and not an abort of the allocator, which would write to stderr.
    fn grow(&mut self, k: usize) -> Result<(), EngineError> {
        let (len, capacity) = (self.values.len(), self.values.capacity());
        if capacity - len >= k {
            return Ok(());
        }
        let new = (2 * capacity).max(len + k).min(self.prefix + self.cap);
        self.values
            .try_reserve_exact(new - len)
            .map_err(|_| out_of_memory(len - self.prefix + k))
    }

    /// Appends the row of the time `t`, the state `y` and the parameters `p`.
    ///
    /// Fails with [`OUTPUT_LIMIT_FLAG`] when the row would exceed the cap of values of the
    /// result.
    pub fn row(&mut self, t: f64, y: &[f64], p: &[f64]) -> Result<(), EngineError> {
        let k = self.columns.len();
        // a row of a run of sensitivities carries one value per parameter of every column, and
        // one of local errors two values per state
        let per_row = k * (1 + self.sensitivities.as_ref().map_or(0, Sensitivities::len))
            + self.local_errors.as_ref().map_or(0, LocalErrors::len);
        if self.values.len() - self.prefix + per_row > self.cap {
            return Err(EngineError::Integration {
                flag: OUTPUT_LIMIT_FLAG,
                time: t,
                message: format!(
                    "the result would exceed the limit of {} values ({} rows of {} columns)",
                    self.cap,
                    self.n_rows,
                    self.columns.len()
                ),
            });
        }
        // a recorder without columns (a stepper's) records no value of the model
        if k > 0 {
            self.model
                .selections(t, y, p, &self.mask, &mut self.selections);
            self.model.check_error()?;
        }
        self.grow(k)?;
        for column in self.columns.iter() {
            self.values.push(match column {
                Column::Time => t,
                Column::Selection(index) => self.selections[*index],
            });
        }
        if let Some(sensitivities) = &mut self.sensitivities {
            sensitivities.row(At {
                model: self.model,
                columns: &self.columns,
                mask: &self.mask,
                t,
                y,
                p,
                values: &self.selections,
            })?;
        }
        if let Some(local_errors) = &mut self.local_errors {
            local_errors.row()?;
        }
        self.n_rows += 1;
        Ok(())
    }

    /// Reads the local error estimates of the last step of the integrator for the rows after
    /// it, for a run which asked for them; the caller calls it after every return of the
    /// integrator.
    pub fn last_step<const DAE: bool, H: crate::delay::Mode>(
        &mut self,
        cvode: &mut Integrator<DAE, H>,
    ) -> Result<(), EngineError> {
        match &mut self.local_errors {
            Some(local) => cvode.local_errors(&mut local.errors, &mut local.weights),
            None => Ok(()),
        }
    }

    /// The sensitivities of the states at the row which is recorded next, which the caller
    /// fills from the integrator at every return.
    pub fn state_sensitivities(&mut self) -> Option<&mut [f64]> {
        self.sensitivities
            .as_mut()
            .map(|sensitivities| sensitivities.states.as_mut_slice())
    }

    /// The sensitivities of the run, to carry them across a change of the request.
    pub fn sensitivities_mut(&mut self) -> Option<&mut Sensitivities> {
        self.sensitivities.as_mut()
    }

    /// Those values, to start the sensitivities of a segment with.
    pub fn sensitivities_at(&self) -> Option<&[f64]> {
        self.sensitivities
            .as_ref()
            .map(|sensitivities| sensitivities.states.as_slice())
    }
}

/// The output of a segment.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SegmentOutput<'a> {
    /// The given times, non-decreasing, within the segment.
    Times(&'a [f64]),
    /// The start, every step of the integrator and the end.
    Steps,
}

/// Whether `t` is too close to `t0` for CVODE to integrate from `t0` to it (the test of
/// `cvHin`: a distance below two units of rounding of the larger magnitude); the state at `t`
/// is then the state at `t0`.
pub(crate) fn too_close(t0: f64, t: f64) -> bool {
    t == t0 || (t - t0).abs() < 2.0 * f64::EPSILON * t0.abs().max(t.abs())
}

/// Integrates a model without events from `t0` with the state `y0` to `end`, with the changes
/// at their times, and records its output.
///
/// The changes at `t0` are applied before the output of `t0`. Every other change ends a
/// segment: CVODE stops at its time (its stop time), the output has the state before the change
/// and the state after it at its time (two rows: the steps of the integrator always, an output
/// of times when the time of the change is one of its times), and the next segment starts CVODE
/// again from the state after it.
///
/// `cvode` is `None` for a model without states, whose rows are the selections at the times
/// of the output. The stop time of CVODE is the end of the segment, so the integration never
/// passes it; with [`SegmentOutput::Times`] it ends at the last time of the output. `y0` is
/// the state the integration ended in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate<const DAE: bool, H: crate::delay::Mode>(
    mut cvode: Option<&mut Integrator<DAE, H>>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    t0: f64,
    y0: &mut [f64],
    end: f64,
    output: SegmentOutput<'_>,
    recorder: &mut Recorder<'_>,
    changes: &Changes,
    mut constraints: Option<&mut crate::solver::Constraints<'_>>,
) -> Result<(), EngineError> {
    let model = problem.model;
    let y = y0;
    let mut k = 0;
    if changes.time(0) == Some(t0) {
        apply_change(
            changes,
            0,
            model,
            y,
            &mut problem.p,
            recorder,
            problem.sensitivity.as_mut(),
        )?;
        if DAE && let Some(projection) = constraints.as_deref_mut() {
            projection.project(t0, y, &problem.p)?;
        }
        k = 1;
    }
    let mut t = t0;
    let mut next = 0;
    loop {
        let change = changes.time(k);
        let stop = change.unwrap_or(end);
        match output {
            SegmentOutput::Times(times) => {
                let mut started = false;
                // the times before the change, every one up to the end without
                while next < times.len() && change.is_none_or(|c| times[next] < c) {
                    let tout = times[next];
                    next += 1;
                    let Some(cvode) = cvode.as_deref_mut() else {
                        recorder.row(tout, y, &problem.p)?;
                        continue;
                    };
                    if !started && too_close(t, tout) {
                        recorder.row(tout, y, &problem.p)?;
                        continue;
                    }
                    if !started {
                        cvode.start(t, y, stop, settings, 0)?;
                        if recorder.sensitivities_at().is_some() {
                            let at = recorder.sensitivities_at().expect("the sensitivities");
                            // the values are the ones of the recorder, which the problem of
                            // the call does not borrow
                            let at = at.to_vec();
                            cvode.start_sensitivities(problem, &at)?;
                        }
                        started = true;
                    }
                    cvode.integrate(problem, tout, CV_NORMAL)?;
                    recorder.last_step(cvode)?;
                    if let Some(states) = recorder.state_sensitivities() {
                        cvode.sensitivities(states)?;
                    }
                    recorder.row(tout, cvode.state(), &problem.p)?;
                }
                // nothing after the last output time is seen
                let Some(c) = change.filter(|_| next < times.len()) else {
                    // the state the output ended in, which the caller continues from
                    if started && let Some(cvode) = cvode.as_deref() {
                        y.copy_from_slice(cvode.state());
                    }
                    return Ok(());
                };
                if let Some(cvode) = cvode.as_deref_mut()
                    && (started || !too_close(t, c))
                {
                    if !started {
                        cvode.start(t, y, c, settings, 0)?;
                        if let Some(at) = recorder.sensitivities_at() {
                            let at = at.to_vec();
                            cvode.start_sensitivities(problem, &at)?;
                        }
                    }
                    cvode.integrate(problem, c, CV_NORMAL)?;
                    recorder.last_step(cvode)?;
                    // the sensitivities at the change, which it carries into the next segment
                    if let Some(states) = recorder.state_sensitivities() {
                        cvode.sensitivities(states)?;
                    }
                    y.copy_from_slice(cvode.state());
                }
                // the state before the change at an output time; the rows of the state after
                // it are the ones of the next segment
                if times[next] == c {
                    recorder.row(c, y, &problem.p)?;
                }
                t = c;
            }
            SegmentOutput::Steps => {
                if t == t0 {
                    recorder.row(t0, y, &problem.p)?;
                }
                if stop != t {
                    match cvode.as_deref_mut() {
                        Some(cvode) if !too_close(t, stop) => {
                            cvode.start(t, y, stop, settings, 0)?;
                            if let Some(at) = recorder.sensitivities_at() {
                                let at = at.to_vec();
                                cvode.start_sensitivities(problem, &at)?;
                            }
                            let mut advance = Advance::new(t);
                            loop {
                                let (tret, flag) = cvode.integrate(problem, stop, CV_ONE_STEP)?;
                                recorder.last_step(cvode)?;
                                if flag != CV_TSTOP_RETURN {
                                    advance.step(tret)?;
                                }
                                // CVODE returns the stop time exactly when it reaches it
                                let tret = if flag == CV_TSTOP_RETURN { stop } else { tret };
                                if let Some(states) = recorder.state_sensitivities() {
                                    cvode.sensitivities(states)?;
                                }
                                recorder.row(tret, cvode.state(), &problem.p)?;
                                if flag == CV_TSTOP_RETURN || tret >= stop {
                                    break;
                                }
                            }
                            y.copy_from_slice(cvode.state());
                        }
                        _ => recorder.row(stop, y, &problem.p)?,
                    }
                }
                let Some(c) = change else {
                    return Ok(());
                };
                t = c;
            }
        }
        // the change at `t`, whose state after it starts the next segment
        apply_change(
            changes,
            k,
            model,
            y,
            &mut problem.p,
            recorder,
            problem.sensitivity.as_mut(),
        )?;
        if DAE && let Some(projection) = constraints.as_deref_mut() {
            projection.project(t, y, &problem.p)?;
        }
        k += 1;
        if matches!(output, SegmentOutput::Steps) {
            recorder.row(t, y, &problem.p)?;
        }
    }
}

/// Applies change `k` of a request to the state and the parameters, and carries the forward
/// sensitivities of the run across it.
///
/// A sensitivity is a direction of the state and of the parameters: its row moves the state
/// by `step * s` and the parameters by its `delta` (see [`Sensitivities`]). The change is
/// applied to the state and the parameters moved along every direction as well, and the
/// difference of the two afterwards over the step is the direction after the change: a value
/// the change sets to a number has no derivative after it, a value it adds a number to keeps
/// its derivative, a form `constant + sum(factor * term)` takes the derivatives of its terms,
/// and the conversions of the change (the size of a compartment, an amount and a
/// concentration, the assignment rules) are carried along, since they are what the change
/// does. Every map of a change is linear in the values it reads but for the conversions
/// between amounts and concentrations, so the difference is the derivative to the rounding.
///
/// The parameters the change moves along a direction are the `df/dp` of the sensitivities
/// after it ([`crate::solver::Sensitivity::set_deltas`]): a parameter the change sets to a number no
/// longer depends on the target.
fn apply_change(
    changes: &Changes,
    k: usize,
    model: &dyn ModelFns,
    y: &mut [f64],
    p: &mut [f64],
    recorder: &mut Recorder<'_>,
    sensitivity: Option<&mut crate::solver::Sensitivity>,
) -> Result<(), EngineError> {
    let Some(sensitivities) = recorder.sensitivities_mut() else {
        return changes.apply(k, model, y, p);
    };
    let n = y.len();
    let n_p = sensitivities.p.len();
    let count = sensitivities.len();
    let mut moved_y = vec![0.0; n];
    let mut moved_p = p.to_vec();
    let mut states = vec![0.0; count * n];
    let mut deltas = vec![0.0; count * n_p];
    let mut moved: Vec<(Vec<f64>, Vec<f64>)> = Vec::with_capacity(count);
    for at in 0..count {
        let step = sensitivities.steps[at];
        for (state, value) in moved_y.iter_mut().enumerate() {
            *value = y[state] + step * sensitivities.states[at * n + state];
        }
        moved_p.copy_from_slice(p);
        for (slot, delta) in moved_p[..n_p]
            .iter_mut()
            .zip(&sensitivities.deltas[at * n_p..(at + 1) * n_p])
        {
            *slot += delta;
        }
        changes.apply(k, model, &mut moved_y, &mut moved_p)?;
        moved.push((moved_y.clone(), moved_p.clone()));
    }
    changes.apply(k, model, y, p)?;
    for (at, (after_y, after_p)) in moved.iter().enumerate() {
        let step = sensitivities.steps[at];
        for state in 0..n {
            states[at * n + state] = (after_y[state] - y[state]) / step;
        }
        for slot in 0..n_p {
            deltas[at * n_p + slot] = after_p[slot] - p[slot];
        }
    }
    sensitivities.states = states;
    sensitivities.deltas.clone_from(&deltas);
    if let Some(sensitivity) = sensitivity {
        sensitivity.set_deltas(deltas);
    }
    Ok(())
}

/// The steps of the integrator in a row which may leave the time where it was.
///
/// A step whose size is below the resolution of the time (CVODE's warning `t + h = t`) leaves
/// the time as it was; CVODE takes it nevertheless and grows the step again, so the steps after
/// a change of the state at a late time (a dose on day 23, 33120 min, with tight tolerances)
/// advance after a few of them, as they do in libroadrunner. A run of steps which does not get
/// past a time is a stall (the steps of Zeno towards a right hand side which is not finite), an
/// error and no loop without end.
const STILL_STEPS: u32 = 100;

/// Whether the steps of the integrator advance the time, see `STILL_STEPS`.
struct Advance {
    /// The last time a step advanced to.
    previous: f64,
    /// The steps in a row since then which did not advance it.
    still: u32,
}

impl Advance {
    /// Start at a time.
    fn new(t: f64) -> Self {
        Self {
            previous: t,
            still: 0,
        }
    }

    /// Record the time a step returned, an error after `STILL_STEPS` steps in a row which did
    /// not advance it.
    fn step(&mut self, t: f64) -> Result<(), EngineError> {
        if t > self.previous {
            self.previous = t;
            self.still = 0;
            return Ok(());
        }
        self.still += 1;
        if self.still > STILL_STEPS {
            return Err(stalled(t, self.previous));
        }
        Ok(())
    }
}

/// The error of a run of the integrator steps which does not advance the time.
fn stalled(t: f64, previous: f64) -> EngineError {
    EngineError::Integration {
        flag: STALL_FLAG,
        time: t,
        message: format!(
            "the integrator stalled: {STILL_STEPS} steps in a row did not advance the time past \
             {previous}"
        ),
    }
}

/// Where the integration of a model with events stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopKind {
    /// A step of the integrator (the output of its steps).
    Step,
    /// A stop time (a pending execution, the end), an output time, or a time too close to
    /// integrate to.
    Stop,
    /// A zero of a root function; `discontinuity` whether CVODE starts again there for a root
    /// function of a discontinuity.
    Root {
        /// Whether CVODE starts again for a root function of a discontinuity.
        discontinuity: bool,
    },
}

pub(crate) use crate::events::BeforeEvent as Scratch;

/// The outputs of a run with events and where they stand.
struct Outputs<'t> {
    scratch: &'t mut Scratch,
    /// The output times (empty for the integrator steps).
    times: &'t [f64],
    /// Whether the output is the steps of the integrator.
    steps: bool,
    /// The next output time.
    next: usize,
    /// The end of the simulation.
    end: f64,
    /// The number of states of the model.
    m: usize,
    /// The time of the next change which is not applied yet (infinity without): an output time
    /// at it or after it is never written before the change, also when it is too close to the
    /// time of a stop to integrate to.
    limit: f64,
    /// Whether the output at the end is the state before the events of the end execute
    /// ([`sbmlsim_abi::SimulationRequest::before_events`]): they are not executed at all, the
    /// simulation ending there, so the last row is the state the end was integrated to and
    /// the changes and the events before the end left.
    before_events: bool,
}

impl Outputs<'_> {
    /// Whether the next output time is at `t` (or before it, or too close to integrate to and
    /// before the next change).
    fn here(&self, t: f64) -> bool {
        self.next < self.times.len()
            && (self.times[self.next] <= t
                || (too_close(t, self.times[self.next]) && self.times[self.next] < self.limit))
    }

    /// Writes the rows of the output times at `t`, every one up to the end for `all`.
    fn write(
        &mut self,
        t: f64,
        y: &[f64],
        p: &[f64],
        all: bool,
        recorder: &mut Recorder<'_>,
    ) -> Result<(), EngineError> {
        while self.next < self.times.len() && (all || self.here(t)) {
            recorder.row(self.times[self.next], &y[..self.m], p)?;
            self.next += 1;
        }
        Ok(())
    }

    /// The integration stopped at `t` (`kind`): the events at `t` execute and the outputs at
    /// `t` are written. In the steps of the integrator: a step of the integrator is one row and
    /// its events execute with the triggers at its time only (no probe right after it: a
    /// trigger which becomes true right after a step is located by its root function in
    /// the next step, so its event executes at its root, as in libroadrunner, and not up to the
    /// window of the probe before it); a stop at a stop time is one row; at a zero of a root
    /// function of a discontinuity, where the integration starts again, two rows of the state
    /// at `t` before the end (libroadrunner's two rows of a root, which it writes a little
    /// before and after `t`); where events execute, their two rows, the state before and after
    /// them. Returns whether an event executed.
    fn stop<const DAE: bool>(
        &mut self,
        t: f64,
        y: &mut [f64],
        p: &mut [f64],
        kind: StopKind,
        events: &mut Events<'_, DAE>,
        recorder: &mut Recorder<'_>,
    ) -> Result<bool, EngineError> {
        let m = self.m;
        let at_end = t >= self.end || too_close(t, self.end);
        if self.steps {
            let after = if at_end {
                // the events of the end are not executed when the caller asked for the state
                // before them, and the row of the end is the one before them
                if self.before_events {
                    None
                } else {
                    Some(Probe::End)
                }
            } else if kind == StopKind::Step {
                None
            } else {
                Some(Probe::After)
            };
            let executed = events.execute_stop(t, &mut y[..m], p, after, Some(self.scratch))?;
            let discontinuity = kind
                == (StopKind::Root {
                    discontinuity: true,
                });
            if executed || (discontinuity && !at_end) {
                let (before, p_before) = if executed {
                    (self.scratch.y.as_slice(), self.scratch.p.as_slice())
                } else {
                    (&y[..m], &p[..])
                };
                recorder.row(t, before, p_before)?;
                recorder.row(t, &y[..m], p)?;
            } else if !matches!(kind, StopKind::Root { .. }) || at_end {
                recorder.row(t, &y[..m], p)?;
            }
            return Ok(executed);
        }
        if at_end {
            if self.before_events {
                self.write(t, y, p, true, recorder)?;
                return Ok(false);
            }
            let executed = events.execute(t, &mut y[..m], p, true)?;
            self.write(t, y, p, true, recorder)?;
            return Ok(executed);
        }
        if !self.here(t) {
            return events.execute(t, &mut y[..m], p, false);
        }
        let first = events.execute_at(t, &mut y[..m], p, Probe::Exact)?;
        self.write(t, y, p, false, recorder)?;
        let second = events.execute_at(t, &mut y[..m], p, Probe::After)?;
        Ok(first || second)
    }

    /// The integration reached the change `k` at its time `t`, without executing the events at
    /// `t`: the rows of the state before the change (a step of the integrator always; the output
    /// times before `t`, also too close to it to integrate to, and `t` once when it is an output
    /// time), the change, the events at `t` with the triggers
    /// before the change ([`Probe::Change`]: the events the change triggers), the rows of the
    /// state after the change and these events, and the events right after `t` (the
    /// executions pending at `t`, the strict comparisons at their threshold); at the end all of
    /// them before the rows of the end.
    #[allow(clippy::too_many_arguments)]
    fn change<const DAE: bool>(
        &mut self,
        t: f64,
        k: usize,
        changes: &Changes,
        y: &mut [f64],
        p: &mut [f64],
        events: &mut Events<'_, DAE>,
        recorder: &mut Recorder<'_>,
    ) -> Result<(), EngineError> {
        let m = self.m;
        let model = events.model();
        let at_end = t >= self.end || too_close(t, self.end);
        while self.next < self.times.len() && self.times[self.next] < t {
            recorder.row(self.times[self.next], &y[..m], p)?;
            self.next += 1;
        }
        if self.steps || self.times.get(self.next) == Some(&t) {
            recorder.row(t, &y[..m], p)?;
        }
        changes.apply(k, model, &mut y[..m], p)?;
        if DAE && let Some(projection) = &mut events.constraints {
            projection.project(t, &mut y[..m], p)?;
        }
        self.limit = changes.time(k + 1).unwrap_or(f64::INFINITY);
        events.execute_at(t, &mut y[..m], p, Probe::Change)?;
        if at_end {
            if !self.before_events {
                events.execute_at(t, &mut y[..m], p, Probe::End)?;
            }
            if self.steps {
                recorder.row(t, &y[..m], p)?;
            } else {
                self.write(t, y, p, true, recorder)?;
            }
            return Ok(());
        }
        if self.steps {
            recorder.row(t, &y[..m], p)?;
            if events.execute_at(t, &mut y[..m], p, Probe::After)? {
                recorder.row(t, &y[..m], p)?;
            }
        } else {
            self.write(t, y, p, false, recorder)?;
            events.execute_at(t, &mut y[..m], p, Probe::After)?;
        }
        Ok(())
    }
}

/// Integrates a model with events from `start` with the state `y0` to `end` and records its
/// output; see `events` for the semantics.
///
/// CVODE locates the zeros of the root functions and stops at the time of every pending
/// execution (its stop time); at every stop the events execute (`Events::execute`), and after an
/// execution CVODE starts again from the new state. At a zero of a root function of a
/// discontinuity of the math (a condition of a `piecewise`, `problem.n_discontinuity_roots`)
/// CVODE starts again from the state there, as libroadrunner does at its piecewise triggers;
/// nothing executes for it (no draw of the order of ties, no count towards the limit of
/// executions). The restarts follow libroadrunner's guard against roots which chatter, and the
/// steps towards one output time have a budget (`roots`). The output:
///
/// - of times: a time before an event sees the state before it, an output at the time of an
///   event the state after it, except for an event whose trigger becomes true only right after
///   that time (a strict comparison at its threshold), which comes after the output; at the end
///   the output sees the state after every event at the end. A discontinuity adds no row.
/// - of the integrator steps (the triggers at a step evaluated at its time only, [`StopKind`]):
///   the start (the state after the events at the start), every step,
///   and two rows at the time of every other stop at which events execute, the state before and
///   after them (the end included), or else at a discontinuity before the end, the state there
///   twice (libroadrunner's two rows of a root, before and after the new start of CVODE).
///
/// The changes: one at the start is applied after the events at the start and its events
/// execute with it ([`Probe::Change`]), before the output of the start; every other one is a
/// stop of CVODE at its time at which the events do not execute before it (`Outputs::change`):
/// the rows of the state before it, the change, its events, the rows of the state after it.
///
/// `cvode` is the memory of `problem.n` states; a model without states has `problem.n_model`
/// 0 and one state of the rate 0.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate_events<const DAE: bool, H: crate::delay::Mode>(
    cvode: &mut Integrator<DAE, H>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    start: f64,
    y0: &mut [f64],
    end: f64,
    output: SegmentOutput<'_>,
    recorder: &mut Recorder<'_>,
    events: &mut Events<'_, DAE>,
    changes: &Changes,
    scratch: &mut Scratch,
    before_events: bool,
) -> Result<(), EngineError> {
    let m = problem.n_model;
    let model = problem.model;
    let n_roots = problem.n_roots;
    // the state of CVODE (of `n` values), the model reads its first `m`
    let y = y0;
    debug_assert_eq!(y.len(), problem.n);
    let (times, steps): (&[f64], bool) = match output {
        SegmentOutput::Times(times) => (times, false),
        SegmentOutput::Steps => (&[], true),
    };
    let mut out = Outputs {
        scratch,
        times,
        steps,
        next: 0,
        end,
        m,
        limit: f64::INFINITY,
        before_events,
    };
    let mut t = start;

    // the start: the events at the start are part of the initial state; an output at the start
    // is written before the events whose triggers become true right after it
    let only_start = start >= end || too_close(start, end);
    events.execute_at(t, &mut y[..m], &mut problem.p, Probe::Exact)?;
    // a change at the start is applied after the initialization (and its events at the start),
    // the events it triggers execute with it
    let mut k = 0;
    if changes.time(0) == Some(start) {
        changes.apply(0, model, &mut y[..m], &mut problem.p)?;
        if DAE && let Some(projection) = &mut events.constraints {
            projection.project(t, &mut y[..m], &problem.p)?;
        }
        events.execute_at(t, &mut y[..m], &mut problem.p, Probe::Change)?;
        k = 1;
    }
    out.limit = changes.time(k).unwrap_or(f64::INFINITY);
    // the start is also the end of a simulation of an empty interval, where the caller which
    // asked for the state before the events of the end gets exactly that
    let at_the_end = only_start && before_events;
    if steps {
        recorder.row(t, &y[..m], &problem.p)?;
        // the state before these events is the row of the start
        let probe = if only_start { Probe::End } else { Probe::After };
        if !at_the_end && events.execute_at(t, &mut y[..m], &mut problem.p, probe)? {
            recorder.row(t, &y[..m], &problem.p)?;
        }
    } else {
        // at the end, the output sees the state after every event
        if !only_start {
            out.write(t, y, &problem.p, false, recorder)?;
        }
        let probe = if only_start { Probe::End } else { Probe::After };
        if !at_the_end {
            events.execute_at(t, &mut y[..m], &mut problem.p, probe)?;
        }
        if only_start {
            out.write(t, y, &problem.p, true, recorder)?;
        }
    }
    if only_start {
        return Ok(());
    }

    let mut started = false;
    let mut stop = end;
    let mut advance = Advance::new(t);
    let resolution = END_RTOL * time_scale(start, end);
    let mut guard = RootGuard::new(t, settings, !steps, cvode.steps(), resolution);
    loop {
        // an output of times ends at its last time, as without events
        if !steps && out.next >= out.times.len() {
            return Ok(());
        }
        let change = changes.time(k);
        let target = change.unwrap_or(end);
        let tstop = events.next_time().map_or(target, |x| x.min(target));
        if too_close(t, tstop) {
            // a stop too close to integrate to: the state at `t` is the state there
            t = tstop;
            guard.stop(t)?;
            if change == Some(t) {
                out.change(t, k, changes, y, &mut problem.p, events, recorder)?;
                k += 1;
                started = false;
                guard.interval(cvode.steps());
                if t >= end || too_close(t, end) {
                    return Ok(());
                }
                continue;
            }
            let next = out.next;
            if out.stop(t, y, &mut problem.p, StopKind::Stop, events, recorder)? {
                started = false;
                guard.executed(t, cvode.steps());
            }
            if out.next != next {
                guard.interval(cvode.steps());
            }
            if t >= end || too_close(t, end) {
                return Ok(());
            }
            continue;
        }
        if !started {
            cvode.start(t, y, tstop, settings, n_roots)?;
            started = true;
            stop = tstop;
            advance = Advance::new(t);
        } else if tstop != stop {
            if tstop < stop {
                // CVODE may have stepped past the new stop: start again from `t`
                cvode.start(t, y, tstop, settings, n_roots)?;
                advance = Advance::new(t);
            } else {
                cvode.set_stop(tstop)?;
            }
            stop = tstop;
        }
        let (tret, flag) = if steps {
            cvode.integrate(problem, tstop, CV_ONE_STEP)?
        } else {
            let tout = out.times.get(out.next).map_or(tstop, |x| x.min(tstop));
            guard.limit(cvode, t)?;
            cvode.integrate(problem, tout, CV_NORMAL)?
        };
        recorder.last_step(cvode)?;
        let root = flag == CV_ROOT_RETURN;
        // a zero of a root function of a discontinuity at which CVODE starts again (the guard
        // of the restarts: libroadrunner's count of roots close together, no second restart at
        // the time of the last one)
        let discontinuity = root && {
            let found = cvode.at_discontinuity(problem)?;
            guard.root(tret, found, cvode.steps())
        };
        if steps && !root && flag != CV_TSTOP_RETURN {
            advance.step(tret)?;
        }
        // CVODE returns the stop time exactly when it reaches it
        t = if flag == CV_TSTOP_RETURN { tstop } else { tret };
        guard.stop(t)?;
        y.copy_from_slice(cvode.state());
        // the time of a change: its events execute after it (also a zero of a root function
        // which CVODE locates at it, or a step too close to it to integrate to); an output time
        // before it is an output, also when it is too close to it
        let output = !steps && !root && flag != CV_TSTOP_RETURN;
        if let Some(c) = change.filter(|c| t >= *c || (!output && too_close(t, *c))) {
            t = c;
            out.change(t, k, changes, y, &mut problem.p, events, recorder)?;
            k += 1;
            started = false;
            guard.interval(cvode.steps());
            if t >= end || too_close(t, end) {
                return Ok(());
            }
            continue;
        }
        let next = out.next;
        let kind = if root {
            StopKind::Root { discontinuity }
        } else if flag == CV_TSTOP_RETURN || !steps {
            StopKind::Stop
        } else {
            StopKind::Step
        };
        let executed = out.stop(t, y, &mut problem.p, kind, events, recorder)?;
        if executed || discontinuity {
            started = false;
        }
        if executed {
            guard.executed(t, cvode.steps());
        }
        if out.next != next {
            guard.interval(cvode.steps());
        }
        if t >= end || too_close(t, end) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use sbmlsim_abi::{IntegratorSettings, Tolerance};

    use super::*;
    use crate::model::Decay;
    use crate::solver::{Config, Jacobian, Linear, Method};

    #[test]
    fn a_step_which_leaves_the_time_is_no_stall() {
        // the first step after a dose on day 23 of rapamycin in pkdb_models (Kelly1999)
        // returned the time of the dose, 33120 min, and the next one advanced
        let mut advance = Advance::new(33120.0);
        advance.step(33120.0).unwrap();
        advance.step(33120.000001).unwrap();
        // the count starts again after a step which advances
        for _ in 0..STILL_STEPS {
            advance.step(33120.000001).unwrap();
        }
        advance.step(33121.0).unwrap();
        for _ in 0..STILL_STEPS {
            advance.step(33121.0).unwrap();
        }
        // one more step in a row which does not advance is a stall
        match advance.step(33121.0) {
            Err(EngineError::Integration {
                flag,
                time,
                message,
            }) => {
                assert_eq!(flag, STALL_FLAG);
                assert_eq!(time, 33121.0);
                assert!(message.contains("stalled"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn steps_stop_at_the_output_limit() {
        let model = Decay { k: 1.0 };
        let desc = model.description();
        let (p, y0) = (vec![1.0, 2f64.ln()], vec![1.0]);
        let settings = IntegratorSettings {
            relative_tolerance: 1e-10,
            absolute_tolerance: Tolerance::Scalar(1e-14),
            ..IntegratorSettings::default()
        };
        let config = Config {
            n_sens: 0,
            n: 1,
            method: Method::Bdf(Linear::Dense {
                jacobian: Jacobian::Analytic,
            }),
        };
        let mut cvode = Integrator::<false>::new(config).unwrap();
        let mut problem = Problem {
            sensitivity: None,
            n: 1,
            n_model: 1,
            n_algebraic: 0,
            interpolation: None,
            n_roots: 0,
            n_discontinuity_roots: 0,
            model: &model,
            p: p.clone(),
            pattern: Some(crate::pattern_of(&desc)),
            jac_values: vec![0.0],
            jacobian_p: None,
            analytic: true,
            difference: None,
            mem: std::ptr::null_mut(),
            krylov: std::ptr::null_mut(),
            counts: crate::solver::Counts::default(),
            callback_error: None,
        };
        let no_changes = Changes::new(&desc, &[], 0.0, 50.0).unwrap();
        let mut run = |cap: usize| {
            let mut y0 = y0.clone();
            let columns = vec![Column::Time, Column::Selection(0)];
            let mut recorder = Recorder::new(&model, columns, 2, cap, 0).unwrap();
            let result = integrate(
                Some(&mut cvode),
                &mut problem,
                &settings,
                0.0,
                &mut y0,
                50.0,
                SegmentOutput::Steps,
                &mut recorder,
                &no_changes,
                None,
            );
            (result, recorder)
        };
        // the whole run is more than 20 rows, so a cap of 40 values stops it
        let (result, recorder) = run(1 << 20);
        result.unwrap();
        assert!(recorder.n_rows > 20, "{}", recorder.n_rows);
        // the growth by doubling never reserves beyond the prefix and the cap, also when the
        // prefix is a power of two (the header of `engine.wasm` is 8 slots for two columns)
        for (prefix, cap) in [(8, 40), (8, 64), (8, 56), (3, 40), (0, 40), (16, 1000)] {
            let columns = vec![Column::Time, Column::Selection(0)];
            let mut recorder = Recorder::new(&model, columns, 2, cap, prefix).unwrap();
            let mut seen = 0;
            loop {
                match recorder.row(0.0, &y0, &p) {
                    Ok(()) => {
                        let capacity = recorder.values.capacity();
                        assert!(capacity <= prefix + cap, "{prefix} {cap}: {capacity}");
                        seen = seen.max(capacity);
                    }
                    Err(EngineError::Integration { flag, .. }) => {
                        assert_eq!(flag, OUTPUT_LIMIT_FLAG);
                        break;
                    }
                    Err(other) => panic!("{other:?}"),
                }
            }
            assert_eq!(
                recorder.values.len(),
                prefix + cap / 2 * 2,
                "{prefix} {cap}"
            );
            assert!(seen <= prefix + cap);
        }
        match run(40) {
            (
                Err(EngineError::Integration {
                    flag,
                    time,
                    message,
                }),
                recorder,
            ) => {
                assert_eq!(flag, OUTPUT_LIMIT_FLAG);
                assert!(flag < -99, "distinct from the flags of CVODE");
                assert!(time > 0.0 && time < 50.0, "{time}");
                assert!(message.contains("limit of 40"), "{message}");
                assert_eq!(recorder.n_rows, 20);
                assert_eq!(recorder.values.len(), 40);
            }
            (other, _) => panic!("{other:?}"),
        }
    }
}

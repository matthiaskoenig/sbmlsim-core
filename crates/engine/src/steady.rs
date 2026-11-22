//! The steady state before a simulation, as sbmlsim's `SteadyState` (`simulation/definition.py`)
//! and its executor (`simulator/executor.py` `steady_state`).
//!
//! The model is initialized once, with the pre-initialization values of the request and of the
//! steady state together, and integrated from the time [`STEADY_STATE_FROM`] (0, whatever the
//! start of the simulation, as sbmlsim integrates it from the time 0 of roadrunner: its
//! `steady_state(model, plan)` has `start=0.0`; probed with a rate which reads `time`, the
//! steady state of a simulation from -1000 is the one integrated from 0) over horizons which
//! grow by a factor of ten (1, 10, 100, ... time units, the last one cut at `max_time`), CVODE
//! started again at the
//! beginning of every horizon and stopped exactly at its end. After every horizon the rates of
//! the states (`model_rhs` at the end of the horizon) are compared with the criterion
//! `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|`, every state in the quantity it
//! stores; the first horizon at whose end every state satisfies it is the steady state. A model
//! which does not satisfy it after `max_time` is [`EngineError::Integration`] with the flag
//! [`STEADY_STATE_FLAG`]; a rate which is NaN never satisfies it.
//!
//! The events are active: the events at the time 0 (the initialization) execute first, the
//! triggers are located and the events execute as in a simulation (`events`), the end of a
//! horizon is a stop like an output time, so an event whose trigger becomes true at the end of a
//! horizon executes once; CVODE starts again at the zero of a root function of a discontinuity
//! of the math, as in an output of times (`roots`: every horizon an interval of libroadrunner's
//! count of roots and of the budget of steps). The simulation then starts at its start from the state at the
//! steady state, with the triggers as they were at its end (an event whose trigger is false at
//! the end of the steady state and true at the start executes at the start) and every pending
//! execution with the time it had left (sbmlsim's roadrunner keeps such an execution at the time
//! of its clock; this engine moves it with the simulation).

use sbmlsim_abi::{IntegratorSettings, SteadyState, SteadyStateAfter};
use sbmlsim_csolvers::ffi::{CV_NORMAL, CV_ROOT_RETURN, CV_TSTOP_RETURN};

use crate::events::{Events, Probe};
use crate::roots::RootGuard;
use crate::segments::too_close;
use crate::solver::{Integrator, Problem};
use crate::{EngineError, STEADY_STATE_FLAG};

/// The time the integration to a steady state starts at.
pub const STEADY_STATE_FROM: f64 = 0.0;

/// Integrates the model from [`STEADY_STATE_FROM`] with the state `y` (of `problem.n` values) to
/// a steady state; `y` and `problem.p` are the state and the parameters at the steady state.
/// `start` and `end` are the times of the simulation which follows: the pending executions are
/// moved to `start`, the windows of the events are the ones of the simulation again.
#[allow(clippy::too_many_arguments)]
pub(crate) fn steady_state<'a, const DAE: bool, H: crate::delay::Mode>(
    cvode: Option<&mut Integrator<DAE, H>>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    start: f64,
    end: f64,
    y: &mut [f64],
    steady: &SteadyState,
    mut events: Option<&mut Events<'a, DAE>>,
    model_id: &str,
    rates: &mut Vec<f64>,
    constraints: Option<&mut crate::solver::Constraints<'a>>,
) -> Result<(), EngineError> {
    let m = problem.n_model;
    let from = STEADY_STATE_FROM;
    if let Some(events) = events.as_deref_mut() {
        // the events at the time 0 are part of the initial state
        events.execute_at(from, &mut y[..m], &mut problem.p, Probe::Exact)?;
    }
    let t = equilibrate(
        cvode,
        problem,
        settings,
        from,
        y,
        &Criterion {
            absolute_tolerance: steady.absolute_tolerance,
            relative_tolerance: steady.relative_tolerance,
            max_time: steady.max_time,
        },
        events.as_deref_mut(),
        model_id,
        rates,
        constraints,
    )?;
    if let Some(events) = events {
        events.rebase(t, start);
        events.set_window(start, end);
    }
    Ok(())
}

/// The criterion of a steady state, which [`SteadyState`] and [`SteadyStateAfter`] share.
struct Criterion {
    absolute_tolerance: f64,
    relative_tolerance: f64,
    max_time: f64,
}

/// Integrates the model from `from` with the state `y` over horizons which grow by a factor of
/// ten until the criterion holds at the end of one of them, and answers the time it reached.
///
/// This is the loop the pre-equilibration and the steady state after the end share; what
/// differs is what the caller does before it (the events of the initialization, or the pending
/// executions moved to `from`) and after it (the pending executions moved to the start of the
/// simulation, or nothing, the simulation being over).
#[allow(clippy::too_many_arguments)]
fn equilibrate<'a, const DAE: bool, H: crate::delay::Mode>(
    mut cvode: Option<&mut Integrator<DAE, H>>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    from: f64,
    y: &mut [f64],
    criterion: &Criterion,
    mut events: Option<&mut Events<'a, DAE>>,
    model_id: &str,
    rates: &mut Vec<f64>,
    mut constraints: Option<&mut crate::solver::Constraints<'a>>,
) -> Result<f64, EngineError> {
    let m = problem.n_model;
    let model = problem.model;
    rates.clear();
    rates.resize(m, 0.0);
    let mut t = from;
    let mut horizon = 1.0f64;
    loop {
        let stop = from + horizon.min(criterion.max_time);
        match (cvode.as_deref_mut(), events.as_deref_mut()) {
            (Some(cvode), Some(events)) => {
                events.set_window(from, stop);
                with_events(cvode, problem, settings, t, stop, y, events)?;
            }
            (Some(cvode), None) if !too_close(t, stop) => {
                cvode.start(t, y, stop, settings, 0)?;
                cvode.integrate(problem, stop, CV_NORMAL)?;
                y.copy_from_slice(cvode.state());
            }
            _ => {}
        }
        t = stop;
        problem.counts.steady_state_rhs += 1;
        model.rhs(t, &y[..m], &problem.p, rates);
        model.check_error()?;
        if DAE
            && let Some(projection) = events
                .as_deref_mut()
                .and_then(|events| events.constraints.as_deref_mut())
                .or(constraints.as_deref_mut())
        {
            projection.algebraic_rates(t, &y[..m], &problem.p, rates)?;
        }
        let steady_now = rates.iter().zip(&y[..m]).all(|(r, x)| {
            r.abs() <= criterion.absolute_tolerance + criterion.relative_tolerance * x.abs()
        });
        if steady_now {
            return Ok(t);
        }
        if t - from >= criterion.max_time {
            let worst = rates.iter().map(|r| r.abs()).fold(0.0, f64::max);
            return Err(EngineError::Integration {
                flag: STEADY_STATE_FLAG,
                time: t,
                message: format!(
                    "the model {model_id} did not reach a steady state up to the time {}, the \
                     largest rate of change is {worst:e}",
                    criterion.max_time
                ),
            });
        }
        horizon *= 10.0;
    }
}

/// Integrates the model to a steady state **after** the end of a simulation, from the state `y`
/// the timecourse ended in at the time `end`, with the triggers of its events as it left them.
///
/// The pending executions are moved from `end` to [`SteadyStateAfter::from`], as the
/// pre-equilibration moves them to the start of the simulation which follows it, and nothing
/// follows this one: the caller records the row of the steady state and the request is done.
///
/// The events are not executed at `from` first, unlike the pre-equilibration, where they are
/// the events of the initialization at the time 0: the model is in the middle of its
/// simulation here, so an event which executed during the timecourse does not execute again at
/// the same threshold, which is what libroadrunner does when it continues from the state of its
/// model.
#[allow(clippy::too_many_arguments)]
pub(crate) fn steady_state_after<'a, const DAE: bool, H: crate::delay::Mode>(
    cvode: Option<&mut Integrator<DAE, H>>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    end: f64,
    y: &mut [f64],
    steady: &SteadyStateAfter,
    mut events: Option<&mut Events<'a, DAE>>,
    model_id: &str,
    rates: &mut Vec<f64>,
    constraints: Option<&mut crate::solver::Constraints<'a>>,
) -> Result<(), EngineError> {
    let from = steady.from;
    if let Some(events) = events.as_deref_mut() {
        events.rebase(end, from);
    }
    equilibrate(
        cvode,
        problem,
        settings,
        from,
        y,
        &Criterion {
            absolute_tolerance: steady.absolute_tolerance,
            relative_tolerance: steady.relative_tolerance,
            max_time: steady.max_time,
        },
        events,
        model_id,
        rates,
        constraints,
    )?;
    Ok(())
}

/// Integrates a model with events from `t` to `stop` without output: CVODE locates the zeros
/// of the root functions and stops at every pending execution, the events execute at every stop
/// ([`Events::execute`], `stop` included) and CVODE starts again after an execution and at a
/// zero of a root function of a discontinuity.
fn with_events<const DAE: bool, H: crate::delay::Mode>(
    cvode: &mut Integrator<DAE, H>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    mut t: f64,
    stop: f64,
    y: &mut [f64],
    events: &mut Events<'_, DAE>,
) -> Result<(), EngineError> {
    let m = problem.n_model;
    let mut started = false;
    let mut current = stop;
    // a horizon is an interval of libroadrunner's count of roots and of the budget of steps
    let mut guard = RootGuard::new(t, settings, true, cvode.steps(), 0.0);
    while !(t >= stop || too_close(t, stop)) {
        let tstop = events.next_time().map_or(stop, |x| x.min(stop));
        if too_close(t, tstop) {
            t = tstop;
        } else {
            if !started {
                cvode.start(t, y, tstop, settings, problem.n_roots)?;
                started = true;
                current = tstop;
            } else if tstop != current {
                if tstop < current {
                    cvode.start(t, y, tstop, settings, problem.n_roots)?;
                } else {
                    cvode.set_stop(tstop)?;
                }
                current = tstop;
            }
            guard.limit(cvode, t)?;
            let (tret, flag) = cvode.integrate(problem, tstop, CV_NORMAL)?;
            t = if flag == CV_TSTOP_RETURN || (flag != CV_ROOT_RETURN && tret >= tstop) {
                tstop
            } else {
                tret
            };
            y.copy_from_slice(cvode.state());
            // a zero of a root function of a discontinuity: CVODE starts again from the state
            // there (unless the guard of the restarts ignores it)
            if flag == CV_ROOT_RETURN {
                let found = cvode.at_discontinuity(problem)?;
                if guard.root(t, found, cvode.steps()) {
                    started = false;
                }
            }
        }
        if events.execute(t, &mut y[..m], &mut problem.p, false)? {
            started = false;
            guard.executed(t, cvode.steps());
        }
    }
    Ok(())
}

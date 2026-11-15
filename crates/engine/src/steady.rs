//! The steady state before a simulation, as sbmlsim's `SteadyState` (`simulation/definition.py`)
//! and its executor (`simulator/executor.py` `steady_state`).
//!
//! The model is initialized once, with the pre-initialization values of the request and of the
//! steady state together, and integrated from the start over horizons which grow by a factor of
//! ten (1, 10, 100, ... time units, the last one cut at `max_time`), CVODE started again at the
//! beginning of every horizon and stopped exactly at its end. After every horizon the rates of
//! the states (`model_rhs` at the end of the horizon) are compared with the criterion
//! `|dx/dt| <= absolute_tolerance + relative_tolerance * |x|`, every state in the quantity it
//! stores; the first horizon at whose end every state satisfies it is the steady state. A model
//! which does not satisfy it after `max_time` is [`EngineError::Integration`] with the flag
//! [`STEADY_STATE_FLAG`]; a rate which is NaN never satisfies it.
//!
//! The events are active: the events at the start (the initialization) execute first, the
//! triggers are located and the events execute as in a simulation (`events`), the end of a
//! horizon is a stop like an output time, so an event whose trigger becomes true at the end of a
//! horizon executes once. The simulation then starts at its start from the state at the steady
//! state, with the triggers as they were at its end (an event whose trigger is false at the end
//! of the steady state and true at the start executes at the start) and every pending execution
//! with the time it had left.

use sbmlsim_abi::{IntegratorSettings, SteadyState};
use sbmlsim_csolvers::ffi::{CV_NORMAL, CV_ROOT_RETURN, CV_TSTOP_RETURN};

use crate::cvode::{Cvode, Problem};
use crate::events::{Events, Probe};
use crate::segments::too_close;
use crate::{EngineError, STEADY_STATE_FLAG};

/// Integrates the model from `start` with the state `y` (of `problem.n` values) to a steady
/// state; `y` and `problem.p` are the state and the parameters at the steady state. `end` is
/// the end of the simulation which follows, whose windows the events get back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn steady_state(
    mut cvode: Option<&mut Cvode>,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    start: f64,
    end: f64,
    y: &mut [f64],
    steady: &SteadyState,
    mut events: Option<&mut Events<'_>>,
    model_id: &str,
) -> Result<(), EngineError> {
    let m = problem.n_model;
    let model = problem.model;
    if let Some(events) = events.as_deref_mut() {
        // the events at the start are part of the initial state
        events.execute_at(start, &mut y[..m], &mut problem.p, Probe::Exact)?;
    }
    let mut rates = vec![0.0; m];
    let mut t = start;
    let mut horizon = 1.0f64;
    loop {
        let stop = start + horizon.min(steady.max_time);
        match (cvode.as_deref_mut(), events.as_deref_mut()) {
            (Some(cvode), Some(events)) => {
                events.set_window(start, stop);
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
        model.rhs(t, &y[..m], &problem.p, &mut rates);
        let steady_now = rates.iter().zip(&y[..m]).all(|(r, x)| {
            r.abs() <= steady.absolute_tolerance + steady.relative_tolerance * x.abs()
        });
        if steady_now {
            break;
        }
        if t - start >= steady.max_time {
            let worst = rates.iter().map(|r| r.abs()).fold(0.0, f64::max);
            return Err(EngineError::Integration {
                flag: STEADY_STATE_FLAG,
                time: t,
                message: format!(
                    "the model {model_id} did not reach a steady state up to the time {}, the \
                     largest rate of change is {worst:e}",
                    steady.max_time
                ),
            });
        }
        horizon *= 10.0;
    }
    if let Some(events) = events {
        events.rebase(t, start);
        events.set_window(start, end);
    }
    Ok(())
}

/// Integrates a model with events from `t` to `stop` without output: CVODE locates the zeros
/// of the root functions and stops at every pending execution, the events execute at every stop
/// ([`Events::execute`], `stop` included) and CVODE starts again after an execution.
fn with_events(
    cvode: &mut Cvode,
    problem: &mut Problem<'_>,
    settings: &IntegratorSettings,
    mut t: f64,
    stop: f64,
    y: &mut [f64],
    events: &mut Events<'_>,
) -> Result<(), EngineError> {
    let m = problem.n_model;
    let mut started = false;
    let mut current = stop;
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
            let (tret, flag) = cvode.integrate(problem, tstop, CV_NORMAL)?;
            t = if flag == CV_TSTOP_RETURN || (flag != CV_ROOT_RETURN && tret >= tstop) {
                tstop
            } else {
                tret
            };
            y.copy_from_slice(cvode.state());
        }
        if events.execute(t, &mut y[..m], &mut problem.p, false)? {
            started = false;
        }
    }
    Ok(())
}

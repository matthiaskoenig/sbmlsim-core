//! The events of a model: their triggers, the pending executions and the execution of the
//! events at one time.
//!
//! The semantics are the ones of SBML Level 3 Version 2 with the choices of sbmlsim's executors
//! (`simulator/changes.py` `fire_events`, roadrunner's events):
//!
//! - An event is triggered when its trigger changes from false to true. Before the start, a
//!   trigger has its initial value, so an event whose initial value is false and whose trigger
//!   is true at the start is triggered at the start.
//! - A triggered event without a delay is pending at its time; one with a delay at the time
//!   plus the delay, evaluated when it is triggered. The values of its assignments are
//!   evaluated when it is triggered (`useValuesFromTriggerTime`) or when it executes. An event
//!   may be pending more than once.
//! - A pending event whose trigger is not persistent is cancelled when its trigger becomes
//!   false.
//! - The pending events whose time has come execute one at a time: the highest priority first
//!   (an event without a priority has the priority 0), a tie of equal priorities chosen by the
//!   generator of the request (`rand_pcg::Pcg64` seeded with its `seed`); the priorities and
//!   the triggers are evaluated again after every execution, so an execution may trigger or
//!   cancel others at the same time. One event executes at most [`MAX_EXECUTIONS_PER_EVENT`]
//!   times at one time, beyond it the simulation fails with [`EVENT_LIMIT_FLAG`] (events which
//!   trigger each other in a cycle).
//!
//! A trigger is evaluated exactly, with `model_trigger`, where the integration stops: at a zero
//! of a root function, at an output time, at the time of a pending event, at the start and at
//! the end. The events at a time execute in two rounds ([`Events::execute`]): first with the
//! triggers at the time ([`Probe::Exact`]), then with the triggers right after it
//! ([`Probe::After`]): a trigger which is false at the time but true at the time plus
//! [`PROBE_RTOL`] times the magnitude of the times of the simulation, with the state moved
//! along its rate for that time, is triggered at the time. That is a strict comparison exactly
//! at its threshold, e.g. `time > 1` at 1, where CVODE places the zero of `time - 1` and where
//! the trigger is false; such a trigger which was true before the time falls and rises there
//! again. An output at that very time is written between the two rounds, so it sees the state
//! before such an event (as libroadrunner and sbmlsim show it); at the end the second round
//! looks [`END_RTOL`] ahead ([`Probe::End`]) and the output is written after it (sbmlsim's
//! event at the end).
//!
//! Both windows are relative: the absolute window is the rtol times the magnitude of the time
//! scale of the simulation, `max(|start|, |end|, end - start)` (1 when that is 0), so a
//! simulation from 0 to 1e6 looks 1e-6 ahead after a stop and 1e-3 at its end. An event within
//! the window after a stop executes at the stop.
//!
//! Limitation: within the second round the triggers are the ones right after the time, also
//! after an execution, so an event executed in that round which sets a trigger exactly to its
//! threshold, where the trigger is false at the time but true right after it, does not trigger
//! that event again (it saw no fall); likewise a trigger which touches its threshold and rises
//! again is a fall and a rise only where the integration stops at the touch. Neither occurs in
//! the SBML Test Suite.

use rand_pcg::Pcg64;
use rand_pcg::rand_core::{Rng, SeedableRng};
use sbmlsim_abi::{EventInfo, ModelDescription};

use crate::model::ModelFns;
use crate::{DELAY_FLAG, EVENT_LIMIT_FLAG, EngineError};

/// The most executions of one event at one time.
pub const MAX_EXECUTIONS_PER_EVENT: u32 = 1000;

/// The resolution of the probe of a trigger, relative to the magnitude of the times of the
/// simulation: a trigger which becomes true within it after a time is triggered at that time.
pub const PROBE_RTOL: f64 = 1e-12;

/// The resolution of the time at the end, relative to the magnitude of the times of the
/// simulation (sbmlsim's `TIME_RTOL`): an event within it after the end is at the end, as
/// sbmlsim's `_event_at_the_end` has it (the events of a period which CVODE locates a little
/// late each, case 00967, end there).
pub const END_RTOL: f64 = 1e-9;

/// A pending execution of an event.
#[derive(Debug, Clone)]
struct Pending {
    /// The index of the event.
    event: u32,
    /// The time of the execution.
    time: f64,
    /// The values of the assignments, evaluated when the event was triggered, or `None` to
    /// evaluate them at the execution.
    values: Option<Vec<f64>>,
}

/// The events of a simulation: the last values of the triggers and the pending executions.
pub(crate) struct Events<'a> {
    model: &'a dyn ModelFns,
    info: &'a [EventInfo],
    /// The value of every trigger where it was evaluated last.
    armed: Vec<bool>,
    /// The pending executions, in the order they were triggered.
    pending: Vec<Pending>,
    rng: Pcg64,
    /// The probe of a trigger: the time after a time at which it is evaluated again.
    probe: f64,
    /// The resolution of the time at the end.
    end_probe: f64,
    /// The executions of every event at the time `executions_at`.
    executions: Vec<u32>,
    executions_at: f64,
    /// Scratch vectors: the triggers, the state and the rate of the probe.
    current: Vec<bool>,
    y_probe: Vec<f64>,
    ydot: Vec<f64>,
}

/// The values of the triggers at a time: at the time itself, or right after it (a trigger which
/// is false at the time but true at the time plus the probe counts as true).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// At the time.
    Exact,
    /// Right after the time, within [`PROBE_RTOL`].
    After,
    /// Right after the end, within [`END_RTOL`]: also a pending execution within it is due.
    End,
}

impl<'a> Events<'a> {
    /// The events of a model for a simulation from `start` to `end` with the seed of the order
    /// of ties; the triggers have their initial values.
    pub fn new(
        model: &'a dyn ModelFns,
        desc: &'a ModelDescription,
        seed: u64,
        start: f64,
        end: f64,
    ) -> Events<'a> {
        let scale = start.abs().max(end.abs()).max(end - start);
        let scale = if scale > 0.0 && scale.is_finite() {
            scale
        } else {
            1.0
        };
        let n = desc.states.len();
        Events {
            model,
            info: &desc.events,
            armed: desc.events.iter().map(|e| e.initial_value).collect(),
            pending: Vec::new(),
            rng: Pcg64::seed_from_u64(seed),
            probe: PROBE_RTOL * scale,
            end_probe: END_RTOL * scale,
            executions: vec![0; desc.events.len()],
            executions_at: f64::NAN,
            current: vec![false; desc.events.len()],
            y_probe: vec![0.0; n],
            ydot: vec![0.0; n],
        }
    }

    /// The time of the next pending execution.
    pub fn next_time(&self) -> Option<f64> {
        self.pending.iter().map(|e| e.time).reduce(f64::min)
    }

    /// The name of event `i` in a message.
    fn name(&self, i: usize) -> String {
        match self.info[i].id.as_str() {
            "" => format!("#{i}"),
            id => id.to_string(),
        }
    }

    /// Evaluates the triggers at `t` into `current`, or right after `t`.
    fn evaluate(&mut self, t: f64, y: &[f64], p: &[f64], probe: Probe) {
        let model = self.model;
        let mut probed = false;
        for i in 0..self.info.len() {
            let exact = model.trigger(i as u32, t, y, p);
            self.current[i] = exact;
            let delta = match probe {
                Probe::Exact => continue,
                Probe::After => self.probe,
                Probe::End => self.end_probe,
            };
            if exact {
                continue;
            }
            if !probed {
                // the state at t + delta, moved along its rate
                model.rhs(t, y, p, &mut self.ydot);
                for ((q, x), r) in self.y_probe.iter_mut().zip(y).zip(&self.ydot) {
                    *q = x + delta * r;
                }
                probed = true;
            }
            self.current[i] = model.trigger(i as u32, t + delta, &self.y_probe, p);
        }
    }

    /// Executes the events at the time `t`: [`Events::execute_at`] with the triggers at `t`,
    /// then right after `t` ([`Probe::End`] at the end).
    pub fn execute(
        &mut self,
        t: f64,
        y: &mut [f64],
        p: &mut [f64],
        at_end: bool,
    ) -> Result<bool, EngineError> {
        let at = self.execute_at(t, y, p, Probe::Exact)?;
        let after = if at_end { Probe::End } else { Probe::After };
        let after = self.execute_at(t, y, p, after)?;
        Ok(at || after)
    }

    /// Executes the events at the time `t`: the triggers are evaluated (at `t` or right after
    /// it), the new triggers are pending and the pending executions of the time execute, one at
    /// a time, until none is left. Returns whether an event executed; `y` and `p` are the state
    /// and the parameters after them (with the assignment rules evaluated).
    pub fn execute_at(
        &mut self,
        t: f64,
        y: &mut [f64],
        p: &mut [f64],
        probe: Probe,
    ) -> Result<bool, EngineError> {
        let model = self.model;
        let mut executed = false;
        if t != self.executions_at {
            self.executions.fill(0);
            self.executions_at = t;
        }
        loop {
            model.assign(t, y, p);
            self.evaluate(t, y, p, probe);
            // a pending execution of an event whose trigger is not persistent and false is
            // cancelled
            let (info, current) = (self.info, &self.current);
            self.pending
                .retain(|e| info[e.event as usize].persistent || current[e.event as usize]);
            for i in 0..self.info.len() {
                if self.armed[i] || !self.current[i] {
                    continue;
                }
                let event = &self.info[i];
                let time = if event.has_delay {
                    let delay = model.delay(i as u32, t, y, p);
                    if delay.is_nan() || delay < 0.0 {
                        return Err(EngineError::Integration {
                            flag: DELAY_FLAG,
                            time: t,
                            message: format!(
                                "the delay {delay} of the event {} is not a time of 0 or more",
                                self.name(i)
                            ),
                        });
                    }
                    t + delay
                } else {
                    t
                };
                let values = event.use_values_from_trigger_time.then(|| {
                    let mut values = vec![0.0; event.n_assignments as usize];
                    model.event_values(i as u32, t, y, p, &mut values);
                    values
                });
                self.pending.push(Pending {
                    event: i as u32,
                    time,
                    values,
                });
            }
            self.armed.copy_from_slice(&self.current);
            let horizon = if probe == Probe::End {
                self.end_probe
            } else {
                0.0
            };
            let Some(k) = self.choose(t, horizon, y, p) else {
                break;
            };
            let chosen = self.pending.remove(k);
            let i = chosen.event as usize;
            self.executions[i] += 1;
            if self.executions[i] > MAX_EXECUTIONS_PER_EVENT {
                return Err(EngineError::Integration {
                    flag: EVENT_LIMIT_FLAG,
                    time: t,
                    message: format!(
                        "the event {} executes more than {MAX_EXECUTIONS_PER_EVENT} times at \
                         one time (events which trigger each other in a cycle)",
                        self.name(i)
                    ),
                });
            }
            let values = chosen.values.unwrap_or_else(|| {
                let mut values = vec![0.0; self.info[i].n_assignments as usize];
                model.event_values(i as u32, t, y, p, &mut values);
                values
            });
            model.event_apply(i as u32, t, &values, y, p);
            executed = true;
        }
        Ok(executed)
    }

    /// The position in `pending` of the execution which comes next at `t`: of the executions
    /// whose time has come (up to `t + horizon`), the one of the highest priority (NaN is the
    /// lowest), a tie chosen by the generator; `None` when no execution is due.
    fn choose(&mut self, t: f64, horizon: f64, y: &[f64], p: &[f64]) -> Option<usize> {
        let mut best: Vec<usize> = Vec::new();
        let mut best_priority = f64::NEG_INFINITY;
        for (k, e) in self.pending.iter().enumerate() {
            if e.time > t + horizon {
                continue;
            }
            let priority = self.model.priority(e.event, t, y, p);
            let priority = if priority.is_nan() {
                f64::NEG_INFINITY
            } else {
                priority
            };
            if best.is_empty() || priority > best_priority {
                best.clear();
                best.push(k);
                best_priority = priority;
            } else if priority == best_priority {
                best.push(k);
            }
        }
        match best.len() {
            0 => None,
            1 => Some(best[0]),
            n => {
                // an index below n, uniform up to 2^-64
                let r = ((u128::from(self.rng.next_u64()) * n as u128) >> 64) as usize;
                Some(best[r])
            }
        }
    }
}

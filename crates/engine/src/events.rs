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
//! again. A trigger which a probe found true stays true up to the time the probe looked at,
//! while it is still true there with the state of a later stop moved along its rate (an
//! execution may change what it reads): a stop before that time (a step of the integrator inside
//! the window, a zero of a root function at the same time again) does not see it fall and rise
//! again, which would execute its event at every such stop. An output at that very time is written between the two rounds, so it sees
//! the state before such an event (as libroadrunner and sbmlsim show it); at the end the second
//! round looks [`END_RTOL`] ahead ([`Probe::End`]) and the output is written after it (sbmlsim's
//! event at the end).
//!
//! At a change (`changes`) the integration stops exactly at its time without executing the
//! events there; after the change the events execute with the triggers before the change
//! ([`Probe::Change`]: the events the change makes true, with their priorities, persistence
//! and values, as sbmlsim's `fire_events`), then the output of the state after the change is
//! written, then the executions which were pending at its time and the strict comparisons
//! execute ([`Probe::After`]). An event triggered by a change which has a delay is pending as
//! any other: a known difference to sbmlsim, whose executor reports such an event and does
//! not fire it (ruling R34).
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
    /// Whether the execution waits for the next round ([`Probe::Change`]).
    held: bool,
}

/// The events of a simulation: the last values of the triggers and the pending executions.
pub(crate) struct Events<'a> {
    model: &'a dyn ModelFns,
    info: &'a [EventInfo],
    /// The value of every trigger where it was evaluated last.
    armed: Vec<bool>,
    /// For a trigger which a probe found true (false at the time, true right after it), the
    /// time the probe looked at; up to it the trigger is true (it rises there), so a stop
    /// before it does not see it fall. `-inf` for every other trigger.
    probed_until: Vec<f64>,
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
    /// The calls of the right hand side by the probes.
    probe_rhs: u64,
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
    /// At the time, right after a change: the triggers as [`Probe::Exact`], but only the
    /// executions this round triggers are due; the executions which were pending before it
    /// wait for the next round (sbmlsim: the events a change triggers fire with the change, an
    /// execution pending at its time fires when the integration continues, after the output of
    /// the state after the change).
    Change,
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
        let n = desc.states.len();
        let mut events = Events {
            model,
            info: &desc.events,
            armed: desc.events.iter().map(|e| e.initial_value).collect(),
            probed_until: vec![f64::NEG_INFINITY; desc.events.len()],
            pending: Vec::new(),
            rng: Pcg64::seed_from_u64(seed),
            probe: 0.0,
            end_probe: 0.0,
            executions: vec![0; desc.events.len()],
            executions_at: f64::NAN,
            current: vec![false; desc.events.len()],
            y_probe: vec![0.0; n],
            ydot: vec![0.0; n],
            probe_rhs: 0,
        };
        events.set_window(start, end);
        events
    }

    /// Sets the windows of the probes for an integration from `start` to `end`: [`PROBE_RTOL`]
    /// and [`END_RTOL`] times the magnitude of its times (1 when that is 0).
    pub fn set_window(&mut self, start: f64, end: f64) {
        let scale = start.abs().max(end.abs()).max(end - start);
        let scale = if scale > 0.0 && scale.is_finite() {
            scale
        } else {
            1.0
        };
        self.probe = PROBE_RTOL * scale;
        self.end_probe = END_RTOL * scale;
    }

    /// Moves the time of the events from `from` to `to`: every pending execution keeps the time
    /// it has left (an integration to a steady state which ends at `from` is followed by the
    /// simulation from `to`).
    pub fn rebase(&mut self, from: f64, to: f64) {
        for e in &mut self.pending {
            e.time = to + (e.time - from);
        }
        for t in &mut self.probed_until {
            *t = to + (*t - from);
        }
        self.executions_at = f64::NAN;
    }

    /// The model of the events.
    pub fn model(&self) -> &'a dyn ModelFns {
        self.model
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

    /// The calls of the right hand side by the probes of the triggers.
    pub fn probe_rhs(&self) -> u64 {
        self.probe_rhs
    }

    /// Evaluates the triggers at `t` into `current`, or right after `t`.
    ///
    /// A trigger which a probe found true at an earlier time stays true up to the time that
    /// probe looked at (`probed_until`) while it is still true there, with the state now moved
    /// along its rate to it (an execution may have changed what it reads): it rises there, so a
    /// stop before it (a step of the integrator, a zero of a root function at the same time
    /// again) does not see it fall and rise again.
    fn evaluate(&mut self, t: f64, y: &[f64], p: &[f64], probe: Probe) {
        // the rate at t and the state moved along it for a time, computed when needed and once
        // per time
        let mut moved = None;
        let model = self.model;
        for i in 0..self.info.len() {
            let exact = model.trigger(i as u32, t, y, p);
            self.current[i] = exact;
            if exact {
                self.probed_until[i] = f64::NEG_INFINITY;
                continue;
            }
            if self.armed[i] && t <= self.probed_until[i] {
                let delta = self.probed_until[i] - t;
                if self.probe_trigger(i, t, delta, y, p, &mut moved) {
                    self.current[i] = true;
                    continue;
                }
                self.probed_until[i] = f64::NEG_INFINITY;
            }
            let delta = match probe {
                Probe::Exact | Probe::Change => continue,
                Probe::After => self.probe,
                Probe::End => self.end_probe,
            };
            self.current[i] = self.probe_trigger(i, t, delta, y, p, &mut moved);
            if self.current[i] {
                self.probed_until[i] = t + delta;
            }
        }
    }

    /// The trigger `i` at `t + delta` with the state moved along its rate at `t` for `delta`;
    /// `moved` is the time `y_probe` was moved for (`None` before the rate at `t` is in `ydot`).
    fn probe_trigger(
        &mut self,
        i: usize,
        t: f64,
        delta: f64,
        y: &[f64],
        p: &[f64],
        moved: &mut Option<f64>,
    ) -> bool {
        let model = self.model;
        if moved.is_none() {
            model.rhs(t, y, p, &mut self.ydot);
            self.probe_rhs += 1;
        }
        if *moved != Some(delta) {
            for ((q, x), r) in self.y_probe.iter_mut().zip(y).zip(&self.ydot) {
                *q = x + delta * r;
            }
            *moved = Some(delta);
        }
        model.trigger(i as u32, t + delta, &self.y_probe, p)
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
        if t != self.executions_at {
            self.executions.fill(0);
            self.executions_at = t;
        }
        if probe == Probe::Change {
            for e in &mut self.pending {
                e.held = true;
            }
        }
        let result = self.rounds(t, y, p, probe);
        for e in &mut self.pending {
            e.held = false;
        }
        result
    }

    /// The executions of [`Events::execute_at`].
    fn rounds(
        &mut self,
        t: f64,
        y: &mut [f64],
        p: &mut [f64],
        probe: Probe,
    ) -> Result<bool, EngineError> {
        let model = self.model;
        let mut executed = false;
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
                    held: false,
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
            if e.time > t + horizon || e.held {
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

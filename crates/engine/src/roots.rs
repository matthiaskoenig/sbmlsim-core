//! The restarts of CVODE at the zeros of the root functions of the discontinuities of the math,
//! with libroadrunner's guard against roots which chatter, and the budget of steps towards one
//! output time.
//!
//! libroadrunner (`CVODEIntegrator::integrate`) counts, in one call of `integrate` (one interval
//! of an output of times), the returns of CVODE at a root closer than the relative tolerance (as
//! a distance in time) to the last root it applied: from the third such root on (`strikes`, 3,
//! reset by a root further away and by every new interval) a root is ignored, neither its events
//! nor its restart of CVODE, which continues where it is; the time of the last applied root is
//! the one of the last root not ignored. In the output of the steps of the integrator every step
//! is a call of its own, so the count starts again at every step and no root is ignored.
//!
//! [`RootGuard`] applies this count to the restarts at the roots of the discontinuities (every
//! return at a root counts, the events execute whatever the count: the semantics of the events
//! are sbmlsim's), in the output of times and in the horizons of a steady state. Besides, in
//! every output a root of a discontinuity at the time of the last restart at one restarts nothing
//! (CVODE, started at a zero of a root function with a first step below the rounding of the
//! time, returns the same zero again).
//!
//! In the output of times (and in a horizon of a steady state) the steps towards one output time,
//! summed over the returns at roots, the restarts at discontinuities and the executions of events,
//! are bounded by `maximum_num_steps` (CVODE counts the steps of one call, which every return at
//! a root ends). An output time and a change start the budget again, and so do a restart at a
//! root and an execution of an event far (further than the relative tolerance, as a distance in
//! time) from the last applied root, so well separated roots and doses do not share one budget.
//! A restart or an execution within that distance of the last applied root does not: roots
//! which chatter and events which accumulate (a Zeno model, the bouncing ball whose bounces
//! accumulate at one time) end in a result or in [`EngineError::Integration`] with CVODE's flag
//! `CV_TOO_MUCH_WORK`.
//!
//! The output of the steps of the integrator writes the rows of every restart and of every
//! execution; it has no budget of steps (every step is a call of its own), but a bound of
//! chatter: `maximum_num_steps` consecutive stops (steps, roots, stop times), each within the
//! resolution of the time ([`crate::events::END_RTOL`] times the magnitude of the times of the
//! simulation) of the one before it, end the run with [`EngineError::Integration`] and the flag
//! `CV_TOO_MUCH_WORK`. Such stops all fall within one interval of any output of times of a
//! practical resolution, where they would exhaust the budget of steps as well.

use sbmlsim_abi::IntegratorSettings;
use sbmlsim_csolvers::ffi::CV_TOO_MUCH_WORK;

use crate::EngineError;
use crate::cvode::Cvode;

/// libroadrunner's count of roots close to the last applied one in one interval: the third
/// such root is ignored.
pub const STRIKES: i32 = 3;

/// The guard of the restarts at the roots of the discontinuities and of the steps towards one
/// output time (see the module documentation).
pub(crate) struct RootGuard {
    /// Whether libroadrunner's count applies (an output of times, a steady state).
    count: bool,
    /// The distance in time within which a root is close to the last applied one (the relative
    /// tolerance of the settings, as libroadrunner compares it).
    close: f64,
    /// The time of the last applied root.
    last: f64,
    /// The strikes left in the interval.
    strikes: i32,
    /// The time of the last restart at a root of a discontinuity.
    restarted_at: f64,
    /// The time of the last return at a root and whether it was far from the last applied root
    /// before it.
    root_at: f64,
    root_far: bool,
    /// The steps of the simulation when the budget of steps started (`None`: no budget).
    budget_from: Option<u64>,
    /// The budget of steps, and the bound of the consecutive stops within the resolution.
    max_steps: u64,
    /// Whether the bound of chatter applies (the output of the steps of the integrator).
    chatter: bool,
    /// The resolution of the time within which consecutive stops chatter.
    resolution: f64,
    /// The time of the last stop and the number of consecutive stops within the resolution of
    /// the one before them.
    stop_at: f64,
    close_stops: u64,
}

impl RootGuard {
    /// The guard of an integration from `start`: with libroadrunner's count and the budget of
    /// steps for an output of times or a steady state (`times`), else the bound of chatter
    /// within `resolution`; `steps` is [`Cvode::steps`] now.
    pub fn new(
        start: f64,
        settings: &IntegratorSettings,
        times: bool,
        steps: u64,
        resolution: f64,
    ) -> RootGuard {
        RootGuard {
            count: times,
            close: settings.relative_tolerance,
            last: start,
            strikes: STRIKES,
            restarted_at: f64::NAN,
            root_at: f64::NAN,
            root_far: false,
            budget_from: times.then_some(steps),
            max_steps: u64::from(settings.maximum_num_steps),
            chatter: !times,
            resolution,
            stop_at: start,
            close_stops: 0,
        }
    }

    /// A new interval (an output time, a change, a horizon): the count and the budget of steps
    /// start again.
    pub fn interval(&mut self, steps: u64) {
        self.strikes = STRIKES;
        self.start_budget(steps);
    }

    /// The budget of steps starts again.
    fn start_budget(&mut self, steps: u64) {
        if let Some(from) = &mut self.budget_from {
            *from = steps;
        }
    }

    /// Events executed at `t` (CVODE starts again with a count of its own): the budget of steps
    /// starts again when `t` is far from the last applied root (at a root: from the one before
    /// it), not for an execution within the tolerance of it, so executions which accumulate
    /// share one budget.
    pub fn executed(&mut self, t: f64, steps: u64) {
        let far = if t == self.root_at {
            self.root_far
        } else {
            (t - self.last).abs() > self.close
        };
        if far {
            self.start_budget(steps);
        }
    }

    /// A stop of the integration at `t` in the output of the steps of the integrator: the
    /// `maximum_num_steps`-th consecutive stop within the resolution of the time of the one
    /// before it is CVODE's error `CV_TOO_MUCH_WORK` (events or discontinuities which chatter).
    pub fn stop(&mut self, t: f64) -> Result<(), EngineError> {
        if !self.chatter {
            return Ok(());
        }
        if (t - self.stop_at).abs() <= self.resolution {
            self.close_stops += 1;
        } else {
            self.close_stops = 0;
        }
        self.stop_at = t;
        if self.close_stops >= self.max_steps {
            return Err(EngineError::Integration {
                flag: CV_TOO_MUCH_WORK,
                time: t,
                message: format!(
                    "{} consecutive stops of the integration within the resolution of the time \
                     ({:.1e}): events or discontinuities which chatter",
                    self.max_steps, self.resolution
                ),
            });
        }
        Ok(())
    }

    /// A return of CVODE at a zero of a root function at `t`; `discontinuity` is whether it is
    /// a zero of a root function of a discontinuity, `steps` is [`Cvode::steps`] now. Returns
    /// whether CVODE starts again for it. A restart at a root far from the last applied one
    /// (where the count starts again) starts the budget of steps again too, so well separated
    /// roots do not share one budget; roots which chatter, within the tolerance of each other,
    /// do (and so do the executions at them, [`RootGuard::executed`]).
    pub fn root(&mut self, t: f64, discontinuity: bool, steps: u64) -> bool {
        let (applied, far) = if self.count {
            let far = (t - self.last).abs() > self.close;
            self.strikes = if far { STRIKES } else { self.strikes - 1 };
            let applied = far || self.strikes > 0;
            if applied {
                self.last = t;
            }
            (applied, far)
        } else {
            (true, false)
        };
        self.root_at = t;
        self.root_far = far;
        let restart = discontinuity && applied && t != self.restarted_at;
        if restart {
            self.restarted_at = t;
            if far {
                self.start_budget(steps);
            }
        }
        restart
    }

    /// Sets the steps CVODE may take towards the next output time (the budget less the steps
    /// since it started), before a call of CVODE at `t`; an exhausted budget is CVODE's error
    /// `CV_TOO_MUCH_WORK`.
    pub fn limit(&self, cvode: &mut Cvode, t: f64) -> Result<(), EngineError> {
        let Some(from) = self.budget_from else {
            return Ok(());
        };
        let used = cvode.steps() - from;
        if used >= self.max_steps {
            return Err(EngineError::Integration {
                flag: CV_TOO_MUCH_WORK,
                time: t,
                message: format!(
                    "{} steps taken before reaching the next output time (the steps of the \
                     restarts at the roots of discontinuities, of the returns at roots and of the \
                     executions of events close to the last applied root included)",
                    self.max_steps
                ),
            });
        }
        cvode.set_max_steps(self.max_steps - used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(times: bool) -> RootGuard {
        let settings = IntegratorSettings {
            relative_tolerance: 1e-6,
            ..IntegratorSettings::default()
        };
        RootGuard::new(0.0, &settings, times, 0, 1e-9)
    }

    #[test]
    fn the_third_close_root_of_an_interval_is_ignored_as_in_roadrunner() {
        let mut g = guard(true);
        // far from the start: applied; two close ones: applied (strikes 2, 1), the third: not,
        // nor any later close one (the last applied root stays at 1 + 2e-7)
        assert!(g.root(1.0, true, 0));
        assert!(g.root(1.0 + 1e-7, true, 0));
        assert!(g.root(1.0 + 2e-7, true, 0));
        assert!(!g.root(1.0 + 3e-7, true, 0));
        assert!(!g.root(1.0 + 1.1e-6, true, 0));
        // further than the tolerance from the last applied root: the count starts again
        assert!(g.root(1.0 + 1.3e-6, true, 0));
        // a new interval starts the count again
        assert!(g.root(1.0 + 1.4e-6, true, 0));
        assert!(g.root(1.0 + 1.5e-6, true, 0));
        g.interval(0);
        assert!(g.root(1.0 + 1.6e-6, true, 0));
        // every return at a root counts, also one of a trigger, which restarts nothing: the
        // next close root is the third
        assert!(!g.root(1.0 + 1.7e-6, false, 0));
        assert!(!g.root(1.0 + 1.8e-6, true, 0));
    }

    #[test]
    fn the_steps_of_the_integrator_restart_at_every_root_but_one_at_the_same_time() {
        let mut g = guard(false);
        for k in 0..10 {
            assert!(g.root(1.0 + f64::from(k) * 1e-9, true, 0));
        }
        // CVODE returned the zero where it started again
        assert!(!g.root(1.0 + 9e-9, true, 0));
        let mut g = guard(true);
        assert!(g.root(2.0, true, 0));
        assert!(!g.root(2.0, true, 0));
    }

    #[test]
    fn an_execution_close_to_the_last_applied_root_shares_its_budget() {
        let settings = IntegratorSettings {
            relative_tolerance: 1e-6,
            maximum_num_steps: 10,
            ..IntegratorSettings::default()
        };
        let mut g = RootGuard::new(0.0, &settings, true, 0, 1e-9);
        // a root far from the start, its execution: the budget starts at 5
        g.root(1.0, false, 5);
        g.executed(1.0, 5);
        assert_eq!(g.budget_from, Some(5));
        // the next bounce, close to it: no new budget, also for an execution at a stop which is
        // no root
        g.root(1.0 + 1e-7, false, 8);
        g.executed(1.0 + 1e-7, 8);
        g.executed(1.0 + 2e-7, 9);
        assert_eq!(g.budget_from, Some(5));
        // far from the last applied root: a new budget
        g.root(2.0, false, 12);
        g.executed(2.0, 12);
        assert_eq!(g.budget_from, Some(12));
        g.executed(3.0, 13);
        assert_eq!(g.budget_from, Some(13));
    }

    #[test]
    fn consecutive_stops_within_the_resolution_end_the_steps_of_the_integrator() {
        let settings = IntegratorSettings {
            maximum_num_steps: 4,
            ..IntegratorSettings::default()
        };
        // an output of times has no bound of chatter
        let mut g = RootGuard::new(0.0, &settings, true, 0, 1e-9);
        for k in 0..10 {
            g.stop(1.0 + f64::from(k) * 1e-12).unwrap();
        }
        let mut g = RootGuard::new(0.0, &settings, false, 0, 1e-9);
        g.stop(1.0).unwrap();
        // three close stops, then a far one: the count starts again
        for k in 1..4 {
            g.stop(1.0 + f64::from(k) * 1e-12).unwrap();
        }
        g.stop(2.0).unwrap();
        for k in 1..4 {
            g.stop(2.0 + f64::from(k) * 1e-12).unwrap();
        }
        match g.stop(2.0 + 4e-12) {
            Err(EngineError::Integration { flag, time, .. }) => {
                assert_eq!(flag, CV_TOO_MUCH_WORK);
                assert_eq!(time, 2.0 + 4e-12);
            }
            other => panic!("{other:?}"),
        }
    }
}

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
//! time, returns the same zero again), and in the output of times the steps towards one output
//! time, summed over the returns at roots without an execution and the restarts at
//! discontinuities, are bounded by `maximum_num_steps` (CVODE counts the steps of one call, which
//! every return at a root ends), so roots which chatter end in a result or in
//! [`EngineError::Integration`] with CVODE's flag `CV_TOO_MUCH_WORK`. The output of the steps of
//! the integrator writes the rows of every restart and is bounded by the limit of the output.

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
    /// The steps of the simulation when the budget of steps started (`None`: no budget).
    budget_from: Option<u64>,
    /// The budget of steps.
    max_steps: u64,
}

impl RootGuard {
    /// The guard of an integration from `start`: with libroadrunner's count and the budget of
    /// steps for an output of times or a steady state (`times`), else neither; `steps` is
    /// [`Cvode::steps`] now.
    pub fn new(start: f64, settings: &IntegratorSettings, times: bool, steps: u64) -> RootGuard {
        RootGuard {
            count: times,
            close: settings.relative_tolerance,
            last: start,
            strikes: STRIKES,
            restarted_at: f64::NAN,
            budget_from: times.then_some(steps),
            max_steps: u64::from(settings.maximum_num_steps),
        }
    }

    /// A new interval (an output time, a change, a horizon): the count and the budget of steps
    /// start again.
    pub fn interval(&mut self, steps: u64) {
        self.strikes = STRIKES;
        self.executed(steps);
    }

    /// An event executed (CVODE starts again with a count of its own): the budget of steps
    /// starts again.
    pub fn executed(&mut self, steps: u64) {
        if let Some(from) = &mut self.budget_from {
            *from = steps;
        }
    }

    /// A return of CVODE at a zero of a root function at `t`; `discontinuity` is whether it is
    /// a zero of a root function of a discontinuity. Returns whether CVODE starts again for it.
    pub fn root(&mut self, t: f64, discontinuity: bool) -> bool {
        let applied = if self.count {
            let far = (t - self.last).abs() > self.close;
            self.strikes = if far { STRIKES } else { self.strikes - 1 };
            let applied = far || self.strikes > 0;
            if applied {
                self.last = t;
            }
            applied
        } else {
            true
        };
        let restart = discontinuity && applied && t != self.restarted_at;
        if restart {
            self.restarted_at = t;
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
                     restarts at the roots of discontinuities and of the returns at roots \
                     included)",
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
        RootGuard::new(0.0, &settings, times, 0)
    }

    #[test]
    fn the_third_close_root_of_an_interval_is_ignored_as_in_roadrunner() {
        let mut g = guard(true);
        // far from the start: applied; two close ones: applied (strikes 2, 1), the third: not,
        // nor any later close one (the last applied root stays at 1 + 2e-7)
        assert!(g.root(1.0, true));
        assert!(g.root(1.0 + 1e-7, true));
        assert!(g.root(1.0 + 2e-7, true));
        assert!(!g.root(1.0 + 3e-7, true));
        assert!(!g.root(1.0 + 1.1e-6, true));
        // further than the tolerance from the last applied root: the count starts again
        assert!(g.root(1.0 + 1.3e-6, true));
        // a new interval starts the count again
        assert!(g.root(1.0 + 1.4e-6, true));
        assert!(g.root(1.0 + 1.5e-6, true));
        g.interval(0);
        assert!(g.root(1.0 + 1.6e-6, true));
        // every return at a root counts, also one of a trigger, which restarts nothing: the
        // next close root is the third
        assert!(!g.root(1.0 + 1.7e-6, false));
        assert!(!g.root(1.0 + 1.8e-6, true));
    }

    #[test]
    fn the_steps_of_the_integrator_restart_at_every_root_but_one_at_the_same_time() {
        let mut g = guard(false);
        for k in 0..10 {
            assert!(g.root(1.0 + f64::from(k) * 1e-9, true));
        }
        // CVODE returned the zero where it started again
        assert!(!g.root(1.0 + 9e-9, true));
        let mut g = guard(true);
        assert!(g.root(2.0, true));
        assert!(!g.root(2.0, true));
    }
}

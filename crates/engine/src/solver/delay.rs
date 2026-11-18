//! Accepted-step dense history, instantiated only for delay models.

use super::*;
use crate::delay::Mode;
use sbmlsim_abi::history as abi;

pub(super) fn integrate<const DAE: bool, H: Mode>(
    solver: &mut Integrator<DAE, H>,
    problem: &mut Problem<'_>,
    tout: f64,
    itask: c_int,
) -> Result<(f64, c_int), EngineError> {
    if DAE {
        problem.mem = solver.mem;
        let data = problem as *mut Problem<'_> as *mut c_void;
        unsafe {
            solver.flag(IDASetUserData(solver.mem, data), tout)?;
        }
        solver.consistent(problem)?;
    }
    let shared = solver.history.shared().unwrap().clone();
    let mut new_segment = false;
    {
        let mut history = shared.borrow_mut();
        if history.new_segment {
            new_segment = true;
            let t = history.start;
            history.point(t, solver.state(), &problem.p)?;
            history.new_segment = false;
            history.sync(problem.model, &mut problem.p)?;
        }
        if let Some((t, flag)) = history.pending
            && (itask == CV_ONE_STEP || tout >= t)
        {
            history.evaluate(t, unsafe { values(solver.y, problem.n) })?;
            history.pending = None;
            return Ok((t, flag));
        }
        if itask == CV_NORMAL && tout <= history.committed {
            history.evaluate(tout, unsafe { values(solver.y, problem.n) })?;
            return Ok((tout, CV_SUCCESS));
        }
    }
    if new_segment {
        let t = shared.borrow().start;
        let mut roots = vec![0.0; problem.n_roots];
        problem.counts.roots += 1;
        problem.model.roots(
            t,
            &solver.state()[..problem.n_model],
            &problem.p,
            &mut roots,
        );
        problem.model.check_error()?;
        shared
            .borrow_mut()
            .set_jump_targets(problem.model, &mut problem.p)?;
    }
    let from = solver.steps();
    loop {
        let history = shared.borrow();
        if solver.steps().saturating_sub(from) >= history.step_budget {
            return Err(EngineError::Integration {
                flag: CV_TOO_MUCH_WORK,
                time: history.committed,
                message: "delay integration exceeded maximum_num_steps before the next output time"
                    .into(),
            });
        }
        drop(history);
        let (t, flag) = advance(solver, problem, tout)?;
        let mut history = shared.borrow_mut();
        if itask == CV_ONE_STEP
            || flag == CV_ROOT_RETURN && t <= tout
            || flag == CV_TSTOP_RETURN && t <= tout
        {
            return Ok((t, flag));
        }
        if t >= tout {
            if flag == CV_ROOT_RETURN || flag == CV_TSTOP_RETURN {
                history.pending = Some((t, flag));
            }
            history.evaluate(tout, unsafe { values(solver.y, problem.n) })?;
            return Ok((tout, CV_SUCCESS));
        }
    }
}

struct Polynomial {
    t: f64,
    h: f64,
    order: usize,
    coefficients: Vec<f64>,
}

impl Polynomial {
    fn evaluate(&self, t: f64, out: &mut [f64]) {
        let x = if self.h == 0.0 {
            0.0
        } else {
            (t - self.t) / self.h
        };
        let n = self.coefficients.len() / (self.order + 1);
        for (i, value) in out.iter_mut().take(n).enumerate() {
            *value = 0.0;
            for k in (0..=self.order).rev() {
                *value = *value * x + self.coefficients[k * n + i];
            }
        }
    }
}

fn capture<const DAE: bool, H: Mode>(
    solver: &mut Integrator<DAE, H>,
    problem: &Problem<'_>,
    t: f64,
) -> Result<Polynomial, EngineError> {
    let (mut order, mut h) = (0, 0.0);
    unsafe {
        solver.flag(
            if DAE {
                IDAGetLastOrder(solver.mem, &mut order)
            } else {
                CVodeGetLastOrder(solver.mem, &mut order)
            },
            t,
        )?;
        solver.flag(
            if DAE {
                IDAGetLastStep(solver.mem, &mut h)
            } else {
                CVodeGetLastStep(solver.mem, &mut h)
            },
            t,
        )?;
    }
    let saved = solver.state().to_vec();
    let mut coefficients = Vec::with_capacity((order as usize + 1) * problem.n_model);
    let mut scale = 1.0;
    for k in 0..=order {
        unsafe {
            solver.flag(
                if DAE {
                    IDAGetDky(solver.mem, t, k, solver.y)
                } else {
                    CVodeGetDky(solver.mem, t, k, solver.y)
                },
                t,
            )?;
        }
        coefficients.extend(
            solver.state()[..problem.n_model]
                .iter()
                .map(|value| value * scale),
        );
        scale *= h / f64::from(k + 1);
    }
    unsafe {
        values(solver.y, problem.n).copy_from_slice(&saved);
    }
    if DAE {
        coefficients[..problem.n_model].copy_from_slice(&saved[..problem.n_model]);
    }
    Ok(Polynomial {
        t,
        h,
        order: order as usize,
        coefficients,
    })
}

/// Compare the old and new continuous extensions on the part queried by this trial.
/// A converged overlap is accepted; an unconverged waveform is retried, never clamped.
fn overlap_error(
    history: &crate::delay::History,
    polynomial: &Polynomial,
    query: f64,
) -> Result<f64, EngineError> {
    let a = history.committed;
    let b = (query - history.stage_shift).min(polynomial.t);
    if b <= a {
        return Ok(0.0);
    }
    let n = polynomial.coefficients.len() / (polynomial.order + 1);
    let mut previous = vec![0.0; n];
    let mut current = previous.clone();
    let mut error = 0.0_f64;
    for k in 1..=17 {
        let t = a + (b - a) * (k as f64 / 17.0);
        history.evaluate(t, &mut previous)?;
        polynomial.evaluate(t, &mut current);
        for (i, (&old, &new)) in previous.iter().zip(&current).enumerate() {
            let atol = match &history.settings.absolute_tolerance {
                Tolerance::Scalar(value) => *value,
                Tolerance::PerState(values) => values[i],
            };
            let weight = (atol + history.settings.relative_tolerance * old.abs().max(new.abs()))
                .max(f64::MIN_POSITIVE);
            let ratio = (new - old).abs() / weight;
            if !ratio.is_finite() {
                return Err(history.failure("nonfinite delay history in an overlapping step"));
            }
            error = error.max(ratio);
        }
    }
    Ok(error)
}

fn advance<const DAE: bool, H: Mode>(
    solver: &mut Integrator<DAE, H>,
    problem: &mut Problem<'_>,
    tout: f64,
) -> Result<(f64, c_int), EngineError> {
    let shared = solver.history.shared().unwrap().clone();
    let (checkpoint, a, stop) = {
        let mut history = shared.borrow_mut();
        let a = history.committed;
        history.evaluate(a, unsafe { values(solver.y, problem.n) })?;
        let limit = if history.max_step > 0.0 {
            history.max_step
        } else {
            f64::INFINITY
        };
        unsafe {
            solver.flag(
                if DAE {
                    IDASetMaxStep(solver.mem, if limit.is_finite() { limit } else { 0.0 })
                } else {
                    CVodeSetMaxStep(solver.mem, if limit.is_finite() { limit } else { 0.0 })
                },
                a,
            )?;
        }
        history.clear_diagnostics();
        history.sync(problem.model, &mut problem.p)?;
        (history.checkpoint(), a, history.stop)
    };
    let initial = solver.state().to_vec();
    let derivative = if DAE {
        unsafe { values(solver.yp, problem.n).to_vec() }
    } else {
        Vec::new()
    };
    let mut trial_stop = stop;
    let mut previous_error = f64::INFINITY;
    for attempt in 0..48 {
        let outcome = solver.integrate_raw(problem, tout.min(trial_stop), CV_ONE_STEP);
        if outcome.is_err() {
            shared.borrow_mut().diagnostics(problem.model)?;
        }
        let (t, mut flag) = outcome?;
        let polynomial = capture(solver, problem, t)?;
        let mut history = shared.borrow_mut();
        history.diagnostics(problem.model)?;
        let query = history.number(abi::MAX_QUERY);
        let lag = history.number(abi::MIN_LAG);
        let error = overlap_error(&history, &polynomial, query)?;
        if error <= 0.1 {
            history.restore(checkpoint);
            history.append(
                t,
                polynomial.h,
                polynomial.order,
                &polynomial.coefficients,
                &problem.p,
            )?;
            history.committed = t;
            let axis = t + history.stage_shift;
            history.put_number(abi::COMMITTED, axis);
            if lag.is_finite() && lag > 0.0 {
                history.lag_limit = lag;
            }
            history.sync(problem.model, &mut problem.p)?;
            unsafe {
                solver.flag(
                    if DAE {
                        IDASetStopTime(solver.mem, stop)
                    } else {
                        CVodeSetStopTime(solver.mem, stop)
                    },
                    t,
                )?;
            }
            if flag == CV_TSTOP_RETURN && t < stop {
                flag = CV_SUCCESS;
            }
            return Ok((t, flag));
        }
        let halve = attempt % 8 == 7 || error >= previous_error * 0.95;
        previous_error = error;
        history.restore(checkpoint);
        history.append(
            polynomial.t,
            polynomial.h,
            polynomial.order,
            &polynomial.coefficients,
            &problem.p,
        )?;
        trial_stop = if halve { a + (t - a) * 0.5 } else { t };
        if trial_stop <= a || !trial_stop.is_finite() {
            return Err(history.failure("delay history iteration cannot advance time"));
        }
        history.clear_diagnostics();
        history.sync(problem.model, &mut problem.p)?;
        drop(history);
        // Rejected trial work remains in diagnostics even though CVODE resets its counters.
        solver.totals = solver.totals.add(solver.current());
        solver.totals.integrations += 1;
        unsafe {
            values(solver.y, problem.n).copy_from_slice(&initial);
            if DAE {
                values(solver.yp, problem.n).copy_from_slice(&derivative);
            }
            solver.flag(
                if DAE {
                    IDAReInit(solver.mem, a, solver.y, solver.yp)
                } else {
                    CVodeReInit(solver.mem, a, solver.y)
                },
                a,
            )?;
            if DAE {
                solver.pending_ic = Some(a);
            }
            solver.flag(
                if DAE {
                    IDASetStopTime(solver.mem, trial_stop)
                } else {
                    CVodeSetStopTime(solver.mem, trial_stop)
                },
                a,
            )?;
            solver.flag(
                if DAE {
                    IDASetInitStep(solver.mem, trial_stop - a)
                } else {
                    CVodeSetInitStep(solver.mem, trial_stop - a)
                },
                a,
            )?;
        }
        if let Some(preconditioner) = solver.krylov.as_mut() {
            preconditioner.valid = false;
        }
    }
    Err(shared
        .borrow()
        .failure("delay history iteration did not converge after 48 trials"))
}

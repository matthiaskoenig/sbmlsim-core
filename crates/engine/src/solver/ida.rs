//! IDA callbacks for F(t, y, y') = (f - y', g) and consistent state projection.

use super::*;
use sbmlsim_abi::ModelDescription;

pub(super) unsafe extern "C" fn residual(
    t: f64,
    y: N_Vector,
    yp: N_Vector,
    res: N_Vector,
    data: *mut c_void,
) -> c_int {
    // SAFETY: IDA supplies distinct serial vectors of the configured dimension.
    unsafe {
        let problem = problem(data);
        callback(problem, |p| {
            let n = p.n;
            let differential = n - p.n_algebraic;
            let result = values(res, n);
            p.model.rhs(t, values(y, n), &p.p, result);
            p.counts.rhs += 1;
            for (r, yp) in result[..differential].iter_mut().zip(values(yp, n)) {
                *r -= *yp;
            }
            0
        })
    }
}

pub(super) unsafe extern "C" fn roots(
    t: f64,
    y: N_Vector,
    _yp: N_Vector,
    g: *mut f64,
    data: *mut c_void,
) -> c_int {
    // SAFETY: IDA supplies a serial state and n_roots outputs. Its interpolated
    // vector belongs to IDA, so project a scratch copy before evaluating roots.
    unsafe {
        let p = problem(data);
        if p.interpolation.is_none() {
            return super::roots(t, y, g, data);
        }
        callback(p, |p| {
            let projection = p.interpolation.as_mut().unwrap();
            let mut state = std::mem::take(&mut projection.probe);
            state.clear();
            state.extend_from_slice(values(y, p.n));
            if let Err(error) = projection.project(t, &mut state, &p.p) {
                p.callback_error = Some(error);
                projection.probe = state;
                return -1;
            }
            p.counts.roots += 1;
            p.model.roots(
                t,
                &state,
                &p.p,
                std::slice::from_raw_parts_mut(g, p.n_roots),
            );
            projection.probe = state;
            0
        })
    }
}

/// Shared analytic or colored numerical Jacobian of the model's [f, g].
unsafe fn jacobian(
    p: &mut Problem<'_>,
    t: f64,
    y: N_Vector,
    yp: N_Vector,
    weights: N_Vector,
    base: N_Vector,
) -> c_int {
    let n = p.n;
    let pattern = p.pattern.expect("IDA requires a model pattern");
    p.counts.jacobian += 1;
    // SAFETY: IDA supplies distinct serial work vectors of length n to the Jacobian.
    unsafe {
        let y = values(y, n);
        if p.analytic {
            p.model.jac(t, y, &p.p, &mut p.jac_values);
            if p.jac_values.iter().all(|v| v.is_finite()) {
                return 0;
            }
            p.counts.fallbacks += 1;
        }
        let dq = p.difference.as_mut().expect("IDA difference workspace");
        let mut h = 0.0;
        if IDAGetErrWeights(p.mem, weights) != 0 || IDAGetCurrentStep(p.mem, &mut h) != 0 {
            return -1;
        }
        let yp = values(yp, n);
        let weights = values(weights, n);
        for j in 0..n {
            // The state increment of idaLsDenseDQJac. The derivative term is linear
            // and added exactly below, independently of the coloring of d[f,g]/dy.
            let magnitude =
                (f64::EPSILON.sqrt() * y[j].abs().max((h * yp[j]).abs())).max(1.0 / weights[j]);
            let inc = if h * yp[j] < 0.0 {
                -magnitude
            } else {
                magnitude
            };
            dq.increments[j] = (y[j] + inc) - y[j];
        }
        let base = values(base, n);
        p.model.rhs(t, y, &p.p, base);
        p.counts.jacobian_rhs += 1;
        dq.evaluate(
            p.model,
            t,
            y,
            &p.p,
            base,
            pattern,
            &mut p.jac_values,
            &mut p.counts,
        );
    }
    0
}

pub(super) unsafe extern "C" fn dense(
    t: f64,
    cj: f64,
    y: N_Vector,
    yp: N_Vector,
    _res: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    w: N_Vector,
    base: N_Vector,
    _tmp: N_Vector,
) -> c_int {
    // SAFETY: IDA supplies a dense matrix of n*n values and its Jacobian work vectors.
    unsafe {
        callback(problem(data), |p| {
            let code = jacobian(p, t, y, yp, w, base);
            if code != 0 {
                return code;
            }
            let n = p.n;
            let pattern = p.pattern.unwrap();
            let matrix = std::slice::from_raw_parts_mut(SUNDenseMatrix_Data(jac), n * n);
            matrix.fill(0.0);
            for j in 0..n {
                for k in pattern.col_ptr[j] as usize..pattern.col_ptr[j + 1] as usize {
                    matrix[j * n + pattern.row_idx[k] as usize] = p.jac_values[k];
                }
                if j < n - p.n_algebraic {
                    matrix[j * n + j] -= cj;
                }
            }
            0
        })
    }
}

pub(super) unsafe extern "C" fn sparse(
    t: f64,
    cj: f64,
    y: N_Vector,
    yp: N_Vector,
    _res: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    w: N_Vector,
    base: N_Vector,
    _tmp: N_Vector,
) -> c_int {
    // SAFETY: new reserves capacity for all model entries plus the differential diagonal.
    unsafe {
        callback(problem(data), |p| {
            let code = jacobian(p, t, y, yp, w, base);
            if code != 0 {
                return code;
            }
            let n = p.n;
            let pattern = p.pattern.unwrap();
            let capacity = SUNSparseMatrix_NNZ(jac) as usize;
            let data = std::slice::from_raw_parts_mut(SUNSparseMatrix_Data(jac), capacity);
            let rows = std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexValues(jac), capacity);
            let cols = std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexPointers(jac), n + 1);
            let mut out = 0;
            for j in 0..n {
                cols[j] = out as i32;
                let mut add_diagonal = j < n - p.n_algebraic;
                for k in pattern.col_ptr[j] as usize..pattern.col_ptr[j + 1] as usize {
                    let i = pattern.row_idx[k];
                    if add_diagonal && i as usize > j {
                        rows[out] = j as i32;
                        data[out] = -cj;
                        out += 1;
                        add_diagonal = false;
                    }
                    rows[out] = i as i32;
                    data[out] = p.jac_values[k];
                    if add_diagonal && i as usize == j {
                        data[out] -= cj;
                        add_diagonal = false;
                    }
                    out += 1;
                }
                if add_diagonal {
                    rows[out] = j as i32;
                    data[out] = -cj;
                    out += 1;
                }
            }
            cols[n] = out as i32;
            0
        })
    }
}

impl<const DAE: bool, H: crate::delay::Mode> Integrator<DAE, H> {
    /// Obtain consistent y and y' after an IDA (re)initialization.
    pub(super) fn consistent(&mut self, problem: &mut Problem<'_>) -> Result<(), EngineError> {
        let Some(t) = self.pending_ic else {
            return Ok(());
        };
        // The horizon sets the IC Newton scale even for zero-duration requests.
        let horizon = ic_horizon(t);
        // SAFETY: the IDA memory is initialized. The data pointer is valid throughout
        // CalcIC, and replaced before any following call that can invoke callbacks.
        unsafe {
            problem.mem = self.mem;
            // At an algebraic jump the algebraic LTE stays finite as h approaches
            // zero. Control differential LTE and project interpolated algebraic
            // outputs/roots with an independent IDA workspace in that case.
            self.flag(
                IDASetSuppressAlg(self.mem, i32::from(problem.interpolation.is_some())),
                t,
            )?;
            self.flag(
                IDASetUserData(self.mem, (problem as *mut Problem<'_>).cast()),
                t,
            )?;
            let flag = IDACalcIC(self.mem, IDA_YA_YDP_INIT, horizon);
            if let Some(error) = problem.callback_error.take() {
                return Err(error);
            }
            self.flag(flag, t)?;
            self.flag(IDAGetConsistentIC(self.mem, self.y, self.yp), t)?;
        }
        self.pending_ic = None;
        Ok(())
    }
}

fn ic_horizon(t: f64) -> f64 {
    let delta = 1.0_f64.max(t.abs() * 1e-6);
    let future = t + delta;
    if future.is_finite() {
        future
    } else {
        t - delta
    }
}

/// Separate IDA memory for projecting changes and event probes without changing
/// the integration history. Construction also accepts a purely algebraic system.
pub(crate) struct Constraints<'a> {
    model: &'a dyn ModelFns,
    desc: &'a ModelDescription,
    settings: IntegratorSettings,
    solver: Integrator<true>,
    difference: Difference,
    counts: Counts,
    parameters: Vec<f64>,
    probe: Vec<f64>,
    jac_values: Vec<f64>,
}

impl<'a> Constraints<'a> {
    pub(crate) fn new(
        model: &'a dyn ModelFns,
        desc: &'a ModelDescription,
        settings: &IntegratorSettings,
    ) -> Result<Self, EngineError> {
        let linear = crate::linear_of(desc, settings)?;
        let config = Config {
            n: desc.states.len(),
            method: Method::Ida {
                linear,
                n_algebraic: desc.n_algebraic as usize,
            },
        };
        let mut settings = settings.clone();
        // A strict crossing probe can be only 1e-12 time units beyond its root.
        // Project to roundoff even when the integration error budget is looser.
        settings.relative_tolerance = settings.relative_tolerance.min(1e-12);
        match &mut settings.absolute_tolerance {
            Tolerance::Scalar(a) => *a = a.min(1e-14),
            Tolerance::PerState(values) => values.iter_mut().for_each(|a| *a = a.min(1e-14)),
        }
        Ok(Self {
            model,
            desc,
            settings,
            solver: Integrator::new(config)?,
            difference: crate::difference_of(desc),
            parameters: Vec::new(),
            counts: Counts::default(),
            probe: Vec::new(),
            jac_values: vec![0.0; desc.jacobian.pattern().nnz()],
        })
    }

    /// Work spent calculating consistent initial conditions on this memory.
    pub(crate) fn counts(&self) -> (Counts, SolverCounts) {
        (self.counts, self.solver.counts())
    }

    /// Estimate algebraic rates along (1, f) on the constraint manifold.
    /// A fresh projection at the horizon also handles an event applied there.
    pub(crate) fn algebraic_rates(
        &mut self,
        t: f64,
        y: &[f64],
        p: &[f64],
        rates: &mut [f64],
    ) -> Result<(), EngineError> {
        let differential = y.len() - self.desc.n_algebraic as usize;
        let h = f64::EPSILON.cbrt() * t.abs().max(1.0);
        let (before, after) = ((t - h).max(0.0), t + h);
        let mut state = std::mem::take(&mut self.probe);
        state.clear();
        state.extend_from_slice(y);
        for j in 0..differential {
            state[j] += (before - t) * rates[j];
        }
        self.project(before, &mut state, p)?;
        rates[differential..].copy_from_slice(&state[differential..]);
        state.copy_from_slice(y);
        for j in 0..differential {
            state[j] += (after - t) * rates[j];
        }
        self.project(after, &mut state, p)?;
        for j in differential..rates.len() {
            rates[j] = (state[j] - rates[j]) / (after - before);
        }
        self.probe = state;
        Ok(())
    }

    pub(crate) fn project(&mut self, t: f64, y: &mut [f64], p: &[f64]) -> Result<(), EngineError> {
        let n = y.len();
        self.solver.start(t, y, ic_horizon(t), &self.settings, 0)?;
        self.parameters.clear();
        self.parameters.extend_from_slice(p);
        let analytic = matches!(
            self.solver.config.method,
            Method::Ida {
                linear: Linear::Dense {
                    jacobian: Jacobian::Analytic
                } | Linear::Klu {
                    jacobian: Jacobian::Analytic,
                    ..
                },
                ..
            }
        );
        let mut problem = Problem {
            n,
            n_model: n,
            n_algebraic: self.desc.n_algebraic as usize,
            interpolation: None,
            n_roots: 0,
            n_discontinuity_roots: 0,
            model: self.model,
            p: std::mem::take(&mut self.parameters),
            pattern: Some(self.desc.jacobian.pattern()),
            jac_values: std::mem::take(&mut self.jac_values),
            analytic,
            difference: Some(&mut self.difference),
            mem: ptr::null_mut(),
            krylov: ptr::null_mut(),
            counts: Counts::default(),
            callback_error: None,
        };
        let result = self.solver.consistent(&mut problem);
        self.counts = self.counts.add(problem.counts);
        self.parameters = problem.p;
        self.jac_values = problem.jac_values;
        result?;
        y.copy_from_slice(self.solver.state());
        Ok(())
    }
}

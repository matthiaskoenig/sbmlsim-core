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
            // checked before the callback; a panic would trap the instance on wasm32
            let Some(projection) = p.interpolation.as_deref_mut() else {
                return -1;
            };
            let mut state = std::mem::take(&mut projection.workspace.probe);
            state.clear();
            state.extend_from_slice(values(y, p.n));
            if let Err(error) = projection.project(t, &mut state, &p.p) {
                p.callback_error = Some(error);
                projection.workspace.probe = state;
                return -1;
            }
            p.counts.roots += 1;
            p.model.roots(
                t,
                &state,
                &p.p,
                std::slice::from_raw_parts_mut(g, p.n_roots),
            );
            projection.workspace.probe = state;
            0
        })
    }
}

/// Shared analytic or colored numerical Jacobian of the model's [f, g]; the pattern its entries
/// follow, `None` when the call failed. A panic must not leave a callback of IDA: it traps the
/// instance on wasm32, where `callback` cannot catch it.
unsafe fn jacobian<'a>(
    p: &mut Problem<'a>,
    t: f64,
    y: N_Vector,
    yp: N_Vector,
    weights: N_Vector,
    base: N_Vector,
) -> Option<&'a SparsityPattern> {
    let n = p.n;
    let Some(pattern) = p.pattern else {
        p.callback_error = Some(EngineError::Internal(
            "the Jacobian of IDA requires the sparsity pattern of the model".into(),
        ));
        return None;
    };
    p.counts.jacobian += 1;
    // SAFETY: IDA supplies distinct serial work vectors of length n to the Jacobian.
    unsafe {
        let y = values(y, n);
        if p.analytic {
            p.model.jac(t, y, &p.p, &mut p.jac_values);
            if p.jac_values.iter().all(|v| v.is_finite()) {
                return Some(pattern);
            }
            p.counts.fallbacks += 1;
        }
        let mut h = 0.0;
        if IDAGetErrWeights(p.mem, weights) != 0 || IDAGetCurrentStep(p.mem, &mut h) != 0 {
            return None;
        }
        let Some(dq) = p.difference.take() else {
            p.callback_error = Some(EngineError::Internal(
                "the Jacobian of IDA requires the workspace of its difference quotient".into(),
            ));
            return None;
        };
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
        p.difference = Some(dq);
    }
    Some(pattern)
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
            let Some(pattern) = jacobian(p, t, y, yp, w, base) else {
                return -1;
            };
            let n = p.n;
            let matrix = std::slice::from_raw_parts_mut(SUNDenseMatrix_Data(jac), n * n);
            // SUNDIALS clears a direct solver matrix before invoking this callback.
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
            let Some(pattern) = jacobian(p, t, y, yp, w, base) else {
                return -1;
            };
            sbmlsim_SUNSparseMatrix_SetJacobian(
                jac,
                pattern.nnz() as sunindextype,
                pattern.col_ptr.as_ptr().cast(),
                pattern.row_idx.as_ptr().cast(),
                p.jac_values.as_ptr(),
                (p.n - p.n_algebraic) as sunindextype,
                0,
                cj,
            )
        })
    }
}

impl<const DAE: bool, H: crate::delay::Mode> Integrator<DAE, H> {
    /// Obtain consistent y and y' after an IDA (re)initialization.
    pub(super) fn consistent(&mut self, problem: &mut Problem<'_>) -> Result<(), EngineError> {
        let Some(t) = self.pending_ic else {
            return Ok(());
        };
        let _parameters = crate::model::ParameterScope::new(problem.model, &problem.p);
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
                IDASetSuppressAlg(
                    self.mem,
                    i32::from(problem.interpolation.is_some() || self.suppress_algebraic),
                ),
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
    workspace: ConstraintWorkspace,
}

/// Owned projection storage, retained independently of a simulation's model borrows.
pub(crate) struct ConstraintWorkspace {
    requested: IntegratorSettings,
    settings: IntegratorSettings,
    solver: Integrator<true>,
    difference: Difference,
    counts: Counts,
    parameters: Vec<f64>,
    probe: Vec<f64>,
    jac_values: Vec<f64>,
}

#[cfg(test)]
impl ConstraintWorkspace {
    pub(crate) fn storage_identity(&self) -> (usize, usize, usize) {
        (
            self.solver.mem as usize,
            self.jac_values.as_ptr() as usize,
            self.difference.rates.as_ptr() as usize,
        )
    }
}

impl<'a> Constraints<'a> {
    pub(crate) fn cached(
        model: &'a dyn ModelFns,
        desc: &'a ModelDescription,
        settings: &IntegratorSettings,
        workspace: Option<ConstraintWorkspace>,
    ) -> Result<Self, EngineError> {
        let linear = crate::linear_of(desc, settings)?;
        let config = Config {
            n: desc.states.len(),
            method: Method::Ida {
                linear,
                n_algebraic: desc.n_algebraic as usize,
            },
            n_sens: 0,
        };
        let mut workspace = match workspace {
            Some(workspace) if workspace.solver.config() == config => workspace,
            _ => ConstraintWorkspace {
                requested: settings.clone(),
                settings: Self::projection_settings(settings),
                solver: Integrator::new(config)?,
                difference: crate::difference_of(desc),
                parameters: Vec::new(),
                counts: Counts::default(),
                probe: Vec::new(),
                jac_values: vec![0.0; desc.jacobian.pattern().nnz()],
            },
        };
        if workspace.requested != *settings {
            workspace.settings = Self::projection_settings(settings);
            workspace.requested.clone_from(settings);
        }
        workspace.counts = Counts::default();
        workspace.solver.begin();
        Ok(Self {
            model,
            desc,
            workspace,
        })
    }

    fn projection_settings(settings: &IntegratorSettings) -> IntegratorSettings {
        let mut settings = settings.clone();
        // A strict crossing probe can be only 1e-12 time units beyond its root.
        // Project to roundoff even when the integration error budget is looser.
        settings.relative_tolerance = settings.relative_tolerance.min(1e-12);
        match &mut settings.absolute_tolerance {
            Tolerance::Scalar(a) => *a = a.min(1e-14),
            Tolerance::PerState(values) => values.iter_mut().for_each(|a| *a = a.min(1e-14)),
        }
        settings
    }

    pub(crate) fn into_workspace(self) -> ConstraintWorkspace {
        self.workspace
    }

    /// Work spent calculating consistent initial conditions on this memory.
    pub(crate) fn counts(&self) -> (Counts, SolverCounts) {
        (self.workspace.counts, self.workspace.solver.counts())
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
        let mut state = std::mem::take(&mut self.workspace.probe);
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
        self.workspace.probe = state;
        Ok(())
    }

    pub(crate) fn project(&mut self, t: f64, y: &mut [f64], p: &[f64]) -> Result<(), EngineError> {
        let n = y.len();
        self.workspace
            .solver
            .start(t, y, ic_horizon(t), &self.workspace.settings, 0)?;
        self.workspace.parameters.clear();
        self.workspace.parameters.extend_from_slice(p);
        let analytic = matches!(
            self.workspace.solver.config.method,
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
            p: std::mem::take(&mut self.workspace.parameters),
            pattern: Some(self.desc.jacobian.pattern()),
            jac_values: std::mem::take(&mut self.workspace.jac_values),
            jacobian_p: None,
            analytic,
            difference: Some(&mut self.workspace.difference),
            sensitivity: None,
            mem: ptr::null_mut(),
            krylov: ptr::null_mut(),
            counts: Counts::default(),
            callback_error: None,
        };
        let result = self.workspace.solver.consistent(&mut problem);
        self.workspace.counts = self.workspace.counts.add(problem.counts);
        self.workspace.parameters = problem.p;
        self.workspace.jac_values = problem.jac_values;
        result?;
        y.copy_from_slice(self.workspace.solver.state());
        Ok(())
    }
}

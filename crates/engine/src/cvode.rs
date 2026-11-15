//! The wrapper of CVODE: a context, the memory of CVODE, the state vector, the matrix and the
//! linear solver (or the fixed point iteration) of one configuration, created once and
//! re-initialized for every segment.
//!
//! The methods are the ones of libroadrunner: `stiff` is BDF with the Newton iteration and a
//! linear solver, non-stiff is Adams-Moulton with the fixed point iteration.
//!
//! The Jacobian of the Newton iteration is the analytic one of the model (`model_jac`) or the
//! sparse difference quotient over the colors of the columns of the pattern of the model
//! ([`Difference`]), each with the dense solver (the dense matrix filled from the entries of the
//! pattern) or with KLU (the CSC matrix of the pattern). The sparse difference quotient takes
//! the increments of CVODE's dense difference quotient (`cvLsDenseDQJac`,
//! `max(sqrt(eps) |y_j|, minInc / ewt_j)`) and computes every entry as it does, so with the dense
//! solver it gives the Jacobian of CVODE's own difference quotient bitwise, with one evaluation
//! of the right hand side per color instead of per state. CVODE's own difference quotient is
//! only used for the one state of the rate 0 of a model without states, which has no pattern.
//!
//! An analytic Jacobian with an entry which is not finite (an infinite derivative at a finite
//! state, e.g. `0.5 * pow(x, -0.5)` of `sqrt(x)` at `x = 0`, or a NaN of `0 * pow(0, -1)` of
//! `x^0`, where CVODE's difference quotient sees a finite slope) would make the factorization of
//! `I - gamma J` NaN and the corrector fail. Such a call computes its whole Jacobian with the
//! sparse difference quotient over the greedy coloring of the pattern instead; the analytic
//! entries are evaluated first in every call, so the choice depends on the state alone and the
//! bits are the same on every host and in every run.
//!
//! SUNDIALS writes nothing: the library is built without logging (`SUNDIALS_LOGGING_LEVEL 0`),
//! the default error handlers of the context, which print, are removed and an error handler of
//! the wrapper records the message of the last error instead, which
//! [`EngineError::Integration`] carries.

use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::ptr;

use sbmlsim_abi::{IntegratorSettings, SparsityPattern, Tolerance};
use sbmlsim_csolvers::ffi::*;

use crate::EngineError;
use crate::model::ModelFns;

/// The number of states above which [`LinearSolver::Auto`](sbmlsim_abi::LinearSolver::Auto)
/// takes KLU, else the dense solver.
pub const KLU_THRESHOLD: usize = 64;

/// The source of the Jacobian of a stiff configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Jacobian {
    /// `model_jac` of the model.
    Analytic,
    /// The sparse difference quotient over the colors of the columns of the pattern.
    Difference,
    /// CVODE's own dense difference quotient, only for the one state of the rate 0 of a model
    /// without states (which has no pattern).
    Internal,
}

/// The linear solver of a configuration and the source of its Jacobian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Linear {
    /// The dense solver.
    Dense {
        /// The source of the Jacobian.
        jacobian: Jacobian,
    },
    /// KLU on the pattern of `nnz` entries (never with [`Jacobian::Internal`]).
    Klu {
        /// The number of entries of the pattern.
        nnz: usize,
        /// The source of the Jacobian.
        jacobian: Jacobian,
    },
}

/// What the sparse difference quotient needs besides the pattern: the columns of every color
/// and its vectors.
pub(crate) struct Difference {
    /// The columns of the color `c` are `columns[starts[c]..starts[c + 1]]`, ascending.
    starts: Vec<usize>,
    columns: Vec<usize>,
    /// The increment of every state.
    increments: Vec<f64>,
    /// The state with the increments of the columns of one color.
    perturbed: Vec<f64>,
    /// The right hand side at `perturbed`.
    rates: Vec<f64>,
}

impl Difference {
    /// The difference quotient over a coloring of the `n` columns of a pattern (checked to share
    /// no row within a color, every color below `max(n, 1)`).
    pub fn new(n: usize, colors: &[u32]) -> Difference {
        let n_colors = colors.iter().map(|c| *c as usize + 1).max().unwrap_or(0);
        let mut starts = vec![0usize; n_colors + 1];
        for &c in colors {
            starts[c as usize + 1] += 1;
        }
        for c in 0..n_colors {
            starts[c + 1] += starts[c];
        }
        let mut next = starts.clone();
        let mut columns = vec![0usize; n];
        for (j, &c) in colors.iter().enumerate() {
            columns[next[c as usize]] = j;
            next[c as usize] += 1;
        }
        Difference {
            starts,
            columns,
            increments: vec![0.0; n],
            perturbed: vec![0.0; n],
            rates: vec![0.0; n],
        }
    }

    /// The number of colors, the evaluations of the right hand side of one Jacobian.
    pub fn n_colors(&self) -> usize {
        self.starts.len() - 1
    }
}

/// The method of a configuration, as libroadrunner chooses it from `stiff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Stiff: BDF with the Newton iteration and a linear solver.
    Bdf(Linear),
    /// Non-stiff: Adams-Moulton with the fixed point iteration, no linear solver.
    Adams,
}

/// What a CVODE memory is created for; another configuration needs another memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Config {
    /// The number of states, at least 1.
    pub n: usize,
    /// The method.
    pub method: Method,
}

/// What the callbacks of CVODE read: the model, the parameters and the pattern of the Jacobian.
pub(crate) struct Problem<'a> {
    /// The number of states, the length of the vectors of CVODE.
    pub n: usize,
    /// The number of states of the model: `n`, or 0 for a model without states which CVODE
    /// integrates with one state of the rate 0 (to locate the zeros of its root functions).
    pub n_model: usize,
    /// The number of root functions of the model.
    pub n_roots: usize,
    /// The model.
    pub model: &'a dyn ModelFns,
    /// The parameters, constant during an integration (events change them between two).
    pub p: Vec<f64>,
    /// The pattern of the Jacobian, `None` for CVODE's own difference quotient and a
    /// non-stiff method.
    pub pattern: Option<&'a SparsityPattern>,
    /// The values of the Jacobian in the CSC order of the pattern.
    pub jac_values: Vec<f64>,
    /// Whether the Jacobian is the analytic one of the model (with the sparse difference
    /// quotient for a call with an entry which is not finite), else the sparse difference
    /// quotient.
    pub analytic: bool,
    /// The sparse difference quotient: the Jacobian, or the fallback of the analytic one; `None`
    /// without a pattern.
    pub difference: Option<Difference>,
    /// The CVODE memory of the running integration, set by [`Cvode::integrate`] (the sparse
    /// difference quotient reads its error weights and step size).
    pub mem: *mut c_void,
}

/// The message of the last error SUNDIALS reported, written by [`record_error`].
#[derive(Debug, Default)]
struct ErrorLog {
    /// The function and the message of the last error.
    message: String,
}

/// The error handler of a context: keeps the message instead of printing it.
unsafe extern "C" fn record_error(
    _line: c_int,
    func: *const c_char,
    _file: *const c_char,
    msg: *const c_char,
    _err_code: SUNErrCode,
    err_user_data: *mut c_void,
    _sunctx: SUNContext,
) {
    // SAFETY: `err_user_data` is the `ErrorLog` of the `Cvode` which owns the context, the
    // strings are null or C strings of SUNDIALS.
    unsafe {
        let log = &mut *(err_user_data as *mut ErrorLog);
        let text = |s: *const c_char| {
            if s.is_null() {
                String::new()
            } else {
                CStr::from_ptr(s).to_string_lossy().into_owned()
            }
        };
        log.message = format!("{}: {}", text(func), text(msg));
    }
}

/// The values of a serial vector of length `n`.
///
/// # Safety
/// `v` is a serial vector of at least `n` values which outlives the slice and is not aliased.
unsafe fn values<'a>(v: N_Vector, n: usize) -> &'a mut [f64] {
    unsafe { std::slice::from_raw_parts_mut(N_VGetArrayPointer(v), n) }
}

/// The problem behind the user data of CVODE.
///
/// # Safety
/// `data` is the pointer to the `Problem` [`Cvode::integrate`] set, which lives during the call.
unsafe fn problem<'a>(data: *mut c_void) -> &'a mut Problem<'a> {
    unsafe { &mut *(data as *mut Problem<'a>) }
}

unsafe extern "C" fn rhs(t: sunrealtype, y: N_Vector, ydot: N_Vector, data: *mut c_void) -> c_int {
    // SAFETY: CVODE passes the vectors of the problem and the user data set before the call.
    unsafe {
        let problem = problem(data);
        let (n, m) = (problem.n, problem.n_model);
        let ydot = values(ydot, n);
        problem
            .model
            .rhs(t, &values(y, n)[..m], &problem.p, &mut ydot[..m]);
        ydot[m..].fill(0.0);
    }
    0
}

unsafe extern "C" fn roots(
    t: sunrealtype,
    y: N_Vector,
    g: *mut sunrealtype,
    data: *mut c_void,
) -> c_int {
    // SAFETY: CVODE passes the state, the `n_roots` values of `g` it was initialized with and
    // the user data set before the call.
    unsafe {
        let problem = problem(data);
        let (n, m) = (problem.n, problem.n_model);
        let g = std::slice::from_raw_parts_mut(g, problem.n_roots);
        problem.model.roots(t, &values(y, n)[..m], &problem.p, g);
    }
    0
}

/// The values of the Jacobian at `(t, y)` into `problem.jac_values`: the analytic one of the
/// model where all its entries are finite, else the sparse difference quotient; `fy` is the
/// right hand side at `y` and `ewt` a vector of `n` values CVODE lends. Returns the return code
/// of the callback.
///
/// # Safety
/// `y`, `fy` and `ewt` are serial vectors of `problem.n` values, `problem.mem` is the CVODE
/// memory of the running integration.
unsafe fn jacobian_values(
    problem: &mut Problem<'_>,
    pattern: &SparsityPattern,
    t: sunrealtype,
    y: N_Vector,
    fy: N_Vector,
    ewt: N_Vector,
) -> c_int {
    let n = pattern.n as usize;
    // SAFETY: as documented; the vectors are not aliased by the slices of the problem.
    unsafe {
        let y_values = values(y, n);
        if problem.analytic {
            problem
                .model
                .jac(t, y_values, &problem.p, &mut problem.jac_values);
            if problem.jac_values.iter().all(|v| v.is_finite()) {
                return 0;
            }
        }
        let Some(dq) = problem.difference.as_mut() else {
            return -1;
        };
        // the increments of `cvLsDenseDQJac` (SUNDIALS 7.9.0), in its order of operations
        if CVodeGetErrWeights(problem.mem, ewt) != 0 {
            return -1;
        }
        let mut h = 0.0;
        if CVodeGetCurrentStep(problem.mem, &mut h) != 0 {
            return -1;
        }
        let uround = f64::EPSILON;
        let srur = uround.sqrt();
        let fnorm = N_VWrmsNorm(fy, ewt);
        let min_inc = if fnorm != 0.0 {
            1000.0 * h.abs() * uround * n as f64 * fnorm
        } else {
            1.0
        };
        let ewt = values(ewt, n);
        for j in 0..n {
            let (a, b) = (srur * y_values[j].abs(), min_inc / ewt[j]);
            dq.increments[j] = if a > b { a } else { b };
        }
        let fy = values(fy, n);
        dq.perturbed.copy_from_slice(y_values);
        for c in 0..dq.n_colors() {
            let columns = &dq.columns[dq.starts[c]..dq.starts[c + 1]];
            for &j in columns {
                dq.perturbed[j] += dq.increments[j];
            }
            problem
                .model
                .rhs(t, &dq.perturbed, &problem.p, &mut dq.rates);
            for &j in columns {
                dq.perturbed[j] = y_values[j];
                // `N_VLinearSum(inc_inv, ftemp, -inc_inv, fy)` is `inc_inv * (ftemp - fy)`
                let inc_inv = 1.0 / dq.increments[j];
                let (start, end) = (pattern.col_ptr[j] as usize, pattern.col_ptr[j + 1] as usize);
                for k in start..end {
                    let i = pattern.row_idx[k] as usize;
                    problem.jac_values[k] = inc_inv * (dq.rates[i] - fy[i]);
                }
            }
        }
    }
    0
}

unsafe extern "C" fn jac_dense(
    t: sunrealtype,
    y: N_Vector,
    fy: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    tmp1: N_Vector,
    _tmp2: N_Vector,
    _tmp3: N_Vector,
) -> c_int {
    // SAFETY: as in `rhs`; the dense matrix has `n * n` values, column-major; `tmp1` is a
    // vector of `n` values CVODE lends to the callback.
    unsafe {
        let problem = problem(data);
        let Some(pattern) = problem.pattern else {
            return -1;
        };
        let code = jacobian_values(problem, pattern, t, y, fy, tmp1);
        if code != 0 {
            return code;
        }
        let n = pattern.n as usize;
        let matrix = std::slice::from_raw_parts_mut(SUNDenseMatrix_Data(jac), n * n);
        matrix.fill(0.0);
        for j in 0..n {
            let (start, end) = (pattern.col_ptr[j] as usize, pattern.col_ptr[j + 1] as usize);
            for k in start..end {
                matrix[j * n + pattern.row_idx[k] as usize] = problem.jac_values[k];
            }
        }
    }
    0
}

unsafe extern "C" fn jac_sparse(
    t: sunrealtype,
    y: N_Vector,
    fy: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    tmp1: N_Vector,
    _tmp2: N_Vector,
    _tmp3: N_Vector,
) -> c_int {
    // SAFETY: as in `rhs`. The matrix is the CSC matrix of the linear solver; its capacity is
    // read every call, since CVODE may have grown it (`SUNMatScaleAddI` adds the diagonal of a
    // pattern which lacks it), and the index arrays are written from the pattern every call.
    unsafe {
        let problem = problem(data);
        let Some(pattern) = problem.pattern else {
            return -1;
        };
        let n = pattern.n as usize;
        let nnz = pattern.row_idx.len();
        let capacity = SUNSparseMatrix_NNZ(jac);
        if capacity < 0 || (capacity as usize) < nnz {
            return -1;
        }
        let code = jacobian_values(problem, pattern, t, y, fy, tmp1);
        if code != 0 {
            return code;
        }
        let capacity = capacity as usize;
        let data = std::slice::from_raw_parts_mut(SUNSparseMatrix_Data(jac), capacity);
        let rows = std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexValues(jac), capacity);
        let cols = std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexPointers(jac), n + 1);
        for (c, p) in cols.iter_mut().zip(&pattern.col_ptr) {
            *c = *p as sunindextype;
        }
        for (r, i) in rows.iter_mut().zip(&pattern.row_idx) {
            *r = *i as sunindextype;
        }
        data[..nnz].copy_from_slice(&problem.jac_values);
    }
    0
}

/// Checks a return code of a SUNDIALS function which creates or sets up the context.
fn check(code: c_int, call: &str) -> Result<(), EngineError> {
    if code == 0 {
        Ok(())
    } else {
        Err(EngineError::Integration {
            flag: code,
            time: f64::NAN,
            message: format!("{call} returned {code}"),
        })
    }
}

/// A CVODE memory of one [`Config`] with its context, vectors, matrix and linear solver.
pub(crate) struct Cvode {
    config: Config,
    ctx: SUNContext,
    mem: *mut c_void,
    y: N_Vector,
    abstol: N_Vector,
    matrix: SUNMatrix,
    solver: SUNLinearSolver,
    nonlinear: SUNNonlinearSolver,
    /// Whether `CVodeInit` succeeded (set right after it); later segments call `CVodeReInit`,
    /// never `CVodeInit` again, which would leak the memory of the first call.
    initialized: bool,
    /// Whether the linear solver (or the nonlinear solver) and the Jacobian are attached; a
    /// failed attach is retried by the next segment.
    attached: bool,
    /// The last error, written by [`record_error`] through the pointer of the context.
    errors: Box<ErrorLog>,
}

impl Drop for Cvode {
    fn drop(&mut self) {
        // SAFETY: every pointer is null or owned by this memory; CVODE is freed before the
        // linear solver and the matrix it uses, the context last.
        unsafe {
            if !self.mem.is_null() {
                CVodeFree(&mut self.mem);
            }
            if !self.solver.is_null() {
                SUNLinSolFree(self.solver);
            }
            if !self.nonlinear.is_null() {
                SUNNonlinSolFree(self.nonlinear);
            }
            if !self.matrix.is_null() {
                SUNMatDestroy(self.matrix);
            }
            for v in [self.abstol, self.y] {
                if !v.is_null() {
                    N_VDestroy(v);
                }
            }
            if !self.ctx.is_null() {
                SUNContext_Free(&mut self.ctx);
            }
        }
    }
}

/// The error of an object SUNDIALS did not create (out of memory).
fn not_created(what: &str) -> EngineError {
    EngineError::Integration {
        flag: -1,
        time: f64::NAN,
        message: format!("SUNDIALS could not create {what}"),
    }
}

impl Cvode {
    /// Creates the memory of a configuration.
    pub fn new(config: Config) -> Result<Cvode, EngineError> {
        let mut cvode = Cvode {
            config,
            ctx: ptr::null_mut(),
            mem: ptr::null_mut(),
            y: ptr::null_mut(),
            abstol: ptr::null_mut(),
            matrix: ptr::null_mut(),
            solver: ptr::null_mut(),
            nonlinear: ptr::null_mut(),
            initialized: false,
            attached: false,
            errors: Box::default(),
        };
        let n = config.n as sunindextype;
        // SAFETY: the objects are created in the context and owned by `cvode`, which frees them
        // when it is dropped, also after a failure here.
        unsafe {
            check(
                SUNContext_Create(SUN_COMM_NULL, &mut cvode.ctx),
                "SUNContext_Create",
            )?;
            check(
                SUNContext_ClearErrHandlers(cvode.ctx),
                "SUNContext_ClearErrHandlers",
            )?;
            let log: *mut ErrorLog = &mut *cvode.errors;
            check(
                SUNContext_PushErrHandler(cvode.ctx, Some(record_error), log as *mut c_void),
                "SUNContext_PushErrHandler",
            )?;
            cvode.y = N_VNew_Serial(n, cvode.ctx);
            cvode.abstol = N_VNew_Serial(n, cvode.ctx);
            if cvode.y.is_null() || cvode.abstol.is_null() {
                return Err(not_created("the vectors"));
            }
            values(cvode.y, config.n).fill(0.0);
            let lmm = match config.method {
                Method::Bdf(_) => CV_BDF,
                Method::Adams => CV_ADAMS,
            };
            cvode.mem = CVodeCreate(lmm, cvode.ctx);
            if cvode.mem.is_null() {
                return Err(not_created("the CVODE memory"));
            }
            let linear = match config.method {
                Method::Bdf(linear) => linear,
                Method::Adams => {
                    cvode.nonlinear = SUNNonlinSol_FixedPoint(cvode.y, 0, cvode.ctx);
                    if cvode.nonlinear.is_null() {
                        return Err(not_created("the fixed point iteration"));
                    }
                    return Ok(cvode);
                }
            };
            match linear {
                Linear::Dense { .. } => {
                    cvode.matrix = SUNDenseMatrix(n, n, cvode.ctx);
                    if !cvode.matrix.is_null() {
                        cvode.solver = SUNLinSol_Dense(cvode.y, cvode.matrix, cvode.ctx);
                    }
                }
                Linear::Klu { nnz, .. } => {
                    // a capacity of at least 1, CVODE grows it when it adds the diagonal
                    let capacity = nnz.max(1) as sunindextype;
                    cvode.matrix = SUNSparseMatrix(n, n, capacity, CSC_MAT, cvode.ctx);
                    if !cvode.matrix.is_null() {
                        cvode.solver = SUNLinSol_KLU(cvode.y, cvode.matrix, cvode.ctx);
                    }
                }
            }
            if cvode.matrix.is_null() || cvode.solver.is_null() {
                return Err(not_created("the linear solver"));
            }
        }
        Ok(cvode)
    }

    /// The configuration of the memory.
    pub fn config(&self) -> Config {
        self.config
    }

    /// The state vector of CVODE, the state at the time of the last return.
    pub fn state(&self) -> &[f64] {
        // SAFETY: `y` is a serial vector of `n` values owned by this memory, which CVODE writes
        // only during a call of `&mut self`.
        unsafe { std::slice::from_raw_parts(N_VGetArrayPointer(self.y), self.config.n) }
    }

    /// Starts a segment at `t0` with the state `y0` and the settings of the integrator; with
    /// `n_roots` root functions of the model CVODE locates their zeros.
    pub fn start(
        &mut self,
        t0: f64,
        y0: &[f64],
        tstop: f64,
        settings: &IntegratorSettings,
        n_roots: usize,
    ) -> Result<(), EngineError> {
        let n = self.config.n;
        self.errors.message.clear();
        // SAFETY: the vectors have `n` values; the memory is initialized before it is
        // re-initialized and before the linear solver is attached.
        unsafe {
            values(self.y, n).copy_from_slice(y0);
            if self.initialized {
                self.flag(CVodeReInit(self.mem, t0, self.y), t0)?;
            } else {
                self.flag(CVodeInit(self.mem, Some(rhs), t0, self.y), t0)?;
                self.initialized = true;
            }
            if !self.attached {
                match self.config.method {
                    Method::Adams => {
                        self.flag(CVodeSetNonlinearSolver(self.mem, self.nonlinear), t0)?;
                    }
                    Method::Bdf(linear) => {
                        self.flag(CVodeSetLinearSolver(self.mem, self.solver, self.matrix), t0)?;
                        let jac: CVLsJacFn = match linear {
                            Linear::Dense {
                                jacobian: Jacobian::Internal,
                            } => None,
                            Linear::Dense { .. } => Some(jac_dense),
                            Linear::Klu { .. } => Some(jac_sparse),
                        };
                        self.flag(CVodeSetJacFn(self.mem, jac), t0)?;
                    }
                }
                self.attached = true;
            }
            let rtol = settings.relative_tolerance;
            match &settings.absolute_tolerance {
                Tolerance::Scalar(atol) => {
                    self.flag(CVodeSStolerances(self.mem, rtol, *atol), t0)?;
                }
                Tolerance::PerState(atol) => {
                    values(self.abstol, n).copy_from_slice(atol);
                    self.flag(CVodeSVtolerances(self.mem, rtol, self.abstol), t0)?;
                }
            }
            // `long` is 32 bits on wasm32: the largest count it holds (`as u32` of `i64::MAX` is
            // `u32::MAX`, so the bound changes nothing on 64-bit targets)
            let max_steps = settings.maximum_num_steps.min(c_long::MAX as u32) as c_long;
            self.flag(CVodeSetMaxNumSteps(self.mem, max_steps), t0)?;
            // 0.0 is CVODE's own default for both: no largest step, an estimated first step
            self.flag(CVodeSetMaxStep(self.mem, settings.maximum_time_step), t0)?;
            self.flag(CVodeSetInitStep(self.mem, settings.initial_time_step), t0)?;
            self.flag(CVodeSetStopTime(self.mem, tstop), t0)?;
            let (nrtfn, g): (c_int, CVRootFn) = if n_roots == 0 {
                (0, None)
            } else {
                let nrtfn = c_int::try_from(n_roots).map_err(|_| {
                    EngineError::Request(format!(
                        "{n_roots} root functions are more than CVODE takes"
                    ))
                })?;
                (nrtfn, Some(roots))
            };
            self.flag(CVodeRootInit(self.mem, nrtfn, g), t0)?;
        }
        Ok(())
    }

    /// Sets the stop time of the running segment, which must not be behind the internal time of
    /// CVODE.
    pub fn set_stop(&mut self, tstop: f64) -> Result<(), EngineError> {
        // SAFETY: the memory is initialized (a segment was started).
        unsafe { self.flag(CVodeSetStopTime(self.mem, tstop), tstop) }
    }

    /// Integrates towards `tout` (`CV_NORMAL`) or takes one step (`CV_ONE_STEP`); returns the
    /// time reached and the return flag of CVODE (`CV_SUCCESS` or `CV_TSTOP_RETURN`).
    pub fn integrate(
        &mut self,
        problem: &mut Problem<'_>,
        tout: f64,
        itask: c_int,
    ) -> Result<(f64, c_int), EngineError> {
        let mut tret = f64::NAN;
        // SAFETY: the user data points to `problem`, which outlives the call of `CVode`.
        unsafe {
            problem.mem = self.mem;
            let data = problem as *mut Problem<'_> as *mut c_void;
            self.flag(CVodeSetUserData(self.mem, data), tout)?;
            let flag = CVode(self.mem, tout, self.y, &mut tret, itask);
            self.flag(flag, tret)?;
            Ok((tret, flag))
        }
    }

    /// The error of a negative flag of CVODE at `time`, with the last recorded message.
    fn flag(&mut self, flag: c_int, time: f64) -> Result<(), EngineError> {
        if flag >= 0 {
            Ok(())
        } else {
            Err(EngineError::Integration {
                flag,
                time,
                message: std::mem::take(&mut self.errors.message),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use sbmlsim_abi::{IntegratorSettings, Tolerance};

    use super::*;
    use crate::model::{ModelFns, Robertson};

    /// Integrates Robertson with a configuration to `4e10`, returns the state at the outputs
    /// `0.4 * 10^i` and the number of steps.
    fn robertson(linear: Linear, difference: Option<Difference>) -> (Vec<f64>, c_long) {
        let analytic = matches!(
            linear,
            Linear::Dense {
                jacobian: Jacobian::Analytic
            } | Linear::Klu {
                jacobian: Jacobian::Analytic,
                ..
            }
        );
        let model = Robertson;
        let desc = model.description();
        let (mut p, mut y) = (vec![0.0; 3], vec![0.0; 3]);
        model.defaults(&mut p, &mut y);
        let mut cvode = Cvode::new(Config {
            n: 3,
            method: Method::Bdf(linear),
        })
        .unwrap();
        let settings = IntegratorSettings {
            relative_tolerance: 1e-8,
            absolute_tolerance: Tolerance::PerState(vec![1e-12, 1e-14, 1e-10]),
            ..IntegratorSettings::default()
        };
        cvode.start(0.0, &y, 4e10, &settings, 0).unwrap();
        let mut problem = Problem {
            n: 3,
            n_model: 3,
            n_roots: 0,
            model: &model,
            p,
            pattern: Some(desc.jacobian.pattern()),
            jac_values: vec![0.0; desc.jacobian.pattern().nnz()],
            analytic,
            difference,
            mem: ptr::null_mut(),
        };
        let mut out = Vec::new();
        for i in 0..11 {
            cvode
                .integrate(&mut problem, 0.4 * 10f64.powi(i), CV_NORMAL)
                .unwrap();
            out.extend_from_slice(cvode.state());
        }
        let mut steps: c_long = 0;
        // SAFETY: the memory is initialized.
        unsafe { CVodeGetNumSteps(cvode.mem, &mut steps) };
        (out, steps)
    }

    /// A stiff chain of `N` states, `y_i' = 1000 (y_{i-1}^2 - y_i^3)` (`y_{-1}` = 1): the
    /// Jacobian is bidiagonal, so the greedy coloring groups every second column.
    struct Chain;

    const N: usize = 7;

    impl ModelFns for Chain {
        fn defaults(&self, _p: &mut [f64], y: &mut [f64]) {
            y.fill(0.0);
        }
        fn init(&self, _t: f64, _p: &mut [f64], _y: &mut [f64], _fixed: &[u8]) {}
        fn rhs(&self, _t: f64, y: &[f64], _p: &[f64], ydot: &mut [f64]) {
            for i in 0..N {
                let before = if i == 0 { 1.0 } else { y[i - 1] * y[i - 1] };
                ydot[i] = 1000.0 * (before - y[i] * y[i] * y[i]);
            }
        }
        fn jac(&self, _t: f64, _y: &[f64], _p: &[f64], _values: &mut [f64]) {
            unreachable!("the chain has no analytic Jacobian")
        }
        fn roots(&self, _t: f64, _y: &[f64], _p: &[f64], _g: &mut [f64]) {}
        fn trigger(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> bool {
            false
        }
        fn priority(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
            0.0
        }
        fn delay(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64]) -> f64 {
            0.0
        }
        fn event_values(&self, _i: u32, _t: f64, _y: &[f64], _p: &[f64], _out: &mut [f64]) {}
        fn event_apply(&self, _i: u32, _t: f64, _v: &[f64], _y: &mut [f64], _p: &mut [f64]) {}
        fn selections(&self, _t: f64, _y: &[f64], _p: &[f64], _out: &mut [f64]) {}
        fn assign(&self, _t: f64, _y: &mut [f64], _p: &mut [f64]) {}
        fn sizes(&self) -> sbmlsim_abi::ModelSizes {
            unreachable!("not loaded into an engine")
        }
    }

    /// The bidiagonal pattern of the chain: column `j` has the rows `j` and `j + 1`.
    fn chain_pattern() -> SparsityPattern {
        let mut col_ptr = vec![0u32];
        let mut row_idx = Vec::new();
        for j in 0..N as u32 {
            row_idx.push(j);
            if (j as usize) + 1 < N {
                row_idx.push(j + 1);
            }
            col_ptr.push(row_idx.len() as u32);
        }
        SparsityPattern {
            n: N as u32,
            col_ptr,
            row_idx,
        }
    }

    /// Integrates the chain to `t = 10` with a configuration; the state at `0.01 * 10^i` and the
    /// number of steps.
    fn chain(linear: Linear, difference: Option<Difference>) -> (Vec<f64>, c_long) {
        let pattern = chain_pattern();
        let mut cvode = Cvode::new(Config {
            n: N,
            method: Method::Bdf(linear),
        })
        .unwrap();
        let settings = IntegratorSettings {
            relative_tolerance: 1e-8,
            absolute_tolerance: Tolerance::Scalar(1e-12),
            ..IntegratorSettings::default()
        };
        cvode.start(0.0, &[0.0; N], 10.0, &settings, 0).unwrap();
        let mut problem = Problem {
            n: N,
            n_model: N,
            n_roots: 0,
            model: &Chain,
            p: vec![],
            pattern: Some(&pattern),
            jac_values: vec![0.0; pattern.nnz()],
            analytic: false,
            difference,
            mem: ptr::null_mut(),
        };
        let mut out = Vec::new();
        for i in 0..4 {
            cvode
                .integrate(&mut problem, 0.01 * 10f64.powi(i), CV_NORMAL)
                .unwrap();
            out.extend_from_slice(cvode.state());
        }
        let mut steps: c_long = 0;
        // SAFETY: the memory is initialized.
        unsafe { CVodeGetNumSteps(cvode.mem, &mut steps) };
        (out, steps)
    }

    #[test]
    fn colors_of_several_columns_give_the_difference_quotient_of_cvode_bitwise() {
        // a color holds every second column of the bidiagonal pattern: one right hand side per
        // color gives the entries of all its columns, each the forward difference of CVODE
        let pattern = chain_pattern();
        let colors = pattern.greedy_colors();
        assert_eq!(colors, [0, 1, 0, 1, 0, 1, 0]);
        let internal = chain(
            Linear::Dense {
                jacobian: Jacobian::Internal,
            },
            None,
        );
        let sparse = chain(
            Linear::Dense {
                jacobian: Jacobian::Difference,
            },
            Some(Difference::new(N, &colors)),
        );
        assert!(internal.1 > 50, "{} steps", internal.1);
        assert_eq!(internal.1, sparse.1);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&internal.0), bits(&sparse.0));
        assert!(internal.0.iter().all(|y| y.is_finite() && *y > 0.0));
    }

    #[test]
    fn the_sparse_difference_quotient_is_the_one_of_cvode_bitwise() {
        // CVODE's own dense difference quotient against the sparse one over the coloring of the
        // pattern of Robertson (every two columns share a row: the densest column y2 first): the
        // same increments and operations give the same Jacobians, so the same steps and bits
        let desc = Robertson.description();
        let colors = desc.jacobian.pattern().greedy_colors();
        assert_eq!(colors, [1, 0, 2]);
        let internal = robertson(
            Linear::Dense {
                jacobian: Jacobian::Internal,
            },
            None,
        );
        let sparse = robertson(
            Linear::Dense {
                jacobian: Jacobian::Difference,
            },
            Some(Difference::new(3, &colors)),
        );
        assert!(internal.1 > 1000, "{} steps", internal.1);
        assert_eq!(internal.1, sparse.1);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&internal.0), bits(&sparse.0));
        // and with KLU it integrates to the same result within the tolerances of the integration
        // (the criterion of `dense_and_klu_same_result`: the step sequences part after t = 4)
        let klu = robertson(
            Linear::Klu {
                nnz: desc.jacobian.pattern().nnz(),
                jacobian: Jacobian::Difference,
            },
            Some(Difference::new(3, &colors)),
        );
        let last = |v: &[f64]| v[v.len() - 3..].to_vec();
        let (yd, yk) = (last(&internal.0), last(&klu.0));
        let norm = (yd
            .iter()
            .zip(&yk)
            .zip([1e-12, 1e-14, 1e-10])
            .map(|((d, k), a)| ((d - k) / (1e-8 * d.abs() + 10.0 * a)).powi(2))
            .sum::<f64>()
            / 3.0)
            .sqrt();
        assert!(norm < 1.0, "{yd:?} {yk:?} ({norm:e})");
    }
}

//! The wrapper of CVODE: a context, the memory of CVODE, the state vector, the matrix and the
//! linear solver (or the fixed point iteration) of one configuration, created once and
//! re-initialized for every segment.
//!
//! The methods are the ones of libroadrunner: `stiff` is BDF with the Newton iteration and a
//! linear solver, non-stiff is Adams-Moulton with the fixed point iteration.
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
/// takes KLU for a model with an analytic Jacobian, else the dense solver.
pub const KLU_THRESHOLD: usize = 64;

/// The linear solver of a configuration and the source of its Jacobian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Linear {
    /// The dense solver, with the analytic Jacobian of the model or CVODE's difference quotient.
    Dense {
        /// Whether the model computes the Jacobian.
        analytic: bool,
    },
    /// KLU with the analytic Jacobian of the model, of `nnz` entries.
    Klu {
        /// The number of entries of the pattern.
        nnz: usize,
    },
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
    /// The model.
    pub model: &'a dyn ModelFns,
    /// The parameters, constant during an integration.
    pub p: &'a [f64],
    /// The pattern of the analytic Jacobian, `None` without.
    pub pattern: Option<&'a SparsityPattern>,
    /// The values of the Jacobian in the CSC order of the pattern.
    pub jac_values: Vec<f64>,
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
        let n = problem.n;
        problem
            .model
            .rhs(t, values(y, n), problem.p, values(ydot, n));
    }
    0
}

unsafe extern "C" fn jac_dense(
    t: sunrealtype,
    y: N_Vector,
    _fy: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    _tmp1: N_Vector,
    _tmp2: N_Vector,
    _tmp3: N_Vector,
) -> c_int {
    // SAFETY: as in `rhs`; the dense matrix has `n * n` values, column-major.
    unsafe {
        let problem = problem(data);
        let Some(pattern) = problem.pattern else {
            return -1;
        };
        let n = pattern.n as usize;
        problem
            .model
            .jac(t, values(y, n), problem.p, &mut problem.jac_values);
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
    _fy: N_Vector,
    jac: SUNMatrix,
    data: *mut c_void,
    _tmp1: N_Vector,
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
        problem
            .model
            .jac(t, values(y, n), problem.p, &mut problem.jac_values);
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
    /// Whether `CVodeInit` was called; later segments call `CVodeReInit`.
    initialized: bool,
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
                Linear::Klu { nnz } => {
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

    /// Starts a segment at `t0` with the state `y0` and the settings of the integrator.
    pub fn start(
        &mut self,
        t0: f64,
        y0: &[f64],
        tstop: f64,
        settings: &IntegratorSettings,
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
                match self.config.method {
                    Method::Adams => {
                        self.flag(CVodeSetNonlinearSolver(self.mem, self.nonlinear), t0)?;
                    }
                    Method::Bdf(linear) => {
                        self.flag(CVodeSetLinearSolver(self.mem, self.solver, self.matrix), t0)?;
                        let jac: CVLsJacFn = match linear {
                            Linear::Dense { analytic: true } => Some(jac_dense),
                            Linear::Dense { analytic: false } => None,
                            Linear::Klu { .. } => Some(jac_sparse),
                        };
                        self.flag(CVodeSetJacFn(self.mem, jac), t0)?;
                    }
                }
                self.initialized = true;
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
        }
        Ok(())
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

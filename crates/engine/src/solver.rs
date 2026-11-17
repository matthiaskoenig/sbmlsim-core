//! A SUNDIALS integrator with its owned context, vectors, matrix and solvers,
//! reused across segments and simulations of the same configuration.
//! ODEs use CVODE. Semi-explicit index-one DAEs use IDA's BDF method with
//! the residual and consistent initialization callbacks in [`ida`].
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

use sbmlsim_abi::{IntegratorSettings, SimulationStats, SparsityPattern, Tolerance};
use sbmlsim_csolvers::ffi::*;

use crate::EngineError;
use crate::model::ModelFns;

mod ida;
pub(crate) use ida::Constraints;

/// The number of states above which [`LinearSolver::Auto`](sbmlsim_abi::LinearSolver::Auto)
/// takes KLU, else the dense solver.
///
/// Measured with the analytic Jacobian in wasmtime (bench/RESULTS.md, the scaling models of
/// `scripts/scaling_models.py`), with KLU in the ordering AMD, 32-bit indices and its symbolic
/// analysis kept across restarts: KLU is as fast as the dense solver at 11 to 12 states (a
/// banded chain at 12, a whole body model whose pools couple to every organ at 11) and faster
/// above, 0.9 of the dense time at 14 to 28 states, 0.6 at 64 and 0.03 to 0.1 at 512; below 11
/// states the dense solver is faster. The PBPK models of 28 to 43 states take 0.6 to 0.9 of the
/// dense time with KLU. (Before, with COLAMD, the crossover was at 11 to 20 states and the
/// threshold 16.)
pub const KLU_THRESHOLD: usize = 12;

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

impl Difference {
    /// Evaluate the colored difference quotient for the increments of the active integrator.
    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        &mut self,
        model: &dyn ModelFns,
        t: f64,
        y_values: &[f64],
        p: &[f64],
        fy: &[f64],
        pattern: &SparsityPattern,
        jac_values: &mut [f64],
        counts: &mut Counts,
    ) {
        self.perturbed.copy_from_slice(y_values);
        for c in 0..self.n_colors() {
            let columns = &self.columns[self.starts[c]..self.starts[c + 1]];
            for &j in columns {
                self.perturbed[j] += self.increments[j];
            }
            model.rhs(t, &self.perturbed, p, &mut self.rates);
            counts.jacobian_rhs += 1;
            for &j in columns {
                self.perturbed[j] = y_values[j];
                // `N_VLinearSum(inc_inv, ftemp, -inc_inv, fy)` is `inc_inv * (ftemp - fy)`
                let inc_inv = 1.0 / self.increments[j];
                let (start, end) = (pattern.col_ptr[j] as usize, pattern.col_ptr[j + 1] as usize);
                for k in start..end {
                    let i = pattern.row_idx[k] as usize;
                    jac_values[k] = inc_inv * (self.rates[i] - fy[i]);
                }
            }
        }
    }
}

/// The method of a configuration, as libroadrunner chooses it from `stiff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// Stiff: BDF with the Newton iteration and a linear solver.
    Bdf(Linear),
    /// IDA for a semi-explicit index-one DAE.
    Ida { linear: Linear, n_algebraic: usize },
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
    /// Number of trailing algebraic residual equations.
    pub n_algebraic: usize,
    /// Project IDA interpolation when algebraic jumps preclude including
    /// algebraic variables in the local error norm.
    pub interpolation: Option<Box<Constraints<'a>>>,
    /// The number of root functions of the model.
    pub n_roots: usize,
    /// The number of root functions of the discontinuities of the math, the last ones of the
    /// `n_roots`: at their zeros the integration starts again without executing anything.
    pub n_discontinuity_roots: usize,
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
    pub difference: Option<&'a mut Difference>,
    /// The CVODE memory of the running integration, set by [`Integrator::integrate`] (the sparse
    /// difference quotient reads its error weights and step size).
    pub mem: *mut c_void,
    /// The calls of the model the callbacks of CVODE and the engine made.
    pub counts: Counts,
    /// The first native callback failure, returned after control leaves C.
    pub callback_error: Option<EngineError>,
}

/// The calls of the model during a simulation, counted where they are made (the counters of
/// [`SimulationStats`] which are no counters of CVODE).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// The calls of the right hand side by CVODE.
    pub rhs: u64,
    /// The calls of the Jacobian function by CVODE.
    pub jacobian: u64,
    /// The calls of the right hand side by the sparse difference quotient.
    pub jacobian_rhs: u64,
    /// The analytic Jacobians with an entry which is not finite.
    pub fallbacks: u64,
    /// The calls of the root functions by CVODE.
    pub roots: u64,
    /// The calls of the right hand side by the criterion of the steady state.
    pub steady_state_rhs: u64,
}

impl Counts {
    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            rhs: self.rhs + other.rhs,
            jacobian: self.jacobian + other.jacobian,
            jacobian_rhs: self.jacobian_rhs + other.jacobian_rhs,
            fallbacks: self.fallbacks + other.fallbacks,
            roots: self.roots + other.roots,
            steady_state_rhs: self.steady_state_rhs + other.steady_state_rhs,
        }
    }
}

/// The counters of the SUNDIALS integrator, summed over the (re)initializations of a simulation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SolverCounts {
    /// The steps.
    pub steps: u64,
    /// The setups of the linear solver.
    pub linear_setups: u64,
    /// The failures of the local error test.
    pub error_test_failures: u64,
    /// The iterations of the nonlinear solver.
    pub nonlinear_iterations: u64,
    /// The convergence failures of the nonlinear solver.
    pub nonlinear_convergence_failures: u64,
    /// The (re)initializations.
    pub integrations: u64,
}

impl SolverCounts {
    /// The sum of two counts.
    pub(crate) fn add(self, other: SolverCounts) -> SolverCounts {
        SolverCounts {
            steps: self.steps + other.steps,
            linear_setups: self.linear_setups + other.linear_setups,
            error_test_failures: self.error_test_failures + other.error_test_failures,
            nonlinear_iterations: self.nonlinear_iterations + other.nonlinear_iterations,
            nonlinear_convergence_failures: self.nonlinear_convergence_failures
                + other.nonlinear_convergence_failures,
            integrations: self.integrations + other.integrations,
        }
    }
}

/// The statistics of a simulation from the counts of the engine, of CVODE and of the probes of
/// the triggers.
pub(crate) fn stats(counts: &Counts, cvode: &SolverCounts, probe_rhs: u64) -> SimulationStats {
    SimulationStats {
        steps: cvode.steps,
        rhs_evals: counts.rhs,
        jacobian_evals: counts.jacobian,
        jacobian_rhs_evals: counts.jacobian_rhs,
        jacobian_fallbacks: counts.fallbacks,
        linear_setups: cvode.linear_setups,
        error_test_failures: cvode.error_test_failures,
        nonlinear_iterations: cvode.nonlinear_iterations,
        nonlinear_convergence_failures: cvode.nonlinear_convergence_failures,
        root_evals: counts.roots,
        integrations: cvode.integrations,
        probe_rhs_evals: probe_rhs,
        steady_state_rhs_evals: counts.steady_state_rhs,
    }
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
    // SAFETY: `err_user_data` is the `ErrorLog` of the `Integrator` which owns the context, the
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
/// `data` is the pointer to the `Problem` [`Integrator::integrate`] set, which lives during the call.
unsafe fn problem<'a>(data: *mut c_void) -> &'a mut Problem<'a> {
    unsafe { &mut *(data as *mut Problem<'a>) }
}

/// Contains native implementation panics before they can cross the C ABI.
#[cfg(not(target_arch = "wasm32"))]
fn callback(problem: &mut Problem<'_>, call: impl FnOnce(&mut Problem<'_>) -> c_int) -> c_int {
    if problem.callback_error.is_some() {
        return -1;
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let code = call(problem);
        problem.model.check_error().map(|()| code)
    }))
    .unwrap_or_else(|error| Err(EngineError::Internal(crate::panic_message(&*error))));
    match outcome {
        Ok(code) => code,
        Err(error) => {
            problem.callback_error = Some(error);
            -1
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn callback(problem: &mut Problem<'_>, call: impl FnOnce(&mut Problem<'_>) -> c_int) -> c_int {
    call(problem)
}

unsafe extern "C" fn rhs(t: sunrealtype, y: N_Vector, ydot: N_Vector, data: *mut c_void) -> c_int {
    // SAFETY: CVODE passes the vectors of the problem and the user data set before the call.
    unsafe {
        let problem = problem(data);
        callback(problem, |problem| {
            problem.counts.rhs += 1;
            let (n, m) = (problem.n, problem.n_model);
            let ydot = values(ydot, n);
            problem
                .model
                .rhs(t, &values(y, n)[..m], &problem.p, &mut ydot[..m]);
            ydot[m..].fill(0.0);
            0
        })
    }
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
        callback(problem, |problem| {
            problem.counts.roots += 1;
            let (n, m) = (problem.n, problem.n_model);
            let g = std::slice::from_raw_parts_mut(g, problem.n_roots);
            problem.model.roots(t, &values(y, n)[..m], &problem.p, g);
            0
        })
    }
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
    problem.counts.jacobian += 1;
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
            problem.counts.fallbacks += 1;
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
        dq.evaluate(
            problem.model,
            t,
            y_values,
            &problem.p,
            fy,
            pattern,
            &mut problem.jac_values,
            &mut problem.counts,
        );
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
        callback(problem, |problem| {
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
            0
        })
    }
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
        callback(problem, |problem| {
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
            0
        })
    }
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

/// A CVODE or IDA memory of one [`Config`] with its context, vectors, matrix and solvers.
pub(crate) struct Integrator<const DAE: bool> {
    config: Config,
    ctx: SUNContext,
    mem: *mut c_void,
    y: N_Vector,
    abstol: N_Vector,
    /// IDA derivative and differential/algebraic identifier vectors.
    yp: N_Vector,
    id: N_Vector,
    /// Start time at which consistent initial conditions are still needed.
    pending_ic: Option<f64>,
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
    /// The counters of the (re)initializations of the simulation before the current one.
    totals: SolverCounts,
    /// Whether the counters of the memory belong to the running simulation (a segment of it
    /// was started since [`Integrator::begin`]).
    live: bool,
    /// The roots found at the last return at a root (`CVodeGetRootInfo`), one per root function.
    found: Vec<c_int>,
}

// These SUNDIALS calls share signatures across CVODE and IDA.
macro_rules! backend {
    ($self:expr, $cvode:ident, $ida:ident, $($arg:expr),+ $(,)?) => {{
        if DAE {
            $ida($self.mem, $($arg),+)
        } else {
            $cvode($self.mem, $($arg),+)
        }
    }};
}

impl<const DAE: bool> Drop for Integrator<DAE> {
    fn drop(&mut self) {
        // SAFETY: every pointer is null or owned by this memory; CVODE is freed before the
        // linear solver and the matrix it uses, the context last.
        unsafe {
            if !self.mem.is_null() {
                if !DAE {
                    CVodeFree(&mut self.mem);
                } else {
                    IDAFree(&mut self.mem);
                }
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
            for v in [self.abstol, self.y, self.yp, self.id] {
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

/// Checks that the vectors and the matrix of a configuration fit the 32 bit indices of SUNDIALS
/// (`sunindextype` is `int32_t`): `n` states, the `n * n` entries of the dense matrix, or the
/// `nnz` entries of the pattern and the `n` diagonal entries CVODE may add for KLU.
pub(crate) fn check_index_range(config: &Config) -> Result<(), EngineError> {
    let n = config.n as u128;
    let (entries, matrix) = match config.method {
        Method::Adams => (n, "the state vector"),
        Method::Bdf(Linear::Dense { .. })
        | Method::Ida {
            linear: Linear::Dense { .. },
            ..
        } => (n * n, "the dense matrix of the Jacobian"),
        Method::Bdf(Linear::Klu { nnz, .. })
        | Method::Ida {
            linear: Linear::Klu { nnz, .. },
            ..
        } => (nnz as u128 + n, "the sparse matrix of the Jacobian"),
    };
    let max = sunindextype::MAX as u128;
    if n > max || entries > max {
        return Err(EngineError::Request(format!(
            "{matrix} of {} states needs {entries} entries, more than the {max} the 32 bit \
             indices of SUNDIALS allow",
            config.n
        )));
    }
    Ok(())
}

impl<const DAE: bool> Integrator<DAE> {
    /// Creates the memory of a configuration; refuses one which exceeds the indices of
    /// SUNDIALS ([`check_index_range`]).
    pub fn new(config: Config) -> Result<Self, EngineError> {
        assert_eq!(DAE, matches!(config.method, Method::Ida { .. }));
        check_index_range(&config)?;
        let mut cvode = Integrator {
            config,
            ctx: ptr::null_mut(),
            mem: ptr::null_mut(),
            y: ptr::null_mut(),
            abstol: ptr::null_mut(),
            yp: ptr::null_mut(),
            id: ptr::null_mut(),
            pending_ic: None,
            matrix: ptr::null_mut(),
            solver: ptr::null_mut(),
            nonlinear: ptr::null_mut(),
            initialized: false,
            attached: false,
            errors: Box::default(),
            totals: SolverCounts::default(),
            live: false,
            found: Vec::new(),
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
            cvode.mem = match config.method {
                Method::Ida { n_algebraic, .. } => {
                    cvode.yp = N_VNew_Serial(n, cvode.ctx);
                    cvode.id = N_VNew_Serial(n, cvode.ctx);
                    if cvode.yp.is_null() || cvode.id.is_null() {
                        return Err(not_created("the IDA vectors"));
                    }
                    let (diff, alg) =
                        values(cvode.id, config.n).split_at_mut(config.n - n_algebraic);
                    diff.fill(1.0);
                    alg.fill(0.0);
                    IDACreate(cvode.ctx)
                }
                Method::Bdf(_) => CVodeCreate(CV_BDF, cvode.ctx),
                Method::Adams => CVodeCreate(CV_ADAMS, cvode.ctx),
            };
            if cvode.mem.is_null() {
                return Err(not_created("the integrator memory"));
            }
            let linear = match config.method {
                Method::Bdf(linear) | Method::Ida { linear, .. } => linear,
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
                    let extra = if let Method::Ida { n_algebraic, .. } = config.method {
                        config.n - n_algebraic
                    } else {
                        0
                    };
                    let capacity = (nnz + extra).max(1) as sunindextype;
                    cvode.matrix = SUNSparseMatrix(n, n, capacity, CSC_MAT, cvode.ctx);
                    if !cvode.matrix.is_null() {
                        // the pattern of the matrix is the same in every segment (`jac_sparse`
                        // writes the pattern of the model, CVODE adds the missing diagonal), so
                        // KLU keeps its symbolic analysis across the restarts, with bitwise the
                        // results of `SUNLinSol_KLU`
                        cvode.solver = sbmlsim_SUNLinSol_KLUKept(cvode.y, cvode.matrix, cvode.ctx);
                        // the ordering AMD, the default of KLU, which `SUNLinSol_KLU` replaces
                        // with COLAMD: on the symmetric patterns of the Jacobians of models
                        // COLAMD gives much more fill (chen KLU 489 ms -> 72 ms natively, 74%
                        // of the time was the refactorization; 556 -> 89 ms in wasmtime;
                        // mp_flat 120 -> 97 ms in wasmtime, fewer steps and about 2 to 10%
                        // faster per step; hctz and mp_core about equal)
                        if !cvode.solver.is_null() {
                            check(
                                SUNLinSol_KLUSetOrdering(cvode.solver, SUNKLU_ORDERING_AMD),
                                "SUNLinSol_KLUSetOrdering",
                            )?;
                        }
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
        if self.live {
            self.totals = self.totals.add(self.current());
        }
        // counted before the call, also when it fails (its counters are then those of the
        // segment before or 0)
        self.live = true;
        self.totals.integrations += 1;
        // SAFETY: the vectors have `n` values; the memory is initialized before it is
        // re-initialized and before the linear solver is attached.
        unsafe {
            values(self.y, n).copy_from_slice(y0);
            if DAE {
                values(self.yp, n).fill(0.0);
                if self.initialized {
                    self.flag(IDAReInit(self.mem, t0, self.y, self.yp), t0)?;
                } else {
                    self.flag(
                        IDAInit(self.mem, Some(ida::residual), t0, self.y, self.yp),
                        t0,
                    )?;
                    self.initialized = true;
                }
                self.flag(IDASetId(self.mem, self.id), t0)?;
                // CalcIC sets differential rates. For an explicitly time-dependent
                // algebraic equation its initial algebraic derivative guess is zero.
                // Permit enough reductions for IDA to estimate and control that rate.
                self.flag(IDASetMaxErrTestFails(self.mem, 30), t0)?;
                self.pending_ic = Some(t0);
            } else if self.initialized {
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
                    Method::Ida { linear, .. } => {
                        self.flag(IDASetLinearSolver(self.mem, self.solver, self.matrix), t0)?;
                        let jac: IDALsJacFn = match linear {
                            Linear::Dense { .. } => Some(ida::dense),
                            Linear::Klu { .. } => Some(ida::sparse),
                        };
                        self.flag(IDASetJacFn(self.mem, jac), t0)?;
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
                    self.flag(
                        backend!(self, CVodeSStolerances, IDASStolerances, rtol, *atol),
                        t0,
                    )?;
                }
                Tolerance::PerState(atol) => {
                    values(self.abstol, n).copy_from_slice(atol);
                    self.flag(
                        backend!(self, CVodeSVtolerances, IDASVtolerances, rtol, self.abstol),
                        t0,
                    )?;
                }
            }
            // `long` is 32 bits on wasm32: the largest count it holds (`as u32` of `i64::MAX` is
            // `u32::MAX`, so the bound changes nothing on 64-bit targets)
            let max_steps = settings.maximum_num_steps.min(c_long::MAX as u32) as c_long;
            self.flag(
                backend!(self, CVodeSetMaxNumSteps, IDASetMaxNumSteps, max_steps),
                t0,
            )?;
            // 0.0 is CVODE's own default for both: no largest step, an estimated first step
            self.flag(
                backend!(
                    self,
                    CVodeSetMaxStep,
                    IDASetMaxStep,
                    settings.maximum_time_step
                ),
                t0,
            )?;
            self.flag(
                backend!(
                    self,
                    CVodeSetInitStep,
                    IDASetInitStep,
                    settings.initial_time_step
                ),
                t0,
            )?;
            self.flag(backend!(self, CVodeSetStopTime, IDASetStopTime, tstop), t0)?;
            let nr = c_int::try_from(n_roots).map_err(|_| {
                EngineError::Request(format!(
                    "{n_roots} root functions exceed the SUNDIALS index range"
                ))
            })?;
            if !DAE {
                self.flag(CVodeRootInit(self.mem, nr, (nr > 0).then_some(roots)), t0)?;
            } else {
                self.flag(
                    IDARootInit(self.mem, nr, (nr > 0).then_some(ida::roots)),
                    t0,
                )?;
            }
        }
        Ok(())
    }

    /// Starts the counters of a simulation: the counters of the memory, which a simulation
    /// before left in it, are no part of it until a segment is started.
    pub fn begin(&mut self) {
        self.totals = SolverCounts::default();
        self.live = false;
    }

    /// The counters of CVODE of the simulation since [`Integrator::begin`].
    pub fn counts(&self) -> SolverCounts {
        if self.live {
            self.totals.add(self.current())
        } else {
            self.totals
        }
    }

    /// The counters of the current (re)initialization of the memory, 0 before the first one
    /// (CVODE answers with an error, which leaves them 0).
    fn current(&self) -> SolverCounts {
        let (mut steps, mut nfe, mut setups, mut netf) = (0, 0, 0, 0);
        let (mut qlast, mut qcur) = (0, 0);
        let (mut hinused, mut hlast, mut hcur, mut tcur) = (0.0, 0.0, 0.0, 0.0);
        let (mut nni, mut ncfn) = (0, 0);
        if self.initialized {
            // SAFETY: the memory is initialized; every out-pointer is a local
            unsafe {
                backend!(
                    self,
                    CVodeGetIntegratorStats,
                    IDAGetIntegratorStats,
                    &mut steps,
                    &mut nfe,
                    &mut setups,
                    &mut netf,
                    &mut qlast,
                    &mut qcur,
                    &mut hinused,
                    &mut hlast,
                    &mut hcur,
                    &mut tcur,
                );
                backend!(
                    self,
                    CVodeGetNonlinSolvStats,
                    IDAGetNonlinSolvStats,
                    &mut nni,
                    &mut ncfn
                );
            }
        }
        let count = |x: c_long| x.max(0) as u64;
        SolverCounts {
            steps: count(steps),
            linear_setups: count(setups),
            error_test_failures: count(netf),
            nonlinear_iterations: count(nni),
            nonlinear_convergence_failures: count(ncfn),
            integrations: 0,
        }
    }

    /// Sets the stop time of the running segment, which must not be behind the internal time of
    /// CVODE.
    pub fn set_stop(&mut self, tstop: f64) -> Result<(), EngineError> {
        // SAFETY: the memory is initialized (a segment was started).
        unsafe {
            self.flag(
                backend!(self, CVodeSetStopTime, IDASetStopTime, tstop),
                tstop,
            )
        }
    }

    /// Whether the return of CVODE at a zero of a root function (`CV_ROOT_RETURN`) is at a zero
    /// of one of the root functions of the discontinuities of `problem` (the last
    /// `problem.n_discontinuity_roots` of its `problem.n_roots`), in either direction.
    pub fn at_discontinuity(&mut self, problem: &Problem<'_>) -> Result<bool, EngineError> {
        let n = problem.n_roots;
        let first = n - problem.n_discontinuity_roots;
        if first == n {
            return Ok(false);
        }
        self.found.resize(n, 0);
        // SAFETY: the memory was initialized with `n` root functions, so CVODE writes `n` values
        // into the buffer of `n`.
        unsafe {
            let flag = backend!(
                self,
                CVodeGetRootInfo,
                IDAGetRootInfo,
                self.found.as_mut_ptr()
            );
            self.flag(flag, f64::NAN)?;
        }
        Ok(self.found[first..].iter().any(|&f| f != 0))
    }

    /// The steps of CVODE in the simulation since [`Integrator::begin`].
    pub fn steps(&self) -> u64 {
        let mut current: c_long = 0;
        if self.live && self.initialized {
            // SAFETY: the memory is initialized; the out-pointer is a local.
            unsafe { backend!(self, CVodeGetNumSteps, IDAGetNumSteps, &mut current) };
        }
        self.totals.steps + current.max(0) as u64
    }

    /// Sets the largest number of steps of the next call of [`Integrator::integrate`] (until the next
    /// start, which sets the one of the settings).
    pub fn set_max_steps(&mut self, steps: u64) -> Result<(), EngineError> {
        let steps = steps.min(c_long::MAX as u64) as c_long;
        // SAFETY: the memory is initialized (a segment was started).
        unsafe {
            self.flag(
                backend!(self, CVodeSetMaxNumSteps, IDASetMaxNumSteps, steps),
                f64::NAN,
            )
        }
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
            self.flag(backend!(self, CVodeSetUserData, IDASetUserData, data), tout)?;
            let flag = if !DAE {
                CVode(self.mem, tout, self.y, &mut tret, itask)
            } else {
                self.consistent(problem)?;
                IDASolve(self.mem, tout, &mut tret, self.y, self.yp, itask)
            };
            if let Some(error) = problem.callback_error.take() {
                return Err(error);
            }
            self.flag(flag, tret)?;
            if DAE && let Some(projection) = &mut problem.interpolation {
                projection.project(tret, values(self.y, problem.n), &problem.p)?;
            }
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

    #[test]
    fn ode_contexts_keep_projection_storage_out_of_line() {
        use crate::events::Events;
        use std::mem::size_of;

        assert!(size_of::<Problem<'_>>() < size_of::<Constraints<'_>>());
        assert!(size_of::<Events<'_, false>>() < size_of::<Constraints<'_>>());
        let cvode = Integrator::<false>::new(Config {
            n: 1,
            method: Method::Adams,
        })
        .unwrap();
        assert!(cvode.yp.is_null() && cvode.id.is_null());
    }

    /// Integrates Robertson with a configuration to `4e10`, returns the state at the outputs
    /// `0.4 * 10^i` and the number of steps.
    fn robertson(linear: Linear, mut difference: Option<Difference>) -> (Vec<f64>, c_long) {
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
        let mut cvode = Integrator::<false>::new(Config {
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
            n_algebraic: 0,
            interpolation: None,
            n_roots: 0,
            n_discontinuity_roots: 0,
            model: &model,
            p,
            pattern: Some(desc.jacobian.pattern()),
            jac_values: vec![0.0; desc.jacobian.pattern().nnz()],
            analytic,
            difference: difference.as_mut(),
            mem: ptr::null_mut(),
            counts: Counts::default(),
            callback_error: None,
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
        fn selections(&self, _t: f64, _y: &[f64], _p: &[f64], _mask: &[u8], _out: &mut [f64]) {}
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
    fn chain(linear: Linear, mut difference: Option<Difference>) -> (Vec<f64>, c_long) {
        let pattern = chain_pattern();
        let mut cvode = Integrator::<false>::new(Config {
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
            n_algebraic: 0,
            interpolation: None,
            n_roots: 0,
            n_discontinuity_roots: 0,
            model: &Chain,
            p: vec![],
            pattern: Some(&pattern),
            jac_values: vec![0.0; pattern.nnz()],
            analytic: false,
            difference: difference.as_mut(),
            mem: ptr::null_mut(),
            counts: Counts::default(),
            callback_error: None,
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

    /// Integrates Robertson with KLU and the analytic Jacobian to `4e10`, restarting CVODE
    /// (`CVodeReInit`) at every output `0.4 * 10^i`; with `plain` the linear solver is
    /// `SUNLinSol_KLU` (with AMD as well), which analyzes the pattern again after every restart.
    /// The states at the outputs, the steps of every segment and the counters of the solver of
    /// the memory (the symbolic analyses and the factorizations which reused one).
    fn robertson_restarted(plain: bool) -> (Vec<f64>, Vec<c_long>, Option<(c_long, c_long)>) {
        let model = Robertson;
        let desc = model.description();
        let pattern = desc.jacobian.pattern();
        let (mut p, mut y) = (vec![0.0; 3], vec![0.0; 3]);
        model.defaults(&mut p, &mut y);
        let mut cvode = Integrator::<false>::new(Config {
            n: 3,
            method: Method::Bdf(Linear::Klu {
                nnz: pattern.nnz(),
                jacobian: Jacobian::Analytic,
            }),
        })
        .unwrap();
        if plain {
            // SAFETY: the solver is not attached yet (the first `start` attaches it)
            unsafe {
                SUNLinSolFree(cvode.solver);
                cvode.solver = SUNLinSol_KLU(cvode.y, cvode.matrix, cvode.ctx);
                assert!(!cvode.solver.is_null());
                // the ordering of the solver of `Integrator::new`
                assert_eq!(
                    SUNLinSol_KLUSetOrdering(cvode.solver, SUNKLU_ORDERING_AMD),
                    0
                );
            }
        }
        let settings = IntegratorSettings {
            relative_tolerance: 1e-8,
            absolute_tolerance: Tolerance::PerState(vec![1e-12, 1e-14, 1e-10]),
            ..IntegratorSettings::default()
        };
        let mut problem = Problem {
            n: 3,
            n_model: 3,
            n_algebraic: 0,
            interpolation: None,
            n_roots: 0,
            n_discontinuity_roots: 0,
            model: &model,
            p,
            pattern: Some(pattern),
            jac_values: vec![0.0; pattern.nnz()],
            analytic: true,
            difference: None,
            mem: ptr::null_mut(),
            counts: Counts::default(),
            callback_error: None,
        };
        let (mut out, mut steps, mut t) = (Vec::new(), Vec::new(), 0.0);
        for i in 0..11 {
            let y0 = if i == 0 {
                y.clone()
            } else {
                cvode.state().to_vec()
            };
            cvode.start(t, &y0, 4e10, &settings, 0).unwrap();
            t = 0.4 * 10f64.powi(i);
            cvode.integrate(&mut problem, t, CV_NORMAL).unwrap();
            out.extend_from_slice(cvode.state());
            let mut n: c_long = 0;
            // SAFETY: the memory is initialized.
            unsafe { CVodeGetNumSteps(cvode.mem, &mut n) };
            steps.push(n);
        }
        let (mut analyses, mut reuses): (c_long, c_long) = (0, 0);
        // SAFETY: the solver belongs to the memory; another one than the kept KLU is refused.
        let stats = unsafe {
            sbmlsim_SUNLinSol_KLUKeptStats(cvode.solver, &mut analyses, &mut reuses) == 0
        };
        (out, steps, stats.then_some((analyses, reuses)))
    }

    #[test]
    fn configurations_beyond_the_32_bit_indices_are_refused() {
        let jacobian = Jacobian::Analytic;
        let config = |n, method| Config { n, method };
        let dense = |n| config(n, Method::Bdf(Linear::Dense { jacobian }));
        let klu = |n, nnz| config(n, Method::Bdf(Linear::Klu { nnz, jacobian }));
        // 46340^2 fits, 46341^2 does not
        assert!(check_index_range(&dense(46_340)).is_ok());
        let err = check_index_range(&dense(46_341)).unwrap_err();
        assert!(
            matches!(&err, EngineError::Request(m) if m.contains("dense matrix")
                && m.contains("2147488281 entries")),
            "{err:?}"
        );
        let max = i32::MAX as usize;
        assert!(check_index_range(&klu(1000, max - 1000)).is_ok());
        let err = check_index_range(&klu(1000, max - 999)).unwrap_err();
        assert!(
            matches!(&err, EngineError::Request(m) if m.contains("sparse matrix")),
            "{err:?}"
        );
        assert!(check_index_range(&config(max, Method::Adams)).is_ok());
        assert!(check_index_range(&config(max + 1, Method::Adams)).is_err());
        assert!(check_index_range(&klu(max + 1, 0)).is_err());
    }

    #[test]
    fn klu_keeps_its_symbolic_analysis_across_restarts_with_the_same_bits() {
        let kept = robertson_restarted(false);
        let plain = robertson_restarted(true);
        // one analysis at the first factorization; every restart (10) reuses it at its first
        // factorization
        assert_eq!(kept.2, Some((1, 10)));
        assert_eq!(plain.2, None);
        assert!(kept.1.iter().sum::<c_long>() > 500, "{:?} steps", kept.1);
        assert_eq!(kept.1, plain.1);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&kept.0), bits(&plain.0));
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

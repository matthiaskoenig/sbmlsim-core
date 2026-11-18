//! The `extern "C"` declarations of the parts of SUNDIALS 7.9 the engine calls.
//!
//! The signatures are the ones of the headers of SUNDIALS v7.9.0 (`sundials_context.h`,
//! `sundials_nvector.h`, `nvector_serial.h`, `sundials_matrix.h`, `sunmatrix_dense.h`,
//! `sunmatrix_sparse.h`, `sundials_linearsolver.h`, `sunlinsol_dense.h`, `sunlinsol_klu.h`,
//! `sunlinsol_spgmr.h`,
//! `sundials_nonlinearsolver.h`, `sunnonlinsol_fixedpoint.h`, `cvodes.h`, `cvodes_ls.h`, `ida.h`, `ida_ls.h`) as this
//! crate configures them: `sunrealtype` is `double`,
//! `sunindextype` is `int32_t`, the counters are `long int` (32 bits on wasm32) and `SUNComm` is
//! `int` (no MPI). The functions with the prefix `sbmlsim_` are the C sources of this crate.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_int, c_long, c_void};

/// The floating point type of SUNDIALS, `double`.
pub type sunrealtype = f64;
/// The index type of SUNDIALS, `int32_t` (`SUNDIALS_INDEX_SIZE=32`).
pub type sunindextype = i32;
/// A boolean of SUNDIALS (`int`).
pub type sunbooleantype = c_int;
/// An error code of SUNDIALS, `0` is success.
pub type SUNErrCode = c_int;
/// The communicator of SUNDIALS, `int` without MPI.
pub type SUNComm = c_int;

/// The communicator of a context without MPI.
pub const SUN_COMM_NULL: SUNComm = 0;

/// Declares an opaque C struct and the pointer type SUNDIALS names it by.
macro_rules! opaque {
    ($($(#[$doc:meta])* $pointer:ident => $object:ident;)*) => {$(
        #[doc = concat!("The opaque object behind [`", stringify!($pointer), "`].")]
        #[repr(C)]
        pub struct $object {
            _private: [u8; 0],
            _not_send_sync: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }
        $(#[$doc])*
        pub type $pointer = *mut $object;
    )*};
}

opaque! {
    /// A context of SUNDIALS, which every object of a simulation belongs to.
    SUNContext => _SUNContext;
    /// A vector (`N_Vector`).
    N_Vector => _generic_N_Vector;
    /// A matrix (`SUNMatrix`).
    SUNMatrix => _generic_SUNMatrix;
    /// A linear solver (`SUNLinearSolver`).
    SUNLinearSolver => _generic_SUNLinearSolver;
    /// A nonlinear solver (`SUNNonlinearSolver`).
    SUNNonlinearSolver => _generic_SUNNonlinearSolver;
}

/// The right hand side `ydot = f(t, y)`; returns 0 on success, a positive value for a
/// recoverable and a negative value for an unrecoverable error.
pub type CVRhsFn = Option<
    unsafe extern "C" fn(
        t: sunrealtype,
        y: N_Vector,
        ydot: N_Vector,
        user_data: *mut c_void,
    ) -> c_int,
>;
/// The root functions `gout = g(t, y)`.
pub type CVRootFn = Option<
    unsafe extern "C" fn(
        t: sunrealtype,
        y: N_Vector,
        gout: *mut sunrealtype,
        user_data: *mut c_void,
    ) -> c_int,
>;
/// The Jacobian `Jac = df/dy (t, y)`, `fy = f(t, y)`.
pub type CVLsJacFn = Option<
    unsafe extern "C" fn(
        t: sunrealtype,
        y: N_Vector,
        fy: N_Vector,
        jac: SUNMatrix,
        user_data: *mut c_void,
        tmp1: N_Vector,
        tmp2: N_Vector,
        tmp3: N_Vector,
    ) -> c_int,
>;

/// The right hand side of every forward sensitivity at once:
/// `ySdot_i = df/dy yS_i + df/dp_i` for `i < Ns`, with `ydot = f(t, y)` given.
///
/// One call of this computes what the whole sensitivity system needs of a step, so a caller
/// which has an analytic `df/dy` evaluates it once per call and not once per sensitivity.
pub type CVSensRhsFn = Option<
    unsafe extern "C" fn(
        Ns: c_int,
        t: sunrealtype,
        y: N_Vector,
        ydot: N_Vector,
        yS: *mut N_Vector,
        ySdot: *mut N_Vector,
        user_data: *mut c_void,
        tmp1: N_Vector,
        tmp2: N_Vector,
    ) -> c_int,
>;
/// The right hand side of one forward sensitivity, `iS` of `Ns`.
pub type CVSensRhs1Fn = Option<
    unsafe extern "C" fn(
        Ns: c_int,
        t: sunrealtype,
        y: N_Vector,
        ydot: N_Vector,
        iS: c_int,
        yS: N_Vector,
        ySdot: N_Vector,
        user_data: *mut c_void,
        tmp1: N_Vector,
        tmp2: N_Vector,
    ) -> c_int,
>;

/// Set up a preconditioner for `I - gamma J`, optionally reusing its saved Jacobian.
pub type CVLsPrecSetupFn = Option<
    unsafe extern "C" fn(
        sunrealtype,
        N_Vector,
        N_Vector,
        sunbooleantype,
        *mut sunbooleantype,
        sunrealtype,
        *mut c_void,
    ) -> c_int,
>;
/// Solve the preconditioner system `P z = r`.
pub type CVLsPrecSolveFn = Option<
    unsafe extern "C" fn(
        sunrealtype,
        N_Vector,
        N_Vector,
        N_Vector,
        N_Vector,
        sunrealtype,
        sunrealtype,
        c_int,
        *mut c_void,
    ) -> c_int,
>;

/// An error handler of a context: the line, the function and the file of the error, its message,
/// its code, the user data given to [`SUNContext_PushErrHandler`] and the context.
pub type SUNErrHandlerFn = Option<
    unsafe extern "C" fn(
        line: c_int,
        func: *const c_char,
        file: *const c_char,
        msg: *const c_char,
        err_code: SUNErrCode,
        err_user_data: *mut c_void,
        sunctx: SUNContext,
    ),
>;

/// The linear multistep method Adams-Moulton (non-stiff).
pub const CV_ADAMS: c_int = 1;
/// The linear multistep method BDF (stiff).
pub const CV_BDF: c_int = 2;
/// `CVode` integrates past `tout` and interpolates `y(tout)`.
pub const CV_NORMAL: c_int = 1;
/// `CVode` takes one internal step.
pub const CV_ONE_STEP: c_int = 2;
/// `CVode` succeeded.
pub const CV_SUCCESS: c_int = 0;
/// `CVode` reached the stop time.
pub const CV_TSTOP_RETURN: c_int = 1;
/// `CVode` found a root.
pub const CV_ROOT_RETURN: c_int = 2;
/// `CVode` took `mxstep` steps before reaching `tout`.
pub const CV_TOO_MUCH_WORK: c_int = -1;
/// The states and their sensitivities are corrected together by one nonlinear solve
/// (`ism` of `CVodeSensInit`).
pub const CV_SIMULTANEOUS: c_int = 1;
/// The states are corrected first and the sensitivities after them, all at once.
pub const CV_STAGGERED: c_int = 2;
/// The states are corrected first and every sensitivity after them on its own.
pub const CV_STAGGERED1: c_int = 3;
/// The difference quotient of `df/dp` of CVODES is centered (`DQtype`).
pub const CV_CENTERED: c_int = 1;
/// The difference quotient of `df/dp` of CVODES is forward.
pub const CV_FORWARD: c_int = 2;
/// A sparse matrix in compressed sparse column format (`SUN_CSC_MAT`).
pub const CSC_MAT: c_int = 0;
/// A sparse matrix in compressed sparse row format (`SUN_CSR_MAT`).
pub const CSR_MAT: c_int = 1;
/// The fill-reducing ordering AMD of KLU (`SUNLinSol_KLUSetOrdering`); 1 is COLAMD, the
/// default of `SUNLinSol_KLU`, 2 is the ordering of the user.
pub const SUNKLU_ORDERING_AMD: c_int = 0;
/// Left preconditioning of an iterative linear solver (`sundials_iterative.h`).
pub const SUN_PREC_LEFT: c_int = 1;

/// IDA residual F(t, y, yp) and root callbacks.
pub type IDAResFn =
    Option<unsafe extern "C" fn(sunrealtype, N_Vector, N_Vector, N_Vector, *mut c_void) -> c_int>;
/// IDA root functions.
pub type IDARootFn = Option<
    unsafe extern "C" fn(sunrealtype, N_Vector, N_Vector, *mut sunrealtype, *mut c_void) -> c_int,
>;
/// IDA Newton matrix dF/dy + cj dF/dyp.
pub type IDALsJacFn = Option<
    unsafe extern "C" fn(
        sunrealtype,
        sunrealtype,
        N_Vector,
        N_Vector,
        N_Vector,
        SUNMatrix,
        *mut c_void,
        N_Vector,
        N_Vector,
        N_Vector,
    ) -> c_int,
>;
/// Hold differential states fixed and calculate algebraic states and differential rates.
pub const IDA_YA_YDP_INIT: c_int = 1;

unsafe extern "C" {
    // the context
    pub fn SUNContext_Create(comm: SUNComm, sunctx_out: *mut SUNContext) -> SUNErrCode;
    pub fn SUNContext_Free(ctx: *mut SUNContext) -> SUNErrCode;
    /// Removes the error handlers of a context, so an error is only returned and not printed.
    pub fn SUNContext_ClearErrHandlers(sunctx: SUNContext) -> SUNErrCode;
    /// Pushes an error handler onto the stack of a context, called with `err_user_data`.
    pub fn SUNContext_PushErrHandler(
        sunctx: SUNContext,
        err_fn: SUNErrHandlerFn,
        err_user_data: *mut c_void,
    ) -> SUNErrCode;

    // vectors
    pub fn N_VNew_Serial(vec_length: sunindextype, sunctx: SUNContext) -> N_Vector;
    pub fn N_VDestroy(v: N_Vector);
    pub fn N_VGetArrayPointer(v: N_Vector) -> *mut sunrealtype;
    /// Data of a serial vector created by `N_VNew_Serial` or cloned from one.
    pub fn N_VGetArrayPointer_Serial(v: N_Vector) -> *mut sunrealtype;
    /// The weighted root mean square norm `sqrt(sum((x_i * w_i)^2) / n)`.
    pub fn N_VWrmsNorm(x: N_Vector, w: N_Vector) -> sunrealtype;
    /// An array of `count` vectors of the shape of `w`, for the forward sensitivities.
    pub fn N_VCloneVectorArray(count: c_int, w: N_Vector) -> *mut N_Vector;
    pub fn N_VDestroyVectorArray(vs: *mut N_Vector, count: c_int);
    pub fn N_VConst(c: sunrealtype, z: N_Vector);

    // matrices
    pub fn SUNDenseMatrix(m: sunindextype, n: sunindextype, sunctx: SUNContext) -> SUNMatrix;
    /// The column-major values of a dense matrix, `A(i, j)` at `j * m + i`.
    pub fn SUNDenseMatrix_Data(a: SUNMatrix) -> *mut sunrealtype;
    pub fn SUNSparseMatrix(
        m: sunindextype,
        n: sunindextype,
        nnz: sunindextype,
        sparsetype: c_int,
        sunctx: SUNContext,
    ) -> SUNMatrix;
    pub fn SUNSparseMatrix_NNZ(a: SUNMatrix) -> sunindextype;
    /// The non-zero values of a sparse matrix (`nnz` of them).
    pub fn SUNSparseMatrix_Data(a: SUNMatrix) -> *mut sunrealtype;
    /// The rows (CSC) or columns (CSR) of the non-zero values.
    pub fn SUNSparseMatrix_IndexValues(a: SUNMatrix) -> *mut sunindextype;
    /// The `n + 1` (CSC) or `m + 1` (CSR) offsets of the columns or rows into the values.
    pub fn SUNSparseMatrix_IndexPointers(a: SUNMatrix) -> *mut sunindextype;
    pub fn SUNMatZero(a: SUNMatrix) -> SUNErrCode;
    pub fn SUNMatDestroy(a: SUNMatrix);

    // linear solvers
    pub fn SUNLinSol_Dense(y: N_Vector, a: SUNMatrix, sunctx: SUNContext) -> SUNLinearSolver;
    pub fn SUNLinSol_KLU(y: N_Vector, a: SUNMatrix, sunctx: SUNContext) -> SUNLinearSolver;
    /// Sets the fill-reducing ordering of the symbolic analysis of a KLU solver (0 AMD,
    /// 1 COLAMD, 2 the ordering of the user).
    pub fn SUNLinSol_KLUSetOrdering(s: SUNLinearSolver, ordering_choice: c_int) -> SUNErrCode;
    /// `SUNLinSol_KLU` which keeps its symbolic analysis across the initializations of the
    /// solver (every `CVodeReInit`): the first factorization after one reuses the analysis while
    /// the pattern of the matrix and the settings of KLU are the ones it was made for, with
    /// bitwise the same results (`csrc/sunlinsol_klu_kept.c` of this crate).
    pub fn sbmlsim_SUNLinSol_KLUKept(
        y: N_Vector,
        a: SUNMatrix,
        sunctx: SUNContext,
    ) -> SUNLinearSolver;
    /// The symbolic analyses a solver of [`sbmlsim_SUNLinSol_KLUKept`] made and the
    /// factorizations which reused a kept one; `SUN_ERR_ARG_CORRUPT` for another solver.
    pub fn sbmlsim_SUNLinSol_KLUKeptStats(
        s: SUNLinearSolver,
        analyses: *mut c_long,
        reuses: *mut c_long,
    ) -> SUNErrCode;
    pub fn SUNLinSolFree(s: SUNLinearSolver) -> SUNErrCode;
    /// Scaled preconditioned GMRES; `maxl` is the Krylov subspace dimension.
    pub fn SUNLinSol_SPGMR(
        y: N_Vector,
        pretype: c_int,
        maxl: c_int,
        sunctx: SUNContext,
    ) -> SUNLinearSolver;
    pub fn SUNLinSol_SPGMRSetMaxRestarts(s: SUNLinearSolver, maxrs: c_int) -> SUNErrCode;

    // nonlinear solvers
    /// The fixed point (functional) iteration with `m` acceleration vectors (0 is none).
    pub fn SUNNonlinSol_FixedPoint(y: N_Vector, m: c_int, sunctx: SUNContext)
    -> SUNNonlinearSolver;
    /// The same iteration for `count` sensitivity vectors of the shape of `y`, which the
    /// staggered forward sensitivities of a non-stiff method need: the Newton iteration
    /// `CVodeSensInit` attaches by default calls the linear solver, which a run of the Adams
    /// method has not got.
    pub fn SUNNonlinSol_FixedPointSens(
        count: c_int,
        y: N_Vector,
        m: c_int,
        sunctx: SUNContext,
    ) -> SUNNonlinearSolver;
    pub fn SUNNonlinSolFree(nls: SUNNonlinearSolver) -> SUNErrCode;

    // CVODE
    pub fn CVodeCreate(lmm: c_int, sunctx: SUNContext) -> *mut c_void;
    pub fn CVodeInit(cvode_mem: *mut c_void, f: CVRhsFn, t0: sunrealtype, y0: N_Vector) -> c_int;
    pub fn CVodeReInit(cvode_mem: *mut c_void, t0: sunrealtype, y0: N_Vector) -> c_int;
    pub fn CVodeSStolerances(
        cvode_mem: *mut c_void,
        reltol: sunrealtype,
        abstol: sunrealtype,
    ) -> c_int;
    pub fn CVodeSVtolerances(
        cvode_mem: *mut c_void,
        reltol: sunrealtype,
        abstol: N_Vector,
    ) -> c_int;
    pub fn CVodeSetUserData(cvode_mem: *mut c_void, user_data: *mut c_void) -> c_int;
    pub fn CVodeGetUserData(cvode_mem: *mut c_void, user_data: *mut *mut c_void) -> c_int;
    pub fn CVodeSetMaxOrd(cvode_mem: *mut c_void, maxord: c_int) -> c_int;
    pub fn CVodeSetMaxNumSteps(cvode_mem: *mut c_void, mxsteps: c_long) -> c_int;
    pub fn CVodeSetMaxStep(cvode_mem: *mut c_void, hmax: sunrealtype) -> c_int;
    pub fn CVodeSetInitStep(cvode_mem: *mut c_void, hin: sunrealtype) -> c_int;
    pub fn CVodeSetStopTime(cvode_mem: *mut c_void, tstop: sunrealtype) -> c_int;
    pub fn CVodeRootInit(cvode_mem: *mut c_void, nrtfn: c_int, g: CVRootFn) -> c_int;
    pub fn CVodeSetRootDirection(cvode_mem: *mut c_void, rootdir: *mut c_int) -> c_int;
    pub fn CVodeSetLinearSolver(cvode_mem: *mut c_void, ls: SUNLinearSolver, a: SUNMatrix)
    -> c_int;
    pub fn CVodeSetNonlinearSolver(cvode_mem: *mut c_void, nls: SUNNonlinearSolver) -> c_int;
    pub fn CVodeSetJacFn(cvode_mem: *mut c_void, jac: CVLsJacFn) -> c_int;
    pub fn CVodeSetPreconditioner(
        cvode_mem: *mut c_void,
        setup: CVLsPrecSetupFn,
        solve: CVLsPrecSolveFn,
    ) -> c_int;
    pub fn CVodeGetNumLinIters(cvode_mem: *mut c_void, n: *mut c_long) -> c_int;
    pub fn CVodeGetNumLinConvFails(cvode_mem: *mut c_void, n: *mut c_long) -> c_int;
    pub fn CVodeGetNumJtimesEvals(cvode_mem: *mut c_void, n: *mut c_long) -> c_int;
    pub fn CVodeGetNumPrecEvals(cvode_mem: *mut c_void, n: *mut c_long) -> c_int;
    pub fn CVodeGetNumPrecSolves(cvode_mem: *mut c_void, n: *mut c_long) -> c_int;
    pub fn CVode(
        cvode_mem: *mut c_void,
        tout: sunrealtype,
        yout: N_Vector,
        tret: *mut sunrealtype,
        itask: c_int,
    ) -> c_int;
    pub fn CVodeGetRootInfo(cvode_mem: *mut c_void, rootsfound: *mut c_int) -> c_int;
    pub fn CVodeGetDky(cvode_mem: *mut c_void, t: sunrealtype, k: c_int, dky: N_Vector) -> c_int;
    pub fn CVodeGetLastOrder(cvode_mem: *mut c_void, q: *mut c_int) -> c_int;
    pub fn CVodeGetLastStep(cvode_mem: *mut c_void, h: *mut f64) -> c_int;
    pub fn CVodeGetCurrentTime(cvode_mem: *mut c_void, t: *mut f64) -> c_int;
    pub fn IDAGetDky(ida_mem: *mut c_void, t: f64, k: c_int, dky: N_Vector) -> c_int;
    pub fn IDAGetLastOrder(ida_mem: *mut c_void, q: *mut c_int) -> c_int;
    pub fn IDAGetLastStep(ida_mem: *mut c_void, h: *mut f64) -> c_int;
    pub fn IDAGetCurrentTime(ida_mem: *mut c_void, t: *mut f64) -> c_int;
    pub fn CVodeGetNumSteps(cvode_mem: *mut c_void, nsteps: *mut c_long) -> c_int;
    /// The step size of the step being attempted (`cv_next_h`, which is `cv_h` while CVODE
    /// sets up the linear solver).
    pub fn CVodeGetCurrentStep(cvode_mem: *mut c_void, hcur: *mut sunrealtype) -> c_int;
    /// Copies the error weights `1 / (rtol |y_i| + atol_i)` into `eweight`.
    pub fn CVodeGetErrWeights(cvode_mem: *mut c_void, eweight: N_Vector) -> c_int;
    pub fn CVodeGetIntegratorStats(
        cvode_mem: *mut c_void,
        nsteps: *mut c_long,
        nfevals: *mut c_long,
        nlinsetups: *mut c_long,
        netfails: *mut c_long,
        qlast: *mut c_int,
        qcur: *mut c_int,
        hinused: *mut sunrealtype,
        hlast: *mut sunrealtype,
        hcur: *mut sunrealtype,
        tcur: *mut sunrealtype,
    ) -> c_int;
    pub fn CVodeGetNonlinSolvStats(
        cvode_mem: *mut c_void,
        nniters: *mut c_long,
        nncfails: *mut c_long,
    ) -> c_int;
    pub fn CVodeGetNumRhsEvals(cvode_mem: *mut c_void, nfevals: *mut c_long) -> c_int;
    pub fn CVodeFree(cvode_mem: *mut *mut c_void);

    // CVODES, the forward sensitivities: `Ns` of them, `ism` one of [`CV_SIMULTANEOUS`],
    // [`CV_STAGGERED`] and [`CV_STAGGERED1`], and `yS0` their initial values (`dy0/dp`).
    // Without a right hand side of the caller (`None`), CVODES differences the one of the
    // model in the parameters [`CVodeSetSensParams`] gave it.
    pub fn CVodeSensInit(
        cvode_mem: *mut c_void,
        Ns: c_int,
        ism: c_int,
        fS: CVSensRhsFn,
        yS0: *mut N_Vector,
    ) -> c_int;
    pub fn CVodeSensInit1(
        cvode_mem: *mut c_void,
        Ns: c_int,
        ism: c_int,
        fS1: CVSensRhs1Fn,
        yS0: *mut N_Vector,
    ) -> c_int;
    pub fn CVodeSensReInit(cvode_mem: *mut c_void, ism: c_int, yS0: *mut N_Vector) -> c_int;
    /// The tolerances of the sensitivities from the ones of the states and the scaling of the
    /// parameters, which is what a caller without a tolerance of its own wants.
    pub fn CVodeSensEEtolerances(cvode_mem: *mut c_void) -> c_int;
    /// The nonlinear solver of the staggered sensitivities, which replaces the Newton
    /// iteration `CVodeSensInit` attaches; the caller owns the solver it passes.
    pub fn CVodeSetNonlinearSolverSensStg(cvode_mem: *mut c_void, nls: SUNNonlinearSolver)
    -> c_int;
    pub fn CVodeSensSStolerances(
        cvode_mem: *mut c_void,
        reltolS: sunrealtype,
        abstolS: *mut sunrealtype,
    ) -> c_int;
    /// Whether the sensitivities take part in the error control of a step (1 by default).
    pub fn CVodeSetSensErrCon(cvode_mem: *mut c_void, errconS: sunbooleantype) -> c_int;
    /// The difference quotient of CVODES: [`CV_CENTERED`] or [`CV_FORWARD`] and the ratio at
    /// which it switches to the one of the simultaneous increments.
    pub fn CVodeSetSensDQMethod(
        cvode_mem: *mut c_void,
        DQtype: c_int,
        DQrhomax: sunrealtype,
    ) -> c_int;
    /// The parameters the sensitivities are of: `p` the values the right hand side reads,
    /// which the difference quotient of CVODES perturbs in place, `pbar` their scaling and
    /// `plist` which of them (`null` for the first `Ns`).
    pub fn CVodeSetSensParams(
        cvode_mem: *mut c_void,
        p: *mut sunrealtype,
        pbar: *mut sunrealtype,
        plist: *mut c_int,
    ) -> c_int;
    pub fn CVodeSensToggleOff(cvode_mem: *mut c_void) -> c_int;
    /// The sensitivities at the time the last [`CVode`] reached.
    pub fn CVodeGetSens(
        cvode_mem: *mut c_void,
        tret: *mut sunrealtype,
        ySout: *mut N_Vector,
    ) -> c_int;
    pub fn CVodeGetSens1(
        cvode_mem: *mut c_void,
        tret: *mut sunrealtype,
        is: c_int,
        ySout: N_Vector,
    ) -> c_int;
    /// The evaluations of the sensitivity right hand side.
    pub fn CVodeGetSensNumRhsEvals(cvode_mem: *mut c_void, nfSevals: *mut c_long) -> c_int;
    /// The evaluations of the right hand side of the model the sensitivities took, which is
    /// what the difference quotient of CVODES costs.
    pub fn CVodeGetNumRhsEvalsSens(cvode_mem: *mut c_void, nfevalsS: *mut c_long) -> c_int;
    pub fn CVodeGetSensNumErrTestFails(cvode_mem: *mut c_void, nSetfails: *mut c_long) -> c_int;
    pub fn CVodeGetSensNumLinSolvSetups(cvode_mem: *mut c_void, nlinsetupsS: *mut c_long) -> c_int;
    pub fn CVodeSensFree(cvode_mem: *mut c_void);

    // IDA
    pub fn IDASStolerances(ida_mem: *mut c_void, reltol: sunrealtype, abstol: sunrealtype)
    -> c_int;
    pub fn IDASVtolerances(ida_mem: *mut c_void, reltol: sunrealtype, abstol: N_Vector) -> c_int;
    pub fn IDASetUserData(ida_mem: *mut c_void, user_data: *mut c_void) -> c_int;
    pub fn IDASetMaxNumSteps(ida_mem: *mut c_void, mxsteps: c_long) -> c_int;
    pub fn IDASetMaxStep(ida_mem: *mut c_void, hmax: sunrealtype) -> c_int;
    pub fn IDASetInitStep(ida_mem: *mut c_void, hin: sunrealtype) -> c_int;
    pub fn IDASetStopTime(ida_mem: *mut c_void, tstop: sunrealtype) -> c_int;
    pub fn IDASetLinearSolver(ida_mem: *mut c_void, ls: SUNLinearSolver, a: SUNMatrix) -> c_int;
    pub fn IDAGetRootInfo(ida_mem: *mut c_void, rootsfound: *mut c_int) -> c_int;
    pub fn IDAGetNumSteps(ida_mem: *mut c_void, nsteps: *mut c_long) -> c_int;
    pub fn IDAGetErrWeights(ida_mem: *mut c_void, eweight: N_Vector) -> c_int;
    pub fn IDAGetCurrentStep(ida_mem: *mut c_void, hcur: *mut sunrealtype) -> c_int;
    pub fn IDAGetIntegratorStats(
        ida_mem: *mut c_void,
        nsteps: *mut c_long,
        nfevals: *mut c_long,
        nlinsetups: *mut c_long,
        netfails: *mut c_long,
        qlast: *mut c_int,
        qcur: *mut c_int,
        hinused: *mut sunrealtype,
        hlast: *mut sunrealtype,
        hcur: *mut sunrealtype,
        tcur: *mut sunrealtype,
    ) -> c_int;
    pub fn IDAGetNonlinSolvStats(
        ida_mem: *mut c_void,
        nniters: *mut c_long,
        nncfails: *mut c_long,
    ) -> c_int;
    pub fn IDAFree(ida_mem: *mut *mut c_void);

    pub fn IDACreate(ctx: SUNContext) -> *mut c_void;
    pub fn IDAInit(
        mem: *mut c_void,
        res: IDAResFn,
        t: sunrealtype,
        y: N_Vector,
        yp: N_Vector,
    ) -> c_int;
    pub fn IDAReInit(mem: *mut c_void, t: sunrealtype, y: N_Vector, yp: N_Vector) -> c_int;
    pub fn IDASetMaxErrTestFails(mem: *mut c_void, maxnef: c_int) -> c_int;
    pub fn IDASetSuppressAlg(mem: *mut c_void, suppressalg: sunbooleantype) -> c_int;
    pub fn IDASetId(mem: *mut c_void, id: N_Vector) -> c_int;
    pub fn IDACalcIC(mem: *mut c_void, opt: c_int, tout: sunrealtype) -> c_int;
    pub fn IDAGetConsistentIC(mem: *mut c_void, y: N_Vector, yp: N_Vector) -> c_int;
    pub fn IDASetJacFn(mem: *mut c_void, jac: IDALsJacFn) -> c_int;
    pub fn IDARootInit(mem: *mut c_void, nr: c_int, roots: IDARootFn) -> c_int;
    pub fn IDASolve(
        mem: *mut c_void,
        tout: sunrealtype,
        tret: *mut sunrealtype,
        y: N_Vector,
        yp: N_Vector,
        itask: c_int,
    ) -> c_int;
}

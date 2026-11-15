//! The `extern "C"` declarations of the parts of SUNDIALS 7.9 the engine calls.
//!
//! The signatures are the ones of the headers of SUNDIALS v7.9.0 (`sundials_context.h`,
//! `sundials_nvector.h`, `nvector_serial.h`, `sundials_matrix.h`, `sunmatrix_dense.h`,
//! `sunmatrix_sparse.h`, `sundials_linearsolver.h`, `sunlinsol_dense.h`, `sunlinsol_klu.h`,
//! `sundials_nonlinearsolver.h`, `sunnonlinsol_fixedpoint.h`, `cvode.h`, `cvode_ls.h`) as this
//! crate configures them: `sunrealtype` is `double`,
//! `sunindextype` is `int64_t`, the counters are `long int` (32 bits on wasm32) and `SUNComm` is
//! `int` (no MPI).

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_int, c_long, c_void};

/// The floating point type of SUNDIALS, `double`.
pub type sunrealtype = f64;
/// The index type of SUNDIALS, `int64_t` (`SUNDIALS_INDEX_SIZE=64`).
pub type sunindextype = i64;
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
/// A sparse matrix in compressed sparse column format (`SUN_CSC_MAT`).
pub const CSC_MAT: c_int = 0;
/// A sparse matrix in compressed sparse row format (`SUN_CSR_MAT`).
pub const CSR_MAT: c_int = 1;

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
    /// The weighted root mean square norm `sqrt(sum((x_i * w_i)^2) / n)`.
    pub fn N_VWrmsNorm(x: N_Vector, w: N_Vector) -> sunrealtype;

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
    pub fn SUNLinSolFree(s: SUNLinearSolver) -> SUNErrCode;

    // nonlinear solvers
    /// The fixed point (functional) iteration with `m` acceleration vectors (0 is none).
    pub fn SUNNonlinSol_FixedPoint(y: N_Vector, m: c_int, sunctx: SUNContext)
    -> SUNNonlinearSolver;
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
    pub fn CVode(
        cvode_mem: *mut c_void,
        tout: sunrealtype,
        yout: N_Vector,
        tret: *mut sunrealtype,
        itask: c_int,
    ) -> c_int;
    pub fn CVodeGetRootInfo(cvode_mem: *mut c_void, rootsfound: *mut c_int) -> c_int;
    pub fn CVodeGetDky(cvode_mem: *mut c_void, t: sunrealtype, k: c_int, dky: N_Vector) -> c_int;
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
    pub fn CVodeFree(cvode_mem: *mut *mut c_void);
}

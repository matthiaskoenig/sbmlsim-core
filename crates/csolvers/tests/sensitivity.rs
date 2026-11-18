//! The forward sensitivities of CVODES against the ones of a model which has them in closed
//! form.
//!
//! `y' = -k y` with `y(0) = y0` is `y(t) = y0 exp(-k t)`, so the sensitivities of its two
//! parameters are `dy/dk = -t y0 exp(-k t)` and `dy/dy0 = exp(-k t)`: one of them is a
//! sensitivity of the right hand side and the other one of the initial value, which the
//! initial sensitivity `yS0` carries.
//!
//! All three ways of giving CVODES the sensitivity equations are measured here: its own
//! difference quotient of `df/dp`, which needs nothing but the parameters of the user data
//! (`CVodeSetSensParams`), a right hand side of one sensitivity of the caller
//! (`CVodeSensInit1`) and one of every sensitivity at once (`CVodeSensInit`), which is what a
//! caller with an analytic jacobian wants: one call of it evaluates `df/dy` once and applies
//! it to every sensitivity.
//!
//! The error control of the sensitivities (`CVodeSetSensErrCon`) is off here, which is the
//! default of SUNDIALS, and the tests below say why: at a relative tolerance of 1e-10 the
//! error test of a sensitivity which is a difference quotient cannot pass, because the
//! quotient itself is only accurate to about 1e-8, and the step size collapses (16944 steps
//! against 161 for the same run). The sensitivities are accurate to about 1e-7 without it.

use std::ffi::{c_int, c_long, c_void};
use std::ptr;

use sbmlsim_csolvers::ffi::*;

/// The number of states.
const NEQ: usize = 1;
/// The parameters `k` and `y0`.
const P: [f64; 2] = [0.3, 2.0];
/// The number of sensitivities, the parameters of the fit.
const NS: usize = 2;
/// The tolerances of the states.
const RTOL: f64 = 1e-10;
const ATOL: f64 = 1e-12;
/// The output times.
const TIMES: [f64; 4] = [0.5, 2.0, 5.0, 10.0];

/// How the sensitivity equations are given to CVODES.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sensitivity {
    /// The difference quotient of CVODES, which perturbs the parameters of the user data.
    Internal,
    /// `J s + df/dp` of the caller, one sensitivity per call.
    One,
    /// `J s + df/dp` of the caller, every sensitivity in one call, which is what a model with
    /// an analytic jacobian wants.
    Every,
}

/// The values of a serial vector of `NEQ` values.
///
/// # Safety
/// `v` is a serial vector of `NEQ` values which lives as long as the returned slice.
unsafe fn values<'a>(v: N_Vector) -> &'a mut [f64] {
    unsafe { std::slice::from_raw_parts_mut(N_VGetArrayPointer(v), NEQ) }
}

/// The parameters of the run, which CVODES perturbs in place on its own path.
struct Data {
    p: [f64; 2],
}

extern "C" fn rhs(_t: sunrealtype, y: N_Vector, ydot: N_Vector, data: *mut c_void) -> c_int {
    let (y, ydot) = unsafe { (values(y), values(ydot)) };
    let data = unsafe { &*data.cast::<Data>() };
    ydot[0] = -data.p[0] * y[0];
    0
}

/// `d/dt (dy/dp_i) = J (dy/dp_i) + df/dp_i`, with `J = -k` and `df/dk = -y`, `df/dy0 = 0`.
extern "C" fn sens_rhs1(
    _n: c_int,
    _t: sunrealtype,
    y: N_Vector,
    _ydot: N_Vector,
    which: c_int,
    sensitivity: N_Vector,
    out: N_Vector,
    data: *mut c_void,
    _tmp1: N_Vector,
    _tmp2: N_Vector,
) -> c_int {
    let (y, s, out) = unsafe { (values(y), values(sensitivity), values(out)) };
    let data = unsafe { &*data.cast::<Data>() };
    out[0] = -data.p[0] * s[0];
    if which == 0 {
        out[0] -= y[0];
    }
    0
}

/// Every sensitivity in one call, which is [`CVSensRhsFn`].
extern "C" fn sens_rhs(
    n: c_int,
    t: sunrealtype,
    y: N_Vector,
    ydot: N_Vector,
    sensitivities: *mut N_Vector,
    out: *mut N_Vector,
    data: *mut c_void,
    tmp1: N_Vector,
    tmp2: N_Vector,
) -> c_int {
    let (sensitivities, out) = unsafe {
        (
            std::slice::from_raw_parts(sensitivities, NS),
            std::slice::from_raw_parts(out, NS),
        )
    };
    for which in 0..n {
        let code = sens_rhs1(
            n,
            t,
            y,
            ydot,
            which,
            sensitivities[which as usize],
            out[which as usize],
            data,
            tmp1,
            tmp2,
        );
        if code != 0 {
            return code;
        }
    }
    0
}

/// Checks a return code of SUNDIALS.
fn check(code: c_int, call: &str) {
    assert_eq!(code, CV_SUCCESS, "{call} returned {code}");
}

/// The state and its sensitivities at an output time.
struct Output {
    t: f64,
    y: f64,
    sensitivities: [f64; NS],
}

/// The outputs of a run and what the sensitivities cost.
struct Run {
    outputs: Vec<Output>,
    steps: c_long,
    rhs_evaluations: c_long,
    sensitivity_rhs_evaluations: c_long,
}

/// Integrates `y' = -k y` with the sensitivities of `k` and `y0`.
fn solve(how: Sensitivity) -> Run {
    let mut data = Box::new(Data { p: P });
    unsafe {
        let mut ctx: SUNContext = ptr::null_mut();
        assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut ctx), 0);
        let y = N_VNew_Serial(NEQ as sunindextype, ctx);
        assert!(!y.is_null());
        values(y)[0] = P[1];

        let mem = CVodeCreate(CV_BDF, ctx);
        assert!(!mem.is_null());
        check(CVodeInit(mem, Some(rhs), 0.0, y), "CVodeInit");
        check(CVodeSStolerances(mem, RTOL, ATOL), "CVodeSStolerances");
        check(
            CVodeSetUserData(mem, (&raw mut *data).cast::<c_void>()),
            "CVodeSetUserData",
        );
        let matrix = SUNDenseMatrix(NEQ as sunindextype, NEQ as sunindextype, ctx);
        let solver = SUNLinSol_Dense(y, matrix, ctx);
        assert!(!matrix.is_null() && !solver.is_null());
        check(
            CVodeSetLinearSolver(mem, solver, matrix),
            "CVodeSetLinearSolver",
        );

        // the sensitivities of the parameters: `dy/dk` starts at zero, `dy/dy0` at one
        let sensitivities = N_VCloneVectorArray(NS as c_int, y);
        assert!(!sensitivities.is_null());
        let slots = std::slice::from_raw_parts_mut(sensitivities, NS);
        values(slots[0])[0] = 0.0;
        values(slots[1])[0] = 1.0;
        match how {
            Sensitivity::Every => check(
                CVodeSensInit(
                    mem,
                    NS as c_int,
                    CV_SIMULTANEOUS,
                    Some(sens_rhs),
                    sensitivities,
                ),
                "CVodeSensInit",
            ),
            how => {
                let fs1: CVSensRhs1Fn = match how {
                    Sensitivity::Internal => None,
                    _ => Some(sens_rhs1),
                };
                check(
                    CVodeSensInit1(mem, NS as c_int, CV_SIMULTANEOUS, fs1, sensitivities),
                    "CVodeSensInit1",
                );
            }
        }
        check(CVodeSensEEtolerances(mem), "CVodeSensEEtolerances");
        // the error control of the sensitivities stays off, see the module doc
        check(CVodeSetSensErrCon(mem, 0), "CVodeSetSensErrCon");
        check(CVodeSetMaxNumSteps(mem, 100_000), "CVodeSetMaxNumSteps");
        if how == Sensitivity::Internal {
            // the difference quotient of CVODES perturbs these values in place, so they are
            // the ones the right hand side reads
            let mut scale = P;
            check(
                CVodeSetSensParams(
                    mem,
                    data.p.as_mut_ptr(),
                    scale.as_mut_ptr(),
                    ptr::null_mut(),
                ),
                "CVodeSetSensParams",
            );
        }

        let mut outputs = Vec::with_capacity(TIMES.len());
        for tout in TIMES {
            let mut t = 0.0;
            check(CVode(mem, tout, y, &mut t, CV_NORMAL), "CVode");
            let mut ts = 0.0;
            check(CVodeGetSens(mem, &mut ts, sensitivities), "CVodeGetSens");
            assert_eq!(
                t, ts,
                "the sensitivities are of another time than the state"
            );
            outputs.push(Output {
                t,
                y: values(y)[0],
                sensitivities: [values(slots[0])[0], values(slots[1])[0]],
            });
        }

        let (mut steps, mut rhs_evaluations) = (0, 0);
        let (mut sensitivity_rhs_evaluations, mut rhs_of_sensitivities) = (0, 0);
        check(CVodeGetNumSteps(mem, &mut steps), "CVodeGetNumSteps");
        check(
            CVodeGetNumRhsEvals(mem, &mut rhs_evaluations),
            "CVodeGetNumRhsEvals",
        );
        check(
            CVodeGetSensNumRhsEvals(mem, &mut sensitivity_rhs_evaluations),
            "CVodeGetSensNumRhsEvals",
        );
        check(
            CVodeGetNumRhsEvalsSens(mem, &mut rhs_of_sensitivities),
            "CVodeGetNumRhsEvalsSens",
        );

        CVodeSensFree(mem);
        N_VDestroyVectorArray(sensitivities, NS as c_int);
        let mut mem = mem;
        CVodeFree(&raw mut mem);
        SUNLinSolFree(solver);
        SUNMatDestroy(matrix);
        N_VDestroy(y);
        SUNContext_Free(&raw mut ctx);
        Run {
            outputs,
            steps,
            rhs_evaluations,
            sensitivity_rhs_evaluations: match how {
                // its own difference quotient counts as evaluations of the right hand side
                Sensitivity::Internal => rhs_of_sensitivities,
                _ => sensitivity_rhs_evaluations,
            },
        }
    }
}

/// `y(t)`, `dy/dk` and `dy/dy0` in closed form.
fn analytic(t: f64) -> (f64, [f64; NS]) {
    let (k, y0) = (P[0], P[1]);
    let decay = (-k * t).exp();
    (y0 * decay, [-t * y0 * decay, decay])
}

#[test]
fn the_sensitivities_of_a_decay_are_the_ones_of_its_solution() {
    for how in [Sensitivity::Internal, Sensitivity::One, Sensitivity::Every] {
        let run = solve(how);

        assert_eq!(run.outputs.len(), TIMES.len());
        for output in &run.outputs {
            let (y, sensitivities) = analytic(output.t);
            assert!(
                (output.y / y - 1.0).abs() < 1e-8,
                "{how:?} at {}: y is {} and not {y}",
                output.t,
                output.y
            );
            for (at, (found, wanted)) in output
                .sensitivities
                .iter()
                .zip(sensitivities.iter())
                .enumerate()
            {
                assert!(
                    (found / wanted - 1.0).abs() < 1e-5,
                    "{how:?} at {}: the sensitivity of the parameter {at} is {found} and not \
                     {wanted}",
                    output.t
                );
            }
        }
        assert!(run.steps > 0 && run.rhs_evaluations > 0);
        assert!(
            run.sensitivity_rhs_evaluations > 0,
            "{how:?} evaluated no sensitivity"
        );
    }
}

#[test]
fn the_sensitivities_of_a_caller_cost_no_evaluation_of_the_right_hand_side() {
    let internal = solve(Sensitivity::Internal);
    let analytic = solve(Sensitivity::Every);

    // the difference quotient of CVODES evaluates the right hand side of the model for every
    // parameter and step; a sensitivity right hand side of the caller evaluates none of it
    assert!(
        internal.rhs_evaluations > analytic.rhs_evaluations,
        "the internal difference quotient took {} evaluations and the analytic one {}",
        internal.rhs_evaluations,
        analytic.rhs_evaluations
    );
    // and both walk the same solution, so the steps are of the same order
    assert!(
        analytic.steps <= 2 * internal.steps && internal.steps <= 2 * analytic.steps,
        "{} steps against {}",
        analytic.steps,
        internal.steps
    );
}

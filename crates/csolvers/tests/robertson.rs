//! The Robertson problem with CVODE (BDF, Newton) and the dense and the KLU linear solver.
//!
//! `y1' = -0.04 y1 + 1e4 y2 y3`, `y3' = 3e7 y2^2`, `y2' = -y1' - y3'`, `y(0) = (1, 0, 0)`,
//! integrated to `t = 4e10` with the analytic Jacobian, as the SUNDIALS examples `cvRoberts_dns`
//! and `cvRoberts_klu` do, at tighter tolerances.

use std::ffi::{c_int, c_long, c_void};
use std::ptr;

use sbmlsim_csolvers::ffi::*;

/// The number of states.
const NEQ: usize = 3;
/// The non-zeros of the Jacobian (`J31` and `J33` are structurally zero).
const NNZ: usize = 7;
/// The relative tolerance.
const RTOL: f64 = 1e-8;
/// The absolute tolerances per state.
const ATOL: [f64; NEQ] = [1e-12, 1e-14, 1e-10];
/// The first output time; every next one is ten times the one before, the last is `4e10`.
const T1: f64 = 0.4;
/// The number of output times.
const NOUT: usize = 12;
/// `y1(4e10)` as the SUNDIALS example `cvRoberts_dns` prints it (`5.2083e-08`); its `check_ans`
/// has the reference `5.2083495894337328e-08`.
const Y1_REFERENCE: f64 = 5.2083e-08;

/// The linear solver of a run.
#[derive(Clone, Copy, Debug)]
enum Solver {
    Dense,
    Klu,
}

/// The values of a serial vector.
///
/// # Safety
/// `v` is a serial vector of `NEQ` values which lives as long as the returned slice.
unsafe fn values<'a>(v: N_Vector) -> &'a mut [f64] {
    unsafe { std::slice::from_raw_parts_mut(N_VGetArrayPointer(v), NEQ) }
}

extern "C" fn rhs(_t: sunrealtype, y: N_Vector, ydot: N_Vector, _data: *mut c_void) -> c_int {
    let (y, ydot) = unsafe { (values(y), values(ydot)) };
    let yd1 = -0.04 * y[0] + 1.0e4 * y[1] * y[2];
    let yd3 = 3.0e7 * y[1] * y[1];
    ydot[0] = yd1;
    ydot[1] = -yd1 - yd3;
    ydot[2] = yd3;
    0
}

/// The entries of the Jacobian as `(row, column, value)` in the order of the columns (CSC).
fn jacobian(y: &[f64]) -> [(usize, usize, f64); NNZ] {
    [
        (0, 0, -0.04),
        (1, 0, 0.04),
        (0, 1, 1.0e4 * y[2]),
        (1, 1, -1.0e4 * y[2] - 6.0e7 * y[1]),
        (2, 1, 6.0e7 * y[1]),
        (0, 2, 1.0e4 * y[1]),
        (1, 2, -1.0e4 * y[1]),
    ]
}

extern "C" fn jac_dense(
    _t: sunrealtype,
    y: N_Vector,
    _fy: N_Vector,
    jac: SUNMatrix,
    _data: *mut c_void,
    _tmp1: N_Vector,
    _tmp2: N_Vector,
    _tmp3: N_Vector,
) -> c_int {
    let y = unsafe { values(y) };
    // column-major, `J(i, j)` at `j * NEQ + i`
    let data = unsafe { std::slice::from_raw_parts_mut(SUNDenseMatrix_Data(jac), NEQ * NEQ) };
    data.fill(0.0);
    for (i, j, v) in jacobian(y) {
        data[j * NEQ + i] = v;
    }
    0
}

extern "C" fn jac_sparse(
    _t: sunrealtype,
    y: N_Vector,
    _fy: N_Vector,
    jac: SUNMatrix,
    _data: *mut c_void,
    _tmp1: N_Vector,
    _tmp2: N_Vector,
    _tmp3: N_Vector,
) -> c_int {
    let y = unsafe { values(y) };
    let (data, rows, cols) = unsafe {
        (
            std::slice::from_raw_parts_mut(SUNSparseMatrix_Data(jac), NNZ),
            std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexValues(jac), NNZ),
            std::slice::from_raw_parts_mut(SUNSparseMatrix_IndexPointers(jac), NEQ + 1),
        )
    };
    cols.fill(0);
    for (k, (i, j, v)) in jacobian(y).into_iter().enumerate() {
        data[k] = v;
        rows[k] = i as sunindextype;
        cols[j + 1] = k as sunindextype + 1;
    }
    0
}

/// Checks a return code of SUNDIALS.
fn check(code: c_int, call: &str) {
    assert_eq!(code, CV_SUCCESS, "{call} returned {code}");
}

/// The state at an output time and the number of steps CVODE took to it.
struct Output {
    t: f64,
    y: [f64; NEQ],
    steps: c_long,
}

/// The result of a run: the outputs at `0.4, 4, ..., 4e10`, `y(4e10)` and the counters of CVODE.
struct Run {
    outputs: Vec<Output>,
    y: [f64; NEQ],
    steps: c_long,
    rhs_evaluations: c_long,
}

/// Integrates the Robertson problem to `4e10` with the given linear solver.
fn solve(solver: Solver) -> Run {
    unsafe {
        let mut ctx: SUNContext = ptr::null_mut();
        assert_eq!(SUNContext_Create(SUN_COMM_NULL, &mut ctx), 0);
        let y = N_VNew_Serial(NEQ as sunindextype, ctx);
        let abstol = N_VNew_Serial(NEQ as sunindextype, ctx);
        assert!(!y.is_null() && !abstol.is_null());
        values(y).copy_from_slice(&[1.0, 0.0, 0.0]);
        values(abstol).copy_from_slice(&ATOL);

        let mut mem = CVodeCreate(CV_BDF, ctx);
        assert!(!mem.is_null());
        check(CVodeInit(mem, Some(rhs), 0.0, y), "CVodeInit");
        check(CVodeSVtolerances(mem, RTOL, abstol), "CVodeSVtolerances");
        check(CVodeSetMaxNumSteps(mem, 100_000), "CVodeSetMaxNumSteps");
        let (matrix, linear_solver) = match solver {
            Solver::Dense => {
                let a = SUNDenseMatrix(NEQ as sunindextype, NEQ as sunindextype, ctx);
                (a, SUNLinSol_Dense(y, a, ctx))
            }
            Solver::Klu => {
                let a = SUNSparseMatrix(
                    NEQ as sunindextype,
                    NEQ as sunindextype,
                    NNZ as sunindextype,
                    CSC_MAT,
                    ctx,
                );
                (a, SUNLinSol_KLU(y, a, ctx))
            }
        };
        assert!(!matrix.is_null() && !linear_solver.is_null());
        check(
            CVodeSetLinearSolver(mem, linear_solver, matrix),
            "CVodeSetLinearSolver",
        );
        let jac: CVLsJacFn = match solver {
            Solver::Dense => Some(jac_dense),
            Solver::Klu => Some(jac_sparse),
        };
        check(CVodeSetJacFn(mem, jac), "CVodeSetJacFn");

        let mut outputs = Vec::with_capacity(NOUT);
        let mut tout = T1;
        for _ in 0..NOUT {
            let mut output = Output {
                t: 0.0,
                y: [0.0; NEQ],
                steps: 0,
            };
            check(CVode(mem, tout, y, &mut output.t, CV_NORMAL), "CVode");
            check(CVodeGetNumSteps(mem, &mut output.steps), "CVodeGetNumSteps");
            output.y.copy_from_slice(values(y));
            outputs.push(output);
            tout *= 10.0;
        }
        let result = outputs[NOUT - 1].y;
        assert_eq!(outputs[NOUT - 1].t, 4e10);

        let (mut steps, mut rhs_evaluations, mut setups, mut fails) = (0, 0, 0, 0);
        let (mut qlast, mut qcur) = (0, 0);
        let (mut h0, mut hlast, mut hcur, mut tcur) = (0.0, 0.0, 0.0, 0.0);
        check(
            CVodeGetIntegratorStats(
                mem,
                &mut steps,
                &mut rhs_evaluations,
                &mut setups,
                &mut fails,
                &mut qlast,
                &mut qcur,
                &mut h0,
                &mut hlast,
                &mut hcur,
                &mut tcur,
            ),
            "CVodeGetIntegratorStats",
        );

        CVodeFree(&mut mem);
        check(SUNLinSolFree(linear_solver), "SUNLinSolFree");
        SUNMatDestroy(matrix);
        N_VDestroy(abstol);
        N_VDestroy(y);
        assert_eq!(SUNContext_Free(&mut ctx), 0);
        Run {
            outputs,
            y: result,
            steps,
            rhs_evaluations,
        }
    }
}

/// The relative difference of `a` from `b`.
fn relative(a: f64, b: f64) -> f64 {
    ((a - b) / b).abs()
}

/// The hex dump of `y(4e10)` of both solvers, one line per solver: `HEX <solver> <bits>...`.
fn hex_dump() -> String {
    let mut dump = String::new();
    for solver in [Solver::Dense, Solver::Klu] {
        let run = solve(solver);
        let bits: Vec<String> = run
            .y
            .iter()
            .map(|v| format!("{:016x}", v.to_bits()))
            .collect();
        dump.push_str(&format!("HEX {solver:?} {}\n", bits.join(" ")));
    }
    dump
}

/// The lines of a hex dump of [`hex_dump`] in `output`, from `HEX` on (the test harness may print
/// the name of the test before the first one).
fn dump_lines(output: &str) -> impl Iterator<Item = &str> {
    output
        .lines()
        .filter_map(|line| line.find("HEX ").map(|start| &line[start..]))
}

/// Parses a hex dump of [`hex_dump`] into the values per solver.
fn parse_dump(dump: &str) -> Vec<(String, Vec<f64>)> {
    dump_lines(dump)
        .filter_map(|line| line.strip_prefix("HEX "))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let solver = parts.next().unwrap().to_string();
            let values = parts
                .map(|bits| f64::from_bits(u64::from_str_radix(bits, 16).unwrap()))
                .collect();
            (solver, values)
        })
        .collect()
}

#[test]
fn robertson_dense_and_klu_agree() {
    let dense = solve(Solver::Dense);
    let klu = solve(Solver::Klu);
    for (name, run) in [("dense", &dense), ("KLU", &klu)] {
        println!(
            "{name}: y(4e10) = {:?}, {} steps, {} right hand sides",
            run.y, run.steps, run.rhs_evaluations
        );
    }
    for run in [&dense, &klu] {
        assert!(
            relative(run.y[0], Y1_REFERENCE) < 1e-4,
            "y1(4e10) = {:e}, expected {Y1_REFERENCE:e}",
            run.y[0]
        );
    }
    // While CVODE takes the same steps with both linear solvers, the solutions differ only by
    // the rounding of the factorizations: within 1e-10 relative.
    let mut same_steps = 0;
    for (d, k) in dense.outputs.iter().zip(&klu.outputs) {
        if d.steps != k.steps {
            break;
        }
        same_steps += 1;
        for (yd, yk) in d.y.iter().zip(&k.y) {
            assert!(
                relative(*yk, *yd) < 1e-10,
                "t = {:e}: dense {yd:e} and KLU {yk:e} differ",
                d.t
            );
        }
    }
    println!("the same steps up to output {same_steps} of {NOUT}");
    assert!(
        same_steps >= 2,
        "dense and KLU take other steps from t = 4 on"
    );
    // Later the rounding decides a step size differently and the step sequences part; at
    // `4e10` the solutions agree within the tolerances of the integration, the criterion of
    // `check_ans` of `cvRoberts_dns`: the weighted RMS norm of the difference with the weights
    // `rtol |y| + 10 atol` is below 1.
    let norm = (dense
        .y
        .iter()
        .zip(&klu.y)
        .zip(&ATOL)
        .map(|((d, k), atol)| ((d - k) / (RTOL * d.abs() + 10.0 * atol)).powi(2))
        .sum::<f64>()
        / NEQ as f64)
        .sqrt();
    println!("weighted RMS norm of the difference at 4e10: {norm:e}");
    assert!(norm < 1.0, "dense {:?} and KLU {:?} differ", dense.y, klu.y);
}

/// Prints the hex dump of `y(4e10)`; [`native_and_wasm_bits`] runs it on wasm32-wasip1.
#[test]
fn robertson_hex_dump() {
    let dump = hex_dump();
    print!("{dump}");
    assert_eq!(parse_dump(&dump).len(), 2);
}

/// Native and wasm32-wasip1 give `y(4e10)` within `1e-12` relative.
///
/// Runs [`robertson_hex_dump`] on wasm32-wasip1 with the runner of
/// `CARGO_TARGET_WASM32_WASIP1_RUNNER` (`wasmtime` without), in a target directory of its own,
/// and compares its dump with the native one. The bits may differ in the last places, since the
/// `pow` of glibc and of wasi-libc may round differently; between wasm hosts they are equal.
#[cfg(not(target_family = "wasm"))]
#[test]
fn native_and_wasm_bits() {
    use std::process::Command;

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let runner =
        std::env::var("CARGO_TARGET_WASM32_WASIP1_RUNNER").unwrap_or_else(|_| "wasmtime".into());
    let target_dir = format!(
        "{}/../../target/csolvers-wasm-test",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new(cargo)
        .args([
            "test",
            "-p",
            "sbmlsim-csolvers",
            "--target",
            "wasm32-wasip1",
            "--test",
            "robertson",
            "--target-dir",
            &target_dir,
            "--",
            "robertson_hex_dump",
            "--exact",
            "--nocapture",
        ])
        .env("CARGO_TARGET_WASM32_WASIP1_RUNNER", runner)
        .output()
        .expect("cargo runs");
    let wasm = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "the wasm32-wasip1 run failed (needs WASI_SDK_PATH and wasmtime):\n{wasm}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let native = hex_dump();
    println!("native\n{native}wasm32-wasip1");
    for line in dump_lines(&wasm) {
        println!("{line}");
    }
    let (native, wasm) = (parse_dump(&native), parse_dump(&wasm));
    assert_eq!(
        wasm.len(),
        native.len(),
        "the dumps differ in their solvers"
    );
    let mut largest: f64 = 0.0;
    for ((solver, n), (other, w)) in native.iter().zip(&wasm) {
        assert_eq!(solver, other);
        for (n, w) in n.iter().zip(w) {
            let difference = relative(*w, *n);
            largest = largest.max(difference);
            assert!(
                difference <= 1e-12,
                "{solver}: native {n:e} and wasm {w:e} differ by {difference:e}"
            );
        }
    }
    println!("largest relative difference {largest:e}");
}

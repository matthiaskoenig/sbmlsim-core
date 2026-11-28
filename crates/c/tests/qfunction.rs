//! The QFunction target: the model with a Rosenbrock integrator as a libCEED QFunction, run by
//! the harness of the crate (`run_points`, `apply_points`) as libCEED calls it, against the
//! stepper of the engine on the native model of the C target.
//!
//! The cell model and its 50 parameter sets are the ones of the tests of the C target
//! (`common`). The cases of `testdata/cases` skip with a note when the directory is absent.

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use common::{
    ALGEBRAIC_CASES, Accumulators, Case, EVENT_CASES, MATH, Random, blow_up, cases, cell_model,
    cell_parameter_sets, driven_against_steppers, driven_models, linear, path_integral, uptake,
    with_event,
};
use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_c::{
    BuildOptions, CError, CModel, CONTROL, CONTROL_DONE, CONTROL_FAILED, CONTROL_H, CONTROL_T,
    COUNTS, Library, Points, QSource, ROSENBROCK_ORDER, RosenbrockContext, apply_points, generate,
    generate_qfunction, generate_qfunction_with, generate_with, run_points,
};
use sbmlsim_engine::Stepper;

/// The bound of the states of the QFunction at the tolerances 1e-6 and 1e-9 against the
/// stepper at 1e-12 and 1e-14: `RELATIVE |y| + ABSOLUTE`, a fifth of the `1e-5 |y| + 1e-8` of
/// the plan, of which the largest difference is a tenth.
const RELATIVE: f64 = 2e-6;
const ABSOLUTE: f64 = 2e-9;

/// The QFunction of `sbml` and its library, compiled without libCEED.
fn qfunction(sbml: &str) -> (QSource, Arc<Library>) {
    let q = generate_qfunction(sbml).unwrap();
    let library = Library::build(&q.host_code()).unwrap();
    (q, library)
}

/// A stepper of the cell model per parameter set, at tight tolerances: the reference, and the
/// initial states and parameters of the points.
fn cell_steppers(sets: &[Vec<(String, f64)>]) -> Vec<Stepper> {
    let source = generate(&cell_model()).unwrap();
    let library = Library::build(&source.code).unwrap();
    let model = CModel::new(library, &source.description).unwrap();
    let description = Arc::new(source.description);
    let settings = IntegratorSettings {
        relative_tolerance: 1e-12,
        absolute_tolerance: Tolerance::Scalar(1e-14),
        ..IntegratorSettings::default()
    };
    sets.iter()
        .map(|set| {
            Stepper::new(
                Box::new(model.clone()),
                description.clone(),
                settings.clone(),
                set,
            )
            .unwrap()
        })
        .collect()
}

/// The points of the initial states and parameters of `steppers`.
fn cell_points(q: &QSource, steppers: &[Stepper]) -> Points {
    let states: Vec<f64> = steppers.iter().flat_map(|s| s.state().to_vec()).collect();
    let parameters: Vec<f64> = steppers
        .iter()
        .flat_map(|s| s.parameters().to_vec())
        .collect();
    Points::new(q, &states, &parameters).unwrap()
}

/// The context of the tests to `t1`, with as many attempts per application as a point takes.
fn context(t1: f64) -> RosenbrockContext {
    RosenbrockContext {
        t1,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-10,
        attempts: 1e6,
    }
}

#[test]
fn rosenbrock_matches_stepper() {
    let (q, library) = qfunction(&cell_model());
    assert_eq!(q.state_ids.len(), 7);
    let mut steppers = cell_steppers(&cell_parameter_sets());
    let mut points = cell_points(&q, &steppers);
    let n = q.state_ids.len();
    let mut worst: f64 = 0.0;
    for k in 0..24 {
        let (t0, t1) = (60.0 * k as f64, 60.0 * (k + 1) as f64);
        points.start(t0);
        run_points(&library, &q, &context(t1), &mut points, 8).unwrap();
        for (i, stepper) in steppers.iter_mut().enumerate() {
            let y = stepper.state().to_vec();
            stepper.advance(&y, t0, t1).unwrap();
            let control = &points.control[i * CONTROL..(i + 1) * CONTROL];
            assert_eq!(control[CONTROL_T], t1, "set {i}");
            assert_eq!(control[CONTROL_DONE], 1.0, "set {i}");
            assert_eq!(control[CONTROL_FAILED], 0.0, "set {i}");
            let state = &points.state[i * n..(i + 1) * n];
            for (s, (a, b)) in state.iter().zip(stepper.state()).enumerate() {
                let bound = RELATIVE * b.abs() + ABSOLUTE;
                worst = worst.max((a - b).abs() / bound);
                assert!(
                    (a - b).abs() <= bound,
                    "set {i}, {} at {t1}: QFunction {a}, stepper {b}",
                    q.state_ids[s]
                );
            }
        }
    }
    let steps: f64 = points.counts.chunks(COUNTS).map(|c| c[0]).sum();
    let rejected: f64 = points.counts.chunks(COUNTS).map(|c| c[1]).sum();
    eprintln!(
        "the worst state is {worst}; {:.1} steps and {:.2} rejected \
         per point and 60 min",
        steps / (50.0 * 24.0),
        rejected / (50.0 * 24.0)
    );
}

#[test]
fn order_of_the_method() {
    let (q, library) = qfunction(&linear());
    assert_eq!(q.state_ids, ["x", "y"]);
    assert_eq!(q.parameter_ids, ["k"]);
    // the derivative of the right hand side in the time is the one of the source
    assert!(q.code.contains("#define SBMLSIM_ROSENBROCK_DFDT"));
    let parameters = [1.0];
    // tolerances which accept every step: the step is the one of the control
    let fixed = RosenbrockContext {
        t1: 1.0,
        rtol: 1e30,
        atol: 1e30,
        h_min: 1e-12,
        attempts: 1.0,
    };
    let exact = [
        (-1.0f64).exp(),
        (1.0f64.cos() + 1.0f64.sin()) / 2.0 + 1.5 * (-1.0f64).exp(),
    ];
    let mut errors = Vec::new();
    for steps in [10u32, 20, 40, 80, 160] {
        let h = 1.0 / steps as f64;
        let mut points = Points::new(&q, &[1.0, 2.0], &parameters).unwrap();
        points.start(0.0);
        while points.control[CONTROL_DONE] == 0.0 {
            points.control[CONTROL_H] = h;
            apply_points(&library, &q, &fixed, &mut points, 1).unwrap();
        }
        assert_eq!(points.control[CONTROL_FAILED], 0.0);
        assert_eq!(points.counts, [steps as f64, 0.0]);
        let error = points
            .state
            .iter()
            .zip(&exact)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        errors.push(error);
    }
    let orders: Vec<f64> = errors.windows(2).map(|e| (e[0] / e[1]).log2()).collect();
    eprintln!("errors {errors:?}, observed orders {orders:?}");
    let observed = *orders.last().unwrap();
    assert!(
        (observed - ROSENBROCK_ORDER as f64).abs() <= 0.2,
        "observed order {observed}, errors {errors:?}"
    );
}

/// Runs the points over 24 intervals of 60 min in blocks of `block`, in the order `order`, with
/// `attempts` per application.
fn run_blocked(
    q: &QSource,
    library: &Library,
    initial: &Points,
    order: &[usize],
    block: usize,
    attempts: f64,
) -> Points {
    let n = q.state_ids.len();
    let m = q.parameter_ids.len();
    let pick = |values: &[f64], size: usize| -> Vec<f64> {
        order
            .iter()
            .flat_map(|&i| values[i * size..(i + 1) * size].to_vec())
            .collect()
    };
    let mut points =
        Points::new(q, &pick(&initial.state, n), &pick(&initial.parameters, m)).unwrap();
    for k in 0..24 {
        points.start(60.0 * k as f64);
        let context = RosenbrockContext {
            attempts,
            ..context(60.0 * (k + 1) as f64)
        };
        run_points(library, q, &context, &mut points, block).unwrap();
    }
    // back in the order of `initial`
    let mut sorted = Points::new(q, &initial.state, &initial.parameters).unwrap();
    for (j, &i) in order.iter().enumerate() {
        sorted.state[i * n..(i + 1) * n].copy_from_slice(&points.state[j * n..(j + 1) * n]);
        sorted.control[i * CONTROL..(i + 1) * CONTROL]
            .copy_from_slice(&points.control[j * CONTROL..(j + 1) * CONTROL]);
        sorted.counts[i * COUNTS..(i + 1) * COUNTS]
            .copy_from_slice(&points.counts[j * COUNTS..(j + 1) * COUNTS]);
    }
    sorted
}

#[test]
fn independent_of_blocking() {
    let (q, library) = qfunction(&cell_model());
    // the 50 sets twice, so the blocks of 7 and 64 end in a partial block
    let sets: Vec<Vec<(String, f64)>> = cell_parameter_sets()
        .into_iter()
        .cycle()
        .take(100)
        .collect();
    let initial = cell_points(&q, &cell_steppers(&sets));
    let identity: Vec<usize> = (0..sets.len()).collect();
    let mut shuffled = identity.clone();
    let mut random = Random(11);
    for i in (1..shuffled.len()).rev() {
        let j = (random.uniform() * (i + 1) as f64) as usize;
        shuffled.swap(i, j);
    }
    // one attempt per application, as the most applications
    let reference = run_blocked(&q, &library, &initial, &identity, 1, 1.0);
    assert!(
        reference
            .control
            .chunks(CONTROL)
            .all(|c| c[CONTROL_DONE] == 1.0)
    );
    for (order, block, attempts) in [
        (&identity, 7, 1.0),
        (&identity, 64, 3.0),
        (&shuffled, 7, 1e6),
        (&shuffled, 64, 1e6),
    ] {
        let other = run_blocked(&q, &library, &initial, order, block, attempts);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<u64>>();
        assert_eq!(bits(&other.state), bits(&reference.state), "block {block}");
        assert_eq!(
            bits(&other.control),
            bits(&reference.control),
            "block {block}"
        );
        assert_eq!(
            bits(&other.counts),
            bits(&reference.counts),
            "block {block}"
        );
    }
}

fn refused(sbml: &str) -> String {
    match generate_qfunction(sbml) {
        Err(CError::Unsupported { feature }) => feature,
        other => panic!("{:?}", other.map(|q| q.function)),
    }
}

#[test]
fn refuses_events() {
    let feature = refused(&with_event());
    assert!(feature.contains("events"), "{feature}");
    let Some(cases) = cases() else { return };
    for case in EVENT_CASES {
        let feature = refused(&Case::read(&cases.join(case)).sbml);
        assert!(feature.contains("events"), "{case}: {feature}");
    }
}

#[test]
fn refuses_algebraic_rules_and_delays() {
    let delay = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>x</ci><cn>1</cn></apply></apply></math></rateRule></listOfRules></model></sbml>"#
    );
    let feature = refused(&delay);
    assert!(feature.contains("delay"), "{feature}");
    let Some(cases) = cases() else { return };
    for case in ALGEBRAIC_CASES {
        let feature = refused(&Case::read(&cases.join(case)).sbml);
        assert!(feature.contains("algebraic rules"), "{case}: {feature}");
    }
}

#[test]
fn step_size_underflow_flags_failed() {
    let (q, library) = qfunction(&blow_up());
    assert_eq!(q.state_ids, ["y"]);
    assert_eq!(q.parameter_ids, ["a"]);
    // the first point grows without bound at t = 1, the others end at 1.5
    let rates = [1.0, -1.0, 0.5];
    let mut points = Points::new(&q, &[1.0; 3], &rates).unwrap();
    let context = RosenbrockContext {
        t1: 1.5,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-8,
        attempts: 1.0,
    };
    points.start(0.0);
    let mut failed_at = None;
    for application in 0..100_000 {
        let before = points.clone();
        apply_points(&library, &q, &context, &mut points, 2).unwrap();
        if before.control[CONTROL_FAILED] == 0.0 && points.control[CONTROL_FAILED] == 1.0 {
            // the failed point keeps the state of its last accepted step
            assert_eq!(points.state[0], before.state[0]);
            assert_eq!(points.control[CONTROL_T], before.control[CONTROL_T]);
            failed_at = Some(application);
        }
        let finished = points
            .control
            .chunks(CONTROL)
            .all(|c| c[CONTROL_DONE] == 1.0 || c[CONTROL_FAILED] == 1.0);
        if finished {
            break;
        }
    }
    assert!(failed_at.is_some(), "{:?}", points.control);
    let control = &points.control[..CONTROL];
    assert_eq!(control[CONTROL_DONE], 0.0);
    assert!(control[CONTROL_H] < context.h_min, "{control:?}");
    let t = control[CONTROL_T];
    assert!(t < 1.0 && t > 0.99, "{control:?}");
    assert!(
        points.state[0].is_finite() && points.state[0] > 100.0,
        "{}",
        points.state[0]
    );
    // the others finish
    for (i, a) in rates.iter().enumerate().skip(1) {
        let control = &points.control[i * CONTROL..(i + 1) * CONTROL];
        assert_eq!(control[CONTROL_DONE], 1.0);
        assert_eq!(control[CONTROL_FAILED], 0.0);
        assert_eq!(control[CONTROL_T], 1.5);
        let exact = 1.0 / (1.0 - a * 1.5);
        assert!(
            (points.state[i] - exact).abs() <= 1e-4 * exact.abs(),
            "{a}: {} against {exact}",
            points.state[i]
        );
    }
    // a failed point is copied by every application after
    let before = points.clone();
    run_points(&library, &q, &context, &mut points, 2).unwrap();
    assert_eq!(points, before);
}

/// Compiles `code` with the C compiler of the build, strictly C99 and without a warning, with
/// the include directory `include` when given.
fn compile_strictly(code: &str, include: Option<&Path>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("qfunction.c");
    std::fs::write(&file, code).unwrap();
    let compiler = BuildOptions::from_env().unwrap().compiler;
    let mut words = compiler.split_whitespace();
    let mut command = Command::new(words.next().unwrap());
    command.args(words).args([
        "-std=c99",
        "-pedantic",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-fsyntax-only",
    ]);
    if let Some(include) = include {
        command.arg("-I").arg(include);
    }
    let output = command.arg(&file).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The source of a model with and without a rate of the time compiles without a warning with
/// the definitions of libCEED of `host_code`, and with libCEED's own `<ceed/types.h>` where
/// `SBMLSIM_LIBCEED_INCLUDE` names its include directory (skipped with a note otherwise).
#[test]
fn sources_compile_without_warnings() {
    let sources = [
        generate_qfunction(&cell_model()).unwrap(),
        generate_qfunction(&linear()).unwrap(),
        generate_qfunction(&blow_up()).unwrap(),
    ];
    for q in &sources {
        compile_strictly(&q.host_code(), None);
    }
    let Some(include) = std::env::var_os("SBMLSIM_LIBCEED_INCLUDE") else {
        eprintln!(
            "note: SBMLSIM_LIBCEED_INCLUDE is unset; the compilation with libCEED is skipped"
        );
        return;
    };
    let include = Path::new(&include);
    assert!(
        include.join("ceed/types.h").is_file(),
        "{}",
        include.display()
    );
    for q in &sources {
        compile_strictly(&q.code, Some(include));
    }
}

#[test]
fn harness_refuses_what_does_not_fit() {
    let (q, library) = qfunction(&blow_up());
    let mut points = Points::new(&q, &[1.0, 1.0], &[1.0, -1.0]).unwrap();
    assert!(matches!(
        Points::new(&q, &[1.0, 1.0], &[1.0]),
        Err(CError::Invalid(_))
    ));
    let good = RosenbrockContext {
        t1: 1.0,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-10,
        attempts: 1.0,
    };
    for context in [
        RosenbrockContext {
            t1: f64::NAN,
            ..good
        },
        RosenbrockContext { rtol: 0.0, ..good },
        RosenbrockContext {
            atol: f64::INFINITY,
            ..good
        },
        RosenbrockContext {
            h_min: -1.0,
            ..good
        },
        RosenbrockContext {
            attempts: 0.0,
            ..good
        },
        RosenbrockContext {
            attempts: 2.5,
            ..good
        },
        RosenbrockContext {
            attempts: f64::INFINITY,
            ..good
        },
    ] {
        let error = apply_points(&library, &q, &context, &mut points, 1).unwrap_err();
        assert!(matches!(error, CError::Invalid(_)), "{error}");
    }
    assert!(matches!(
        run_points(&library, &q, &good, &mut points, 0),
        Err(CError::Invalid(_))
    ));
    let mut short = points.clone();
    short.counts.pop();
    assert!(matches!(
        apply_points(&library, &q, &good, &mut short, 1),
        Err(CError::Invalid(_))
    ));
    // the library of another QFunction
    let (other, _) = qfunction(&linear());
    let error = apply_points(&library, &other, &good, &mut points, 1).unwrap_err();
    assert!(
        matches!(error, CError::Load(_) | CError::Model(_)),
        "{error}"
    );
    // nothing above changed the points
    assert_eq!(points, Points::new(&q, &[1.0, 1.0], &[1.0, -1.0]).unwrap());
}

/// What a JIT backend of libCEED gives the source of a QFunction, after `cuda-jit.h`: its
/// macros, the types, and the math functions of the device as functions, but no `<math.h>`
/// (the source guards it, libCEED's loader drops it), so neither `INFINITY` nor `NAN`.
const JIT_PASS: &str = r#"#define __device__
#define CEED_QFUNCTION(name) inline __device__ int name
#define CEED_QFUNCTION_HELPER inline __device__
#define CeedPragmaSIMD
#define CEED_Q_VLA 1
typedef int int32_t;
typedef int32_t CeedInt;
typedef double CeedScalar;
extern "C" {
double fabs(double); double fmax(double, double); double fmin(double, double);
double sqrt(double); double cbrt(double); double pow(double, double); double exp(double);
double log(double); double log10(double); double log2(double); double sin(double);
double cos(double); double tan(double); double asin(double); double acos(double);
double atan(double); double atan2(double, double); double sinh(double); double cosh(double);
double tanh(double); double asinh(double); double acosh(double); double atanh(double);
double floor(double); double ceil(double); double trunc(double); double round(double);
double fmod(double, double); double hypot(double, double); double copysign(double, double);
}
bool isfinite(double);
bool isnan(double);
bool isinf(double);
"#;

/// The source of a model as a JIT backend compiles it: C++17 (as NVRTC and HIPRTC read it)
/// with `CEED_RUNNING_JIT_PASS`, the environment of [`JIT_PASS`] and no include of libCEED's
/// types (which only repeats the definitions there). Skipped with a note without `c++`.
#[test]
fn sources_compile_in_the_jit_pass() {
    let compiler = std::env::var("CXX").unwrap_or_else(|_| "c++".to_string());
    let dir = tempfile::tempdir().unwrap();
    for (name, sbml) in [
        ("cell", cell_model()),
        ("linear", linear()),
        ("blow_up", blow_up()),
    ] {
        let q = generate_qfunction(&sbml).unwrap();
        let code = q.code.replacen("#include <ceed/types.h>\n", "", 1);
        let file = dir.path().join(format!("{name}.cpp"));
        std::fs::write(&file, format!("{JIT_PASS}{code}")).unwrap();
        let mut words = compiler.split_whitespace();
        let output = match Command::new(words.next().unwrap())
            .args(words)
            .args([
                "-x",
                "c++",
                "-std=c++17",
                "-fsyntax-only",
                "-nostdinc",
                "-DCEED_RUNNING_JIT_PASS=1",
            ])
            .arg(&file)
            .output()
        {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("note: no C++ compiler `{compiler}`; the JIT pass is skipped");
                return;
            }
            Err(e) => panic!("{compiler}: {e}"),
        };
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A point whose last step of an interval is shortened to a small part of its step, with an
/// error near 1 (so its factor shrinks the step), goes on in the next interval: the step it
/// stores is the one before the shortening scaled by the factor, not the shortened one, which
/// would fall below `h_min` and fail the point at the start of the next interval.
#[test]
fn a_short_last_step_keeps_the_step() {
    let (q, library) = qfunction(&blow_up());
    // y' = -y^2 from y = 1
    let start = Points::new(&q, &[1.0], &[-1.0]).unwrap();
    let rest = 0.1;
    // the error of one step of `rest` from t = 0, with an absolute tolerance alone: err is the
    // error at atol = 1 over atol, read from the step the point stores after an accepted step
    // of the unclamped factor q = err^(1/4) / 0.9 in (1/6, 5)
    let attempt = |atol: f64, h: f64, t1: f64, h_min: f64| {
        let mut points = start.clone();
        points.control[CONTROL_H] = h;
        let context = RosenbrockContext {
            t1,
            rtol: 1e-300,
            atol,
            h_min,
            attempts: 1.0,
        };
        apply_points(&library, &q, &context, &mut points, 1).unwrap();
        points
    };
    let mut atol = 1.0;
    let error_at_one = loop {
        let points = attempt(atol, rest, 10.0, 1e-10);
        assert_eq!(points.control[CONTROL_T], rest, "atol {atol}");
        let factor = rest / points.control[CONTROL_H];
        if factor <= 1.0 / 6.0 + 1e-12 {
            atol /= 1e3;
            continue;
        }
        assert!(factor < 5.0, "atol {atol}");
        break (0.9 * factor).powi(4) * atol;
    };
    // the tolerance of an error of 0.95, q = 1.097: the last step from the shortened one would
    // be 0.91 rest, below h_min
    let atol = error_at_one / 0.95;
    let h_min = 0.93 * rest;
    let mut points = attempt(atol, 100.0 * rest, rest, h_min);
    assert_eq!(points.counts, [1.0, 0.0]);
    assert_eq!(points.control[CONTROL_DONE], 1.0);
    assert_eq!(points.control[CONTROL_T], rest);
    let stored = points.control[CONTROL_H];
    assert!(stored >= h_min, "{stored} below h_min {h_min}");
    // the next interval at loose tolerances, with the same h_min
    points.start(rest);
    let context = RosenbrockContext {
        t1: rest + 1.0,
        rtol: 1e-3,
        atol: 1e-3,
        h_min,
        attempts: 1e6,
    };
    run_points(&library, &q, &context, &mut points, 1).unwrap();
    assert_eq!(points.control[CONTROL_FAILED], 0.0, "{:?}", points.control);
    assert_eq!(points.control[CONTROL_DONE], 1.0);
    let exact = 1.0 / (1.0 + rest + 1.0);
    assert!(
        (points.state[0] - exact).abs() < 1e-2 * exact,
        "{}",
        points.state[0]
    );
}

/// The QFunction of a driven model against a stepper of its native model ([`generate_with`]) at
/// tight tolerances (`common::driven_against_steppers`), at the tolerances 1e-8 and 1e-11, two
/// orders below those of [`context`], every state, the accumulators included, within
/// `4e-6 |y| + 4e-9`: the QFunction computes the constants of every point in every call from
/// the parameters of the call. At 1e-6 and 1e-9 an accumulator misses that bound relative to
/// itself (by up to 28 times on the cell model, 2.5 times at 1e-7 and 1e-10), since its error is
/// the error of the amounts of the cell it balances, which `driven_accumulators_at_1e_6` bounds.
#[test]
fn driven_matches_stepper() {
    for model in driven_models() {
        let q = generate_qfunction_with(&model.sbml, &model.ids).unwrap();
        let library = Library::build(&q.host_code()).unwrap();
        let source = generate_with(&model.sbml, &model.ids).unwrap();
        assert_eq!(q.description, source.description);
        let worst = driven_against_steppers(
            &model,
            1e-8,
            1e-11,
            Accumulators::Own,
            &mut |points, context| {
                run_points(&library, &q, context, points, 8).unwrap();
            },
        );
        eprintln!("{}: the worst state is {worst}", q.description.id);
    }
}

/// The QFunction of a driven model at the tolerances of production, 1e-6 and 1e-9, against a
/// stepper of its native model at tight tolerances: every state within `4e-6 |y| + 4e-9`, but
/// an accumulator within `4e-6 s + 4e-9` of the sum `s` of the amounts of the cell it balances
/// (`common::Accumulators::Balanced`).
#[test]
fn driven_accumulators_at_1e_6() {
    for model in driven_models() {
        let q = generate_qfunction_with(&model.sbml, &model.ids).unwrap();
        let library = Library::build(&q.host_code()).unwrap();
        let worst = driven_against_steppers(
            &model,
            1e-6,
            1e-9,
            Accumulators::Balanced,
            &mut |points, context| {
                run_points(&library, &q, context, points, 8).unwrap();
            },
        );
        eprintln!("{}: the worst state is {worst}", q.description.id);
    }
}

/// An unknown driven species is refused with the message of the compiler.
#[test]
fn driven_refuses_an_unknown_species() {
    let ids = ["Q".to_string()];
    let error = generate_qfunction_with(&uptake(), &ids).unwrap_err();
    let options = sbmlsim_compiler::CompileOptions {
        driven: ids.to_vec(),
        ..sbmlsim_compiler::CompileOptions::default()
    };
    let compiler = sbmlsim_compiler::compile(&uptake(), &options).unwrap_err();
    assert_eq!(error.to_string(), compiler.to_string());
    assert!(error.to_string().contains('Q'), "{error}");
    let error = generate_with(&uptake(), &ids).unwrap_err();
    assert_eq!(error.to_string(), compiler.to_string());
}

/// `X -> C` of the rate `k1 X` and `C ->` of the rate `k2 C`, both compartments of size 1, with
/// `X` driven: `C' = k1 (value + slope (t - t_ref)) - k2 C`, of the solution
/// `C = A + B t + (C0 - A) exp(-k2 t)` with `B = k1 slope / k2` and
/// `A = (k1 (value - slope t_ref) - B) / k2`, and the exchange `-k1` times the integral of the
/// path.
fn forced_decay() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="forced_decay"><listOfCompartments><compartment id="blood" size="1" constant="true"/><compartment id="cell" size="1" constant="true"/></listOfCompartments><listOfSpecies><species id="X" compartment="blood" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies><listOfParameters><parameter id="k1" value="1" constant="true"/><parameter id="k2" value="1" constant="true"/></listOfParameters><listOfReactions><reaction id="R1" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math {MATH}><apply><times/><ci>k1</ci><ci>X</ci></apply></math></kineticLaw></reaction><reaction id="R2" reversible="false"><listOfReactants><speciesReference species="C" stoichiometry="1" constant="true"/></listOfReactants><kineticLaw><math {MATH}><apply><times/><ci>k2</ci><ci>C</ci></apply></math></kineticLaw></reaction></listOfReactions></model></sbml>"#
    )
}

/// The observed orders of the QFunction of the library on [`forced_decay`] along the path
/// `30 + 50 (t - 0.5)` over `[0, 1]`, at fixed steps of `1/10` to `1/160`, and the errors.
fn driven_orders(q: &QSource, library: &Library) -> (Vec<f64>, Vec<f64>) {
    let (value, slope, t_ref, k1, k2) = (30.0, 50.0, 0.5, 1.0, 1.0);
    let mut parameters: Vec<f64> = q.description.parameters.iter().map(|p| p.initial).collect();
    let at = |id: &str| q.parameter_ids.iter().position(|x| x == id).unwrap();
    let d = &q.description.driven[0];
    parameters[d.value as usize] = value;
    parameters[d.slope as usize] = slope;
    parameters[q.description.t_ref.unwrap() as usize] = t_ref;
    assert_eq!((parameters[at("k1")], parameters[at("k2")]), (k1, k2));
    let b = k1 * slope / k2;
    let a = (k1 * (value - slope * t_ref) - b) / k2;
    let mut exact = vec![0.0; 2];
    exact[q.state_ids.iter().position(|s| s == "C").unwrap()] = a + b + (0.0 - a) * (-k2).exp();
    exact[d.exchange as usize] = -k1 * path_integral(value, slope, t_ref, 0.0, 1.0);
    let fixed = RosenbrockContext {
        t1: 1.0,
        rtol: 1e30,
        atol: 1e30,
        h_min: 1e-12,
        attempts: 1.0,
    };
    let mut errors = Vec::new();
    for steps in [10u32, 20, 40, 80, 160] {
        let h = 1.0 / steps as f64;
        let mut points = Points::new(q, &[0.0, 0.0], &parameters).unwrap();
        points.start(0.0);
        while points.control[CONTROL_DONE] == 0.0 {
            points.control[CONTROL_H] = h;
            apply_points(library, q, &fixed, &mut points, 1).unwrap();
        }
        assert_eq!(points.control[CONTROL_FAILED], 0.0);
        assert_eq!(points.counts, [steps as f64, 0.0]);
        let error = points
            .state
            .iter()
            .zip(&exact)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        errors.push(error);
    }
    let orders = errors.windows(2).map(|e| (e[0] / e[1]).log2()).collect();
    (orders, errors)
}

/// The path of a driven species reads the time, so the QFunction of a driven model has the
/// derivative of its right hand side in the time, and RODAS4 keeps its order 4 on a path of a
/// large slope (as `order_of_the_method`); without that derivative it does not.
#[test]
fn driven_time_derivative() {
    let ids = ["X".to_string()];
    let q = generate_qfunction_with(&forced_decay(), &ids).unwrap();
    assert_eq!(q.state_ids, ["C", "X__exchange"]);
    assert!(q.code.contains("#define SBMLSIM_ROSENBROCK_DFDT"));
    let library = Library::build(&q.host_code()).unwrap();
    let (orders, errors) = driven_orders(&q, &library);
    eprintln!("errors {errors:?}, observed orders {orders:?}");
    let observed = *orders.last().unwrap();
    assert!(
        (observed - ROSENBROCK_ORDER as f64).abs() <= 0.2,
        "observed order {observed}, errors {errors:?}"
    );
    // the same integrator without the derivative in the time loses the order
    let define = format!(
        "#define SBMLSIM_ROSENBROCK_DFDT {}",
        q.function.replace("qfunction", "dfdt")
    );
    assert!(q.code.contains(&define), "{define}");
    let without = q.host_code().replacen(&define, "", 1);
    let library = Library::build(&without).unwrap();
    let (orders, errors) = driven_orders(&q, &library);
    eprintln!("without the derivative in the time: errors {errors:?}, observed orders {orders:?}");
    let observed = *orders.last().unwrap();
    assert!(
        observed < 3.0,
        "observed order {observed}, errors {errors:?}"
    );
}

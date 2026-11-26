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

use common::{ALGEBRAIC_CASES, Case, EVENT_CASES, Random, cases, cell_model, cell_parameter_sets};
use sbmlsim_abi::{IntegratorSettings, Tolerance};
use sbmlsim_c::{
    BuildOptions, CError, CModel, CONTROL, CONTROL_DONE, CONTROL_FAILED, CONTROL_H, CONTROL_T,
    COUNTS, Library, Points, QSource, ROSENBROCK_ORDER, RosenbrockContext, apply_points, generate,
    generate_qfunction, run_points,
};
use sbmlsim_engine::Stepper;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

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

fn context(t1: f64) -> RosenbrockContext {
    RosenbrockContext {
        t1,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-10,
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
        "the largest difference is {worst:.3} of the bound; {:.1} steps and {:.2} rejected \
         per point and 60 min",
        steps / (50.0 * 24.0),
        rejected / (50.0 * 24.0)
    );
}

/// `x' = -x` and `y' = k (cos t - y)` with `k = 1`, `x(0) = 1`, `y(0) = 2`: `x = exp(-t)` and
/// `y = (cos t + sin t) / 2 + 3/2 exp(-t)`, a rate which depends on the time. (Not the
/// Prothero-Robinson problem `y' = -k (y - g(t)) + g'(t)` of a large `k`, on which RODAS4, as
/// every Rosenbrock method of its kind, loses order.)
fn linear() -> String {
    let time = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;
    let rate_y = format!(
        "<apply><times/><ci>k</ci><apply><minus/><apply><cos/>{time}</apply><ci>y</ci></apply></apply>"
    );
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="linear"><listOfParameters><parameter id="k" value="1" constant="true"/><parameter id="x" value="1" constant="false"/><parameter id="y" value="2" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><ci>x</ci></apply></math></rateRule><rateRule variable="y"><math {MATH}>{rate_y}</math></rateRule></listOfRules></model></sbml>"#
    )
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

/// Runs the points over 24 intervals of 60 min in blocks of `block`, in the order `order`.
fn run_blocked(
    q: &QSource,
    library: &Library,
    initial: &Points,
    order: &[usize],
    block: usize,
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
        run_points(
            library,
            q,
            &context(60.0 * (k + 1) as f64),
            &mut points,
            block,
        )
        .unwrap();
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
    let reference = run_blocked(&q, &library, &initial, &identity, 1);
    assert!(
        reference
            .control
            .chunks(CONTROL)
            .all(|c| c[CONTROL_DONE] == 1.0)
    );
    for (order, block) in [
        (&identity, 7),
        (&identity, 64),
        (&shuffled, 7),
        (&shuffled, 64),
    ] {
        let other = run_blocked(&q, &library, &initial, order, block);
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

/// A model of `x` with an event which halves it at `x < 0.5`.
fn with_event() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="event"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><ci>x</ci></apply></math></rateRule></listOfRules><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="true" persistent="true"><math {MATH}><apply><lt/><ci>x</ci><cn>0.5</cn></apply></math></trigger><listOfEventAssignments><eventAssignment variable="x"><math {MATH}><apply><divide/><ci>x</ci><cn>2</cn></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents></model></sbml>"#
    )
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

/// `y' = a y^2` with `y(0) = 1`: `y = 1 / (1 - a t)`, which grows without bound at `t = 1 / a`.
fn blow_up() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="blow_up"><listOfParameters><parameter id="a" value="1" constant="true"/><parameter id="y" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="y"><math {MATH}><apply><times/><ci>a</ci><ci>y</ci><ci>y</ci></apply></math></rateRule></listOfRules></model></sbml>"#
    )
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

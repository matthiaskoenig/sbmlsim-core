//! The batched kernel `kernel/1` of `sbmlsim_codegen::emit_kernel`: a model with its RODAS4
//! integrator in one WebAssembly module, run in wasmtime with the functions of mathlib (natively,
//! the same bits as `mathlib.wasm`) against the QFunction of the C target (`sbmlsim_c`, the same
//! method and step control, compiled with the C compiler of the machine) and the initialization
//! of the engine.
//!
//! The cell model and the small models are the ones of the tests of the QFunction (`common`,
//! shared by its path). The cases of `testdata/cases` skip with a note when the directory is
//! absent.

#[path = "../../c/tests/common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{
    ALGEBRAIC_CASES, Case, EVENT_CASES, MATH, Random, blow_up, cases, cell_model, linear,
    with_event,
};
use sbmlsim_abi::{IntegratorSettings, KERNEL_CONTRACT, KernelDescription, Tolerance};
use sbmlsim_c::{
    CModel, CONTROL, CONTROL_DONE, CONTROL_FAILED, CONTROL_H, CONTROL_T, COUNTS, Library, Points,
    RosenbrockContext, generate, generate_qfunction, run_points,
};
use sbmlsim_codegen::emit_kernel;
use sbmlsim_engine::Stepper;
use sbmlsim_ir::{CompileError, Model};
use wasmparser::{Operator, Parser, Payload, TypeRef};
use wasmtime::{Engine, Linker, Memory, MemoryType, Module, Store, TypedFunc};

/// The model representation of an SBML document.
fn model(sbml: &str) -> Model {
    sbmlsim_ir::build(&sbmlsim_sbml::read(sbml).unwrap()).unwrap()
}

/// The decay model of the tests of the hosts: `y' = -k y`, `y(0) = 1`, `k = 0.5`, and the
/// assignment rule `t_half = ln(2) / k`.
fn decay() -> String {
    std::fs::read_to_string(common::testdata().join("../crates/host/tests/decay.xml")).unwrap()
}

/// The functions of mathlib of one argument, natively.
fn math1(name: &str) -> Option<fn(f64) -> f64> {
    use sbmlsim_mathlib::f;
    Some(match name {
        "math_exp" => f::math_exp,
        "math_log" => f::math_log,
        "math_log10" => f::math_log10,
        "math_sqrt" => f::math_sqrt,
        "math_sin" => f::math_sin,
        "math_cos" => f::math_cos,
        "math_tan" => f::math_tan,
        "math_asin" => f::math_asin,
        "math_acos" => f::math_acos,
        "math_atan" => f::math_atan,
        "math_sinh" => f::math_sinh,
        "math_cosh" => f::math_cosh,
        "math_tanh" => f::math_tanh,
        "math_asinh" => f::math_asinh,
        "math_acosh" => f::math_acosh,
        "math_atanh" => f::math_atanh,
        "math_floor" => f::math_floor,
        "math_ceil" => f::math_ceil,
        "math_factorial" => f::math_factorial,
        _ => return None,
    })
}

/// The functions of mathlib of two arguments, natively.
fn math2(name: &str) -> Option<fn(f64, f64) -> f64> {
    use sbmlsim_mathlib::f;
    Some(match name {
        "math_pow" => f::math_pow,
        "math_fmod" => f::math_fmod,
        _ => return None,
    })
}

/// The size of a page of WebAssembly.
const PAGE: usize = 65536;

/// The points of a kernel in its memory: where each array starts.
struct Layout {
    state: usize,
    parameters: usize,
    control: usize,
    counts: usize,
    context: usize,
    scratch: usize,
    end: usize,
}

/// The arguments of `sbmlsim_kernel_advance`: `n`, then the pointers to the states, the
/// parameters, the controls, the counts, the context and the scratch.
type AdvanceArgs = (i32, i32, i32, i32, i32, i32, i32);

/// A kernel instantiated in wasmtime with a memory of its caller.
struct Kernel {
    description: KernelDescription,
    store: Store<()>,
    memory: Memory,
    init: TypedFunc<(i32, i32, i32), i32>,
    advance: TypedFunc<AdvanceArgs, i32>,
    assign: TypedFunc<(i32, i32, i32, i32), i32>,
}

impl Kernel {
    /// The kernel of an SBML document.
    fn new(sbml: &str) -> Kernel {
        let (wasm, description) = emit_kernel(&model(sbml)).unwrap();
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let memory = Memory::new(&mut store, MemoryType::new(1, None)).unwrap();
        let mut linker = Linker::new(&engine);
        linker.define(&store, "env", "memory", memory).unwrap();
        for import in module.imports() {
            if import.module() != "math" {
                continue;
            }
            let name = import.name();
            if let Some(g) = math1(name) {
                linker.func_wrap("math", name, move |x: f64| g(x)).unwrap();
            } else if let Some(g) = math2(name) {
                linker
                    .func_wrap("math", name, move |x: f64, y: f64| g(x, y))
                    .unwrap();
            } else {
                panic!("the kernel imports math.{name}, which mathlib does not export");
            }
        }
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let init = instance
            .get_typed_func(&mut store, "sbmlsim_kernel_init")
            .unwrap();
        let advance = instance
            .get_typed_func(&mut store, "sbmlsim_kernel_advance")
            .unwrap();
        let assign = instance
            .get_typed_func(&mut store, "sbmlsim_kernel_assign")
            .unwrap();
        Kernel {
            description,
            store,
            memory,
            init,
            advance,
            assign,
        }
    }

    /// The numbers of states and parameters.
    fn sizes(&self) -> (usize, usize) {
        (
            self.description.state_ids.len(),
            self.description.parameter_ids.len(),
        )
    }

    /// Grows the memory to `bytes` at least.
    fn reserve(&mut self, bytes: usize) {
        let have = self.memory.data_size(&self.store);
        if bytes > have {
            let pages = (bytes - have).div_ceil(PAGE);
            self.memory.grow(&mut self.store, pages as u64).unwrap();
        }
    }

    fn write(&mut self, at: usize, values: &[f64]) {
        let data = self.memory.data_mut(&mut self.store);
        for (k, x) in values.iter().enumerate() {
            data[at + 8 * k..at + 8 * k + 8].copy_from_slice(&x.to_le_bytes());
        }
    }

    fn read(&self, at: usize, values: &mut [f64]) {
        let data = self.memory.data(&self.store);
        for (k, x) in values.iter_mut().enumerate() {
            *x = f64::from_le_bytes(data[at + 8 * k..at + 8 * k + 8].try_into().unwrap());
        }
    }

    /// The layout of `n` points, from the byte 64 on.
    fn layout(&self, n: usize) -> Layout {
        let (ns, np) = self.sizes();
        let mut at = 64;
        let mut next = |len: usize| {
            let start = at;
            at += 8 * len;
            start
        };
        let state = next(n * ns);
        let parameters = next(n * np);
        let control = next(n * CONTROL);
        let counts = next(n * COUNTS);
        let context = next(5);
        let scratch = next(n * self.description.scratch_per_point as usize);
        Layout {
            state,
            parameters,
            control,
            counts,
            context,
            scratch,
            end: at,
        }
    }

    /// `sbmlsim_kernel_init` of the parameters of every point (in and out): the states.
    fn init(&mut self, parameters: &mut [f64]) -> Vec<f64> {
        let (ns, np) = self.sizes();
        let n = parameters.len() / np.max(1);
        let (p, y) = (64, 64 + 8 * parameters.len());
        self.reserve(y + 8 * n * ns);
        self.write(p, parameters);
        let status = self
            .init
            .call(&mut self.store, (n as i32, p as i32, y as i32))
            .unwrap();
        assert_eq!(status, 0);
        self.read(p, parameters);
        let mut states = vec![0.0; n * ns];
        self.read(y, &mut states);
        states
    }

    /// Calls `sbmlsim_kernel_advance` once per block `(first, count)` of the points, on the
    /// arrays of the points written once into the memory, and reads them back; the results.
    fn advance_blocks(
        &mut self,
        points: &mut Points,
        context: &RosenbrockContext,
        blocks: &[(usize, usize)],
    ) -> Vec<i32> {
        let (ns, np) = self.sizes();
        let spp = self.description.scratch_per_point as usize;
        let layout = self.layout(points.len());
        self.reserve(layout.end);
        self.write(layout.state, &points.state);
        self.write(layout.parameters, &points.parameters);
        self.write(layout.control, &points.control);
        self.write(layout.counts, &points.counts);
        let RosenbrockContext {
            t1,
            rtol,
            atol,
            h_min,
            attempts,
        } = *context;
        self.write(layout.context, &[t1, rtol, atol, h_min, attempts]);
        let results = blocks
            .iter()
            .map(|&(first, count)| {
                let at = |start: usize, size: usize| (start + 8 * size * first) as i32;
                self.advance
                    .call(
                        &mut self.store,
                        (
                            count as i32,
                            at(layout.state, ns),
                            at(layout.parameters, np),
                            at(layout.control, CONTROL),
                            at(layout.counts, COUNTS),
                            layout.context as i32,
                            at(layout.scratch, spp),
                        ),
                    )
                    .unwrap()
            })
            .collect();
        self.read(layout.state, &mut points.state);
        self.read(layout.parameters, &mut points.parameters);
        self.read(layout.control, &mut points.control);
        self.read(layout.counts, &mut points.counts);
        results
    }

    /// One call of all points; the number of failed points.
    fn advance(&mut self, points: &mut Points, context: &RosenbrockContext) -> i32 {
        let n = points.len();
        self.advance_blocks(points, context, &[(0, n)])[0]
    }

    /// `sbmlsim_kernel_assign` of the points, written into the memory and read back (the
    /// states, the parameters and the controls); its result.
    fn assign(&mut self, points: &mut Points) -> i32 {
        let layout = self.layout(points.len());
        self.reserve(layout.end);
        self.write(layout.state, &points.state);
        self.write(layout.parameters, &points.parameters);
        self.write(layout.control, &points.control);
        let status = self
            .assign
            .call(
                &mut self.store,
                (
                    points.len() as i32,
                    layout.state as i32,
                    layout.parameters as i32,
                    layout.control as i32,
                ),
            )
            .unwrap();
        self.read(layout.state, &mut points.state);
        self.read(layout.parameters, &mut points.parameters);
        self.read(layout.control, &mut points.control);
        status
    }

    /// `sbmlsim_kernel_advance` with raw arguments (for the refusals).
    fn advance_raw(&mut self, args: AdvanceArgs) -> i32 {
        self.advance.call(&mut self.store, args).unwrap()
    }
}

/// The points of states and parameters (point by point) at `t = 0` without a step.
fn points(states: Vec<f64>, parameters: Vec<f64>, n: usize) -> Points {
    Points {
        state: states,
        parameters,
        control: vec![0.0; n * CONTROL],
        counts: vec![0.0; n * COUNTS],
    }
}

/// The context of the tests to `t1`, with as many attempts per call as a point takes.
fn context(t1: f64) -> RosenbrockContext {
    RosenbrockContext {
        t1,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-10,
        attempts: 1e6,
    }
}

/// `count` seeded parameter sets of the cell model: its pattern, the protein and the position
/// of the cell, and, with `substrate`, the concentration of the external substrate (a state),
/// as pre-initialization values.
fn cell_sets(count: usize, seed: u64, substrate: bool) -> Vec<Vec<(String, f64)>> {
    let mut random = Random(seed);
    (0..count)
        .map(|_| {
            let mut set = vec![
                ("pattern".to_string(), 7.0),
                ("protein_ext".to_string(), random.uniform()),
                ("position".to_string(), random.uniform()),
            ];
            if substrate {
                set.push(("[S_ext]".to_string(), random.between(0.0, 5.0)));
            }
            set
        })
        .collect()
}

/// A stepper of the cell model per parameter set (the initialization of the engine).
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

/// The points of the initial states and parameters of the cell model of `sets`.
fn cell_points(sets: &[Vec<(String, f64)>]) -> Points {
    let steppers = cell_steppers(sets);
    let states: Vec<f64> = steppers.iter().flat_map(|s| s.state().to_vec()).collect();
    let parameters: Vec<f64> = steppers
        .iter()
        .flat_map(|s| s.parameters().to_vec())
        .collect();
    points(states, parameters, sets.len())
}

/// The bound of the states of the kernel against the QFunction, both at 1e-6 and 1e-9:
/// `RELATIVE |y| + ABSOLUTE`.
const RELATIVE: f64 = 4e-6;
const ABSOLUTE: f64 = 4e-9;

/// The kernel and the QFunction advance the same points over the same intervals: every point
/// is done (or failed) in both, takes the same accepted and rejected steps in both after every
/// interval, and its states are within the bound.
fn compare(name: &str, sbml: &str, initial: &Points, intervals: &[f64]) {
    let q = generate_qfunction(sbml).unwrap();
    let library = Library::build(&q.host_code()).unwrap();
    let mut kernel = Kernel::new(sbml);
    assert_eq!(kernel.description.state_ids, q.state_ids, "{name}");
    assert_eq!(kernel.description.parameter_ids, q.parameter_ids, "{name}");
    let n = q.state_ids.len();
    let mut reference = initial.clone();
    let mut mine = initial.clone();
    let mut worst: f64 = 0.0;
    for window in intervals.windows(2) {
        let (t0, t1) = (window[0], window[1]);
        reference.start(t0);
        mine.start(t0);
        run_points(&library, &q, &context(t1), &mut reference, 8).unwrap();
        let failed = kernel.advance(&mut mine, &context(t1));
        assert!(mine.finished(), "{name} at {t1}");
        let expected = reference
            .control
            .chunks(CONTROL)
            .filter(|c| c[CONTROL_FAILED] != 0.0)
            .count();
        assert_eq!(failed as usize, expected, "{name} at {t1}");
        for i in 0..mine.len() {
            let (a, b) = (
                &mine.control[i * CONTROL..(i + 1) * CONTROL],
                &reference.control[i * CONTROL..(i + 1) * CONTROL],
            );
            assert_eq!(
                a[CONTROL_DONE], b[CONTROL_DONE],
                "{name}, point {i} at {t1}"
            );
            assert_eq!(a[CONTROL_FAILED], b[CONTROL_FAILED], "{name}, point {i}");
            assert_eq!(a[CONTROL_T], b[CONTROL_T], "{name}, point {i} at {t1}");
            assert_eq!(
                &mine.counts[i * COUNTS..(i + 1) * COUNTS],
                &reference.counts[i * COUNTS..(i + 1) * COUNTS],
                "{name}, the steps of point {i} at {t1}"
            );
            let (ya, yb) = (
                &mine.state[i * n..(i + 1) * n],
                &reference.state[i * n..(i + 1) * n],
            );
            for (s, (a, b)) in ya.iter().zip(yb).enumerate() {
                let bound = RELATIVE * b.abs() + ABSOLUTE;
                worst = worst.max((a - b).abs() / bound);
                assert!(
                    (a - b).abs() <= bound,
                    "{name}, point {i}, {} at {t1}: kernel {a}, QFunction {b}",
                    q.state_ids[s]
                );
            }
        }
    }
    // the points moved
    let steps = |p: &Points| p.counts.chunks(COUNTS).map(|c| c[0]).sum::<f64>();
    assert!(steps(&mine) >= mine.len() as f64, "{name}");
    assert_ne!(mine.state, initial.state, "{name}");
    eprintln!(
        "{name}: the largest difference is {worst:.3e} of the bound, {} steps, the same per point",
        steps(&mine)
    );
}

#[test]
fn kernel_matches_qfunction() {
    // the cell model, 200 seeded parameter sets, 24 intervals of 60 min
    let intervals: Vec<f64> = (0..=24).map(|k| 60.0 * k as f64).collect();
    compare(
        "cell",
        &cell_model(),
        &cell_points(&cell_sets(200, 3, true)),
        &intervals,
    );
    // x' = -x, y' = k (cos t - y), 200 seeded points, 4 intervals of 0.25
    let quarter: Vec<f64> = (0..=4).map(|k| 0.25 * k as f64).collect();
    let mut random = Random(5);
    let mut states = Vec::new();
    let mut parameters = Vec::new();
    for _ in 0..200 {
        states.extend([random.between(0.5, 2.0), random.between(0.0, 3.0)]);
        parameters.push(random.between(0.2, 3.0));
    }
    compare(
        "linear",
        &linear(),
        &points(states, parameters, 200),
        &quarter,
    );
    // y' = a y^2, 200 seeded points which do not blow up before t = 1
    let mut random = Random(9);
    let states = vec![1.0; 200];
    let parameters: Vec<f64> = (0..200).map(|_| random.between(-2.0, 0.5)).collect();
    compare(
        "blow_up",
        &blow_up(),
        &points(states, parameters, 200),
        &quarter,
    );
}

/// The points of `y' = a y^2` from `y = 1` with `a = 1, -1, 0.5` to `t1 = 1.5`, one attempt per
/// call, as the QFunction's `step_size_underflow_flags_failed`: the first point blows up at
/// `t = 1`, its step falls below `h_min` and it fails, and the others finish.
#[test]
fn kernel_step_size_underflow_flags_failed() {
    let sbml = blow_up();
    let mut kernel = Kernel::new(&sbml);
    assert_eq!(kernel.description.state_ids, ["y"]);
    assert_eq!(kernel.description.parameter_ids, ["a"]);
    let rates = [1.0, -1.0, 0.5];
    let initial = points(vec![1.0; 3], rates.to_vec(), 3);
    let context = RosenbrockContext {
        t1: 1.5,
        rtol: 1e-6,
        atol: 1e-9,
        h_min: 1e-8,
        attempts: 1.0,
    };
    let mut mine = initial.clone();
    mine.start(0.0);
    let mut failed_at = None;
    for call in 0..100_000 {
        let before = mine.clone();
        let failed = kernel.advance(&mut mine, &context);
        // the result is the number of the points which are failed
        let flagged = (mine.control.chunks(CONTROL))
            .filter(|c| c[CONTROL_FAILED] == 1.0)
            .count();
        assert_eq!(failed as usize, flagged, "call {call}");
        if before.control[CONTROL_FAILED] == 0.0 && mine.control[CONTROL_FAILED] == 1.0 {
            // the failed point keeps the state and the time of its last accepted step
            assert_eq!(mine.state[0].to_bits(), before.state[0].to_bits());
            assert_eq!(
                mine.control[CONTROL_T].to_bits(),
                before.control[CONTROL_T].to_bits()
            );
            failed_at = Some(call);
        }
        if mine.finished() {
            break;
        }
    }
    assert!(failed_at.is_some(), "{:?}", mine.control);
    let control = &mine.control[..CONTROL];
    assert_eq!(control[CONTROL_DONE], 0.0);
    assert_eq!(control[CONTROL_FAILED], 1.0);
    assert!(control[CONTROL_H] < context.h_min, "{control:?}");
    let t = control[CONTROL_T];
    assert!(t < 1.0 && t > 0.99, "{control:?}");
    assert!(
        mine.state[0].is_finite() && mine.state[0] > 100.0,
        "{}",
        mine.state[0]
    );
    // the others finish at their values
    for (i, a) in rates.iter().enumerate().skip(1) {
        let control = &mine.control[i * CONTROL..(i + 1) * CONTROL];
        assert_eq!(control[CONTROL_DONE], 1.0);
        assert_eq!(control[CONTROL_FAILED], 0.0);
        assert_eq!(control[CONTROL_T], 1.5);
        let exact = 1.0 / (1.0 - a * 1.5);
        assert!(
            (mine.state[i] - exact).abs() <= 1e-4 * exact.abs(),
            "{a}: {} against {exact}",
            mine.state[i]
        );
    }
    // the QFunction fails the same point after the same steps, and finishes the others alike
    let q = generate_qfunction(&sbml).unwrap();
    let library = Library::build(&q.host_code()).unwrap();
    let mut reference = initial.clone();
    reference.start(0.0);
    run_points(&library, &q, &context, &mut reference, 2).unwrap();
    assert_eq!(mine.counts, reference.counts);
    for i in 0..rates.len() {
        let (a, b) = (
            &mine.control[i * CONTROL..(i + 1) * CONTROL],
            &reference.control[i * CONTROL..(i + 1) * CONTROL],
        );
        assert_eq!(a[CONTROL_DONE], b[CONTROL_DONE], "point {i}");
        assert_eq!(a[CONTROL_FAILED], b[CONTROL_FAILED], "point {i}");
        assert!(
            (a[CONTROL_T] - b[CONTROL_T]).abs() <= 1e-12 * b[CONTROL_T],
            "point {i}: {a:?} against {b:?}"
        );
    }
    // a failed point is copied by every call after, and the result stays the count
    let before = mine.clone();
    assert_eq!(kernel.advance(&mut mine, &context), 1);
    assert_eq!(mine, before);
}

/// `x' = -k x` with the assignment rules `r = x t + c` (a state and the time) and
/// `s = r^2 + k^0.75` (another rule and a power of a parameter alone, a constant of the
/// optimizer), `x(0) = x0`. The document declares `s` before `r`, so the rules are evaluated in
/// their order of dependency only where the kernel orders them (`s` read before `r` is stored
/// would be the value of the step before).
fn assigned() -> String {
    let time = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="assigned"><listOfParameters><parameter id="k" value="0.7" constant="true"/><parameter id="c" value="2" constant="true"/><parameter id="x" value="1" constant="false"/><parameter id="r" constant="false"/><parameter id="s" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><times/><apply><minus/><ci>k</ci></apply><ci>x</ci></apply></math></rateRule><assignmentRule variable="s"><math {MATH}><apply><plus/><apply><times/><ci>r</ci><ci>r</ci></apply><apply><power/><ci>k</ci><cn>0.75</cn></apply></apply></math></assignmentRule><assignmentRule variable="r"><math {MATH}><apply><plus/><apply><times/><ci>x</ci>{time}</apply><ci>c</ci></apply></math></assignmentRule></listOfRules></model></sbml>"#
    )
}

/// The values of the targets of the assignment rules of the engine (`model_assign` of the C
/// target, the function the engine calls after a change) at the time, the states and the
/// parameters of a point: the parameters with the targets updated. The constants behind the
/// parameters depend on no target of a rule, so the second call reads those of the first.
fn engine_rules(model: &CModel, n_constants: usize, t: f64, y: &[f64], p: &[f64]) -> Vec<f64> {
    use sbmlsim_engine::ModelFns;
    let mut y = y.to_vec();
    let mut full = p.to_vec();
    full.resize(p.len() + n_constants, 0.0);
    model.assign(t, &mut y, &mut full);
    model.assign(t, &mut y, &mut full);
    full.truncate(p.len());
    full
}

#[test]
fn kernel_assign_matches_engine() {
    let sbml = assigned();
    let mut kernel = Kernel::new(&sbml);
    assert_eq!(kernel.description.state_ids, ["x"]);
    assert_eq!(kernel.description.parameter_ids, ["k", "c", "r", "s"]);
    assert_eq!(kernel.description.assigned_ids, ["r", "s"]);
    let source = generate(&sbml).unwrap();
    let library = Library::build(&source.code).unwrap();
    let engine = CModel::new(library, &source.description).unwrap();
    let n_constants = source.description.n_constants as usize;
    // 20 points of their own k, NaN but for k
    let mut random = Random(11);
    let n = 20;
    let mut parameters = Vec::new();
    for _ in 0..n {
        parameters.extend([random.between(0.1, 2.0), f64::NAN, f64::NAN, f64::NAN]);
    }
    let states = kernel.init(&mut parameters);
    let mut mine = points(states, parameters, n);
    for t1 in [0.5, 1.7, 4.0] {
        let t0 = mine.control[CONTROL_T];
        mine.start(t0);
        assert_eq!(kernel.advance(&mut mine, &context(t1)), 0);
        assert!(mine.finished());
        let before = mine.clone();
        assert_eq!(kernel.assign(&mut mine), 0);
        // only the targets of the rules change
        assert_eq!(mine.state, before.state);
        assert_eq!(mine.control, before.control);
        for i in 0..n {
            let p = &mine.parameters[4 * i..4 * i + 4];
            assert_eq!(&p[..2], &before.parameters[4 * i..4 * i + 2]);
            let x = mine.state[i];
            let t = mine.control[i * CONTROL + CONTROL_T];
            assert_eq!(t, t1);
            let expected = engine_rules(
                &engine,
                n_constants,
                t,
                &[x],
                &before.parameters[4 * i..4 * i + 4],
            );
            for (k, id) in ["r", "s"].iter().enumerate() {
                let (a, b) = (p[2 + k], expected[2 + k]);
                assert!(
                    (a - b).abs() <= 1e-12 * b.abs(),
                    "point {i}, {id} at {t}: kernel {a}, engine {b}"
                );
            }
            // r = x t + c, not the value of init (c)
            assert!((p[2] - (x * t + p[1])).abs() <= 1e-12 * p[2].abs());
            assert_ne!(p[2], p[1]);
        }
    }
}

/// The cell model: `protein`, `SIM_Vmax` and `PEX_Vmax` are rules of the parameters alone, so
/// `assign` after an advance gives them as `init` did, bit for bit, while `necrosis_trigger`
/// reads the state `T` and gets the value of the engine at the time of the point. The other
/// parameters, the states and the controls stay as they are.
#[test]
fn kernel_assign_on_the_cell_model() {
    let sbml = cell_model();
    let mut kernel = Kernel::new(&sbml);
    let mut assigned = kernel.description.assigned_ids.clone();
    assigned.sort();
    assert_eq!(
        assigned,
        ["PEX_Vmax", "SIM_Vmax", "necrosis_trigger", "protein"]
    );
    let ids = kernel.description.parameter_ids.clone();
    let at = |id: &str| ids.iter().position(|p| p == id).unwrap();
    let trigger = at("necrosis_trigger");
    let source = generate(&sbml).unwrap();
    let library = Library::build(&source.code).unwrap();
    let engine = CModel::new(library, &source.description).unwrap();
    let n_constants = source.description.n_constants as usize;
    // the parameters of init from NaN but for the changes of parameters, the states of the
    // engine (with the change of the state [S_ext], which the caller writes after init)
    let sets = cell_sets(20, 7, true);
    let np = ids.len();
    let mut parameters = Vec::new();
    for set in &sets {
        let mut unset = vec![f64::NAN; np];
        for (id, value) in set.iter().filter(|(id, _)| !id.starts_with('[')) {
            unset[at(id)] = *value;
        }
        // a threshold of T below its concentrations and a wide ramp, so that the trigger
        // leaves 0 for the polynomial of its ramp
        unset[at("T_threshold")] = 0.05;
        unset[at("necrosis_w")] = 4.0;
        parameters.extend(unset);
    }
    kernel.init(&mut parameters);
    let states = cell_points(&sets).state;
    let initial = parameters.clone();
    let mut mine = points(states, parameters, sets.len());
    let (mut moved, mut ramp) = (0, 0);
    for t1 in [60.0, 240.0, 1440.0] {
        let t0 = mine.control[CONTROL_T];
        mine.start(t0);
        kernel.advance(&mut mine, &context(t1));
        assert!(mine.finished());
        let before = mine.clone();
        assert_eq!(kernel.assign(&mut mine), 0);
        assert_eq!(bits(&mine.state), bits(&before.state));
        assert_eq!(bits(&mine.control), bits(&before.control));
        let ns = mine.state.len() / sets.len();
        for i in 0..sets.len() {
            let p = &mine.parameters[i * np..(i + 1) * np];
            let expected = engine_rules(
                &engine,
                n_constants,
                t1,
                &mine.state[i * ns..(i + 1) * ns],
                &before.parameters[i * np..(i + 1) * np],
            );
            for (k, (a, b)) in p.iter().zip(&initial[i * np..(i + 1) * np]).enumerate() {
                if k == trigger {
                    let e = expected[k];
                    assert!(
                        (a - e).abs() <= 1e-12 * e.abs(),
                        "point {i} at {t1}: necrosis_trigger kernel {a}, engine {e}"
                    );
                    moved += usize::from(a != b);
                    ramp += usize::from(*a > 0.0 && *a < 1.0);
                } else {
                    assert_eq!(a.to_bits(), b.to_bits(), "point {i} at {t1}: {}", ids[k]);
                }
            }
        }
    }
    assert!(
        moved > 0 && ramp > 0,
        "necrosis_trigger: {moved} moved, {ramp} on the ramp"
    );
    eprintln!("cell: necrosis_trigger moved {moved} times, {ramp} times on the ramp");
}

#[test]
fn kernel_assign_refuses_invalid_arguments() {
    let mut kernel = Kernel::new(&assigned());
    let mut parameters = vec![
        0.7,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        1.2,
        f64::NAN,
        f64::NAN,
        f64::NAN,
    ];
    let states = kernel.init(&mut parameters);
    let mut mine = points(states, parameters, 2);
    mine.start(0.0);
    kernel.advance(&mut mine, &context(1.0));
    // the points in the memory: the arrays, then a call of no point
    let layout = kernel.layout(2);
    assert_eq!(kernel.assign(&mut mine.clone()), 0);
    kernel.write(layout.state, &mine.state);
    kernel.write(layout.parameters, &mine.parameters);
    kernel.write(layout.control, &mine.control);
    let base = (
        2,
        layout.state as i32,
        layout.parameters as i32,
        layout.control as i32,
    );
    let size = kernel.memory.data_size(&kernel.store) as i32;
    for args in [
        (-1, base.1, base.2, base.3),
        // a pointer which is not a multiple of 8
        (base.0, base.1 + 4, base.2, base.3),
        (base.0, base.1, base.2 + 1, base.3),
        (base.0, base.1, base.2, base.3 + 2),
        // arrays past the end of the memory
        (base.0, size - 8, base.2, base.3),
        (base.0, base.1, size - 8, base.3),
        (base.0, base.1, base.2, size - 32),
        (i32::MAX, base.1, base.2, base.3),
    ] {
        assert_eq!(
            kernel.assign.call(&mut kernel.store, args).unwrap(),
            -1,
            "{args:?}"
        );
    }
    // nothing was written
    let mut read = mine.clone();
    kernel.read(layout.state, &mut read.state);
    kernel.read(layout.parameters, &mut read.parameters);
    kernel.read(layout.control, &mut read.control);
    assert_eq!(bits(&read.parameters), bits(&mine.parameters));
    assert_eq!(read.state, mine.state);
    assert_eq!(read.control, mine.control);
    // a valid call of the same arrays assigns
    assert_eq!(kernel.assign.call(&mut kernel.store, base).unwrap(), 0);
    kernel.read(layout.parameters, &mut read.parameters);
    let t = mine.control[CONTROL_T];
    assert!((read.parameters[2] - (mine.state[0] * t + 2.0)).abs() < 1e-12);
    // no point is a valid call which writes nothing
    assert_eq!(
        kernel
            .assign
            .call(&mut kernel.store, (0, base.1, base.2, base.3))
            .unwrap(),
        0
    );
}

/// The bits of the values (NaN compared by its bits).
fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// Whether `a` is within `1e-14` of `b`, relatively (both NaN, or both the same infinity).
fn close(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a == b || (a - b).abs() <= 1e-14 * b.abs()
}

#[test]
fn kernel_init_matches_engine() {
    // the cell model: the initial states of the engine from the parameters of the engine, and
    // the parameters and states of the engine from the pre-initialization values alone (NaN
    // for every other parameter)
    let sets = cell_sets(50, 13, false);
    let steppers = cell_steppers(&sets);
    let mut kernel = Kernel::new(&cell_model());
    let ids = kernel.description.parameter_ids.clone();
    for (i, (set, stepper)) in sets.iter().zip(&steppers).enumerate() {
        let mut given = stepper.parameters().to_vec();
        let states = kernel.init(&mut given);
        for (k, (a, b)) in given.iter().zip(stepper.parameters()).enumerate() {
            assert!(
                close(*a, *b),
                "set {i}, {} is kept: {a}, engine {b}",
                ids[k]
            );
        }
        for (k, (a, b)) in states.iter().zip(stepper.state()).enumerate() {
            assert!(
                close(*a, *b),
                "set {i}, {}: kernel {a}, engine {b}",
                kernel.description.state_ids[k]
            );
        }
        let mut unset = vec![f64::NAN; ids.len()];
        for (id, value) in set {
            unset[ids.iter().position(|p| p == id).unwrap()] = *value;
        }
        let states = kernel.init(&mut unset);
        for (k, (a, b)) in unset.iter().zip(stepper.parameters()).enumerate() {
            assert!(close(*a, *b), "set {i}, {}: kernel {a}, engine {b}", ids[k]);
        }
        for (k, (a, b)) in states.iter().zip(stepper.state()).enumerate() {
            assert!(
                close(*a, *b),
                "set {i}, {} from the changes: kernel {a}, engine {b}",
                kernel.description.state_ids[k]
            );
        }
    }
    // the decay model of NaN parameters: k = 0.5 and t_half = ln(2) / k, y = 1
    let mut kernel = Kernel::new(&decay());
    assert_eq!(kernel.description.parameter_ids, ["k", "t_half"]);
    let mut parameters = vec![f64::NAN, f64::NAN, 2.0, f64::NAN];
    let states = kernel.init(&mut parameters);
    assert_eq!(states, [1.0, 1.0]);
    let ln2 = sbmlsim_mathlib::f::math_log(2.0);
    assert_eq!(parameters, [0.5, ln2 / 0.5, 2.0, ln2 / 2.0]);
}

#[test]
fn kernel_independent_of_block() {
    let initial = cell_points(&cell_sets(60, 17, true));
    let n = initial.len();
    let run = |blocks: &[(usize, usize)], attempts: f64| {
        let mut kernel = Kernel::new(&cell_model());
        let mut points = initial.clone();
        for k in 0..4 {
            points.start(60.0 * k as f64);
            let context = RosenbrockContext {
                attempts,
                ..context(60.0 * (k + 1) as f64)
            };
            while !points.finished() {
                kernel.advance_blocks(&mut points, &context, blocks);
            }
        }
        points
    };
    let all = run(&[(0, n)], 1e6);
    assert!(all.control.chunks(CONTROL).all(|c| c[CONTROL_DONE] == 1.0));
    let single: Vec<(usize, usize)> = (0..n).map(|i| (i, 1)).collect();
    let uneven = [(0, 7), (7, 30), (37, n - 37)];
    for (name, other) in [
        ("one call per point", run(&single, 1e6)),
        (
            "blocks of 7, 30 and 23, one attempt per call",
            run(&uneven, 1.0),
        ),
    ] {
        assert_eq!(bits(&other.state), bits(&all.state), "{name}");
        assert_eq!(bits(&other.control), bits(&all.control), "{name}");
        assert_eq!(bits(&other.counts), bits(&all.counts), "{name}");
    }
}

/// The feature of the refusal of `sbml`.
fn refused(sbml: &str) -> String {
    match emit_kernel(&model(sbml)) {
        Err(CompileError::Unsupported { feature, .. }) => feature,
        other => panic!("{:?}", other.map(|(_, d)| d.version)),
    }
}

#[test]
fn kernel_refuses_events() {
    let feature = refused(&with_event());
    assert!(feature.contains("events"), "{feature}");
    assert!(feature.contains("in a kernel"), "{feature}");
    let delay = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><minus/><apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>x</ci><cn>1</cn></apply></apply></math></rateRule></listOfRules></model></sbml>"#
    );
    let feature = refused(&delay);
    assert!(feature.contains("delay"), "{feature}");
    let stateless = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"><listOfParameters><parameter id="k" value="1" constant="true"/></listOfParameters></model></sbml>"#;
    let feature = refused(stateless);
    assert!(feature.contains("without states"), "{feature}");
    let Some(cases) = cases() else { return };
    for case in EVENT_CASES {
        let feature = refused(&Case::read(&cases.join(case)).sbml);
        assert!(feature.contains("events"), "{case}: {feature}");
    }
    for case in ALGEBRAIC_CASES {
        let feature = refused(&Case::read(&cases.join(case)).sbml);
        assert!(feature.contains("algebraic rules"), "{case}: {feature}");
    }
}

#[test]
fn kernel_module_imports_only_memory_and_math() {
    for sbml in [cell_model(), linear(), blow_up(), decay()] {
        let (wasm, description) = emit_kernel(&model(&sbml)).unwrap();
        wasmparser::validate(&wasm).unwrap();
        assert_eq!(description.version, KERNEL_CONTRACT);
        assert_eq!(description.control, CONTROL as u32);
        assert_eq!(description.counts, COUNTS as u32);
        let q = generate_qfunction(&sbml).unwrap();
        assert_eq!(
            description.model, q.description,
            "the description of the C target"
        );
        let mut memory = 0;
        let mut exports = Vec::new();
        for payload in Parser::new(0).parse_all(&wasm) {
            match payload.unwrap() {
                Payload::ImportSection(section) => {
                    for import in section.into_imports() {
                        let import = import.unwrap();
                        match (import.module, import.ty) {
                            ("env", TypeRef::Memory(ty)) if import.name == "memory" => {
                                assert!(!ty.shared && !ty.memory64 && ty.maximum.is_none());
                                memory += 1;
                            }
                            ("math", TypeRef::Func(_)) => {
                                assert!(
                                    math1(import.name).is_some() || math2(import.name).is_some(),
                                    "math.{}",
                                    import.name
                                );
                            }
                            (module, ty) => panic!("an import {module}.{} {ty:?}", import.name),
                        }
                    }
                }
                Payload::ExportSection(section) => {
                    for export in section {
                        exports.push(export.unwrap().name.to_string());
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    let mut reader = body.get_operators_reader().unwrap();
                    while !reader.eof() {
                        let op = reader.read().unwrap();
                        assert!(!matches!(op, Operator::MemoryGrow { .. }), "memory.grow");
                    }
                }
                Payload::DataSection(_)
                | Payload::DataCountSection { .. }
                | Payload::GlobalSection(_)
                | Payload::MemorySection(_)
                | Payload::TableSection(_)
                | Payload::StartSection { .. }
                | Payload::ElementSection(_) => panic!("a section the kernel has none of"),
                _ => {}
            }
        }
        assert_eq!(memory, 1);
        exports.sort();
        assert_eq!(
            exports,
            [
                "sbmlsim_kernel_advance",
                "sbmlsim_kernel_assign",
                "sbmlsim_kernel_init"
            ]
        );
    }
}

#[test]
fn kernel_refuses_invalid_arguments() {
    let mut kernel = Kernel::new(&blow_up());
    let mut points = points(vec![1.0, 1.0], vec![-1.0, -0.5], 2);
    let valid = context(1.0);
    // the points in the memory, without a call
    kernel.advance_blocks(&mut points, &valid, &[]);
    let layout = kernel.layout(2);
    let base = (
        2,
        layout.state as i32,
        layout.parameters as i32,
        layout.control as i32,
        layout.counts as i32,
        layout.context as i32,
        layout.scratch as i32,
    );
    let values = [
        valid.t1,
        valid.rtol,
        valid.atol,
        valid.h_min,
        valid.attempts,
    ];
    // NaN t1, a tolerance of 0 or infinite, a negative h_min, attempts 0, 2.5 or infinite
    for (k, x) in [
        (0, f64::NAN),
        (1, 0.0),
        (2, f64::INFINITY),
        (3, -1.0),
        (4, 0.0),
        (4, 2.5),
        (4, f64::INFINITY),
    ] {
        let mut context = values;
        context[k] = x;
        kernel.write(layout.context, &context);
        assert_eq!(kernel.advance_raw(base), -1, "context {context:?}");
    }
    kernel.write(layout.context, &values);
    let size = kernel.memory.data_size(&kernel.store) as i32;
    for args in [
        (-1, base.1, base.2, base.3, base.4, base.5, base.6),
        // a pointer which is not a multiple of 8
        (base.0, base.1 + 4, base.2, base.3, base.4, base.5, base.6),
        (base.0, base.1, base.2, base.3, base.4, base.5 + 1, base.6),
        // arrays past the end of the memory
        (base.0, base.1, base.2, base.3, base.4, base.5, size - 8),
        (base.0, size - 8, base.2, base.3, base.4, base.5, base.6),
        (base.0, base.1, base.2, base.3, base.4, size - 32, base.6),
        (i32::MAX, base.1, base.2, base.3, base.4, base.5, base.6),
    ] {
        assert_eq!(kernel.advance_raw(args), -1, "{args:?}");
    }
    for args in [(-1, 64, 64), (1, 60, 64), (1, 64, size)] {
        assert_eq!(
            kernel.init.call(&mut kernel.store, args).unwrap(),
            -1,
            "{args:?}"
        );
    }
    // nothing above changed the points, which advance once the arguments are valid
    let mut read = points.clone();
    kernel.read(layout.state, &mut read.state);
    kernel.read(layout.control, &mut read.control);
    kernel.read(layout.counts, &mut read.counts);
    assert_eq!(read, points);
    assert_eq!(kernel.advance(&mut points, &valid), 0);
    assert!(points.finished());
    for (i, a) in [-1.0f64, -0.5].iter().enumerate() {
        let exact = 1.0 / (1.0 - a);
        assert!(
            (points.state[i] - exact).abs() < 1e-5,
            "{}",
            points.state[i]
        );
    }
}

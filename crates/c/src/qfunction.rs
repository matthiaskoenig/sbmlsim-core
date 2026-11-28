//! The QFunction target: a model with a Rosenbrock integrator as one libCEED QFunction, which
//! advances every point (a cell of a tissue, an element of a mesh) by up to a number of step
//! attempts per application.
//!
//! [`generate_qfunction`] writes one C source: `<ceed/types.h>` and `<math.h>` (guarded by
//! `CEED_RUNNING_JIT_PASS`, as libCEED asks of the source of its JIT backends), the constants,
//! the right hand side, the dense Jacobian and, for a model whose rates depend on the time, the
//! derivative of the right hand side in the time, all from the programs of the emitter of the
//! C target (`emit.rs`) as `CEED_QFUNCTION_HELPER` functions, then the integrator
//! (`include/sbmlsim_rosenbrock.h`, RODAS4, written once for every model) and the QFunction
//! [`QSource::function`] of the interior signature of libCEED:
//!
//! - inputs `state` ([`QSource::state_ids`] per point), `parameters`
//!   ([`QSource::parameter_ids`] per point, without the constants, which the QFunction computes)
//!   `control` ([`CONTROL`] per point: `t`, `h`, `done`, `failed`) and `counts` ([`COUNTS`] per
//!   point: the accepted and the rejected steps so far);
//! - outputs `state_out`, `control_out` and `counts_out` of the same sizes;
//! - the context [`RosenbrockContext`] (`struct SbmlsimRosenbrockContext`), in the time unit of
//!   the model.
//!
//! An application makes step attempts for every point which is neither done nor failed, until it
//! is done or failed but at most [`RosenbrockContext::attempts`], and copies the others, so the
//! sequence of steps of a point depends on nothing but the point: not on the other points, the
//! number of points per call, their order, the backend or the attempts per application. The
//! caller applies it until every point is done or failed, feeding the outputs back as the inputs:
//! with many attempts per application (as on a CPU) usually once, with one attempt (where a call
//! should do bounded work) as often as the point which takes the most attempts. A
//! point is done when its `t` reaches `t1`, failed when its next step would be smaller than
//! `h_min`; it keeps the state of its last accepted step. The step `h` of a point carries over
//! to the next interval (`Points::start` keeps it); a point without one (`h <= 0`) starts with an
//! estimate.
//!
//! The source compiles under libCEED unchanged: the CPU backends call the function the compiled
//! library exports as [`QSource::host_symbol`] (the `CEED_QFUNCTION` itself is static), the JIT
//! backends read the source and its function [`QSource::function`]. [`QSource::host_code`] is
//! the source with the definitions of `<ceed/types.h>` it uses in place of the include, which
//! [`Library::build`] compiles without libCEED; [`run_points`] and [`apply_points`] call that
//! library on arrays as libCEED does (every field `[size][Q]`), for tests and benchmarks.
//!
//! Refused ([`CError::Unsupported`], naming the feature): events, delays, algebraic rules, a
//! model without states and a model without an analytic Jacobian.

use std::ffi::c_void;
use std::fmt::Write as _;
use std::os::raw::c_int;

use sbmlsim_abi::ModelDescription;
use sbmlsim_ir::derive;
use sbmlsim_ir::{Model, Symbol};

use crate::emit::{self, Unit, symbol_prefix};
use crate::{CError, Library};

/// The order of the Rosenbrock method of the QFunction (RODAS4).
pub const ROSENBROCK_ORDER: u32 = 4;

/// The values of the control of a point.
pub const CONTROL: usize = 4;
/// The time of a point in its control.
pub const CONTROL_T: usize = 0;
/// The step a point tries next, in its control (`0` for an estimate).
pub const CONTROL_H: usize = 1;
/// `1.0` where a point has reached `t1`, in its control.
pub const CONTROL_DONE: usize = 2;
/// `1.0` where a point failed, in its control.
pub const CONTROL_FAILED: usize = 3;
/// The values of the counts of a point: its accepted steps and its rejected step attempts.
pub const COUNTS: usize = 2;

/// The fields of the inputs of the QFunction, in their order.
pub const INPUTS: [&str; 4] = ["state", "parameters", "control", "counts"];
/// The fields of the outputs of the QFunction, in their order.
pub const OUTPUTS: [&str; 3] = ["state_out", "control_out", "counts_out"];

/// The most applications [`run_points`] makes before it gives up on points which do not finish.
const MAX_APPLICATIONS: u64 = 100_000_000;

/// The integrator, pasted into every QFunction.
const ROSENBROCK: &str = include_str!("../include/sbmlsim_rosenbrock.h");

/// `<math.h>` outside the JIT pass of libCEED; inside it (where libCEED's loader drops the
/// include, and NVRTC and HIPRTC define no `INFINITY` and `NAN`) the two constants the math of
/// the model writes, as the builtins of GCC, Clang and the CUDA compilers.
const MATH_H: &str = "#ifndef CEED_RUNNING_JIT_PASS
#include <math.h>
#else
#ifndef INFINITY
#define INFINITY __builtin_huge_val()
#endif
#ifndef NAN
#define NAN __builtin_nan(\"\")
#endif
#endif

";

/// The include of libCEED's types, which [`QSource::host_code`] replaces.
const CEED_TYPES: &str = "#include <ceed/types.h>\n";

/// The definitions of `<ceed/types.h>` of libCEED v1.1.2 which the QFunction reads, as libCEED
/// defines them.
const CEED_SHIM: &str = r#"/* The definitions of <ceed/types.h> of libCEED (v1.1.2) which the QFunction reads, as libCEED
   defines them, for a compilation without libCEED. */
#include <stdint.h>
#ifndef CEED_QFUNCTION_ATTR
#ifndef __NO_INLINE__
#if defined(__GNUC__) || defined(__clang__)
#define CEED_QFUNCTION_ATTR __attribute__((flatten))
#elif defined(__INTEL_COMPILER) || defined(__INTEL_LLVM_COMPILER)
#define CEED_QFUNCTION_ATTR _Pragma("forceinline")
#else
#define CEED_QFUNCTION_ATTR
#endif
#else
#define CEED_QFUNCTION_ATTR
#endif
#if defined(__GNUC__) || defined(__clang__)
#define CEED_QFUNCTION_HELPER_ATTR CEED_QFUNCTION_ATTR __attribute__((always_inline))
#else
#define CEED_QFUNCTION_HELPER_ATTR CEED_QFUNCTION_ATTR
#endif
#endif
#ifndef CEED_QFUNCTION
#define CEED_QFUNCTION(name)                                        \
  static const char              name##_loc[] = __FILE__ ":" #name; \
  CEED_QFUNCTION_ATTR static int name
#endif
#ifndef CEED_QFUNCTION_HELPER
#define CEED_QFUNCTION_HELPER CEED_QFUNCTION_HELPER_ATTR static inline
#endif
typedef int32_t CeedInt;
typedef double CeedScalar;
"#;

/// The context of the QFunction (`struct SbmlsimRosenbrockContext`), in the time unit of the
/// model.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RosenbrockContext {
    /// The end of the interval.
    pub t1: f64,
    /// The relative tolerance.
    pub rtol: f64,
    /// The absolute tolerance.
    pub atol: f64,
    /// The smallest step: a point which needs a smaller one fails.
    pub h_min: f64,
    /// The most step attempts of a point in one application: at least 1, a whole number.
    pub attempts: f64,
}

/// The QFunction of a model: its C, the name of its QFunction, the ids of its states and
/// parameters in the order of the fields, and the description of the model (the one of
/// [`crate::generate`]).
#[derive(Debug, Clone, PartialEq)]
pub struct QSource {
    /// The C, a source for libCEED, see the module documentation.
    pub code: String,
    /// The name of the `CEED_QFUNCTION`: `sbmlsim_model_<id>_qfunction`.
    pub function: String,
    /// The states, in the order of the field `state`.
    pub state_ids: Vec<String>,
    /// The parameters, in the order of the field `parameters`.
    pub parameter_ids: Vec<String>,
    /// The description of the model.
    pub description: ModelDescription,
}

impl QSource {
    /// The C with the definitions of `<ceed/types.h>` the QFunction reads in place of the
    /// include, as libCEED defines them, for [`Library::build`] without libCEED.
    pub fn host_code(&self) -> String {
        self.code.replacen(CEED_TYPES, CEED_SHIM, 1)
    }

    /// The function a compiled library exports for its callers, the QFunction:
    /// `<function>_host`, of the signature of `CeedQFunctionUser`.
    pub fn host_symbol(&self) -> String {
        format!("{}_host", self.function)
    }

    /// The function a compiled library exports with the sizes of its fields: `<function>_sizes`,
    /// which writes the numbers of the states, the parameters, the control and the counts.
    fn sizes_symbol(&self) -> String {
        format!("{}_sizes", self.function)
    }
}

/// The QFunction of the SBML document `sbml`; refuses a model the QFunction target does not
/// support ([`CError::Unsupported`]) and a model the reader or the model representation
/// refuses ([`CError::Model`]).
pub fn generate_qfunction(sbml: &str) -> Result<QSource, CError> {
    generate_qfunction_with(sbml, &[])
}

/// [`generate_qfunction`] with the species `driven` (SBML ids, in this order) driven
/// ([`sbmlsim_ir::driven::apply`], see [`crate::generate_with`]): the field `parameters` carries
/// the paths of the species per point and call (`X__value`, `X__slope`, `driven__t_ref`, at the
/// indices of `ModelDescription::driven` and `t_ref`), from which every application computes the
/// constants of the point, and the field `state` the accumulators `X__exchange`. The paths read
/// the time, so the QFunction has the derivative of the right hand side in the time.
pub fn generate_qfunction_with(sbml: &str, driven: &[String]) -> Result<QSource, CError> {
    generate_qfunction_model(&emit::model_of(sbml, driven)?)
}

/// The QFunction of a model representation (see [`generate_qfunction`]).
pub fn generate_qfunction_model(model: &Model) -> Result<QSource, CError> {
    let unsupported = |feature: &str| CError::Unsupported {
        feature: format!(
            "{feature} in a QFunction, which integrates ordinary differential equations without \
             events, delays and algebraic rules (the engine integrates such a model)"
        ),
    };
    if !model.events.is_empty() {
        return Err(unsupported("a model with events"));
    }
    if model.n_algebraic > 0 {
        return Err(unsupported("a model with algebraic rules"));
    }
    if model.states.is_empty() {
        return Err(unsupported("a model without states"));
    }
    let (plan, description) = emit::prepare(model)?;
    let dfdt = derive::time_derivatives(model).map_err(|op| CError::Unsupported {
        feature: format!(
            "a model whose rates depend on the time through an operator without a derivative \
             ({op:?}) in a QFunction"
        ),
    })?;
    let prefix = symbol_prefix(&description.id);
    let function = format!("{prefix}qfunction");
    let n = model.states.len();
    let n_parameters = model.parameters.len();
    let mut unit = Unit {
        code: String::new(),
        prefix,
        n_params: n_parameters as u32,
        qualifier: "CEED_QFUNCTION_HELPER ",
    };
    writeln!(
        unit.code,
        "/* The QFunction `{function}` of the model `{}` of sbmlsim-core (sbmlsim-c {}): the model \
         with the Rosenbrock integrator RODAS4, step attempts per point. */",
        description.id.replace("*/", "* /"),
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap();
    unit.code.push_str(CEED_TYPES);
    unit.code.push_str(MATH_H);
    unit.code.push_str(&emit::math("CEED_QFUNCTION_HELPER"));
    unit.code.push('\n');

    // constants(p): the constants behind the parameters
    unit.function("void", "constants", "double *p", |b| {
        b.line("(void)p;");
        b.program(&plan.constants, |b, k, e| {
            let lvalue = b.address(Symbol::Constant(k));
            b.store(&lvalue, e);
        });
    });
    const VECTORS: &str = "double t, const double *y, const double *p, double *out";
    // the arguments a function of a model may not read
    const UNUSED: &str = "(void)t;\n    (void)y;\n    (void)p;";
    unit.function("void", "rhs", VECTORS, |b| {
        b.line(UNUSED);
        b.program(&plan.rhs, |b, k, e| b.store(&format!("out[{k}]"), e));
    });
    // jac(t, y, p, out): dense, row major, from the entries of the pattern in its CSC order
    let pattern = description.jacobian.pattern();
    let mut dense = Vec::with_capacity(pattern.row_idx.len());
    for column in 0..n {
        let entries = pattern.col_ptr[column] as usize..pattern.col_ptr[column + 1] as usize;
        for k in entries {
            dense.push(pattern.row_idx[k] as usize * n + column);
        }
    }
    let jac = plan.jac.as_ref().expect("an analytic Jacobian");
    unit.function("void", "jac", VECTORS, |b| {
        b.line(UNUSED);
        b.line(&format!(
            "for (int k = 0; k < {}; k++) out[k] = 0.0;",
            n * n
        ));
        b.program(jac, |b, k, e| {
            b.store(&format!("out[{}]", dense[k as usize]), e)
        });
    });
    if let Some(dfdt) = &dfdt {
        unit.function("void", "dfdt", VECTORS, |b| {
            b.line(UNUSED);
            b.assign_locals(model);
            for (j, d) in &dfdt.rules {
                b.let_(Symbol::Derivative(*j), d);
            }
            for (k, d) in dfdt.rates.iter().enumerate() {
                match d {
                    Some(d) => b.store(&format!("out[{k}]"), d),
                    None => b.line(&format!("out[{k}] = 0.0;")),
                }
            }
        });
    }

    let prefix = &unit.prefix;
    let memory = (n_parameters + description.n_constants as usize).max(1);
    let mut macros = format!(
        "#define SBMLSIM_ROSENBROCK_N {n}\n\
         #define SBMLSIM_ROSENBROCK_N_PARAMETERS {n_parameters}\n\
         #define SBMLSIM_ROSENBROCK_N_MEMORY {memory}\n\
         #define SBMLSIM_ROSENBROCK_CONSTANTS {prefix}constants\n\
         #define SBMLSIM_ROSENBROCK_RHS {prefix}rhs\n\
         #define SBMLSIM_ROSENBROCK_JAC {prefix}jac\n"
    );
    if dfdt.is_some() {
        writeln!(macros, "#define SBMLSIM_ROSENBROCK_DFDT {prefix}dfdt").unwrap();
    }
    unit.code.push_str(&macros);
    unit.code.push('\n');
    unit.code.push_str(ROSENBROCK);
    write!(
        unit.code,
        r#"
CEED_QFUNCTION({function})(void *ctx, CeedInt Q, const CeedScalar *const *in, CeedScalar *const *out) {{
    return sbmlsim_rosenbrock_points(ctx, Q, in, out);
}}

#ifndef CEED_RUNNING_JIT_PASS
/* The QFunction for a caller which loads the compiled library (the CPU backends of libCEED, the
   harness of sbmlsim-c), and the sizes of its fields: states, parameters, control, counts. */
int {function}_host(void *ctx, CeedInt Q, const CeedScalar *const *in, CeedScalar *const *out) {{
    (void){function}_loc;
    return {function}(ctx, Q, in, out);
}}

void {function}_sizes(CeedInt *out) {{
    out[0] = {n};
    out[1] = {n_parameters};
    out[2] = {CONTROL};
    out[3] = {COUNTS};
}}
#endif
"#
    )
    .unwrap();

    Ok(QSource {
        code: unit.code,
        function,
        state_ids: description.states.iter().map(|e| e.id.clone()).collect(),
        parameter_ids: description
            .parameters
            .iter()
            .map(|e| e.id.clone())
            .collect(),
        description,
    })
}

/// The values of the points of a QFunction, every field point by point (`[point][size]`): the
/// layout of a caller, which [`apply_points`] reads into the `[size][Q]` of libCEED per call.
#[derive(Debug, Clone, PartialEq)]
pub struct Points {
    /// The states, [`QSource::state_ids`] per point.
    pub state: Vec<f64>,
    /// The parameters, [`QSource::parameter_ids`] per point.
    pub parameters: Vec<f64>,
    /// The control, [`CONTROL`] per point.
    pub control: Vec<f64>,
    /// The counts, [`COUNTS`] per point.
    pub counts: Vec<f64>,
}

impl Points {
    /// The points of the states and the parameters (point by point) of the QFunction `q`, at
    /// `t = 0` without a step (an estimate), not done, not failed, without steps.
    pub fn new(q: &QSource, states: &[f64], parameters: &[f64]) -> Result<Points, CError> {
        let (n, m) = (q.state_ids.len(), q.parameter_ids.len());
        let count = states.len() / n;
        if states.len() != count * n || parameters.len() != count * m {
            return Err(CError::Invalid(format!(
                "{} states and {} parameters are no points of {n} states and {m} parameters",
                states.len(),
                parameters.len()
            )));
        }
        Ok(Points {
            state: states.to_vec(),
            parameters: parameters.to_vec(),
            control: vec![0.0; count * CONTROL],
            counts: vec![0.0; count * COUNTS],
        })
    }

    /// The number of points.
    pub fn len(&self) -> usize {
        self.control.len() / CONTROL
    }

    /// Whether there are no points.
    pub fn is_empty(&self) -> bool {
        self.control.is_empty()
    }

    /// Starts an interval at `t0`: every point which has not failed is at `t0` and not done; its
    /// step is kept.
    pub fn start(&mut self, t0: f64) {
        for control in self.control.chunks_mut(CONTROL) {
            if control[CONTROL_FAILED] == 0.0 {
                control[CONTROL_T] = t0;
                control[CONTROL_DONE] = 0.0;
            }
        }
    }

    /// Whether every point is done or failed.
    pub fn finished(&self) -> bool {
        self.control
            .chunks(CONTROL)
            .all(|c| c[CONTROL_DONE] != 0.0 || c[CONTROL_FAILED] != 0.0)
    }
}

/// The QFunction of a library.
type QFunction =
    unsafe extern "C" fn(*mut c_void, i32, *const *const f64, *const *mut f64) -> c_int;
type Sizes = unsafe extern "C" fn(*mut i32);

/// The QFunction of a library with the buffers of its fields for blocks of `block` points.
struct Harness {
    function: QFunction,
    sizes: [usize; 4],
    block: usize,
    inputs: [Vec<f64>; 4],
    outputs: [Vec<f64>; 3],
}

impl Harness {
    /// The QFunction of `library`, checked against `q`, the context and the points.
    fn new(
        library: &Library,
        q: &QSource,
        context: &RosenbrockContext,
        points: &Points,
        block: usize,
    ) -> Result<Harness, CError> {
        // SAFETY: the types are the signatures `generate_qfunction` writes for these names; the
        // functions are called while the caller holds the library.
        let (function, sizes): (QFunction, Sizes) = unsafe {
            (
                library.symbol(&q.host_symbol())?,
                library.symbol(&q.sizes_symbol())?,
            )
        };
        let mut words = [0i32; 4];
        // SAFETY: `sizes` writes four words.
        unsafe { sizes(words.as_mut_ptr()) };
        let sizes = words.map(|w| w as usize);
        let expected = [q.state_ids.len(), q.parameter_ids.len(), CONTROL, COUNTS];
        if sizes != expected {
            return Err(CError::Model(format!(
                "the library {} is not the one of the QFunction {}: its sizes are {sizes:?}, \
                 those of the QFunction {expected:?}",
                library.path().display(),
                q.function
            )));
        }
        let count = points.len();
        let lengths = [
            points.state.len(),
            points.parameters.len(),
            points.control.len(),
            points.counts.len(),
        ];
        if lengths != sizes.map(|s| s * count) {
            return Err(CError::Invalid(format!(
                "the fields of {count} points have the lengths {lengths:?}, not those of the sizes \
                 {sizes:?}"
            )));
        }
        let RosenbrockContext {
            t1,
            rtol,
            atol,
            h_min,
            attempts,
        } = *context;
        let positive = |x: f64| x.is_finite() && x > 0.0;
        if !t1.is_finite()
            || ![rtol, atol, h_min].into_iter().all(positive)
            || !(attempts >= 1.0 && attempts.is_finite() && attempts.fract() == 0.0)
        {
            return Err(CError::Invalid(format!(
                "the context {context:?} needs a finite t1, finite positive tolerances and h_min, \
                 and a whole number of attempts of at least 1"
            )));
        }
        if block == 0 {
            return Err(CError::Invalid("a block of no points".to_string()));
        }
        let buffer = |size: usize| vec![0.0; size * block];
        Ok(Harness {
            function,
            sizes,
            block,
            inputs: sizes.map(buffer),
            outputs: [buffer(sizes[0]), buffer(sizes[2]), buffer(sizes[3])],
        })
    }

    /// One application to every point, in blocks.
    fn apply(&mut self, context: &RosenbrockContext, points: &mut Points) -> Result<(), CError> {
        let count = points.len();
        let mut first = 0;
        while first < count {
            let q = self.block.min(count - first);
            let fields: [&[f64]; 4] = [
                &points.state,
                &points.parameters,
                &points.control,
                &points.counts,
            ];
            // every field [size][Q], as libCEED gives it to a QFunction
            for ((input, field), &size) in self.inputs.iter_mut().zip(fields).zip(&self.sizes) {
                for i in 0..q {
                    for k in 0..size {
                        input[k * q + i] = field[(first + i) * size + k];
                    }
                }
            }
            let inputs: [*const f64; 4] = self.inputs.each_ref().map(|v| v.as_ptr());
            let outputs: [*mut f64; 3] = self.outputs.each_mut().map(|v| v.as_mut_ptr());
            // SAFETY: every buffer holds the `size * q` values of its field (the sizes are the
            // ones of the library), and the context is the struct of the C.
            let status = unsafe {
                (self.function)(
                    context as *const RosenbrockContext as *mut c_void,
                    q as i32,
                    inputs.as_ptr(),
                    outputs.as_ptr(),
                )
            };
            if status != 0 {
                return Err(CError::Model(format!("the QFunction returned {status}")));
            }
            let fields: [(&mut Vec<f64>, usize); 3] = [
                (&mut points.state, self.sizes[0]),
                (&mut points.control, self.sizes[2]),
                (&mut points.counts, self.sizes[3]),
            ];
            for (output, (field, size)) in self.outputs.iter().zip(fields) {
                for i in 0..q {
                    for k in 0..size {
                        field[(first + i) * size + k] = output[k * q + i];
                    }
                }
            }
            first += q;
        }
        Ok(())
    }
}

/// One application of the QFunction `q` of `library` (the library of [`QSource::host_code`]) to
/// every point, in calls of `block` points (the last one fewer) as libCEED makes them: step
/// attempts for every point which is neither done nor failed, until it is, at most
/// `context.attempts`. The outputs are the points afterwards.
pub fn apply_points(
    library: &Library,
    q: &QSource,
    context: &RosenbrockContext,
    points: &mut Points,
    block: usize,
) -> Result<(), CError> {
    Harness::new(library, q, context, points, block)?.apply(context, points)
}

/// Applies the QFunction (see [`apply_points`]) until every point is done or failed; the number
/// of applications.
pub fn run_points(
    library: &Library,
    q: &QSource,
    context: &RosenbrockContext,
    points: &mut Points,
    block: usize,
) -> Result<u64, CError> {
    let mut harness = Harness::new(library, q, context, points, block)?;
    let mut applications = 0;
    while !points.finished() {
        if applications == MAX_APPLICATIONS {
            return Err(CError::Invalid(format!(
                "the points did not finish in {MAX_APPLICATIONS} applications"
            )));
        }
        harness.apply(context, points)?;
        applications += 1;
    }
    Ok(applications)
}

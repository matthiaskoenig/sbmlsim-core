//! The batched kernel (`kernel/2`, see `sbmlsim_abi`): a model of ordinary differential
//! equations with the Rosenbrock integrator RODAS4 in one WebAssembly module, which advances any
//! number of points per call.
//!
//! A model with driven species (`sbmlsim_ir::driven::apply` before the emission) is an ordinary
//! model whose rates read the time and the parameters of the paths, so the kernel needs nothing
//! of its own for them: `advance` copies the parameters of every point into its work and computes
//! the constants behind them in every call, before its first attempt, so a path written into the
//! parameters between the calls is the one the attempts read; `init` evaluates the initial values
//! of each point from its parameters and `assign` the rules as written. The description lists
//! the driven species and the reference time in the description of its model
//! (`ModelDescription::driven`, `t_ref`).
//!
//! [`emit_kernel`] emits the functions of the model from the programs of the optimizer
//! (`sbmlsim_ir::optimize`, as the C target and its QFunction take them): the constants behind
//! the parameters, the right hand side, the dense Jacobian and, for a model whose rates depend
//! on the time, their derivative in the time, and the initialization of a point. They are the
//! functions of `model.wasm` (the same statements, split the same way above the limits), but not
//! exported. The integrator is written here once in WebAssembly, with the sizes of the model as
//! constants: the one of `sbmlsim_rosenbrock.h` of the C target, operation for operation (the
//! coefficients of `rodas.f`, the step control, the first step of Hairer and Wanner, the dense LU
//! decomposition with partial pivoting), with `fmin` and `fmax` of the C library (NaN is the
//! missing argument) and `pow` of mathlib. A point agrees with the QFunction within the
//! tolerances of the integration; its math functions are those of mathlib, the C library's
//! differ in the last bits.
//!
//! Every array is the caller's: the work of a point (the parameters and the constants, the rates
//! of the first stage, the matrix, the stages, the pivots) is `scratch_per_point` values of
//! `scratch`, so the module has no data segment, no global and never grows the memory.

use std::collections::HashSet;

use sbmlsim_abi::{KERNEL_ADVANCE, KERNEL_ASSIGN, KERNEL_CONTRACT, KERNEL_INIT, KernelDescription};
use sbmlsim_ir::derive::{self, JacobianIr};
use sbmlsim_ir::optimize;
use sbmlsim_ir::{CompileError, Expr, Model, Symbol};
use wasm_encoder::ValType::{F64, I32};
use wasm_encoder::{BlockType, Ieee64, InstructionSink, MemArg, MemArgTarget, ValType};

use crate::split::{Guard, Signature, Stmt};
use crate::wasm::{self, Ctx, MathFn, ModuleBuilder, RawCode, call_defined};
use crate::{VECTORS, description, program_stmts, rule_lets};

/// The values of the control of a point: `t`, `h`, `done`, `failed`.
const CONTROL: u32 = 4;
/// The values of the counts of a point: the accepted steps and the rejected step attempts.
const COUNTS: u32 = 2;
/// The values of the context: `t1`, `rtol`, `atol`, `h_min`, `attempts`.
const CONTEXT: u32 = 5;

/// The refusal of a model the kernel does not integrate.
fn unsupported(model: &Model, feature: &str) -> CompileError {
    CompileError::Unsupported {
        feature: format!(
            "{feature} in a kernel, which integrates ordinary differential equations without \
             events, delays and algebraic rules (the engine integrates such a model)"
        ),
        element: model.id.clone().unwrap_or_default(),
    }
}

/// Emits the batched kernel of a model (`kernel/2`) and its description; a model with driven
/// species gives a driven kernel (see the module documentation).
///
/// Refuses ([`CompileError::Unsupported`], naming the feature, as the QFunction of the C target
/// does): events, delays, algebraic rules, a model without states, a model without an analytic
/// Jacobian, and a model whose rates depend on the time through an operator without a
/// derivative; and, as [`crate::emit`] does, a statement above the limits of WebAssembly
/// engines.
pub fn emit_kernel(model: &Model) -> Result<(Vec<u8>, KernelDescription), CompileError> {
    if !model.events.is_empty() {
        return Err(unsupported(model, "a model with events"));
    }
    if model.n_algebraic > 0 {
        return Err(unsupported(model, "a model with algebraic rules"));
    }
    if model.states.is_empty() {
        return Err(unsupported(model, "a model without states"));
    }
    let discontinuities = model.try_discontinuity_roots()?;
    if !wasm::delay_sites(model, &discontinuities).is_empty() {
        return Err(unsupported(
            model,
            "a model which reads its history (delay)",
        ));
    }
    let jacobian = derive::jacobian(model);
    if let JacobianIr::ColoredDifference { reason, .. } = &jacobian {
        return Err(unsupported(
            model,
            &format!("a model without an analytic Jacobian ({reason})"),
        ));
    }
    let dfdt = derive::time_derivatives(model).map_err(|op| {
        unsupported(
            model,
            &format!(
                "a model whose rates depend on the time through an operator without a \
                 derivative ({op:?})"
            ),
        )
    })?;
    let plan = optimize::plan(model, &jacobian, None, &discontinuities, true);
    let description = description(
        model,
        jacobian,
        None,
        &discontinuities,
        plan.n_constants(),
        0,
        None,
    );

    let n = model.states.len() as u32;
    let n_parameters = model.parameters.len() as u32;
    // the byte offsets into the work of a point are `i32` (a bound above the vectors of `Work`)
    let states = u64::from(n);
    let memory = u64::from(n_parameters) + u64::from(plan.n_constants()) + 1;
    if 8 * (memory + states * states + 12 * states) > i32::MAX as u64 {
        return Err(unsupported(
            model,
            &format!("a model of {n} states, whose work per point would be above 2 GiB"),
        ));
    }
    let work = Work::new(n, n_parameters, plan.n_constants(), dfdt.is_some());
    let mut m = ModuleBuilder::default();
    m.constants = n_parameters;

    // constants(p): the constants behind the parameters, each stored before the next reads it
    let constants = program_stmts(&plan.constants, |k, e| Stmt::Store(Symbol::Constant(k), e));
    m.sequence(
        &internal("constants", &[I32], Ctx { t: 0, y: 0, p: 0 }),
        &constants,
    )?;
    let constants = m.last_position();

    // rhs(t, y, p, out), jac(t, y, p, out): dense, row major, only the entries of the pattern
    let vectors = [F64, I32, I32, I32];
    let out = |k, e| Stmt::StoreAt { ptr: 3, k, expr: e };
    m.sequence(
        &internal("rhs", &vectors, VECTORS),
        &program_stmts(&plan.rhs, out),
    )?;
    let rhs = m.last_position();
    let pattern = description.jacobian.pattern();
    let mut dense = Vec::with_capacity(pattern.row_idx.len());
    for column in 0..n as usize {
        let rows = pattern.col_ptr[column] as usize..pattern.col_ptr[column + 1] as usize;
        for &row in &pattern.row_idx[rows] {
            dense.push(row * n + column as u32);
        }
    }
    let jac = plan.jac.as_ref().expect("an analytic Jacobian");
    let entries = program_stmts(jac, |k, e| Stmt::StoreAt {
        ptr: 3,
        k: dense[k as usize],
        expr: e,
    });
    m.sequence(&internal("jac", &vectors, VECTORS), &entries)?;
    let jac = m.last_position();

    // dfdt(t, y, p, out): the rules, the derivatives of those which depend on the time, then
    // the derivative of every rate
    let zero = Expr::Number(0.0);
    let dfdt = match &dfdt {
        Some(d) => {
            let mut stmts: Vec<Stmt<'_>> = rule_lets(model).collect();
            stmts.extend(
                d.rules
                    .iter()
                    .map(|(j, e)| Stmt::Let(Symbol::Derivative(*j), e)),
            );
            stmts.extend(d.rates.iter().enumerate().map(|(k, e)| Stmt::StoreAt {
                ptr: 3,
                k: k as u32,
                expr: e.as_ref().unwrap_or(&zero),
            }));
            m.sequence(&internal("dfdt", &vectors, VECTORS), &stmts)?;
            Some(m.last_position())
        }
        None => None,
    };

    // init(t, p, y): a parameter which is NaN takes its SBML value, a state its own; then each
    // initial evaluation, of a parameter only where it is NaN
    let targets: HashSet<Symbol> = model.initial.iter().map(|a| a.target).collect();
    let mut init: Vec<Stmt<'_>> = Vec::new();
    for (i, p) in model.parameters.iter().enumerate() {
        let target = Symbol::Param(i as u32);
        if !targets.contains(&target) {
            init.push(Stmt::Block {
                guard: Guard::Unset(target),
                body: vec![Stmt::Constant(target, p.default)],
            });
        }
    }
    for (i, s) in model.states.iter().enumerate() {
        init.push(Stmt::Constant(Symbol::State(i as u32), s.default));
    }
    for (a, program) in model.initial.iter().zip(&plan.init) {
        let guard = match a.target {
            Symbol::Param(_) => Guard::Unset(a.target),
            _ => Guard::Always,
        };
        init.push(Stmt::Block {
            guard,
            body: program_stmts(program, |_, e| Stmt::Store(a.target, e)),
        });
    }
    m.sequence(
        &internal("init", &[F64, I32, I32], Ctx { t: 0, y: 2, p: 1 }),
        &init,
    )?;
    let init = m.last_position();

    // assign(t, y, p): every assignment rule as written, in the order of dependency, stored
    // before the next one reads it; the math as written reads no constant (which `p` of the
    // caller does not hold), and gives the bits of the programs of the optimizer, whose
    // rewrites are exact
    let assign: Vec<Stmt<'_>> = model
        .assignment_rules
        .iter()
        .map(|a| Stmt::Store(a.target, &a.expr))
        .collect();
    m.sequence(&internal("assign", &[F64, I32, I32], VECTORS), &assign)?;
    let assign = m.last_position();

    let model_fns = ModelFns {
        constants,
        rhs,
        jac,
        dfdt,
        init,
        assign,
    };
    let fmax = c_minmax(&mut m, true);
    let fmin = c_minmax(&mut m, false);
    let pow = m.math(MathFn::Pow);
    let norm = norm(&mut m, &work, fmax);
    let lu = lu(&mut m, &work);
    let solve = solve(&mut m, &work);
    let math = Math {
        fmax,
        fmin,
        pow,
        norm,
        lu,
        solve,
    };
    let first = first_step(&mut m, &work, &model_fns, &math);
    let attempt = attempt(&mut m, &work, &model_fns, &math, first);
    init_export(&mut m, &work, &model_fns);
    advance_export(&mut m, &work, &model_fns, attempt);
    assign_export(&mut m, &work, &model_fns);

    let kernel = KernelDescription {
        version: KERNEL_CONTRACT.to_string(),
        state_ids: description.states.iter().map(|e| e.id.clone()).collect(),
        parameter_ids: description
            .parameters
            .iter()
            .map(|e| e.id.clone())
            .collect(),
        assigned_ids: model
            .assignment_rules
            .iter()
            .map(|a| match a.target {
                Symbol::Param(i) => description.parameters[i as usize].id.clone(),
                target => unreachable!("an assignment rule of {target:?}, which is no parameter"),
            })
            .collect(),
        control: CONTROL,
        counts: COUNTS,
        scratch_per_point: work.per_point,
        model: description,
    };
    Ok((m.finish(), kernel))
}

/// The signature of a function of the model which the module does not export.
fn internal<'a>(name: &'static str, params: &'a [ValType], ctx: Ctx) -> Signature<'a> {
    Signature {
        name,
        exported: false,
        params,
        results: &[],
        ctx,
    }
}

/// The positions of the functions of the model among the defined functions.
struct ModelFns {
    constants: u32,
    rhs: u32,
    jac: u32,
    dfdt: Option<u32>,
    init: u32,
    assign: u32,
}

/// The positions of the helpers of the integrator (and the index of `pow` of mathlib).
struct Math {
    fmax: u32,
    fmin: u32,
    pow: u32,
    norm: u32,
    lu: u32,
    solve: u32,
}

/// The work of a point in `scratch`: the byte offset of every vector from the start of the
/// work of the point, and its number of `f64`. `p` (the parameters, then the constants behind
/// them) is at the offset 0.
struct Work {
    /// The states.
    n: u32,
    /// The parameters, without the constants.
    parameters: u32,
    f0: u64,
    e: u64,
    ynew: u64,
    dy: u64,
    k: [u64; 6],
    ft: Option<u64>,
    /// The pivots, `i32`.
    pivot: u64,
    per_point: u32,
}

impl Work {
    /// The work of a model of `n` states, `parameters` parameters and `constants` constants,
    /// with the derivative of the rates in the time where `dfdt`.
    fn new(n: u32, parameters: u32, constants: u32, dfdt: bool) -> Work {
        let mut at = (parameters + constants).max(1);
        let mut next = |len: u32| {
            let offset = 8 * u64::from(at);
            at += len;
            offset
        };
        let f0 = next(n);
        let e = next(n * n);
        let ynew = next(n);
        let dy = next(n);
        let k = [(); 6].map(|()| next(n));
        let ft = dfdt.then(|| next(n));
        let pivot = next(n.div_ceil(2));
        Work {
            n,
            parameters,
            f0,
            e,
            ynew,
            dy,
            k,
            ft,
            pivot,
            per_point: at,
        }
    }
}

/// The memory argument of an `f64` at `offset` bytes.
fn f64_at(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

/// The memory argument of an `i32` at `offset` bytes.
fn i32_at(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 2,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

fn num(x: f64) -> Ieee64 {
    Ieee64::from(x)
}

/// The `i32` locals every function of the integrator declares after its parameters, before
/// its `f64` locals.
const INTEGERS: u32 = 12;

/// The code of a function of the integrator being written.
struct Code {
    bytes: Vec<u8>,
    calls: Vec<(usize, u32)>,
    params: u32,
    integers: u32,
    floats: u32,
}

impl Code {
    fn new(params: u32) -> Code {
        Code {
            bytes: Vec::new(),
            calls: Vec::new(),
            params,
            integers: 0,
            floats: 0,
        }
    }

    fn sink(&mut self) -> InstructionSink<'_> {
        InstructionSink::new(&mut self.bytes)
    }

    /// A new `i32` local.
    fn int(&mut self) -> u32 {
        assert!(self.integers < INTEGERS, "the integer locals of a function");
        self.integers += 1;
        self.params + self.integers - 1
    }

    /// A new `f64` local.
    fn float(&mut self) -> u32 {
        self.floats += 1;
        self.params + INTEGERS + self.floats - 1
    }

    /// `call` of the defined function at `position`.
    fn call(&mut self, position: u32) {
        call_defined(&mut self.bytes, &mut self.calls, position);
    }

    /// Adds the function to the module; its position.
    fn finish(
        self,
        m: &mut ModuleBuilder,
        export: Option<&'static str>,
        params: &[ValType],
        results: &[ValType],
    ) -> u32 {
        debug_assert_eq!(params.len() as u32, self.params);
        m.raw(
            export,
            params,
            results,
            RawCode {
                integers: INTEGERS,
                floats: self.floats,
                code: self.bytes,
                calls: self.calls,
            },
        )
    }

    /// `for var in start..end` of `i32`, `start` and `end` pushing one each (`end` before every
    /// iteration).
    fn for_range(
        &mut self,
        var: u32,
        start: impl FnOnce(&mut Code),
        end: impl FnOnce(&mut Code),
        body: impl FnOnce(&mut Code),
    ) {
        start(self);
        self.sink()
            .local_set(var)
            .block(BlockType::Empty)
            .loop_(BlockType::Empty)
            .local_get(var);
        end(self);
        self.sink().i32_ge_s().br_if(1);
        body(self);
        self.sink()
            .local_get(var)
            .i32_const(1)
            .i32_add()
            .local_set(var)
            .br(0)
            .end()
            .end();
    }

    /// `for kb in (0..8 n).step_by(8)`, `n` at least 1: the byte offset of the element `k` of a
    /// vector in the local `kb`.
    fn for_vector(&mut self, kb: u32, n: u32, body: impl FnOnce(&mut Code)) {
        assert!(n >= 1, "a loop over no element");
        self.sink()
            .i32_const(0)
            .local_set(kb)
            .loop_(BlockType::Empty);
        body(self);
        self.sink()
            .local_get(kb)
            .i32_const(8)
            .i32_add()
            .local_tee(kb)
            .i32_const((8 * n) as i32)
            .i32_lt_u()
            .br_if(0)
            .end();
    }

    /// The address of the element of the vector `v` at the byte offset in the local `kb`
    /// (without the static offset of `v`).
    fn element(&mut self, v: Vector, kb: u32) {
        self.sink().local_get(v.base).local_get(kb).i32_add();
    }

    /// Pushes the element of `v` at `kb`.
    fn load(&mut self, v: Vector, kb: u32) {
        self.element(v, kb);
        self.sink().f64_load(f64_at(v.offset));
    }

    /// `v[kb] = value`, `value` pushing the `f64`.
    fn store(&mut self, v: Vector, kb: u32, value: impl FnOnce(&mut Code)) {
        self.element(v, kb);
        value(self);
        self.sink().f64_store(f64_at(v.offset));
    }

    /// Pushes the address of `v` (its base and its static offset).
    fn address(&mut self, v: Vector) {
        self.sink().local_get(v.base);
        if v.offset != 0 {
            self.sink().i32_const(v.offset as i32).i32_add();
        }
    }

    /// Pushes whether the `f64` on the stack is finite (`|x| < inf`).
    fn is_finite(&mut self) {
        self.sink().f64_abs().f64_const(num(f64::INFINITY)).f64_lt();
    }
}

/// A vector of the work of a point (or the states): the local of its base pointer and its
/// static byte offset from it.
#[derive(Debug, Clone, Copy)]
struct Vector {
    base: u32,
    offset: u64,
}

/// `fmax(a, b)` (`max`) or `fmin(a, b)` of the C library: NaN is the missing argument.
fn c_minmax(m: &mut ModuleBuilder, max: bool) -> u32 {
    let mut c = Code::new(2);
    c.sink()
        .local_get(0)
        .local_get(0)
        .f64_ne()
        .if_(BlockType::Result(F64))
        .local_get(1)
        .else_()
        .local_get(1)
        .local_get(1)
        .f64_ne()
        .if_(BlockType::Result(F64))
        .local_get(0)
        .else_()
        .local_get(0)
        .local_get(1);
    if max {
        c.sink().f64_max();
    } else {
        c.sink().f64_min();
    }
    c.sink().end().end();
    c.finish(m, None, &[F64, F64], &[F64])
}

/// `norm(v, a, b, rtol, atol) -> f64`: the root mean square of `v` scaled by
/// `atol + rtol max(|a|, |b|)` (the vectors of `n` at the pointers `v`, `a` and `b`).
fn norm(m: &mut ModuleBuilder, work: &Work, fmax: u32) -> u32 {
    let mut c = Code::new(5);
    let (v, a, b, rtol, atol) = (0, 1, 2, 3, 4);
    let kb = c.int();
    let sum = c.float();
    let x = c.float();
    let at = |base| Vector { base, offset: 0 };
    c.sink().f64_const(num(0.0)).local_set(sum);
    c.for_vector(kb, work.n, |c| {
        // x = v[k] / (atol + rtol * fmax(|a[k]|, |b[k]|)); sum += x * x
        c.load(at(v), kb);
        c.sink().local_get(atol).local_get(rtol);
        c.load(at(a), kb);
        c.sink().f64_abs();
        c.load(at(b), kb);
        c.sink().f64_abs();
        c.call(fmax);
        c.sink()
            .f64_mul()
            .f64_add()
            .f64_div()
            .local_set(x)
            .local_get(sum)
            .local_get(x)
            .local_get(x)
            .f64_mul()
            .f64_add()
            .local_set(sum);
    });
    c.sink()
        .local_get(sum)
        .f64_const(num(f64::from(work.n)))
        .f64_div()
        .f64_sqrt();
    c.finish(m, None, &[I32, I32, I32, F64, F64], &[F64])
}

/// Pushes the address of the element `(row, column)` of the `n` by `n` row major matrix at the
/// pointer in the local `a`, `row` and `column` in locals.
fn matrix(c: &mut Code, a: u32, n: u32, row: u32, column: u32) {
    c.sink()
        .local_get(a)
        .local_get(row)
        .i32_const(n as i32)
        .i32_mul()
        .local_get(column)
        .i32_add()
        .i32_const(3)
        .i32_shl()
        .i32_add();
}

/// `lu(a, pivot) -> i32`: the LU decomposition with partial pivoting of the row major matrix
/// at `a` in place (`sbmlsim_rosenbrock_lu`): row `k` was swapped with row `pivot[k]` (`i32`)
/// at the elimination of column `k`; 0 where a pivot is 0 or not finite.
fn lu(m: &mut ModuleBuilder, work: &Work) -> u32 {
    let n = work.n;
    let mut c = Code::new(2);
    let (a, pivot) = (0, 1);
    let (j, p, r, col) = (c.int(), c.int(), c.int(), c.int());
    let (largest, x, d, l) = (c.float(), c.float(), c.float(), c.float());
    c.for_range(
        j,
        |c| {
            c.sink().i32_const(0);
        },
        |c| {
            c.sink().i32_const(n as i32);
        },
        |c| {
            // p = j; largest = |a[j][j]|
            c.sink().local_get(j).local_set(p);
            matrix(c, a, n, j, j);
            c.sink().f64_load(f64_at(0)).f64_abs().local_set(largest);
            c.for_range(
                r,
                |c| {
                    c.sink().local_get(j).i32_const(1).i32_add();
                },
                |c| {
                    c.sink().i32_const(n as i32);
                },
                |c| {
                    matrix(c, a, n, r, j);
                    c.sink()
                        .f64_load(f64_at(0))
                        .f64_abs()
                        .local_tee(x)
                        .local_get(largest)
                        .f64_gt()
                        .if_(BlockType::Empty)
                        .local_get(x)
                        .local_set(largest)
                        .local_get(r)
                        .local_set(p)
                        .end();
                },
            );
            // pivot[j] = p
            c.sink()
                .local_get(pivot)
                .local_get(j)
                .i32_const(2)
                .i32_shl()
                .i32_add()
                .local_get(p)
                .i32_store(i32_at(0));
            // if !(largest > 0) || !isfinite(largest): return 0
            c.sink()
                .local_get(largest)
                .f64_const(num(0.0))
                .f64_gt()
                .i32_eqz()
                .local_get(largest);
            c.is_finite();
            c.sink()
                .i32_eqz()
                .i32_or()
                .if_(BlockType::Empty)
                .i32_const(0)
                .return_()
                .end();
            // swap the rows j and p
            c.sink()
                .local_get(p)
                .local_get(j)
                .i32_ne()
                .if_(BlockType::Empty);
            c.for_range(
                col,
                |c| {
                    c.sink().i32_const(0);
                },
                |c| {
                    c.sink().i32_const(n as i32);
                },
                |c| {
                    matrix(c, a, n, j, col);
                    c.sink().f64_load(f64_at(0)).local_set(x);
                    matrix(c, a, n, j, col);
                    matrix(c, a, n, p, col);
                    c.sink().f64_load(f64_at(0)).f64_store(f64_at(0));
                    matrix(c, a, n, p, col);
                    c.sink().local_get(x).f64_store(f64_at(0));
                },
            );
            c.sink().end();
            // d = a[j][j]; the rows below: l = a[r][j] / d, a[r][c] -= l a[j][c]
            matrix(c, a, n, j, j);
            c.sink().f64_load(f64_at(0)).local_set(d);
            c.for_range(
                r,
                |c| {
                    c.sink().local_get(j).i32_const(1).i32_add();
                },
                |c| {
                    c.sink().i32_const(n as i32);
                },
                |c| {
                    matrix(c, a, n, r, j);
                    matrix(c, a, n, r, j);
                    c.sink()
                        .f64_load(f64_at(0))
                        .local_get(d)
                        .f64_div()
                        .local_tee(l)
                        .f64_store(f64_at(0));
                    c.for_range(
                        col,
                        |c| {
                            c.sink().local_get(j).i32_const(1).i32_add();
                        },
                        |c| {
                            c.sink().i32_const(n as i32);
                        },
                        |c| {
                            matrix(c, a, n, r, col);
                            matrix(c, a, n, r, col);
                            c.sink().f64_load(f64_at(0)).local_get(l);
                            matrix(c, a, n, j, col);
                            c.sink()
                                .f64_load(f64_at(0))
                                .f64_mul()
                                .f64_sub()
                                .f64_store(f64_at(0));
                        },
                    );
                },
            );
        },
    );
    c.sink().i32_const(1);
    c.finish(m, None, &[I32, I32], &[I32])
}

/// `solve(a, pivot, b)`: solves `a x = b` in place of `b` with the decomposition of [`lu`]
/// (`sbmlsim_rosenbrock_solve`).
fn solve(m: &mut ModuleBuilder, work: &Work) -> u32 {
    let n = work.n;
    let mut c = Code::new(3);
    let (a, pivot, b) = (0, 1, 2);
    let (j, p, r, col, back) = (c.int(), c.int(), c.int(), c.int(), c.int());
    let x = c.float();
    // the address of b[k], k in the local `k`
    let element = |c: &mut Code, k: u32| {
        c.sink()
            .local_get(b)
            .local_get(k)
            .i32_const(3)
            .i32_shl()
            .i32_add();
    };
    let zero = |c: &mut Code| {
        c.sink().i32_const(0);
    };
    let to_n = |c: &mut Code| {
        c.sink().i32_const(n as i32);
    };
    // the permutation of the decomposition
    c.for_range(j, zero, to_n, |c| {
        c.sink()
            .local_get(pivot)
            .local_get(j)
            .i32_const(2)
            .i32_shl()
            .i32_add()
            .i32_load(i32_at(0))
            .local_tee(p)
            .local_get(j)
            .i32_ne()
            .if_(BlockType::Empty);
        element(c, j);
        c.sink().f64_load(f64_at(0)).local_set(x);
        element(c, j);
        element(c, p);
        c.sink().f64_load(f64_at(0)).f64_store(f64_at(0));
        element(c, p);
        c.sink().local_get(x).f64_store(f64_at(0)).end();
    });
    // forward: b[r] -= a[r][c] b[c] for c < r
    c.for_range(
        r,
        |c| {
            c.sink().i32_const(1);
        },
        to_n,
        |c| {
            element(c, r);
            c.sink().f64_load(f64_at(0)).local_set(x);
            c.for_range(
                col,
                zero,
                |c| {
                    c.sink().local_get(r);
                },
                |c| {
                    c.sink().local_get(x);
                    matrix(c, a, n, r, col);
                    c.sink().f64_load(f64_at(0));
                    element(c, col);
                    c.sink()
                        .f64_load(f64_at(0))
                        .f64_mul()
                        .f64_sub()
                        .local_set(x);
                },
            );
            element(c, r);
            c.sink().local_get(x).f64_store(f64_at(0));
        },
    );
    // backward: r from n - 1 down to 0, b[r] = (b[r] - a[r][c] b[c] for c > r) / a[r][r]
    c.for_range(back, zero, to_n, |c| {
        c.sink()
            .i32_const(n as i32 - 1)
            .local_get(back)
            .i32_sub()
            .local_set(r);
        element(c, r);
        c.sink().f64_load(f64_at(0)).local_set(x);
        c.for_range(
            col,
            |c| {
                c.sink().local_get(r).i32_const(1).i32_add();
            },
            to_n,
            |c| {
                c.sink().local_get(x);
                matrix(c, a, n, r, col);
                c.sink().f64_load(f64_at(0));
                element(c, col);
                c.sink()
                    .f64_load(f64_at(0))
                    .f64_mul()
                    .f64_sub()
                    .local_set(x);
            },
        );
        element(c, r);
        c.sink().local_get(x);
        matrix(c, a, n, r, r);
        c.sink().f64_load(f64_at(0)).f64_div().f64_store(f64_at(0));
    });
    c.finish(m, None, &[I32, I32, I32], &[])
}

/// `first_step(t, rest, y, w, rtol, atol) -> f64`: the first step at `(t, y)` of the rates
/// `f0` of the work `w`, at most `rest`: `HINIT` of Hairer and Wanner for a method of order 4
/// (`sbmlsim_rosenbrock_first_step`); `ynew` and `dy` of the work are its vectors.
fn first_step(m: &mut ModuleBuilder, work: &Work, model: &ModelFns, math: &Math) -> u32 {
    let mut c = Code::new(6);
    let (t, rest, y, w, rtol, atol) = (0, 1, 2, 3, 4, 5);
    let kb = c.int();
    let (d0, d1, h0, d2, d12, h1) = (
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
    );
    let yv = Vector { base: y, offset: 0 };
    let f0 = Vector {
        base: w,
        offset: work.f0,
    };
    let y1 = Vector {
        base: w,
        offset: work.ynew,
    };
    let f1 = Vector {
        base: w,
        offset: work.dy,
    };
    let norm = |c: &mut Code, v: Vector| {
        c.address(v);
        c.sink()
            .local_get(y)
            .local_get(y)
            .local_get(rtol)
            .local_get(atol);
        c.call(math.norm);
    };
    // d0 = norm(y, y, y), d1 = norm(f0, y, y)
    norm(&mut c, yv);
    c.sink().local_set(d0);
    norm(&mut c, f0);
    c.sink().local_set(d1);
    // h0 = (d0 < 1e-5 || d1 < 1e-5) ? 1e-6 : 0.01 * d0 / d1; h0 = fmin(h0, rest)
    c.sink()
        .local_get(d0)
        .f64_const(num(1e-5))
        .f64_lt()
        .local_get(d1)
        .f64_const(num(1e-5))
        .f64_lt()
        .i32_or()
        .if_(BlockType::Result(F64))
        .f64_const(num(1e-6))
        .else_()
        .f64_const(num(0.01))
        .local_get(d0)
        .f64_mul()
        .local_get(d1)
        .f64_div()
        .end()
        .local_get(rest);
    c.call(math.fmin);
    c.sink().local_set(h0);
    // y1 = y + h0 f0; f1 = rhs(t + h0, y1) - f0
    c.for_vector(kb, work.n, |c| {
        c.store(y1, kb, |c| {
            c.load(yv, kb);
            c.sink().local_get(h0);
            c.load(f0, kb);
            c.sink().f64_mul().f64_add();
        });
    });
    c.sink().local_get(t).local_get(h0).f64_add();
    c.address(y1);
    c.sink().local_get(w);
    c.address(f1);
    c.call(model.rhs);
    c.for_vector(kb, work.n, |c| {
        c.store(f1, kb, |c| {
            c.load(f1, kb);
            c.load(f0, kb);
            c.sink().f64_sub();
        });
    });
    // d2 = norm(f1, y, y) / h0; d12 = fmax(d1, d2)
    norm(&mut c, f1);
    c.sink().local_get(h0).f64_div().local_set(d2);
    c.sink().local_get(d1).local_get(d2);
    c.call(math.fmax);
    c.sink().local_set(d12);
    // h1 = (d12 <= 1e-15 || !isfinite(d12)) ? fmax(1e-6, h0 * 1e-3) : pow(0.01 / d12, 0.25)
    c.sink()
        .local_get(d12)
        .f64_const(num(1e-15))
        .f64_le()
        .local_get(d12);
    c.is_finite();
    c.sink()
        .i32_eqz()
        .i32_or()
        .if_(BlockType::Result(F64))
        .f64_const(num(1e-6))
        .local_get(h0)
        .f64_const(num(1e-3))
        .f64_mul();
    c.call(math.fmax);
    c.sink()
        .else_()
        .f64_const(num(0.01))
        .local_get(d12)
        .f64_div()
        .f64_const(num(0.25))
        .call(math.pow)
        .end()
        .local_set(h1);
    // fmin(fmin(100 h0, h1), rest)
    c.sink()
        .f64_const(num(100.0))
        .local_get(h0)
        .f64_mul()
        .local_get(h1);
    c.call(math.fmin);
    c.sink().local_get(rest);
    c.call(math.fmin);
    c.finish(m, None, &[F64, F64, I32, I32, F64, F64], &[F64])
}

/// The coefficients of RODAS4 (`rodas.f`, `METH = 1`), written as `rodas.f` and
/// `sbmlsim_rosenbrock.h` write them (the same doubles).
#[allow(clippy::excessive_precision)]
mod rodas4 {
    pub const GAMMA: f64 = 0.25;
    pub const C2: f64 = 0.386;
    pub const C3: f64 = 0.21;
    pub const C4: f64 = 0.63;
    pub const D: [f64; 4] = [0.25, -0.1043, 0.1035, -0.3620000000000023e-01];
    pub const A21: f64 = 0.1544e+01;
    pub const A31: f64 = 0.9466785280815826e+00;
    pub const A32: f64 = 0.2557011698983284e+00;
    pub const A41: f64 = 0.3314825187068521e+01;
    pub const A42: f64 = 0.2896124015972201e+01;
    pub const A43: f64 = 0.9986419139977817e+00;
    pub const A51: f64 = 0.1221224509226641e+01;
    pub const A52: f64 = 0.6019134481288629e+01;
    pub const A53: f64 = 0.1253708332932087e+02;
    pub const A54: f64 = -0.6878860361058950e+00;
    pub const C21: f64 = -0.5668800000000000e+01;
    pub const C31: f64 = -0.2430093356833875e+01;
    pub const C32: f64 = -0.2063599157091915e+00;
    pub const C41: f64 = -0.1073529058151375e+00;
    pub const C42: f64 = -0.9594562251023355e+01;
    pub const C43: f64 = -0.2047028614809616e+02;
    pub const C51: f64 = 0.7496443313967647e+01;
    pub const C52: f64 = -0.1024680431464352e+02;
    pub const C53: f64 = -0.3399990352819905e+02;
    pub const C54: f64 = 0.1170890893206160e+02;
    pub const C61: f64 = 0.8083246795921522e+01;
    pub const C62: f64 = -0.7981132988064893e+01;
    pub const C63: f64 = -0.3152159432874371e+02;
    pub const C64: f64 = 0.1631930543123136e+02;
    pub const C65: f64 = -0.6058818238834054e+01;
}

/// `attempt(y, control, counts, w, t1, rtol, atol, h_min)`: one step attempt of RODAS4 of the
/// point of the states `y`, the control, the counts and the work `w` (whose `p` holds the
/// parameters and the constants), `sbmlsim_rosenbrock_attempt` operation for operation.
fn attempt(m: &mut ModuleBuilder, work: &Work, model: &ModelFns, math: &Math, first: u32) -> u32 {
    use rodas4::*;
    let n = work.n;
    let mut c = Code::new(8);
    let (y, ctl, cnt, w, t1, rtol, atol, h_min) = (0, 1, 2, 3, 4, 5, 6, 7);
    let (kb, singular, finite, last, r) = (c.int(), c.int(), c.int(), c.int(), c.int());
    let (t, rest, h, proposed, fac, err, q, next) = (
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
    );
    let hd: [u32; 4] = [(); 4].map(|()| c.float());
    // c_ij / h, by (i, j) of the stages 2 to 6
    let pairs: [(usize, usize, f64); 15] = [
        (2, 1, C21),
        (3, 1, C31),
        (3, 2, C32),
        (4, 1, C41),
        (4, 2, C42),
        (4, 3, C43),
        (5, 1, C51),
        (5, 2, C52),
        (5, 3, C53),
        (5, 4, C54),
        (6, 1, C61),
        (6, 2, C62),
        (6, 3, C63),
        (6, 4, C64),
        (6, 5, C65),
    ];
    let hc: Vec<u32> = pairs.iter().map(|_| c.float()).collect();
    let hc_of = |i: usize, j: usize| {
        hc[pairs
            .iter()
            .position(|&(a, b, _)| (a, b) == (i, j))
            .expect("a coefficient")]
    };
    let at = |offset| Vector { base: w, offset };
    let yv = Vector { base: y, offset: 0 };
    let f0 = at(work.f0);
    let e = at(work.e);
    let ynew = at(work.ynew);
    let dy = at(work.dy);
    let k = work.k.map(at);
    let ft = work.ft.map(at);
    let pivot = at(work.pivot);

    // t = control[0]; rest = t1 - t; a point at t1 is done
    c.sink()
        .local_get(ctl)
        .f64_load(f64_at(0))
        .local_set(t)
        .local_get(t1)
        .local_get(t)
        .f64_sub()
        .local_set(rest)
        .local_get(rest)
        .f64_const(num(0.0))
        .f64_gt()
        .i32_eqz()
        .if_(BlockType::Empty)
        .local_get(ctl)
        .f64_const(num(1.0))
        .f64_store(f64_at(16))
        .return_()
        .end();
    // f0 = rhs(t, y)
    c.sink().local_get(t).local_get(y).local_get(w);
    c.address(f0);
    c.call(model.rhs);
    // h = control[1], or the first step where it is none
    c.sink()
        .local_get(ctl)
        .f64_load(f64_at(8))
        .local_set(h)
        .local_get(h)
        .f64_const(num(0.0))
        .f64_gt()
        .i32_eqz()
        .local_get(h);
    c.is_finite();
    c.sink()
        .i32_eqz()
        .i32_or()
        .if_(BlockType::Empty)
        .local_get(t)
        .local_get(rest)
        .local_get(y)
        .local_get(w)
        .local_get(rtol)
        .local_get(atol);
    c.call(first);
    c.sink().local_get(h_min);
    c.call(math.fmax);
    c.sink().local_set(h).end();
    // the step before it is shortened or stretched to t1
    c.sink()
        .local_get(h)
        .local_set(proposed)
        .i32_const(0)
        .local_set(last)
        .local_get(h)
        .f64_const(num(1.0001))
        .f64_mul()
        .local_get(rest)
        .f64_ge()
        .if_(BlockType::Empty)
        .local_get(rest)
        .local_set(h)
        .i32_const(1)
        .local_set(last)
        .end();
    // !last && (h < h_min || !(t + h > t)): failed
    c.sink()
        .local_get(last)
        .i32_eqz()
        .local_get(h)
        .local_get(h_min)
        .f64_lt()
        .local_get(t)
        .local_get(h)
        .f64_add()
        .local_get(t)
        .f64_gt()
        .i32_eqz()
        .i32_or()
        .i32_and()
        .if_(BlockType::Empty)
        .local_get(ctl)
        .f64_const(num(1.0))
        .f64_store(f64_at(24))
        .return_()
        .end();

    // e = 1/(h gamma) I - J
    c.for_vector(kb, n * n, |c| {
        c.store(e, kb, |c| {
            c.sink().f64_const(num(0.0));
        });
    });
    c.sink().local_get(t).local_get(y).local_get(w);
    c.address(e);
    c.call(model.jac);
    c.sink()
        .f64_const(num(1.0))
        .local_get(h)
        .f64_const(num(GAMMA))
        .f64_mul()
        .f64_div()
        .local_set(fac);
    c.for_vector(kb, n * n, |c| {
        c.store(e, kb, |c| {
            c.load(e, kb);
            c.sink().f64_neg();
        });
    });
    // the diagonal: the byte offset r (n + 1) 8
    c.for_range(
        r,
        |c| {
            c.sink().i32_const(0);
        },
        |c| {
            c.sink().i32_const(n as i32);
        },
        |c| {
            c.sink()
                .local_get(r)
                .i32_const((8 * (n + 1)) as i32)
                .i32_mul()
                .local_set(kb);
            c.store(e, kb, |c| {
                c.load(e, kb);
                c.sink().local_get(fac).f64_add();
            });
        },
    );
    // err is valid where the matrix is not singular; finite where every new state is
    c.sink()
        .f64_const(num(0.0))
        .local_set(err)
        .i32_const(0)
        .local_set(finite);
    c.address(e);
    c.address(pivot);
    c.call(math.lu);
    c.sink()
        .i32_eqz()
        .local_tee(singular)
        .i32_eqz()
        .if_(BlockType::Empty);
    if let (Some(dfdt), Some(ft)) = (model.dfdt, ft) {
        c.sink().local_get(t).local_get(y).local_get(w);
        c.address(ft);
        c.call(dfdt);
        for (i, d) in D.iter().enumerate() {
            c.sink()
                .local_get(h)
                .f64_const(num(*d))
                .f64_mul()
                .local_set(hd[i]);
        }
    }
    for (&(_, _, coefficient), &local) in pairs.iter().zip(&hc) {
        c.sink()
            .f64_const(num(coefficient))
            .local_get(h)
            .f64_div()
            .local_set(local);
    }
    let solve = |c: &mut Code, v: Vector| {
        c.address(e);
        c.address(pivot);
        c.address(v);
        c.call(math.solve);
    };
    // ynew = y + sum a_j k_j, in the order of the C, then dy = rhs(t + ch, ynew)
    let stage_point = |c: &mut Code, a: &[f64], time: Option<f64>| {
        c.for_vector(kb, n, |c| {
            c.store(ynew, kb, |c| {
                c.load(yv, kb);
                for (j, coefficient) in a.iter().enumerate() {
                    c.sink().f64_const(num(*coefficient));
                    c.load(k[j], kb);
                    c.sink().f64_mul().f64_add();
                }
            });
        });
        c.sink().local_get(t);
        if let Some(factor) = time {
            c.sink().f64_const(num(factor)).local_get(h).f64_mul();
        } else {
            c.sink().local_get(h);
        }
        c.sink().f64_add();
        c.address(ynew);
        c.sink().local_get(w);
        c.address(dy);
        c.call(model.rhs);
    };
    // dy [+ hd ft], the first part of a stage of 2 to 4
    let with_time = |c: &mut Code, stage: usize| {
        c.load(dy, kb);
        if let Some(ft) = ft {
            c.sink().local_get(hd[stage - 1]);
            c.load(ft, kb);
            c.sink().f64_mul().f64_add();
        }
    };
    // pushes hc_i,j1 k_j1 + hc_i,j2 k_j2 + ..., left to right, in the order `js`
    let sum_of = |c: &mut Code, i: usize, js: &[usize]| {
        for (position, &j) in js.iter().enumerate() {
            c.sink().local_get(hc_of(i, j));
            c.load(k[j - 1], kb);
            c.sink().f64_mul();
            if position > 0 {
                c.sink().f64_add();
            }
        }
    };
    // stage 1: k1 = f0 [+ hd1 ft]
    c.for_vector(kb, n, |c| {
        c.store(k[0], kb, |c| {
            c.load(f0, kb);
            if let Some(ft) = ft {
                c.sink().local_get(hd[0]);
                c.load(ft, kb);
                c.sink().f64_mul().f64_add();
            }
        });
    });
    solve(&mut c, k[0]);
    // stage 2: k2 = (dy [+ hd2 ft]) + hc21 k1
    stage_point(&mut c, &[A21], Some(C2));
    c.for_vector(kb, n, |c| {
        c.store(k[1], kb, |c| {
            with_time(c, 2);
            sum_of(c, 2, &[1]);
            c.sink().f64_add();
        });
    });
    solve(&mut c, k[1]);
    // stage 3: k3 = (dy [+ hd3 ft]) + (hc31 k1 + hc32 k2)
    stage_point(&mut c, &[A31, A32], Some(C3));
    c.for_vector(kb, n, |c| {
        c.store(k[2], kb, |c| {
            with_time(c, 3);
            sum_of(c, 3, &[1, 2]);
            c.sink().f64_add();
        });
    });
    solve(&mut c, k[2]);
    // stage 4: k4 = (dy [+ hd4 ft]) + (hc41 k1 + hc42 k2 + hc43 k3)
    stage_point(&mut c, &[A41, A42, A43], Some(C4));
    c.for_vector(kb, n, |c| {
        c.store(k[3], kb, |c| {
            with_time(c, 4);
            sum_of(c, 4, &[1, 2, 3]);
            c.sink().f64_add();
        });
    });
    solve(&mut c, k[3]);
    // stage 5, at t + h: k5 = dy + (hc52 k2 + hc54 k4 + hc51 k1 + hc53 k3)
    stage_point(&mut c, &[A51, A52, A53, A54], None);
    c.for_vector(kb, n, |c| {
        c.store(k[4], kb, |c| {
            c.load(dy, kb);
            sum_of(c, 5, &[2, 4, 1, 3]);
            c.sink().f64_add();
        });
    });
    solve(&mut c, k[4]);
    // the embedded solution ynew += k5, then stage 6, whose solution is the estimate of its
    // error: k6 = dy + (hc61 k1 + hc62 k2 + hc65 k5 + hc64 k4 + hc63 k3)
    c.for_vector(kb, n, |c| {
        c.store(ynew, kb, |c| {
            c.load(ynew, kb);
            c.load(k[4], kb);
            c.sink().f64_add();
        });
    });
    c.sink().local_get(t).local_get(h).f64_add();
    c.address(ynew);
    c.sink().local_get(w);
    c.address(dy);
    c.call(model.rhs);
    c.for_vector(kb, n, |c| {
        c.store(k[5], kb, |c| {
            c.load(dy, kb);
            sum_of(c, 6, &[1, 2, 5, 4, 3]);
            c.sink().f64_add();
        });
    });
    solve(&mut c, k[5]);
    // ynew += k6, finite where every new state is; err = norm(k6, y, ynew)
    c.sink().i32_const(1).local_set(finite);
    c.for_vector(kb, n, |c| {
        c.store(ynew, kb, |c| {
            c.load(ynew, kb);
            c.load(k[5], kb);
            c.sink().f64_add();
        });
        c.sink().local_get(finite);
        c.load(ynew, kb);
        c.is_finite();
        c.sink().i32_and().local_set(finite);
    });
    c.address(k[5]);
    c.sink().local_get(y);
    c.address(ynew);
    c.sink().local_get(rtol).local_get(atol);
    c.call(math.norm);
    c.sink().local_tee(err);
    c.is_finite();
    c.sink().local_get(finite).i32_and().local_set(finite).end();

    // pushes err^(1/4) / 0.9
    let factor = |c: &mut Code| {
        c.sink()
            .local_get(err)
            .f64_const(num(0.25))
            .call(math.pow)
            .f64_const(num(0.9))
            .f64_div();
    };
    // accepted: !singular && finite && err <= 1
    c.sink()
        .local_get(singular)
        .i32_eqz()
        .local_get(finite)
        .i32_and()
        .local_get(err)
        .f64_const(num(1.0))
        .f64_le()
        .i32_and()
        .if_(BlockType::Empty);
    // q = fmax(1/6, fmin(5, err^(1/4) / 0.9)); next = h / q
    c.sink().f64_const(num(1.0 / 6.0)).f64_const(num(5.0));
    factor(&mut c);
    c.call(math.fmin);
    c.call(math.fmax);
    c.sink()
        .local_set(q)
        .local_get(h)
        .local_get(q)
        .f64_div()
        .local_set(next);
    c.for_vector(kb, n, |c| {
        c.store(yv, kb, |c| c.load(ynew, kb));
    });
    c.sink()
        .local_get(cnt)
        .local_get(cnt)
        .f64_load(f64_at(0))
        .f64_const(num(1.0))
        .f64_add()
        .f64_store(f64_at(0));
    // the last step: done at t1, with the step before it was shortened (or stretched),
    // scaled by the factor of the error where that asks for a smaller one
    c.sink()
        .local_get(last)
        .if_(BlockType::Empty)
        .local_get(ctl)
        .local_get(t1)
        .f64_store(f64_at(0))
        .local_get(ctl)
        .f64_const(num(1.0))
        .f64_store(f64_at(16))
        .local_get(ctl)
        .local_get(next)
        .local_get(proposed)
        .local_get(proposed)
        .local_get(q)
        .f64_div();
    c.call(math.fmin);
    c.call(math.fmax);
    c.sink()
        .f64_store(f64_at(8))
        .else_()
        .local_get(ctl)
        .local_get(t)
        .local_get(h)
        .f64_add()
        .f64_store(f64_at(0))
        .local_get(ctl)
        .local_get(next)
        .f64_store(f64_at(8))
        .end();
    // rejected: a singular matrix halves the step, an error which is not finite divides it by 5
    c.sink()
        .else_()
        .local_get(singular)
        .if_(BlockType::Result(F64))
        .f64_const(num(0.5))
        .local_get(h)
        .f64_mul()
        .else_()
        .local_get(finite)
        .if_(BlockType::Result(F64))
        .local_get(h)
        .f64_const(num(5.0));
    factor(&mut c);
    c.call(math.fmin);
    c.sink()
        .f64_div()
        .else_()
        .f64_const(num(0.2))
        .local_get(h)
        .f64_mul()
        .end()
        .end()
        .local_set(next)
        .local_get(cnt)
        .local_get(cnt)
        .f64_load(f64_at(8))
        .f64_const(num(1.0))
        .f64_add()
        .f64_store(f64_at(8))
        .local_get(ctl)
        .local_get(next)
        .f64_store(f64_at(8))
        .local_get(next)
        .local_get(h_min)
        .f64_lt()
        .local_get(t)
        .local_get(next)
        .f64_add()
        .local_get(t)
        .f64_gt()
        .i32_eqz()
        .i32_or()
        .if_(BlockType::Empty)
        .local_get(ctl)
        .f64_const(num(1.0))
        .f64_store(f64_at(24))
        .end()
        .end();
    c.finish(m, None, &[I32, I32, I32, I32, F64, F64, F64, F64], &[])
}

/// Pushes whether the array of `n` points (the count in the local `count`) of `bytes` per point
/// at the pointer in the local `pointer` reaches past the end of the memory.
fn out_of_bounds(c: &mut Code, pointer: u32, count: u32, bytes: u64) {
    c.sink()
        .local_get(pointer)
        .i64_extend_i32_u()
        .local_get(count)
        .i64_extend_i32_u()
        .i64_const(bytes as i64)
        .i64_mul()
        .i64_add()
        .memory_size(0)
        .i64_extend_i32_u()
        .i64_const(16)
        .i64_shl()
        .i64_gt_u();
}

/// Pushes whether any of the pointers in the locals `pointers` is not a multiple of 8.
fn misaligned(c: &mut Code, pointers: &[u32]) {
    for (position, &p) in pointers.iter().enumerate() {
        c.sink().local_get(p);
        if position > 0 {
            c.sink().i32_or();
        }
    }
    c.sink().i32_const(7).i32_and();
}

/// `return -1` where the `i32` on the stack is not 0.
fn refuse_if(c: &mut Code) {
    c.sink().if_(BlockType::Empty).i32_const(-1).return_().end();
}

/// The export `sbmlsim_kernel_init(n, parameters, state) -> i32`.
fn init_export(m: &mut ModuleBuilder, work: &Work, model: &ModelFns) {
    let n_states = u64::from(work.n);
    let n_parameters = u64::from(work.parameters);
    let mut c = Code::new(3);
    let (count, parameters, state) = (0, 1, 2);
    let i = c.int();
    c.sink().local_get(count).i32_const(0).i32_lt_s();
    refuse_if(&mut c);
    misaligned(&mut c, &[parameters, state]);
    refuse_if(&mut c);
    out_of_bounds(&mut c, parameters, count, 8 * n_parameters);
    out_of_bounds(&mut c, state, count, 8 * n_states);
    c.sink().i32_or();
    refuse_if(&mut c);
    c.for_range(
        i,
        |c| {
            c.sink().i32_const(0);
        },
        |c| {
            c.sink().local_get(count);
        },
        |c| {
            c.sink()
                .f64_const(num(0.0))
                .local_get(parameters)
                .local_get(i)
                .i32_const((8 * n_parameters) as i32)
                .i32_mul()
                .i32_add()
                .local_get(state)
                .local_get(i)
                .i32_const((8 * n_states) as i32)
                .i32_mul()
                .i32_add();
            c.call(model.init);
        },
    );
    c.sink().i32_const(0);
    c.finish(m, Some(KERNEL_INIT), &[I32, I32, I32], &[I32]);
}

/// The export `sbmlsim_kernel_assign(n, state, parameters, control) -> i32`: the assignment
/// rules of every point at its time (`control[0]`) and its states, into its parameters.
fn assign_export(m: &mut ModuleBuilder, work: &Work, model: &ModelFns) {
    let n_states = u64::from(work.n);
    let n_parameters = u64::from(work.parameters);
    let n_control = u64::from(CONTROL);
    let mut c = Code::new(4);
    let (count, state, parameters, control) = (0, 1, 2, 3);
    let i = c.int();
    c.sink().local_get(count).i32_const(0).i32_lt_s();
    refuse_if(&mut c);
    misaligned(&mut c, &[state, parameters, control]);
    refuse_if(&mut c);
    out_of_bounds(&mut c, state, count, 8 * n_states);
    out_of_bounds(&mut c, parameters, count, 8 * n_parameters);
    c.sink().i32_or();
    out_of_bounds(&mut c, control, count, 8 * n_control);
    c.sink().i32_or();
    refuse_if(&mut c);
    c.for_range(
        i,
        |c| {
            c.sink().i32_const(0);
        },
        |c| {
            c.sink().local_get(count);
        },
        |c| {
            // assign(control[i][0], state + i, parameters + i)
            c.sink()
                .local_get(control)
                .local_get(i)
                .i32_const((8 * n_control) as i32)
                .i32_mul()
                .i32_add()
                .f64_load(f64_at(0))
                .local_get(state)
                .local_get(i)
                .i32_const((8 * n_states) as i32)
                .i32_mul()
                .i32_add()
                .local_get(parameters)
                .local_get(i)
                .i32_const((8 * n_parameters) as i32)
                .i32_mul()
                .i32_add();
            c.call(model.assign);
        },
    );
    c.sink().i32_const(0);
    c.finish(m, Some(KERNEL_ASSIGN), &[I32, I32, I32, I32], &[I32]);
}

/// The export
/// `sbmlsim_kernel_advance(n, state, parameters, control, counts, context, scratch) -> i32`.
fn advance_export(m: &mut ModuleBuilder, work: &Work, model: &ModelFns, attempt: u32) {
    let n_states = u64::from(work.n);
    let n_parameters = work.parameters;
    let per_point = u64::from(work.per_point);
    let mut c = Code::new(7);
    let (count, state, parameters, control, counts, context, scratch) = (0, 1, 2, 3, 4, 5, 6);
    let (i, y, p, ctl, cnt, w, kb, failed) = (
        c.int(),
        c.int(),
        c.int(),
        c.int(),
        c.int(),
        c.int(),
        c.int(),
        c.int(),
    );
    let (t1, rtol, atol, h_min, attempts, a) = (
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
        c.float(),
    );
    // the arguments
    c.sink().local_get(count).i32_const(0).i32_lt_s();
    refuse_if(&mut c);
    misaligned(
        &mut c,
        &[state, parameters, control, counts, context, scratch],
    );
    refuse_if(&mut c);
    out_of_bounds(&mut c, state, count, 8 * n_states);
    out_of_bounds(&mut c, parameters, count, 8 * u64::from(n_parameters));
    c.sink().i32_or();
    out_of_bounds(&mut c, control, count, 8 * u64::from(CONTROL));
    c.sink().i32_or();
    out_of_bounds(&mut c, counts, count, 8 * u64::from(COUNTS));
    c.sink().i32_or();
    out_of_bounds(&mut c, scratch, count, 8 * per_point);
    c.sink().i32_or();
    // the context: one "point" of 5 values
    c.sink()
        .local_get(context)
        .i64_extend_i32_u()
        .i64_const(8 * i64::from(CONTEXT))
        .i64_add()
        .memory_size(0)
        .i64_extend_i32_u()
        .i64_const(16)
        .i64_shl()
        .i64_gt_u()
        .i32_or();
    refuse_if(&mut c);
    for (k, local) in [t1, rtol, atol, h_min, attempts].into_iter().enumerate() {
        c.sink()
            .local_get(context)
            .f64_load(f64_at(8 * k as u64))
            .local_set(local);
    }
    // t1 finite; rtol, atol, h_min finite and positive; attempts a whole number of at least 1
    c.sink().local_get(t1);
    c.is_finite();
    c.sink().i32_eqz();
    for local in [rtol, atol, h_min] {
        c.sink()
            .local_get(local)
            .f64_const(num(0.0))
            .f64_gt()
            .local_get(local);
        c.is_finite();
        c.sink().i32_and().i32_eqz().i32_or();
    }
    c.sink()
        .local_get(attempts)
        .f64_const(num(1.0))
        .f64_ge()
        .local_get(attempts);
    c.is_finite();
    c.sink()
        .i32_and()
        .local_get(attempts)
        .f64_floor()
        .local_get(attempts)
        .f64_eq()
        .i32_and()
        .i32_eqz()
        .i32_or();
    refuse_if(&mut c);

    c.sink().i32_const(0).local_set(failed);
    let pointer = |c: &mut Code, base: u32, bytes: u64, into: u32| {
        c.sink()
            .local_get(base)
            .local_get(i)
            .i32_const(bytes as i32)
            .i32_mul()
            .i32_add()
            .local_set(into);
    };
    c.for_range(
        i,
        |c| {
            c.sink().i32_const(0);
        },
        |c| {
            c.sink().local_get(count);
        },
        |c| {
            pointer(c, state, 8 * n_states, y);
            pointer(c, parameters, 8 * u64::from(n_parameters), p);
            pointer(c, control, 8 * u64::from(CONTROL), ctl);
            pointer(c, counts, 8 * u64::from(COUNTS), cnt);
            pointer(c, scratch, 8 * per_point, w);
            // a point neither done nor failed: its parameters and constants into the work,
            // then attempts until it is done or failed, at most `attempts`
            c.sink()
                .local_get(ctl)
                .f64_load(f64_at(16))
                .f64_const(num(0.0))
                .f64_eq()
                .local_get(ctl)
                .f64_load(f64_at(24))
                .f64_const(num(0.0))
                .f64_eq()
                .i32_and()
                .if_(BlockType::Empty);
            if n_parameters > 0 {
                let pv = Vector { base: p, offset: 0 };
                let wv = Vector { base: w, offset: 0 };
                c.for_vector(kb, n_parameters, |c| {
                    c.store(wv, kb, |c| c.load(pv, kb));
                });
            }
            c.sink().local_get(w);
            c.call(model.constants);
            c.sink()
                .f64_const(num(0.0))
                .local_set(a)
                .block(BlockType::Empty)
                .loop_(BlockType::Empty)
                .local_get(a)
                .local_get(attempts)
                .f64_lt()
                .i32_eqz()
                .local_get(ctl)
                .f64_load(f64_at(16))
                .f64_const(num(0.0))
                .f64_ne()
                .i32_or()
                .local_get(ctl)
                .f64_load(f64_at(24))
                .f64_const(num(0.0))
                .f64_ne()
                .i32_or()
                .br_if(1)
                .local_get(y)
                .local_get(ctl)
                .local_get(cnt)
                .local_get(w)
                .local_get(t1)
                .local_get(rtol)
                .local_get(atol)
                .local_get(h_min);
            c.call(attempt);
            c.sink()
                .local_get(a)
                .f64_const(num(1.0))
                .f64_add()
                .local_set(a)
                .br(0)
                .end()
                .end()
                .end();
            // the failed points
            c.sink()
                .local_get(ctl)
                .f64_load(f64_at(24))
                .f64_const(num(0.0))
                .f64_ne()
                .local_get(failed)
                .i32_add()
                .local_set(failed);
        },
    );
    c.sink().local_get(failed);
    c.finish(
        m,
        Some(KERNEL_ADVANCE),
        &[I32, I32, I32, I32, I32, I32, I32],
        &[I32],
    );
}

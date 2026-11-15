//! The encoding of `model.wasm`: the module, its functions and the code of the expressions.
//!
//! The module imports the memory of the engine as `env.memory` and the functions of mathlib it
//! calls as `math.<name>`, in the order of their first call; it defines the functions of
//! `model/2` and exports them, and has no memory, table, global, data segment or start function
//! of its own. A function is a list of statements, each one store of one expression into the
//! memory or into a local; temporaries are locals. Every function is checked against the limits
//! [`MAX_FUNCTION_BYTES`] (the whole encoded body, the declaration of its locals included) and
//! [`MAX_LOCALS`] (with the parameters), which are well below the limits of V8 (7,654,321
//! bytes and 50,000 locals); a function of the code generation above them is split into helper
//! functions (`split.rs`), and a statement which alone is above them is an error and not a
//! module an engine refuses.

use std::collections::{BTreeMap, HashMap};

use sbmlsim_ir::{Assignment, CompileError, Expr, Op, Symbol};
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    Ieee64, ImportSection, InstructionSink, MemArg, MemArgTarget, MemoryType, Module, TypeSection,
    ValType,
};

use crate::math::{HALF_PI, QUOTIENT_HIGH, QUOTIENT_INVALID, QUOTIENT_LOW, is_boolean};

/// The largest encoded body of a function of `model.wasm` in bytes (the declaration of the
/// locals, the code and its `end`); a larger function is split into helper functions. Below
/// V8's limit of 7,654,321 bytes, and small enough that V8 compiles a function quickly.
pub const MAX_FUNCTION_BYTES: usize = 1_000_000;
/// The most locals (with the parameters) of a function of `model.wasm`; a function with more is
/// split into helper functions. Below V8's limit of 50,000, and a frame of at most 160 kB.
pub const MAX_LOCALS: usize = 20_000;

/// The feature of the error of a function above the limits ([`Body::finish`]).
pub(crate) const ABOVE_THE_LIMITS: &str = "a function above the limits of WebAssembly engines";

/// Whether an error is the one of a function (a statement of it) above the limits.
pub fn is_above_the_limits(error: &CompileError) -> bool {
    matches!(error, CompileError::Unsupported { feature, .. } if feature.starts_with(ABOVE_THE_LIMITS))
}

/// A function of mathlib a model calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum MathFn {
    Exp,
    Log,
    Pow,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Factorial,
    Fmod,
}

impl MathFn {
    /// The name of the export of mathlib.
    pub(crate) fn name(self) -> &'static str {
        match self {
            MathFn::Exp => "math_exp",
            MathFn::Log => "math_log",
            MathFn::Pow => "math_pow",
            MathFn::Sin => "math_sin",
            MathFn::Cos => "math_cos",
            MathFn::Tan => "math_tan",
            MathFn::Asin => "math_asin",
            MathFn::Acos => "math_acos",
            MathFn::Atan => "math_atan",
            MathFn::Sinh => "math_sinh",
            MathFn::Cosh => "math_cosh",
            MathFn::Tanh => "math_tanh",
            MathFn::Factorial => "math_factorial",
            MathFn::Fmod => "math_fmod",
        }
    }

    /// The number of `f64` arguments.
    fn arity(self) -> usize {
        match self {
            MathFn::Pow | MathFn::Fmod => 2,
            _ => 1,
        }
    }
}

/// A signature of a function.
type Signature = (Vec<ValType>, Vec<ValType>);

/// The limits a function is checked against (and split above).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest body in bytes.
    pub bytes: usize,
    /// The most locals, with the parameters.
    pub locals: usize,
}

impl Default for Limits {
    /// The limits of WebAssembly engines, [`MAX_FUNCTION_BYTES`] and [`MAX_LOCALS`].
    fn default() -> Self {
        Limits {
            bytes: MAX_FUNCTION_BYTES,
            locals: MAX_LOCALS,
        }
    }
}

/// The module being built.
#[derive(Default)]
pub(crate) struct ModuleBuilder {
    types: Vec<Signature>,
    math: Vec<MathFn>,
    math_index: BTreeMap<MathFn, u32>,
    functions: Vec<DefinedFunction>,
    /// The limits of every function (the defaults but in tests).
    pub limits: Limits,
    /// The slot of the first constant in `p` (`Symbol::Constant`): the number of parameters.
    pub constants: u32,
}

/// A function the module defines: a function of `model/2` it exports, or a helper function of a
/// split one.
struct DefinedFunction {
    export: Option<&'static str>,
    ty: u32,
    locals: u32,
    code: Vec<u8>,
    /// The calls of helper functions: the offset of the padded index in the code and the
    /// position of the callee among the defined functions.
    calls: Vec<(usize, u32)>,
}

/// The bytes of the index of a call of a helper function: a LEB128 padded to its most bytes,
/// patched when the module is finished (the index of a defined function follows the imports of
/// mathlib, which are known only then).
const CALL_INDEX_BYTES: usize = 5;

/// The number of bytes of the unsigned LEB128 of `x`.
fn leb_len(x: u64) -> usize {
    (64 - x.max(1).leading_zeros() as usize).div_ceil(7)
}

/// The size of the encoded body of a function of `locals` locals (one group of `f64`) and `code`
/// bytes of code without its `end`.
pub(crate) fn body_size(locals: u32, code: usize) -> usize {
    leb_len(1) + leb_len(u64::from(locals)) + 1 + code + 1
}

impl ModuleBuilder {
    /// The index of a type.
    fn type_index(&mut self, signature: Signature) -> u32 {
        let position = match self.types.iter().position(|t| *t == signature) {
            Some(position) => position,
            None => {
                self.types.push(signature);
                self.types.len() - 1
            }
        };
        position as u32
    }

    /// The function index of an import of mathlib, imported at its first call.
    fn math(&mut self, f: MathFn) -> u32 {
        if let Some(index) = self.math_index.get(&f) {
            return *index;
        }
        let index = self.math.len() as u32;
        self.math.push(f);
        self.math_index.insert(f, index);
        index
    }

    /// Starts a function of the given parameters and result, exported as `export`.
    pub(crate) fn function<'b>(
        &'b mut self,
        export: &'static str,
        params: &[ValType],
        results: &[ValType],
        ctx: Ctx,
    ) -> Body<'b> {
        self.body(Some(export), params, results, ctx)
    }

    /// Starts a function, exported as `export` or a helper function without.
    pub(crate) fn body<'b>(
        &'b mut self,
        export: Option<&'static str>,
        params: &[ValType],
        results: &[ValType],
        ctx: Ctx,
    ) -> Body<'b> {
        let ty = self.type_index((params.to_vec(), results.to_vec()));
        Body {
            module: self,
            export,
            ty,
            n_params: params.len() as u32,
            n_locals: 0,
            free: Vec::new(),
            code: Vec::new(),
            calls: Vec::new(),
            ctx,
            rules: HashMap::new(),
        }
    }

    /// The bytes of the module.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        let math_types: Vec<u32> = self
            .math
            .clone()
            .into_iter()
            .map(|f| self.type_index((vec![ValType::F64; f.arity()], vec![ValType::F64])))
            .collect();
        let mut types = TypeSection::new();
        for (params, results) in &self.types {
            types
                .ty()
                .function(params.iter().copied(), results.iter().copied());
        }
        let mut imports = ImportSection::new();
        imports.import(
            "env",
            "memory",
            EntityType::Memory(MemoryType {
                minimum: 0,
                maximum: None,
                memory64: false,
                shared: false,
                page_size_log2: None,
            }),
        );
        for (f, ty) in self.math.iter().zip(&math_types) {
            imports.import("math", f.name(), EntityType::Function(*ty));
        }
        let mut functions = FunctionSection::new();
        let mut exports = ExportSection::new();
        let mut code = CodeSection::new();
        let first = self.math.len() as u32;
        for (i, f) in self.functions.iter_mut().enumerate() {
            functions.function(f.ty);
            if let Some(export) = f.export {
                exports.export(export, ExportKind::Func, first + i as u32);
            }
            for &(offset, callee) in &f.calls {
                let mut index = first + callee;
                for byte in &mut f.code[offset..offset + CALL_INDEX_BYTES] {
                    *byte = (index & 0x7f) as u8 | 0x80;
                    index >>= 7;
                }
                f.code[offset + CALL_INDEX_BYTES - 1] &= 0x7f;
            }
            let mut body = Function::new([(f.locals, ValType::F64)]);
            body.raw(f.code.iter().copied());
            code.function(&body);
        }
        let mut module = Module::new();
        module
            .section(&types)
            .section(&imports)
            .section(&functions)
            .section(&exports)
            .section(&code);
        module.finish()
    }
}

/// Where an expression reads its values: the locals of the time and of the pointers to `y` and
/// `p`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctx {
    pub t: u32,
    pub y: u32,
    pub p: u32,
}

/// The body of a function being built.
pub(crate) struct Body<'b> {
    module: &'b mut ModuleBuilder,
    export: Option<&'static str>,
    ty: u32,
    n_params: u32,
    n_locals: u32,
    free: Vec<u32>,
    code: Vec<u8>,
    /// The calls of helper functions ([`DefinedFunction::calls`]).
    calls: Vec<(usize, u32)>,
    ctx: Ctx,
    /// The locals holding the values of the targets of assignment rules, which the expressions
    /// read instead of the memory.
    rules: HashMap<Symbol, u32>,
}

/// The memory argument of the `f64` at index `i` of a vector.
fn slot(i: u32) -> MemArg {
    MemArg {
        offset: 8 * u64::from(i),
        align: 3,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

/// The memory argument of the byte at index `i`.
fn byte(i: u32) -> MemArg {
    MemArg {
        offset: u64::from(i),
        align: 0,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

fn f64c(x: f64) -> Ieee64 {
    Ieee64::from(x)
}

/// A position in a body to go back to ([`Body::mark`], [`Body::rollback`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mark {
    code: usize,
    calls: usize,
    n_locals: u32,
}

impl Body<'_> {
    fn sink(&mut self) -> InstructionSink<'_> {
        InstructionSink::new(&mut self.code)
    }

    /// The position after the code so far.
    pub(crate) fn mark(&self) -> Mark {
        Mark {
            code: self.code.len(),
            calls: self.calls.len(),
            n_locals: self.n_locals,
        }
    }

    /// Drops the code and the locals after `mark`; the function is only finished after it
    /// (the locals of the expressions and the rules may refer to dropped locals).
    pub(crate) fn rollback(&mut self, mark: Mark) {
        self.code.truncate(mark.code);
        self.calls.truncate(mark.calls);
        self.n_locals = mark.n_locals;
    }

    /// Whether the function, finished now, is within the limits of the module.
    pub(crate) fn within_limits(&self) -> bool {
        self.within_limits_with(0)
    }

    /// Whether the function, finished after `bytes` more bytes of code, is within the limits.
    pub(crate) fn within_limits_with(&self, bytes: usize) -> bool {
        let limits = self.module.limits;
        self.size() + bytes <= limits.bytes && self.locals() <= limits.locals
    }

    /// The size of the encoded body if the function were finished now.
    pub(crate) fn size(&self) -> usize {
        body_size(self.n_locals, self.code.len())
    }

    /// The number of locals with the parameters.
    pub(crate) fn locals(&self) -> usize {
        (self.n_params + self.n_locals) as usize
    }

    /// The value of an expression into a local (one given back before, or a new one), which
    /// later expressions read for `symbol` until [`Body::release_let`].
    pub(crate) fn let_(&mut self, symbol: Symbol, e: &Expr) {
        self.f64(e);
        let local = self.temp();
        self.sink().local_set(local);
        self.rules.insert(symbol, local);
    }

    /// Whether a local holds the value of `symbol`.
    pub(crate) fn has_let(&self, symbol: Symbol) -> bool {
        self.rules.contains_key(&symbol)
    }

    /// Gives back the local of `symbol` (after the last statement which reads it): later
    /// expressions read the memory for it, and later locals may reuse the local.
    pub(crate) fn release_let(&mut self, symbol: Symbol) {
        if let Some(local) = self.rules.remove(&symbol) {
            self.free.push(local);
        }
    }

    /// `local.get` of every parameter, then the call of the helper function at the position
    /// `callee` among the defined functions.
    pub(crate) fn call_helper(&mut self, callee: u32) {
        for i in 0..self.n_params {
            self.sink().local_get(i);
        }
        self.code.push(0x10);
        self.calls.push((self.code.len(), callee));
        self.code.extend_from_slice(&[0; CALL_INDEX_BYTES]);
    }

    /// `if (local 0 as u32) < k {`, the `i32` index in the local 0; closed by [`Body::end`].
    pub(crate) fn if_index_below(&mut self, k: u32) {
        self.sink()
            .local_get(0)
            .i32_const(k as i32)
            .i32_lt_u()
            .if_(BlockType::Empty);
    }

    /// A new `f64` local.
    fn local(&mut self) -> u32 {
        let index = self.n_params + self.n_locals;
        self.n_locals += 1;
        index
    }

    /// A temporary `f64` local, given back with [`Body::release`].
    fn temp(&mut self) -> u32 {
        match self.free.pop() {
            Some(index) => index,
            None => self.local(),
        }
    }

    fn release(&mut self, index: u32) {
        self.free.push(index);
    }

    /// `p[i] = expr` or `y[i] = expr` (the vector of the symbol).
    pub(crate) fn store(&mut self, target: Symbol, expr: &Expr) {
        let (ptr, i) = self.address(target);
        self.sink().local_get(ptr);
        self.f64(expr);
        self.sink().f64_store(slot(i));
    }

    /// `out[i] = expr`, `out` the pointer in the local `ptr`.
    pub(crate) fn store_at(&mut self, ptr: u32, i: u32, expr: &Expr) {
        self.sink().local_get(ptr);
        self.f64(expr);
        self.sink().f64_store(slot(i));
    }

    /// `p[i] = x` or `y[i] = x` for a constant.
    pub(crate) fn store_constant(&mut self, target: Symbol, x: f64) {
        let (ptr, i) = self.address(target);
        self.sink()
            .local_get(ptr)
            .f64_const(f64c(x))
            .f64_store(slot(i));
    }

    /// `if mask[k] != 0 {`, `mask` the pointer in the local `mask`; closed by [`Body::end`].
    pub(crate) fn if_masked(&mut self, mask: u32, k: u32) {
        self.sink()
            .local_get(mask)
            .i32_load8_u(byte(k))
            .if_(BlockType::Empty);
    }

    /// `if fixed[k] == 0 {`, `fixed` the pointer in the local `fixed`; closed by [`Body::end`].
    pub(crate) fn if_not_fixed(&mut self, fixed: u32, k: u32) {
        self.sink()
            .local_get(fixed)
            .i32_load8_u(byte(k))
            .i32_eqz()
            .if_(BlockType::Empty);
    }

    /// Evaluates an assignment rule into a local, which later expressions read for its target.
    pub(crate) fn assign_local(&mut self, target: Symbol, expr: &Expr) {
        self.f64(expr);
        let local = self.local();
        self.sink().local_set(local);
        self.rules.insert(target, local);
    }

    /// `unreachable`.
    pub(crate) fn unreachable(&mut self) {
        self.sink().unreachable();
    }

    /// Evaluates the assignment rules into locals, which later expressions read for their
    /// targets; the locals of an earlier evaluation are forgotten.
    pub(crate) fn assign_locals(&mut self, rules: &[Assignment]) {
        self.rules.clear();
        for a in rules {
            self.assign_local(a.target, &a.expr);
        }
    }

    /// Forgets the locals of the assignment rules: later expressions read the memory.
    pub(crate) fn forget_rules(&mut self) {
        self.rules.clear();
    }

    /// `if (local i == k) {`, the `i32` index in the local `i`; closed by [`Body::end`].
    pub(crate) fn if_index(&mut self, i: u32, k: u32) {
        self.sink()
            .local_get(i)
            .i32_const(k as i32)
            .i32_eq()
            .if_(BlockType::Empty);
    }

    /// `}` of a block.
    pub(crate) fn end(&mut self) {
        self.sink().end();
    }

    /// Returns the expression as a boolean (`i32`).
    pub(crate) fn return_bool(&mut self, e: &Expr) {
        self.bool(e);
        self.sink().return_();
    }

    /// Returns the expression as an `f64`.
    pub(crate) fn return_f64(&mut self, e: &Expr) {
        self.f64(e);
        self.sink().return_();
    }

    /// `return`.
    pub(crate) fn return_(&mut self) {
        self.sink().return_();
    }

    /// Pushes an `i32` constant.
    pub(crate) fn i32_const(&mut self, x: i32) {
        self.sink().i32_const(x);
    }

    /// `out[k] = x` for the `i32` vector `out` in the local `ptr`.
    pub(crate) fn store_i32_at(&mut self, ptr: u32, k: u32, x: u32) {
        let arg = MemArg {
            offset: 4 * u64::from(k),
            align: 2,
            target: MemArgTarget::Memory(0),
            ordering: None,
        };
        self.sink()
            .local_get(ptr)
            .i32_const(x as i32)
            .i32_store(arg);
    }

    /// The value of `e` in a new local, kept to the end of the function.
    pub(crate) fn keep(&mut self, e: &Expr) -> u32 {
        self.f64(e);
        let local = self.local();
        self.sink().local_set(local);
        local
    }

    /// `target = source * from / to` when the locals `from` and `to` differ (or are NaN), else
    /// `target = source`: the source is `values[j]` (the vector in the local `values`) or,
    /// without `j`, the current value of the target (which then stays as it is).
    pub(crate) fn store_rescaled(
        &mut self,
        target: Symbol,
        values: u32,
        j: Option<u32>,
        from: u32,
        to: u32,
    ) {
        let (ptr, i) = self.address(target);
        let source = |body: &mut Self| match j {
            Some(j) => {
                body.sink().local_get(values).f64_load(slot(j));
            }
            None => {
                body.sink().local_get(ptr).f64_load(slot(i));
            }
        };
        self.sink()
            .local_get(from)
            .local_get(to)
            .f64_ne()
            .if_(BlockType::Empty)
            .local_get(ptr);
        source(self);
        self.sink()
            .local_get(from)
            .f64_mul()
            .local_get(to)
            .f64_div()
            .f64_store(slot(i));
        if j.is_some() {
            self.sink().else_().local_get(ptr);
            source(self);
            self.sink().f64_store(slot(i));
        }
        self.sink().end();
    }

    /// `target = source`, where the source is `values[j]` (the vector in the local `values`)
    /// or, without `j`, the current value of the target, times the local `times` and divided by
    /// the local `divide` when they are given.
    pub(crate) fn store_scaled(
        &mut self,
        target: Symbol,
        values: u32,
        j: Option<u32>,
        times: Option<u32>,
        divide: Option<u32>,
    ) {
        let (ptr, i) = self.address(target);
        self.sink().local_get(ptr);
        match j {
            Some(j) => {
                self.sink().local_get(values).f64_load(slot(j));
            }
            None => {
                self.sink().local_get(ptr).f64_load(slot(i));
            }
        }
        if let Some(t) = times {
            self.sink().local_get(t).f64_mul();
        }
        if let Some(d) = divide {
            self.sink().local_get(d).f64_div();
        }
        self.sink().f64_store(slot(i));
    }

    /// The pointer local and the index of a symbol of a state or a parameter.
    fn address(&self, target: Symbol) -> (u32, u32) {
        match target {
            Symbol::State(i) => (self.ctx.y, i),
            Symbol::Param(i) => (self.ctx.p, i),
            Symbol::Constant(k) => (self.ctx.p, self.module.constants + k),
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) => {
                unreachable!("{target:?} is no entity of the memory")
            }
        }
    }

    /// The error of this function (of the export `element`) above the limits.
    pub(crate) fn above_the_limits(&self, element: &str, what: &str) -> CompileError {
        let limits = self.module.limits;
        CompileError::Unsupported {
            feature: format!(
                "{ABOVE_THE_LIMITS} ({what}{} bytes, {} locals; at most {} bytes and {} locals)",
                self.size(),
                self.locals(),
                limits.bytes,
                limits.locals
            ),
            element: element.to_string(),
        }
    }

    /// Ends the function, checks it against the limits and adds it to the module; gives its
    /// position among the defined functions.
    pub(crate) fn finish(mut self) -> Result<u32, CompileError> {
        if !self.within_limits() {
            return Err(self.above_the_limits(self.export.unwrap_or("a helper function"), ""));
        }
        self.sink().end();
        let position = self.module.functions.len() as u32;
        self.module.functions.push(DefinedFunction {
            export: self.export,
            ty: self.ty,
            locals: self.n_locals,
            code: self.code,
            calls: self.calls,
        });
        Ok(position)
    }

    /// Reads a symbol.
    fn read(&mut self, s: Symbol) {
        if let Some(local) = self.rules.get(&s).copied() {
            self.sink().local_get(local);
            return;
        }
        match s {
            Symbol::Time => {
                let t = self.ctx.t;
                self.sink().local_get(t);
            }
            _ => {
                let (ptr, i) = self.address(s);
                self.sink().local_get(ptr).f64_load(slot(i));
            }
        }
    }

    /// Calls a function of mathlib.
    fn call(&mut self, f: MathFn) {
        let index = self.module.math(f);
        self.sink().call(index);
    }

    /// The code of an expression as an `f64`.
    pub(crate) fn f64(&mut self, e: &Expr) {
        match e {
            Expr::Number(x) => {
                self.sink().f64_const(f64c(*x));
            }
            Expr::Bool(b) => {
                self.sink().f64_const(f64c(if *b { 1.0 } else { 0.0 }));
            }
            Expr::Symbol(s) => self.read(*s),
            Expr::Apply { op, .. } if is_boolean(*op) => {
                self.bool(e);
                self.sink().f64_convert_i32_u();
            }
            Expr::Apply { op, args } => self.apply(*op, args),
            Expr::Piecewise { pieces, otherwise } => {
                for (value, condition) in pieces {
                    self.bool(condition);
                    self.sink().if_(BlockType::Result(ValType::F64));
                    self.f64(value);
                    self.sink().else_();
                }
                match otherwise {
                    Some(o) => self.f64(o),
                    None => {
                        self.sink().f64_const(f64c(f64::NAN));
                    }
                }
                for _ in pieces {
                    self.sink().end();
                }
            }
        }
    }

    /// The code of an expression as a boolean (`i32` 0 or 1).
    fn bool(&mut self, e: &Expr) {
        match e {
            Expr::Bool(b) => {
                self.sink().i32_const(i32::from(*b));
            }
            Expr::Apply { op, args } if is_boolean(*op) => match op {
                Op::And | Op::Or | Op::Xor => {
                    let (identity, op) = (i32::from(*op == Op::And), *op);
                    match args.split_first() {
                        None => {
                            self.sink().i32_const(identity);
                        }
                        Some((first, rest)) => {
                            self.bool(first);
                            for a in rest {
                                self.bool(a);
                                match op {
                                    Op::And => self.sink().i32_and(),
                                    Op::Or => self.sink().i32_or(),
                                    _ => self.sink().i32_xor(),
                                };
                            }
                        }
                    }
                }
                Op::Not => {
                    self.bool(&args[0]);
                    self.sink().i32_eqz();
                }
                Op::Implies => {
                    self.bool(&args[0]);
                    self.sink().i32_eqz();
                    self.bool(&args[1]);
                    self.sink().i32_or();
                }
                _ => self.relation(*op, args),
            },
            _ => {
                self.f64(e);
                self.sink().f64_abs().f64_const(f64c(0.0)).f64_gt();
            }
        }
    }

    /// An unordered relation over its arguments, a chain over more than two.
    fn relation(&mut self, op: Op, args: &[Expr]) {
        if args.len() < 2 {
            self.sink().i32_const(1);
            return;
        }
        if args.len() == 2 && op != Op::Eq {
            self.f64(&args[0]);
            self.f64(&args[1]);
            self.compare(op);
            return;
        }
        let temps: Vec<u32> = args
            .iter()
            .map(|a| {
                self.f64(a);
                let t = self.temp();
                self.sink().local_set(t);
                t
            })
            .collect();
        for (k, w) in temps.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if op == Op::Eq {
                // !(a < b || a > b)
                self.sink()
                    .local_get(a)
                    .local_get(b)
                    .f64_lt()
                    .local_get(a)
                    .local_get(b)
                    .f64_gt()
                    .i32_or()
                    .i32_eqz();
            } else {
                self.sink().local_get(a).local_get(b);
                self.compare(op);
            }
            if k > 0 {
                self.sink().i32_and();
            }
        }
        for t in temps.into_iter().rev() {
            self.release(t);
        }
    }

    /// An unordered comparison of the two numbers on the stack (not `eq`).
    fn compare(&mut self, op: Op) {
        let mut s = self.sink();
        match op {
            Op::Neq => s.f64_ne(),
            Op::Gt => s.f64_le().i32_eqz(),
            Op::Lt => s.f64_ge().i32_eqz(),
            Op::Geq => s.f64_lt().i32_eqz(),
            Op::Leq => s.f64_gt().i32_eqz(),
            _ => unreachable!("{op:?} is compared with temporaries"),
        };
    }

    /// `1 / x`, `x` on the stack.
    fn reciprocal_of_stack(&mut self) {
        let t = self.temp();
        self.sink()
            .local_set(t)
            .f64_const(f64c(1.0))
            .local_get(t)
            .f64_div();
        self.release(t);
    }

    /// The value of `e` in a temporary.
    fn eval_to_temp(&mut self, e: &Expr) -> u32 {
        self.f64(e);
        let t = self.temp();
        self.sink().local_set(t);
        t
    }

    /// `(log(1 + r) - log(1 - r)) * 0.5` of the temporary `r`.
    fn half_log_ratio(&mut self, r: u32) {
        self.sink().f64_const(f64c(1.0)).local_get(r).f64_add();
        self.call(MathFn::Log);
        self.sink().f64_const(f64c(1.0)).local_get(r).f64_sub();
        self.call(MathFn::Log);
        self.sink().f64_sub().f64_const(f64c(0.5)).f64_mul();
    }

    /// An arithmetic operator or a function.
    fn apply(&mut self, op: Op, args: &[Expr]) {
        match op {
            Op::Plus | Op::Times => match args.split_first() {
                None => {
                    let identity = if op == Op::Plus { 0.0 } else { 1.0 };
                    self.sink().f64_const(f64c(identity));
                }
                Some((first, rest)) => {
                    self.f64(first);
                    for a in rest {
                        self.f64(a);
                        if op == Op::Plus {
                            self.sink().f64_add();
                        } else {
                            self.sink().f64_mul();
                        }
                    }
                }
            },
            Op::Minus if args.len() == 1 => {
                self.sink().f64_const(f64c(0.0));
                self.f64(&args[0]);
                self.sink().f64_sub();
            }
            Op::Minus => {
                self.f64(&args[0]);
                self.f64(&args[1]);
                self.sink().f64_sub();
            }
            Op::Divide => {
                self.f64(&args[0]);
                self.f64(&args[1]);
                self.sink().f64_div();
            }
            Op::Power => {
                self.f64(&args[0]);
                self.f64(&args[1]);
                self.call(MathFn::Pow);
            }
            Op::Root if args.len() == 1 => {
                self.f64(&args[0]);
                self.sink().f64_const(f64c(0.5));
                self.call(MathFn::Pow);
            }
            Op::Root => {
                self.f64(&args[1]);
                self.sink().f64_const(f64c(1.0));
                self.f64(&args[0]);
                self.sink().f64_div();
                self.call(MathFn::Pow);
            }
            Op::Abs => {
                self.f64(&args[0]);
                self.sink().f64_abs();
            }
            Op::Floor => {
                self.f64(&args[0]);
                self.sink().f64_floor();
            }
            Op::Ceiling => {
                self.f64(&args[0]);
                self.sink().f64_ceil();
            }
            Op::Log if args.len() == 1 => {
                self.f64(&args[0]);
                self.call(MathFn::Log);
                self.sink().f64_const(f64c(10.0));
                self.call(MathFn::Log);
                self.sink().f64_div();
            }
            Op::Log => {
                self.f64(&args[1]);
                self.call(MathFn::Log);
                self.f64(&args[0]);
                self.call(MathFn::Log);
                self.sink().f64_div();
            }
            Op::Exp
            | Op::Ln
            | Op::Factorial
            | Op::Sin
            | Op::Cos
            | Op::Tan
            | Op::Sinh
            | Op::Cosh
            | Op::Tanh
            | Op::Arcsin
            | Op::Arccos
            | Op::Arctan => {
                let f = match op {
                    Op::Exp => MathFn::Exp,
                    Op::Ln => MathFn::Log,
                    Op::Factorial => MathFn::Factorial,
                    Op::Sin => MathFn::Sin,
                    Op::Cos => MathFn::Cos,
                    Op::Tan => MathFn::Tan,
                    Op::Sinh => MathFn::Sinh,
                    Op::Cosh => MathFn::Cosh,
                    Op::Tanh => MathFn::Tanh,
                    Op::Arcsin => MathFn::Asin,
                    Op::Arccos => MathFn::Acos,
                    _ => MathFn::Atan,
                };
                self.f64(&args[0]);
                self.call(f);
            }
            Op::Sec | Op::Csc | Op::Cot | Op::Sech | Op::Csch => {
                let f = match op {
                    Op::Sec => MathFn::Cos,
                    Op::Csc => MathFn::Sin,
                    Op::Cot => MathFn::Tan,
                    Op::Sech => MathFn::Cosh,
                    _ => MathFn::Sinh,
                };
                self.sink().f64_const(f64c(1.0));
                self.f64(&args[0]);
                self.call(f);
                self.sink().f64_div();
            }
            Op::Coth => {
                let x = self.eval_to_temp(&args[0]);
                self.sink().local_get(x);
                self.call(MathFn::Cosh);
                self.sink().local_get(x);
                self.call(MathFn::Sinh);
                self.sink().f64_div();
                self.release(x);
            }
            Op::Arcsec | Op::Arccsc => {
                self.f64(&args[0]);
                self.reciprocal_of_stack();
                self.call(if op == Op::Arcsec {
                    MathFn::Acos
                } else {
                    MathFn::Asin
                });
            }
            Op::Arccot => {
                let x = self.eval_to_temp(&args[0]);
                self.sink()
                    .local_get(x)
                    .f64_const(f64c(0.0))
                    .f64_eq()
                    .if_(BlockType::Result(ValType::F64))
                    .f64_const(f64c(HALF_PI))
                    .else_()
                    .f64_const(f64c(1.0))
                    .local_get(x)
                    .f64_div();
                self.call(MathFn::Atan);
                self.sink().end();
                self.release(x);
            }
            Op::Arcsinh | Op::Arccosh => {
                // log(x + sqrt(x * x +- 1))
                let x = self.eval_to_temp(&args[0]);
                self.sink()
                    .local_get(x)
                    .local_get(x)
                    .local_get(x)
                    .f64_mul()
                    .f64_const(f64c(1.0));
                if op == Op::Arcsinh {
                    self.sink().f64_add();
                } else {
                    self.sink().f64_sub();
                }
                self.sink().f64_sqrt().f64_add();
                self.call(MathFn::Log);
                self.release(x);
            }
            Op::Arctanh => {
                let x = self.eval_to_temp(&args[0]);
                self.half_log_ratio(x);
                self.release(x);
            }
            Op::Arccoth => {
                self.f64(&args[0]);
                self.reciprocal_of_stack();
                let r = self.temp();
                self.sink().local_set(r);
                self.half_log_ratio(r);
                self.release(r);
            }
            Op::Arcsech => {
                // log(r + sqrt(r - 1) * sqrt(r + 1)), r = 1 / x
                self.f64(&args[0]);
                self.reciprocal_of_stack();
                let r = self.temp();
                self.sink()
                    .local_tee(r)
                    .local_get(r)
                    .f64_const(f64c(1.0))
                    .f64_sub()
                    .f64_sqrt()
                    .local_get(r)
                    .f64_const(f64c(1.0))
                    .f64_add()
                    .f64_sqrt()
                    .f64_mul()
                    .f64_add();
                self.call(MathFn::Log);
                self.release(r);
            }
            Op::Arccsch => {
                // log(1 / x + sqrt(1 / (x * x) + 1))
                let x = self.eval_to_temp(&args[0]);
                self.sink()
                    .f64_const(f64c(1.0))
                    .local_get(x)
                    .f64_div()
                    .f64_const(f64c(1.0))
                    .local_get(x)
                    .local_get(x)
                    .f64_mul()
                    .f64_div()
                    .f64_const(f64c(1.0))
                    .f64_add()
                    .f64_sqrt()
                    .f64_add();
                self.call(MathFn::Log);
                self.release(x);
            }
            Op::Max | Op::Min => match args.split_first() {
                None => {
                    let empty = if op == Op::Max {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    };
                    self.sink().f64_const(f64c(empty));
                }
                Some((first, [])) => self.f64(first),
                Some((first, rest)) => {
                    let a = self.eval_to_temp(first);
                    for e in rest {
                        let b = self.eval_to_temp(e);
                        // a = (a > b) ? a : b, or (a < b) for min
                        let mut s = self.sink();
                        s.local_get(a).local_get(b).local_get(a).local_get(b);
                        if op == Op::Max {
                            s.f64_gt();
                        } else {
                            s.f64_lt();
                        }
                        s.select().local_set(a);
                        self.release(b);
                    }
                    self.sink().local_get(a);
                    self.release(a);
                }
            },
            Op::Rem => {
                self.f64(&args[0]);
                self.f64(&args[1]);
                self.call(MathFn::Fmod);
            }
            Op::Quotient => {
                self.f64(&args[0]);
                self.f64(&args[1]);
                self.sink().f64_div();
                let q = self.temp();
                self.sink()
                    .local_set(q)
                    .local_get(q)
                    .f64_const(f64c(QUOTIENT_LOW))
                    .f64_gt()
                    .local_get(q)
                    .f64_const(f64c(QUOTIENT_HIGH))
                    .f64_lt()
                    .i32_and()
                    .if_(BlockType::Result(ValType::F64))
                    .local_get(q)
                    .i32_trunc_f64_s()
                    .f64_convert_i32_s()
                    .else_()
                    .f64_const(f64c(QUOTIENT_INVALID))
                    .end();
                self.release(q);
            }
            _ => unreachable!("{op:?} is boolean"),
        }
    }
}

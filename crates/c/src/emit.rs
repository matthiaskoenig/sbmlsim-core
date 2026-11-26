//! The C of a model: the functions of the contract `model/2` as C functions of `t`, `y`, `p`
//! and their outputs.
//!
//! [`generate`] reads the SBML, builds the model representation and emits the programs of the
//! optimizer ([`optimize::plan`] with the common subexpressions, as `model.wasm` of
//! `sbmlsim_codegen::emit_optimized`) and the code as written where the WebAssembly target has
//! it as written (`model_defaults`, `model_event_apply`), statement for statement: a let is a
//! `const double` local, an output a store, a function of an event index a `switch`. Every
//! function is pure, it reads its arguments and writes its outputs, and the source has no
//! global variable, so a caller may run any number of them at once.
//!
//! An expression is the C of the meanings of `sbmlsim_ir::eval` in the order of the operations
//! of `model.wasm`: `+` and `*` the left folds of their arguments (C's `+` is left associative,
//! and the compiler neither reassociates nor contracts them under `-ffp-contract=off
//! -fno-fast-math`), the relations unordered (`a > b` is `!(a <= b)`, true where an operand is
//! NaN), a boolean as a number `1.0` or `0.0` and a number as a boolean `fabs(x) > 0.0`, a
//! `piecewise` the lazy chain of `?:`, NaN without `otherwise`. The operators which read an
//! argument more than once are static inline functions of the source (`sbmlsim_math_*`), so
//! every argument is evaluated once. The functions of the math are those of `<math.h>` and not
//! those of mathlib, so the results agree with the WebAssembly model to the last bits of the
//! math library, and the integration within its tolerances.
//!
//! A subexpression nested deeper than [`MAX_NESTING`] is computed into a local before its
//! statement (a `piecewise` into a lazy chain of `if`s), so no expression is nested deeper than
//! the parsers of the C compilers take (clang refuses 256 nested brackets).
//!
//! Refused ([`CError::Unsupported`]): a model which reads its history (`delay`), and a model
//! without an analytic Jacobian (`JacobianIr::ColoredDifference`).

use std::collections::HashMap;
use std::fmt::Write as _;

use sbmlsim_abi::{ModelDescription, ModelSizes};
use sbmlsim_ir::derive::{self, JacobianIr};
use sbmlsim_ir::eval::{HALF_PI, QUOTIENT_HIGH, QUOTIENT_INVALID, QUOTIENT_LOW, is_boolean};
use sbmlsim_ir::optimize::{self, Branches, Plan, Program, Statement};
use sbmlsim_ir::{EntityKind, Event, Expr, Model, Op, SpeciesValue, Symbol};

use crate::CError;

/// The deepest nesting of an expression in a statement; a deeper subexpression is computed
/// into a local before the statement.
pub const MAX_NESTING: usize = 48;

/// The C of a model and its description.
#[derive(Debug, Clone, PartialEq)]
pub struct CSource {
    /// The C, a translation unit which includes `<math.h>` and `<stdint.h>`.
    pub code: String,
    /// The description of the model, the one the WebAssembly target gives it without
    /// `model_jac_p`.
    pub description: ModelDescription,
}

/// The C of the SBML document `sbml` and its description; refuses a model the C target does
/// not support ([`CError::Unsupported`]) and a model the reader or the model representation
/// refuses ([`CError::Model`]).
pub fn generate(sbml: &str) -> Result<CSource, CError> {
    let doc = sbmlsim_sbml::read(sbml).map_err(|e| CError::Model(e.to_string()))?;
    let model = sbmlsim_ir::build(&doc).map_err(|e| CError::Model(e.to_string()))?;
    generate_model(&model)
}

/// The C of a model representation and its description (see [`generate`]).
pub fn generate_model(model: &Model) -> Result<CSource, CError> {
    let (plan, description) = prepare(model)?;
    let code = source(model, &plan, &description);
    Ok(CSource { code, description })
}

/// The programs of the optimizer of a model and its description; refuses a model which reads
/// its history and a model without an analytic Jacobian.
pub(crate) fn prepare(model: &Model) -> Result<(Plan, ModelDescription), CError> {
    let discontinuities = model
        .try_discontinuity_roots()
        .map_err(|e| CError::Model(e.to_string()))?;
    if reads_history(model, &discontinuities) {
        return Err(CError::Unsupported {
            feature: "a model which reads its history (delay)".to_string(),
        });
    }
    let jacobian = derive::jacobian(model);
    if let JacobianIr::ColoredDifference { reason, .. } = &jacobian {
        return Err(CError::Unsupported {
            feature: format!("a model without an analytic Jacobian: {reason}"),
        });
    }
    let plan = optimize::plan(model, &jacobian, None, &discontinuities, true);
    let description = sbmlsim_codegen::description(
        model,
        jacobian,
        None,
        &discontinuities,
        plan.n_constants(),
        0,
        None,
    );
    Ok((plan, description))
}

/// The prefix of the C functions of the model of the id `id`: `sbmlsim_model_<id>_`, every
/// character of the id which may not be in a C identifier `_`.
pub fn symbol_prefix(id: &str) -> String {
    let id: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("sbmlsim_model_{id}_")
}

/// Whether an expression of the model reads its history.
fn reads_history(model: &Model, discontinuities: &[Expr]) -> bool {
    fn reads(e: &Expr) -> bool {
        match e {
            Expr::Number(_) | Expr::Bool(_) | Expr::Symbol(_) => false,
            Expr::Apply { op, args } => {
                matches!(op, Op::Delay | Op::HistoryQuery) || args.iter().any(reads)
            }
            Expr::Piecewise { pieces, otherwise } => {
                pieces.iter().any(|(v, c)| reads(v) || reads(c))
                    || otherwise.as_deref().is_some_and(reads)
            }
        }
    }
    let events = model.events.iter().flat_map(|e| {
        std::iter::once(&e.trigger)
            .chain(&e.delay)
            .chain(&e.priority)
            .chain(e.assignments.iter().map(|a| &a.value))
    });
    model
        .rates
        .iter()
        .chain(model.assignment_rules.iter().map(|a| &a.expr))
        .chain(model.initial.iter().map(|a| &a.expr))
        .chain(model.selections.iter().map(|s| &s.expr))
        .chain(events)
        .chain(discontinuities)
        .any(reads)
}

/// The operators of the model representation which are more than an expression of
/// `<math.h>`, as `model.wasm` computes them, each argument evaluated once; guarded, so the
/// C of several models may be one translation unit.
const MATH: &str = r#"#ifndef SBMLSIM_C_MATH
#define SBMLSIM_C_MATH
static inline int sbmlsim_math_eq(double a, double b) { return !(a < b || a > b); }
static inline int sbmlsim_math_neq(double a, double b) { return a != b; }
static inline int sbmlsim_math_gt(double a, double b) { return !(a <= b); }
static inline int sbmlsim_math_lt(double a, double b) { return !(a >= b); }
static inline int sbmlsim_math_geq(double a, double b) { return !(a < b); }
static inline int sbmlsim_math_leq(double a, double b) { return !(a > b); }
static inline double sbmlsim_math_max(double a, double b) { return a > b ? a : b; }
static inline double sbmlsim_math_min(double a, double b) { return a < b ? a : b; }
static inline double sbmlsim_math_xorsign(double a, double b) {
    double left = (a < 0.0 - b) ? a : 0.0 - b;
    double right = (0.0 - a < b) ? 0.0 - a : b;
    return left > right ? left : right;
}
static inline double sbmlsim_math_quotient(double a, double b) {
    double q = a / b;
    return (q > QUOTIENT_LOW && q < QUOTIENT_HIGH) ? (double)(int32_t)q : QUOTIENT_INVALID;
}
static inline double sbmlsim_math_factorial(double x) {
    if (x != x) return NAN;
    if (x <= 0.0 || x == INFINITY) return 1.0;
    if (x >= 171.0) return INFINITY;
    double terms[172];
    int n = 0;
    for (double a = x; a > 0.0; a -= 1.0) terms[n++] = a;
    double r = 1.0;
    while (n > 0) r *= terms[--n];
    return r;
}
static inline double sbmlsim_math_coth(double x) { return cosh(x) / sinh(x); }
static inline double sbmlsim_math_arccot(double x) { return x == 0.0 ? HALF_PI : atan(1.0 / x); }
static inline double sbmlsim_math_arcsinh(double x) { return log(x + sqrt(x * x + 1.0)); }
static inline double sbmlsim_math_arccosh(double x) { return log(x + sqrt(x * x - 1.0)); }
static inline double sbmlsim_math_half_log_ratio(double r) {
    return (log(1.0 + r) - log(1.0 - r)) * 0.5;
}
static inline double sbmlsim_math_arcsech(double x) {
    double r = 1.0 / x;
    return log(r + sqrt(r - 1.0) * sqrt(r + 1.0));
}
static inline double sbmlsim_math_arccsch(double x) {
    return log(1.0 / x + sqrt(1.0 / (x * x) + 1.0));
}
#endif
"#;

/// The operators of [`MATH`] with the numbers of `sbmlsim_ir::eval`, each function declared
/// `qualifier` (`static inline`, or `CEED_QFUNCTION_HELPER` in a QFunction).
pub(crate) fn math(qualifier: &str) -> String {
    MATH.replace("static inline", qualifier)
        .replace("QUOTIENT_LOW", &number(QUOTIENT_LOW))
        .replace("QUOTIENT_HIGH", &number(QUOTIENT_HIGH))
        .replace("QUOTIENT_INVALID", &number(QUOTIENT_INVALID))
        .replace("HALF_PI", &number(HALF_PI))
}

/// A double as a C literal of its bits: the shortest decimal which reads back as it (a
/// negative one in parentheses), `NAN` and `INFINITY` of `<math.h>`.
pub(crate) fn number(x: f64) -> String {
    if x.is_nan() {
        "NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "INFINITY" } else { "(-INFINITY)" }.to_string()
    } else if x.is_sign_negative() {
        format!("({x:e})")
    } else {
        format!("{x:e}")
    }
}

/// How a function reads the time, the states and the parameters (the constants behind them).
pub(crate) struct Access {
    /// The time.
    pub time: String,
    /// The state of an index.
    pub state: Box<dyn Fn(u32) -> String>,
    /// The parameter of an index of `p`, the constants behind the parameters.
    pub param: Box<dyn Fn(u32) -> String>,
}

impl Access {
    /// `t`, `y[i]` and `p[i]`, the arguments of the functions of `model/2`.
    pub(crate) fn vectors() -> Access {
        Access {
            time: "t".to_string(),
            state: Box::new(|i| format!("y[{i}]")),
            param: Box::new(|i| format!("p[{i}]")),
        }
    }
}

/// The body of a C function being written: its lines, the locals of its lets, and how it reads
/// the symbols which are no lets.
pub(crate) struct Body {
    code: String,
    indent: usize,
    lets: HashMap<Symbol, String>,
    saved: Vec<HashMap<Symbol, String>>,
    locals: u32,
    n_params: u32,
    access: Access,
}

impl Body {
    /// An empty body of a model of `n_params` parameters (the first constant is `p[n_params]`).
    pub(crate) fn new(n_params: u32, access: Access) -> Body {
        Body {
            code: String::new(),
            indent: 1,
            lets: HashMap::new(),
            saved: Vec::new(),
            locals: 0,
            n_params,
            access,
        }
    }

    /// The lines written.
    pub(crate) fn finish(self) -> String {
        self.code
    }

    /// A line at the indentation of the block.
    pub(crate) fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.code.push_str("    ");
        }
        self.code.push_str(text);
        self.code.push('\n');
    }

    /// Opens a block after `header` (`if (...)`, `case 3:`); the lets of the block end with it.
    pub(crate) fn open(&mut self, header: &str) {
        self.line(&format!("{header} {{"));
        self.indent += 1;
        self.saved.push(self.lets.clone());
    }

    /// Closes the block, followed by `suffix`.
    pub(crate) fn close(&mut self, suffix: &str) {
        self.lets = self.saved.pop().expect("an open block");
        self.indent -= 1;
        self.line(&format!("}}{suffix}"));
    }

    /// A new name of a local.
    fn fresh(&mut self, stem: &str) -> String {
        self.locals += 1;
        format!("{stem}{}", self.locals)
    }

    /// The memory of a state, a parameter or a constant.
    pub(crate) fn address(&self, s: Symbol) -> String {
        match s {
            Symbol::State(i) => (self.access.state)(i),
            Symbol::Param(i) => (self.access.param)(i),
            Symbol::Constant(k) => (self.access.param)(self.n_params + k),
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) => {
                unreachable!("{s:?} is no entity of the memory")
            }
        }
    }

    /// The value of a symbol: its let, else the time or its memory.
    fn read(&self, s: Symbol) -> String {
        if let Some(local) = self.lets.get(&s) {
            return local.clone();
        }
        match s {
            Symbol::Time => self.access.time.clone(),
            Symbol::Derivative(_) | Symbol::Common(_) => unreachable!("{s:?} read before its let"),
            _ => self.address(s),
        }
    }

    /// `const double v = e;`, which later expressions read for `symbol`.
    pub(crate) fn let_(&mut self, symbol: Symbol, e: &Expr) {
        let local = self.keep(e);
        self.lets.insert(symbol, local);
    }

    /// The value of `e` in a new local.
    pub(crate) fn keep(&mut self, e: &Expr) -> String {
        let value = self.f64(e);
        self.keep_text(&value)
    }

    /// The value of a C expression in a new local.
    fn keep_text(&mut self, value: &str) -> String {
        let local = self.fresh("v");
        self.line(&format!("const double {local} = {value};"));
        local
    }

    /// `lvalue = e;`.
    pub(crate) fn store(&mut self, lvalue: &str, e: &Expr) {
        let value = self.f64(e);
        self.line(&format!("{lvalue} = {value};"));
    }

    /// Evaluates the assignment rules into locals, which later expressions read for their
    /// targets; the locals of an earlier evaluation are forgotten.
    pub(crate) fn assign_locals(&mut self, model: &Model) {
        self.lets.clear();
        for a in &model.assignment_rules {
            self.let_(a.target, &a.expr);
        }
    }

    /// Forgets the locals of the assignment rules: later expressions read the memory.
    fn forget_rules(&mut self) {
        self.lets.clear();
    }

    /// The statements of a program, the output `k` written by `output`.
    pub(crate) fn program(
        &mut self,
        program: &Program,
        mut output: impl FnMut(&mut Body, u32, &Expr),
    ) {
        let mut k = 0;
        for statement in &program.statements {
            match statement {
                Statement::Let(symbol, e) => self.let_(*symbol, e),
                Statement::Output(e) => {
                    output(self, k, e);
                    k += 1;
                }
            }
        }
    }

    /// The C of an expression as a double.
    pub(crate) fn f64(&mut self, e: &Expr) -> String {
        self.value(e, 0)
    }

    /// The C of an expression as a boolean (an `int`, 0 or 1).
    pub(crate) fn bool(&mut self, e: &Expr) -> String {
        self.boolean(e, 0)
    }

    /// An expression as a double at the nesting `level` of its statement.
    fn value(&mut self, e: &Expr, level: usize) -> String {
        if level > MAX_NESTING && !is_leaf(e) {
            return self.hoist_value(e);
        }
        match e {
            Expr::Number(x) => number(*x),
            Expr::Bool(b) => if *b { "1.0" } else { "0.0" }.to_string(),
            Expr::Symbol(s) => self.read(*s),
            Expr::Apply { op, .. } if is_boolean(*op) => {
                format!("((double){})", self.boolean(e, level + 1))
            }
            Expr::Apply { op, args } => self.apply(*op, args, level),
            Expr::Piecewise { pieces, otherwise } => {
                let mut text = String::from("(");
                let mut level = level;
                for (value, condition) in pieces {
                    level += 1;
                    let condition = self.boolean(condition, level);
                    let value = self.value(value, level);
                    write!(text, "{condition} ? {value} : ").unwrap();
                }
                match otherwise {
                    Some(o) => text.push_str(&self.value(o, level + 1)),
                    None => text.push_str("NAN"),
                }
                text.push(')');
                text
            }
        }
    }

    /// An expression as a double in a local before the statement: a `piecewise` as the lazy
    /// chain of its pieces.
    fn hoist_value(&mut self, e: &Expr) -> String {
        match e {
            Expr::Piecewise { pieces, otherwise } => {
                let local = self.fresh("v");
                self.line(&format!("double {local};"));
                self.open("do");
                for (value, condition) in pieces {
                    let condition = self.boolean(condition, 0);
                    self.open(&format!("if ({condition})"));
                    self.store(&local, value);
                    self.line("break;");
                    self.close("");
                }
                match otherwise {
                    Some(o) => self.store(&local, o),
                    None => self.line(&format!("{local} = NAN;")),
                }
                self.close(" while (0);");
                local
            }
            _ => {
                let value = self.value(e, 0);
                self.keep_text(&value)
            }
        }
    }

    /// An expression as a boolean at the nesting `level` of its statement.
    fn boolean(&mut self, e: &Expr, level: usize) -> String {
        if level > MAX_NESTING && !is_leaf(e) {
            let value = self.boolean(e, 0);
            let local = self.fresh("b");
            self.line(&format!("const int {local} = {value};"));
            return local;
        }
        let next = level + 1;
        match e {
            Expr::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            Expr::Apply { op, args } if is_boolean(*op) => match op {
                Op::And | Op::Or | Op::Xor => {
                    let (identity, join) = match op {
                        Op::And => ("1", " && "),
                        Op::Or => ("0", " || "),
                        _ => ("0", " ^ "),
                    };
                    if args.is_empty() {
                        return identity.to_string();
                    }
                    let parts: Vec<String> = args.iter().map(|a| self.boolean(a, next)).collect();
                    format!("({})", parts.join(join))
                }
                Op::Not => format!("(!{})", self.boolean(&args[0], next)),
                Op::Implies => {
                    let a = self.boolean(&args[0], next);
                    let b = self.boolean(&args[1], next);
                    format!("(!{a} || {b})")
                }
                _ => self.relation(*op, args, level),
            },
            _ => format!("(fabs({}) > 0.0)", self.value(e, next)),
        }
    }

    /// An unordered relation over its arguments, a chain over more than two (each argument
    /// evaluated once, into a local).
    fn relation(&mut self, op: Op, args: &[Expr], level: usize) -> String {
        let name = match op {
            Op::Eq => "eq",
            Op::Neq => "neq",
            Op::Gt => "gt",
            Op::Lt => "lt",
            Op::Geq => "geq",
            Op::Leq => "leq",
            _ => unreachable!("{op:?} is no relation"),
        };
        match args {
            [] | [_] => "1".to_string(),
            [a, b] => {
                let a = self.value(a, level + 1);
                let b = self.value(b, level + 1);
                format!("sbmlsim_math_{name}({a}, {b})")
            }
            _ => {
                let locals: Vec<String> = args.iter().map(|a| self.keep(a)).collect();
                let pairs: Vec<String> = locals
                    .windows(2)
                    .map(|w| format!("sbmlsim_math_{name}({}, {})", w[0], w[1]))
                    .collect();
                format!("({})", pairs.join(" && "))
            }
        }
    }

    /// A call of a function of one argument.
    fn call(&mut self, function: &str, arg: &Expr, level: usize) -> String {
        format!("{function}({})", self.value(arg, level + 1))
    }

    /// A left fold of `function` of two arguments; every [`MAX_NESTING`] arguments the
    /// accumulated value goes into a local.
    fn fold(&mut self, function: &str, first: &Expr, rest: &[Expr], level: usize) -> String {
        let mut acc = self.value(first, level + 1);
        for (k, e) in rest.iter().enumerate() {
            if k > 0 && k % MAX_NESTING == 0 {
                acc = self.keep_text(&acc);
            }
            let b = self.value(e, level + 1 + k % MAX_NESTING);
            acc = format!("{function}({acc}, {b})");
        }
        acc
    }

    /// An arithmetic operator or a function.
    fn apply(&mut self, op: Op, args: &[Expr], level: usize) -> String {
        let next = level + 1;
        let one = number(1.0);
        match op {
            Op::Delay | Op::HistoryQuery => unreachable!("a model which reads its history"),
            Op::Plus | Op::Times => match args {
                [] => number(if op == Op::Plus { 0.0 } else { 1.0 }),
                [a] => self.value(a, level),
                _ => {
                    let join = if op == Op::Plus { " + " } else { " * " };
                    let parts: Vec<String> = args.iter().map(|a| self.value(a, next)).collect();
                    format!("({})", parts.join(join))
                }
            },
            Op::Minus if args.len() == 1 => {
                format!("({} - {})", number(0.0), self.value(&args[0], next))
            }
            Op::Minus | Op::Divide => {
                let a = self.value(&args[0], next);
                let b = self.value(&args[1], next);
                let operator = if op == Op::Minus { '-' } else { '/' };
                format!("({a} {operator} {b})")
            }
            Op::Power => {
                let a = self.value(&args[0], next);
                let b = self.value(&args[1], next);
                format!("pow({a}, {b})")
            }
            Op::Root if args.len() == 1 => {
                format!("pow({}, {})", self.value(&args[0], next), number(0.5))
            }
            Op::Root => {
                let x = self.value(&args[1], next);
                let n = self.value(&args[0], next + 1);
                format!("pow({x}, ({one} / {n}))")
            }
            Op::Abs => self.call("fabs", &args[0], level),
            Op::Floor => self.call("floor", &args[0], level),
            Op::Ceiling => self.call("ceil", &args[0], level),
            Op::Log if args.len() == 1 => {
                let x = self.value(&args[0], next);
                format!("(log({x}) / log({}))", number(10.0))
            }
            Op::Log => {
                let x = self.value(&args[1], next);
                let b = self.value(&args[0], next);
                format!("(log({x}) / log({b}))")
            }
            Op::Exp => self.call("exp", &args[0], level),
            Op::Ln => self.call("log", &args[0], level),
            Op::Factorial => self.call("sbmlsim_math_factorial", &args[0], level),
            Op::Sin => self.call("sin", &args[0], level),
            Op::Cos => self.call("cos", &args[0], level),
            Op::Tan => self.call("tan", &args[0], level),
            Op::Sinh => self.call("sinh", &args[0], level),
            Op::Cosh => self.call("cosh", &args[0], level),
            Op::Tanh => self.call("tanh", &args[0], level),
            Op::Arcsin => self.call("asin", &args[0], level),
            Op::Arccos => self.call("acos", &args[0], level),
            Op::Arctan => self.call("atan", &args[0], level),
            Op::Sec | Op::Csc | Op::Cot | Op::Sech | Op::Csch => {
                let function = match op {
                    Op::Sec => "cos",
                    Op::Csc => "sin",
                    Op::Cot => "tan",
                    Op::Sech => "cosh",
                    _ => "sinh",
                };
                format!("({one} / {})", self.call(function, &args[0], next))
            }
            Op::Coth => self.call("sbmlsim_math_coth", &args[0], level),
            Op::Arcsec | Op::Arccsc => {
                let function = if op == Op::Arcsec { "acos" } else { "asin" };
                let x = self.value(&args[0], next + 1);
                format!("{function}(({one} / {x}))")
            }
            Op::Arccot => self.call("sbmlsim_math_arccot", &args[0], level),
            Op::Arcsinh => self.call("sbmlsim_math_arcsinh", &args[0], level),
            Op::Arccosh => self.call("sbmlsim_math_arccosh", &args[0], level),
            Op::Arctanh => self.call("sbmlsim_math_half_log_ratio", &args[0], level),
            Op::Arccoth => {
                let x = self.value(&args[0], next + 1);
                format!("sbmlsim_math_half_log_ratio(({one} / {x}))")
            }
            Op::Arcsech => self.call("sbmlsim_math_arcsech", &args[0], level),
            Op::Arccsch => self.call("sbmlsim_math_arccsch", &args[0], level),
            Op::XorSign => match args.split_first() {
                None => number(-1.0),
                Some((first, rest)) => self.fold("sbmlsim_math_xorsign", first, rest, level),
            },
            Op::Max | Op::Min => match args.split_first() {
                None => number(if op == Op::Max {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }),
                Some((first, [])) => self.value(first, level),
                Some((first, rest)) => {
                    let function = if op == Op::Max {
                        "sbmlsim_math_max"
                    } else {
                        "sbmlsim_math_min"
                    };
                    self.fold(function, first, rest, level)
                }
            },
            Op::Rem => {
                let a = self.value(&args[0], next);
                let b = self.value(&args[1], next);
                format!("fmod({a}, {b})")
            }
            Op::Quotient => {
                let a = self.value(&args[0], next);
                let b = self.value(&args[1], next);
                format!("sbmlsim_math_quotient({a}, {b})")
            }
            _ => unreachable!("{op:?} is boolean"),
        }
    }
}

/// Whether an expression is a literal or a symbol.
fn is_leaf(e: &Expr) -> bool {
    matches!(e, Expr::Number(_) | Expr::Bool(_) | Expr::Symbol(_))
}

/// The translation unit being written: its code, the prefix of its functions, the number of
/// parameters of the model and the qualifier of its functions (empty, or
/// `CEED_QFUNCTION_HELPER ` in a QFunction).
pub(crate) struct Unit {
    pub code: String,
    pub prefix: String,
    pub n_params: u32,
    pub qualifier: &'static str,
}

impl Unit {
    /// Adds the function `<qualifier><result> <prefix><name><arguments>` whose body `body`
    /// writes.
    pub(crate) fn function(
        &mut self,
        result: &str,
        name: &str,
        arguments: &str,
        body: impl FnOnce(&mut Body),
    ) {
        let mut b = Body::new(self.n_params, Access::vectors());
        body(&mut b);
        let (qualifier, prefix) = (self.qualifier, &self.prefix);
        writeln!(
            self.code,
            "{qualifier}{result} {prefix}{name}({arguments}) {{"
        )
        .unwrap();
        self.code.push_str(&b.finish());
        self.code.push_str("}\n\n");
    }
}

/// The arguments of the functions of `(t, y, p, out)`.
const VECTORS: &str =
    "double t, const double *restrict y, const double *restrict p, double *restrict out";
/// The arguments of the functions of an event index `(i, t, y, p)`.
const INDEXED: &str = "uint32_t i, double t, const double *restrict y, const double *restrict p";

/// The C of the model: the functions of `model/2` from the programs of `plan`.
fn source(model: &Model, plan: &Plan, description: &ModelDescription) -> String {
    let mut unit = Unit {
        code: String::new(),
        prefix: symbol_prefix(&description.id),
        n_params: model.parameters.len() as u32,
        qualifier: "",
    };
    writeln!(
        unit.code,
        "/* The model `{}` of sbmlsim-core (sbmlsim-c {}): the functions of the contract {}. */",
        description.id.replace("*/", "* /"),
        env!("CARGO_PKG_VERSION"),
        sbmlsim_abi::MODEL_CONTRACT
    )
    .unwrap();
    unit.code
        .push_str("#include <math.h>\n#include <stdint.h>\n\n");
    unit.code.push_str(&math("static inline"));
    unit.code.push('\n');

    // defaults(p, y): the default of every parameter and state
    unit.function(
        "void",
        "defaults",
        "double *restrict p, double *restrict y",
        |b| {
            for (i, e) in model.parameters.iter().enumerate() {
                let lvalue = b.address(Symbol::Param(i as u32));
                b.line(&format!("{lvalue} = {};", number(e.default)));
            }
            for (i, e) in model.states.iter().enumerate() {
                let lvalue = b.address(Symbol::State(i as u32));
                b.line(&format!("{lvalue} = {};", number(e.default)));
            }
        },
    );

    // init(t, p, y, fixed): each initial evaluation unless its target is fixed, then the
    // constants
    let constants = |b: &mut Body| {
        b.program(&plan.constants, |b, k, e| {
            let lvalue = b.address(Symbol::Constant(k));
            b.store(&lvalue, e);
        });
    };
    unit.function(
        "void",
        "init",
        "double t, double *restrict p, double *restrict y, const uint8_t *restrict fixed",
        |b| {
            for (a, program) in model.initial.iter().zip(&plan.init) {
                let k = match a.target {
                    Symbol::Param(i) => i,
                    Symbol::State(i) => model.parameters.len() as u32 + i,
                    other => unreachable!("{other:?} is no target"),
                };
                b.open(&format!("if (fixed[{k}] == 0)"));
                let lvalue = b.address(a.target);
                b.program(program, |b, _, e| b.store(&lvalue, e));
                b.close("");
            }
            constants(b);
        },
    );

    // rhs, jac, roots of (t, y, p, out)
    let jac = plan.jac.as_ref().expect("an analytic Jacobian");
    for (name, program) in [("rhs", &plan.rhs), ("jac", jac), ("roots", &plan.roots)] {
        unit.function("void", name, VECTORS, |b| {
            b.program(program, |b, k, e| b.store(&format!("out[{k}]"), e));
        });
    }

    // the functions of an event index
    indexed(
        &mut unit,
        "int",
        "trigger",
        INDEXED,
        &plan.trigger,
        "0",
        |b, e| {
            let value = b.bool(e);
            b.line(&format!("return {value};"));
        },
    );
    for (name, part) in [("priority", &plan.priority), ("delay", &plan.delay)] {
        indexed(&mut unit, "double", name, INDEXED, part, "0.0", |b, e| {
            let value = b.f64(e);
            b.line(&format!("return {value};"));
        });
    }
    let arguments = format!("{INDEXED}, double *restrict out");
    unit.function("void", "event_values", &arguments, |b| {
        b.program(&plan.event_values.prefix, |_, _, _| unreachable!("a let"));
        b.open("switch (i)");
        for (k, branch) in plan.event_values.branches.iter().enumerate() {
            let branch = branch.as_ref().expect("every event has its values");
            b.open(&format!("case {k}:"));
            b.program(branch, |b, j, e| b.store(&format!("out[{j}]"), e));
            b.line("return;");
            b.close("");
        }
        b.close("");
    });

    // event_apply(i, t, values, y, p), as written
    unit.function(
        "void",
        "event_apply",
        "uint32_t i, double t, const double *restrict values, double *restrict y, \
         double *restrict p",
        |b| {
            b.open("switch (i)");
            for (k, e) in model.events.iter().enumerate() {
                b.open(&format!("case {k}:"));
                event_apply(model, e, b);
                b.line("return;");
                b.close("");
            }
            b.close("");
        },
    );

    // selections(t, y, p, mask, out): the selections of the mask
    unit.function(
        "void",
        "selections",
        "double t, const double *restrict y, const double *restrict p, \
         const uint8_t *restrict mask, double *restrict out",
        |b| {
            b.program(&plan.selections, |b, k, e| {
                b.open(&format!("if (mask[{k}] != 0)"));
                b.store(&format!("out[{k}]"), e);
                b.close("");
            });
        },
    );

    // assign(t, y, p): every rule stored before the next one reads the memory, then the
    // constants
    unit.function(
        "void",
        "assign",
        "double t, double *restrict y, double *restrict p",
        |b| {
            let rules = &model.assignment_rules;
            b.program(&plan.assign, |b, k, e| {
                let lvalue = b.address(rules[k as usize].target);
                b.store(&lvalue, e);
            });
            constants(b);
        },
    );

    // sizes(out) and event_sizes(out): the sizes of the description
    let sizes = ModelSizes::of(description);
    unit.function("void", "sizes", "uint32_t *out", |b| {
        for (k, x) in sizes.words().iter().enumerate() {
            b.line(&format!("out[{k}] = {x};"));
        }
    });
    unit.function("void", "event_sizes", "uint32_t *out", |b| {
        for (k, x) in sizes.assignments.iter().enumerate() {
            b.line(&format!("out[{k}] = {x};"));
        }
    });
    unit.code
}

/// A function of an event index which returns a value: the lets of the prefix, then a case
/// per event which has the part, which returns its value (`end`); `tail` without.
fn indexed(
    unit: &mut Unit,
    result: &str,
    name: &str,
    arguments: &str,
    part: &Branches,
    tail: &str,
    end: impl Fn(&mut Body, &Expr),
) {
    unit.function(result, name, arguments, |b| {
        b.program(&part.prefix, |_, _, _| unreachable!("a let"));
        b.open("switch (i)");
        for (k, branch) in part.branches.iter().enumerate() {
            let Some(branch) = branch else { continue };
            b.open(&format!("case {k}:"));
            b.program(branch, |b, _, e| end(b, e));
            b.close("");
        }
        b.close("");
        b.line(&format!("return {tail};"));
    });
}

/// The body of `event_apply` for the event `e`, as `model_event_apply` of `model.wasm`
/// (`sbmlsim_codegen`): the assignments in document order, those of compartments last; a
/// species of a concentration gets the amount of its value times its compartment as it is at
/// its turn; a species which stores its concentration is rescaled by its compartment before
/// over after the event, when the compartment changed. The values are in `values`.
fn event_apply(model: &Model, e: &Event, f: &mut Body) {
    let is_compartment = |s: Symbol| matches!(model.entity(s).kind, EntityKind::Compartment);
    let is_rule_target = |s: Symbol| model.assignment_rules.iter().any(|a| a.target == s);
    // the states which store a concentration, with their compartments
    let stored: Vec<(Symbol, Symbol)> = (0..model.states.len())
        .map(|i| Symbol::State(i as u32))
        .filter_map(|s| match &model.entity(s).kind {
            EntityKind::Species {
                compartment,
                value: SpeciesValue::Concentration,
                ..
            } => Some((s, *compartment)),
            _ => None,
        })
        .collect();
    let mut compartments: Vec<Symbol> = Vec::new();
    for (_, c) in &stored {
        if !compartments.contains(c) {
            compartments.push(*c);
        }
    }
    let lookup = |list: &[(Symbol, String)], c: Symbol| {
        list.iter()
            .find(|(s, _)| *s == c)
            .map(|(_, l)| l.clone())
            .expect("a compartment of the list")
    };
    // the compartments before the assignments
    let mut rules_current = false;
    let before: Vec<(Symbol, String)> = if compartments.is_empty() {
        Vec::new()
    } else {
        f.assign_locals(model);
        rules_current = true;
        compartments
            .iter()
            .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
            .collect()
    };
    // the assignments in document order, the compartments last; the amounts of the species
    // which store their concentration are kept in locals until the compartments are known
    let mut amounts: Vec<(Symbol, usize, String)> = Vec::new();
    let order = e
        .assignments
        .iter()
        .enumerate()
        .filter(|(_, a)| !is_compartment(a.target))
        .chain(
            e.assignments
                .iter()
                .enumerate()
                .filter(|(_, a)| is_compartment(a.target)),
        );
    for (j, a) in order {
        let target = f.address(a.target);
        let Some(c) = a.compartment else {
            f.line(&format!("{target} = values[{j}];"));
            rules_current = false;
            continue;
        };
        // the compartment at the turn of this assignment
        if is_rule_target(c) && !rules_current {
            f.assign_locals(model);
            rules_current = true;
        }
        let now = f.keep(&Expr::Symbol(c));
        if stored.iter().any(|(s, _)| *s == a.target) {
            amounts.push((a.target, j, now));
        } else {
            f.line(&format!("{target} = values[{j}] * {now};"));
            rules_current = false;
        }
    }
    if stored.is_empty() {
        f.forget_rules();
        return;
    }
    // the compartments after the assignments
    f.assign_locals(model);
    let after: Vec<(Symbol, String)> = compartments
        .iter()
        .map(|c| (*c, f.keep(&Expr::Symbol(*c))))
        .collect();
    f.forget_rules();
    for (s, c) in &stored {
        let to = lookup(&after, *c);
        let target = f.address(*s);
        match amounts.iter().find(|(t, _, _)| t == s) {
            // value * now / after, the value itself when the compartment did not change
            Some((_, j, now)) => {
                f.open(&format!("if ({now} != {to})"));
                f.line(&format!("{target} = values[{j}] * {now} / {to};"));
                f.close("");
                f.open("else");
                f.line(&format!("{target} = values[{j}];"));
                f.close("");
            }
            // the amount is kept: y * before / after, when the compartment changed
            None => {
                let from = lookup(&before, *c);
                f.open(&format!("if ({from} != {to})"));
                f.line(&format!("{target} = {target} * {from} / {to};"));
                f.close("");
            }
        }
    }
}

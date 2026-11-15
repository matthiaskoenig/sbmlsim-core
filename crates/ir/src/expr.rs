//! Expressions with resolved symbols, and their construction from the MathML of the reader.
//!
//! An [`Expr`] mirrors the MathML it comes from: the operators keep their arguments in the
//! written order, nothing is folded, reassociated or simplified (`q / 49` stays one division).
//! What the construction changes is what SBML defines: an identifier becomes the [`Symbol`] of
//! its entity (a species of a concentration the quotient of its amount by its compartment), a
//! local parameter its value, a reaction id its rate, a call of a function definition its body
//! with the argument expressions for the bound variables, `rateOf(x)` the rate of `x`, and the
//! constants `pi`, `exponentiale`, `infinity`, `notanumber` and the csymbol `avogadro` their
//! doubles.

use std::collections::HashMap;

use sbmlsim_sbml::math::{Node, Op as MathOp};

use crate::CompileError;

/// The value of the csymbol `avogadro`, as libroadrunner 2.11 has it (and sbmlsim's
/// `sbml_math.AVOGADRO`), not the CODATA 2018 value `6.02214076e23`.
pub const AVOGADRO: f64 = 6.02214179e23;

/// A value an expression reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Symbol {
    /// The state of this index in `y`.
    State(u32),
    /// The parameter of this index in `p`.
    Param(u32),
    /// The csymbol `time`.
    Time,
    /// The derivative of this index of a Jacobian (`JacobianIr::Analytic::derivatives` of
    /// [`crate::derive`]), only in the expressions of a Jacobian.
    Derivative(u32),
    /// The common subexpression of this index of a function, only in the programs of the
    /// optimizer ([`crate::optimize::Program`]).
    Common(u32),
}

/// An operator, those of the MathML of SBML Level 3 Version 2 without the csymbols `delay` and
/// `rateOf` and the calls of function definitions, which the construction resolves.
///
/// The arity is the one of the MathML: `Minus` has one argument (negation) or two, `Log` has
/// `[x]` (base 10) or `[base, x]`, `Root` has `[x]` (degree 2) or `[degree, x]`, the n-ary
/// operators any number in the written order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Op {
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Root,
    Abs,
    Exp,
    Ln,
    Log,
    Floor,
    Ceiling,
    Factorial,
    Sin,
    Cos,
    Tan,
    Sec,
    Csc,
    Cot,
    Sinh,
    Cosh,
    Tanh,
    Sech,
    Csch,
    Coth,
    Arcsin,
    Arccos,
    Arctan,
    Arcsec,
    Arccsc,
    Arccot,
    Arcsinh,
    Arccosh,
    Arctanh,
    Arcsech,
    Arccsch,
    Arccoth,
    Eq,
    Neq,
    Gt,
    Lt,
    Geq,
    Leq,
    And,
    Or,
    Xor,
    Not,
    Implies,
    Max,
    Min,
    Rem,
    Quotient,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number, the double of its text.
    Number(f64),
    /// `true` or `false`.
    Bool(bool),
    /// The value of a symbol.
    Symbol(Symbol),
    /// An operator applied to its arguments.
    Apply {
        /// The operator.
        op: Op,
        /// The arguments, in the written order.
        args: Vec<Expr>,
    },
    /// The first value whose condition is true, else `otherwise` (NaN when absent).
    Piecewise {
        /// The pieces, `(value, condition)` in the written order.
        pieces: Vec<(Expr, Expr)>,
        /// The value when no condition is true.
        otherwise: Option<Box<Expr>>,
    },
}

impl Expr {
    /// `op` applied to `args`.
    pub fn apply(op: Op, args: Vec<Expr>) -> Expr {
        Expr::Apply { op, args }
    }

    /// Calls `f` on every symbol the expression reads, in the written order.
    pub fn visit_symbols(&self, f: &mut impl FnMut(Symbol)) {
        match self {
            Expr::Number(_) | Expr::Bool(_) => {}
            Expr::Symbol(s) => f(*s),
            Expr::Apply { args, .. } => args.iter().for_each(|a| a.visit_symbols(f)),
            Expr::Piecewise { pieces, otherwise } => {
                for (value, condition) in pieces {
                    value.visit_symbols(f);
                    condition.visit_symbols(f);
                }
                if let Some(o) = otherwise {
                    o.visit_symbols(f);
                }
            }
        }
    }

    /// The symbols the expression reads, each once, in the order of their first appearance.
    pub fn symbols(&self) -> Vec<Symbol> {
        let mut out = Vec::new();
        self.visit_symbols(&mut |s| {
            if !out.contains(&s) {
                out.push(s);
            }
        });
        out
    }
}

/// What the construction of an expression needs to know of the model.
pub(crate) trait Context {
    /// The value of a model identifier as the math sees it (not a local parameter or a bound
    /// variable), `element` the element of the math for an error.
    fn identifier(&mut self, id: &str, element: &str) -> Result<Expr, CompileError>;
    /// The rate of the value of `id` as the math sees it.
    fn rate_of(&mut self, id: &str, element: &str) -> Result<Expr, CompileError>;
    /// The math of the function definition `id`: `None` when there is no such definition,
    /// `Some(None)` when it has no math.
    fn function(&self, id: &str) -> Option<Option<Node>>;
}

/// Where an identifier of a math is resolved.
pub(crate) struct Scope<'s> {
    /// The element of the math, named in an error.
    pub element: &'s str,
    /// The local parameters of a kinetic law, by id, their values.
    pub locals: Option<&'s HashMap<String, f64>>,
    /// The bound variables of an inlined function, by id, their argument expressions.
    pub bvars: Option<&'s HashMap<String, Expr>>,
    /// The function definitions being inlined, outermost first.
    pub inlining: &'s [String],
}

impl<'s> Scope<'s> {
    /// The scope of a math outside a kinetic law.
    pub fn global(element: &'s str) -> Scope<'s> {
        Scope {
            element,
            locals: None,
            bvars: None,
            inlining: &[],
        }
    }
}

fn unsupported(feature: &str, element: &str) -> CompileError {
    CompileError::Unsupported {
        feature: feature.to_string(),
        element: element.to_string(),
    }
}

/// The expression of a math node in a scope.
pub(crate) fn build(
    node: &Node,
    cx: &mut dyn Context,
    scope: &Scope,
) -> Result<Expr, CompileError> {
    Ok(match node {
        Node::Number(x) => Expr::Number(*x),
        Node::True => Expr::Bool(true),
        Node::False => Expr::Bool(false),
        Node::Pi => Expr::Number(std::f64::consts::PI),
        Node::ExponentialE => Expr::Number(std::f64::consts::E),
        Node::Infinity => Expr::Number(f64::INFINITY),
        Node::NotANumber => Expr::Number(f64::NAN),
        Node::CsymbolTime => Expr::Symbol(Symbol::Time),
        Node::CsymbolAvogadro => Expr::Number(AVOGADRO),
        Node::CsymbolDelay => return Err(unsupported("delay csymbol", scope.element)),
        // libroadrunner: "Unknown ASTNode type of 9999"
        Node::Empty => return Err(unsupported("math without content", scope.element)),
        Node::CsymbolRateOf => {
            return Err(CompileError::Invalid(format!(
                "the csymbol rateOf is not applied to an argument in {}",
                scope.element
            )));
        }
        Node::Symbol(id) => {
            if let Some(e) = scope.bvars.and_then(|b| b.get(id)) {
                e.clone()
            } else if let Some(v) = scope.locals.and_then(|l| l.get(id)) {
                Expr::Number(*v)
            } else {
                cx.identifier(id, scope.element)?
            }
        }
        Node::Lambda { .. } => {
            return Err(CompileError::Invalid(format!(
                "a lambda outside a function definition in {}",
                scope.element
            )));
        }
        Node::Piecewise { pieces, otherwise } => Expr::Piecewise {
            pieces: pieces
                .iter()
                .map(|(value, condition)| {
                    Ok((build(value, cx, scope)?, build(condition, cx, scope)?))
                })
                .collect::<Result<_, CompileError>>()?,
            otherwise: otherwise
                .as_ref()
                .map(|o| build(o, cx, scope).map(Box::new))
                .transpose()?,
        },
        Node::Apply { op, args } => match op {
            MathOp::Delay => return Err(unsupported("delay csymbol", scope.element)),
            MathOp::RateOf => match args.as_slice() {
                [Node::Symbol(id)] => {
                    if scope.bvars.is_some_and(|b| b.contains_key(id)) {
                        return Err(unsupported(
                            "rateOf of a bound variable of a function definition",
                            scope.element,
                        ));
                    }
                    if scope.locals.is_some_and(|l| l.contains_key(id)) {
                        // a local parameter is a constant
                        Expr::Number(0.0)
                    } else {
                        cx.rate_of(id, scope.element)?
                    }
                }
                _ => {
                    return Err(CompileError::Invalid(format!(
                        "rateOf of something other than an identifier in {}",
                        scope.element
                    )));
                }
            },
            MathOp::Function(id) => inline(id, args, cx, scope)?,
            other => Expr::Apply {
                op: operator(other),
                args: args
                    .iter()
                    .map(|a| build(a, cx, scope))
                    .collect::<Result<_, _>>()?,
            },
        },
    })
}

/// The body of the function definition `id` with the argument expressions for its bound
/// variables.
fn inline(
    id: &str,
    args: &[Node],
    cx: &mut dyn Context,
    scope: &Scope,
) -> Result<Expr, CompileError> {
    let element = scope.element;
    let math = match cx.function(id) {
        None => {
            return Err(CompileError::Invalid(format!(
                "{id} is called in {element} but is no function definition"
            )));
        }
        // libroadrunner crashes on a call of a function definition without math and refuses one
        // whose math is empty ("math element of function definition must be a lambda")
        Some(None | Some(Node::Empty)) => {
            return Err(unsupported("function definition without math", id));
        }
        Some(Some(math)) => math,
    };
    if scope.inlining.iter().any(|f| f == id) {
        return Err(CompileError::Invalid(format!(
            "the function definition {id} calls itself ({} -> {id})",
            scope.inlining.join(" -> ")
        )));
    }
    let Node::Lambda { params, body } = math else {
        return Err(CompileError::Invalid(format!(
            "the math of the function definition {id} is no lambda"
        )));
    };
    if params.len() != args.len() {
        return Err(CompileError::Invalid(format!(
            "the function definition {id} takes {} arguments, {} are given in {element}",
            params.len(),
            args.len()
        )));
    }
    let mut bvars = HashMap::new();
    for (param, arg) in params.iter().zip(args) {
        bvars.insert(param.clone(), build(arg, cx, scope)?);
    }
    let mut inlining = scope.inlining.to_vec();
    inlining.push(id.to_string());
    // the body sees its bound variables and the model, not the local parameters of the caller
    build(
        &body,
        cx,
        &Scope {
            element,
            locals: None,
            bvars: Some(&bvars),
            inlining: &inlining,
        },
    )
}

fn operator(op: &MathOp) -> Op {
    match op {
        MathOp::Plus => Op::Plus,
        MathOp::Minus => Op::Minus,
        MathOp::Times => Op::Times,
        MathOp::Divide => Op::Divide,
        MathOp::Power => Op::Power,
        MathOp::Root => Op::Root,
        MathOp::Abs => Op::Abs,
        MathOp::Exp => Op::Exp,
        MathOp::Ln => Op::Ln,
        MathOp::Log => Op::Log,
        MathOp::Floor => Op::Floor,
        MathOp::Ceiling => Op::Ceiling,
        MathOp::Factorial => Op::Factorial,
        MathOp::Sin => Op::Sin,
        MathOp::Cos => Op::Cos,
        MathOp::Tan => Op::Tan,
        MathOp::Sec => Op::Sec,
        MathOp::Csc => Op::Csc,
        MathOp::Cot => Op::Cot,
        MathOp::Sinh => Op::Sinh,
        MathOp::Cosh => Op::Cosh,
        MathOp::Tanh => Op::Tanh,
        MathOp::Sech => Op::Sech,
        MathOp::Csch => Op::Csch,
        MathOp::Coth => Op::Coth,
        MathOp::Arcsin => Op::Arcsin,
        MathOp::Arccos => Op::Arccos,
        MathOp::Arctan => Op::Arctan,
        MathOp::Arcsec => Op::Arcsec,
        MathOp::Arccsc => Op::Arccsc,
        MathOp::Arccot => Op::Arccot,
        MathOp::Arcsinh => Op::Arcsinh,
        MathOp::Arccosh => Op::Arccosh,
        MathOp::Arctanh => Op::Arctanh,
        MathOp::Arcsech => Op::Arcsech,
        MathOp::Arccsch => Op::Arccsch,
        MathOp::Arccoth => Op::Arccoth,
        MathOp::Eq => Op::Eq,
        MathOp::Neq => Op::Neq,
        MathOp::Gt => Op::Gt,
        MathOp::Lt => Op::Lt,
        MathOp::Geq => Op::Geq,
        MathOp::Leq => Op::Leq,
        MathOp::And => Op::And,
        MathOp::Or => Op::Or,
        MathOp::Xor => Op::Xor,
        MathOp::Not => Op::Not,
        MathOp::Implies => Op::Implies,
        MathOp::Max => Op::Max,
        MathOp::Min => Op::Min,
        MathOp::Rem => Op::Rem,
        MathOp::Quotient => Op::Quotient,
        MathOp::Delay | MathOp::RateOf | MathOp::Function(_) => {
            unreachable!("resolved by the construction")
        }
    }
}

//! The Jacobian of the right hand side with respect to the states.
//!
//! [`jacobian`] gives the conservative sparsity pattern of `J[i][j] = d rate_i / d y_j` and
//! either the analytic entries ([`JacobianIr::Analytic`]) or, for a model of which a rate
//! depends on a state through an operator without a derivative, the coloring of the columns of
//! the pattern for a sparse difference quotient ([`JacobianIr::ColoredDifference`]) with the
//! reason. Events and the changes of a simulation are not differentiated: the engine
//! reinitializes CVODE after them.
//!
//! # The pattern
//!
//! A state `y_j` can influence the rate of `y_i` where the rate reads `y_j`, or reads the target
//! of an assignment rule which reads `y_j`, directly or through other rules (anywhere in the
//! math, also in a condition of a `piecewise` or in the argument of `floor`, so the pattern holds
//! every entry a difference quotient can see). The pattern is in CSC order, the rows of a
//! column ascending, as KLU takes it; a matrix of `n` states with every entry has `n * n`
//! entries, a sparse model only its own.
//!
//! # The assignment rules
//!
//! The rules are differentiated rule by rule, in their order of dependency (the chain rule over
//! the rules, not the rules inlined into the rates): the derivative of the target of a rule `r`
//! with respect to `y_j`, for every `y_j` the pattern of the rule holds, is an expression of the
//! rule's math and of the derivatives of the rules it reads. Such a derivative which is a number
//! or a symbol is used where it is read; every other one is an entry of
//! [`JacobianIr::Analytic::derivatives`], which later derivatives and the entries read as
//! [`Symbol::Derivative`] of its index, so a rule read by many rates is differentiated once.
//! Only the rules a rate reads, directly or through other rules, are differentiated.
//!
//! # The derivatives of the operators
//!
//! `u'` is the derivative of the argument `u` (of `a`, `b`, ...); a term whose derivative is
//! structurally zero (it reads no state, directly or through a rule) is left out, and a factor
//! which is the number `1` is left out of a product (`1 * x` is `x` for every double), nothing
//! else is simplified or folded but `c - 1` of a number `c`, whose double is the one the
//! operation gives. The math is the one `sbmlsim-codegen` emits for every operator (its
//! `math.rs`):
//!
//! | operator | derivative |
//! |---|---|
//! | `a + b + ...` | `a' + b' + ...` |
//! | `-u`, `a - b` | `-u'`, `a' - b'` |
//! | `a * b * c` | `a' * b * c + a * b' * c + a * b * c'` |
//! | `a / b` | `a' / b - (a * b') / (b * b)` |
//! | `pow(a, b)` | `b * pow(a, b - 1) * a' + pow(a, b) * ln(a) * b'` (without the second term where `b` reads no state, so `x^c` at `x = 0` is no NaN) |
//! | `root(n, x)`, `root(x)` | the derivative of `pow(x, 1 / n)`, `pow(x, 0.5)` (the code of the value) |
//! | `abs(u)` | `-u'` where `u < 0`, else `u'` (also at `u = 0`) |
//! | `exp(u)` | `exp(u) * u'` |
//! | `ln(u)` | `u' / u` |
//! | `log(b, x)`, `log(x)` | the derivative of `ln(x) / ln(b)`, `ln(x) / ln(10)` (the code of the value) |
//! | `floor`, `ceiling`, `quotient` | 0 (between the jumps) |
//! | `rem(a, b)` | `a' - ((a - rem(a, b)) / b) * b'`, the derivative of `a - b * trunc(a / b)` between the jumps |
//! | `factorial(u)` | none: a model where `u` reads a state has no analytic Jacobian |
//! | `sin`, `cos`, `tan` | `cos(u) * u'`, `-(sin(u) * u')`, `u' / (cos(u) * cos(u))` |
//! | `sec`, `csc`, `cot` | `(sin(u) * u') / (cos(u) * cos(u))`, `-((cos(u) * u') / (sin(u) * sin(u)))`, `-(u' / (sin(u) * sin(u)))` |
//! | `sinh`, `cosh`, `tanh` | `cosh(u) * u'`, `sinh(u) * u'`, `u' / (cosh(u) * cosh(u))` |
//! | `sech`, `csch`, `coth` | `-((sinh(u) * u') / (cosh(u) * cosh(u)))`, `-((cosh(u) * u') / (sinh(u) * sinh(u)))`, `-(u' / (sinh(u) * sinh(u)))` |
//! | `arcsin`, `arccos` | `u' / sqrt(1 - u * u)`, `-(u' / sqrt(1 - u * u))` |
//! | `arctan`, `arccot` | `u' / (1 + u * u)`, `-(u' / (1 + u * u))` |
//! | `arcsec`, `arccsc` | `u' / (abs(u) * sqrt(u * u - 1))`, `-(u' / (abs(u) * sqrt(u * u - 1)))` |
//! | `arcsinh`, `arccosh` | `u' / sqrt(u * u + 1)`, `u' / (sqrt(u - 1) * sqrt(u + 1))` |
//! | `arctanh`, `arccoth` | `u' / (1 - u * u)` (both) |
//! | `arcsech`, `arccsch` | `-(u' / (u * sqrt(1 - u * u)))`, `-(u' / (abs(u) * sqrt(1 + u * u)))` |
//! | `max(a, b)`, `min(a, b)` | `a'` where `a > b` (`a < b`), else `b'`: the derivative of the argument the value takes, on a tie the later one; a left fold over more arguments |
//! | `piecewise` | the piecewise of the derivatives of the values with the same conditions (not differentiated), without `otherwise` none (the value is NaN there) |
//! | relations, `and`, `or`, `xor`, `not`, `implies`, numbers, `time` | 0 |
//!
//! `sqrt(x)` is `root(x)`, so `pow(x, 0.5)` as the value. The relations of the conditions are the
//! unordered ones of the emitted code, so at a NaN the derivative of `abs` and of `max` may take
//! the other branch than the value; the value is NaN there.
//!
//! The crate does no file system or OS access.

use std::collections::{BTreeSet, HashMap};

use sbmlsim_abi::SparsityPattern;

use crate::{Expr, Model, Op, Symbol, index};

/// The Jacobian of a model.
#[derive(Debug, Clone, PartialEq)]
pub enum JacobianIr {
    /// The analytic Jacobian.
    Analytic {
        /// The conservative pattern, in CSC order.
        pattern: SparsityPattern,
        /// The derivatives of the assignment rules the entries read, in this order, the
        /// derivative of index `k` as [`Symbol::Derivative`]`(k)`; one may read those before it.
        derivatives: Vec<Expr>,
        /// The entries in the CSC order of the pattern, over the states, the parameters (the
        /// targets of the assignment rules as their values), the time and the derivatives.
        entries: Vec<Expr>,
    },
    /// A difference quotient over the colors of the columns of the pattern.
    ColoredDifference {
        /// The conservative pattern, in CSC order.
        pattern: SparsityPattern,
        /// The color of every column ([`SparsityPattern::greedy_colors`]).
        colors: Vec<u32>,
        /// Why the model has no analytic Jacobian, naming the operator and the element.
        reason: String,
    },
}

impl JacobianIr {
    /// The pattern.
    pub fn pattern(&self) -> &SparsityPattern {
        match self {
            JacobianIr::Analytic { pattern, .. } => pattern,
            JacobianIr::ColoredDifference { pattern, .. } => pattern,
        }
    }

    /// The difference quotient over the pattern of this Jacobian, with the reason.
    pub fn into_difference(self, reason: impl Into<String>) -> JacobianIr {
        match self {
            JacobianIr::Analytic { pattern, .. } => JacobianIr::ColoredDifference {
                colors: pattern.greedy_colors(),
                pattern,
                reason: reason.into(),
            },
            difference => difference,
        }
    }
}

/// The Jacobian of the right hand side of a model with respect to its states.
pub fn jacobian(model: &Model) -> JacobianIr {
    let rules = Rules::of(model);
    let pattern = pattern_of(model, &rules);
    match analytic(model, &rules, &pattern) {
        Ok((derivatives, entries)) => JacobianIr::Analytic {
            pattern,
            derivatives,
            entries,
        },
        Err(reason) => JacobianIr::ColoredDifference {
            colors: pattern.greedy_colors(),
            pattern,
            reason,
        },
    }
}

/// The conservative pattern of the Jacobian of a model.
pub fn pattern(model: &Model) -> SparsityPattern {
    pattern_of(model, &Rules::of(model))
}

/// The assignment rules by target, with the states each depends on.
struct Rules {
    /// The position of the rule of a target.
    position: HashMap<Symbol, usize>,
    /// The states every rule depends on, ascending.
    deps: Vec<Vec<u32>>,
}

impl Rules {
    fn of(model: &Model) -> Rules {
        let position: HashMap<Symbol, usize> = model
            .assignment_rules
            .iter()
            .enumerate()
            .map(|(r, a)| (a.target, r))
            .collect();
        let mut deps: Vec<Vec<u32>> = Vec::with_capacity(model.assignment_rules.len());
        for a in &model.assignment_rules {
            // the rules come in their order of dependency: a rule reads only those before it
            let d = states_read(&a.expr, &position, &deps);
            deps.push(d);
        }
        Rules { position, deps }
    }
}

/// The states an expression depends on, directly or through the rules before, ascending.
fn states_read(e: &Expr, position: &HashMap<Symbol, usize>, deps: &[Vec<u32>]) -> Vec<u32> {
    let mut out = BTreeSet::new();
    e.visit_symbols(&mut |s| match s {
        Symbol::State(j) => {
            out.insert(j);
        }
        Symbol::Param(_) => {
            if let Some(d) = position.get(&s).and_then(|&r| deps.get(r)) {
                out.extend(d.iter().copied());
            }
        }
        Symbol::Time | Symbol::Derivative(_) => {}
        Symbol::Common(_) | Symbol::Constant(_) => {
            unreachable!("the symbols of the optimizer are only in its programs")
        }
    });
    out.into_iter().collect()
}

/// The conservative pattern: the rows of a column are the rates which depend on its state.
fn pattern_of(model: &Model, rules: &Rules) -> SparsityPattern {
    let n = model.states.len();
    let mut columns: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, rate) in model.rates.iter().enumerate() {
        for j in states_read(rate, &rules.position, &rules.deps) {
            // the rates in ascending order: every column gets its rows ascending
            columns[j as usize].push(index(i));
        }
    }
    let mut col_ptr = Vec::with_capacity(n + 1);
    col_ptr.push(0u32);
    let mut row_idx = Vec::new();
    for rows in columns {
        row_idx.extend(rows);
        col_ptr.push(index(row_idx.len()));
    }
    SparsityPattern {
        n: index(n),
        col_ptr,
        row_idx,
    }
}

/// The derivatives of the rules and the entries of the analytic Jacobian, or why there is none.
fn analytic(
    model: &Model,
    rules: &Rules,
    pattern: &SparsityPattern,
) -> Result<(Vec<Expr>, Vec<Expr>), String> {
    // the rules a rate reads, directly or through other rules
    let mut needed = vec![false; model.assignment_rules.len()];
    let mark = |e: &Expr, needed: &mut Vec<bool>| {
        e.visit_symbols(&mut |s| {
            if let Some(&r) = rules.position.get(&s) {
                needed[r] = true;
            }
        })
    };
    for rate in &model.rates {
        mark(rate, &mut needed);
    }
    for r in (0..model.assignment_rules.len()).rev() {
        if needed[r] {
            mark(&model.assignment_rules[r].expr, &mut needed);
        }
    }

    // the derivative of the target of rule `r` with respect to `y_j`, where it is not zero
    let mut known: HashMap<(usize, u32), Expr> = HashMap::new();
    let mut derivatives: Vec<Expr> = Vec::new();
    for (r, a) in model.assignment_rules.iter().enumerate() {
        if !needed[r] {
            continue;
        }
        for &j in &rules.deps[r] {
            let wrt = |s: Symbol| symbol_derivative(s, j, rules, &known);
            let d = derivative(&a.expr, &wrt).map_err(|op| {
                no_derivative(
                    op,
                    &format!("the assignment rule of {}", model.entity(a.target).id),
                )
            })?;
            let Some(d) = d else { continue };
            let d = match d {
                Expr::Number(_) | Expr::Symbol(_) => d,
                _ => {
                    derivatives.push(d);
                    Expr::Symbol(Symbol::Derivative(index(derivatives.len() - 1)))
                }
            };
            known.insert((r, j), d);
        }
    }

    let n = model.states.len();
    let mut entries = Vec::with_capacity(pattern.nnz());
    for j in 0..n {
        let state = index(j);
        for &i in pattern.rows(j) {
            let wrt = |s: Symbol| symbol_derivative(s, state, rules, &known);
            let d = derivative(&model.rates[i as usize], &wrt).map_err(|op| {
                no_derivative(op, &format!("the rate of {}", model.states[i as usize].id))
            })?;
            entries.push(d.unwrap_or(Expr::Number(0.0)));
        }
    }
    Ok((derivatives, entries))
}

/// The derivative of a symbol with respect to `y_j`: 1 for `y_j`, the derivative of a rule for
/// its target, else none (zero).
fn symbol_derivative(
    s: Symbol,
    j: u32,
    rules: &Rules,
    known: &HashMap<(usize, u32), Expr>,
) -> Option<Expr> {
    match s {
        Symbol::State(k) if k == j => Some(Expr::Number(1.0)),
        Symbol::Param(_) => rules
            .position
            .get(&s)
            .and_then(|&r| known.get(&(r, j)))
            .cloned(),
        _ => None,
    }
}

/// The reason of a model without an analytic Jacobian.
fn no_derivative(op: Op, element: &str) -> String {
    format!(
        "{} has no derivative ({element})",
        format!("{op:?}").to_lowercase()
    )
}

/// `a * b`, without a factor which is the number 1.
fn times(a: Expr, b: Expr) -> Expr {
    match (a, b) {
        (Expr::Number(1.0), b) => b,
        (a, Expr::Number(1.0)) => a,
        (a, b) => Expr::apply(Op::Times, vec![a, b]),
    }
}

/// `a * a`.
fn square(a: &Expr) -> Expr {
    Expr::apply(Op::Times, vec![a.clone(), a.clone()])
}

fn div(a: Expr, b: Expr) -> Expr {
    Expr::apply(Op::Divide, vec![a, b])
}

fn neg(a: Expr) -> Expr {
    Expr::apply(Op::Minus, vec![a])
}

fn minus(a: Expr, b: Expr) -> Expr {
    Expr::apply(Op::Minus, vec![a, b])
}

fn plus(a: Expr, b: Expr) -> Expr {
    Expr::apply(Op::Plus, vec![a, b])
}

fn call(op: Op, u: &Expr) -> Expr {
    Expr::apply(op, vec![u.clone()])
}

fn num(x: f64) -> Expr {
    Expr::Number(x)
}

/// `sqrt(x)` as the code of the value computes it, `root(x)`.
fn sqrt(x: Expr) -> Expr {
    Expr::apply(Op::Root, vec![x])
}

/// The derivative of `e`, `None` where it is structurally zero; `wrt` gives the derivative of a
/// symbol. The error is the operator without a derivative.
pub fn derivative(e: &Expr, wrt: &dyn Fn(Symbol) -> Option<Expr>) -> Result<Option<Expr>, Op> {
    let d = |e: &Expr| derivative(e, wrt);
    Ok(match e {
        Expr::Number(_) | Expr::Bool(_) => None,
        Expr::Symbol(s) => wrt(*s),
        Expr::Piecewise { pieces, otherwise } => {
            let values: Vec<Option<Expr>> =
                pieces.iter().map(|(v, _)| d(v)).collect::<Result<_, _>>()?;
            let other = otherwise.as_deref().map(d).transpose()?;
            if values.iter().all(Option::is_none) && other.as_ref().is_none_or(Option::is_none) {
                None
            } else {
                Some(Expr::Piecewise {
                    pieces: values
                        .into_iter()
                        .zip(pieces)
                        .map(|(dv, (_, c))| (dv.unwrap_or(num(0.0)), c.clone()))
                        .collect(),
                    otherwise: other.map(|o| Box::new(o.unwrap_or(num(0.0)))),
                })
            }
        }
        Expr::Apply { op, args } => apply(*op, args, e, wrt)?,
    })
}

/// The derivative of an operator applied to its arguments, `e` the application.
fn apply(
    op: Op,
    args: &[Expr],
    e: &Expr,
    wrt: &dyn Fn(Symbol) -> Option<Expr>,
) -> Result<Option<Expr>, Op> {
    let d = |e: &Expr| derivative(e, wrt);
    // a function of one argument: f'(u) * u', none where u' is none
    let chain = |f: &dyn Fn(&Expr, Expr) -> Expr| -> Result<Option<Expr>, Op> {
        let u = &args[0];
        Ok(d(u)?.map(|du| f(u, du)))
    };
    Ok(match op {
        Op::Plus => {
            let terms: Vec<Expr> = args
                .iter()
                .map(d)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            match terms.len() {
                0 => None,
                1 => terms.into_iter().next(),
                _ => Some(Expr::apply(Op::Plus, terms)),
            }
        }
        Op::Minus if args.len() == 1 => d(&args[0])?.map(neg),
        Op::Minus => match (d(&args[0])?, d(&args[1])?) {
            (None, None) => None,
            (Some(da), None) => Some(da),
            (None, Some(db)) => Some(neg(db)),
            (Some(da), Some(db)) => Some(minus(da, db)),
        },
        Op::Times => {
            let mut terms = Vec::new();
            for (k, a) in args.iter().enumerate() {
                let Some(da) = d(a)? else { continue };
                let mut factors: Vec<Expr> = Vec::with_capacity(args.len());
                for (m, b) in args.iter().enumerate() {
                    if m != k {
                        factors.push(b.clone());
                    } else if !matches!(da, Expr::Number(x) if x == 1.0) {
                        factors.push(da.clone());
                    }
                }
                terms.push(match factors.len() {
                    0 => num(1.0),
                    1 => factors.pop().expect("one factor"),
                    _ => Expr::apply(Op::Times, factors),
                });
            }
            match terms.len() {
                0 => None,
                1 => terms.pop(),
                _ => Some(Expr::apply(Op::Plus, terms)),
            }
        }
        Op::Divide => {
            let (a, b) = (&args[0], &args[1]);
            let second = |db: Expr| div(times(a.clone(), db), square(b));
            match (d(a)?, d(b)?) {
                (None, None) => None,
                (Some(da), None) => Some(div(da, b.clone())),
                (None, Some(db)) => Some(neg(second(db))),
                (Some(da), Some(db)) => Some(minus(div(da, b.clone()), second(db))),
            }
        }
        Op::Power => {
            let (a, b) = (&args[0], &args[1]);
            let first = |da: Expr| {
                let exponent = match b {
                    Expr::Number(c) => num(c - 1.0),
                    _ => minus(b.clone(), num(1.0)),
                };
                times(
                    times(b.clone(), Expr::apply(Op::Power, vec![a.clone(), exponent])),
                    da,
                )
            };
            let second = |db: Expr| times(times(e.clone(), call(Op::Ln, a)), db);
            match (d(a)?, d(b)?) {
                (None, None) => None,
                (Some(da), None) => Some(first(da)),
                (None, Some(db)) => Some(second(db)),
                (Some(da), Some(db)) => Some(plus(first(da), second(db))),
            }
        }
        Op::Root => {
            let power = match args {
                [x] => Expr::apply(Op::Power, vec![x.clone(), num(0.5)]),
                [n, x] => Expr::apply(Op::Power, vec![x.clone(), div(num(1.0), n.clone())]),
                _ => unreachable!("root has one or two arguments"),
            };
            derivative(&power, wrt)?
        }
        Op::Log => {
            let (base, x) = match args {
                [x] => (num(10.0), x),
                [b, x] => (b.clone(), x),
                _ => unreachable!("log has one or two arguments"),
            };
            let quotient = div(call(Op::Ln, x), call(Op::Ln, &base));
            derivative(&quotient, wrt)?
        }
        Op::Abs => chain(&|u, du| Expr::Piecewise {
            pieces: vec![(
                neg(du.clone()),
                Expr::apply(Op::Lt, vec![u.clone(), num(0.0)]),
            )],
            otherwise: Some(Box::new(du)),
        })?,
        Op::Exp => chain(&|_, du| times(e.clone(), du))?,
        Op::Ln => chain(&|u, du| div(du, u.clone()))?,
        Op::Floor | Op::Ceiling | Op::Quotient => None,
        Op::Rem => {
            let (a, b) = (&args[0], &args[1]);
            let second = |db: Expr| {
                let truncated = div(minus(a.clone(), e.clone()), b.clone());
                times(truncated, db)
            };
            match (d(a)?, d(b)?) {
                (None, None) => None,
                (Some(da), None) => Some(da),
                (None, Some(db)) => Some(neg(second(db))),
                (Some(da), Some(db)) => Some(minus(da, second(db))),
            }
        }
        Op::Factorial => match d(&args[0])? {
            None => None,
            Some(_) => return Err(Op::Factorial),
        },
        Op::Sin => chain(&|u, du| times(call(Op::Cos, u), du))?,
        Op::Cos => chain(&|u, du| neg(times(call(Op::Sin, u), du)))?,
        Op::Tan => chain(&|u, du| div(du, square(&call(Op::Cos, u))))?,
        Op::Sec => chain(&|u, du| div(times(call(Op::Sin, u), du), square(&call(Op::Cos, u))))?,
        Op::Csc => {
            chain(&|u, du| neg(div(times(call(Op::Cos, u), du), square(&call(Op::Sin, u)))))?
        }
        Op::Cot => chain(&|u, du| neg(div(du, square(&call(Op::Sin, u)))))?,
        Op::Sinh => chain(&|u, du| times(call(Op::Cosh, u), du))?,
        Op::Cosh => chain(&|u, du| times(call(Op::Sinh, u), du))?,
        Op::Tanh => chain(&|u, du| div(du, square(&call(Op::Cosh, u))))?,
        Op::Sech => chain(&|u, du| {
            neg(div(
                times(call(Op::Sinh, u), du),
                square(&call(Op::Cosh, u)),
            ))
        })?,
        Op::Csch => chain(&|u, du| {
            neg(div(
                times(call(Op::Cosh, u), du),
                square(&call(Op::Sinh, u)),
            ))
        })?,
        Op::Coth => chain(&|u, du| neg(div(du, square(&call(Op::Sinh, u)))))?,
        Op::Arcsin => chain(&|u, du| div(du, sqrt(minus(num(1.0), square(u)))))?,
        Op::Arccos => chain(&|u, du| neg(div(du, sqrt(minus(num(1.0), square(u))))))?,
        Op::Arctan => chain(&|u, du| div(du, plus(num(1.0), square(u))))?,
        Op::Arccot => chain(&|u, du| neg(div(du, plus(num(1.0), square(u)))))?,
        Op::Arcsec => chain(&|u, du| {
            div(
                du,
                times(call(Op::Abs, u), sqrt(minus(square(u), num(1.0)))),
            )
        })?,
        Op::Arccsc => chain(&|u, du| {
            neg(div(
                du,
                times(call(Op::Abs, u), sqrt(minus(square(u), num(1.0)))),
            ))
        })?,
        Op::Arcsinh => chain(&|u, du| div(du, sqrt(plus(square(u), num(1.0)))))?,
        Op::Arccosh => chain(&|u, du| {
            div(
                du,
                times(
                    sqrt(minus(u.clone(), num(1.0))),
                    sqrt(plus(u.clone(), num(1.0))),
                ),
            )
        })?,
        Op::Arctanh | Op::Arccoth => chain(&|u, du| div(du, minus(num(1.0), square(u))))?,
        Op::Arcsech => {
            chain(&|u, du| neg(div(du, times(u.clone(), sqrt(minus(num(1.0), square(u)))))))?
        }
        Op::Arccsch => chain(&|u, du| {
            neg(div(
                du,
                times(call(Op::Abs, u), sqrt(plus(num(1.0), square(u)))),
            ))
        })?,
        Op::Max | Op::Min => {
            let Some((first, rest)) = args.split_first() else {
                return Ok(None);
            };
            let relation = if op == Op::Max { Op::Gt } else { Op::Lt };
            let mut current = d(first)?;
            for (k, b) in rest.iter().enumerate() {
                let db = d(b)?;
                if current.is_none() && db.is_none() {
                    continue;
                }
                // the value of the fold so far, which the emitted code compares with `b`
                let so_far = if k == 0 {
                    first.clone()
                } else {
                    Expr::apply(op, args[..=k].to_vec())
                };
                current = Some(Expr::Piecewise {
                    pieces: vec![(
                        current.unwrap_or(num(0.0)),
                        Expr::apply(relation, vec![so_far, b.clone()]),
                    )],
                    otherwise: Some(Box::new(db.unwrap_or(num(0.0)))),
                });
            }
            current
        }
        Op::Eq
        | Op::Neq
        | Op::Gt
        | Op::Lt
        | Op::Geq
        | Op::Leq
        | Op::And
        | Op::Or
        | Op::Xor
        | Op::Not
        | Op::Implies => None,
    })
}

//! The discontinuities of the model math: the root functions of the conditions of `piecewise`,
//! at whose zeros the engine stops the integration and starts the integrator again, as
//! libroadrunner does.
//!
//! libroadrunner (the "piecewise triggers" of its LLVM model generator,
//! `ModelGeneratorContext::addAllPiecewiseTriggers`) collects, from the math of every rule, of
//! every kinetic law and of every event trigger (not of the initial assignments, the event
//! assignments, the delays and the priorities, which are evaluated where the integration
//! stops), one boolean per branch of every `piecewise` (nested ones included, wherever they
//! are): `c1`, `c2 && !c1`, ..., `ck && !c1 && ... && !c(k-1)` and, with an `otherwise`,
//! `!c1 && ... && !ck`, the value of each a root function of CVODE (`1` true, `-1` false). The
//! integration stops where one of them changes, that is where the branch the `piecewise`
//! selects changes, and CVODE is started again from the state there.
//!
//! [`Model::discontinuity_roots`] gives root functions which change their sign where these
//! booleans change, continuous ones of the operands as the math states them (a comparison
//! `a > b` is `a - b`, as the root functions of the triggers of the events), so CVODE locates
//! them as the zeros of the root functions of the events and not by bisection of a step
//! function: the root function `G(c)` of a boolean `c` ([`sign_of`]) is positive where `c` is
//! true and negative where it is false, and the root function of the branch `i` is
//! `min(G(ci), -G(c1), ..., -G(c(i-1)))`. The branch of the `otherwise` gives none: it changes
//! only where one of the others changes (the branches exclude each other). A root function
//! which depends neither on the time nor on a state (directly or through assignment rules),
//! after constant folding ([`fold`]), gives none: it changes only where the integration stops
//! anyway (a change, an event). Equal root functions are one.
//!
//! The kinetic laws are those of every reaction, also one with local parameters (libroadrunner
//! takes no piecewise triggers from a kinetic law with local parameters, which is a slip of
//! `addPiecewiseTriggersFrom(const KineticLaw*)`: it substitutes the values into a copy of the
//! math and deletes it unread); the calls of function definitions are inlined, as
//! libroadrunner inlines them for its triggers.

use std::collections::HashSet;

use crate::optimize::fold;
use crate::{CompileError, Expr, Model, Op, SelectionKind, Symbol, is_boolean_valued};

impl Model {
    /// The root functions of the discontinuities of the math (see the module documentation):
    /// of the assignment rules in their order, the rates of the states, the rates of the
    /// reactions and the triggers of the events, each folded ([`fold`]), within each in the
    /// written order, one per branch of a `piecewise` but its `otherwise`, without those which
    /// depend neither on the time nor on a state and without repetitions.
    pub fn discontinuity_roots(&self) -> Vec<Expr> {
        let varying = self.varying_parameters();
        let sources = self
            .assignment_rules
            .iter()
            .map(|a| &a.expr)
            .chain(&self.rates)
            .chain(
                self.selections
                    .iter()
                    .filter(|s| s.kind == SelectionKind::ReactionRate)
                    .map(|s| &s.expr),
            )
            .chain(self.events.iter().map(|e| &e.trigger));
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for e in sources.filter(|e| has_piecewise(e)) {
            let mut roots = Vec::new();
            branches(&fold(e), &mut roots);
            for root in roots {
                let root = fold(&root);
                // the bits of the numbers tell trees apart (`{:?}` of a double is exact)
                if varies(&root, &varying) && seen.insert(format!("{root:?}")) {
                    out.push(root);
                }
            }
        }
        out
    }

    /// Builds discontinuity roots after checking a conservative bound on generated nodes.
    pub fn try_discontinuity_roots(&self) -> Result<Vec<Expr>, CompileError> {
        let mut remaining = crate::expr::MAX_EXPR_NODES;
        let sources = self
            .assignment_rules
            .iter()
            .map(|a| &a.expr)
            .chain(&self.rates)
            .chain(
                self.selections
                    .iter()
                    .filter(|s| s.kind == SelectionKind::ReactionRate)
                    .map(|s| &s.expr),
            )
            .chain(self.events.iter().map(|e| &e.trigger));
        for source in sources {
            check_branches(source, &mut remaining)?;
        }
        Ok(self.discontinuity_roots())
    }

    /// Whether each parameter is the target of an assignment rule which depends on the time or
    /// a state, directly or through other rules.
    fn varying_parameters(&self) -> Vec<bool> {
        let mut varying = vec![false; self.parameters.len()];
        for a in &self.assignment_rules {
            if let Symbol::Param(i) = a.target
                && varies(&a.expr, &varying)
            {
                varying[i as usize] = true;
            }
        }
        varying
    }
}

/// Whether `e` has a `piecewise`.
fn has_piecewise(e: &Expr) -> bool {
    match e {
        Expr::Number(_) | Expr::Bool(_) | Expr::Symbol(_) => false,
        Expr::Apply { args, .. } => args.iter().any(has_piecewise),
        Expr::Piecewise { .. } => true,
    }
}

// Saturating preflight sizes bound allocation before any branch/sign tree is cloned.
fn node_count(e: &Expr) -> usize {
    match e {
        Expr::Apply { args, .. } => args
            .iter()
            .fold(1usize, |n, a| n.saturating_add(node_count(a))),
        Expr::Piecewise { pieces, otherwise } => pieces
            .iter()
            .fold(1usize, |n, (v, c)| {
                n.saturating_add(node_count(v))
                    .saturating_add(node_count(c))
            })
            .saturating_add(otherwise.as_deref().map_or(0, node_count)),
        _ => 1,
    }
}

fn sign_size(e: &Expr) -> usize {
    match e {
        Expr::Bool(_) => 1,
        Expr::Apply { op, args } => match op {
            Op::Gt | Op::Geq | Op::Lt | Op::Leq | Op::Eq | Op::Neq => {
                args.windows(2).fold(1usize, |n, w| {
                    n.saturating_add(3)
                        .saturating_add(node_count(&w[0]))
                        .saturating_add(node_count(&w[1]))
                })
            }
            Op::And | Op::Or | Op::Xor | Op::Not | Op::Implies => args
                .iter()
                .fold(4usize, |n, a| n.saturating_add(sign_size(a))),
            _ => node_count(e).saturating_add(1),
        },
        Expr::Piecewise { pieces, otherwise } if is_boolean_valued(e) => {
            let mut size = 1usize;
            let mut preceding = 0usize;
            for (v, c) in pieces {
                let condition = sign_size(c);
                size = size
                    .saturating_add(1)
                    .saturating_add(sign_size(v))
                    .saturating_add(condition)
                    .saturating_add(preceding);
                preceding = preceding.saturating_add(condition).saturating_add(1);
            }
            if let Some(o) = otherwise {
                size = size
                    .saturating_add(1)
                    .saturating_add(sign_size(o))
                    .saturating_add(preceding);
            }
            size
        }
        _ => node_count(e).saturating_add(1),
    }
}

fn sign_depth(e: &Expr) -> usize {
    match e {
        Expr::Bool(_) => 1,
        Expr::Apply { op, args } => match op {
            Op::Eq | Op::Neq | Op::Gt | Op::Geq | Op::Lt | Op::Leq => {
                args.iter().map(Expr::depth).max().unwrap_or(0) + 4
            }
            Op::And | Op::Or | Op::Xor | Op::Not => {
                args.iter().map(sign_depth).max().unwrap_or(0) + 1
            }
            Op::Implies => args.iter().map(sign_depth).max().unwrap_or(0) + 2,
            _ => e.depth() + 1,
        },
        Expr::Piecewise { pieces, otherwise } if is_boolean_valued(e) => {
            pieces
                .iter()
                .map(|(v, c)| sign_depth(v).max(sign_depth(c) + 1))
                .chain(otherwise.as_deref().map(sign_depth))
                .max()
                .unwrap_or(0)
                + 2
        }
        _ => e.depth() + 1,
    }
}

fn check_branches(e: &Expr, remaining: &mut usize) -> Result<(), CompileError> {
    match e {
        Expr::Apply { args, .. } => {
            for a in args {
                check_branches(a, remaining)?;
            }
        }
        Expr::Piecewise { pieces, otherwise } => {
            let mut preceding = 0usize;
            for (value, condition) in pieces {
                let size = sign_size(condition);
                if sign_depth(condition) + 2 > crate::expr::MAX_EXPR_DEPTH {
                    return Err(CompileError::Invalid(format!(
                        "discontinuity root construction exceeds its {} depth budget",
                        crate::expr::MAX_EXPR_DEPTH
                    )));
                }
                *remaining = remaining
                    .checked_sub(size.saturating_add(preceding).saturating_add(1))
                    .ok_or_else(|| {
                        CompileError::Invalid(format!(
                            "discontinuity root construction exceeds its {} node budget",
                            crate::expr::MAX_EXPR_NODES
                        ))
                    })?;
                preceding = preceding.saturating_add(size).saturating_add(1);
                check_branches(value, remaining)?;
                check_branches(condition, remaining)?;
            }
            if let Some(o) = otherwise {
                check_branches(o, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether `e` reads the time, a state or a varying parameter.
fn varies(e: &Expr, varying: &[bool]) -> bool {
    let mut any = false;
    e.visit_symbols(&mut |s| {
        any |= match s {
            Symbol::Time | Symbol::State(_) => true,
            Symbol::Param(i) => varying[i as usize],
            Symbol::Derivative(_) | Symbol::Common(_) | Symbol::Constant(_) => false,
        }
    });
    any
}

/// Collects the root functions of the branches of every `piecewise` in `e`, in the written
/// order (a `piecewise` before the ones inside it).
fn branches(e: &Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Number(_) | Expr::Bool(_) | Expr::Symbol(_) => {}
        Expr::Apply { args, .. } => args.iter().for_each(|a| branches(a, out)),
        Expr::Piecewise { pieces, otherwise } => {
            for i in 0..pieces.len() {
                let mut terms = vec![sign_of(&pieces[i].1)];
                terms.extend(pieces[..i].iter().map(|(_, c)| negate(sign_of(c))));
                out.push(min(terms));
            }
            for (value, condition) in pieces {
                branches(value, out);
                branches(condition, out);
            }
            if let Some(o) = otherwise {
                branches(o, out);
            }
        }
    }
}

/// A function of the operands of the boolean `c` which is positive where `c` is true and
/// negative where it is false (0 only where `c` may change):
///
/// - `a > b`, `a >= b`: `a - b`; `a < b`, `a <= b`: `b - a`; `a == b`: `-abs(a - b)`;
///   `a != b`: `abs(a - b)` (both change only at a single point, where neither crosses 0, as
///   libroadrunner's boolean is a different value only there); a chain the `min` of its
///   adjacent pairs;
/// - `and`: `min`, `or`: `max` (of none `1` and `-1`), `not`: the negation, `xor`:
///   `max(min(x, -y), min(-x, y))` folded from the left using temporaries (of none `-1`), `implies`:
///   `max(-x, y)`;
/// - `true`: `1`, `false`: `-1`;
/// - a `piecewise` whose values are booleans: the `max` over its branches of the `min` of the
///   value and the boolean of the branch (no branch taken: `-1`);
/// - a number, which is true where it is not 0: `abs` of it.
pub(crate) fn sign_of(c: &Expr) -> Expr {
    let pairs = |args: &[Expr], f: &dyn Fn(Expr, Expr) -> Expr| {
        min(args
            .windows(2)
            .map(|w| f(w[0].clone(), w[1].clone()))
            .collect())
    };
    match c {
        Expr::Bool(b) => Expr::Number(if *b { 1.0 } else { -1.0 }),
        Expr::Apply { op, args } => match op {
            Op::Gt | Op::Geq => pairs(args, &difference),
            Op::Lt | Op::Leq => pairs(args, &|a, b| difference(b, a)),
            Op::Eq => pairs(args, &|a, b| negate(abs(difference(a, b)))),
            Op::Neq => pairs(args, &|a, b| abs(difference(a, b))),
            Op::And => min(args.iter().map(sign_of).collect()),
            Op::Or => max(args.iter().map(sign_of).collect()),
            Op::Not => negate(sign_of(&args[0])),
            Op::Xor => Expr::apply(Op::XorSign, args.iter().map(sign_of).collect()),
            Op::Implies => max(vec![negate(sign_of(&args[0])), sign_of(&args[1])]),
            _ => abs(c.clone()),
        },
        Expr::Piecewise { pieces, otherwise } if is_boolean_valued(c) => {
            let mut terms = Vec::new();
            for i in 0..pieces.len() {
                let mut branch = vec![sign_of(&pieces[i].0), sign_of(&pieces[i].1)];
                branch.extend(pieces[..i].iter().map(|(_, c)| negate(sign_of(c))));
                terms.push(min(branch));
            }
            if let Some(o) = otherwise {
                let mut branch = vec![sign_of(o)];
                branch.extend(pieces.iter().map(|(_, c)| negate(sign_of(c))));
                terms.push(min(branch));
            }
            max(terms)
        }
        _ => abs(c.clone()),
    }
}

/// `a - b`.
fn difference(a: Expr, b: Expr) -> Expr {
    Expr::apply(Op::Minus, vec![a, b])
}

/// `-x`.
fn negate(x: Expr) -> Expr {
    Expr::apply(Op::Minus, vec![x])
}

/// `abs(x)`.
fn abs(x: Expr) -> Expr {
    Expr::apply(Op::Abs, vec![x])
}

/// The `min` of the terms, the term itself of one, `1` (true) of none.
fn min(mut terms: Vec<Expr>) -> Expr {
    match terms.len() {
        0 => Expr::Number(1.0),
        1 => terms.remove(0),
        _ => Expr::apply(Op::Min, terms),
    }
}

/// The `max` of the terms, the term itself of one, `-1` (false) of none.
fn max(mut terms: Vec<Expr>) -> Expr {
    match terms.len() {
        0 => Expr::Number(-1.0),
        1 => terms.remove(0),
        _ => Expr::apply(Op::Max, terms),
    }
}

#[cfg(test)]
mod xor_tests {
    use super::*;
    use crate::eval::eval;

    #[test]
    fn signed_xor_preserves_the_ordered_fold_including_nonfinite_values() {
        let values = [
            f64::NEG_INFINITY,
            -2.0,
            -0.0,
            0.0,
            3.0,
            f64::INFINITY,
            f64::NAN,
        ];
        for a in values {
            for b in values {
                for c in values {
                    let args = vec![Expr::Number(a), Expr::Number(b), Expr::Number(c)];
                    let expected = args[1..].iter().fold(args[0].clone(), |x, y| {
                        max(vec![
                            min(vec![x.clone(), negate(y.clone())]),
                            min(vec![negate(x), y.clone()]),
                        ])
                    });
                    let actual = Expr::apply(Op::XorSign, args);
                    let expected = eval(&expected, &|_| unreachable!());
                    let actual = eval(&actual, &|_| unreachable!());
                    assert!(
                        actual.to_bits() == expected.to_bits()
                            || actual.is_nan() && expected.is_nan(),
                        "{a:?} {b:?} {c:?}: {actual:?} != {expected:?}"
                    );
                }
            }
        }
    }
}

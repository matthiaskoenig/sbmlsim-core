//! The optimizer of `sbmlsim_ir::optimize`: constant folding and the rewrites which are exact in
//! binary64, the common subexpressions of a function and the assignment rules no output reads.

use sbmlsim_ir::derive::jacobian;
use sbmlsim_ir::eval::eval;
use sbmlsim_ir::optimize::{Program, Statement, fold, plan, program};
use sbmlsim_ir::{Assignment, Entity, EntityKind, Expr, Model, Op, Symbol};

fn n(v: f64) -> Expr {
    Expr::Number(v)
}

fn p(i: u32) -> Expr {
    Expr::Symbol(Symbol::Param(i))
}

fn x() -> Expr {
    Expr::Symbol(Symbol::State(0))
}

fn t() -> Expr {
    Expr::Symbol(Symbol::Time)
}

fn ap(op: Op, args: Vec<Expr>) -> Expr {
    Expr::apply(op, args)
}

fn common(k: u32) -> Expr {
    Expr::Symbol(Symbol::Common(k))
}

/// Structural equality with the bits of the numbers (`0.0` is not `-0.0`, a NaN is the NaN of
/// its bits).
fn same(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Number(x), Expr::Number(y)) => x.to_bits() == y.to_bits(),
        (Expr::Bool(x), Expr::Bool(y)) => x == y,
        (Expr::Symbol(x), Expr::Symbol(y)) => x == y,
        (Expr::Apply { op: o, args: a }, Expr::Apply { op: q, args: b }) => {
            o == q && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (
            Expr::Piecewise {
                pieces: a,
                otherwise: oa,
            },
            Expr::Piecewise {
                pieces: b,
                otherwise: ob,
            },
        ) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((va, ca), (vb, cb))| same(va, vb) && same(ca, cb))
                && match (oa, ob) {
                    (None, None) => true,
                    (Some(a), Some(b)) => same(a, b),
                    _ => false,
                }
        }
        _ => false,
    }
}

#[track_caller]
fn assert_same(a: &Expr, b: &Expr) {
    assert!(same(a, b), "\n{a:?}\n!=\n{b:?}");
}

/// The value of an expression of literals as the emitted code computes it.
fn value(e: &Expr) -> f64 {
    eval(e, &|s| panic!("{s:?} is read"))
}

/// The number of applications of `op` in the statements of a program.
fn count(prog: &Program, op: Op) -> usize {
    fn walk(e: &Expr, op: Op) -> usize {
        match e {
            Expr::Apply { op: o, args } => {
                usize::from(*o == op) + args.iter().map(|a| walk(a, op)).sum::<usize>()
            }
            Expr::Piecewise { pieces, otherwise } => {
                pieces
                    .iter()
                    .map(|(v, c)| walk(v, op) + walk(c, op))
                    .sum::<usize>()
                    + otherwise.as_deref().map_or(0, |o| walk(o, op))
            }
            _ => 0,
        }
    }
    prog.statements
        .iter()
        .map(|s| match s {
            Statement::Let(_, e) | Statement::Output(e) => walk(e, op),
        })
        .sum()
}

fn outputs(prog: &Program) -> Vec<&Expr> {
    prog.statements
        .iter()
        .filter_map(|s| match s {
            Statement::Output(e) => Some(e),
            Statement::Let(..) => None,
        })
        .collect()
}

fn lets(prog: &Program) -> Vec<(Symbol, &Expr)> {
    prog.statements
        .iter()
        .filter_map(|s| match s {
            Statement::Let(symbol, e) => Some((*symbol, e)),
            Statement::Output(_) => None,
        })
        .collect()
}

#[test]
fn no_reassociation() {
    let a = p(0);
    // (a + 1e16) - 1e16 stays as written: it is 0 for a = 1, a would be 1
    let e = ap(
        Op::Minus,
        vec![ap(Op::Plus, vec![a.clone(), n(1e16)]), n(1e16)],
    );
    assert_same(&fold(&e), &e);
    // the same of literals is the double the code computes, 0 and not 1
    let literal = ap(
        Op::Minus,
        vec![ap(Op::Plus, vec![n(1.0), n(1e16)]), n(1e16)],
    );
    assert_same(&fold(&literal), &n(0.0));
    // a + 1 + 2 is (a + 1) + 2, never a + 3; 1 + 2 + a is (1 + 2) + a, so 3 + a
    let e = ap(Op::Plus, vec![a.clone(), n(1.0), n(2.0)]);
    assert_same(&fold(&e), &e);
    assert_same(
        &fold(&ap(Op::Plus, vec![n(1.0), n(2.0), a.clone()])),
        &ap(Op::Plus, vec![n(3.0), a.clone()]),
    );
    let e = ap(Op::Times, vec![a.clone(), n(2.0), n(3.0)]);
    assert_same(&fold(&e), &e);
    // no distribution, no cancellation
    for e in [
        ap(
            Op::Times,
            vec![n(2.0), ap(Op::Plus, vec![a.clone(), n(1.0)])],
        ),
        ap(Op::Minus, vec![a.clone(), a.clone()]),
        ap(Op::Divide, vec![a.clone(), a.clone()]),
    ] {
        assert_same(&fold(&e), &e);
    }
    // the program of the expression keeps it too
    let mut commons = 0;
    let prog = program(&[], &[e_of_reassociation()], true, &mut commons);
    assert_eq!(prog.statements.len(), 1);
    assert_same(outputs(&prog)[0], &e_of_reassociation());
}

fn e_of_reassociation() -> Expr {
    ap(Op::Minus, vec![ap(Op::Plus, vec![p(0), n(1e16)]), n(1e16)])
}

#[test]
fn exact_rewrites_only() {
    let a = p(0);
    let b = p(1);
    // exact for every double, NaN, the infinities and the zeros
    for (e, expected) in [
        (ap(Op::Times, vec![a.clone(), n(1.0)]), a.clone()),
        (ap(Op::Times, vec![n(1.0), a.clone()]), a.clone()),
        (
            ap(Op::Times, vec![a.clone(), n(1.0), b.clone()]),
            ap(Op::Times, vec![a.clone(), b.clone()]),
        ),
        (
            ap(Op::Times, vec![n(1.0), a.clone(), b.clone()]),
            ap(Op::Times, vec![a.clone(), b.clone()]),
        ),
        (ap(Op::Times, vec![n(2.0), n(0.5), a.clone()]), a.clone()),
        (ap(Op::Divide, vec![a.clone(), n(1.0)]), a.clone()),
        (ap(Op::Minus, vec![a.clone(), n(0.0)]), a.clone()),
        (ap(Op::Plus, vec![a.clone()]), a.clone()),
        (ap(Op::Times, vec![a.clone()]), a.clone()),
        (ap(Op::Max, vec![a.clone()]), a.clone()),
        // the stoichiometry 1 of a reactant
        (
            ap(Op::Minus, vec![ap(Op::Times, vec![n(1.0), a.clone()])]),
            ap(Op::Minus, vec![a.clone()]),
        ),
        // an n-ary max or min is the left fold the code computes
        (
            ap(Op::Max, vec![a.clone(), b.clone(), x()]),
            ap(Op::Max, vec![ap(Op::Max, vec![a.clone(), b.clone()]), x()]),
        ),
        (
            ap(Op::Min, vec![n(3.0), n(1.0), a.clone(), b.clone()]),
            ap(
                Op::Min,
                vec![ap(Op::Min, vec![n(1.0), a.clone()]), b.clone()],
            ),
        ),
    ] {
        assert_same(&fold(&e), &expected);
    }
    // not exact: x + 0 of -0 is +0, 0 * x of inf or NaN is NaN, x - x and x / x of inf or NaN,
    // x - (-0) of -0 is +0, 0 - (0 - x) of +0 is +0 and 1 * x of a negative 1 is no identity
    for e in [
        ap(Op::Plus, vec![a.clone(), n(0.0)]),
        ap(Op::Plus, vec![n(0.0), a.clone()]),
        ap(Op::Plus, vec![a.clone(), n(-0.0)]),
        ap(Op::Times, vec![n(0.0), a.clone()]),
        ap(Op::Times, vec![a.clone(), n(0.0)]),
        ap(Op::Times, vec![a.clone(), n(-1.0)]),
        ap(Op::Minus, vec![a.clone(), a.clone()]),
        ap(Op::Minus, vec![a.clone(), n(-0.0)]),
        ap(Op::Divide, vec![a.clone(), a.clone()]),
        ap(Op::Divide, vec![n(1.0), a.clone()]),
        ap(Op::Minus, vec![ap(Op::Minus, vec![a.clone()])]),
        ap(Op::Power, vec![a.clone(), n(1.0)]),
        ap(Op::Max, vec![a.clone(), a.clone()]),
    ] {
        assert_same(&fold(&e), &e);
    }
}

#[test]
fn literals_fold_into_the_doubles_of_the_code() {
    let cases = [
        ap(Op::Exp, vec![n(1.0)]),
        ap(Op::Log, vec![n(1000.0)]),
        ap(Op::Log, vec![n(10.0), n(1000.0)]),
        ap(Op::Root, vec![n(-0.0)]),
        ap(Op::Root, vec![n(3.0), n(27.0)]),
        ap(Op::Max, vec![n(f64::NAN), n(1.0)]),
        ap(Op::Max, vec![n(0.0), n(-0.0)]),
        ap(Op::Minus, vec![n(0.0)]),
        ap(Op::Divide, vec![n(0.0), n(0.0)]),
        ap(Op::Quotient, vec![n(7.0), n(0.0)]),
        ap(Op::Arccot, vec![n(-0.0)]),
        ap(Op::Arcsech, vec![n(0.7)]),
        ap(Op::Plus, vec![]),
        ap(Op::Times, vec![]),
        ap(Op::Min, vec![]),
        ap(Op::Times, vec![n(0.1), n(0.2), n(0.3)]),
    ];
    for e in cases {
        let folded = fold(&e);
        let Expr::Number(v) = folded else {
            panic!("{e:?} folds to {folded:?}")
        };
        let expected = value(&e);
        assert!(
            v.to_bits() == expected.to_bits() || (v.is_nan() && expected.is_nan()),
            "{e:?}: {v:?} != {expected:?}"
        );
    }
    // relations and boolean operators of literals are booleans
    assert_eq!(
        fold(&ap(Op::Lt, vec![n(f64::NAN), n(1.0)])),
        Expr::Bool(true)
    );
    assert_eq!(
        fold(&ap(Op::Eq, vec![n(1.0), n(1.0), n(2.0)])),
        Expr::Bool(false)
    );
    assert_eq!(
        fold(&ap(Op::And, vec![Expr::Bool(true), n(2.0)])),
        Expr::Bool(true)
    );
    // a piecewise drops the pieces whose literal condition is false and ends at a true one
    let pw = Expr::Piecewise {
        pieces: vec![
            (p(0), ap(Op::Gt, vec![n(1.0), n(2.0)])),
            (p(1), ap(Op::Gt, vec![x(), n(0.0)])),
            (p(2), n(f64::NAN)),
            (p(3), n(-1.0)),
            (p(4), x()),
        ],
        otherwise: Some(Box::new(p(5))),
    };
    assert_same(
        &fold(&pw),
        &Expr::Piecewise {
            pieces: vec![(p(1), ap(Op::Gt, vec![x(), n(0.0)]))],
            otherwise: Some(Box::new(p(3))),
        },
    );
    let none = Expr::Piecewise {
        pieces: vec![(p(0), Expr::Bool(false))],
        otherwise: None,
    };
    assert!(matches!(fold(&none), Expr::Number(v) if v.is_nan()));
    let first = Expr::Piecewise {
        pieces: vec![(p(0), n(-0.5)), (p(1), x())],
        otherwise: None,
    };
    assert_same(&fold(&first), &p(0));
    // never across a symbol
    let e = ap(Op::Plus, vec![x(), ap(Op::Times, vec![n(2.0), n(3.0)])]);
    assert_same(&fold(&e), &ap(Op::Plus, vec![x(), n(6.0)]));
}

#[test]
fn cse_shares_subtrees() {
    let k = p(0);
    let ekt = ap(Op::Exp, vec![ap(Op::Times, vec![k.clone(), t()])]);
    let outs = [
        ap(Op::Times, vec![ekt.clone(), x()]),
        ap(Op::Plus, vec![ekt.clone(), x()]),
    ];
    let mut commons = 0;
    let prog = program(&[], &outs, true, &mut commons);
    assert_eq!(count(&prog, Op::Exp), 1, "{prog:?}");
    assert_eq!(count(&prog, Op::Times), 2);
    assert_eq!(commons, 1);
    assert_eq!(lets(&prog), vec![(Symbol::Common(0), &ekt)]);
    assert_same(outputs(&prog)[0], &ap(Op::Times, vec![common(0), x()]));
    assert_same(outputs(&prog)[1], &ap(Op::Plus, vec![common(0), x()]));
    // the indices of the common subexpressions continue
    let prog = program(&[], &outs, true, &mut commons);
    assert_eq!(lets(&prog), vec![(Symbol::Common(1), &ekt)]);
    // without CSE every occurrence is computed
    let mut none = 0;
    let prog = program(&[], &outs, false, &mut none);
    assert_eq!(count(&prog, Op::Exp), 2);
    assert_eq!(none, 0);

    // within one output, and the square of a derivative
    let cos = ap(Op::Cos, vec![x()]);
    let square = ap(
        Op::Divide,
        vec![n(1.0), ap(Op::Times, vec![cos.clone(), cos.clone()])],
    );
    let prog = program(&[], &[square], true, &mut 0);
    assert_eq!(count(&prog, Op::Cos), 1);

    // the bits of a literal are part of a subtree: x * 0 and x * -0 differ
    let a = ap(Op::Exp, vec![ap(Op::Times, vec![x(), n(0.0)])]);
    let b = ap(Op::Exp, vec![ap(Op::Times, vec![x(), n(-0.0)])]);
    let prog = program(&[], &[a.clone(), b], true, &mut 0);
    assert_eq!(count(&prog, Op::Exp), 2);
    let prog = program(&[], &[a.clone(), a], true, &mut 0);
    assert_eq!(count(&prog, Op::Exp), 1);

    // the prefixes of an n-ary max, as its derivative reads them, are the subtrees of its value
    let (a, b, c, d) = (p(0), p(1), p(2), x());
    let value = ap(Op::Max, vec![a.clone(), b.clone(), c.clone(), d.clone()]);
    let prefix = ap(Op::Max, vec![a.clone(), b.clone(), c.clone()]);
    let condition = ap(Op::Gt, vec![prefix, d.clone()]);
    let prog = program(&[], &[fold(&value), fold(&condition)], true, &mut 0);
    assert_eq!(count(&prog, Op::Max), 3, "{prog:?}");
}

#[test]
fn a_subtree_only_in_lazy_branches_is_not_hoisted() {
    let e = ap(Op::Exp, vec![x()]);
    let lazy = Expr::Piecewise {
        pieces: vec![(e.clone(), ap(Op::Gt, vec![x(), n(0.0)]))],
        otherwise: Some(Box::new(ap(Op::Times, vec![n(2.0), e.clone()]))),
    };
    let prog = program(&[], std::slice::from_ref(&lazy), true, &mut 0);
    assert!(lets(&prog).is_empty(), "{prog:?}");
    assert_same(outputs(&prog)[0], &lazy);
    // in the first condition it is computed in any case, so it is shared with the branches
    let eager = Expr::Piecewise {
        pieces: vec![(e.clone(), ap(Op::Gt, vec![e.clone(), n(1.0)]))],
        otherwise: Some(Box::new(ap(Op::Times, vec![n(2.0), e.clone()]))),
    };
    let prog = program(&[], &[eager], true, &mut 0);
    assert_eq!(count(&prog, Op::Exp), 1);
    // and so is one which another output computes
    let prog = program(&[], &[lazy, e], true, &mut 0);
    assert_eq!(count(&prog, Op::Exp), 1);
    // a boolean is no common subexpression (the locals are doubles)
    let rel = ap(Op::Gt, vec![x(), n(0.0)]);
    let both = [
        ap(Op::And, vec![rel.clone(), ap(Op::Lt, vec![x(), n(9.0)])]),
        ap(Op::Or, vec![rel.clone(), ap(Op::Lt, vec![x(), n(-9.0)])]),
    ];
    let prog = program(&[], &both, true, &mut 0);
    assert!(lets(&prog).is_empty(), "{prog:?}");
}

fn entity(id: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Parameter,
        default: 1.0,
        settable: true,
        unit: None,
    }
}

#[test]
fn unread_rules_are_not_computed() {
    // a = 2 * x, b = a * a, c = exp(x) (read by nothing but the selections)
    let rules = vec![
        Assignment {
            target: Symbol::Param(1),
            expr: ap(Op::Times, vec![n(2.0), x()]),
        },
        Assignment {
            target: Symbol::Param(2),
            expr: ap(Op::Times, vec![p(1), p(1)]),
        },
        Assignment {
            target: Symbol::Param(3),
            expr: ap(Op::Exp, vec![x()]),
        },
    ];
    let prog = program(&rules, &[ap(Op::Minus, vec![p(2)])], true, &mut 0);
    let targets: Vec<Symbol> = lets(&prog).iter().map(|(s, _)| *s).collect();
    assert_eq!(targets, [Symbol::Param(1), Symbol::Param(2)]);
    let prog = program(&rules, &[p(0)], true, &mut 0);
    assert!(lets(&prog).is_empty());

    let model = Model {
        n_algebraic: 0,
        id: Some("m".to_string()),
        states: vec![entity("x")],
        parameters: ["k", "a", "b", "c"].iter().map(|s| entity(s)).collect(),
        assignment_rules: rules,
        initial: vec![],
        rates: vec![ap(Op::Minus, vec![ap(Op::Times, vec![n(1.0), p(2)])])],
        events: vec![],
        selections: (0..4)
            .map(|i| sbmlsim_ir::SelectionDef {
                id: format!("s{i}"),
                kind: sbmlsim_ir::SelectionKind::Parameter,
                expr: p(i),
                unit: None,
            })
            .collect(),
    };
    let plan = plan(
        &model,
        &jacobian(&model),
        &model.discontinuity_roots(),
        true,
    );
    let rhs: Vec<Symbol> = lets(&plan.rhs).iter().map(|(s, _)| *s).collect();
    assert_eq!(rhs, [Symbol::Param(1), Symbol::Param(2)]);
    assert_same(outputs(&plan.rhs)[0], &ap(Op::Minus, vec![p(2)]));
    // the Jacobian reads the rules a and b through their derivatives
    let jac = plan.jac.as_ref().expect("an analytic Jacobian");
    assert!(
        lets(jac).iter().all(|(s, _)| *s != Symbol::Param(3)),
        "{jac:?}"
    );
    // every selection is an output, so every rule is computed
    assert_eq!(outputs(&plan.selections).len(), 4);
    assert_eq!(lets(&plan.selections).len(), 3);
}

fn constant(k: u32) -> Expr {
    Expr::Symbol(Symbol::Constant(k))
}

#[test]
fn subtrees_of_the_parameters_alone_are_constants() {
    // parameters k, V, BW, b (b = pow(BW, 0.75), a rule of parameters alone) and r (r = x * k,
    // a rule of the state); x' = k * V * b * x - r + x * k * V
    let rules = vec![
        Assignment {
            target: Symbol::Param(3),
            expr: ap(Op::Power, vec![p(2), n(0.75)]),
        },
        Assignment {
            target: Symbol::Param(4),
            expr: ap(Op::Times, vec![x(), p(0)]),
        },
    ];
    let rate = ap(
        Op::Plus,
        vec![
            ap(
                Op::Minus,
                vec![ap(Op::Times, vec![p(0), p(1), p(3), x()]), p(4)],
            ),
            ap(Op::Times, vec![x(), p(0), p(1)]),
        ],
    );
    let model = Model {
        n_algebraic: 0,
        id: Some("m".to_string()),
        states: vec![entity("x")],
        parameters: ["k", "V", "BW", "b", "r"]
            .iter()
            .map(|s| entity(s))
            .collect(),
        assignment_rules: rules,
        initial: vec![],
        rates: vec![rate],
        events: vec![],
        selections: vec![],
    };
    // without the common subexpressions, which would share `k * V * b` in the constants
    let plan = plan(&model, &jacobian(&model), &[], false);
    // the rule b is the constant 0, the leading factors `k * V * b` another (and the entry of
    // the Jacobian, of the parameters alone, a third); the rule r is a let of the state;
    // `x * k * V` is `(x * k) * V` and stays
    assert_eq!(plan.n_constants(), 3, "{:?}", plan.constants);
    let constants = outputs(&plan.constants);
    assert_same(constants[0], &ap(Op::Power, vec![p(2), n(0.75)]));
    let kvb = ap(Op::Times, vec![p(0), p(1), constant(0)]);
    let k = constants
        .iter()
        .position(|c| same(c, &kvb))
        .expect("k * V * b") as u32;
    let rhs: Vec<Symbol> = lets(&plan.rhs).iter().map(|(s, _)| *s).collect();
    assert_eq!(rhs, [Symbol::Param(4)]);
    assert_same(
        outputs(&plan.rhs)[0],
        &ap(
            Op::Plus,
            vec![
                ap(Op::Minus, vec![ap(Op::Times, vec![constant(k), x()]), p(4)]),
                ap(Op::Times, vec![x(), p(0), p(1)]),
            ],
        ),
    );
    // the entry of the Jacobian is a constant, and no function computes pow
    let jac = plan.jac.as_ref().expect("an analytic Jacobian");
    assert!(
        matches!(outputs(jac)[0], Expr::Symbol(Symbol::Constant(_))),
        "{jac:?}"
    );
    assert_eq!(count(jac, Op::Power), 0, "{jac:?}");
    assert_eq!(count(&plan.rhs, Op::Power), 0);
    // the constants evaluate to the doubles of the code as written
    let read = |s: Symbol| match s {
        Symbol::Param(2) => 70.0,
        Symbol::Param(_) => 1.5,
        _ => unreachable!("{s:?}"),
    };
    assert_eq!(
        eval(constants[0], &read).to_bits(),
        sbmlsim_mathlib::f::math_pow(70.0, 0.75).to_bits()
    );
}

#[test]
fn a_parameter_of_a_rule_of_the_time_is_no_constant() {
    // a = 2 * time, x' = -(a * k) * x: `a * k` varies, k alone is a symbol
    let rules = vec![Assignment {
        target: Symbol::Param(1),
        expr: ap(Op::Times, vec![n(2.0), t()]),
    }];
    let model = Model {
        n_algebraic: 0,
        id: Some("m".to_string()),
        states: vec![entity("x")],
        parameters: ["k", "a"].iter().map(|s| entity(s)).collect(),
        assignment_rules: rules,
        initial: vec![],
        rates: vec![ap(
            Op::Times,
            vec![ap(Op::Minus, vec![ap(Op::Times, vec![p(1), p(0)])]), x()],
        )],
        events: vec![],
        selections: vec![],
    };
    let plan = plan(&model, &jacobian(&model), &[], true);
    assert_eq!(plan.n_constants(), 0, "{:?}", plan.constants);
}

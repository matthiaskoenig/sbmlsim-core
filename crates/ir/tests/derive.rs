//! The Jacobian of `sbmlsim_ir::derive::jacobian`: the derivatives of the operators, the
//! assignment rules, the pattern, the fallback to the colored difference quotient and the
//! coloring.

use std::path::Path;

use sbmlsim_abi::SparsityPattern;
use sbmlsim_ir::derive::{JacobianIr, jacobian, parameter_jacobian};
use sbmlsim_ir::{Assignment, Entity, EntityKind, Expr, Model, Op, Symbol, build};

/// A random generator of the tests (xorshift64*), seeded.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    fn uniform(&mut self, (lo, hi): (f64, f64)) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

fn entity(id: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Parameter,
        default: 0.0,
        settable: true,
        unit: None,
    }
}

/// A model of the states `x`, `y`, ... with the given rates, parameters and assignment rules.
fn model(states: &[&str], params: &[&str], rules: Vec<Assignment>, rates: Vec<Expr>) -> Model {
    Model {
        n_algebraic: 0,
        id: Some("m".to_string()),
        states: states.iter().map(|s| entity(s)).collect(),
        parameters: params.iter().map(|s| entity(s)).collect(),
        assignment_rules: rules,
        initial: vec![],
        rates,
        events: vec![],
        selections: vec![],
        rate_rules: vec![],
        rate_of: vec![],
        driven: vec![],
        t_ref: None,
        time_unit: None,
        units: Default::default(),
        unit_definitions: Default::default(),
    }
}

fn x() -> Expr {
    Expr::Symbol(Symbol::State(0))
}

fn y() -> Expr {
    Expr::Symbol(Symbol::State(1))
}

fn n(v: f64) -> Expr {
    Expr::Number(v)
}

fn ap(op: Op, args: Vec<Expr>) -> Expr {
    Expr::apply(op, args)
}

/// The value of an expression with the meanings of the emitted code (`sbmlsim-codegen`'s
/// `math.rs`), the derivatives `d` already evaluated.
fn eval(e: &Expr, y: &[f64], p: &[f64], d: &[f64]) -> f64 {
    let v = |e: &Expr| eval(e, y, p, d);
    let b = |e: &Expr| {
        let x = v(e);
        x != 0.0 && !x.is_nan()
    };
    match e {
        Expr::Number(x) => *x,
        Expr::Bool(t) => f64::from(u8::from(*t)),
        Expr::Symbol(Symbol::State(i)) => y[*i as usize],
        Expr::Symbol(Symbol::Param(i)) => p[*i as usize],
        Expr::Symbol(Symbol::Time) => 0.0,
        Expr::Symbol(Symbol::Derivative(k)) => d[*k as usize],
        Expr::Symbol(Symbol::Common(_) | Symbol::Constant(_)) => {
            unreachable!("no symbol of the optimizer")
        }
        Expr::Piecewise { pieces, otherwise } => {
            for (value, condition) in pieces {
                if b(condition) {
                    return v(value);
                }
            }
            otherwise.as_deref().map_or(f64::NAN, v)
        }
        Expr::Apply { op, args } => {
            let a: Vec<f64> = args.iter().map(v).collect();
            let rel = |f: fn(f64, f64) -> bool| f64::from(u8::from(f(a[0], a[1])));
            match op {
                Op::Plus => a.iter().sum(),
                Op::Minus if a.len() == 1 => 0.0 - a[0],
                Op::Minus => a[0] - a[1],
                Op::Times => a.iter().product(),
                Op::Divide => a[0] / a[1],
                Op::Power => a[0].powf(a[1]),
                Op::Root if a.len() == 1 => a[0].powf(0.5),
                Op::Root => a[1].powf(1.0 / a[0]),
                Op::Abs => a[0].abs(),
                Op::Exp => a[0].exp(),
                Op::Ln => a[0].ln(),
                Op::Log if a.len() == 1 => a[0].ln() / 10f64.ln(),
                Op::Log => a[1].ln() / a[0].ln(),
                Op::Floor => a[0].floor(),
                Op::Ceiling => a[0].ceil(),
                Op::Sin => a[0].sin(),
                Op::Cos => a[0].cos(),
                Op::Tan => a[0].tan(),
                Op::Sec => 1.0 / a[0].cos(),
                Op::Csc => 1.0 / a[0].sin(),
                Op::Cot => 1.0 / a[0].tan(),
                Op::Sinh => a[0].sinh(),
                Op::Cosh => a[0].cosh(),
                Op::Tanh => a[0].tanh(),
                Op::Sech => 1.0 / a[0].cosh(),
                Op::Csch => 1.0 / a[0].sinh(),
                Op::Coth => a[0].cosh() / a[0].sinh(),
                Op::Arcsin => a[0].asin(),
                Op::Arccos => a[0].acos(),
                Op::Arctan => a[0].atan(),
                Op::Arcsec => (1.0 / a[0]).acos(),
                Op::Arccsc => (1.0 / a[0]).asin(),
                Op::Arccot => (1.0 / a[0]).atan(),
                Op::Arcsinh => a[0].asinh(),
                Op::Arccosh => a[0].acosh(),
                Op::Arctanh => a[0].atanh(),
                Op::Arcsech => (1.0 / a[0]).acosh(),
                Op::Arccsch => (1.0 / a[0]).asinh(),
                Op::Arccoth => (1.0 / a[0]).atanh(),
                Op::Max => a
                    .iter()
                    .copied()
                    .reduce(|m, x| if m > x { m } else { x })
                    .unwrap(),
                Op::Min => a
                    .iter()
                    .copied()
                    .reduce(|m, x| if m < x { m } else { x })
                    .unwrap(),
                Op::Rem => a[0] % a[1],
                Op::Quotient => (a[0] / a[1]).trunc(),
                Op::Factorial => panic!("factorial is not evaluated here"),
                Op::Gt => rel(|a, b| a > b),
                Op::Lt => rel(|a, b| a < b),
                Op::Geq => rel(|a, b| a >= b),
                Op::Leq => rel(|a, b| a <= b),
                Op::Eq => rel(|a, b| a == b),
                Op::Neq => rel(|a, b| a != b),
                Op::And => f64::from(u8::from(args.iter().all(b))),
                Op::Or => f64::from(u8::from(args.iter().any(b))),
                Op::Not => f64::from(u8::from(!b(&args[0]))),
                Op::Xor | Op::Implies | Op::XorSign | Op::Delay | Op::HistoryQuery => {
                    unimplemented!()
                }
            }
        }
    }
}

/// The rates of a model at `y` with the parameters `p`, the assignment rules evaluated first.
fn rates(m: &Model, y: &[f64], p: &[f64]) -> Vec<f64> {
    let mut p = p.to_vec();
    for a in &m.assignment_rules {
        let value = eval(&a.expr, y, &p, &[]);
        let Symbol::Param(i) = a.target else {
            unreachable!()
        };
        p[i as usize] = value;
    }
    m.rates.iter().map(|r| eval(r, y, &p, &[])).collect()
}

/// The entries of the analytic Jacobian at `y`, in the CSC order of its pattern.
fn analytic(m: &Model, y: &[f64], p: &[f64]) -> (SparsityPattern, Vec<f64>) {
    let JacobianIr::Analytic {
        pattern,
        derivatives,
        entries,
    } = jacobian(m)
    else {
        panic!("no analytic Jacobian")
    };
    let mut p = p.to_vec();
    for a in &m.assignment_rules {
        let value = eval(&a.expr, y, &p, &[]);
        let Symbol::Param(i) = a.target else {
            unreachable!()
        };
        p[i as usize] = value;
    }
    let mut d = Vec::new();
    for e in &derivatives {
        let value = eval(e, y, &p, &d);
        d.push(value);
    }
    let values = entries.iter().map(|e| eval(e, y, &p, &d)).collect();
    (pattern, values)
}

/// The derivative of the rate `i` with respect to the state `j` by Richardson's extrapolation of
/// central differences (fourth order).
fn central(m: &Model, y: &[f64], p: &[f64], i: usize, j: usize) -> f64 {
    let h = 1e-3 * y[j].abs().max(1e-2);
    let at = |dx: f64| {
        let mut z = y.to_vec();
        z[j] += dx;
        rates(m, &z, p)[i]
    };
    (8.0 * (at(h) - at(-h)) - (at(2.0 * h) - at(-2.0 * h))) / (12.0 * h)
}

/// Every entry of the analytic Jacobian of `m` agrees with the central differences at random
/// points of the domains of the states within `1e-7` relative.
fn check_against_differences(name: &str, m: &Model, domains: &[(f64, f64)], rng: &mut Rng) {
    for _ in 0..20 {
        let y: Vec<f64> = domains.iter().map(|d| rng.uniform(*d)).collect();
        let p = vec![0.0; m.parameters.len()];
        let (pattern, values) = analytic(m, &y, &p);
        for j in 0..y.len() {
            let rows = pattern.rows(j);
            let start = pattern.col_ptr[j] as usize;
            for i in 0..m.rates.len() {
                let difference = central(m, &y, &p, i, j);
                let a = match rows.iter().position(|r| *r as usize == i) {
                    Some(k) => values[start + k],
                    None => 0.0,
                };
                let scale = a.abs().max(difference.abs());
                assert!(
                    (a - difference).abs() <= 1e-7 * scale || (a - difference).abs() <= 1e-12,
                    "{name}: d rate {i} / d y{j} at {y:?}: analytic {a:e}, difference \
                     {difference:e}"
                );
            }
        }
    }
}

#[test]
fn derivatives_of_operators() {
    use Op::*;
    let one = |op: Op| ap(op, vec![x()]);
    // every operator as the rate of x over the states x and y, with the domains of x and y
    type Case = (&'static str, Expr, (f64, f64), (f64, f64));
    let cases: Vec<Case> = vec![
        (
            "plus",
            ap(Plus, vec![x(), y(), x()]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        ("plus()", ap(Plus, vec![]), (-3.0, 3.0), (-3.0, 3.0)),
        ("negation", ap(Minus, vec![x()]), (-3.0, 3.0), (-3.0, 3.0)),
        ("minus", ap(Minus, vec![x(), y()]), (-3.0, 3.0), (-3.0, 3.0)),
        (
            "minus of y",
            ap(Minus, vec![n(2.0), y()]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "times",
            ap(Times, vec![x(), y(), x()]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "times 2",
            ap(Times, vec![n(2.0), x()]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "divide",
            ap(Divide, vec![x(), y()]),
            (-3.0, 3.0),
            (0.5, 3.0),
        ),
        (
            "reciprocal",
            ap(Divide, vec![n(1.0), x()]),
            (0.5, 3.0),
            (0.5, 3.0),
        ),
        ("power", ap(Power, vec![x(), y()]), (0.5, 3.0), (-2.0, 2.0)),
        (
            "power of a number",
            ap(Power, vec![x(), n(2.5)]),
            (0.5, 3.0),
            (0.5, 3.0),
        ),
        (
            "power of x",
            ap(Power, vec![n(2.0), x()]),
            (-2.0, 2.0),
            (0.5, 3.0),
        ),
        ("root", one(Root), (0.5, 3.0), (0.5, 3.0)),
        (
            "cube root",
            ap(Root, vec![n(3.0), x()]),
            (0.5, 3.0),
            (0.5, 3.0),
        ),
        (
            "root of degree y",
            ap(Root, vec![y(), x()]),
            (0.5, 3.0),
            (1.5, 4.0),
        ),
        ("abs", one(Abs), (0.1, 3.0), (0.5, 3.0)),
        ("abs of a negative", one(Abs), (-3.0, -0.1), (0.5, 3.0)),
        ("exp", one(Exp), (-3.0, 3.0), (0.5, 3.0)),
        ("ln", one(Ln), (0.1, 5.0), (0.5, 3.0)),
        ("log", one(Log), (0.1, 5.0), (0.5, 3.0)),
        (
            "log of base y",
            ap(Log, vec![y(), x()]),
            (0.1, 5.0),
            (1.5, 5.0),
        ),
        (
            "floor",
            ap(Times, vec![one(Floor), x()]),
            (0.1, 0.9),
            (0.5, 3.0),
        ),
        (
            "ceiling",
            ap(Times, vec![one(Ceiling), x()]),
            (1.1, 1.9),
            (0.5, 3.0),
        ),
        ("rem", ap(Rem, vec![x(), y()]), (3.1, 3.9), (1.2, 1.4)),
        (
            "rem of a negative",
            ap(Rem, vec![x(), y()]),
            (-3.9, -3.1),
            (1.2, 1.4),
        ),
        (
            "quotient",
            ap(Times, vec![ap(Quotient, vec![x(), y()]), y()]),
            (3.1, 3.9),
            (1.2, 1.4),
        ),
        ("sin", one(Sin), (-3.0, 3.0), (0.5, 3.0)),
        ("cos", one(Cos), (-3.0, 3.0), (0.5, 3.0)),
        ("tan", one(Tan), (-1.2, 1.2), (0.5, 3.0)),
        ("sec", one(Sec), (-1.2, 1.2), (0.5, 3.0)),
        ("csc", one(Csc), (0.3, 2.8), (0.5, 3.0)),
        ("cot", one(Cot), (0.3, 2.8), (0.5, 3.0)),
        ("sinh", one(Sinh), (-3.0, 3.0), (0.5, 3.0)),
        ("cosh", one(Cosh), (-3.0, 3.0), (0.5, 3.0)),
        ("tanh", one(Tanh), (-3.0, 3.0), (0.5, 3.0)),
        ("sech", one(Sech), (-3.0, 3.0), (0.5, 3.0)),
        ("csch", one(Csch), (0.2, 3.0), (0.5, 3.0)),
        ("coth", one(Coth), (0.2, 3.0), (0.5, 3.0)),
        ("arcsin", one(Arcsin), (-0.9, 0.9), (0.5, 3.0)),
        ("arccos", one(Arccos), (-0.9, 0.9), (0.5, 3.0)),
        ("arctan", one(Arctan), (-3.0, 3.0), (0.5, 3.0)),
        ("arcsec", one(Arcsec), (1.2, 4.0), (0.5, 3.0)),
        (
            "arcsec of a negative",
            one(Arcsec),
            (-4.0, -1.2),
            (0.5, 3.0),
        ),
        ("arccsc", one(Arccsc), (1.2, 4.0), (0.5, 3.0)),
        (
            "arccsc of a negative",
            one(Arccsc),
            (-4.0, -1.2),
            (0.5, 3.0),
        ),
        ("arccot", one(Arccot), (0.2, 3.0), (0.5, 3.0)),
        (
            "arccot of a negative",
            one(Arccot),
            (-3.0, -0.2),
            (0.5, 3.0),
        ),
        ("arcsinh", one(Arcsinh), (-3.0, 3.0), (0.5, 3.0)),
        ("arccosh", one(Arccosh), (1.2, 4.0), (0.5, 3.0)),
        ("arctanh", one(Arctanh), (-0.9, 0.9), (0.5, 3.0)),
        ("arcsech", one(Arcsech), (0.1, 0.9), (0.5, 3.0)),
        ("arccsch", one(Arccsch), (0.2, 3.0), (0.5, 3.0)),
        (
            "arccsch of a negative",
            one(Arccsch),
            (-3.0, -0.2),
            (0.5, 3.0),
        ),
        ("arccoth", one(Arccoth), (1.2, 4.0), (0.5, 3.0)),
        ("max", ap(Max, vec![x(), y()]), (-3.0, 3.0), (-3.0, 3.0)),
        ("min", ap(Min, vec![x(), y()]), (-3.0, 3.0), (-3.0, 3.0)),
        (
            "max of three",
            ap(Max, vec![x(), y(), ap(Times, vec![x(), x()])]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "min of three",
            ap(Min, vec![y(), n(0.5), ap(Times, vec![x(), y()])]),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "piecewise",
            Expr::Piecewise {
                pieces: vec![
                    (ap(Times, vec![x(), x()]), ap(Gt, vec![x(), y()])),
                    (one(Sin), ap(Lt, vec![x(), n(0.5)])),
                ],
                otherwise: Some(Box::new(ap(Times, vec![y(), x()]))),
            },
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "relations",
            ap(
                Times,
                vec![
                    ap(
                        Plus,
                        vec![
                            ap(Gt, vec![x(), y()]),
                            ap(And, vec![ap(Lt, vec![x(), n(1.0)])]),
                        ],
                    ),
                    x(),
                ],
            ),
            (-3.0, 3.0),
            (-3.0, 3.0),
        ),
        (
            "composition",
            ap(
                Divide,
                vec![
                    ap(Sin, vec![ap(Times, vec![ap(Exp, vec![x()]), y()])]),
                    ap(Plus, vec![n(1.0), ap(Power, vec![x(), n(2.0)])]),
                ],
            ),
            (-2.0, 2.0),
            (-2.0, 2.0),
        ),
    ];
    let mut rng = Rng(0x5EED_0015);
    for (name, rate, dx, dy) in cases {
        let m = model(&["x", "y"], &[], vec![], vec![rate.clone(), rate]);
        check_against_differences(name, &m, &[dx, dy], &mut rng);
    }
}

#[test]
fn a_power_of_a_number_has_no_logarithm() {
    // the derivative of x^c is c * x^(c - 1) * x', without the term ln(x) * c' (structurally
    // zero), which would be NaN at x = 0
    let m = model(&["x"], &[], vec![], vec![ap(Op::Power, vec![x(), n(2.5)])]);
    let JacobianIr::Analytic { entries, .. } = jacobian(&m) else {
        panic!()
    };
    assert_eq!(
        entries,
        [ap(
            Op::Times,
            vec![n(2.5), ap(Op::Power, vec![x(), n(1.5)])]
        )]
    );
    assert_eq!(analytic(&m, &[0.0], &[]).1, [0.0]);
}

#[test]
fn a_rule_chain_is_differentiated_rule_by_rule() {
    // x' = -k * a with the rules b = x * x + y and a = 2 * b * b (in their order of dependency),
    // y' = 1: d x' / d x = -k * 2 * 2 * b * 2 * x, d x' / d y = -k * 2 * 2 * b
    let (k, a, b) = (
        Expr::Symbol(Symbol::Param(0)),
        Expr::Symbol(Symbol::Param(1)),
        Expr::Symbol(Symbol::Param(2)),
    );
    let rules = vec![
        Assignment {
            target: Symbol::Param(2),
            expr: ap(Op::Plus, vec![ap(Op::Times, vec![x(), x()]), y()]),
        },
        Assignment {
            target: Symbol::Param(1),
            expr: ap(Op::Times, vec![n(2.0), b.clone(), b]),
        },
    ];
    let rates = vec![ap(Op::Minus, vec![ap(Op::Times, vec![k, a])]), n(1.0)];
    let m = model(&["x", "y"], &["k", "a", "b"], rules, rates);
    let JacobianIr::Analytic {
        pattern,
        derivatives,
        entries,
    } = jacobian(&m)
    else {
        panic!()
    };
    // the rate of x depends on x and y through both rules, the rate of y on nothing
    assert_eq!(pattern.col_ptr, [0, 1, 2]);
    assert_eq!(pattern.row_idx, [0, 0]);
    // d b / d x = x + x is a derivative of its own, which d a / d x reads; d b / d y = 1 is
    // used where it is read, so d a / d y = 2 * b + 2 * b; the entries read d a / d x and
    // d a / d y
    let derivative = |k: u32| Expr::Symbol(Symbol::Derivative(k));
    let b = Expr::Symbol(Symbol::Param(2));
    assert_eq!(
        derivatives,
        [
            ap(Op::Plus, vec![x(), x()]),
            ap(
                Op::Plus,
                vec![
                    ap(Op::Times, vec![n(2.0), derivative(0), b.clone()]),
                    ap(Op::Times, vec![n(2.0), b.clone(), derivative(0)]),
                ]
            ),
            ap(
                Op::Plus,
                vec![
                    ap(Op::Times, vec![n(2.0), b.clone()]),
                    ap(Op::Times, vec![n(2.0), b]),
                ]
            ),
        ]
    );
    let k = Expr::Symbol(Symbol::Param(0));
    assert_eq!(
        entries,
        [
            ap(
                Op::Minus,
                vec![ap(Op::Times, vec![k.clone(), derivative(1)])]
            ),
            ap(Op::Minus, vec![ap(Op::Times, vec![k, derivative(2)])]),
        ]
    );
    let mut rng = Rng(7);
    for _ in 0..20 {
        let (xv, yv, kv) = (rng.uniform((-2.0, 2.0)), rng.uniform((-2.0, 2.0)), 0.7);
        let (_, values) = analytic(&m, &[xv, yv], &[kv, 0.0, 0.0]);
        let bv = xv * xv + yv;
        let expected = [-kv * 4.0 * bv * 2.0 * xv, -kv * 4.0 * bv];
        for (v, e) in values.iter().zip(expected) {
            assert!(
                (v - e).abs() <= 1e-12 * e.abs().max(1.0),
                "{values:?} {expected:?}"
            );
        }
    }
    // a rule which no rate reads is not differentiated
    let mut unread = m.clone();
    unread.rates[0] = n(0.0);
    let JacobianIr::Analytic { derivatives, .. } = jacobian(&unread) else {
        panic!()
    };
    assert!(derivatives.is_empty());
}

#[test]
fn factorial_falls_back() {
    let factorial = ap(Op::Factorial, vec![x()]);
    let m = model(&["x"], &[], vec![], vec![factorial.clone()]);
    match jacobian(&m) {
        JacobianIr::ColoredDifference {
            pattern,
            colors,
            reason,
        } => {
            assert_eq!(reason, "factorial has no derivative (the rate of x)");
            assert_eq!(pattern.row_idx, [0]);
            assert_eq!(colors, [0]);
        }
        other => panic!("{other:?}"),
    }
    // in an assignment rule a rate reads
    let rule = Assignment {
        target: Symbol::Param(0),
        expr: factorial,
    };
    let m = model(
        &["x"],
        &["f"],
        vec![rule.clone()],
        vec![Expr::Symbol(Symbol::Param(0))],
    );
    let JacobianIr::ColoredDifference { reason, .. } = jacobian(&m) else {
        panic!()
    };
    assert_eq!(
        reason,
        "factorial has no derivative (the assignment rule of f)"
    );
    // a factorial of a parameter has the derivative 0, and one in a rule no rate reads is not
    // differentiated
    let constant = ap(Op::Times, vec![ap(Op::Factorial, vec![n(3.0)]), x()]);
    assert!(matches!(
        jacobian(&model(&["x"], &[], vec![], vec![constant])),
        JacobianIr::Analytic { .. }
    ));
    assert!(matches!(
        jacobian(&model(&["x"], &["f"], vec![rule], vec![x()])),
        JacobianIr::Analytic { .. }
    ));
}

/// The models of the prepared cases of the SBML Test Suite which build.
fn suite_models() -> Vec<(String, Model)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        assert!(
            std::env::var_os("SBMLSIM_CORE_REQUIRE_CASES").is_none(),
            "{} is absent and SBMLSIM_CORE_REQUIRE_CASES is set",
            dir.display()
        );
        eprintln!("note: {} is absent; skipped", dir.display());
        return vec![];
    };
    let mut models = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        let Ok(sbml) = std::fs::read_to_string(path.join("model.xml")) else {
            continue;
        };
        let Ok(doc) = sbmlsim_sbml::read(&sbml) else {
            continue;
        };
        if let Ok(m) = build(&doc) {
            models.push((path.file_name().unwrap().to_string_lossy().into_owned(), m));
        }
    }
    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

/// Checks without `check_colors` that the columns of a color share no row.
fn assert_colors_share_no_row(name: &str, pattern: &SparsityPattern, colors: &[u32]) {
    let n = pattern.n as usize;
    assert_eq!(colors.len(), n, "{name}");
    for c in 0..=colors.iter().copied().max().unwrap_or(0) {
        let mut seen = vec![false; n];
        for j in (0..n).filter(|&j| colors[j] == c) {
            for &i in pattern.rows(j) {
                assert!(!seen[i as usize], "{name}: color {c} has row {i} twice");
                seen[i as usize] = true;
            }
        }
    }
}

#[test]
fn colors_share_no_row() {
    // the patterns of the suite
    let models = suite_models();
    let mut colored = 0;
    for (name, m) in &models {
        let pattern = sbmlsim_ir::derive::pattern(m);
        assert!(pattern.is_csc_of(m.states.len()), "{name}");
        let colors = pattern.greedy_colors();
        assert_colors_share_no_row(name, &pattern, &colors);
        pattern.check_colors(&colors).unwrap();
        if m.states.len() > 1 {
            let n_colors = colors.iter().max().map_or(0, |c| c + 1) as usize;
            colored += usize::from(n_colors < m.states.len());
        }
    }
    if !models.is_empty() {
        assert!(models.len() > 1500, "{} models", models.len());
        assert!(
            colored > 100,
            "{colored} models with fewer colors than states"
        );
    }
    // random patterns of 1 to 60 columns, from sparse to dense
    let mut rng = Rng(42);
    for trial in 0..300 {
        let n = 1 + (rng.next() * 60.0) as usize;
        let density = rng.next();
        let mut col_ptr = vec![0u32];
        let mut row_idx = Vec::new();
        for _ in 0..n {
            for i in 0..n {
                if rng.next() < density * density {
                    row_idx.push(i as u32);
                }
            }
            col_ptr.push(row_idx.len() as u32);
        }
        let pattern = SparsityPattern {
            n: n as u32,
            col_ptr,
            row_idx,
        };
        let colors = pattern.greedy_colors();
        assert_colors_share_no_row(&format!("random pattern {trial}"), &pattern, &colors);
        pattern.check_colors(&colors).unwrap();
    }
    // a diagonal needs one color, a full matrix one per column
    let diagonal = SparsityPattern {
        n: 4,
        col_ptr: vec![0, 1, 2, 3, 4],
        row_idx: vec![0, 1, 2, 3],
    };
    assert_eq!(diagonal.greedy_colors(), [0, 0, 0, 0]);
    let full = SparsityPattern {
        n: 3,
        col_ptr: vec![0, 3, 6, 9],
        row_idx: vec![0, 1, 2, 0, 1, 2, 0, 1, 2],
    };
    assert_eq!(full.greedy_colors(), [0, 1, 2]);
    // check_colors refuses a conflict, naming the columns and the row
    assert_eq!(
        full.check_colors(&[0, 1, 0]),
        Err("the columns 0 and 2 of the color 0 share the row 0".to_string())
    );
}

#[test]
fn the_pattern_of_the_suite_is_sparse() {
    // no dense n * n pattern: the entries are the dependencies of the rates
    let models = suite_models();
    let (mut entries, mut dense) = (0usize, 0usize);
    for (_, m) in &models {
        let pattern = sbmlsim_ir::derive::pattern(m);
        entries += pattern.nnz();
        dense += m.states.len() * m.states.len();
    }
    if !models.is_empty() {
        assert!(entries < dense / 2, "{entries} entries of {dense}");
    }
}

/// The parameter `j` of a model.
fn param(j: u32) -> Expr {
    Expr::Symbol(Symbol::Param(j))
}

#[test]
fn the_columns_of_the_parameter_jacobian_are_the_rates_which_read_them() {
    // x' = -(k x), y' = c, with the parameter `rest` no rate reads
    let rates = vec![
        ap(Op::Minus, vec![ap(Op::Times, vec![param(0), x()])]),
        param(1),
    ];
    let m = model(&["x", "y"], &["k", "c", "rest"], vec![], rates);

    let jacobian = parameter_jacobian(&m).unwrap();

    assert_eq!(jacobian.pattern.rows, 2);
    assert_eq!(jacobian.pattern.col_ptr, [0, 1, 2, 2]);
    assert_eq!(jacobian.pattern.row_idx, [0, 1]);
    // d x'/dk = -x and d y'/dc = 1, each the only entry of its column
    assert_eq!(jacobian.columns[0].entries, [ap(Op::Minus, vec![x()])]);
    assert_eq!(jacobian.columns[1].entries, [n(1.0)]);
    assert!(jacobian.columns[2].entries.is_empty());
}

#[test]
fn a_parameter_a_rate_reads_through_an_assignment_rule_chains() {
    // x' = -(a x) with the rule a = 2 k: `a` is recomputed at every step, so it is no variable
    // of the jacobian and its column is empty, while the column of `k` is the chain
    // d x'/dk = -(da/dk x) = -(2 x)
    let rules = vec![Assignment {
        target: Symbol::Param(1),
        expr: ap(Op::Times, vec![n(2.0), param(0)]),
    }];
    let rates = vec![ap(Op::Minus, vec![ap(Op::Times, vec![param(1), x()])])];
    let m = model(&["x"], &["k", "a"], rules, rates);

    let jacobian = parameter_jacobian(&m).unwrap();

    assert_eq!(jacobian.pattern.col_ptr, [0, 1, 1]);
    assert_eq!(jacobian.pattern.row_idx, [0]);
    assert_eq!(
        jacobian.columns[0].entries,
        [ap(Op::Minus, vec![ap(Op::Times, vec![n(2.0), x()])])]
    );
    assert!(jacobian.columns[1].entries.is_empty());
}

#[test]
fn the_entries_of_the_parameter_jacobian_are_the_derivatives_of_the_rates() {
    // x' = k exp(-(d x)) + c x / (k + x), y' = sin(k y) * c, numerically against a central
    // difference of the rate in the parameter
    let (k, c, d) = (param(0), param(1), param(2));
    let rates = vec![
        ap(
            Op::Plus,
            vec![
                ap(
                    Op::Times,
                    vec![
                        k.clone(),
                        ap(
                            Op::Exp,
                            vec![ap(Op::Minus, vec![ap(Op::Times, vec![d, x()])])],
                        ),
                    ],
                ),
                ap(
                    Op::Divide,
                    vec![
                        ap(Op::Times, vec![c.clone(), x()]),
                        ap(Op::Plus, vec![k.clone(), x()]),
                    ],
                ),
            ],
        ),
        ap(
            Op::Times,
            vec![ap(Op::Sin, vec![ap(Op::Times, vec![k, y()])]), c],
        ),
    ];
    let m = model(&["x", "y"], &["k", "c", "d"], vec![], rates.clone());

    let jacobian = parameter_jacobian(&m).unwrap();

    let state = [0.7, 1.3];
    let p = [0.4, 1.1, 2.3];
    for (column, parameter) in jacobian.columns.iter().enumerate() {
        let mut derivatives: Vec<f64> = Vec::new();
        for expr in &parameter.derivatives {
            let value = eval(expr, &state, &p, &derivatives);
            derivatives.push(value);
        }
        for (at, row) in jacobian.pattern.rows(column).iter().enumerate() {
            let found = eval(&parameter.entries[at], &state, &p, &derivatives);
            let step = 1e-6;
            let (mut below, mut above) = (p, p);
            below[column] -= step;
            above[column] += step;
            let rate = &rates[*row as usize];
            let wanted =
                (eval(rate, &state, &above, &[]) - eval(rate, &state, &below, &[])) / (2.0 * step);
            assert!(
                (found - wanted).abs() < 1e-6 * wanted.abs().max(1e-6),
                "d rate {row} / d p{column} is {found} and not {wanted}"
            );
        }
    }
}

#[test]
fn a_rate_whose_parameter_has_no_derivative_has_no_parameter_jacobian() {
    let rates = vec![ap(Op::Times, vec![ap(Op::Factorial, vec![param(0)]), x()])];
    let m = model(&["x"], &["k"], vec![], rates);

    let refusal = parameter_jacobian(&m).unwrap_err();

    assert!(refusal.contains("factorial"), "{refusal}");
    assert!(refusal.contains("the rate of x"), "{refusal}");
}

#[test]
fn a_rule_chain_of_a_parameter_is_differentiated_rule_by_rule() {
    // x' = -(a x) with the rules b = exp(k) + c and a = b * b: `d a/dk` reads `d b/dk`, which
    // is a derivative of its own, and the entry of the column of `k` reads that one
    let (k, c, b) = (param(0), param(1), param(3));
    let rules = vec![
        Assignment {
            target: Symbol::Param(3),
            expr: ap(Op::Plus, vec![ap(Op::Exp, vec![k]), c]),
        },
        Assignment {
            target: Symbol::Param(2),
            expr: ap(Op::Times, vec![b.clone(), b]),
        },
    ];
    let rates = vec![ap(Op::Minus, vec![ap(Op::Times, vec![param(2), x()])])];
    let m = model(&["x"], &["k", "c", "a", "b"], rules, rates.clone());

    let jacobian = parameter_jacobian(&m).unwrap();

    // the columns of `k` and of `c` hold the rate of `x`, the ones of the rule targets nothing
    assert_eq!(jacobian.pattern.col_ptr, [0, 1, 2, 2, 2]);
    assert!(!jacobian.columns[0].derivatives.is_empty());
    // the value against a central difference of the rate, with the rules evaluated into `p`
    let state = [0.7];
    let values = |p: &[f64]| {
        let mut p = p.to_vec();
        p[3] = p[0].exp() + p[1];
        p[2] = p[3] * p[3];
        p
    };
    for column in [0usize, 1] {
        let p = values(&[0.4, 1.1, 0.0, 0.0]);
        let mut derivatives: Vec<f64> = Vec::new();
        for expr in &jacobian.columns[column].derivatives {
            let value = eval(expr, &state, &p, &derivatives);
            derivatives.push(value);
        }
        let found = eval(
            &jacobian.columns[column].entries[0],
            &state,
            &p,
            &derivatives,
        );
        let step = 1e-6;
        let (mut below, mut above) = ([0.4, 1.1, 0.0, 0.0], [0.4, 1.1, 0.0, 0.0]);
        below[column] -= step;
        above[column] += step;
        let wanted = (eval(&rates[0], &state, &values(&above), &[])
            - eval(&rates[0], &state, &values(&below), &[]))
            / (2.0 * step);
        assert!(
            (found - wanted).abs() < 1e-6 * wanted.abs().max(1e-6),
            "d x'/d p{column} is {found} and not {wanted}"
        );
    }
}

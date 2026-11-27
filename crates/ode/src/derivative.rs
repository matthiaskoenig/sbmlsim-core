//! Symbolic derivatives for rates of assigned compartment sizes.
use super::ast::*;
use sbmlsim_document::{Ast, AstKind as K};
pub(super) fn multiply(factors: Vec<Ast>) -> Ast {
    let mut flat = Vec::new();
    let mut coefficient = 1.0;
    let mut stack = factors;
    stack.reverse();
    while let Some(mut a) = stack.pop() {
        if a.kind == K::Times {
            stack.extend(std::mem::take(&mut a.children).into_iter().rev());
        } else if let Some(n) = value(&a) {
            coefficient *= n;
        } else {
            flat.push(a);
        }
    }
    if coefficient == 0.0 {
        return number(0.0);
    }
    if flat.is_empty() {
        return number(coefficient);
    }
    if coefficient == 1.0 {
        return product(flat);
    }
    if coefficient == -1.0 {
        return negate(product(flat));
    }
    flat.insert(0, number(coefficient));
    product(flat)
}
fn div(n: Ast, d: Ast) -> Ast {
    if is(&n, 0.0) || is(&d, 1.0) {
        n
    } else {
        node(K::Divide, vec![n, d])
    }
}
fn power(a: Ast, b: Ast) -> Ast {
    if is(&b, 0.0) {
        number(1.0)
    } else if is(&b, 1.0) {
        a
    } else {
        node(K::Power, vec![a, b])
    }
}
fn square(a: Ast) -> Ast {
    power(a, number(2.0))
}
fn root(a: Ast) -> Ast {
    node(K::FunctionRoot, vec![number(2.0), a])
}
fn call(k: K, a: Ast) -> Ast {
    node(k, vec![a])
}
fn quotient(u: Ast, v: Ast, du: Ast, dv: Ast) -> Ast {
    if is(&dv, 0.0) {
        div(du, v)
    } else {
        div(
            sum(vec![
                (1, multiply(vec![du, v.clone()])),
                (-1, multiply(vec![u, dv])),
            ]),
            square(v),
        )
    }
}
fn power_rate(u: Ast, v: Ast, du: Ast, dv: Ast) -> Ast {
    if is(&dv, 0.0) {
        if is(&du, 0.0) {
            return number(0.0);
        }
        let lower = value(&v).map_or_else(
            || sum(vec![(1, v.clone()), (-1, number(1.0))]),
            |n| number(n - 1.0),
        );
        multiply(vec![v, power(u, lower), du])
    } else {
        multiply(vec![
            power(u.clone(), v.clone()),
            sum(vec![
                (1, multiply(vec![dv, call(K::FunctionLn, u.clone())])),
                (1, multiply(vec![v, div(du, u)])),
            ]),
        ])
    }
}
pub(super) fn derivative(a: &Ast, variable: &str, depth: usize) -> Option<Ast> {
    if depth > 100 {
        return None;
    }
    if a.kind == K::Name {
        return Some(number(f64::from(a.name.as_deref() == Some(variable))));
    }
    if a.kind == K::NameTime {
        return Some(number(f64::from(variable == "@time")));
    }
    if value(a).is_some()
        || matches!(
            a.kind,
            K::ConstantE
                | K::ConstantPi
                | K::ConstantTrue
                | K::ConstantFalse
                | K::NameAvogadro
                | K::FunctionCeiling
                | K::FunctionFloor
                | K::LogicalAnd
                | K::LogicalOr
                | K::LogicalNot
                | K::LogicalXor
                | K::LogicalImplies
                | K::RelationalEq
                | K::RelationalNeq
                | K::RelationalLt
                | K::RelationalLeq
                | K::RelationalGt
                | K::RelationalGeq
        )
    {
        return Some(number(0.0));
    }
    if a.kind == K::FunctionPiecewise {
        let mut children = Vec::new();
        for (i, c) in a.children.iter().enumerate() {
            children.push(if i % 2 == 0 {
                derivative(c, variable, depth + 1)?
            } else {
                c.clone()
            });
        }
        return Some(if children.iter().step_by(2).all(|c| is(c, 0.0)) {
            number(0.0)
        } else {
            node(K::FunctionPiecewise, children)
        });
    }
    let d = a
        .children
        .iter()
        .map(|c| derivative(c, variable, depth + 1))
        .collect::<Option<Vec<_>>>()?;
    let c = &a.children;
    Some(match a.kind {
        K::Plus => sum(d.into_iter().map(|d| (1, d)).collect()),
        K::Minus if c.len() == 1 => sum(vec![(-1, d[0].clone())]),
        K::Minus if c.len() == 2 => sum(vec![(1, d[0].clone()), (-1, d[1].clone())]),
        K::Times => {
            let mut terms = Vec::new();
            for (i, di) in d.into_iter().enumerate() {
                if !is(&di, 0.0) {
                    let mut factors = c.clone();
                    factors[i] = di;
                    terms.push((1, multiply(factors)));
                }
            }
            sum(terms)
        }
        K::Divide if c.len() == 2 => {
            quotient(c[0].clone(), c[1].clone(), d[0].clone(), d[1].clone())
        }
        K::Power | K::FunctionPower if c.len() == 2 => {
            power_rate(c[0].clone(), c[1].clone(), d[0].clone(), d[1].clone())
        }
        K::FunctionRoot if !c.is_empty() && c.len() <= 2 => {
            let degree = if c.len() == 2 {
                c[0].clone()
            } else {
                number(2.0)
            };
            let rate = if c.len() == 2 {
                d[0].clone()
            } else {
                number(0.0)
            };
            let inverse = value(&degree)
                .map_or_else(|| div(number(1.0), degree.clone()), |n| number(1.0 / n));
            power_rate(
                c.last()?.clone(),
                inverse,
                d.last()?.clone(),
                quotient(number(1.0), degree, number(0.0), rate),
            )
        }
        K::FunctionLog if !c.is_empty() && c.len() <= 2 => {
            if c.len() == 2 && !is(&d[0], 0.0) {
                return None;
            }
            let base = if c.len() == 2 {
                c[0].clone()
            } else {
                number(10.0)
            };
            div(
                d.last()?.clone(),
                multiply(vec![c.last()?.clone(), call(K::FunctionLn, base)]),
            )
        }
        K::FunctionAbs if c.len() == 1 => {
            if is(&d[0], 0.0) {
                d[0].clone()
            } else {
                node(
                    K::FunctionPiecewise,
                    vec![
                        d[0].clone(),
                        node(K::RelationalGt, vec![c[0].clone(), number(0.0)]),
                        negate(d[0].clone()),
                    ],
                )
            }
        }
        _ if c.len() == 1 => {
            if is(&d[0], 0.0) {
                return Some(number(0.0));
            }
            let u = || c[0].clone();
            let outer = match a.kind {
                K::FunctionExp => call(K::FunctionExp, u()),
                K::FunctionLn => div(number(1.0), u()),
                K::FunctionSin => call(K::FunctionCos, u()),
                K::FunctionCos => negate(call(K::FunctionSin, u())),
                K::FunctionTan => square(call(K::FunctionSec, u())),
                K::FunctionSec => {
                    multiply(vec![call(K::FunctionSec, u()), call(K::FunctionTan, u())])
                }
                K::FunctionCsc => negate(multiply(vec![
                    call(K::FunctionCsc, u()),
                    call(K::FunctionCot, u()),
                ])),
                K::FunctionCot => negate(square(call(K::FunctionCsc, u()))),
                K::FunctionArcsin => div(
                    number(1.0),
                    root(sum(vec![(1, number(1.0)), (-1, square(u()))])),
                ),
                K::FunctionArccos => negate(div(
                    number(1.0),
                    root(sum(vec![(1, number(1.0)), (-1, square(u()))])),
                )),
                K::FunctionArctan => {
                    div(number(1.0), sum(vec![(1, number(1.0)), (1, square(u()))]))
                }
                K::FunctionSinh => call(K::FunctionCosh, u()),
                K::FunctionCosh => call(K::FunctionSinh, u()),
                K::FunctionTanh => div(number(1.0), square(call(K::FunctionCosh, u()))),
                K::FunctionArcsinh => div(
                    number(1.0),
                    root(sum(vec![(1, square(u())), (1, number(1.0))])),
                ),
                K::FunctionArccosh => div(
                    number(1.0),
                    root(sum(vec![(1, square(u())), (-1, number(1.0))])),
                ),
                K::FunctionArctanh => {
                    div(number(1.0), sum(vec![(1, number(1.0)), (-1, square(u()))]))
                }
                _ => return None,
            };
            if outer.kind == K::Divide && value(&outer.children[0]).is_some() {
                div(
                    multiply(vec![outer.children[0].clone(), d[0].clone()]),
                    outer.children[1].clone(),
                )
            } else {
                multiply(vec![d[0].clone(), outer])
            }
        }
        _ => return None,
    })
}

//! Small, ownership-safe operations used to derive equations.
use sbmlsim_document::{Ast, AstKind as K, AstNumber};
pub(super) fn node(kind: K, children: Vec<Ast>) -> Ast {
    Ast {
        kind,
        name: None,
        number: None,
        units: None,
        children,
    }
}
pub(super) fn name(s: &str) -> Ast {
    Ast {
        kind: K::Name,
        name: Some(s.to_owned()),
        number: None,
        units: None,
        children: vec![],
    }
}
pub(super) fn number(n: f64) -> Ast {
    Ast {
        kind: K::Real,
        name: None,
        number: Some(AstNumber::Real(n)),
        units: None,
        children: vec![],
    }
}
pub(super) fn value(ast: &Ast) -> Option<f64> {
    match &ast.number {
        Some(AstNumber::Integer(n)) => Some(*n as f64),
        Some(AstNumber::Real(n)) => Some(*n),
        Some(AstNumber::RealE { mantissa, exponent }) => {
            Some(sbmlsim_document::scientific_value(*mantissa, *exponent))
        }
        Some(AstNumber::Rational {
            numerator,
            denominator,
        }) => Some(*numerator as f64 / *denominator as f64),
        None => None,
    }
}
pub(super) fn is(ast: &Ast, n: f64) -> bool {
    value(ast) == Some(n)
}
pub(super) fn product(mut factors: Vec<Ast>) -> Ast {
    if factors.len() == 1 {
        factors.remove(0)
    } else {
        node(K::Times, factors)
    }
}
pub(super) fn negate(mut ast: Ast) -> Ast {
    if let Some(n) = value(&ast) {
        return number(-n);
    }
    if ast.kind == K::Minus && ast.children.len() == 1 {
        return ast.children.remove(0);
    }
    node(K::Minus, vec![ast])
}
pub(super) fn sum(terms: Vec<(i8, Ast)>) -> Ast {
    let mut terms = terms.into_iter().filter(|(_, a)| !is(a, 0.0));
    let Some((sign, first)) = terms.next() else {
        return number(0.0);
    };
    let mut total = if sign < 0 { negate(first) } else { first };
    let mut open = false;
    for (sign, term) in terms {
        if sign > 0 && open {
            total.children.push(term);
        } else {
            total = node(if sign > 0 { K::Plus } else { K::Minus }, vec![total, term]);
            open = sign > 0;
        }
    }
    total
}
pub(super) fn names(ast: &Ast) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut stack = vec![ast];
    while let Some(a) = stack.pop() {
        if a.kind == K::Name
            && let Some(n) = &a.name
        {
            names.insert(n.clone());
        }
        stack.extend(&a.children);
    }
    names
}
pub(super) fn rename(ast: &mut Ast, names: &std::collections::HashMap<String, String>) {
    let mut stack = vec![ast];
    while let Some(a) = stack.pop() {
        if a.kind == K::Name
            && let Some(name) = &a.name
            && let Some(new) = names.get(name)
        {
            a.name = Some(new.clone());
        }
        stack.extend(&mut a.children);
    }
}

pub(super) fn depth(ast: &Ast) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(ast, 1)];
    while let Some((a, d)) = stack.pop() {
        deepest = deepest.max(d);
        stack.extend(a.children.iter().map(|c| (c, d + 1)));
    }
    deepest
}
pub(super) fn check_depth(ast: &Ast, element: &str) -> Result<(), String> {
    let levels = depth(ast);
    if levels <= 100 {
        return Ok(());
    }
    let subject = if element.is_empty() {
        "The math".to_owned()
    } else {
        format!("The math of '{element}'")
    };
    Err(format!(
        "{subject} is nested {levels} levels deep, more than the 100 levels sbmlode reads. Math nested this deeply is written by a tool rather than by hand: write the inner expressions as assignment rules of their own and refer to them by their id."
    ))
}
pub(super) fn continuous_trigger(ast: &Ast) -> bool {
    let mut stack = vec![ast];
    while let Some(a) = stack.pop() {
        match a.kind {
            K::RelationalGt | K::RelationalGeq | K::RelationalLt | K::RelationalLeq => {
                if a.children.len() < 2 {
                    return false;
                }
            }
            K::ConstantTrue | K::ConstantFalse | K::Integer | K::Real | K::RealE | K::Rational => {}
            K::LogicalAnd | K::LogicalOr | K::LogicalXor => stack.extend(&a.children),
            K::LogicalNot if a.children.len() == 1 => stack.extend(&a.children),
            K::LogicalImplies if a.children.len() == 2 => stack.extend(&a.children),
            _ => return false,
        }
    }
    true
}

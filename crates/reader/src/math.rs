//! Content MathML into the existing owned, stack-safe AST.

use crate::{MATHML, ReadError, elements, text};
use roxmltree::Node;
use sbmlsim_document::{Ast, AstKind as K, AstNumber, MAX_AST_DEPTH};

fn ast(
    kind: K,
    name: Option<String>,
    number: Option<AstNumber>,
    units: Option<String>,
    children: Vec<Ast>,
) -> Ast {
    Ast {
        kind,
        name,
        number,
        units,
        children,
    }
}
fn number(value: i64) -> Ast {
    ast(
        K::Integer,
        None,
        Some(AstNumber::Integer(value)),
        None,
        vec![],
    )
}

pub(crate) fn operator(name: &str) -> K {
    match name {
        "plus" => K::Plus,
        "minus" => K::Minus,
        "times" => K::Times,
        "divide" => K::Divide,
        "power" => K::FunctionPower,
        "abs" => K::FunctionAbs,
        "arccos" => K::FunctionArccos,
        "arccosh" => K::FunctionArccosh,
        "arccot" => K::FunctionArccot,
        "arccoth" => K::FunctionArccoth,
        "arccsc" => K::FunctionArccsc,
        "arccsch" => K::FunctionArccsch,
        "arcsec" => K::FunctionArcsec,
        "arcsech" => K::FunctionArcsech,
        "arcsin" => K::FunctionArcsin,
        "arcsinh" => K::FunctionArcsinh,
        "arctan" => K::FunctionArctan,
        "arctanh" => K::FunctionArctanh,
        "ceiling" => K::FunctionCeiling,
        "cos" => K::FunctionCos,
        "cosh" => K::FunctionCosh,
        "cot" => K::FunctionCot,
        "coth" => K::FunctionCoth,
        "csc" => K::FunctionCsc,
        "csch" => K::FunctionCsch,
        "exp" => K::FunctionExp,
        "factorial" => K::FunctionFactorial,
        "floor" => K::FunctionFloor,
        "ln" => K::FunctionLn,
        "log" => K::FunctionLog,
        "max" => K::FunctionMax,
        "min" => K::FunctionMin,
        "quotient" => K::FunctionQuotient,
        "rem" => K::FunctionRem,
        "root" => K::FunctionRoot,
        "sec" => K::FunctionSec,
        "sech" => K::FunctionSech,
        "sin" => K::FunctionSin,
        "sinh" => K::FunctionSinh,
        "tan" => K::FunctionTan,
        "tanh" => K::FunctionTanh,
        "eq" => K::RelationalEq,
        "geq" => K::RelationalGeq,
        "gt" => K::RelationalGt,
        "leq" => K::RelationalLeq,
        "lt" => K::RelationalLt,
        "neq" => K::RelationalNeq,
        "and" => K::LogicalAnd,
        "not" => K::LogicalNot,
        "or" => K::LogicalOr,
        "xor" => K::LogicalXor,
        "implies" => K::LogicalImplies,
        "true" => K::ConstantTrue,
        "false" => K::ConstantFalse,
        "pi" => K::ConstantPi,
        "exponentiale" => K::ConstantE,
        _ => K::Unknown,
    }
}

fn csymbol(node: Node) -> K {
    let url = node.attribute("definitionURL").unwrap_or("");
    match url.trim_end_matches('/').rsplit('/').next().unwrap_or("") {
        "time" => K::NameTime,
        "avogadro" => K::NameAvogadro,
        "delay" => K::FunctionDelay,
        "rateOf" => K::FunctionRateOf,
        "bernoulli" => K::DistribBernoulli,
        "binomial" => K::DistribBinomial,
        "cauchy" => K::DistribCauchy,
        "chisquare" => K::DistribChisquare,
        "exponential" => K::DistribExponential,
        "gamma" => K::DistribGamma,
        "laplace" => K::DistribLaplace,
        "lognormal" => K::DistribLognormal,
        "normal" => K::DistribNormal,
        "poisson" => K::DistribPoisson,
        "rayleigh" => K::DistribRayleigh,
        "uniform" => K::DistribUniform,
        _ => K::CsymbolFunction,
    }
}

fn cn(node: Node) -> Result<Ast, ReadError> {
    let kind = node.attribute("type").unwrap_or("real");
    let parts = || {
        let mut values = [String::new(), String::new()];
        let mut index = 0;
        for child in node.children() {
            if child.is_element() && child.tag_name().name() == "sep" {
                index = 1;
            } else if child.is_text() {
                values[index].push_str(child.text().unwrap_or(""));
            }
        }
        values
    };
    let invalid = || ReadError(format!("Invalid MathML number: {}", text(node)));
    let (kind, value) = match kind {
        "integer" => (
            K::Integer,
            AstNumber::Integer(text(node).parse().map_err(|_| invalid())?),
        ),
        "rational" => {
            let p = parts();
            (
                K::Rational,
                AstNumber::Rational {
                    numerator: p[0].trim().parse().map_err(|_| invalid())?,
                    denominator: p[1].trim().parse().map_err(|_| invalid())?,
                },
            )
        }
        "e-notation" => {
            let p = parts();
            (
                K::RealE,
                AstNumber::RealE {
                    mantissa: p[0].trim().parse().map_err(|_| invalid())?,
                    exponent: p[1].trim().parse().map_err(|_| invalid())?,
                },
            )
        }
        _ => (
            K::Real,
            AstNumber::Real(match text(node).as_str() {
                "INF" => f64::INFINITY,
                "-INF" => f64::NEG_INFINITY,
                "NaN" => f64::NAN,
                value => value.parse().map_err(|_| invalid())?,
            }),
        ),
    };
    let units = node
        .attributes()
        .find(|a| a.name() == "units")
        .map(|a| a.value().to_owned());
    Ok(ast(kind, None, Some(value), units, vec![]))
}

fn operands<'a, 'input>(node: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    match node.tag_name().name() {
        "cn" | "ci" | "csymbol" => vec![],
        "apply" => elements(node).skip(1).collect(),
        "semantics" | "math" => elements(node).take(1).collect(),
        _ => elements(node).collect(),
    }
}

pub(crate) fn from_node(root: Node) -> Result<Option<Ast>, ReadError> {
    enum Work<'a, 'input> {
        Visit(Node<'a, 'input>, usize),
        Finish(Node<'a, 'input>, usize),
    }
    let mut stack = vec![Work::Visit(root, 0)];
    let mut values: Vec<Vec<Ast>> = Vec::new();
    while let Some(work) = stack.pop() {
        match work {
            Work::Visit(node, depth) => {
                if depth > MAX_AST_DEPTH {
                    return Err(ReadError(format!(
                        "the math is nested deeper than {MAX_AST_DEPTH} levels"
                    )));
                }
                if node.tag_name().namespace() != Some(MATHML) {
                    return Err(ReadError(
                        "A mathematical element has no MathML namespace.".into(),
                    ));
                }
                let children = operands(node);
                stack.push(Work::Finish(node, children.len()));
                stack.extend(
                    children
                        .into_iter()
                        .rev()
                        .map(|child| Work::Visit(child, depth + 1)),
                );
            }
            Work::Finish(node, count) => {
                let start = values.len().saturating_sub(count);
                let mut children: Vec<Ast> = values.drain(start..).flatten().collect();
                let name = node.tag_name().name();
                if matches!(
                    name,
                    "math" | "semantics" | "bvar" | "degree" | "logbase" | "piece" | "otherwise"
                ) {
                    values.push(children);
                    continue;
                }
                let result = match name {
                    "ci" => ast(K::Name, Some(text(node)), None, None, vec![]),
                    "csymbol" => ast(csymbol(node), Some(text(node)), None, None, vec![]),
                    "cn" => cn(node)?,
                    "lambda" => ast(K::Lambda, None, None, None, children),
                    "piecewise" => ast(K::FunctionPiecewise, None, None, None, children),
                    "apply" => {
                        let op = elements(node).next();
                        let (kind, symbol) = if let Some(op) = op {
                            match op.tag_name().name() {
                                "ci" => (K::Function, Some(text(op))),
                                "csymbol" => (csymbol(op), Some(text(op))),
                                other => (operator(other), None),
                            }
                        } else {
                            (K::Unknown, None)
                        };
                        if matches!(kind, K::FunctionRoot | K::FunctionLog) && children.len() == 1 {
                            children
                                .insert(0, number(if kind == K::FunctionRoot { 2 } else { 10 }));
                        }
                        if matches!(kind, K::Plus | K::Times) && children.len() > 2 {
                            let mut terms = children.into_iter();
                            let mut left = terms
                                .next()
                                .unwrap_or_else(|| ast(K::Unknown, None, None, None, vec![]));
                            for right in terms {
                                left = ast(kind, None, None, None, vec![left, right]);
                            }
                            left
                        } else {
                            ast(kind, symbol, None, None, children)
                        }
                    }
                    other => ast(operator(other), None, None, None, children),
                };
                values.push(vec![result]);
            }
        }
    }
    Ok(values.pop().and_then(|mut nodes| nodes.pop()))
}

/// Read Content MathML without linking any native library.
///
/// Empty math has no AST. The tree retains numeric types, units, user function
/// names and lambda binders so every downstream consumer sees the same data.
pub fn read_math(source: &str) -> Result<Option<Ast>, ReadError> {
    let document = roxmltree::Document::parse(source).map_err(|e| ReadError(e.to_string()))?;
    let root = document.root_element();
    if root.tag_name().name() != "math" {
        return Err(ReadError("The document is not Content MathML.".into()));
    }
    from_node(root)
}

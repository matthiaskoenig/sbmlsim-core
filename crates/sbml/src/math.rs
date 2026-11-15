//! The MathML of SBML Level 3 Version 2.
//!
//! Representation notes: n-ary operators keep all arguments in the written order (no folding or
//! reassociation); `minus` has one (negation) or two arguments; `log` has `[x]` (base 10) or
//! `[base, x]`; `root` has `[x]` (degree 2) or `[degree, x]`; `delay` and `rateOf` in operator
//! position are [`Op::Delay`] and [`Op::RateOf`], while the csymbols alone are
//! [`Node::CsymbolDelay`] and [`Node::CsymbolRateOf`]; a `ci` in operator position is
//! [`Op::Function`]; `semantics` is its first child without the annotations.

use crate::ReadError;
use roxmltree::Node as X;

/// The MathML namespace.
pub const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";

const TIME_URL: &str = "http://www.sbml.org/sbml/symbols/time";
const AVOGADRO_URL: &str = "http://www.sbml.org/sbml/symbols/avogadro";
const DELAY_URL: &str = "http://www.sbml.org/sbml/symbols/delay";
const RATEOF_URL: &str = "http://www.sbml.org/sbml/symbols/rateOf";

/// An operator or function.
#[derive(Debug, Clone, PartialEq)]
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
    /// The csymbol `delay` applied to its arguments.
    Delay,
    /// The csymbol `rateOf` applied to its argument.
    RateOf,
    /// A user function by its id.
    Function(String),
}

/// A node of a math expression.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Node {
    Number(f64),
    Symbol(String),
    CsymbolTime,
    CsymbolAvogadro,
    CsymbolDelay,
    CsymbolRateOf,
    Apply {
        op: Op,
        args: Vec<Node>,
    },
    Piecewise {
        pieces: Vec<(Node, Node)>,
        otherwise: Option<Box<Node>>,
    },
    Lambda {
        params: Vec<String>,
        body: Box<Node>,
    },
    True,
    False,
    Pi,
    ExponentialE,
    Infinity,
    NotANumber,
}

fn err(message: String) -> ReadError {
    ReadError::Xml(message)
}

fn operator(name: &str) -> Option<Op> {
    Some(match name {
        "plus" => Op::Plus,
        "minus" => Op::Minus,
        "times" => Op::Times,
        "divide" => Op::Divide,
        "power" => Op::Power,
        "root" => Op::Root,
        "abs" => Op::Abs,
        "exp" => Op::Exp,
        "ln" => Op::Ln,
        "log" => Op::Log,
        "floor" => Op::Floor,
        "ceiling" => Op::Ceiling,
        "factorial" => Op::Factorial,
        "sin" => Op::Sin,
        "cos" => Op::Cos,
        "tan" => Op::Tan,
        "sec" => Op::Sec,
        "csc" => Op::Csc,
        "cot" => Op::Cot,
        "sinh" => Op::Sinh,
        "cosh" => Op::Cosh,
        "tanh" => Op::Tanh,
        "sech" => Op::Sech,
        "csch" => Op::Csch,
        "coth" => Op::Coth,
        "arcsin" => Op::Arcsin,
        "arccos" => Op::Arccos,
        "arctan" => Op::Arctan,
        "arcsec" => Op::Arcsec,
        "arccsc" => Op::Arccsc,
        "arccot" => Op::Arccot,
        "arcsinh" => Op::Arcsinh,
        "arccosh" => Op::Arccosh,
        "arctanh" => Op::Arctanh,
        "arcsech" => Op::Arcsech,
        "arccsch" => Op::Arccsch,
        "arccoth" => Op::Arccoth,
        "eq" => Op::Eq,
        "neq" => Op::Neq,
        "gt" => Op::Gt,
        "lt" => Op::Lt,
        "geq" => Op::Geq,
        "leq" => Op::Leq,
        "and" => Op::And,
        "or" => Op::Or,
        "xor" => Op::Xor,
        "not" => Op::Not,
        "implies" => Op::Implies,
        "max" => Op::Max,
        "min" => Op::Min,
        "rem" => Op::Rem,
        "quotient" => Op::Quotient,
        _ => return None,
    })
}

fn elements<'a, 'i>(n: X<'a, 'i>) -> impl Iterator<Item = X<'a, 'i>> {
    n.children().filter(|c| c.is_element())
}

fn text(n: X) -> String {
    n.children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn check_ns(n: X) -> Result<(), ReadError> {
    if n.tag_name().namespace() == Some(MATHML_NS) {
        Ok(())
    } else {
        Err(err(format!(
            "unknown MathML element <{}> (not in the MathML namespace)",
            n.tag_name().name()
        )))
    }
}

/// Reads a `<math>` element: its first element child is the expression.
pub(crate) fn read_math(math: X) -> Result<Node, ReadError> {
    let first = elements(math)
        .next()
        .ok_or_else(|| err("empty <math> element".into()))?;
    expr(first)
}

fn number(n: X) -> Result<Node, ReadError> {
    let kind = n.attribute("type").unwrap_or("real");
    let bad = |t: &str| err(format!("invalid number {t:?} in <cn type=\"{kind}\">"));
    let parts = || -> Result<(String, String), ReadError> {
        let mut head = String::new();
        let mut tail = String::new();
        let mut after = false;
        for c in n.children() {
            if c.is_element() && c.tag_name().name() == "sep" {
                after = true;
            } else if let Some(t) = c.text().filter(|_| c.is_text()) {
                if after {
                    tail.push_str(t)
                } else {
                    head.push_str(t)
                }
            }
        }
        if !after {
            return Err(err(format!("<cn type=\"{kind}\"> without <sep/>")));
        }
        Ok((head.trim().to_string(), tail.trim().to_string()))
    };
    let parse = |t: &str| t.parse::<f64>().map_err(|_| bad(t));
    match kind {
        "real" | "integer" => Ok(Node::Number(parse(&text(n))?)),
        "e-notation" => {
            let (m, e) = parts()?;
            Ok(Node::Number(parse(&format!("{m}e{e}"))?))
        }
        "rational" => {
            let (a, b) = parts()?;
            Ok(Node::Number(parse(&a)? / parse(&b)?))
        }
        other => Err(err(format!("unknown <cn> type {other:?}"))),
    }
}

fn expr(n: X) -> Result<Node, ReadError> {
    check_ns(n)?;
    let name = n.tag_name().name();
    match name {
        "cn" => number(n),
        "ci" => Ok(Node::Symbol(text(n))),
        "csymbol" => match n.attribute("definitionURL") {
            Some(TIME_URL) => Ok(Node::CsymbolTime),
            Some(AVOGADRO_URL) => Ok(Node::CsymbolAvogadro),
            Some(DELAY_URL) => Ok(Node::CsymbolDelay),
            Some(RATEOF_URL) => Ok(Node::CsymbolRateOf),
            other => Err(err(format!("unknown csymbol definitionURL {other:?}"))),
        },
        "true" => Ok(Node::True),
        "false" => Ok(Node::False),
        "pi" => Ok(Node::Pi),
        "exponentiale" => Ok(Node::ExponentialE),
        "infinity" => Ok(Node::Infinity),
        "notanumber" => Ok(Node::NotANumber),
        "semantics" => {
            let first = elements(n)
                .find(|c| !matches!(c.tag_name().name(), "annotation" | "annotation-xml"))
                .ok_or_else(|| err("<semantics> without an expression".into()))?;
            expr(first)
        }
        "apply" => apply(n),
        "piecewise" => piecewise(n),
        "lambda" => lambda(n),
        other => Err(err(format!("unknown MathML element <{other}>"))),
    }
}

fn inner(n: X) -> Result<Node, ReadError> {
    let c = elements(n)
        .next()
        .ok_or_else(|| err(format!("empty <{}>", n.tag_name().name())))?;
    expr(c)
}

fn apply(n: X) -> Result<Node, ReadError> {
    let mut kids = elements(n);
    let head = kids.next().ok_or_else(|| err("empty <apply>".into()))?;
    check_ns(head)?;
    let hname = head.tag_name().name();
    let op = match hname {
        "ci" => Op::Function(text(head)),
        "csymbol" => match head.attribute("definitionURL") {
            Some(DELAY_URL) => Op::Delay,
            Some(RATEOF_URL) => Op::RateOf,
            other => return Err(err(format!("csymbol {other:?} cannot be applied"))),
        },
        _ => operator(hname).ok_or_else(|| err(format!("unknown MathML function <{hname}>")))?,
    };
    let mut args = Vec::new();
    for k in kids {
        check_ns(k)?;
        match k.tag_name().name() {
            // the qualifiers come first: `[base, x]` and `[degree, x]`
            "logbase" if op == Op::Log => args.push(inner(k)?),
            "degree" if op == Op::Root => args.push(inner(k)?),
            _ => args.push(expr(k)?),
        }
    }
    let n_args = args.len();
    let arity = match op {
        Op::Divide | Op::Rem | Op::Quotient | Op::Implies => Some((2, 2)),
        Op::Power => Some((2, 2)),
        Op::Minus => Some((1, 2)),
        Op::Log | Op::Root => Some((1, 2)),
        Op::Abs
        | Op::Exp
        | Op::Ln
        | Op::Floor
        | Op::Ceiling
        | Op::Factorial
        | Op::Not
        | Op::RateOf => Some((1, 1)),
        Op::Delay => Some((2, 2)),
        Op::Plus
        | Op::Times
        | Op::Eq
        | Op::Neq
        | Op::Gt
        | Op::Lt
        | Op::Geq
        | Op::Leq
        | Op::And
        | Op::Or
        | Op::Xor
        | Op::Max
        | Op::Min
        | Op::Function(_) => None,
        _ => Some((1, 1)), // trigonometric and hyperbolic functions and inverses
    };
    if let Some((lo, hi)) = arity
        && !(lo..=hi).contains(&n_args)
    {
        return Err(err(format!(
            "<{hname}> takes {lo}..{hi} arguments, found {n_args}"
        )));
    }
    Ok(Node::Apply { op, args })
}

fn piecewise(n: X) -> Result<Node, ReadError> {
    let mut pieces = Vec::new();
    let mut otherwise = None;
    for c in elements(n) {
        check_ns(c)?;
        match c.tag_name().name() {
            "piece" => {
                let mut e = elements(c);
                let (Some(v), Some(cond), None) = (e.next(), e.next(), e.next()) else {
                    return Err(err("<piece> needs a value and a condition".into()));
                };
                pieces.push((expr(v)?, expr(cond)?));
            }
            "otherwise" => otherwise = Some(Box::new(inner(c)?)),
            other => return Err(err(format!("unknown element <{other}> in <piecewise>"))),
        }
    }
    Ok(Node::Piecewise { pieces, otherwise })
}

fn lambda(n: X) -> Result<Node, ReadError> {
    let mut params = Vec::new();
    let mut body = None;
    for c in elements(n) {
        check_ns(c)?;
        if c.tag_name().name() == "bvar" {
            match inner(c)? {
                Node::Symbol(s) => params.push(s),
                _ => return Err(err("<bvar> must hold a <ci>".into())),
            }
        } else if body.is_none() {
            body = Some(expr(c)?);
        } else {
            return Err(err("<lambda> has more than one body".into()));
        }
    }
    let body = body.ok_or_else(|| err("<lambda> without a body".into()))?;
    Ok(Node::Lambda {
        params,
        body: Box::new(body),
    })
}

//! The math of PEtab v2: its expressions read as the math of SBML, and their values.
//!
//! A formula of a problem, the `observableFormula` and `noiseFormula` of an observable and the
//! `targetValue` of a condition, is read by [`parse`] into the same [`Node`] tree a `<math>` of
//! SBML gives, so it is compiled into a model or evaluated by the operations of the compiler
//! and of the engine, and never by a second implementation of them. The grammar is the one of
//! the specification (`Math expressions syntax`), from the weakest binding to the tightest:
//!
//! | precedence | operators | associativity |
//! | --- | --- | --- |
//! | 7 | `&&`, `\|\|` | left to right, one level: `a \|\| b && c` is `(a \|\| b) && c` |
//! | 6 | `<`, `<=`, `>`, `>=`, `==`, `!=` | left to right |
//! | 5 | `+`, `-` | left to right |
//! | 4 | `*`, `/` | left to right |
//! | 3 | unary `+`, `-`, `!` | right to left |
//! | 2 | `^` | right to left: `2^2^3` is `2^(2^3)`, and `-2^4` is `-(2^4)` |
//! | 1 | a call `f(a, b)`, `( )` | |
//!
//! The exponent of `^` is a unary expression, so `9 * 3 ^ -2` is `9 * 3^(-2)`. The functions
//! are the ones of the specification and nothing else: `log(x)` is the natural logarithm and
//! `log(a, b)` the logarithm of `a` to the base `b` (in SBML the base comes first), `sign(x)`
//! is `piecewise(1, x > 0, -1, x < 0, 0)` and `piecewise(v1, c1, ..., otherwise)` takes its
//! pieces as value and condition. `true`, `false` and `inf` are literals and `time` is the time
//! of the model, all of them case-insensitive as the reserved words of PEtab are; `nan` is
//! reserved and refused. A boolean is a number where a number is expected and the other way
//! round, which is what the compiled math does with them.

use std::collections::HashMap;

use sbmlsim_ir::{Expr, Symbol};
use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{Document, Model, Parameter, Rule};
use thiserror::Error;

/// A formula which is no expression of PEtab, or which cannot be evaluated.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("'{formula}': {message}")]
pub struct MathError {
    /// The formula.
    pub formula: String,
    /// What is wrong with it.
    pub message: String,
}

/// A token of an expression.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    /// An operator of one or two characters, or a bracket or a comma.
    Mark(&'static str),
}

/// The operators of the grammar, the ones of two characters first.
const MARKS: [&str; 17] = [
    "&&", "||", "<=", ">=", "==", "!=", "<", ">", "+", "-", "*", "/", "^", "!", "(", ")", ",",
];

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    'next: while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let literal: String = chars[start..i].iter().collect();
            let value = literal
                .parse::<f64>()
                .map_err(|_| format!("'{literal}' is no number"))?;
            tokens.push(Token::Number(value));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }
        for mark in MARKS {
            let width = mark.len();
            if i + width <= chars.len() && chars[i..i + width].iter().copied().eq(mark.chars()) {
                tokens.push(Token::Mark(mark));
                i += width;
                continue 'next;
            }
        }
        return Err(format!("'{c}' is no part of an expression"));
    }
    Ok(tokens)
}

/// The recursive descent over the tokens of one expression.
struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn mark(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Token::Mark(mark)) => Some(mark),
            _ => None,
        }
    }

    fn expect(&mut self, mark: &'static str) -> Result<(), String> {
        if self.mark() == Some(mark) {
            self.at += 1;
            Ok(())
        } else {
            Err(match self.peek() {
                Some(token) => format!("'{mark}' expected, found {}", describe(token)),
                None => format!("'{mark}' expected at the end"),
            })
        }
    }

    /// `&&` and `||`, one level of precedence.
    fn logical(&mut self) -> Result<Node, String> {
        let mut left = self.comparison()?;
        while let Some(mark @ ("&&" | "||")) = self.mark() {
            self.at += 1;
            let right = self.comparison()?;
            let op = if mark == "&&" { Op::And } else { Op::Or };
            left = apply(op, vec![left, right]);
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Node, String> {
        let mut left = self.additive()?;
        while let Some(mark @ ("<" | "<=" | ">" | ">=" | "==" | "!=")) = self.mark() {
            self.at += 1;
            let right = self.additive()?;
            let op = match mark {
                "<" => Op::Lt,
                "<=" => Op::Leq,
                ">" => Op::Gt,
                ">=" => Op::Geq,
                "==" => Op::Eq,
                _ => Op::Neq,
            };
            left = apply(op, vec![left, right]);
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Node, String> {
        let mut left = self.multiplicative()?;
        while let Some(mark @ ("+" | "-")) = self.mark() {
            self.at += 1;
            let right = self.multiplicative()?;
            let op = if mark == "+" { Op::Plus } else { Op::Minus };
            left = apply(op, vec![left, right]);
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Node, String> {
        let mut left = self.unary()?;
        while let Some(mark @ ("*" | "/")) = self.mark() {
            self.at += 1;
            let right = self.unary()?;
            let op = if mark == "*" { Op::Times } else { Op::Divide };
            left = apply(op, vec![left, right]);
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Node, String> {
        match self.mark() {
            Some("+") => {
                self.at += 1;
                self.unary()
            }
            Some("-") => {
                self.at += 1;
                Ok(apply(Op::Minus, vec![self.unary()?]))
            }
            Some("!") => {
                self.at += 1;
                Ok(apply(Op::Not, vec![self.unary()?]))
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Node, String> {
        let base = self.primary()?;
        if self.mark() == Some("^") {
            self.at += 1;
            let exponent = self.unary()?;
            return Ok(apply(Op::Power, vec![base, exponent]));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Node, String> {
        let token = self
            .peek()
            .cloned()
            .ok_or_else(|| "the expression ends early".to_string())?;
        self.at += 1;
        match token {
            Token::Number(value) => Ok(Node::number(value)),
            Token::Mark("(") => {
                let inner = self.logical()?;
                self.expect(")")?;
                Ok(inner)
            }
            Token::Mark(mark) => Err(format!("'{mark}' where a value is expected")),
            Token::Name(name) => {
                if self.mark() == Some("(") {
                    self.at += 1;
                    let mut args = Vec::new();
                    if self.mark() != Some(")") {
                        args.push(self.logical()?);
                        while self.mark() == Some(",") {
                            self.at += 1;
                            args.push(self.logical()?);
                        }
                    }
                    self.expect(")")?;
                    return call(&name, args);
                }
                literal(&name)
            }
        }
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(value) => format!("the number {value}"),
        Token::Name(name) => format!("'{name}'"),
        Token::Mark(mark) => format!("'{mark}'"),
    }
}

fn apply(op: Op, args: Vec<Node>) -> Node {
    Node::Apply { op, args }
}

/// A name which is no call: a literal, `time` or a symbol.
fn literal(name: &str) -> Result<Node, String> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "true" => Node::True,
        "false" => Node::False,
        "inf" => Node::Infinity,
        "time" => Node::CsymbolTime,
        "nan" => return Err("'nan' is reserved and no number of PEtab".to_string()),
        _ => Node::Symbol(name.to_string()),
    })
}

/// A call of a function of PEtab.
fn call(name: &str, mut args: Vec<Node>) -> Result<Node, String> {
    let arity = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!(
                "{name} takes {n} argument{}, not {}",
                if n == 1 { "" } else { "s" },
                args.len()
            ))
        }
    };
    let op = match name {
        "pow" => {
            arity(2)?;
            return Ok(apply(Op::Power, args));
        }
        "exp" => Op::Exp,
        "sqrt" => Op::Root,
        "ln" => Op::Ln,
        "log" => {
            return match args.len() {
                1 => Ok(apply(Op::Ln, args)),
                2 => {
                    // PEtab writes the value first and the base second, SBML the other way
                    args.swap(0, 1);
                    Ok(apply(Op::Log, args))
                }
                n => Err(format!("log takes 1 or 2 arguments, not {n}")),
            };
        }
        "log2" | "log10" => {
            arity(1)?;
            let base = if name == "log2" { 2.0 } else { 10.0 };
            args.insert(0, Node::number(base));
            return Ok(apply(Op::Log, args));
        }
        "sin" => Op::Sin,
        "cos" => Op::Cos,
        "tan" => Op::Tan,
        "cot" => Op::Cot,
        "sec" => Op::Sec,
        "csc" => Op::Csc,
        "arcsin" => Op::Arcsin,
        "arccos" => Op::Arccos,
        "arctan" => Op::Arctan,
        "arccot" => Op::Arccot,
        "arcsec" => Op::Arcsec,
        "arccsc" => Op::Arccsc,
        "sinh" => Op::Sinh,
        "cosh" => Op::Cosh,
        "tanh" => Op::Tanh,
        "coth" => Op::Coth,
        "sech" => Op::Sech,
        "csch" => Op::Csch,
        "arcsinh" => Op::Arcsinh,
        "arccosh" => Op::Arccosh,
        "arctanh" => Op::Arctanh,
        "arccoth" => Op::Arccoth,
        "arcsech" => Op::Arcsech,
        "arccsch" => Op::Arccsch,
        "abs" => Op::Abs,
        "sign" => {
            arity(1)?;
            let x = args.pop().expect("one argument");
            let zero = || Node::number(0.0);
            return Ok(Node::Piecewise {
                pieces: vec![
                    (Node::number(1.0), apply(Op::Gt, vec![x.clone(), zero()])),
                    (Node::number(-1.0), apply(Op::Lt, vec![x, zero()])),
                ],
                otherwise: Some(Box::new(zero())),
            });
        }
        "min" | "max" => {
            arity(2)?;
            return Ok(apply(if name == "min" { Op::Min } else { Op::Max }, args));
        }
        "piecewise" => {
            if args.is_empty() {
                return Err("piecewise takes at least one argument".to_string());
            }
            let otherwise = (args.len() % 2 == 1).then(|| Box::new(args.pop().expect("odd")));
            let mut pieces = Vec::with_capacity(args.len() / 2);
            let mut rest = args.into_iter();
            while let (Some(value), Some(condition)) = (rest.next(), rest.next()) {
                pieces.push((value, condition));
            }
            return Ok(Node::Piecewise { pieces, otherwise });
        }
        _ => return Err(format!("'{name}' is no function of PEtab")),
    };
    arity(1)?;
    Ok(apply(op, args))
}

/// Reads an expression of PEtab as the math of SBML.
///
/// # Errors
///
/// What the text holds instead of an expression, with the text.
pub fn parse(formula: &str) -> Result<Node, MathError> {
    let fault = |message: String| MathError {
        formula: formula.to_string(),
        message,
    };
    let tokens = tokenize(formula).map_err(fault)?;
    if tokens.is_empty() {
        return Err(fault("the expression is empty".to_string()));
    }
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    let node = parser.logical().map_err(fault)?;
    match parser.peek() {
        None => Ok(node),
        Some(token) => Err(fault(format!("{} is left over", describe(token)))),
    }
}

/// The symbols an expression reads, each once, in the order of their first appearance;
/// `time` is no symbol.
#[must_use]
pub fn symbols(node: &Node) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for symbol in node.symbols() {
        if !out.iter().any(|known| known == symbol) {
            out.push(symbol.to_string());
        }
    }
    out
}

/// The expression with every symbol `replace` gives a node for replaced by that node.
#[must_use]
pub fn substitute(node: &Node, replace: &dyn Fn(&str) -> Option<Node>) -> Node {
    match node {
        Node::Symbol(name) => replace(name).unwrap_or_else(|| node.clone()),
        Node::Apply { op, args } => Node::Apply {
            op: op.clone(),
            args: args.iter().map(|arg| substitute(arg, replace)).collect(),
        },
        Node::Piecewise { pieces, otherwise } => Node::Piecewise {
            pieces: pieces
                .iter()
                .map(|(value, condition)| {
                    (substitute(value, replace), substitute(condition, replace))
                })
                .collect(),
            otherwise: otherwise
                .as_ref()
                .map(|node| Box::new(substitute(node, replace))),
        },
        _ => node.clone(),
    }
}

/// Whether an expression reads the time of the model.
#[must_use]
pub fn reads_time(node: &Node) -> bool {
    match node {
        Node::CsymbolTime => true,
        Node::Apply { args, .. } => args.iter().any(reads_time),
        Node::Piecewise { pieces, otherwise } => {
            pieces
                .iter()
                .any(|(value, condition)| reads_time(value) || reads_time(condition))
                || otherwise.as_deref().is_some_and(reads_time)
        }
        _ => false,
    }
}

/// The number an expression is without reading anything, a literal or the sign of one; `None`
/// for every other expression, which [`Formula`] evaluates.
#[must_use]
pub fn number(node: &Node) -> Option<f64> {
    match node {
        Node::Number { value, .. } => Some(*value),
        Node::Infinity => Some(f64::INFINITY),
        Node::Apply {
            op: Op::Minus,
            args,
        } if args.len() == 1 => number(&args[0]).map(|value| 0.0 - value),
        _ => None,
    }
}

/// The id of the parameter a [`Formula`] assigns its expression in the model it is evaluated
/// in, which no symbol of PEtab can be: an id of PEtab does not start with an underscore and
/// two of them.
const VALUE: &str = "__petab_value";

/// An expression of PEtab compiled into the operations of the model representation, which
/// evaluates it at values of its symbols.
///
/// The expression is the assignment rule of a model whose parameters are its symbols, built by
/// [`sbmlsim_ir::build`], so it is evaluated by [`sbmlsim_ir::eval`]: the operations of the
/// compiled models and of the engine, operation for operation.
#[derive(Debug, Clone)]
pub struct Formula {
    /// The symbols, in the order of [`Formula::evaluate`]'s values.
    symbols: Vec<String>,
    /// The expression over the parameters of the symbols.
    expr: Expr,
    /// The position of a parameter of the model among the symbols.
    positions: Vec<usize>,
}

impl Formula {
    /// Compiles an expression.
    ///
    /// # Errors
    ///
    /// The refusal of the builder, e.g. a call of a function which has none.
    pub fn new(node: &Node) -> Result<Formula, String> {
        let symbols = symbols(node);
        let mut parameters: Vec<Parameter> = symbols
            .iter()
            .map(|id| Parameter {
                id: id.clone(),
                value: Some(0.0),
                units: None,
                constant: true,
            })
            .collect();
        parameters.push(Parameter {
            id: VALUE.to_string(),
            value: None,
            units: None,
            constant: false,
        });
        let document = Document {
            level: 3,
            version: 2,
            model: Model {
                id: Some("petab".to_string()),
                parameters,
                rules: vec![Rule::Assignment {
                    id: None,
                    variable: VALUE.to_string(),
                    math: Some(node.clone()),
                }],
                ..Model::default()
            },
        };
        let model = sbmlsim_ir::build(&document).map_err(|err| err.to_string())?;
        let target = model
            .symbol(VALUE)
            .ok_or_else(|| "the value is no entity of its model".to_string())?;
        let expr = model
            .assignment_rules
            .iter()
            .find(|rule| rule.target == target)
            .map(|rule| rule.expr.clone())
            .ok_or_else(|| "the value has no rule".to_string())?;
        let by_id: HashMap<&str, usize> = symbols
            .iter()
            .enumerate()
            .map(|(k, id)| (id.as_str(), k))
            .collect();
        let positions = model
            .parameters
            .iter()
            .map(|parameter| {
                by_id
                    .get(parameter.id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            })
            .collect();
        Ok(Formula {
            symbols,
            expr,
            positions,
        })
    }

    /// The symbols, in the order of the values of [`Formula::evaluate`].
    #[must_use]
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// The value at the values of the symbols, in the order of [`Formula::symbols`], and at a
    /// time of the model.
    #[must_use]
    pub fn evaluate(&self, values: &[f64], time: f64) -> f64 {
        let read = |symbol: Symbol| match symbol {
            Symbol::Param(i) => self
                .positions
                .get(i as usize)
                .and_then(|&k| values.get(k))
                .copied()
                .unwrap_or(f64::NAN),
            Symbol::Time => time,
            _ => f64::NAN,
        };
        sbmlsim_ir::eval::eval(&self.expr, &read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> f64 {
        let node = parse(text).unwrap();
        Formula::new(&node).unwrap().evaluate(&[], 0.0)
    }

    #[test]
    fn the_precedence_is_the_one_of_petab() {
        assert_eq!(value("-2^4"), -16.0);
        assert_eq!(value("2^2^3"), 256.0);
        assert_eq!(value("9 * 3 ^ -2"), 1.0);
        assert_eq!(value("1 + 2 * 3"), 7.0);
        assert_eq!(value("piecewise(0, true || false && false, 1)"), 1.0);
        assert_eq!(value("--1.0"), 1.0);
    }

    #[test]
    fn the_logarithms_are_the_ones_of_petab() {
        assert_eq!(value("log(exp(1))"), 1.0);
        assert_eq!(value("log(100, 10)"), 2.0);
        assert_eq!(value("log10(10)"), 1.0);
        assert_eq!(value("log2(2)"), 1.0);
        assert_eq!(value("log(0)"), f64::NEG_INFINITY);
    }

    #[test]
    fn booleans_are_numbers_and_numbers_booleans() {
        assert_eq!(value("3 + true"), 4.0);
        assert_eq!(value("!1"), 0.0);
        assert_eq!(value("-!0"), -1.0);
        assert_eq!(value("piecewise(1, -0.1, 0)"), 1.0);
        assert_eq!(value("sign(-2)"), -1.0);
    }

    #[test]
    fn symbols_and_time_are_read() {
        let formula = Formula::new(&parse("a * b + time").unwrap()).unwrap();
        assert_eq!(formula.symbols(), ["a", "b"]);
        assert_eq!(formula.evaluate(&[2.0, 3.0], 4.0), 10.0);
    }

    #[test]
    fn what_is_no_expression_is_refused() {
        for text in ["", "1 +", "a b", "nan", "foo(1)", "log(1, 2, 3)", "(1"] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}

//! The Level 1 formula grammar, using heap stacks for operators and grouping.
use crate::ReadError;
use sbmlsim_document::{Ast, AstKind as K, AstNumber};

fn node(kind: K, children: Vec<Ast>) -> Ast {
    Ast {
        kind,
        name: None,
        number: None,
        units: None,
        children,
    }
}
fn integer(value: i64) -> Ast {
    Ast {
        kind: K::Integer,
        name: None,
        number: Some(AstNumber::Integer(value)),
        units: None,
        children: vec![],
    }
}
#[derive(Clone)]
enum Token {
    Identifier(String),
    Number(String),
    Operator(char),
    Open,
    Close,
    Comma,
}
fn tokens(source: &str) -> Result<Vec<Token>, ReadError> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match c {
            b'(' => out.push(Token::Open),
            b')' => out.push(Token::Close),
            b',' => out.push(Token::Comma),
            b'+' | b'-' | b'*' | b'/' | b'^' => out.push(Token::Operator(c as char)),
            b'0'..=b'9' | b'.' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                    i += 1;
                    if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                out.push(Token::Number(source[start..i].into()));
                continue;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(Token::Identifier(source[start..i].into()));
                continue;
            }
            _ => {
                return Err(ReadError(format!(
                    "Invalid character in a Level 1 formula at byte {i}."
                )));
            }
        }
        i += 1;
    }
    Ok(out)
}
enum Op {
    Operator(char, bool),
    Group(Option<String>, usize),
}
fn precedence(operator: char, unary: bool) -> u8 {
    if unary {
        4
    } else {
        match operator {
            '+' | '-' => 1,
            '*' | '/' => 2,
            '^' => 3,
            _ => 0,
        }
    }
}
fn reduce(op: Op, values: &mut Vec<Ast>) -> Result<(), ReadError> {
    let Op::Operator(operator, unary) = op else {
        return Err(ReadError("Unbalanced formula grouping.".into()));
    };
    let value = values
        .pop()
        .ok_or_else(|| ReadError("A formula operator has no operand.".into()))?;
    if unary {
        if operator == '+' {
            values.push(value);
        } else {
            let value = match value.number {
                Some(AstNumber::Integer(n)) => integer(
                    n.checked_neg()
                        .ok_or_else(|| ReadError("Integer formula overflow.".into()))?,
                ),
                Some(AstNumber::Real(n)) => Ast {
                    kind: K::Real,
                    name: None,
                    number: Some(AstNumber::Real(-n)),
                    units: None,
                    children: vec![],
                },
                Some(AstNumber::RealE { mantissa, exponent }) => Ast {
                    kind: K::RealE,
                    name: None,
                    number: Some(AstNumber::RealE {
                        mantissa: -mantissa,
                        exponent,
                    }),
                    units: None,
                    children: vec![],
                },
                _ => node(K::Minus, vec![value]),
            };
            values.push(value);
        }
        return Ok(());
    }
    let left = values
        .pop()
        .ok_or_else(|| ReadError("A formula operator has no left operand.".into()))?;
    values.push(node(
        match operator {
            '+' => K::Plus,
            '-' => K::Minus,
            '*' => K::Times,
            '/' => K::Divide,
            '^' => K::Power,
            _ => K::Unknown,
        },
        vec![left, value],
    ));
    Ok(())
}
fn function(name: String, mut args: Vec<Ast>) -> Ast {
    let kind = match name.as_str() {
        "acos" => K::FunctionArccos,
        "acosh" => K::FunctionArccosh,
        "asin" => K::FunctionArcsin,
        "asinh" => K::FunctionArcsinh,
        "atan" => K::FunctionArctan,
        "atanh" => K::FunctionArctanh,
        "ceil" => K::FunctionCeiling,
        "log" | "ln" => K::FunctionLn,
        "pow" => K::FunctionPower,
        "sqrt" => {
            args.insert(0, integer(2));
            K::FunctionRoot
        }
        "sqr" => {
            args.push(integer(2));
            K::FunctionPower
        }
        "log10" => {
            args.insert(0, integer(10));
            K::FunctionLog
        }
        _ => crate::math::operator(&name),
    };
    if kind == K::Unknown {
        Ast {
            kind: K::Function,
            name: Some(name),
            number: None,
            units: None,
            children: args,
        }
    } else {
        node(kind, args)
    }
}
/// Read the legacy formula attribute of a Level 1 SBML object.
///
/// # Errors
/// Returns an error for malformed tokens, grouping or operator arity.
pub fn read_formula(source: &str) -> Result<Option<Ast>, ReadError> {
    let tokens = tokens(source)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let mut ops = Vec::new();
    let mut values = Vec::new();
    let mut operand = false;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Number(text) => {
                let (kind, number) = if let Some((mantissa, exponent)) = text.split_once(['e', 'E'])
                {
                    (
                        K::RealE,
                        AstNumber::RealE {
                            mantissa: mantissa
                                .parse()
                                .map_err(|_| ReadError("Invalid formula mantissa.".into()))?,
                            exponent: exponent
                                .parse()
                                .map_err(|_| ReadError("Invalid formula exponent.".into()))?,
                        },
                    )
                } else if text.contains('.') {
                    (
                        K::Real,
                        AstNumber::Real(
                            text.parse()
                                .map_err(|_| ReadError("Invalid formula number.".into()))?,
                        ),
                    )
                } else {
                    (
                        K::Integer,
                        AstNumber::Integer(
                            text.parse()
                                .map_err(|_| ReadError("Invalid formula integer.".into()))?,
                        ),
                    )
                };
                values.push(Ast {
                    kind,
                    name: None,
                    number: Some(number),
                    units: None,
                    children: vec![],
                });
                operand = true;
            }
            Token::Identifier(name) => {
                if matches!(tokens.get(i + 1), Some(Token::Open)) {
                    ops.push(Op::Group(Some(name.clone()), 0));
                    operand = false;
                    i += 1;
                } else {
                    let kind = match name.as_str() {
                        "pi" => K::ConstantPi,
                        "exponentiale" => K::ConstantE,
                        "true" => K::ConstantTrue,
                        "false" => K::ConstantFalse,
                        _ => K::Name,
                    };
                    values.push(Ast {
                        kind,
                        name: (kind == K::Name).then(|| name.clone()),
                        number: None,
                        units: None,
                        children: vec![],
                    });
                    operand = true;
                }
            }
            Token::Open => {
                ops.push(Op::Group(None, 0));
                operand = false;
            }
            Token::Close => {
                while matches!(ops.last(), Some(Op::Operator(..))) {
                    reduce(ops.pop().unwrap(), &mut values)?;
                }
                let Some(Op::Group(function_name, commas)) = ops.pop() else {
                    return Err(ReadError("Unbalanced formula parentheses.".into()));
                };
                if let Some(name) = function_name {
                    let count = if operand { commas + 1 } else { 0 };
                    if values.len() < count {
                        return Err(ReadError("A function argument is missing.".into()));
                    }
                    let args = values.split_off(values.len() - count);
                    values.push(function(name, args));
                } else if !operand {
                    return Err(ReadError("An empty formula group has no value.".into()));
                }
                operand = true;
            }
            Token::Comma => {
                if !operand {
                    return Err(ReadError("An empty function argument has no value.".into()));
                }
                while matches!(ops.last(), Some(Op::Operator(..))) {
                    reduce(ops.pop().unwrap(), &mut values)?;
                }
                if let Some(Op::Group(Some(_), commas)) = ops.last_mut() {
                    *commas += 1;
                } else {
                    return Err(ReadError("A comma is outside a function call.".into()));
                }
                operand = false;
            }
            Token::Operator(operator) => {
                let unary = !operand;
                if unary && !matches!(operator, '+' | '-') {
                    return Err(ReadError(
                        "A binary formula operator has no left operand.".into(),
                    ));
                }
                while let Some(Op::Operator(previous, was_unary)) = ops.last() {
                    if precedence(*previous, *was_unary) < precedence(*operator, unary) || unary {
                        break;
                    }
                    reduce(ops.pop().unwrap(), &mut values)?;
                }
                ops.push(Op::Operator(*operator, unary));
                operand = false;
            }
        }
        i += 1;
    }
    if !operand {
        return Err(ReadError("A formula ends without an operand.".into()));
    }
    while let Some(op) = ops.pop() {
        reduce(op, &mut values)?;
    }
    if values.len() != 1 {
        return Err(ReadError(
            "A formula has more than one unconnected value.".into(),
        ));
    }
    Ok(values.pop())
}

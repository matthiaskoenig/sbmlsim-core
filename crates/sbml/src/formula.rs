//! The infix formulas of SBML Level 1, which carries its mathematics in attributes.
//!
//! [`parse`] reads a formula as the same [`Node`] tree a `<math>` of a later level gives, so a
//! consumer never sees the difference. The grammar is the one of libsbml's parser of a Level 1
//! formula, whose readings this module follows:
//!
//! - `+` and `-` bind weakest and associate to the left, then `*` and `/`, then `^`, which
//!   associates to the left as well (`2^3^2` is `(2^3)^2`), and a sign binds tightest of all
//!   (`-x^2` is `(-x)^2`, and `-2^2` is `(-2)^2`, the sign of the number itself)
//! - `log(x)` is the natural logarithm, `log(b, x)` the logarithm of `x` to the base `b` and
//!   `log10(x)` the one to the base 10; `sqrt(x)` is the root of the degree 2 and `root(d, x)`
//!   of the degree `d`
//! - `and`, `or`, `xor`, `not` and the comparisons are function calls, Level 1 has no infix
//!   operator for them, and `delay(x, d)` is the csymbol of a delay
//! - `true`, `false`, `pi`, `exponentiale`, `INF` and `NaN` are the constants they name, every
//!   other name is a symbol (`time` and `avogadro` included, which Level 1 does not reserve),
//!   and a call of a name which is no function of the grammar is the call of that function

use crate::math::{Node, Op};

/// Reads an infix formula of Level 1 as the tree of a `<math>` element.
///
/// # Errors
///
/// The message of what the formula holds instead of a formula, without a position: the formula
/// is short and the message quotes it.
pub fn parse(formula: &str) -> Result<Node, String> {
    let tokens = tokenize(formula)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    let node = parser.expression()?;
    match parser.peek() {
        None => Ok(node),
        Some(token) => Err(format!("{token:?} is left over")),
    }
}

/// A token of a formula.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// A number.
    Number(f64),
    /// A name, of a symbol or of a function.
    Name(String),
    /// An operator or a bracket, one character.
    Mark(char),
}

/// The characters which are an operator or a bracket.
const MARKS: [char; 7] = ['+', '-', '*', '/', '^', '(', ')'];

fn tokenize(formula: &str) -> Result<Vec<Token>, String> {
    let characters: Vec<char> = formula.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < characters.len() {
        let c = characters[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ',' {
            tokens.push(Token::Mark(','));
            i += 1;
        } else if MARKS.contains(&c) {
            tokens.push(Token::Mark(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < characters.len() && (characters[i].is_ascii_digit() || characters[i] == '.') {
                i += 1;
            }
            // the exponent of the scientific notation, with its sign
            if i < characters.len() && (characters[i] == 'e' || characters[i] == 'E') {
                let exponent = i;
                i += 1;
                if i < characters.len() && (characters[i] == '+' || characters[i] == '-') {
                    i += 1;
                }
                if i < characters.len() && characters[i].is_ascii_digit() {
                    while i < characters.len() && characters[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    // `2e` and `2e+` are a number and a name, as libsbml reads them
                    i = exponent;
                }
            }
            let text: String = characters[start..i].iter().collect();
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("{text:?} of {formula:?} is no number"))?;
            tokens.push(Token::Number(value));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < characters.len() && (characters[i].is_alphanumeric() || characters[i] == '_')
            {
                i += 1;
            }
            tokens.push(Token::Name(characters[start..i].iter().collect()));
        } else {
            return Err(format!("{c:?} of {formula:?} is no part of a formula"));
        }
    }
    Ok(tokens)
}

/// The tokens of a formula and the position the parser is at.
struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    /// Takes the mark `c` when it is next.
    fn take(&mut self, c: char) -> bool {
        if self.peek() == Some(&Token::Mark(c)) {
            self.at += 1;
            return true;
        }
        false
    }

    /// `term (('+' | '-') term)*`, to the left.
    fn expression(&mut self) -> Result<Node, String> {
        let mut left = self.term()?;
        loop {
            let op = if self.take('+') {
                Op::Plus
            } else if self.take('-') {
                Op::Minus
            } else {
                return Ok(left);
            };
            let right = self.term()?;
            left = Node::Apply {
                op,
                args: vec![left, right],
            };
        }
    }

    /// `power (('*' | '/') power)*`, to the left.
    fn term(&mut self) -> Result<Node, String> {
        let mut left = self.power()?;
        loop {
            let op = if self.take('*') {
                Op::Times
            } else if self.take('/') {
                Op::Divide
            } else {
                return Ok(left);
            };
            let right = self.power()?;
            left = Node::Apply {
                op,
                args: vec![left, right],
            };
        }
    }

    /// `signed ('^' signed)*`, to the left, as libsbml reads it.
    fn power(&mut self) -> Result<Node, String> {
        let mut left = self.signed()?;
        while self.take('^') {
            let right = self.signed()?;
            left = Node::Apply {
                op: Op::Power,
                args: vec![left, right],
            };
        }
        Ok(left)
    }

    /// `('-' | '+')* primary`; a sign of a number is the sign of the number itself.
    fn signed(&mut self) -> Result<Node, String> {
        if self.take('+') {
            return self.signed();
        }
        if self.take('-') {
            return Ok(match self.signed()? {
                Node::Number(value) => Node::Number(-value),
                node => Node::Apply {
                    op: Op::Minus,
                    args: vec![node],
                },
            });
        }
        self.primary()
    }

    /// A number, a constant, a symbol, a call or a bracketed expression.
    fn primary(&mut self) -> Result<Node, String> {
        match self.peek().cloned() {
            Some(Token::Number(value)) => {
                self.at += 1;
                Ok(Node::Number(value))
            }
            Some(Token::Name(name)) => {
                self.at += 1;
                if !self.take('(') {
                    return Ok(constant(&name).unwrap_or(Node::Symbol(name)));
                }
                let mut args = Vec::new();
                if !self.take(')') {
                    loop {
                        args.push(self.expression()?);
                        if self.take(')') {
                            break;
                        }
                        if !self.take(',') {
                            return Err(format!("the call of {name:?} is not closed"));
                        }
                    }
                }
                call(&name, args)
            }
            Some(Token::Mark('(')) => {
                self.at += 1;
                let node = self.expression()?;
                if self.take(')') {
                    Ok(node)
                } else {
                    Err("a bracket is not closed".to_string())
                }
            }
            Some(token) => Err(format!("{token:?} is no value")),
            None => Err("the formula ends where a value belongs".to_string()),
        }
    }
}

/// The constant a name stands for, `None` for a name which is a symbol.
fn constant(name: &str) -> Option<Node> {
    match name {
        "true" => Some(Node::True),
        "false" => Some(Node::False),
        "pi" => Some(Node::Pi),
        "exponentiale" => Some(Node::ExponentialE),
        _ if name.eq_ignore_ascii_case("inf") || name.eq_ignore_ascii_case("infinity") => {
            Some(Node::Infinity)
        }
        _ if name.eq_ignore_ascii_case("nan") => Some(Node::NotANumber),
        _ => None,
    }
}

/// A call of `name` with `args`: the operator of the grammar, else the user function.
fn call(name: &str, args: Vec<Node>) -> Result<Node, String> {
    // the logarithms and the roots carry their base and their degree as an argument
    match name {
        "log" if args.len() == 1 => {
            return Ok(Node::Apply { op: Op::Ln, args });
        }
        "log10" if args.len() == 1 => {
            return Ok(Node::Apply {
                op: Op::Log,
                args: vec![Node::Number(10.0), args[0].clone()],
            });
        }
        "sqrt" if args.len() == 1 => {
            return Ok(Node::Apply {
                op: Op::Root,
                args: vec![Node::Number(2.0), args[0].clone()],
            });
        }
        _ => {}
    }
    let op = match name {
        "abs" => Op::Abs,
        "acos" | "arccos" => Op::Arccos,
        "arccosh" => Op::Arccosh,
        "arccot" => Op::Arccot,
        "arccoth" => Op::Arccoth,
        "arccsc" => Op::Arccsc,
        "arccsch" => Op::Arccsch,
        "arcsec" => Op::Arcsec,
        "arcsech" => Op::Arcsech,
        "asin" | "arcsin" => Op::Arcsin,
        "arcsinh" => Op::Arcsinh,
        "atan" | "arctan" => Op::Arctan,
        "arctanh" => Op::Arctanh,
        "ceil" | "ceiling" => Op::Ceiling,
        "cos" => Op::Cos,
        "cosh" => Op::Cosh,
        "cot" => Op::Cot,
        "coth" => Op::Coth,
        "csc" => Op::Csc,
        "csch" => Op::Csch,
        "delay" => Op::Delay,
        "exp" => Op::Exp,
        "factorial" => Op::Factorial,
        "floor" => Op::Floor,
        "ln" => Op::Ln,
        "log" => Op::Log,
        "pow" | "power" => Op::Power,
        "root" => Op::Root,
        "sec" => Op::Sec,
        "sech" => Op::Sech,
        "sin" => Op::Sin,
        "sinh" => Op::Sinh,
        "tan" => Op::Tan,
        "tanh" => Op::Tanh,
        "and" => Op::And,
        "not" => Op::Not,
        "or" => Op::Or,
        "xor" => Op::Xor,
        "eq" => Op::Eq,
        "geq" => Op::Geq,
        "gt" => Op::Gt,
        "leq" => Op::Leq,
        "lt" => Op::Lt,
        "neq" => Op::Neq,
        // a name which is no function of the grammar is the call of a function of the model
        _ => Op::Function(name.to_string()),
    };
    Ok(Node::Apply { op, args })
}

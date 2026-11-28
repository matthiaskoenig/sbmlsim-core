//! The grammar of a unit expression, the one pint reads.
//!
//! An expression is numbers and names of units combined by `*`, `/`, `**` (or `^`), unary
//! signs, parentheses and juxtaposition (`kg m` is `kg * m`, `2m` is `2 * m`), with the
//! priorities of pint: `**` binds tightest and to the right (`m**-1**2` is `m**-1`), a sign
//! next, then `*`, `/` and juxtaposition alike from the left (`1/kg m` is `m/kg`). ` per ` is
//! `/`. A sum is no unit and is refused.
//!
//! Parsing gives a tree, which [`Expr::evaluate`] folds into a [`Value`]: the number of the
//! expression, its magnitude, computed in the order of the tree as python computes it, and its
//! units, the names with their exponents in the order pint's container keeps them.

use crate::UnitError;

/// A token of an expression.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Times,
    Divide,
    Power,
    Plus,
    Minus,
    Open,
    Close,
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Name(String),
    Negate(Box<Expr>),
    Product(Box<Expr>, Box<Expr>),
    Quotient(Box<Expr>, Box<Expr>),
    Power(Box<Expr>, Box<Expr>),
}

/// The units of a value: every unit with its exponent, in the order of their first appearance,
/// a unit whose exponent becomes zero removed, as `UnitsContainer` of pint keeps them.
pub(crate) type Units<K> = Vec<(K, i32)>;

/// A value of an expression: its magnitude and its units.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Value<K> {
    pub magnitude: f64,
    pub units: Units<K>,
}

impl<K: PartialEq + Clone> Value<K> {
    /// A number without units.
    pub fn number(magnitude: f64) -> Value<K> {
        Value {
            magnitude,
            units: Vec::new(),
        }
    }

    fn combine(mut self, other: Value<K>, sign: i32, magnitude: f64) -> Value<K> {
        for (key, exponent) in other.units {
            add(&mut self.units, key, sign * exponent);
        }
        self.magnitude = magnitude;
        self
    }

    /// The product of two values.
    pub fn times(self, other: Value<K>) -> Value<K> {
        let magnitude = self.magnitude * other.magnitude;
        self.combine(other, 1, magnitude)
    }

    /// The quotient of two values.
    pub fn divide(self, other: Value<K>) -> Value<K> {
        let magnitude = self.magnitude / other.magnitude;
        self.combine(other, -1, magnitude)
    }
}

/// Adds an exponent of a unit to units, removing a unit whose exponent becomes zero.
pub(crate) fn add<K: PartialEq>(units: &mut Units<K>, key: K, exponent: i32) {
    if let Some(at) = units.iter().position(|(k, _)| *k == key) {
        units[at].1 += exponent;
        if units[at].1 == 0 {
            units.remove(at);
        }
    } else if exponent != 0 {
        units.push((key, exponent));
    }
}

/// Whether a character continues a name: a letter, a digit, `_` and the signs which names of
/// pint contain (`°C`, `Ω`, `µ`, `ℓ`).
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '°'
}

/// The tokens of an expression.
fn tokens(expression: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = expression.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // an exponent only where digits follow, so `2em` is not one
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
            let text: String = chars[start..i].iter().collect();
            let number = text
                .parse::<f64>()
                .map_err(|_| format!("'{text}' is no number"))?;
            out.push(Token::Number(number));
        } else if name_char(c) {
            let start = i;
            while i < chars.len() && name_char(chars[i]) {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            if name == "per" {
                out.push(Token::Divide);
            } else {
                out.push(Token::Name(name));
            }
        } else {
            match c {
                '*' if chars.get(i + 1) == Some(&'*') => {
                    out.push(Token::Power);
                    i += 1;
                }
                '*' | '·' | '⋅' => out.push(Token::Times),
                '^' => out.push(Token::Power),
                '/' => out.push(Token::Divide),
                '+' => out.push(Token::Plus),
                '-' => out.push(Token::Minus),
                '(' => out.push(Token::Open),
                ')' => out.push(Token::Close),
                other => return Err(format!("the character '{other}' is no part of a unit")),
            }
            i += 1;
        }
    }
    Ok(out)
}

/// A recursive descent parser over the tokens.
struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    /// A product or quotient of factors, from the left.
    fn product(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            match self.peek() {
                Some(Token::Times) => {
                    self.at += 1;
                    left = Expr::Product(Box::new(left), Box::new(self.unary()?));
                }
                Some(Token::Divide) => {
                    self.at += 1;
                    left = Expr::Quotient(Box::new(left), Box::new(self.unary()?));
                }
                // juxtaposition is a product of the priority of `*`
                Some(Token::Number(_) | Token::Name(_) | Token::Open) => {
                    left = Expr::Product(Box::new(left), Box::new(self.unary()?));
                }
                Some(Token::Plus | Token::Minus) => {
                    return Err("a sum or difference is no unit".to_owned());
                }
                _ => return Ok(left),
            }
        }
    }

    /// A signed power.
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.at += 1;
                Ok(Expr::Negate(Box::new(self.unary()?)))
            }
            Some(Token::Plus) => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// An atom with its power, to the right: the exponent is itself signed and a power.
    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if self.peek() == Some(&Token::Power) {
            self.at += 1;
            let exponent = self.unary()?;
            return Ok(Expr::Power(Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Name(name)) => Ok(Expr::Name(name)),
            Some(Token::Open) => {
                let inner = self.product()?;
                match self.next() {
                    Some(Token::Close) => Ok(inner),
                    _ => Err("a parenthesis is not closed".to_owned()),
                }
            }
            Some(Token::Close) => Err("a parenthesis closes which was not opened".to_owned()),
            Some(_) => Err("an operator lacks its operand".to_owned()),
            None => Err("the expression ends where an operand is expected".to_owned()),
        }
    }
}

/// Parses an expression; an empty one is `None`, which is dimensionless.
pub(crate) fn parse(expression: &str) -> Result<Option<Expr>, UnitError> {
    let syntax = |reason: String| UnitError::Syntax {
        expression: expression.to_owned(),
        reason,
    };
    let tokens = tokens(expression).map_err(syntax)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let mut parser = Parser { tokens, at: 0 };
    let expr = parser.product().map_err(syntax)?;
    if parser.at < parser.tokens.len() {
        return Err(syntax(
            "a parenthesis closes which was not opened".to_owned(),
        ));
    }
    Ok(Some(expr))
}

/// The error of a power of units out of the range of an exponent.
pub(crate) fn too_large(expression: &str) -> UnitError {
    UnitError::Syntax {
        expression: expression.to_owned(),
        reason: "a power of its units is too large".to_owned(),
    }
}

impl Expr {
    /// Evaluates the expression, the units of a name given by `name`.
    ///
    /// `expression` is the text of the whole expression, for the errors.
    pub fn evaluate<K: PartialEq + Clone>(
        &self,
        expression: &str,
        name: &mut impl FnMut(&str) -> Result<Value<K>, UnitError>,
    ) -> Result<Value<K>, UnitError> {
        Ok(match self {
            Expr::Number(n) => Value::number(*n),
            Expr::Name(n) => name(n)?,
            Expr::Negate(inner) => {
                let mut value = inner.evaluate(expression, name)?;
                value.magnitude = -value.magnitude;
                value
            }
            Expr::Product(a, b) => a
                .evaluate(expression, name)?
                .times(b.evaluate(expression, name)?),
            Expr::Quotient(a, b) => a
                .evaluate(expression, name)?
                .divide(b.evaluate(expression, name)?),
            Expr::Power(base, exponent) => {
                let base = base.evaluate(expression, name)?;
                let exponent = exponent.evaluate(expression, name)?;
                if !exponent.units.is_empty() {
                    return Err(UnitError::Syntax {
                        expression: expression.to_owned(),
                        reason: "an exponent is a number without units".to_owned(),
                    });
                }
                let p = exponent.magnitude;
                let magnitude = libm::pow(base.magnitude, p);
                if base.units.is_empty() {
                    return Ok(Value::number(magnitude));
                }
                if p.fract() != 0.0 || p.abs() > f64::from(i16::MAX) {
                    return Err(UnitError::FractionalPower {
                        expression: expression.to_owned(),
                        exponent: p,
                    });
                }
                #[allow(clippy::cast_possible_truncation)]
                let p = p as i32;
                let mut units = Vec::new();
                for (key, e) in base.units {
                    let power = e.checked_mul(p).ok_or_else(|| too_large(expression))?;
                    add(&mut units, key, power);
                }
                Value { magnitude, units }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Expr, parse};

    fn tree(expression: &str) -> Expr {
        parse(expression).unwrap().unwrap()
    }

    fn name(n: &str) -> Box<Expr> {
        Box::new(Expr::Name(n.to_owned()))
    }

    #[test]
    fn priorities_of_pint() {
        // `1/kg m` is `(1/kg) * m`
        assert_eq!(
            tree("1/kg m"),
            Expr::Product(
                Box::new(Expr::Quotient(Box::new(Expr::Number(1.0)), name("kg"))),
                name("m")
            )
        );
        // `**` to the right, its exponent signed
        assert_eq!(
            tree("m**-1**2"),
            Expr::Power(
                name("m"),
                Box::new(Expr::Negate(Box::new(Expr::Power(
                    Box::new(Expr::Number(1.0)),
                    Box::new(Expr::Number(2.0))
                ))))
            )
        );
        assert_eq!(tree("m^2"), tree("m**2"));
        assert_eq!(tree("mg per min"), tree("mg/min"));
        assert_eq!(tree("2m"), tree("2*m"));
    }

    #[test]
    fn empty_and_invalid() {
        assert_eq!(parse("  ").unwrap(), None);
        for bad in ["m +", "(m", "m)", "m + s", "m $", "*m"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}

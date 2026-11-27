//! Precedence-aware scalar expressions for the standalone executable targets.
use crate::ast;
use sbmlsim_document::{Ast, AstKind as K, AstNumber};
use std::collections::{HashMap, HashSet};

#[path = "code_tables.rs"]
mod tables;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Python,
    Diffrax,
    Julia,
    R,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Diffrax => "jax",
            Self::Julia => "julia",
            Self::R => "r",
        }
    }
    pub fn index(self) -> usize {
        usize::from(matches!(self, Self::Julia | Self::R))
    }
    fn prefix(self) -> &'static str {
        if self == Self::Diffrax { "jnp" } else { "np" }
    }
    pub fn number(self, value: f64) -> String {
        if value.is_nan() {
            return match self {
                Self::Python => "np.nan",
                Self::Diffrax => "jnp.nan",
                _ => "NaN",
            }
            .into();
        }
        if value.is_infinite() {
            return format!(
                "{}{}",
                if value < 0.0 { "-" } else { "" },
                match self {
                    Self::Python => "np.inf",
                    Self::Diffrax => "jnp.inf",
                    _ => "Inf",
                }
            );
        }
        let text = sbmlsim_document::formatting::python_float(value);
        if self == Self::R {
            text.strip_suffix(".0").unwrap_or(&text).to_owned()
        } else if self == Self::Julia && text.contains('e') {
            let (m, e) = text.split_once('e').unwrap();
            format!(
                "{}{}e{}",
                m,
                if m.contains('.') { "" } else { ".0" },
                e.parse::<i32>().unwrap()
            )
        } else {
            text
        }
    }
    fn integer(self, n: i64) -> String {
        if matches!(self, Self::Julia | Self::Diffrax) {
            self.number(n as f64)
        } else {
            n.to_string()
        }
    }
    pub fn reserved(self) -> &'static [&'static str] {
        tables::reserved(self)
    }
}

#[derive(Clone)]
struct Printed {
    code: String,
    precedence: u8,
}
fn p(code: impl Into<String>, precedence: u8) -> Printed {
    Printed {
        code: code.into(),
        precedence,
    }
}
fn wrap(a: &Printed, precedence: u8) -> String {
    if a.precedence < precedence {
        format!("({})", a.code)
    } else {
        a.code.clone()
    }
}
fn call(name: &str, args: &[Printed]) -> Printed {
    p(
        format!(
            "{name}({})",
            args.iter()
                .map(|a| a.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        11,
    )
}
fn infix(args: &[Printed], op: &str, precedence: u8) -> Printed {
    p(
        args.iter()
            .enumerate()
            .map(|(i, a)| wrap(a, precedence + u8::from(i > 0)))
            .collect::<Vec<_>>()
            .join(op),
        precedence,
    )
}
fn literal(code: String) -> Printed {
    let prec = if code.starts_with('-') { 9 } else { 11 };
    p(code, prec)
}
fn fold(name: &str, args: &[Printed]) -> Printed {
    let mut tail = args.last().cloned().unwrap();
    for a in args[..args.len() - 1].iter().rev() {
        tail = call(name, &[a.clone(), tail]);
    }
    tail
}

/// Resolve language keywords, runtime names and collisions without changing safe identifiers.
pub fn names<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    target: Target,
) -> Result<HashMap<String, String>, String> {
    let mut ordered = Vec::new();
    let mut taken: HashSet<String> = target.reserved().iter().map(|s| (*s).into()).collect();
    for id in ids {
        crate::latex::symbol(id)?;
        if !ordered.contains(&id) {
            ordered.push(id);
        }
        taken.insert(id.into());
    }
    let valid = |s: &str| {
        !(target == Target::R && s.starts_with('_')
            || target == Target::Julia && s.bytes().all(|c| c == b'_'))
    };
    let mut out = HashMap::new();
    for id in ordered {
        let mut code = id.to_owned();
        if target.reserved().contains(&id) || !valid(id) {
            if !valid(id) {
                code.insert(0, 'x');
            }
            while taken.contains(&code) {
                code.push('_');
            }
            taken.insert(code.clone());
        }
        out.insert(id.to_owned(), code);
    }
    Ok(out)
}

struct Printer<'a> {
    target: Target,
    symbols: &'a HashMap<String, String>,
}
impl Printer<'_> {
    fn boolean_number(&self, a: Printed) -> Printed {
        match self.target {
            Target::Python => call("float", &[a]),
            Target::Julia => call("Float64", &[a]),
            Target::R => call("as.numeric", &[a]),
            Target::Diffrax => call("jnp.where", &[a, p("1.0", 11), p("0.0", 11)]),
        }
    }
    fn negate_bool(&self, a: Printed) -> Printed {
        match self.target {
            Target::Python => p(format!("not {}", wrap(&a, 4)), 4),
            Target::Diffrax => call("jnp.logical_not", &[a]),
            _ => p(format!("!{}", wrap(&a, 9)), 4),
        }
    }
    fn logic(&self, kind: K, args: &[Printed]) -> Printed {
        if args.is_empty() {
            return p(
                tables::constants(
                    self.target,
                    if kind == K::LogicalAnd {
                        K::ConstantTrue
                    } else {
                        K::ConstantFalse
                    },
                )
                .unwrap(),
                11,
            );
        }
        if args.len() == 1 {
            return args[0].clone();
        }
        if self.target == Target::Diffrax {
            return fold(
                match kind {
                    K::LogicalAnd => "jnp.logical_and",
                    K::LogicalOr => "jnp.logical_or",
                    _ => "jnp.logical_xor",
                },
                args,
            );
        }
        if kind == K::LogicalXor {
            return match self.target {
                Target::Python => p(
                    args.iter()
                        .map(|a| format!("bool({})", a.code))
                        .collect::<Vec<_>>()
                        .join(" ^ "),
                    6,
                ),
                Target::Julia => call("xor", args),
                _ => args[1..]
                    .iter()
                    .fold(args[0].clone(), |a, b| call("xor", &[a, b.clone()])),
            };
        }
        infix(
            args,
            match (kind, self.target) {
                (K::LogicalAnd, Target::Python) => " and ",
                (K::LogicalOr, Target::Python) => " or ",
                (K::LogicalAnd, _) => " && ",
                _ => " || ",
            },
            if kind == K::LogicalAnd { 3 } else { 2 },
        )
    }
    fn relation(&self, kind: K, a: Printed, b: Printed) -> Printed {
        let code = infix(
            &[a, b],
            match kind {
                K::RelationalEq => " == ",
                K::RelationalNeq if self.target == Target::Julia => " != ",
                K::RelationalNeq if self.target != Target::R => " != ",
                K::RelationalNeq => " == ",
                K::RelationalGt => " > ",
                K::RelationalGeq => " >= ",
                K::RelationalLt => " < ",
                _ => " <= ",
            },
            5,
        );
        if self.target == Target::R {
            let code = call("isTRUE", &[code]);
            if kind == K::RelationalNeq {
                self.negate_bool(code)
            } else {
                code
            }
        } else {
            code
        }
    }
    fn power(&self, a: Printed, b: Printed) -> Printed {
        match self.target {
            Target::Diffrax => call("jnp.power", &[a, b]),
            Target::Julia => call("NaNMath.pow", &[a, b]),
            Target::Python => p(format!("{} ** {}", wrap(&a, 11), wrap(&b, 9)), 10),
            Target::R => p(format!("{} ^ {}", wrap(&a, 11), wrap(&b, 10)), 10),
        }
    }
    fn chain(&self, root: &Ast, depth: usize) -> Result<Printed, String> {
        enum Step<'a> {
            Visit(&'a Ast),
            Finish(K, usize),
        }
        let mut pending = vec![Step::Visit(root)];
        let mut values = Vec::<Printed>::new();
        while let Some(step) = pending.pop() {
            match step {
                Step::Visit(a) if matches!(a.kind, K::Plus | K::Minus | K::Times | K::Divide) => {
                    pending.push(Step::Finish(a.kind, a.children.len()));
                    pending.extend(a.children.iter().rev().map(Step::Visit));
                }
                Step::Visit(a) => values.push(self.print(a, false, depth + 1)?),
                Step::Finish(kind, n) => {
                    let args = values.split_off(values.len() - n);
                    let value = match kind {
                        K::Plus | K::Times if n == 0 => {
                            literal(self.target.integer(if kind == K::Plus { 0 } else { 1 }))
                        }
                        K::Plus | K::Times => infix(
                            &args,
                            if kind == K::Plus { " + " } else { " * " },
                            if kind == K::Plus { 7 } else { 8 },
                        ),
                        K::Minus if n == 1 => p(format!("-{}", wrap(&args[0], 10)), 9),
                        K::Minus | K::Divide if n == 2 => infix(
                            &args,
                            if kind == K::Minus { " - " } else { " / " },
                            if kind == K::Minus { 7 } else { 8 },
                        ),
                        _ => return Err(format!("Invalid {kind:?} arity")),
                    };
                    values.push(value);
                }
            }
        }
        values
            .pop()
            .ok_or_else(|| "The arithmetic expression is empty".into())
    }
    fn print(&self, a: &Ast, condition: bool, depth: usize) -> Result<Printed, String> {
        if depth > 100 {
            return Err("The expression exceeds the 100 level code generation limit.".into());
        }
        let is_bool = matches!(
            a.kind,
            K::ConstantTrue
                | K::ConstantFalse
                | K::LogicalAnd
                | K::LogicalOr
                | K::LogicalNot
                | K::LogicalXor
                | K::LogicalImplies
                | K::RelationalEq
                | K::RelationalNeq
                | K::RelationalGt
                | K::RelationalGeq
                | K::RelationalLt
                | K::RelationalLeq
        );
        if !condition && is_bool {
            return Ok(self.boolean_number(self.print(a, true, depth)?));
        }
        if condition
            && !is_bool
            && a.kind != K::FunctionPiecewise
            && matches!(self.target, Target::Julia | Target::Diffrax)
        {
            if let Some(n) = ast::value(a) {
                return Ok(p(
                    tables::constants(
                        self.target,
                        if n == 0.0 {
                            K::ConstantFalse
                        } else {
                            K::ConstantTrue
                        },
                    )
                    .unwrap(),
                    11,
                ));
            }
            return Ok(self.relation(
                K::RelationalNeq,
                self.print(a, false, depth)?,
                p(self.target.number(0.0), 11),
            ));
        }
        if matches!(a.kind, K::Plus | K::Minus | K::Times | K::Divide) {
            return self.chain(a, depth);
        }
        let child = |i: usize, c: bool| {
            a.children
                .get(i)
                .ok_or_else(|| format!("The {:?} expression lacks operand {}.", a.kind, i + 1))
                .and_then(|a| self.print(a, c, depth + 1))
        };
        if let Some(code) = tables::constants(self.target, a.kind) {
            return Ok(p(code, 11));
        }
        let out = match a.kind {
            K::Integer => literal(match a.number {
                Some(AstNumber::Integer(n)) => self.target.integer(n),
                _ => return Err("Invalid integer.".into()),
            }),
            K::Real | K::RealE => literal(
                self.target
                    .number(ast::value(a).ok_or("Invalid real number.")?),
            ),
            K::Rational => {
                let Some(AstNumber::Rational {
                    numerator,
                    denominator,
                }) = a.number
                else {
                    return Err("Invalid rational.".into());
                };
                infix(
                    &[
                        literal(self.target.integer(numerator)),
                        literal(self.target.integer(denominator)),
                    ],
                    " / ",
                    8,
                )
            }
            K::Name => p(
                self.symbols
                    .get(a.name.as_deref().unwrap_or(""))
                    .ok_or_else(|| format!("No code symbol for {:?}.", a.name))?
                    .clone(),
                11,
            ),
            K::NameAvogadro => literal(self.target.number(6.02214076e23)),
            K::Plus | K::Times => {
                if a.children.is_empty() {
                    literal(self.target.integer(if a.kind == K::Plus { 0 } else { 1 }))
                } else {
                    let args = (0..a.children.len())
                        .map(|i| child(i, false))
                        .collect::<Result<Vec<_>, _>>()?;
                    infix(
                        &args,
                        if a.kind == K::Plus { " + " } else { " * " },
                        if a.kind == K::Plus { 7 } else { 8 },
                    )
                }
            }
            K::Minus => {
                if a.children.len() == 1 {
                    let v = child(0, false)?;
                    p(format!("-{}", wrap(&v, 10)), 9)
                } else {
                    infix(&[child(0, false)?, child(1, false)?], " - ", 7)
                }
            }
            K::Divide => infix(&[child(0, false)?, child(1, false)?], " / ", 8),
            K::Power | K::FunctionPower => self.power(child(0, false)?, child(1, false)?),
            K::FunctionPiecewise => {
                let mut otherwise = if a.children.len() % 2 == 1 {
                    child(a.children.len() - 1, condition)?
                } else {
                    p(self.target.number(f64::NAN), 11)
                };
                for i in (0..a.children.len() / 2).rev() {
                    let value = child(2 * i, condition)?;
                    let test = child(2 * i + 1, true)?;
                    otherwise = match self.target {
                        Target::Diffrax => call("jnp.where", &[test, value, otherwise]),
                        Target::Python => p(
                            format!(
                                "{} if {} else {}",
                                wrap(&value, 2),
                                wrap(&test, 2),
                                wrap(&otherwise, 1)
                            ),
                            1,
                        ),
                        Target::Julia => p(
                            format!(
                                "{} ? {} : {}",
                                wrap(&test, 2),
                                wrap(&value, 2),
                                wrap(&otherwise, 1)
                            ),
                            1,
                        ),
                        Target::R => p(
                            format!(
                                "if ({}) {} else {}",
                                test.code,
                                wrap(&value, 2),
                                otherwise.code
                            ),
                            1,
                        ),
                    };
                }
                otherwise
            }
            K::LogicalNot => self.negate_bool(child(0, true)?),
            K::LogicalImplies => self.logic(
                K::LogicalOr,
                &[self.negate_bool(child(0, true)?), child(1, true)?],
            ),
            K::LogicalAnd | K::LogicalOr | K::LogicalXor => self.logic(
                a.kind,
                &(0..a.children.len())
                    .map(|i| child(i, true))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            K::RelationalEq
            | K::RelationalNeq
            | K::RelationalGt
            | K::RelationalGeq
            | K::RelationalLt
            | K::RelationalLeq => {
                if a.children.len() < 2 {
                    return Err("A relation needs at least two operands.".into());
                }
                let terms = (0..a.children.len() - 1)
                    .map(|i| Ok(self.relation(a.kind, child(i, false)?, child(i + 1, false)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                self.logic(K::LogicalAnd, &terms)
            }
            K::Function => {
                let name = a.name.as_deref().unwrap_or("");
                let code = self
                    .symbols
                    .get(name)
                    .ok_or_else(|| format!("No code symbol for '{name}'."))?;
                call(
                    code,
                    &(0..a.children.len())
                        .map(|i| child(i, false))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            K::FunctionLog => {
                let last = a
                    .children
                    .len()
                    .checked_sub(1)
                    .ok_or("log has no operand")?;
                let v = child(last, false)?;
                if last == 0 || ast::value(&a.children[0]) == Some(10.0) {
                    call(
                        &format!(
                            "{}log10",
                            if matches!(self.target, Target::Python | Target::Diffrax) {
                                format!("{}.", self.target.prefix())
                            } else if self.target == Target::Julia {
                                "NaNMath.".into()
                            } else {
                                String::new()
                            }
                        ),
                        &[v],
                    )
                } else {
                    let b = child(0, false)?;
                    match self.target {
                        Target::R => call("log", &[v, b]),
                        Target::Julia => call("log", &[b, v]),
                        _ => infix(
                            &[
                                call(&format!("{}.log", self.target.prefix()), &[v]),
                                call(&format!("{}.log", self.target.prefix()), &[b]),
                            ],
                            " / ",
                            8,
                        ),
                    }
                }
            }
            K::FunctionRoot => {
                let last = a
                    .children
                    .len()
                    .checked_sub(1)
                    .ok_or("root has no operand")?;
                let v = child(last, false)?;
                if last == 0 {
                    call(
                        match self.target {
                            Target::Python => "np.sqrt",
                            Target::Diffrax => "jnp.sqrt",
                            Target::Julia => "NaNMath.sqrt",
                            Target::R => "sqrt",
                        },
                        &[v],
                    )
                } else {
                    self.power(
                        v,
                        infix(
                            &[literal(self.target.integer(1)), child(0, false)?],
                            " / ",
                            8,
                        ),
                    )
                }
            }
            K::FunctionFactorial => call(
                match self.target {
                    Target::Python => "math.gamma",
                    Target::Diffrax => "jax.scipy.special.gamma",
                    _ => "gamma",
                },
                &[infix(
                    &[child(0, false)?, literal(self.target.integer(1))],
                    " + ",
                    7,
                )],
            ),
            K::FunctionQuotient => call(
                match self.target {
                    Target::Python => "np.trunc",
                    Target::Diffrax => "jnp.trunc",
                    _ => "trunc",
                },
                &[infix(&[child(0, false)?, child(1, false)?], " / ", 8)],
            ),
            K::FunctionRem => {
                let x = child(0, false)?;
                let y = child(1, false)?;
                if self.target == Target::R {
                    infix(
                        &[
                            call("sign", std::slice::from_ref(&x)),
                            infix(&[call("abs", &[x]), call("abs", &[y])], " %% ", 8),
                        ],
                        " * ",
                        8,
                    )
                } else {
                    call(
                        match self.target {
                            Target::Python => "np.fmod",
                            Target::Diffrax => "jnp.fmod",
                            _ => "rem",
                        },
                        &[x, y],
                    )
                }
            }
            K::FunctionMin | K::FunctionMax if self.target == Target::Diffrax => {
                let args = (0..a.children.len())
                    .map(|i| child(i, false))
                    .collect::<Result<Vec<_>, _>>()?;
                if args.is_empty() {
                    return Err("min/max needs an operand".into());
                }
                fold(
                    if a.kind == K::FunctionMin {
                        "jnp.minimum"
                    } else {
                        "jnp.maximum"
                    },
                    &args,
                )
            }
            _ => {
                if let Some(f) = tables::functions(self.target, a.kind) {
                    if a.children.is_empty()
                        || (!matches!(a.kind, K::FunctionMin | K::FunctionMax)
                            && a.children.len() != 1)
                    {
                        return Err(format!("Invalid {:?} arity", a.kind));
                    }
                    call(
                        f,
                        &(0..a.children.len())
                            .map(|i| child(i, false))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else if let Some(f) = tables::reciprocals(self.target, a.kind) {
                    infix(
                        &[
                            literal(self.target.integer(1)),
                            call(f, &[child(0, false)?]),
                        ],
                        " / ",
                        8,
                    )
                } else if let Some(f) = tables::of_reciprocals(self.target, a.kind) {
                    call(
                        f,
                        &[infix(
                            &[literal(self.target.integer(1)), child(0, false)?],
                            " / ",
                            8,
                        )],
                    )
                } else {
                    return Err(format!(
                        "The {:?} math is unsupported by the {} target.",
                        a.kind,
                        self.target.name()
                    ));
                }
            }
        };
        Ok(out)
    }
}

pub fn print(
    ast: &Ast,
    target: Target,
    symbols: &HashMap<String, String>,
    condition: bool,
) -> Result<String, String> {
    Printer { target, symbols }
        .print(ast, condition, 0)
        .map(|p| p.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_expressions_preserve_existing_semantics() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../testdata/ode/code_maths.json")).unwrap();
        let mut failures = vec![];
        for (i, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let Some(ast) = sbmlsim_reader::read_math(case["mathml"].as_str().unwrap()).unwrap()
            else {
                continue;
            };
            let symbols = serde_json::from_value(case["symbols"].clone()).unwrap();
            for target in [Target::Python, Target::Diffrax, Target::Julia, Target::R] {
                for condition in [false, true] {
                    let key = format!(
                        "{}_{}",
                        target.name(),
                        if condition { "condition" } else { "math" }
                    );
                    let expected = &case[&key];
                    let actual = print(&ast, target, &symbols, condition);
                    if let Some(text) = expected.get("ok").and_then(|v| v.as_str()) {
                        if actual.as_deref() != Ok(text) {
                            failures.push(
                                format!("case {i} {key}: {actual:?}; expected {text}")
                                    .chars()
                                    .take(500)
                                    .collect(),
                            );
                        }
                    } else if actual.is_ok() {
                        failures.push(format!("case {i} {key}: accepted unsupported expression"));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures
                .iter()
                .take(24)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

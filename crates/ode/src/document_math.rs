//! Textbook LaTeX used by the equation report, distinct from the MathML view.
use super::ast;
use sbmlsim_document::{Ast, AstKind as K, AstNumber};
use std::{borrow::Cow, collections::HashMap};

#[derive(Clone)]
struct Printed {
    code: String,
    precedence: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialect {
    Latex,
    Typst,
}
fn printed(code: impl Into<String>, precedence: u8) -> Printed {
    Printed {
        code: code.into(),
        precedence,
    }
}
fn signed(a: &Ast) -> bool {
    match a.kind {
        K::Plus | K::Minus => !a.children.is_empty(),
        K::Times if a.children.len() > 1 => {
            let f = &a.children[0];
            negative(f) || (f.kind == K::Minus && f.children.len() == 1)
        }
        _ => false,
    }
}
fn negative(a: &Ast) -> bool {
    matches!(a.kind, K::Integer | K::Real | K::RealE) && ast::value(a).is_some_and(|n| n < 0.0)
}
fn terms(root: &Ast) -> Vec<(bool, Cow<'_, Ast>)> {
    let mut out = Vec::new();
    let mut stack = vec![(false, Cow::Borrowed(root))];
    while let Some((sign, current)) = stack.pop() {
        if matches!(current.kind, K::Plus | K::Minus) && !current.children.is_empty() {
            let subtract = current.kind == K::Minus;
            let unary = subtract && current.children.len() == 1;
            match current {
                Cow::Borrowed(a) => {
                    for (i, c) in a.children.iter().enumerate().rev() {
                        stack.push((sign ^ (subtract && (i > 0 || unary)), Cow::Borrowed(c)));
                    }
                }
                Cow::Owned(mut a) => {
                    for (i, c) in std::mem::take(&mut a.children)
                        .into_iter()
                        .enumerate()
                        .rev()
                    {
                        stack.push((sign ^ (subtract && (i > 0 || unary)), Cow::Owned(c)));
                    }
                }
            }
        } else if negative(&current) {
            out.push((
                !sign,
                Cow::Owned(ast::number(-ast::value(&current).unwrap_or(0.0))),
            ));
        } else if current.kind == K::Times && signed(&current) {
            let first = &current.children[0];
            let factor = if first.kind == K::Minus {
                first.children[0].clone()
            } else {
                ast::number(-ast::value(first).unwrap_or(0.0))
            };
            let mut factors = Vec::new();
            if !ast::is(&factor, 1.0) {
                factors.push(factor);
            }
            factors.extend(current.children.iter().skip(1).cloned());
            stack.push((!sign, Cow::Owned(ast::product(factors))));
        } else {
            out.push((sign, current));
        }
    }
    out
}
fn supported_symbol(sid: &str) -> Result<(), String> {
    let mut chars = sid.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Err(format!(
            "The id '{sid}' is not an SId and cannot be typeset."
        ))
    } else {
        Ok(())
    }
}
fn chain_node(a: &Ast) -> bool {
    !signed(a)
        && ((a.kind == K::Times && a.children.len() >= 2)
            || (a.kind == K::Divide && a.children.len() == 2))
}

impl Dialect {
    pub(super) fn derivative(&self, plain: &str, linked: &str) -> String {
        if *self == Self::Typst {
            return format!("(dif {linked})/(dif t)");
        }
        let space = if plain.starts_with(r"\mathrm") {
            r"\,"
        } else {
            ""
        };
        format!(r"\frac{{\mathrm{{d}} {space}{linked}}}{{\mathrm{{d}} t}}")
    }
    pub(super) fn new_volume(&self, symbol: &str) -> String {
        if *self == Self::Typst {
            format!("{symbol}^(\"new\")")
        } else {
            format!(r"{symbol}^{{\mathrm{{new}}}}")
        }
    }
    pub(super) fn call_symbol(&self, symbol: &str, args: &[String]) -> String {
        format!("{symbol}{}", self.parentheses(&args.join(", ")))
    }

    fn parentheses(&self, code: &str) -> String {
        if *self == Self::Typst {
            format!("({code})")
        } else {
            format!(r"\mathopen{{}}\left({code}\right)")
        }
    }
    fn wrap(&self, p: &Printed, precedence: u8) -> String {
        if p.precedence < precedence {
            self.parentheses(&p.code)
        } else {
            p.code.clone()
        }
    }
    fn unsigned(&self, p: Printed) -> Printed {
        if p.precedence == 9 {
            printed(self.parentheses(&p.code), 11)
        } else {
            p
        }
    }
    fn negate(&self, p: Printed) -> Printed {
        printed(format!("-{}", self.wrap(&self.unsigned(p), 8)), 9)
    }
    fn literal(&self, code: String) -> Printed {
        let precedence = if code.starts_with('-') {
            9
        } else if code.contains(r" \times ") || code.contains(" times ") {
            8
        } else if code.contains('^') {
            10
        } else {
            11
        };
        printed(code, precedence)
    }
    fn call(&self, name: &str, args: Vec<Printed>) -> Printed {
        printed(
            format!(
                "{name}{}",
                self.parentheses(
                    &args
                        .into_iter()
                        .map(|p| p.code)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            ),
            10,
        )
    }
    pub(super) fn number(&self, value: f64) -> String {
        if value.is_nan() {
            return if *self == Self::Typst {
                "\"NaN\""
            } else {
                r"\mathrm{NaN}"
            }
            .into();
        }
        if value.is_infinite() {
            return match (*self, value < 0.0) {
                (Self::Typst, true) => "-infinity",
                (Self::Typst, false) => "infinity",
                (_, true) => r"-\infty",
                _ => r"\infty",
            }
            .into();
        }
        let text = sbmlsim_document::formatting::python_float(value + 0.0);
        let (m, e) = text.split_once('e').unwrap_or((&text, ""));
        let m = m.strip_suffix(".0").unwrap_or(m);
        if e.is_empty() {
            return m.to_owned();
        }
        let e = e.parse::<i32>().unwrap_or(0);
        if *self == Self::Typst {
            return if m == "1" {
                format!("10^({e})")
            } else if m == "-1" {
                format!("-10^({e})")
            } else {
                format!("{m} times 10^({e})")
            };
        }
        if m == "1" {
            format!("10^{{{e}}}")
        } else if m == "-1" {
            format!("-10^{{{e}}}")
        } else {
            format!(r"{m} \times 10^{{{e}}}")
        }
    }
    fn sum_codes(&self, terms: Vec<(bool, Printed)>) -> Vec<String> {
        let single = terms.len() == 1;
        terms
            .into_iter()
            .enumerate()
            .map(|(i, (negative, p))| {
                if i == 0 {
                    let p = if negative { self.negate(p) } else { p };
                    if single { p.code } else { self.wrap(&p, 7) }
                } else {
                    format!(
                        "{}{}",
                        if negative { "- " } else { "+ " },
                        self.wrap(&self.unsigned(p), 8)
                    )
                }
            })
            .collect()
    }
    fn logic(
        &self,
        operands: Vec<Printed>,
        operator: &str,
        precedence: u8,
        relations: bool,
    ) -> Printed {
        printed(
            operands
                .into_iter()
                .map(|p| {
                    if p.precedence == 4 || p.precedence >= 10 || (relations && p.precedence == 5) {
                        p.code
                    } else {
                        self.parentheses(&p.code)
                    }
                })
                .collect::<Vec<_>>()
                .join(operator),
            precedence,
        )
    }
    fn boolean(&self, p: Printed, condition: bool) -> Printed {
        if condition {
            p
        } else {
            printed(
                if *self == Self::Typst {
                    format!("[{}]", p.code)
                } else {
                    format!(r"\mathopen{{}}\left[{}\right]", p.code)
                },
                11,
            )
        }
    }
    fn lookup(&self, symbols: &HashMap<String, String>, sid: &str) -> Result<String, String> {
        if let Some(symbol) = symbols.get(sid) {
            return Ok(symbol.clone());
        }
        supported_symbol(sid)?;
        symbols.get(sid).cloned().ok_or_else(||format!("The identifier '{sid}' has no symbol and is not supported by the latex printer (e.g. a local parameter or a function definition which is not part of the symbols)."))
    }

    fn operation(&self, kind: K, mut args: Vec<Printed>) -> Printed {
        if kind == K::Divide {
            return printed(
                if *self == Self::Typst {
                    format!("({})/({})", args[0].code, args[1].code)
                } else {
                    format!(r"\frac{{{}}}{{{}}}", args[0].code, args[1].code)
                },
                10,
            );
        }
        let first = args.remove(0);
        let negative = first.precedence == 9;
        let mut parts = vec![self.wrap(&first, 8)];
        parts.extend(args.into_iter().map(|p| self.wrap(&self.unsigned(p), 9)));
        printed(
            parts.join(if *self == Self::Typst {
                " dot "
            } else {
                r" \cdot "
            }),
            if negative { 9 } else { 8 },
        )
    }
    fn render_chain(
        &self,
        a: &Ast,
        symbols: &HashMap<String, String>,
        depth: usize,
    ) -> Result<Printed, String> {
        let mut frames = vec![(a, 0, Vec::new())];
        loop {
            let Some((current, next, mut args)) = frames.pop() else {
                return Ok(printed("0", 11));
            };
            if let Some(child) = current.children.get(next) {
                if chain_node(child) {
                    frames.push((current, next + 1, args));
                    frames.push((child, 0, vec![]));
                } else {
                    args.push(self.render(child, symbols, false, depth + 1)?);
                    frames.push((current, next + 1, args));
                }
            } else {
                let result = self.operation(current.kind, args);
                if let Some(parent) = frames.last_mut() {
                    parent.2.push(result);
                } else {
                    return Ok(result);
                }
            }
        }
    }
    fn render(
        &self,
        a: &Ast,
        symbols: &HashMap<String, String>,
        condition: bool,
        depth: usize,
    ) -> Result<Printed, String> {
        if depth >= 100 {
            return Err(
                "The math is nested more than 100 levels deep, more than the latex printer writes."
                    .into(),
            );
        }
        if signed(a) {
            let rendered = terms(a)
                .into_iter()
                .map(|(s, a)| Ok((s, self.render(&a, symbols, false, depth + 1)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let single = rendered.len() == 1;
            let precedence = if single {
                if rendered[0].0 {
                    9
                } else {
                    rendered[0].1.precedence
                }
            } else {
                7
            };
            return Ok(printed(self.sum_codes(rendered).join(" "), precedence));
        }
        if chain_node(a) && a.children.iter().any(chain_node) {
            return self.render_chain(a, symbols, depth);
        }
        let mut args = Vec::new();
        for (i, c) in a.children.iter().enumerate() {
            let cond = match a.kind {
                K::LogicalAnd
                | K::LogicalOr
                | K::LogicalXor
                | K::LogicalNot
                | K::LogicalImplies => true,
                K::FunctionPiecewise => {
                    if i % 2 == 1 {
                        true
                    } else {
                        condition
                    }
                }
                _ => false,
            };
            args.push(self.render(c, symbols, cond, depth + 1)?);
        }
        let unsupported = || {
            format!(
                "The math '{}' is not supported by the latex printer.",
                sbmlsim_document::formatting::formula(a).unwrap_or_default()
            )
        };
        let p = match a.kind {
            K::Name => printed(self.lookup(symbols, a.name.as_deref().unwrap_or(""))?, 11),
            K::Function => self.call(
                &self.lookup(symbols, a.name.as_deref().unwrap_or(""))?,
                args,
            ),
            K::Integer => self.literal(match a.number {
                Some(AstNumber::Integer(n)) => n.to_string(),
                _ => "0".into(),
            }),
            K::Real | K::RealE => self.literal(self.number(ast::value(a).unwrap_or(f64::NAN))),
            K::Rational => match a.number {
                Some(AstNumber::Rational {
                    numerator,
                    denominator,
                }) => printed(
                    if *self == Self::Typst {
                        format!("({numerator})/({denominator})")
                    } else {
                        format!(r"\frac{{{numerator}}}{{{denominator}}}")
                    },
                    10,
                ),
                _ => return Err(unsupported()),
            },
            K::ConstantE => printed(
                if *self == Self::Typst {
                    "upright(\"e\")"
                } else {
                    r"\mathrm{e}"
                },
                11,
            ),
            K::ConstantPi => printed(if *self == Self::Typst { "pi" } else { r"\pi" }, 11),
            K::NameTime => printed("t", 11),
            K::NameAvogadro => printed(
                if *self == Self::Typst {
                    "N_(upright(\"A\"))"
                } else {
                    r"N_{\mathrm{A}}"
                },
                11,
            ),
            K::ConstantTrue | K::ConstantFalse => {
                return Ok(self.boolean(
                    printed(
                        if a.kind == K::ConstantTrue {
                            if *self == Self::Typst {
                                "\"true\""
                            } else {
                                r"\mathrm{true}"
                            }
                        } else {
                            if *self == Self::Typst {
                                "\"false\""
                            } else {
                                r"\mathrm{false}"
                            }
                        },
                        11,
                    ),
                    condition,
                ));
            }
            K::Plus => printed("0", 11),
            K::Times => {
                if args.is_empty() {
                    printed("1", 11)
                } else if args.len() == 1 {
                    args.remove(0)
                } else {
                    let first = args.remove(0);
                    let negative = first.precedence == 9;
                    let mut parts = vec![self.wrap(&first, 8)];
                    parts.extend(args.into_iter().map(|p| self.wrap(&self.unsigned(p), 9)));
                    printed(
                        parts.join(if *self == Self::Typst {
                            " dot "
                        } else {
                            r" \cdot "
                        }),
                        if negative { 9 } else { 8 },
                    )
                }
            }
            K::Divide if args.len() == 2 => printed(
                if *self == Self::Typst {
                    format!("({})/({})", args[0].code, args[1].code)
                } else {
                    format!(r"\frac{{{}}}{{{}}}", args[0].code, args[1].code)
                },
                10,
            ),
            K::Power | K::FunctionPower if args.len() == 2 => printed(
                if *self == Self::Typst {
                    format!("{}^({})", self.wrap(&args[0], 11), args[1].code)
                } else {
                    format!("{}^{{{}}}", self.wrap(&args[0], 11), args[1].code)
                },
                10,
            ),
            K::FunctionRoot | K::FunctionLog if !args.is_empty() && args.len() <= 2 => {
                let value = args.pop().unwrap();
                let degree = if a.children.len() == 2
                    && !ast::is(
                        &a.children[0],
                        if a.kind == K::FunctionRoot { 2.0 } else { 10.0 },
                    ) {
                    args.pop()
                } else {
                    None
                };
                if *self == Self::Typst {
                    if a.kind == K::FunctionRoot {
                        if let Some(degree) = degree {
                            self.call("root", vec![degree, value])
                        } else {
                            self.call("sqrt", vec![value])
                        }
                    } else {
                        printed(
                            format!(
                                "log_({}) ({})",
                                degree.map(|p| p.code).unwrap_or_else(|| "10".into()),
                                value.code
                            ),
                            10,
                        )
                    }
                } else if a.kind == K::FunctionRoot {
                    let index = degree
                        .map(|p| {
                            if p.code.contains(']') {
                                format!("[{{{}}}]", p.code)
                            } else {
                                format!("[{}]", p.code)
                            }
                        })
                        .unwrap_or_default();
                    printed(format!(r"\sqrt{index}{{{}}}", value.code), 10)
                } else {
                    self.call(
                        &format!(
                            r"\log_{{{}}}",
                            degree.map(|p| p.code).unwrap_or_else(|| "10".into())
                        ),
                        vec![value],
                    )
                }
            }
            K::FunctionFactorial if args.len() == 1 => {
                printed(format!("{}!", self.wrap(&args[0], 11)), 10)
            }
            K::FunctionAbs | K::FunctionCeiling | K::FunctionFloor if args.len() == 1 => {
                let (left, right) = if *self == Self::Typst {
                    match a.kind {
                        K::FunctionAbs => ("abs(", ")"),
                        K::FunctionCeiling => ("ceil(", ")"),
                        _ => ("floor(", ")"),
                    }
                } else {
                    match a.kind {
                        K::FunctionAbs => (r"\mathopen{}\left| ", r" \right|"),
                        K::FunctionCeiling => (r"\mathopen{}\left\lceil ", r" \right\rceil"),
                        _ => (r"\mathopen{}\left\lfloor ", r" \right\rfloor"),
                    }
                };
                printed(format!("{left}{}{right}", args[0].code), 11)
            }
            K::FunctionPiecewise => {
                let mut rows = Vec::new();
                for pair in args.as_chunks::<2>().0 {
                    rows.push(if *self == Self::Typst {
                        format!("{} & quad \"if\" {}", pair[0].code, pair[1].code)
                    } else {
                        format!(r"{} & \text{{if }} {}", pair[0].code, pair[1].code)
                    });
                }
                let otherwise = if args.len() % 2 == 1 {
                    args.last().unwrap().code.clone()
                } else {
                    self.number(f64::NAN)
                };
                rows.push(if *self == Self::Typst {
                    format!("{otherwise} & quad \"otherwise\"")
                } else {
                    format!(r"{otherwise} & \text{{otherwise}}")
                });
                printed(
                    if *self == Self::Typst {
                        format!("cases({})", rows.join(", "))
                    } else {
                        format!(r"\begin{{cases}} {} \end{{cases}}", rows.join(r" \\ "))
                    },
                    1,
                )
            }
            K::LogicalAnd | K::LogicalOr | K::LogicalXor | K::LogicalImplies => {
                let p = if args.is_empty() {
                    printed(
                        if a.kind == K::LogicalAnd {
                            if *self == Self::Typst {
                                "\"true\""
                            } else {
                                r"\mathrm{true}"
                            }
                        } else {
                            if *self == Self::Typst {
                                "\"false\""
                            } else {
                                r"\mathrm{false}"
                            }
                        },
                        11,
                    )
                } else if args.len() == 1 {
                    args.remove(0)
                } else {
                    let (op, precedence) = match a.kind {
                        K::LogicalAnd => (
                            if *self == Self::Typst {
                                " and "
                            } else {
                                r" \land "
                            },
                            3,
                        ),
                        K::LogicalOr => (
                            if *self == Self::Typst {
                                " or "
                            } else {
                                r" \lor "
                            },
                            2,
                        ),
                        K::LogicalXor => (
                            if *self == Self::Typst {
                                " xor "
                            } else {
                                r" \oplus "
                            },
                            2,
                        ),
                        _ => (
                            if *self == Self::Typst {
                                " => "
                            } else {
                                r" \Rightarrow "
                            },
                            2,
                        ),
                    };
                    self.logic(args, op, precedence, a.kind != K::LogicalXor)
                };
                return Ok(self.boolean(p, condition));
            }
            K::LogicalNot if args.len() == 1 => {
                let arg = &args[0];
                let code = if arg.precedence == 4 || arg.precedence >= 10 {
                    arg.code.clone()
                } else {
                    self.parentheses(&arg.code)
                };
                return Ok(self.boolean(
                    printed(
                        if *self == Self::Typst {
                            format!("not {code}")
                        } else {
                            format!(r"\lnot {code}")
                        },
                        4,
                    ),
                    condition,
                ));
            }
            K::RelationalEq
            | K::RelationalNeq
            | K::RelationalGt
            | K::RelationalGeq
            | K::RelationalLt
            | K::RelationalLeq => {
                let op = match a.kind {
                    K::RelationalEq => "=",
                    K::RelationalNeq => {
                        if *self == Self::Typst {
                            "!="
                        } else {
                            r"\neq"
                        }
                    }
                    K::RelationalGt => ">",
                    K::RelationalGeq => {
                        if *self == Self::Typst {
                            ">="
                        } else {
                            r"\geq"
                        }
                    }
                    K::RelationalLt => "<",
                    _ => {
                        if *self == Self::Typst {
                            "<="
                        } else {
                            r"\leq"
                        }
                    }
                };
                let relations = args
                    .windows(2)
                    .map(|p| {
                        printed(
                            format!("{} {op} {}", self.wrap(&p[0], 6), self.wrap(&p[1], 6)),
                            5,
                        )
                    })
                    .collect::<Vec<_>>();
                let p = if relations.len() == 1 {
                    relations[0].clone()
                } else if relations.is_empty() {
                    printed(
                        if *self == Self::Typst {
                            "\"true\""
                        } else {
                            r"\mathrm{true}"
                        },
                        11,
                    )
                } else {
                    self.logic(
                        relations,
                        if *self == Self::Typst {
                            " and "
                        } else {
                            r" \land "
                        },
                        3,
                        true,
                    )
                };
                return Ok(self.boolean(p, condition));
            }
            K::FunctionDelay | K::FunctionRateOf | K::Lambda | K::Unknown | K::Other => {
                return Err(unsupported());
            }
            _ => {
                let debug = format!("{:?}", a.kind);
                let function = debug
                    .strip_prefix("Function")
                    .or_else(|| debug.strip_prefix("Distrib"))
                    .ok_or_else(unsupported)?
                    .to_lowercase();
                if matches!(a.kind, K::FunctionMin | K::FunctionMax) && args.len() == 1 {
                    args.remove(0)
                } else {
                    let operator = if *self == Self::Typst {
                        if matches!(
                            function.as_str(),
                            "arccos"
                                | "arcsin"
                                | "arctan"
                                | "cos"
                                | "cosh"
                                | "cot"
                                | "coth"
                                | "csc"
                                | "csch"
                                | "exp"
                                | "ln"
                                | "max"
                                | "min"
                                | "sec"
                                | "sech"
                                | "sin"
                                | "sinh"
                                | "tan"
                                | "tanh"
                        ) {
                            function.clone()
                        } else {
                            format!("op(\"{function}\")")
                        }
                    } else if matches!(
                        function.as_str(),
                        "arccos"
                            | "arcsin"
                            | "arctan"
                            | "cos"
                            | "cosh"
                            | "cot"
                            | "coth"
                            | "csc"
                            | "exp"
                            | "ln"
                            | "max"
                            | "min"
                            | "sec"
                            | "sin"
                            | "sinh"
                            | "tan"
                            | "tanh"
                    ) {
                        format!("\\{function}")
                    } else {
                        format!(r"\operatorname{{{function}}}")
                    };
                    self.call(&operator, args)
                }
            }
        };
        Ok(p)
    }

    pub(super) fn print(
        &self,
        ast: &Ast,
        symbols: &HashMap<String, String>,
        condition: bool,
    ) -> Result<String, String> {
        Ok(self.render(ast, symbols, condition, 0)?.code)
    }
    pub(super) fn lines(
        &self,
        ast: &Ast,
        symbols: &HashMap<String, String>,
    ) -> Result<Vec<String>, String> {
        let parts = terms(ast)
            .into_iter()
            .map(|(s, a)| Ok((s, self.render(&a, symbols, false, 0)?)))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(self
            .sum_codes(parts)
            .chunks(4)
            .map(|c| c.join(" "))
            .collect())
    }
    pub(super) fn symbol(&self, sid: &str) -> Result<String, String> {
        supported_symbol(sid)?;
        Ok(self.typeset(sid, true))
    }
    fn text(&self, s: &str) -> String {
        if *self == Self::Typst {
            format!("upright(\"{s}\")")
        } else {
            format!(r"\mathrm{{{}}}", s.replace('_', r"\_"))
        }
    }
    fn typeset(&self, sid: &str, greek: bool) -> String {
        let digit = sid.find(|c: char| c.is_ascii_digit());
        let numbered = digit.filter(|i| {
            *i > 0
                && sid[..*i].chars().all(|c| c.is_ascii_alphabetic())
                && sid[*i..].chars().all(|c| c.is_ascii_digit())
        });
        let numbered = numbered.map(|i| format!("{}_{}", &sid[..i], &sid[i..]));
        let sid = numbered.as_deref().unwrap_or(sid);
        let (base, sub) = sid
            .split_once('_')
            .map_or((sid, None), |(b, s)| (b, Some(s)));
        if base.is_empty() || sub == Some("") {
            return self.text(sid);
        }
        let is_greek = [
            "alpha", "beta", "gamma", "delta", "zeta", "eta", "theta", "iota", "kappa", "lambda",
            "mu", "nu", "xi", "pi", "rho", "sigma", "tau", "upsilon", "chi", "psi", "omega",
            "Gamma", "Delta", "Theta", "Lambda", "Xi", "Pi", "Sigma", "Upsilon", "Phi", "Psi",
            "Omega",
        ]
        .contains(&base);
        let base = if greek && base == "epsilon" {
            if *self == Self::Typst {
                "epsilon"
            } else {
                r"\varepsilon"
            }
            .into()
        } else if greek && base == "phi" {
            if *self == Self::Typst {
                "phi"
            } else {
                r"\varphi"
            }
            .into()
        } else if greek && base == "omicron" {
            "o".into()
        } else if greek && is_greek {
            if *self == Self::Typst {
                base.to_owned()
            } else {
                format!("\\{base}")
            }
        } else if base.len() == 1 {
            base.into()
        } else {
            self.text(base)
        };
        sub.map_or(base.clone(), |s| {
            format!(
                "{}{}{}{}",
                base,
                if *self == Self::Typst { "_(" } else { "_{" },
                if s.chars().all(|c| c.is_ascii_digit()) {
                    s.into()
                } else if *self == Self::Typst {
                    format!("\"{s}\"")
                } else {
                    self.text(s)
                },
                if *self == Self::Typst { ")" } else { "}" }
            )
        })
    }
    pub(super) fn symbols(&self, ids: &[String]) -> Result<HashMap<String, String>, String> {
        let mut result = HashMap::new();
        for id in ids {
            result.insert(id.clone(), self.symbol(id)?);
        }
        let mut counts = HashMap::new();
        for v in result.values() {
            *counts.entry(v.clone()).or_insert(0) += 1;
        }
        for id in ids {
            if counts[&result[id]] > 1
                && id.chars().any(|c| c.is_ascii_digit())
                && !id.contains('_')
            {
                result.insert(id.clone(), self.text(id));
            }
        }
        let mut counts = HashMap::new();
        for v in result.values() {
            *counts.entry(v.clone()).or_insert(0) += 1;
        }
        for id in ids {
            if counts[&result[id]] > 1 {
                result.insert(id.clone(), self.typeset(id, false));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typst_uses_the_same_mathematical_traversal() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../testdata/ode/typst_maths.json")).unwrap();
        let mut errors = vec![];
        for (i, case) in fixtures["cases"].as_array().unwrap().iter().enumerate() {
            let Some(ast) = sbmlsim_reader::read_math(case["mathml"].as_str().unwrap()).unwrap()
            else {
                continue;
            };
            let symbols = serde_json::from_value(case["symbols"].clone()).unwrap();
            for mode in ["math", "condition", "lines"] {
                let actual = if mode == "lines" {
                    Dialect::Typst
                        .lines(&ast, &symbols)
                        .map(|v| serde_json::json!(v))
                } else {
                    Dialect::Typst
                        .print(&ast, &symbols, mode == "condition")
                        .map(|v| serde_json::json!(v))
                };
                if let Some(expected) = case[mode].get("ok") {
                    if actual.as_ref() != Ok(expected) {
                        errors.push(
                            format!("case {i} {mode}: got {actual:?}, expected {expected}")
                                .chars()
                                .take(400)
                                .collect::<String>(),
                        );
                    }
                } else if actual.is_ok() {
                    errors.push(format!("case {i} {mode}: accepted unsupported math"));
                }
            }
        }
        assert!(
            errors.is_empty(),
            "{} differences:\n{}",
            errors.len(),
            errors
                .iter()
                .take(18)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

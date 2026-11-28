//! The reductions of a formula over the time of a simulation, sbmlsim's `reduce_formula` and
//! `evaluate_reduced` (`simulator/formula.py`): `max(x)`, `min(x)`, `mean(x)` and `at(x, t)`
//! are replaced by symbols which stand for their values, an inner reduction before the one
//! whose argument it is, so the formula outside of them and their arguments are formulas of
//! the math of PEtab. `max` and `min` of several arguments are the functions of PEtab.

use crate::formula::CompiledFormula;
use sbmlsim_timecourse::grid_weights;

/// The prefix of the symbol which stands for the value of a reduction.
const PLACEHOLDER: &str = "sbmlsim_reduction__";

/// The function of a reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Function {
    Max,
    Min,
    Mean,
    At,
}

impl Function {
    fn of(name: &str) -> Function {
        match name {
            "max" => Function::Max,
            "min" => Function::Min,
            "mean" => Function::Mean,
            _ => Function::At,
        }
    }

    /// How it is called, for a message.
    fn call(self) -> &'static str {
        match self {
            Function::Max => "max(x)",
            Function::Min => "min(x)",
            Function::Mean => "mean(x)",
            Function::At => "at(x, t)",
        }
    }
}

/// A reduction: the symbol of its value, its function and its arguments.
#[derive(Debug, Clone)]
pub(crate) struct Reduction {
    pub symbol: String,
    pub function: Function,
    pub arguments: Vec<CompiledFormula>,
}

/// A formula whose reductions are replaced by symbols.
#[derive(Debug, Clone)]
pub(crate) struct Reduced {
    /// The formula as it was given.
    pub formula: String,
    /// The formula outside of the reductions.
    pub outer: CompiledFormula,
    /// The reductions, an inner one first.
    pub reductions: Vec<Reduction>,
}

/// The value of a formula at one point: a number (a value per simulation) or a value per row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Scalar(f64),
    Series(Vec<f64>),
}

impl Val {
    /// The value at a row.
    pub fn at(&self, row: usize) -> f64 {
        match self {
            Val::Scalar(x) => *x,
            Val::Series(xs) => xs[row],
        }
    }
}

/// Whether a reduction call starts at a position: the name is no end of a longer identifier.
fn call_at(chars: &[char], i: usize) -> Option<(&'static str, usize)> {
    if i > 0 && (chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '.')
    {
        return None;
    }
    for name in ["max", "min", "mean", "at"] {
        let n = name.len();
        if i + n <= chars.len() && chars[i..i + n].iter().copied().eq(name.chars()) {
            let mut j = i + n;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '(' {
                return Some((name, j + 1));
            }
        }
    }
    None
}

impl Reduced {
    /// Reads the reductions of a formula and compiles every part of it.
    ///
    /// # Errors
    ///
    /// Unbalanced parentheses, a reduction without its number of arguments, or a part which is
    /// no math of PEtab.
    pub fn new(formula: &str) -> Result<Reduced, String> {
        let mut texts: Vec<(Function, Vec<String>)> = Vec::new();
        let chars: Vec<char> = formula.chars().collect();
        let outer = replace(&chars, formula, &mut texts)?;
        let compile = |part: &str| {
            CompiledFormula::new(part)
                .map_err(|err| format!("The formula '{formula}' is not valid math: {err}"))
        };
        let outer = compile(&outer)?;
        let reductions = texts
            .into_iter()
            .enumerate()
            .map(|(k, (function, arguments))| {
                Ok(Reduction {
                    symbol: format!("{PLACEHOLDER}{k}"),
                    function,
                    arguments: arguments
                        .iter()
                        .map(|a| compile(a))
                        .collect::<Result<_, String>>()?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Reduced {
            formula: formula.to_string(),
            outer,
            reductions,
        })
    }

    fn placeholder(name: &str) -> bool {
        name.starts_with(PLACEHOLDER)
    }

    /// The symbols the formula reads, also inside of its reductions, sorted; `time` among them
    /// where a part reads the time.
    pub fn symbols(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let parts = std::iter::once(&self.outer)
            .chain(self.reductions.iter().flat_map(|r| r.arguments.iter()));
        for part in parts {
            for s in part.symbols() {
                if !Self::placeholder(s) && !found.contains(s) {
                    found.push(s.clone());
                }
            }
            if part.reads_time() && !found.iter().any(|s| s == "time") {
                found.push("time".to_string());
            }
        }
        found.sort();
        found
    }

    /// The symbols the formula reads outside of its reductions, `time` among them where it
    /// reads the time.
    pub fn outer_symbols(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .outer
            .symbols()
            .iter()
            .filter(|s| !Self::placeholder(s))
            .cloned()
            .collect();
        if self.outer.reads_time() {
            found.push("time".to_string());
        }
        found
    }

    /// The symbols the times of the `at` reductions read.
    pub fn time_symbols(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for r in self
            .reductions
            .iter()
            .filter(|r| r.function == Function::At)
        {
            for s in r.arguments[1].symbols() {
                if !Self::placeholder(s) && !found.contains(s) {
                    found.push(s.clone());
                }
            }
            if r.arguments[1].reads_time() && !found.iter().any(|s| s == "time") {
                found.push("time".to_string());
            }
        }
        found.sort();
        found
    }
}

/// The text with every reduction replaced by its symbol, the inner ones first, sbmlsim's
/// `_replace_reductions`.
fn replace(
    chars: &[char],
    formula: &str,
    reductions: &mut Vec<(Function, Vec<String>)>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut position = 0;
    let mut i = 0;
    while i < chars.len() {
        let Some((name, open)) = call_at(chars, i) else {
            i += 1;
            continue;
        };
        let end = closing(chars, open, formula)?;
        let mut arguments = Vec::new();
        for argument in split(&chars[open..end]) {
            arguments.push(replace(&argument, formula, reductions)?);
        }
        out.extend(&chars[position..i]);
        if (name == "max" || name == "min") && arguments.len() > 1 {
            out.push_str(&format!("{name}({})", arguments.join(",")));
        } else {
            let function = Function::of(name);
            let expected = if function == Function::At { 2 } else { 1 };
            if arguments.len() != expected || arguments.iter().any(|a| a.trim().is_empty()) {
                return Err(format!(
                    "The reduction '{name}' of the formula '{formula}' is called as {}.",
                    function.call()
                ));
            }
            out.push_str(&format!("{PLACEHOLDER}{}", reductions.len()));
            reductions.push((function, arguments));
        }
        position = end + 1;
        i = position;
    }
    out.extend(&chars[position.min(chars.len())..]);
    Ok(out)
}

/// The parenthesis which closes the one opened before `start`.
fn closing(chars: &[char], start: usize, formula: &str) -> Result<usize, String> {
    let mut depth = 1;
    for (k, c) in chars.iter().enumerate().skip(start) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(k);
                }
            }
            _ => {}
        }
    }
    Err(format!(
        "The parentheses of the formula '{formula}' are not balanced."
    ))
}

/// The arguments of a call, split at the commas outside of parentheses.
fn split(chars: &[char]) -> Vec<Vec<char>> {
    let mut arguments = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (k, c) in chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                arguments.push(chars[start..k].to_vec());
                start = k + 1;
            }
            _ => {}
        }
    }
    arguments.push(chars[start..].to_vec());
    arguments
}

/// The time points of one point and what `at` needs of its plan.
pub(crate) struct Times<'a> {
    /// The time of every row, in the time of the result.
    pub time: &'a [f64],
    /// The resolution of the time of the simulation.
    pub tolerance: f64,
    /// The times of the changes after the start, in the time of the result.
    pub changes: &'a [f64],
}

/// Evaluates a part of a formula at one point.
pub(crate) fn evaluate_part(
    part: &CompiledFormula,
    scope: &dyn Fn(&str) -> Option<Val>,
    times: &Times<'_>,
    formula: &str,
) -> Result<Val, String> {
    let mut values = Vec::with_capacity(part.symbols().len());
    let mut missing = Vec::new();
    for symbol in part.symbols() {
        match scope(symbol) {
            Some(value) => values.push(value),
            None => missing.push(symbol.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "ValueError: The formula '{formula}' reads {}, which have no values.",
            crate::text::list(&missing)
        ));
    }
    let series = part.reads_time() || values.iter().any(|v| matches!(v, Val::Series(_)));
    if !series {
        let xs: Vec<f64> = values.iter().map(|v| v.at(0)).collect();
        return Ok(Val::Scalar(part.evaluate(&xs, f64::NAN)));
    }
    let mut xs = vec![0.0; values.len()];
    let out = (0..times.time.len())
        .map(|row| {
            for (x, v) in xs.iter_mut().zip(&values) {
                *x = v.at(row);
            }
            part.evaluate(&xs, times.time[row])
        })
        .collect();
    Ok(Val::Series(out))
}

/// Evaluates a formula with its reductions at one point, sbmlsim's `evaluate_reduced`.
pub(crate) fn evaluate(
    reduced: &Reduced,
    scope: &dyn Fn(&str) -> Option<Val>,
    times: &Times<'_>,
) -> Result<Val, String> {
    let mut placeholders: Vec<(String, Val)> = Vec::new();
    for reduction in &reduced.reductions {
        let lookup = |name: &str| {
            placeholders
                .iter()
                .find(|(s, _)| s == name)
                .map(|(_, v)| v.clone())
                .or_else(|| scope(name))
        };
        let x = evaluate_part(&reduction.arguments[0], &lookup, times, &reduced.formula)?;
        let value = match reduction.function {
            Function::Max | Function::Min => extreme(reduction.function == Function::Max, x, times),
            Function::Mean => mean(x, times),
            Function::At => {
                let when =
                    evaluate_part(&reduction.arguments[1], &lookup, times, &reduced.formula)?;
                at(x, &when, times)
            }
        };
        placeholders.push((reduction.symbol.clone(), value));
    }
    let lookup = |name: &str| {
        placeholders
            .iter()
            .find(|(s, _)| s == name)
            .map(|(_, v)| v.clone())
            .or_else(|| scope(name))
    };
    evaluate_part(&reduced.outer, &lookup, times, &reduced.formula)
}

/// Whether a value is constant in time, a number or a value of one row: a reduction returns it
/// as it is (`_constant`).
fn constant(x: &Val) -> bool {
    match x {
        Val::Scalar(_) => true,
        Val::Series(xs) => xs.len() == 1,
    }
}

/// The largest or the smallest value at the finite times, ignoring `NaN` (`np.fmax.reduce`).
fn extreme(max: bool, x: Val, times: &Times<'_>) -> Val {
    if constant(&x) {
        return x;
    }
    let Val::Series(xs) = x else { unreachable!() };
    Val::Scalar(sbmlsim_timecourse::extreme(times.time, max, |row| xs[row]).0)
}

/// The time weighted mean, see [`sbmlsim_timecourse::mean`].
fn mean(x: Val, times: &Times<'_>) -> Val {
    if constant(&x) {
        return x;
    }
    let Val::Series(m) = x else { unreachable!() };
    Val::Scalar(sbmlsim_timecourse::mean(times.time, |row| m[row]))
}

/// The value at a time, interpolated with the rules of `grid_weights`.
fn at(x: Val, when: &Val, times: &Times<'_>) -> Val {
    if constant(&x) {
        return x;
    }
    let Val::Series(xs) = x else { unreachable!() };
    let target = when.at(0);
    let weights = grid_weights(times.time, &[target], times.tolerance, times.changes, None);
    Val::Scalar(weights[0].value(&xs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reductions_are_replaced_inner_first() {
        let r = Reduced::new("max(at([e__A], 5)) / mean(x) + max(a, b)").unwrap();
        assert_eq!(r.reductions.len(), 3);
        assert_eq!(r.reductions[0].function, Function::At);
        assert_eq!(r.reductions[1].function, Function::Max);
        assert_eq!(r.symbols(), ["[e__A]", "a", "b", "x"]);
        assert!(r.outer_symbols().iter().all(|s| s == "a" || s == "b"));
        assert!(Reduced::new("mean(x, y)").is_err());
        assert!(Reduced::new("max(x").is_err());
    }

    #[test]
    fn a_reduction_over_the_rows() {
        let time = [0.0, 1.0, 2.0, f64::INFINITY];
        let times = Times {
            time: &time,
            tolerance: 1e-9,
            changes: &[],
        };
        let x = Val::Series(vec![0.0, 2.0, 4.0, 100.0]);
        let scope = |s: &str| (s == "x").then(|| x.clone());
        let v = |f: &str| evaluate(&Reduced::new(f).unwrap(), &scope, &times).unwrap();
        assert_eq!(v("max(x)"), Val::Scalar(4.0));
        assert_eq!(v("mean(x)"), Val::Scalar(2.0));
        assert_eq!(v("at(x, 1.5)"), Val::Scalar(3.0));
        assert_eq!(v("2 * x"), Val::Series(vec![0.0, 4.0, 8.0, 200.0]));
    }

    #[test]
    fn reductions_are_the_ones_of_sbmlsim_to_the_bit() {
        // the values of sbmlsim's `evaluate_reduced` on this point, a change at 1.3 and the
        // steady state after the end
        let time = [0.0, 0.5, 1.3, 1.3, 2.7, f64::INFINITY];
        let times = Times {
            time: &time,
            tolerance: 1e-9,
            changes: &[1.3],
        };
        let x = Val::Series(vec![1.0, 2.5, 3.1, 7.2, 4.4, 9.9]);
        let scope = |s: &str| (s == "x").then(|| x.clone());
        for (formula, expected) in [
            ("max(x)", 7.2),
            ("min(x)", 1.0),
            ("mean(x)", 4.161111111111111),
            ("at(x, 1.3)", 7.2),
            ("at(x, 2.0)", 5.8),
            ("at(x, 0.25)", 1.75),
            ("mean(x * 3 / 7)", 1.7833333333333334),
            ("max(x) / mean(x) + 0.1", 1.8303070761014686),
        ] {
            let value = evaluate(&Reduced::new(formula).unwrap(), &scope, &times).unwrap();
            let Val::Scalar(value) = value else {
                panic!("{formula}: no value per simulation")
            };
            assert_eq!(
                value.to_bits(),
                f64::to_bits(expected),
                "{formula}: {value}"
            );
        }
    }
}

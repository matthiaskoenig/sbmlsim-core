//! The meaning of every operator of the model representation, as libroadrunner 2.11 computes
//! it, and its native evaluation.
//!
//! The emitter of `sbmlsim-codegen` (its `wasm.rs`) and the native evaluator here implement
//! the same operations in the same order, so the initial values of the description are the bits
//! `model_init` computes (the tests of `sbmlsim-host` compare both), and the optimizer
//! ([`crate::optimize`]) folds an operation of literal operands into the double the emitted code
//! computes. The mappings, each probed against libroadrunner 2.11.1
//! (`roadrunner.RoadRunner(sbml).getValue(...)` of assignment rules over parameters):
//!
//! - `plus`, `times`: a left fold, `((a + b) + c)`; without arguments `0` and `1`.
//! - unary `minus`: `0 - x` (so `-(0)` is `+0`); binary `a - b`.
//! - `power`: `pow(a, b)`; `root(n, x)`: `pow(x, 1 / n)`, `root(x)`: `pow(x, 0.5)` (so the
//!   root of `-0` is `+0` and of `-inf` is `inf`, as `pow` and not `sqrt` gives them).
//! - `ln`: `log`; `log(b, x)`: `log(x) / log(b)`, `log(x)`: `log(x) / log(10)` (not `log10`:
//!   the base 10 logarithm of 1000 is `2.9999999999999996`).
//! - `abs`, `floor`, `ceiling`: the IEEE operations (`f64.abs`, `f64.floor`, `f64.ceil`).
//! - `exp`, `sin`, `cos`, `tan`, `sinh`, `cosh`, `tanh`, `arcsin`, `arccos`, `arctan`,
//!   `factorial`: the functions of mathlib.
//! - `sec`, `csc`, `cot`, `sech`, `csch`: `1 / cos`, `1 / sin`, `1 / tan`, `1 / cosh`,
//!   `1 / sinh`; `coth`: `cosh(x) / sinh(x)`.
//! - `arcsec`, `arccsc`: `acos(1 / x)`, `asin(1 / x)`; `arccot`: `pi / 2` where `x == 0`
//!   (also `-0`), else `atan(1 / x)`.
//! - `arcsinh`: `log(x + sqrt(x * x + 1))`; `arccosh`: `log(x + sqrt(x * x - 1))`; `arctanh`:
//!   `(log(1 + x) - log(1 - x)) * 0.5`; `arcsech`: `log(r + sqrt(r - 1) * sqrt(r + 1))` with
//!   `r = 1 / x`; `arccsch`: `log(1 / x + sqrt(1 / (x * x) + 1))`; `arccoth`:
//!   `(log(1 + r) - log(1 - r)) * 0.5` with `r = 1 / x` (libroadrunner's support functions,
//!   not `asinh`, `acosh`, `atanh` of libm).
//! - `max(a, b)`: `a > b ? a : b`, `min(a, b)`: `a < b ? a : b` (so `max(NaN, 1)` is `1`,
//!   `max(1, NaN)` is NaN, `max(0, -0)` is `-0`), a left fold over more arguments, `max()` is
//!   `-inf` and `min()` is `inf`.
//! - `rem`: `fmod` (the remainder of the truncated division); `quotient`: the quotient `a / b`
//!   truncated to an `i32` as x86 does it, `-2147483648` where it does not fit (also NaN).
//! - relations are unordered, true where an operand is NaN (`NaN == NaN`, `NaN != NaN`,
//!   `NaN < 1` are all true); more than two operands are a chain (`a < b and b < c`), fewer
//!   than two are true.
//! - `and`, `or`, `xor`: folds, without arguments `true`, `false`, `false`; `xor` is the
//!   parity; `implies(a, b)`: `not a or b`.
//! - a boolean as a number is `1` or `0`; a number as a boolean is true where it is neither
//!   zero nor NaN.
//! - `piecewise`: the first piece whose condition is true, else `otherwise`, else NaN.

use crate::{Expr, Op, Symbol};
use sbmlsim_mathlib::f;

/// `pi / 2`, the `arccot` of `0`.
pub const HALF_PI: f64 = std::f64::consts::FRAC_PI_2;
/// The smallest quotient which truncates to an `i32`, exclusive.
pub const QUOTIENT_LOW: f64 = -2147483649.0;
/// The largest quotient which truncates to an `i32`, exclusive.
pub const QUOTIENT_HIGH: f64 = 2147483648.0;
/// The quotient where the truncation does not fit an `i32`.
pub const QUOTIENT_INVALID: f64 = -2147483648.0;

/// Whether an operator gives a boolean.
pub fn is_boolean(op: Op) -> bool {
    matches!(
        op,
        Op::Eq
            | Op::Neq
            | Op::Gt
            | Op::Lt
            | Op::Geq
            | Op::Leq
            | Op::And
            | Op::Or
            | Op::Xor
            | Op::Not
            | Op::Implies
    )
}

/// An unordered relation of two numbers (true where one is NaN), written as the negations of the
/// ordered comparisons the emitted code uses.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::nonminimal_bool,
    clippy::double_comparisons,
    reason = "the negations of ordered comparisons are the unordered relations"
)]
pub fn relation(op: Op, a: f64, b: f64) -> bool {
    match op {
        Op::Eq => !(a < b || a > b),
        Op::Neq => a != b,
        Op::Gt => !(a <= b),
        Op::Lt => !(a >= b),
        Op::Geq => !(a < b),
        Op::Leq => !(a > b),
        _ => unreachable!("{op:?} is no relation"),
    }
}

/// `a / b` truncated to an `i32` as x86 does it.
pub fn quotient(a: f64, b: f64) -> f64 {
    let q = a / b;
    if q > QUOTIENT_LOW && q < QUOTIENT_HIGH {
        f64::from(q as i32)
    } else {
        QUOTIENT_INVALID
    }
}

/// The native evaluation of an expression; `read` gives the value of a symbol.
pub fn eval(e: &Expr, read: &dyn Fn(Symbol) -> f64) -> f64 {
    match e {
        Expr::Number(x) => *x,
        Expr::Bool(b) => f64::from(u8::from(*b)),
        Expr::Symbol(s) => read(*s),
        Expr::Apply { op, .. } if is_boolean(*op) => f64::from(u8::from(eval_bool(e, read))),
        Expr::Apply { op, args } => apply(*op, args, read),
        Expr::Piecewise { pieces, otherwise } => {
            for (value, condition) in pieces {
                if eval_bool(condition, read) {
                    return eval(value, read);
                }
            }
            otherwise.as_ref().map_or(f64::NAN, |o| eval(o, read))
        }
    }
}

/// The native evaluation of an expression as a boolean.
pub fn eval_bool(e: &Expr, read: &dyn Fn(Symbol) -> f64) -> bool {
    match e {
        Expr::Bool(b) => *b,
        Expr::Apply { op, args } if is_boolean(*op) => {
            let b = |i: usize| eval_bool(&args[i], read);
            match op {
                Op::And => (0..args.len()).fold(true, |acc, i| acc & b(i)),
                Op::Or => (0..args.len()).fold(false, |acc, i| acc | b(i)),
                Op::Xor => (0..args.len()).fold(false, |acc, i| acc ^ b(i)),
                Op::Not => !b(0),
                Op::Implies => !b(0) | b(1),
                _ => {
                    let values: Vec<f64> = args.iter().map(|a| eval(a, read)).collect();
                    values.windows(2).all(|w| relation(*op, w[0], w[1]))
                }
            }
        }
        _ => eval(e, read).abs() > 0.0,
    }
}

fn apply(op: Op, args: &[Expr], read: &dyn Fn(Symbol) -> f64) -> f64 {
    let x = |i: usize| eval(&args[i], read);
    match op {
        Op::Plus => match args.split_first() {
            None => 0.0,
            Some((first, rest)) => rest
                .iter()
                .fold(eval(first, read), |acc, a| acc + eval(a, read)),
        },
        Op::Times => match args.split_first() {
            None => 1.0,
            Some((first, rest)) => rest
                .iter()
                .fold(eval(first, read), |acc, a| acc * eval(a, read)),
        },
        Op::Minus if args.len() == 1 => 0.0 - x(0),
        Op::Minus => x(0) - x(1),
        Op::Divide => x(0) / x(1),
        Op::Power => f::math_pow(x(0), x(1)),
        Op::Root if args.len() == 1 => f::math_pow(x(0), 0.5),
        Op::Root => f::math_pow(x(1), 1.0 / x(0)),
        Op::Abs => x(0).abs(),
        Op::Floor => x(0).floor(),
        Op::Ceiling => x(0).ceil(),
        Op::Exp => f::math_exp(x(0)),
        Op::Ln => f::math_log(x(0)),
        Op::Log if args.len() == 1 => f::math_log(x(0)) / f::math_log(10.0),
        Op::Log => f::math_log(x(1)) / f::math_log(x(0)),
        Op::Factorial => f::math_factorial(x(0)),
        Op::Sin => f::math_sin(x(0)),
        Op::Cos => f::math_cos(x(0)),
        Op::Tan => f::math_tan(x(0)),
        Op::Sec => 1.0 / f::math_cos(x(0)),
        Op::Csc => 1.0 / f::math_sin(x(0)),
        Op::Cot => 1.0 / f::math_tan(x(0)),
        Op::Sinh => f::math_sinh(x(0)),
        Op::Cosh => f::math_cosh(x(0)),
        Op::Tanh => f::math_tanh(x(0)),
        Op::Sech => 1.0 / f::math_cosh(x(0)),
        Op::Csch => 1.0 / f::math_sinh(x(0)),
        Op::Coth => {
            let v = x(0);
            f::math_cosh(v) / f::math_sinh(v)
        }
        Op::Arcsin => f::math_asin(x(0)),
        Op::Arccos => f::math_acos(x(0)),
        Op::Arctan => f::math_atan(x(0)),
        Op::Arcsec => f::math_acos(1.0 / x(0)),
        Op::Arccsc => f::math_asin(1.0 / x(0)),
        Op::Arccot => {
            let v = x(0);
            if v == 0.0 {
                HALF_PI
            } else {
                f::math_atan(1.0 / v)
            }
        }
        Op::Arcsinh => {
            let v = x(0);
            f::math_log(v + (v * v + 1.0).sqrt())
        }
        Op::Arccosh => {
            let v = x(0);
            f::math_log(v + (v * v - 1.0).sqrt())
        }
        Op::Arctanh => {
            let v = x(0);
            (f::math_log(1.0 + v) - f::math_log(1.0 - v)) * 0.5
        }
        Op::Arcsech => {
            let r = 1.0 / x(0);
            f::math_log(r + (r - 1.0).sqrt() * (r + 1.0).sqrt())
        }
        Op::Arccsch => {
            let v = x(0);
            f::math_log(1.0 / v + (1.0 / (v * v) + 1.0).sqrt())
        }
        Op::Arccoth => {
            let r = 1.0 / x(0);
            (f::math_log(1.0 + r) - f::math_log(1.0 - r)) * 0.5
        }
        Op::Max | Op::Min => match args.split_first() {
            None if op == Op::Max => f64::NEG_INFINITY,
            None => f64::INFINITY,
            Some((first, rest)) => rest.iter().fold(eval(first, read), |a, e| {
                let b = eval(e, read);
                let pick_a = if op == Op::Max { a > b } else { a < b };
                if pick_a { a } else { b }
            }),
        },
        Op::Rem => f::math_fmod(x(0), x(1)),
        Op::Quotient => quotient(x(0), x(1)),
        _ => unreachable!("{op:?} is boolean"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> Expr {
        Expr::Number(x)
    }

    fn ap(op: Op, args: Vec<Expr>) -> Expr {
        Expr::apply(op, args)
    }

    fn value(e: &Expr) -> f64 {
        eval(e, &|_| unreachable!())
    }

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn min_and_max_as_libroadrunner() {
        let nan = f64::NAN;
        assert_eq!(value(&ap(Op::Max, vec![n(nan), n(1.0)])), 1.0);
        assert!(value(&ap(Op::Max, vec![n(1.0), n(nan)])).is_nan());
        assert_eq!(bits(value(&ap(Op::Max, vec![n(0.0), n(-0.0)]))), bits(-0.0));
        assert_eq!(bits(value(&ap(Op::Min, vec![n(0.0), n(-0.0)]))), bits(-0.0));
        assert_eq!(bits(value(&ap(Op::Min, vec![n(-0.0), n(0.0)]))), bits(0.0));
        assert_eq!(value(&ap(Op::Max, vec![n(7.0), n(nan), n(1.0)])), 1.0);
        assert_eq!(value(&ap(Op::Max, vec![])), f64::NEG_INFINITY);
        assert_eq!(value(&ap(Op::Min, vec![])), f64::INFINITY);
    }

    #[test]
    fn rem_and_quotient_as_libroadrunner() {
        assert_eq!(value(&ap(Op::Rem, vec![n(-7.0), n(3.0)])), -1.0);
        assert!(value(&ap(Op::Rem, vec![n(7.0), n(0.0)])).is_nan());
        assert_eq!(value(&ap(Op::Quotient, vec![n(-7.0), n(3.0)])), -2.0);
        assert_eq!(value(&ap(Op::Quotient, vec![n(7.5), n(2.0)])), 3.0);
        assert_eq!(bits(value(&ap(Op::Quotient, vec![n(-0.0), n(3.0)]))), 0);
        for (a, b) in [(7.0, 0.0), (3e9, 1.0), (-3e9, 1.0), (f64::NAN, 1.0)] {
            assert_eq!(value(&ap(Op::Quotient, vec![n(a), n(b)])), -2147483648.0);
        }
    }

    #[test]
    fn relations_are_unordered() {
        let nan = f64::NAN;
        for op in [Op::Eq, Op::Neq, Op::Lt, Op::Gt, Op::Leq, Op::Geq] {
            assert_eq!(value(&ap(op, vec![n(nan), n(1.0)])), 1.0, "{op:?}");
        }
        assert_eq!(value(&ap(Op::Eq, vec![n(1.0), n(1.0), n(2.0)])), 0.0);
        assert_eq!(value(&ap(Op::Lt, vec![n(0.0), n(1.0), n(2.0)])), 1.0);
        assert_eq!(value(&ap(Op::Lt, vec![n(0.0), n(2.0), n(1.0)])), 0.0);
    }

    #[test]
    fn booleans_and_numbers() {
        let pw = |c: Expr| Expr::Piecewise {
            pieces: vec![(n(1.0), c)],
            otherwise: Some(Box::new(n(2.0))),
        };
        assert_eq!(value(&pw(n(2.0))), 1.0);
        assert_eq!(value(&pw(n(f64::NAN))), 2.0);
        assert_eq!(value(&pw(n(-0.0))), 2.0);
        assert_eq!(value(&ap(Op::Not, vec![n(f64::NAN)])), 1.0);
        assert_eq!(value(&ap(Op::And, vec![])), 1.0);
        assert_eq!(value(&ap(Op::Or, vec![])), 0.0);
        let t = Expr::Bool(true);
        assert_eq!(value(&ap(Op::Xor, vec![t.clone(), t.clone(), t])), 1.0);
        let none = Expr::Piecewise {
            pieces: vec![(n(1.0), Expr::Bool(false))],
            otherwise: None,
        };
        assert!(value(&none).is_nan());
    }

    #[test]
    fn functions_as_libroadrunner() {
        assert_eq!(bits(value(&ap(Op::Minus, vec![n(0.0)]))), 0);
        assert_eq!(
            value(&ap(Op::Log, vec![n(10.0), n(1000.0)])),
            2.9999999999999996
        );
        assert_eq!(value(&ap(Op::Log, vec![n(1000.0)])), 2.9999999999999996);
        assert_eq!(
            value(&ap(Op::Root, vec![n(f64::NEG_INFINITY)])),
            f64::INFINITY
        );
        assert_eq!(bits(value(&ap(Op::Root, vec![n(-0.0)]))), 0);
        assert_eq!(value(&ap(Op::Arccot, vec![n(-0.0)])), HALF_PI);
        // probed: arcsinh(0.13), arctanh(0.41), arcsech(0.66), arccsch(-0.47), arccoth(2.7)
        // as libroadrunner computes them, not asinh, atanh, acosh of libm
        let probe = |op, x: f64| value(&ap(op, vec![n(x)]));
        assert_eq!(probe(Op::Arccoth, 1.7), 0.674963358474508);
        assert_eq!(probe(Op::Arccoth, 3.1), 0.33452481449044236);
        assert_eq!(probe(Op::Arccsch, -0.3), -1.9188964720985329);
        assert_eq!(probe(Op::Arcsech, 0.7), 0.8955880995299759);
        assert_eq!(probe(Op::Coth, 0.7), 1.6546216358026298);
        assert_eq!(probe(Op::Cot, 0.7), 1.1872418321266793);
    }
}

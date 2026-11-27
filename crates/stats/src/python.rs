//! The arithmetic of python on floats where it differs from the one of Rust: the text of a
//! double (`repr`), the compensated sum of `sum`, `max` and `min` of not a number and the
//! errors of a power and of an exponential past the largest double and of a division by zero.

use crate::{Result, StatsError};

/// The text python gives a double, `repr`: the shortest digits which read back as it, in
/// decimals from 1e-4 to below 1e16 and with an exponent of a sign and two digits at least
/// outside (`1e-05`, `1e+16`), `nan`, `inf` and `-inf`.
pub(crate) fn python_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    // Debug: the shortest digits, in decimals in the same range as python
    let text = format!("{x:?}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let exponent: i32 = exponent.parse().expect("an exponent is an integer");
            let sign = if exponent < 0 { '-' } else { '+' };
            format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
        }
        None => text,
    }
}

/// The message of the OverflowError of a power of python past the largest double
/// (`OverflowError(34, 'Numerical result out of range')`, of the errno `ERANGE`).
pub(crate) const POWER_OVERFLOW: &str = "(34, 'Numerical result out of range')";

/// The message of the OverflowError of a function of `math` past the largest double.
pub(crate) const MATH_RANGE: &str = "math range error";

/// The message of the ZeroDivisionError of a division of floats by zero (python 3.14).
pub(crate) const DIVISION_BY_ZERO: &str = "division by zero";

/// The sum of floats of python, `sum` of an iterable of floats from the start 0: the first
/// value, then the improved Kahan-Babuska sum of Neumaier (python 3.12 and later), the
/// compensation added at the end where it is finite and not zero, so an infinite or
/// overflowed sum is not turned into a not a number.
pub(crate) fn python_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return 0.0;
    };
    let (mut sum, mut compensation) = (0.0 + first, 0.0);
    for x in values {
        let t = sum + x;
        if sum.abs() >= x.abs() {
            compensation += (sum - t) + x;
        } else {
            compensation += (x - t) + sum;
        }
        sum = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        sum += compensation;
    }
    sum
}

/// The larger of two floats as python's `max(a, b)`: `b` only where `b > a`, so a not a number
/// `a` is kept.
pub(crate) fn python_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// The smaller of two floats as python's `min(a, b)`: `b` only where `b < a`.
pub(crate) fn python_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// The largest of floats as python's `max` of a list: the first, replaced by every later one
/// which is larger.
pub(crate) fn python_max_of(values: &[f64]) -> f64 {
    values[1..].iter().fold(values[0], |m, &x| python_max(m, x))
}

/// The power `x ** n` of a float and an integer exponent as python computes it, `pow` of the
/// C library: an error with the message of the OverflowError python raises where a finite
/// `x` gives an infinite power.
pub(crate) fn python_pow(x: f64, n: i32) -> Result<f64> {
    let power = libm::pow(x, f64::from(n));
    if x.is_finite() && power.is_infinite() {
        return Err(StatsError(POWER_OVERFLOW.to_owned()));
    }
    Ok(power)
}

/// `math.exp`: the error of the OverflowError of python past the largest double.
pub(crate) fn math_exp(x: f64) -> Result<f64> {
    checked_range(x, libm::exp(x))
}

/// `math.expm1`: the error of the OverflowError of python past the largest double.
pub(crate) fn math_expm1(x: f64) -> Result<f64> {
    checked_range(x, libm::expm1(x))
}

/// The value of a function of `math` of a finite argument, an error where it is infinite.
fn checked_range(x: f64, value: f64) -> Result<f64> {
    if x.is_finite() && value.is_infinite() {
        return Err(StatsError(MATH_RANGE.to_owned()));
    }
    Ok(value)
}

/// The quotient of floats as python divides them: the error of the ZeroDivisionError by zero.
pub(crate) fn python_div(a: f64, b: f64) -> Result<f64> {
    if b == 0.0 {
        return Err(StatsError(DIVISION_BY_ZERO.to_owned()));
    }
    Ok(a / b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_is_written_as_python_writes_it() {
        for (x, text) in [
            (-1.0, "-1.0"),
            (-0.0, "-0.0"),
            (0.5, "0.5"),
            (1e-4, "0.0001"),
            (-1e-5, "-1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e16, "1e+16"),
            (9_999_999_999_999_998.0, "9999999999999998.0"),
            (123_456_789_012_345.67, "123456789012345.67"),
            (-1e300, "-1e+300"),
            (5e-324, "5e-324"),
            (f64::NAN, "nan"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            assert_eq!(python_repr(x), text);
        }
    }

    #[test]
    fn the_sum_of_python_is_compensated() {
        assert_eq!(python_sum([]), 0.0);
        assert_eq!(python_sum([-0.0]), 0.0);
        assert_eq!(python_sum([1e16, 1.0, -1e16]), 1.0);
        assert_eq!(python_sum([0.1; 10]), 1.0);
        assert_eq!(python_sum([f64::INFINITY, 1.0]), f64::INFINITY);
        assert!(python_sum([f64::NAN, 1.0]).is_nan());
    }

    #[test]
    fn max_and_min_keep_a_first_not_a_number_as_python() {
        assert!(python_max(f64::NAN, 0.0).is_nan());
        assert_eq!(python_max(0.0, f64::NAN), 0.0);
        assert!(python_min(f64::NAN, 1.0).is_nan());
        assert_eq!(python_max(-0.0, 0.0).to_bits(), (-0.0_f64).to_bits());
        assert_eq!(python_max_of(&[1.0, 3.0, 2.0]), 3.0);
        assert!(python_max_of(&[f64::NAN, 3.0]).is_nan());
    }

    #[test]
    fn a_power_an_exponential_and_a_division_refuse_as_python() {
        assert_eq!(python_pow(3.0, 2), Ok(9.0));
        assert_eq!(python_pow(f64::INFINITY, 4), Ok(f64::INFINITY));
        assert_eq!(python_pow(1e100, 4).unwrap_err().0, POWER_OVERFLOW);
        assert_eq!(math_exp(f64::INFINITY), Ok(f64::INFINITY));
        assert_eq!(math_exp(1000.0).unwrap_err().0, MATH_RANGE);
        assert_eq!(math_expm1(1000.0).unwrap_err().0, MATH_RANGE);
        assert_eq!(python_div(1.0, 0.0).unwrap_err().0, DIVISION_BY_ZERO);
    }
}

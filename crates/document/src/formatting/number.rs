//! A double written the way python writes it.

/// The `str()` of python 3 for a float, which the readable equation of a reaction is written with.
///
/// Python writes the coefficient of a half equation as an f-string of the float
/// (`SBMLDocumentInfo._half_equation`), which is its `str()`, and that is `format_float_short` of CPython in
/// mode `r`: the shortest digits which round trip, in the fixed form while the decimal point sits in
/// `-4 < decpt <= 16` and in the exponent form outside of those two bounds, the exponent signed and padded
/// to two digits, and a trailing `.0` where the fixed form leaves no fractional digit. The upper bound is 16
/// and not the 17 of python 3.0 and before, so `1e16` is already written as an exponent.
///
/// The shared JSON float formatter supplies shortest digits with round-to-even
/// tie breaking. Only presentation is changed here: Python's fixed/exponent
/// threshold, signed two-digit exponent, and trailing `.0` for whole floats.
pub fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value == f64::INFINITY {
        return "inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }
    // `{:e}` of a finite double always writes `<mantissa>e<exponent>`; the two fallbacks take anything
    // else as an exponent of zero rather than panicking on it.
    let json = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
    let (decimal, explicit_exponent) = json.split_once('e').unwrap_or((&json, "0"));
    let explicit_exponent = explicit_exponent.parse::<i32>().unwrap_or(0);
    let negative = decimal.starts_with('-');
    let decimal = decimal.trim_start_matches('-');
    let integer_digits = decimal.find('.').unwrap_or(decimal.len());
    let digits = decimal.replace('.', "");
    let leading = digits.bytes().take_while(|b| *b == b'0').count();
    let significant = digits[leading..].trim_end_matches('0');
    let significant = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    let exponent = if significant == "0" {
        0
    } else {
        integer_digits as i32 - leading as i32 - 1 + explicit_exponent
    };
    let sign = if negative { "-" } else { "" };
    let mantissa = if significant.len() == 1 {
        format!("{sign}{significant}")
    } else {
        format!("{sign}{}.{}", &significant[..1], &significant[1..])
    };
    // The place of the decimal point within the digits, the `decpt` of CPython. `{:e}` has normalized the
    // mantissa to one digit before the point, so the exponent form is that mantissa with a written exponent.
    let point = exponent + 1;
    if point <= -4 || point > 16 {
        return format!("{mantissa}e{exponent:+03}");
    }
    let (sign, magnitude) = match mantissa.strip_prefix('-') {
        Some(magnitude) => ("-", magnitude),
        None => ("", mantissa.as_str()),
    };
    let digits: String = magnitude.chars().filter(char::is_ascii_digit).collect();
    if point <= 0 {
        let zeros = "0".repeat(point.unsigned_abs() as usize);
        return format!("{sign}0.{zeros}{digits}");
    }
    let point = point.unsigned_abs() as usize;
    if point >= digits.len() {
        let zeros = "0".repeat(point - digits.len());
        format!("{sign}{digits}{zeros}.0")
    } else {
        let (integer, fraction) = digits.split_at_checked(point).unwrap_or((&digits, ""));
        format!("{sign}{integer}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_float_is_written_as_python_writes_it() {
        // generated from python 3.14: `for v in ...: print(repr(v), str(v))`
        for (value, text) in [
            (1.0, "1.0"),
            (2.0, "2.0"),
            (2.5, "2.5"),
            (0.5, "0.5"),
            (-1.0, "-1.0"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (0.1, "0.1"),
            (0.0001, "0.0001"),
            (1e-5, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1.0 / 3.0, "0.3333333333333333"),
            (12345.6789, "12345.6789"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1e21, "1e+21"),
            (1e100, "1e+100"),
            (123456789012345680.0, "1.2345678901234568e+17"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            assert_eq!(python_float(value), text, "{value:?}");
        }
        assert_eq!(python_float(f64::NAN), "nan");
    }

    #[test]
    fn a_double_between_two_shortest_representations_rounds_to_even() {
        assert_eq!(python_float(1333333333333333.3), "1333333333333333.2");
    }
}

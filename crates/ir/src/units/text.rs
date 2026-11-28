//! The unit expressions of sbmlsim, `Units.udef_to_str` and `Units.quotient` of
//! `sbmlsim.units`, with the formats of the numbers of python they write.

use super::algebra::{Definition, KINDS, Unit, pow, reorder};

/// The symbol of pint of a unit kind, `Units._symbols`, the kind itself for the others.
fn symbol(kind: usize) -> &'static str {
    let name = KINDS.get(kind).copied().unwrap_or("(Invalid UnitKind)");
    match name {
        "kilogram" => "kg",
        "gram" => "g",
        "metre" | "meter" => "m",
        "second" => "s",
        "mole" => "mol",
        "litre" | "liter" => "l",
        "katal" => "kat",
        "dimensionless" => "",
        other => other,
    }
}

/// The SI prefix of a scale, `Units._prefixes`.
fn prefix(scale: i64) -> Option<&'static str> {
    Some(match scale {
        9 => "G",
        6 => "M",
        3 => "k",
        2 => "h",
        1 => "da",
        -1 => "d",
        -2 => "c",
        -3 => "m",
        -6 => "u",
        -9 => "n",
        -12 => "p",
        -15 => "f",
        _ => return None,
    })
}

/// The name of a multiple of a second, `Units._seconds`.
fn seconds(value: f64) -> Option<&'static str> {
    Some(match value {
        60.0 => "min",
        3600.0 => "hr",
        86400.0 => "day",
        604_800.0 => "week",
        _ => return None,
    })
}

/// `numpy.isclose` of the default tolerances unless given.
pub(crate) fn isclose(a: f64, b: f64, rtol: f64, atol: f64) -> bool {
    (a - b).abs() <= atol + rtol * b.abs()
}

/// `round(x, digits)` of python: the double nearest to `x` rounded to `digits` decimals.
fn round_to(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}

/// The decimal digits of the shortest representation of a finite positive double and the
/// position of its decimal point (`0.d1d2... * 10^point`).
fn shortest_digits(x: f64) -> (String, i32) {
    // `{:e}` writes the shortest digits which read back as the same double
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e').expect("an exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    (digits, exponent + 1)
}

/// `repr` of a python float.
pub(crate) fn repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let (digits, point) = shortest_digits(x.abs());
    let n = digits.len() as i32;
    let body = if -4 < point && point <= 16 {
        if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point >= n {
            format!("{digits}{}.0", "0".repeat((point - n) as usize))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        }
    } else {
        let mantissa = if n == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        let e = point - 1;
        format!("{mantissa}e{}{:02}", if e < 0 { "-" } else { "+" }, e.abs())
    };
    format!("{sign}{body}")
}

/// `format(x, "g")` of python, six significant digits.
pub(crate) fn general(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let text = format!("{x:.5e}");
    let (mantissa, exponent) = text.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let strip = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if (-4..6).contains(&exponent) {
        let decimals = (5 - exponent) as usize;
        strip(format!("{x:.decimals$}"))
    } else {
        let e = if exponent < 0 { "-" } else { "+" };
        format!("{}e{e}{:02}", strip(mantissa.to_string()), exponent.abs())
    }
}

/// `Units._unit_to_str`: a unit without the sign of its exponent.
fn unit_to_str(u: &Unit) -> String {
    let symbol = symbol(u.kind);
    let mut multiplier = u.multiplier;
    let mut scale = i64::from(u.scale);
    let exponent = u.exponent.abs();
    let close = |m: f64| isclose(m, 1.0, 1e-5, 1e-8);
    if symbol.is_empty() && close(multiplier) && scale == 0 {
        return String::new();
    }
    // a multiplier which is a power of ten is a scale, e.g. 0.001 * mole
    let value = multiplier * pow(10.0, scale as f64);
    let power = if value > 0.0 {
        libm::log10(value)
    } else {
        f64::NAN
    };
    if power.is_finite() && isclose(power, power.round_ties_even(), 1e-5, 1e-9) {
        scale = power.round_ties_even() as i64;
        multiplier = 1.0;
    }
    let mut factor = String::new();
    let second = KINDS.get(u.kind) == Some(&"second");
    let multiple = if second {
        seconds(round_to(multiplier * pow(10.0, scale as f64), 9))
    } else {
        None
    };
    let name = if let Some(name) = multiple {
        name.to_string()
    } else {
        let kilogram = KINDS.get(u.kind) == Some(&"kilogram");
        let name = if scale == 0 {
            symbol.to_string()
        } else if let (Some(p), false, false) = (prefix(scale), kilogram, symbol.is_empty()) {
            format!("{p}{symbol}")
        } else if let (true, Some(p)) = (kilogram, prefix(scale + 3)) {
            format!("{p}g")
        } else if symbol.is_empty() {
            format!("10^{scale}")
        } else {
            format!("10^{scale}*{symbol}")
        };
        if !close(multiplier) {
            factor = format!("{}*", repr(multiplier));
        }
        name
    };
    let term = format!("{factor}{name}");
    let compound = term.contains('*');
    if isclose(exponent, 1.0, 1e-5, 1e-8) {
        return if compound { format!("({term})") } else { term };
    }
    let exponent = general(exponent);
    if compound {
        format!("({term})^{exponent}")
    } else {
        format!("{term}^{exponent}")
    }
}

/// `Units.udef_to_str`: a unit definition as a unit expression of pint, e.g. `mmol/min`.
pub(crate) fn udef_to_str(ud: &Definition) -> String {
    let mut ud = ud.clone();
    reorder(&mut ud);
    let mut nom = Vec::new();
    let mut denom = Vec::new();
    for u in &ud {
        let term = unit_to_str(u);
        if term.is_empty() {
            continue;
        }
        if u.exponent >= 0.0 {
            nom.push(term);
        } else {
            denom.push(term);
        }
    }
    let nom_str = nom.join(" * ");
    let mut denom_str = denom.join(" * ");
    if denom.len() > 1 {
        denom_str = format!("({denom_str})");
    }
    match (nom_str.is_empty(), denom_str.is_empty()) {
        (false, false) => format!("{nom_str}/{denom_str}"),
        (false, true) => nom_str,
        (true, false) => format!("1/{denom_str}"),
        (true, true) => String::new(),
    }
}

/// `_enclosed`: one pair of brackets around the whole expression.
fn enclosed(expression: &str) -> bool {
    if !(expression.starts_with('(') && expression.ends_with(')')) {
        return false;
    }
    let mut depth = 0;
    let n = expression.chars().count();
    for (k, c) in expression.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && k < n - 1 {
            return false;
        }
    }
    true
}

/// `Units.quotient`: the quotient of two unit expressions with the brackets it needs.
pub(crate) fn quotient(numerator: &str, denominator: &str) -> String {
    let group = |e: &str| {
        let simple = e.chars().all(|c| c.is_alphanumeric() || "_^.".contains(c));
        if simple || enclosed(e) {
            e.to_string()
        } else {
            format!("({e})")
        }
    };
    if denominator.trim().is_empty() {
        return numerator.to_string();
    }
    if numerator.trim().is_empty() {
        return format!("1/{}", group(denominator));
    }
    format!("{}/{}", group(numerator), group(denominator))
}

/// Whether a unit expression of [`udef_to_str`] is a unit of pint, which `parse_units` reads:
/// it has no number but 1 outside of an exponent, and no `avogadro`, which pint does not have.
pub(crate) fn is_pint_unit(expression: &str) -> bool {
    expression
        .split(|c: char| " */()".contains(c))
        .filter(|t| !t.is_empty())
        .all(|token| {
            let base = token.split('^').next().unwrap_or("");
            let numeric = base.starts_with(|c: char| c.is_ascii_digit() || c == '.');
            !(numeric && base != "1") && !base.ends_with("avogadro")
        })
}

#[cfg(test)]
mod tests {
    use super::super::algebra::kind_of;
    use super::*;

    fn u(kind: &str, exponent: f64, scale: i32, multiplier: f64) -> Unit {
        Unit {
            kind: kind_of(kind),
            exponent,
            scale,
            multiplier,
            checking: false,
        }
    }

    #[test]
    fn floats_are_written_as_python_writes_them() {
        assert_eq!(repr(60.0), "60.0");
        assert_eq!(repr(0.016_666_666_666_666_7), "0.0166666666666667");
        assert_eq!(repr(1.666_666_666_666_67e-5), "1.66666666666667e-05");
        assert_eq!(repr(1e16), "1e+16");
        assert_eq!(repr(1234.5), "1234.5");
        assert_eq!(repr(0.0001), "0.0001");
        assert_eq!(general(0.5), "0.5");
        assert_eq!(general(1.0 / 3.0), "0.333333");
        assert_eq!(general(2.0), "2");
        assert_eq!(general(1e-5), "1e-05");
        assert_eq!(general(1234567.0), "1.23457e+06");
    }

    #[test]
    fn unit_definitions_are_written_as_sbmlsim_writes_them() {
        let mmol_per_min = vec![u("second", -1.0, 0, 60.0), u("mole", 1.0, -3, 1.0)];
        assert_eq!(udef_to_str(&mmol_per_min), "mmol/min");
        assert_eq!(
            udef_to_str(&vec![u("kilogram", 1.0, -3, 1.0)]),
            "(10^-3*kg)"
        );
        assert_eq!(udef_to_str(&vec![u("second", 1.0, 0, 2.5)]), "(2.5*s)");
        assert_eq!(udef_to_str(&vec![u("metre", 0.5, 0, 1.0)]), "m^0.5");
        assert_eq!(udef_to_str(&vec![u("dimensionless", 1.0, 0, 1.0)]), "");
        assert_eq!(quotient("mmol", "litre"), "mmol/litre");
        assert_eq!(quotient("mmol * s", "l"), "(mmol * s)/l");
        assert!(is_pint_unit("mmol^0.5/l^0.5") && is_pint_unit("1/min"));
        assert!(!is_pint_unit("(2.5*s)") && !is_pint_unit("10^-3*kg"));
    }
}

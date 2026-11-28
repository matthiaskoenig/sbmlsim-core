//! Numbers and labels in the words of a message, as python writes them, so a message of the
//! planner reads as the one of sbmlsim.

use serde_json::Value as Json;

/// A number as python's `repr` writes a float: the shortest digits which read back as the
/// number, `5.0` and not `5`, positional from `1e-4` to below `1e16` and scientific with a
/// signed exponent of at least two digits outside (`1e-05`, `1e+16`), `nan`, `inf`, `-inf`.
pub(crate) fn number(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let scientific = format!("{:e}", x.abs());
    let (mantissa, exponent) = scientific.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if x < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let text = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            format!("{whole}.{fraction}")
        };
        format!("{sign}{text}")
    } else {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!("{sign}{mantissa}e{exponent_sign}{:02}", exponent.abs())
    }
}

/// A label of a dimension as python writes the element of the array of the labels: a number
/// as numpy writes it, a text as it is.
pub(crate) fn label(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        Json::Number(n) if n.is_f64() => number(n.as_f64().unwrap_or(f64::NAN)),
        Json::Number(n) => n.to_string(),
        Json::Bool(true) => "True".into(),
        Json::Bool(false) => "False".into(),
        Json::Null => "None".into(),
        other => other.to_string(),
    }
}

/// A text as python's `repr` writes it: in single quotes, in double quotes when it has a
/// single quote and no double quote, the backslash, the quote and the control characters
/// escaped; every other character, non-ASCII ones among them, as it is (python escapes other
/// characters it does not print as well, e.g. `\u200b`, which no name of a model holds).
pub(crate) fn repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            // the control characters of ASCII and of Latin-1, which python writes as `\xNN`
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// A list of texts as python's `repr` writes it, `['a', 'b']`.
pub(crate) fn list<S: AsRef<str>>(items: &[S]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| repr(s.as_ref())).collect();
    format!("[{}]", quoted.join(", "))
}

/// A label as python's `repr` writes the element of the list of the labels: a text quoted, a
/// number as `label` writes it.
pub(crate) fn label_repr(value: &Json) -> String {
    match value {
        Json::String(s) => repr(s),
        other => label(other),
    }
}

/// The labels of a dimension as python's `repr` writes their list, `[0, 1]` or `['a', 'b']`.
pub(crate) fn labels(values: &[Json]) -> String {
    let items: Vec<String> = values.iter().map(label_repr).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::{label_repr, labels, list, number, repr};

    #[test]
    fn texts_and_labels_as_python_reprs_them() {
        // the expected texts are the output of python 3.14's `repr`
        for (text, python) in [
            ("fast", "'fast'"),
            ("it's", "\"it's\""),
            ("say \"hi\"", "'say \"hi\"'"),
            ("it's \"x\"", "'it\\'s \"x\"'"),
            ("Größe µM", "'Größe µM'"),
            ("Kₘ ✓ 日本", "'Kₘ ✓ 日本'"),
            ("tab\tnew\nline", "'tab\\tnew\\nline'"),
            ("back\\slash", "'back\\\\slash'"),
            ("\u{7f}\u{85}", "'\\x7f\\x85'"),
        ] {
            assert_eq!(repr(text), python, "{text:?}");
        }
        let json = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(label_repr(&json("\"it's\"")), "\"it's\"");
        assert_eq!(label_repr(&json("1")), "1");
        assert_eq!(label_repr(&json("0.5")), "0.5");
        assert_eq!(
            labels(&[json("\"a\""), json("\"b'c\""), json("2.0")]),
            "['a', \"b'c\", 2.0]"
        );
    }

    #[test]
    fn lists_as_python_writes_them() {
        assert_eq!(list(&["a", "b"]), "['a', 'b']");
        assert_eq!(list::<&str>(&[]), "[]");
        assert_eq!(list(&["it's"]), "[\"it's\"]");
        assert_eq!(list(&["a'b\"c"]), "['a\\'b\"c']");
        assert_eq!(list(&["a\\b\n"]), "['a\\\\b\\n']");
    }

    #[test]
    fn numbers_as_python_writes_them() {
        for (x, text) in [
            (5.0, "5.0"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (1e-4, "0.0001"),
            (1e-5, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (0.4999999890469133, "0.4999999890469133"),
            (1e100, "1e+100"),
            (f64::INFINITY, "inf"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(number(x), text, "{x}");
        }
    }
}

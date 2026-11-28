//! What the oracle tests of every module share: the fixtures of `oracles/<module>.json`, the
//! cases with their inputs and expected values, the tolerances, the comparisons and the hash
//! of the bits of the values found.
//!
//! A fixture holds its cases by name, every case its inputs and the expected values, the
//! numbers as the doubles they were and the numbers which JSON cannot hold as the strings
//! "nan", "inf" and "-inf". A test runs a function over the inputs of a case and holds every
//! value to the expected one at a relative tolerance; it prints the largest relative error it
//! found (`cargo test -p sbmlsim-stats -- --nocapture`).

// every test binary of a module uses a part of these
#![allow(dead_code)]

use serde_json::Value;

/// The fixture of a module, `oracles/<module>.json`.
pub struct Fixture {
    pub module: String,
    pub value: Value,
}

/// The text of the fixture of a module, compiled in.
pub fn source(module: &str) -> &'static str {
    match module {
        "special" => include_str!("../../oracles/special.json"),
        "linalg" => include_str!("../../oracles/linalg.json"),
        "metrics" => include_str!("../../oracles/metrics.json"),
        "fisher" => include_str!("../../oracles/fisher.json"),
        "fisher_decomposition" => include_str!("../../oracles/fisher_decomposition.json"),
        "uncertainty" => include_str!("../../oracles/uncertainty.json"),
        "draws" => include_str!("../../oracles/draws.json"),
        "testing" => include_str!("../../oracles/testing.json"),
        _ => panic!("no fixture of the module {module}"),
    }
}

impl Fixture {
    pub fn load(module: &str) -> Self {
        let value = serde_json::from_str(source(module))
            .unwrap_or_else(|e| panic!("oracles/{module}.json: {e}"));
        Self {
            module: module.to_string(),
            value,
        }
    }

    /// The names of the cases which start with `prefix`, sorted, so the order does not
    /// depend on whether a crate of the build turns on `preserve_order` of serde_json.
    pub fn cases(&self, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .value
            .as_object()
            .unwrap()
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        names.sort();
        assert!(
            !names.is_empty(),
            "{}.json has no case {prefix}*",
            self.module
        );
        names
    }

    /// The case `name` of the fixture.
    pub fn case(&self, name: &str) -> Case<'_> {
        let value = self
            .value
            .get(name)
            .unwrap_or_else(|| panic!("{}.json has no case {name}", self.module));
        Case {
            name: name.to_string(),
            value,
        }
    }
}

/// A case of a fixture: its inputs and the expected values.
pub struct Case<'a> {
    pub name: String,
    pub value: &'a Value,
}

impl Case<'_> {
    /// The input `name`, a list of numbers.
    pub fn input(&self, name: &str) -> Vec<f64> {
        let value = self.value["inputs"]
            .get(name)
            .unwrap_or_else(|| panic!("the case {} has no input {name}", self.name));
        numbers(value)
    }

    /// The input `name`, a list of non-negative integers.
    pub fn input_u64(&self, name: &str) -> Vec<u64> {
        self.value["inputs"][name]
            .as_array()
            .unwrap_or_else(|| panic!("the input {name} of {} is not a list", self.name))
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .unwrap_or_else(|| panic!("{value} of {} is no integer", self.name))
            })
            .collect()
    }

    /// The input `name`, a number.
    pub fn input_f64(&self, name: &str) -> f64 {
        number(&self.value["inputs"][name])
    }

    /// The input `name`, a list of integers.
    pub fn input_i64(&self, name: &str) -> Vec<i64> {
        self.value["inputs"][name]
            .as_array()
            .unwrap_or_else(|| panic!("the input {name} of {} is not a list", self.name))
            .iter()
            .map(|value| {
                value
                    .as_i64()
                    .unwrap_or_else(|| panic!("{value} of {} is no integer", self.name))
            })
            .collect()
    }

    /// The input `name`, a non-negative integer.
    pub fn input_usize(&self, name: &str) -> usize {
        let value = &self.value["inputs"][name];
        let integer = value
            .as_u64()
            .unwrap_or_else(|| panic!("the input {name} of {} is no integer", self.name));
        usize::try_from(integer).unwrap()
    }

    /// The input `name`, a boolean.
    pub fn input_bool(&self, name: &str) -> bool {
        self.value["inputs"][name]
            .as_bool()
            .unwrap_or_else(|| panic!("the input {name} of {} is no boolean", self.name))
    }

    /// The input `name`, a list of texts.
    pub fn input_strings(&self, name: &str) -> Vec<String> {
        self.value["inputs"][name]
            .as_array()
            .unwrap_or_else(|| panic!("the input {name} of {} is not a list", self.name))
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect()
    }

    /// The input `name`, a text.
    pub fn input_str(&self, name: &str) -> &str {
        self.value["inputs"][name]
            .as_str()
            .unwrap_or_else(|| panic!("the input {name} of {} is no text", self.name))
    }

    /// The input `name`, a number or `None` of null.
    pub fn input_option(&self, name: &str) -> Option<f64> {
        match &self.value["inputs"][name] {
            Value::Null => None,
            value => Some(number(value)),
        }
    }

    /// The expected output `name`, a number.
    pub fn expected_f64(&self, name: &str) -> f64 {
        number(&self.value["expected"][name])
    }

    /// The message of the ValueError the python raised, `None` for a case with values.
    pub fn error(&self) -> Option<&str> {
        self.value["expected"]
            .get("error")
            .map(|e| e.as_str().unwrap())
    }

    /// The expected values, a list of numbers.
    pub fn expected(&self) -> Vec<f64> {
        numbers(&self.value["expected"])
    }

    /// The expected output `name` of a case with several: `None` for an output the case does
    /// not have, `Some(None)` for a null one and `Some(Some(values))` for a list of numbers.
    pub fn output(&self, name: &str) -> Option<Option<Vec<f64>>> {
        match self.value["expected"].get(name)? {
            Value::Null => Some(None),
            value => Some(Some(numbers(value))),
        }
    }

    /// The expected output `name`, which the case must have.
    pub fn required(&self, name: &str) -> Vec<f64> {
        self.output(name)
            .flatten()
            .unwrap_or_else(|| panic!("the case {} has no output {name}", self.name))
    }
}

/// A number of a fixture: a JSON number or one of the strings "nan", "inf" and "-inf".
pub fn number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap(),
        Value::String(text) => match text.as_str() {
            "nan" => f64::NAN,
            "inf" => f64::INFINITY,
            "-inf" => f64::NEG_INFINITY,
            _ => panic!("{text} is no number"),
        },
        _ => panic!("{value} is no number"),
    }
}

/// A list of numbers of a fixture.
pub fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is not a list"))
        .iter()
        .map(number)
        .collect()
}

/// The relative error of `found` against `expected`: zero for the same number (two not a
/// numbers, two equal infinities), infinite for a not a number or an infinity against another
/// value. Two values below the smallest normal double (zero or subnormal) are the same: a
/// subnormal double has fewer digits than the tolerances ask for, and scipy flushes some of
/// them to zero (the normal distribution function at -38).
pub fn relative_error(found: f64, expected: f64) -> f64 {
    if found == expected || (found.is_nan() && expected.is_nan()) {
        return 0.0;
    }
    if !found.is_finite() || !expected.is_finite() {
        return f64::INFINITY;
    }
    if found.abs() < f64::MIN_POSITIVE && expected.abs() < f64::MIN_POSITIVE {
        return 0.0;
    }
    (found - expected).abs() / expected.abs().max(f64::MIN_POSITIVE)
}

/// Holds every value found to the expected one at the relative tolerance `tolerance`, prints
/// the largest relative error and returns it; `rows` are the inputs written out, the value
/// found and the value expected.
pub fn assert_close(name: &str, tolerance: f64, rows: Vec<(String, f64, f64)>) -> f64 {
    let mut worst = 0.0_f64;
    let mut failures = Vec::new();
    for (inputs, found, expected) in &rows {
        let error = relative_error(*found, *expected);
        worst = worst.max(error);
        if error > tolerance {
            failures.push(format!(
                "{name}({inputs}) = {found:e}, expected {expected:e}"
            ));
        }
    }
    println!(
        "{name}: largest relative error {worst:.3e} over {} values",
        rows.len()
    );
    assert!(
        failures.is_empty(),
        "{} of {} values of {name} off by more than {tolerance:e}:\n{}",
        failures.len(),
        rows.len(),
        failures[..failures.len().min(20)].join("\n")
    );
    worst
}

/// The tolerance of the normal distribution.
pub const NORMAL: f64 = 1e-13;
/// The tolerance of a deterministic kernel, the logarithm of the beta function.
pub const KERNEL: f64 = 1e-12;
/// The tolerance of the distribution functions of the incomplete beta function: the incomplete
/// beta itself and the t and binomial distributions.
pub const DISTRIBUTION: f64 = 1e-10;
/// The tolerance of the error function of libm.
pub const ERF: f64 = 1e-15;
/// The tolerance of the gamma function of libm.
pub const GAMMA: f64 = 1e-13;
/// The tolerance of the regularized incomplete gamma function and the chi-squared
/// distribution function.
pub const GAMMA_INC: f64 = 1e-13;
/// The tolerance of the quantile of the chi-squared distribution.
pub const CHI2_PPF: f64 = 1e-13;

/// The FNV-1a hash of the bits of the values, every not a number as the one of `f64::NAN`: the
/// sign of a not a number which an operation makes is the target's (x86 sets it, wasm does
/// not), and no function of the crate gives it a meaning.
pub fn hash(values: &[f64]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |state, value| {
        let value = if value.is_nan() { f64::NAN } else { *value };
        value
            .to_bits()
            .to_le_bytes()
            .iter()
            .fold(state, |s, &byte| {
                (s ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            })
    })
}

/// The largest absolute value, zero of none.
pub fn largest(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |m: f64, v| m.max(v.abs()))
}

/// Holds every value found to the expected one at `tolerance` times `scale`, prints the
/// largest error relative to `scale` and returns it.
pub fn assert_close_to(
    name: &str,
    tolerance: f64,
    scale: f64,
    found: &[f64],
    expected: &[f64],
) -> f64 {
    assert_eq!(found.len(), expected.len(), "{name}: the number of values");
    let mut worst = 0.0_f64;
    let mut failures = Vec::new();
    for (index, (found, expected)) in found.iter().zip(expected).enumerate() {
        let error = (found - expected).abs() / scale;
        worst = worst.max(error);
        if error.is_nan() || error > tolerance {
            failures.push(format!(
                "{name}[{index}] = {found:e}, expected {expected:e}"
            ));
        }
    }
    println!(
        "{name}: largest error {worst:.3e} relative to {scale:e} over {} values",
        found.len()
    );
    assert!(
        failures.is_empty(),
        "{} of {} values of {name} off by more than {tolerance:e} times {scale:e}:\n{}",
        failures.len(),
        found.len(),
        failures[..failures.len().min(20)].join("\n")
    );
    worst
}

/// Holds an error of the crate to the one of the python, `{"error": <message>, "exception":
/// <class>}`: the same message and the kind of the class of its exception, a `ValueError`
/// where the fixture names no class.
pub fn check_error(error: &sbmlsim_stats::StatsError, expected: &Value, what: &str) {
    assert_eq!(error.message, expected["error"].as_str().unwrap(), "{what}");
    let class = expected
        .get("exception")
        .map_or("ValueError", |class| class.as_str().unwrap());
    assert_eq!(error.kind.name(), class, "{what}: {error}");
}

/// The residuals `0..n` of `large_residuals` of `oracles/make_oracles.py`, bit for bit:
/// `(((i * 2654435761) % 1000003) / 1000003 - 0.5) * 2^(i % 31 - 15)`.
pub fn large_residuals(n: usize) -> Vec<f64> {
    (0..n as i64)
        .map(|i| {
            let a = (i * 2_654_435_761) % 1_000_003;
            // 2^e exactly, from its bits
            let power = f64::from_bits(((i % 31 - 15 + 1023) as u64) << 52);
            (a as f64 / 1_000_003.0 - 0.5) * power
        })
        .collect()
}

/// The outcome of a call of the python with several values: the numbers (a boolean as 0 or
/// 1, an integer as its double), or the message of the error it raised
/// (`{"error": <message>, "exception": <class>}`).
pub fn values_of(value: &Value) -> std::result::Result<Vec<f64>, &str> {
    let one = |value: &Value| match value {
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        value => number(value),
    };
    match value {
        Value::Object(object) => Err(object["error"].as_str().unwrap()),
        Value::Array(items) => Ok(items.iter().map(one).collect()),
        value => Ok(vec![one(value)]),
    }
}

/// Holds a result of the crate of several values to the outcome of the python: an error to
/// the same message, the values to the expected ones, which it adds to `rows`.
pub fn check_values(
    rows: &mut Vec<(String, f64, f64)>,
    what: &str,
    found: sbmlsim_stats::Result<Vec<f64>>,
    expected: &Value,
) {
    match (found, values_of(expected)) {
        (Ok(found), Ok(expected)) => {
            assert_eq!(found.len(), expected.len(), "{what}: the number of values");
            for (index, (found, expected)) in found.into_iter().zip(expected).enumerate() {
                rows.push((format!("{what}[{index}]"), found, expected));
            }
        }
        (Err(error), Err(_)) => check_error(&error, expected, what),
        (found, expected) => panic!("{what}: {found:?}, the python {expected:?}"),
    }
}

//! The functions of the crate against numpy and scipy: the values `oracles/make_oracles.py`
//! computed once and stored as a JSON fixture per module (`oracles/<module>.json`).
//!
//! A fixture holds its cases by name, every case its inputs and the expected values, the
//! numbers as the doubles they were and the numbers which JSON cannot hold as the strings
//! "nan", "inf" and "-inf". A test runs a function over the inputs of a case and holds every
//! value to the expected one at a relative tolerance; it prints the largest relative error it
//! found (`cargo test -p sbmlsim-stats -- --nocapture`).
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

use std::collections::BTreeMap;

use serde_json::Value;

use sbmlsim_stats::correlation::{self, Marginal};
use sbmlsim_stats::testing::{self, Decision, Side};
use sbmlsim_stats::{
    Scale, draws, fisher, linalg, metrics, posterior, special, uncertainty, variability,
};

/// The fixture of a module, `oracles/<module>.json`.
struct Fixture {
    module: String,
    value: Value,
}

/// The text of the fixture of a module, compiled in.
fn source(module: &str) -> &'static str {
    match module {
        "special" => include_str!("../oracles/special.json"),
        "linalg" => include_str!("../oracles/linalg.json"),
        "metrics" => include_str!("../oracles/metrics.json"),
        "fisher" => include_str!("../oracles/fisher.json"),
        "uncertainty" => include_str!("../oracles/uncertainty.json"),
        "draws" => include_str!("../oracles/draws.json"),
        "testing" => include_str!("../oracles/testing.json"),
        _ => panic!("no fixture of the module {module}"),
    }
}

impl Fixture {
    fn load(module: &str) -> Self {
        let value = serde_json::from_str(source(module))
            .unwrap_or_else(|e| panic!("oracles/{module}.json: {e}"));
        Self {
            module: module.to_string(),
            value,
        }
    }

    /// The names of the cases which start with `prefix`, sorted, so the order does not
    /// depend on whether a crate of the build turns on `preserve_order` of serde_json.
    fn cases(&self, prefix: &str) -> Vec<String> {
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
    fn case(&self, name: &str) -> Case<'_> {
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
struct Case<'a> {
    name: String,
    value: &'a Value,
}

impl Case<'_> {
    /// The input `name`, a list of numbers.
    fn input(&self, name: &str) -> Vec<f64> {
        let value = self.value["inputs"]
            .get(name)
            .unwrap_or_else(|| panic!("the case {} has no input {name}", self.name));
        numbers(value)
    }

    /// The input `name`, a list of non-negative integers.
    fn input_u64(&self, name: &str) -> Vec<u64> {
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
    fn input_f64(&self, name: &str) -> f64 {
        number(&self.value["inputs"][name])
    }

    /// The input `name`, a list of integers.
    fn input_i64(&self, name: &str) -> Vec<i64> {
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
    fn input_usize(&self, name: &str) -> usize {
        let value = &self.value["inputs"][name];
        let integer = value
            .as_u64()
            .unwrap_or_else(|| panic!("the input {name} of {} is no integer", self.name));
        usize::try_from(integer).unwrap()
    }

    /// The input `name`, a boolean.
    fn input_bool(&self, name: &str) -> bool {
        self.value["inputs"][name]
            .as_bool()
            .unwrap_or_else(|| panic!("the input {name} of {} is no boolean", self.name))
    }

    /// The input `name`, a list of texts.
    fn input_strings(&self, name: &str) -> Vec<String> {
        self.value["inputs"][name]
            .as_array()
            .unwrap_or_else(|| panic!("the input {name} of {} is not a list", self.name))
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect()
    }

    /// The input `name`, a text.
    fn input_str(&self, name: &str) -> &str {
        self.value["inputs"][name]
            .as_str()
            .unwrap_or_else(|| panic!("the input {name} of {} is no text", self.name))
    }

    /// The input `name`, a number or `None` of null.
    fn input_option(&self, name: &str) -> Option<f64> {
        match &self.value["inputs"][name] {
            Value::Null => None,
            value => Some(number(value)),
        }
    }

    /// The expected output `name`, a number.
    fn expected_f64(&self, name: &str) -> f64 {
        number(&self.value["expected"][name])
    }

    /// The message of the ValueError the python raised, `None` for a case with values.
    fn error(&self) -> Option<&str> {
        self.value["expected"]
            .get("error")
            .map(|e| e.as_str().unwrap())
    }

    /// The expected values, a list of numbers.
    fn expected(&self) -> Vec<f64> {
        numbers(&self.value["expected"])
    }

    /// The expected output `name` of a case with several: `None` for an output the case does
    /// not have, `Some(None)` for a null one and `Some(Some(values))` for a list of numbers.
    fn output(&self, name: &str) -> Option<Option<Vec<f64>>> {
        match self.value["expected"].get(name)? {
            Value::Null => Some(None),
            value => Some(Some(numbers(value))),
        }
    }

    /// The expected output `name`, which the case must have.
    fn required(&self, name: &str) -> Vec<f64> {
        self.output(name)
            .flatten()
            .unwrap_or_else(|| panic!("the case {} has no output {name}", self.name))
    }
}

/// A number of a fixture: a JSON number or one of the strings "nan", "inf" and "-inf".
fn number(value: &Value) -> f64 {
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
fn numbers(value: &Value) -> Vec<f64> {
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
fn relative_error(found: f64, expected: f64) -> f64 {
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
fn assert_close(name: &str, tolerance: f64, rows: Vec<(String, f64, f64)>) -> f64 {
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

// --- special ----------------------------------------------------------------------------------

/// The tolerance of the normal distribution.
const NORMAL: f64 = 1e-13;
/// The tolerance of a deterministic kernel, the logarithm of the beta function.
const KERNEL: f64 = 1e-12;
/// The tolerance of the distribution functions of the incomplete beta function: the incomplete
/// beta itself and the t and binomial distributions.
const DISTRIBUTION: f64 = 1e-10;

#[test]
fn ln_beta_is_betaln_of_scipy() {
    let fixture = Fixture::load("special");
    let case = fixture.case("ln_beta");
    let rows = case
        .input("a")
        .into_iter()
        .zip(case.input("b"))
        .zip(case.expected())
        .map(|((a, b), expected)| (format!("{a}, {b}"), special::ln_beta(a, b), expected))
        .collect();
    assert_close("ln_beta", KERNEL, rows);
}

#[test]
fn ln_beta_of_a_large_and_a_small_parameter_is_accurate() {
    // mpmath at 50 digits; betaln of scipy 1.18 is off by 1.5e-10 and 3.5e-11 relative here
    let rows = [
        (0.5, 1e6, -6.335_390_211_057_437),
        (5e5, 0.5, -5.988_816_495_777_464),
        (0.5, 1e15, -16.697_023_254_530_64),
    ]
    .into_iter()
    .map(|(a, b, expected)| (format!("{a}, {b}"), special::ln_beta(a, b), expected))
    .collect();
    assert_close("ln_beta", 1e-14, rows);
}

#[test]
fn beta_inc_is_betainc_of_scipy() {
    let fixture = Fixture::load("special");
    for name in ["beta_inc", "beta_inc_large"] {
        let case = fixture.case(name);
        let rows = case
            .input("a")
            .into_iter()
            .zip(case.input("b"))
            .zip(case.input("x"))
            .zip(case.expected())
            .map(|(((a, b), x), expected)| {
                (
                    format!("{a}, {b}, {x}"),
                    special::beta_inc(a, b, x),
                    expected,
                )
            })
            .collect();
        assert_close(name, DISTRIBUTION, rows);
    }
}

#[test]
fn beta_inc_is_not_a_number_outside_of_its_domain() {
    for (a, b, x) in [
        (0.0, 1.0, 0.5),
        (1.0, -1.0, 0.5),
        (1.0, 1.0, -0.1),
        (1.0, 1.0, 1.1),
        (f64::NAN, 1.0, 0.5),
        (1.0, 1.0, f64::NAN),
    ] {
        assert!(special::beta_inc(a, b, x).is_nan(), "{a}, {b}, {x}");
    }
}

#[test]
fn beta_inc_of_an_infinite_parameter_is_the_limit_as_in_scipy() {
    // scipy 1.18: betainc(inf, 1, 0.5) = 0, betainc(2, inf, 0.5) = 1, betainc(inf, inf, x) nan
    let inf = f64::INFINITY;
    for (a, b, x, expected) in [
        (inf, 1.0, 0.5, 0.0),
        (inf, 2.0, 0.999, 0.0),
        (inf, 1.0, 0.0, 0.0),
        (inf, 1.0, 1.0, 1.0),
        (2.0, inf, 0.5, 1.0),
        (1.0, inf, 0.5, 1.0),
        (2.0, inf, 0.0, 0.0),
        (2.0, inf, 1.0, 1.0),
    ] {
        assert_eq!(special::beta_inc(a, b, x), expected, "{a}, {b}, {x}");
    }
    assert!(special::beta_inc(inf, inf, 0.5).is_nan());
}

#[test]
fn beta_inc_is_not_a_number_where_it_would_lose_its_digits() {
    // two parameters above CF_MAX, where the continued fraction would lose more than 1e-6
    assert!(special::beta_inc(1e11, 1e11, 0.5).is_nan());
    assert!(special::binom_cdf(10_000_000_000, 30_000_000_000, 0.4).is_nan());
    assert!(special::beta_inc(1e9, 1e9, 0.5).is_finite());
}

#[test]
fn normal_cdf_is_norm_cdf_of_scipy() {
    let fixture = Fixture::load("special");
    let case = fixture.case("normal_cdf");
    let rows = case
        .input("x")
        .into_iter()
        .zip(case.expected())
        .map(|(x, expected)| (format!("{x}"), special::normal_cdf(x), expected))
        .collect();
    assert_close("normal_cdf", NORMAL, rows);
}

#[test]
fn normal_ppf_is_norm_ppf_of_scipy() {
    let fixture = Fixture::load("special");
    let case = fixture.case("normal_ppf");
    let rows = case
        .input("p")
        .into_iter()
        .zip(case.expected())
        .map(|(p, expected)| (format!("{p}"), special::normal_ppf(p), expected))
        .collect();
    assert_close("normal_ppf", NORMAL, rows);
    assert_eq!(special::normal_ppf(0.5), 0.0);
}

#[test]
fn t_cdf_is_t_cdf_of_scipy() {
    let fixture = Fixture::load("special");
    for name in ["t_cdf", "t_cdf_large_df"] {
        let case = fixture.case(name);
        let rows = case
            .input("x")
            .into_iter()
            .zip(case.input("df"))
            .zip(case.expected())
            .map(|((x, df), expected)| (format!("{x}, {df}"), special::t_cdf(x, df), expected))
            .collect();
        assert_close(name, DISTRIBUTION, rows);
    }
}

#[test]
fn t_ppf_is_t_ppf_of_scipy() {
    let fixture = Fixture::load("special");
    let case = fixture.case("t_ppf");
    let rows = case
        .input("p")
        .into_iter()
        .zip(case.input("df"))
        .zip(case.expected())
        .map(|((p, df), expected)| (format!("{p}, {df}"), special::t_ppf(p, df), expected))
        .collect();
    assert_close("t_ppf", DISTRIBUTION, rows);
}

#[test]
fn the_t_distribution_of_infinitely_many_degrees_of_freedom_is_the_normal_one() {
    for df in [1e300, f64::INFINITY] {
        let normal = special::normal_ppf(0.975);
        assert!((special::t_ppf(0.975, df) - normal).abs() <= 1e-12 * normal);
        for x in [-30.0, -2.0, 0.0, 1.5] {
            let normal = special::normal_cdf(x);
            assert!(
                (special::t_cdf(x, df) - normal).abs() <= 1e-12 * normal,
                "{x}, {df}"
            );
        }
    }
}

#[test]
fn the_t_distribution_is_symmetric_and_its_quantile_inverts_it() {
    for df in [1.0, 2.0, 3.5, 10.0, 30.0, 1e6, 1e9, 1e14] {
        assert_eq!(special::t_ppf(0.5, df), 0.0);
        assert_eq!(special::t_cdf(0.0, df), 0.5);
        for p in [1e-300, 1e-12, 0.01, 0.3, 0.49] {
            let t = special::t_ppf(p, df);
            let back = special::t_cdf(t, df);
            assert!((back - p).abs() <= 1e-12 * p, "{p}, {df}: {back}");
        }
        // probabilities whose complement is exact
        for (p, q) in [(0.25, 0.75), (0.125, 0.875), (0.0625, 0.9375)] {
            assert_eq!(1.0 - p, q);
            assert_eq!(special::t_ppf(p, df), -special::t_ppf(q, df), "{p}, {df}");
        }
        for x in [1e-3, 0.5, 1.0, 2.0, 10.0, 1e3] {
            let (lower, upper) = (special::t_cdf(-x, df), special::t_cdf(x, df));
            assert!(
                (lower - (1.0 - upper)).abs() <= 1e-15 * lower.max(1e-300) + f64::EPSILON / 2.0,
                "{x}, {df}: {lower} {upper}"
            );
        }
    }
}

#[test]
fn the_t_distribution_is_not_a_number_outside_of_its_domain() {
    assert_eq!(special::t_ppf(0.0, 3.0), f64::NEG_INFINITY);
    assert_eq!(special::t_ppf(1.0, 3.0), f64::INFINITY);
    for (p, df) in [
        (-0.1, 3.0),
        (1.1, 3.0),
        (f64::NAN, 3.0),
        (0.5, 0.0),
        (0.5, -1.0),
    ] {
        assert!(special::t_ppf(p, df).is_nan(), "{p}, {df}");
    }
    for (x, df) in [(f64::NAN, 3.0), (1.0, 0.0), (1.0, f64::NAN)] {
        assert!(special::t_cdf(x, df).is_nan(), "{x}, {df}");
    }
    assert_eq!(special::t_cdf(f64::NEG_INFINITY, 3.0), 0.0);
    assert_eq!(special::t_cdf(f64::INFINITY, 3.0), 1.0);
}

#[test]
fn binom_cdf_is_binom_cdf_of_scipy() {
    let fixture = Fixture::load("special");
    let case = fixture.case("binom_cdf");
    let rows = case
        .input_u64("k")
        .into_iter()
        .zip(case.input_u64("n"))
        .zip(case.input("p"))
        .zip(case.expected())
        .map(|(((k, n), p), expected)| {
            (
                format!("{k}, {n}, {p}"),
                special::binom_cdf(k, n, p),
                expected,
            )
        })
        .collect();
    assert_close("binom_cdf", DISTRIBUTION, rows);
}

#[test]
fn binom_ppf_is_binom_ppf_of_scipy_exactly() {
    let fixture = Fixture::load("special");
    let case = fixture.case("binom_ppf");
    let rows = case
        .input("q")
        .into_iter()
        .zip(case.input_u64("n"))
        .zip(case.input("p"))
        .zip(case.expected())
        .map(|(((q, n), p), expected)| {
            (
                format!("{q}, {n}, {p}"),
                special::binom_ppf(q, n, p),
                expected,
            )
        })
        .collect();
    assert_close("binom_ppf", 0.0, rows);
}

#[test]
fn binom_ppf_at_a_value_of_the_distribution_function_is_at_most_one_off_scipy() {
    // a q which is a value of the distribution function is a tie, which the last digits of the
    // distribution function decide, of the crate or of scipy (Boost); scipy itself is not
    // consistent there (binom.ppf(0.1, 1, 0.9) is 0, binom.ppf(0.01, 2, 0.9) is 1)
    let fixture = Fixture::load("special");
    let case = fixture.case("binom_ppf_ties");
    let mut same = 0;
    let expected = case.expected();
    for (((q, n), p), expected) in case
        .input("q")
        .into_iter()
        .zip(case.input_u64("n"))
        .zip(case.input("p"))
        .zip(&expected)
    {
        let found = special::binom_ppf(q, n, p);
        if found == *expected || (found.is_nan() && expected.is_nan()) {
            same += 1;
        } else {
            assert!(
                (found - expected).abs() == 1.0,
                "binom_ppf({q}, {n}, {p}) = {found}, expected {expected}"
            );
        }
    }
    println!(
        "binom_ppf_ties: {same} of {} values those of scipy, the others one off",
        expected.len()
    );
}

#[test]
fn the_binomial_distribution_at_its_ends_and_of_a_degenerate_probability() {
    for n in [0, 1, 10, 4096] {
        let n_f64 = n as f64;
        for p in [0.0, 0.3, 1.0] {
            // the lower end of the support minus one and its upper end, as scipy
            assert_eq!(special::binom_ppf(0.0, n, p), -1.0);
            assert_eq!(special::binom_ppf(1.0, n, p), n_f64);
            assert_eq!(special::binom_cdf(n, n, p), 1.0);
        }
        for q in [1e-12, 0.5, 1.0 - 1e-9] {
            // every draw a failure or every draw a success
            assert_eq!(special::binom_ppf(q, n, 0.0), 0.0);
            assert_eq!(special::binom_ppf(q, n, 1.0), n_f64);
        }
        for k in 0..n.min(20) {
            assert_eq!(special::binom_cdf(k, n, 0.0), 1.0);
            assert_eq!(special::binom_cdf(k, n, 1.0), 0.0);
        }
    }
    for (q, p) in [
        (-0.1, 0.5),
        (1.1, 0.5),
        (f64::NAN, 0.5),
        (0.5, -0.1),
        (0.5, 1.1),
    ] {
        assert!(special::binom_ppf(q, 10, p).is_nan(), "{q}, {p}");
    }
    assert!(special::binom_cdf(3, 10, f64::NAN).is_nan());
}

/// The FNV-1a hash of the bits of the values, every not a number as the one of `f64::NAN`: the
/// sign of a not a number which an operation makes is the target's (x86 sets it, wasm does
/// not), and no function of the crate gives it a meaning.
fn hash(values: &[f64]) -> u64 {
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

#[test]
fn the_special_functions_give_the_same_bits_on_every_target() {
    // the values of every case of the fixture, natively and in wasm (libm on both)
    let fixture = Fixture::load("special");
    let mut values = Vec::new();
    let case = fixture.case("ln_beta");
    for (a, b) in case.input("a").into_iter().zip(case.input("b")) {
        values.push(special::ln_beta(a, b));
    }
    for name in ["beta_inc", "beta_inc_large"] {
        let case = fixture.case(name);
        for ((a, b), x) in case
            .input("a")
            .into_iter()
            .zip(case.input("b"))
            .zip(case.input("x"))
        {
            values.push(special::beta_inc(a, b, x));
        }
    }
    for x in fixture.case("normal_cdf").input("x") {
        values.push(special::normal_cdf(x));
    }
    for p in fixture.case("normal_ppf").input("p") {
        values.push(special::normal_ppf(p));
    }
    for name in ["t_cdf", "t_cdf_large_df"] {
        let case = fixture.case(name);
        for (x, df) in case.input("x").into_iter().zip(case.input("df")) {
            values.push(special::t_cdf(x, df));
        }
    }
    let case = fixture.case("t_ppf");
    for (p, df) in case.input("p").into_iter().zip(case.input("df")) {
        values.push(special::t_ppf(p, df));
    }
    let case = fixture.case("binom_cdf");
    for ((k, n), p) in case
        .input_u64("k")
        .into_iter()
        .zip(case.input_u64("n"))
        .zip(case.input("p"))
    {
        values.push(special::binom_cdf(k, n, p));
    }
    for name in ["binom_ppf", "binom_ppf_ties"] {
        let case = fixture.case(name);
        for ((q, n), p) in case
            .input("q")
            .into_iter()
            .zip(case.input_u64("n"))
            .zip(case.input("p"))
        {
            values.push(special::binom_ppf(q, n, p));
        }
    }
    assert_eq!(
        hash(&values),
        0xd800_ea5c_0a1d_4805,
        "{} values",
        values.len()
    );
}

// --- linalg -----------------------------------------------------------------------------------

/// The tolerance of the linear algebra against numpy, relative to the largest value of the
/// matrix compared: LAPACK sums in another order, and an eigenvector is only as accurate as
/// the gap of its eigenvalue to the next one.
const LINALG: f64 = 1e-10;

/// The cases of `oracles/linalg.json` of a symmetric matrix.
const SYMMETRIC: [&str; 6] = [
    "one",
    "two",
    "rank_deficient",
    "spd_10",
    "spd_200",
    "indefinite_5",
];

/// The largest absolute value, zero of none.
fn largest(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |m: f64, v| m.max(v.abs()))
}

/// Holds every value found to the expected one at `tolerance` times `scale`, prints the
/// largest error relative to `scale` and returns it.
fn assert_close_to(name: &str, tolerance: f64, scale: f64, found: &[f64], expected: &[f64]) -> f64 {
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

/// The product of the row-major `n x m` matrix `a` and the `m x p` matrix `b`.
fn matmul(a: &[f64], b: &[f64], n: usize, m: usize, p: usize) -> Vec<f64> {
    let mut c = vec![0.0; n * p];
    for i in 0..n {
        for l in 0..m {
            for j in 0..p {
                c[i * p + j] += a[i * m + l] * b[l * p + j];
            }
        }
    }
    c
}

/// The transpose of the row-major `n x m` matrix `a`.
fn transpose(a: &[f64], n: usize, m: usize) -> Vec<f64> {
    let mut t = vec![0.0; n * m];
    for i in 0..n {
        for j in 0..m {
            t[j * n + i] = a[i * m + j];
        }
    }
    t
}

/// The `k x k` identity.
fn identity(k: usize) -> Vec<f64> {
    let mut e = vec![0.0; k * k];
    for i in 0..k {
        e[i * k + i] = 1.0;
    }
    e
}

/// `V diag(values) V^T`, the matrix of its eigendecomposition.
fn reconstruct(values: &[f64], vectors: &[f64], k: usize) -> Vec<f64> {
    let mut scaled = vectors.to_vec();
    for i in 0..k {
        for j in 0..k {
            scaled[i * k + j] *= values[j];
        }
    }
    matmul(&scaled, &transpose(vectors, k, k), k, k, k)
}

/// `F F^T` of a factor.
fn outer(f: &[f64], k: usize) -> Vec<f64> {
    matmul(f, &transpose(f, k, k), k, k, k)
}

#[test]
fn eigh_is_eigh_of_numpy() {
    let fixture = Fixture::load("linalg");
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let (values, vectors) = linalg::eigh(&a, k).unwrap();
        let expected = case.required("eigenvalues");
        let scale = largest(&expected);
        assert_close_to(
            &format!("{name} eigenvalues"),
            LINALG,
            scale,
            &values,
            &expected,
        );
        assert!(
            values.windows(2).all(|w| w[0] >= w[1]),
            "{name}: not descending"
        );
        let product = reconstruct(&values, &vectors, k);
        assert_close_to(
            &format!("{name} V D V^T"),
            LINALG,
            largest(&a),
            &product,
            &a,
        );
        let orthonormal = matmul(&transpose(&vectors, k, k), &vectors, k, k, k);
        assert_close_to(
            &format!("{name} V^T V"),
            LINALG,
            1.0,
            &orthonormal,
            &identity(k),
        );
        // the sign of an eigenvector: its largest component positive
        let columns = transpose(&vectors, k, k);
        for column in columns.chunks(k) {
            let first_largest = column
                .iter()
                .copied()
                .reduce(|m, v| if v.abs() > m.abs() { v } else { m })
                .unwrap();
            assert!(first_largest > 0.0, "{name}: {column:?}");
        }
        // numpy's eigenvectors, whose sign LAPACK chooses: the one closer to ours
        if let Some(Some(numpy)) = case.output("vectors") {
            let mut numpy = transpose(&numpy, k, k);
            for (column, ours) in numpy.chunks_mut(k).zip(columns.chunks(k)) {
                let dot: f64 = column.iter().zip(ours).map(|(x, y)| x * y).sum();
                if dot < 0.0 {
                    column.iter_mut().for_each(|x| *x = -*x);
                }
            }
            assert_close_to(&format!("{name} vectors"), LINALG, 1.0, &columns, &numpy);
        }
    }
}

#[test]
fn eigvalsh_is_eigvalsh_of_numpy_and_the_eigenvalues_of_eigh() {
    let fixture = Fixture::load("linalg");
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let values = linalg::eigvalsh(&a, k).unwrap();
        let expected = case.required("eigenvalues");
        assert_close_to(
            &format!("{name} eigvalsh"),
            LINALG,
            largest(&expected),
            &values,
            &expected,
        );
        let (of_eigh, _) = linalg::eigh(&a, k).unwrap();
        assert_eq!(
            values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            of_eigh.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{name}"
        );
    }
}

#[test]
fn pinv_symmetric_is_pinv_of_numpy_and_a_moore_penrose_inverse() {
    let fixture = Fixture::load("linalg");
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let found = linalg::pinv_symmetric(&a, k).unwrap();
        // exactly symmetric
        assert_eq!(found, transpose(&found, k, k), "{name}");
        // numpy's, stored for the small matrices
        if let Some(Some(expected)) = case.output("pinv") {
            let scale = largest(&expected);
            assert_close_to(&format!("{name} pinv"), LINALG, scale, &found, &expected);
        }
        // the identities of Moore and Penrose, A A+ A = A, A+ A A+ = A+ and A A+ and A+ A
        // symmetric: an error of about the condition times the rounding of a sum of k
        // products, 7.7e3 * 2.2e-16 * sqrt(200) = 2.4e-11 for the 200 x 200 matrix
        let a_pinv = matmul(&a, &found, k, k, k);
        let back = matmul(&a_pinv, &a, k, k, k);
        assert_close_to(&format!("{name} A A+ A"), LINALG, largest(&a), &back, &a);
        // A and A+ are symmetric, so (A A+)ᵀ = A+ A: A A+ is symmetric if it equals A+ A,
        // which then is as well; both are the projection onto the range of A, of entries
        // of at most 1
        let pinv_a = matmul(&found, &a, k, k, k);
        assert_close_to(
            &format!("{name} A A+ = A+ A"),
            LINALG,
            1.0,
            &a_pinv,
            &pinv_a,
        );
        let pinv_back = matmul(&pinv_a, &found, k, k, k);
        let scale = largest(&found);
        assert_close_to(
            &format!("{name} A+ A A+"),
            LINALG,
            scale,
            &pinv_back,
            &found,
        );
    }
}

#[test]
fn a_subnormal_matrix_is_decomposed_exactly() {
    // the largest value below the smallest normal double: the scaling of the Jacobi method
    // by a power of two of more than 1023
    let tiny = 2f64.powi(-1023);
    assert!(tiny < f64::MIN_POSITIVE);
    assert_eq!(linalg::eigh(&[tiny], 1).unwrap(), (vec![tiny], vec![1.0]));
    assert_eq!(linalg::eigvalsh(&[tiny], 1).unwrap(), vec![tiny]);
    assert_eq!(
        linalg::pinv_symmetric(&[tiny], 1).unwrap(),
        vec![2f64.powi(1023)]
    );
    assert_eq!(linalg::eigvalsh(&[1e-310], 1).unwrap(), vec![1e-310]);
    // [[1, 1/4], [1/4, 1]] times 2^-1023, the eigenvalues 5/4 and 3/4 of it exact, the
    // eigenvectors those of the normal matrix
    let unit = [1.0, 0.25, 0.25, 1.0];
    let a: Vec<f64> = unit.iter().map(|v| v * tiny).collect();
    let (values, vectors) = linalg::eigh(&a, 2).unwrap();
    assert_eq!(values, vec![1.25 * tiny, 0.75 * tiny]);
    assert_eq!(linalg::eigvalsh(&a, 2).unwrap(), values);
    assert_eq!(vectors, linalg::eigh(&unit, 2).unwrap().1);
    // the inverse, 2^1023 / (15/16) [[1, -1/4], [-1/4, 1]]
    let huge = 2f64.powi(1023);
    let expected = [
        16.0 / 15.0 * huge,
        -4.0 / 15.0 * huge,
        -4.0 / 15.0 * huge,
        16.0 / 15.0 * huge,
    ];
    let found = linalg::pinv_symmetric(&a, 2).unwrap();
    assert_close_to(
        "subnormal pinv",
        4.0 * f64::EPSILON,
        largest(&expected),
        &found,
        &expected,
    );
}

#[test]
fn cholesky_is_cholesky_of_numpy_and_none_of_an_indefinite_matrix() {
    let fixture = Fixture::load("linalg");
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let found = linalg::cholesky(&a, k);
        match case.output("cholesky") {
            Some(Some(expected)) => {
                let found = found.unwrap_or_else(|| panic!("{name}: no factor"));
                assert_close_to(
                    &format!("{name} cholesky"),
                    LINALG,
                    largest(&expected),
                    &found,
                    &expected,
                );
            }
            Some(None) => assert!(found.is_none(), "{name}: a factor of an indefinite matrix"),
            // the large matrix: its factor, lower triangular, gives it back
            None if name == "spd_200" => {
                let found = found.unwrap();
                for i in 0..k {
                    assert!(found[i * k + i + 1..(i + 1) * k].iter().all(|&v| v == 0.0));
                }
                assert_close_to("spd_200 L L^T", LINALG, largest(&a), &outer(&found, k), &a);
            }
            None => {}
        }
    }
}

#[test]
fn factor_is_cholesky_else_the_eigen_factor_as_draws() {
    let fixture = Fixture::load("linalg");
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let found = linalg::factor(&a, k).unwrap();
        if let Some(lower) = linalg::cholesky(&a, k) {
            assert_eq!(found, lower, "{name}");
        } else {
            // V sqrt(clip(values, 0)), of the eigendecomposition of the crate
            let (values, mut fallback) = linalg::eigh(&a, k).unwrap();
            for i in 0..k {
                for j in 0..k {
                    fallback[i * k + j] *= values[j].max(0.0).sqrt();
                }
            }
            assert_eq!(found, fallback, "{name}");
        }
        let expected = case
            .output("factor_product")
            .flatten()
            .unwrap_or_else(|| a.clone());
        assert_close_to(
            &format!("{name} F F^T"),
            LINALG,
            largest(&a),
            &outer(&found, k),
            &expected,
        );
    }
    // the indefinite matrix takes the eigen factor
    assert!(linalg::cholesky(&fixture.case("indefinite_5").input("a"), 5).is_none());
}

#[test]
fn gram_is_the_product_of_numpy_and_its_pinv_cuts_the_null_space() {
    let fixture = Fixture::load("linalg");
    for name in ["gram", "gram_rank_deficient"] {
        let case = fixture.case(name);
        let (n, k, j) = (
            case.input_usize("n"),
            case.input_usize("k"),
            case.input("j"),
        );
        let found = linalg::gram(&j, n, k).unwrap();
        assert_eq!(found, transpose(&found, k, k), "{name}");
        // relative to sqrt(G_aa G_bb), the bound of |G_ab| (Cauchy-Schwarz)
        let expected = case.required("gram");
        let mut worst = 0.0_f64;
        for a in 0..k {
            for b in 0..k {
                let scale = (expected[a * k + a] * expected[b * k + b]).sqrt();
                let error = (found[a * k + b] - expected[a * k + b]).abs() / scale;
                assert!(error <= 1e-12, "{name}[{a}, {b}]: {}", found[a * k + b]);
                worst = worst.max(error);
            }
        }
        println!("{name}: largest error {worst:.3e} relative to sqrt(G_aa G_bb)");
        let pinv = linalg::pinv_symmetric(&found, k).unwrap();
        let expected = case.required("pinv");
        assert_close_to(
            &format!("{name} pinv"),
            LINALG,
            largest(&expected),
            &pinv,
            &expected,
        );
    }
}

#[test]
fn gram_propagates_a_not_a_number_into_its_row_and_column() {
    // J of 3 x 2 with a not a number in the second column, as J.T @ J of numpy
    let j = [1.0, 2.0, 3.0, f64::NAN, 5.0, 6.0];
    let g = linalg::gram(&j, 3, 2).unwrap();
    assert_eq!(g[0], 35.0);
    assert!(g[1].is_nan() && g[2].is_nan() && g[3].is_nan());
    assert_eq!(linalg::gram(&[], 0, 2).unwrap(), vec![0.0; 4]);
}

#[test]
fn a_matrix_which_is_not_symmetric_is_an_error() {
    let a = [1.0, 2.0, 2.1, 1.0];
    for result in [
        linalg::eigh(&a, 2).map(|_| ()),
        linalg::eigvalsh(&a, 2).map(|_| ()),
        linalg::pinv_symmetric(&a, 2).map(|_| ()),
        linalg::factor(&a, 2).map(|_| ()),
    ] {
        let error = result.unwrap_err();
        assert!(error.message.contains("is not symmetric"), "{error}");
    }
    // a difference of the last digits is symmetric, the mean of the two is taken
    let (values, _) = linalg::eigh(&[1.0, 2.0, 2.0 * (1.0 + 1e-15), 1.0], 2).unwrap();
    assert!((values[0] - 3.0).abs() < 1e-14 && (values[1] + 1.0).abs() < 1e-14);
}

#[test]
fn a_matrix_of_the_wrong_size_or_not_finite_is_an_error() {
    for a in [
        vec![1.0, f64::NAN, f64::NAN, 1.0],
        vec![1.0, 0.0, 0.0, f64::INFINITY],
    ] {
        let error = linalg::eigh(&a, 2).unwrap_err();
        assert!(error.message.contains("is not finite"), "{error}");
        assert!(linalg::eigvalsh(&a, 2).is_err());
        assert!(linalg::pinv_symmetric(&a, 2).is_err());
        assert!(linalg::factor(&a, 2).is_err());
        assert!(linalg::cholesky(&a, 2).is_none());
    }
    let error = linalg::eigh(&[1.0, 0.0, 0.0], 2).unwrap_err();
    assert!(error.message.contains("2 x 2"), "{error}");
    assert!(linalg::cholesky(&[1.0, 0.0, 0.0], 2).is_none());
}

#[test]
fn the_empty_and_the_zero_matrix() {
    assert_eq!(linalg::eigh(&[], 0).unwrap(), (vec![], vec![]));
    assert_eq!(linalg::pinv_symmetric(&[], 0).unwrap(), Vec::<f64>::new());
    assert_eq!(linalg::factor(&[], 0).unwrap(), Vec::<f64>::new());
    let zero = vec![0.0; 9];
    let (values, vectors) = linalg::eigh(&zero, 3).unwrap();
    assert_eq!((values, vectors), (vec![0.0; 3], identity(3)));
    // numpy: pinv of zeros is zeros, the Cholesky factor of zeros fails
    assert_eq!(linalg::pinv_symmetric(&zero, 3).unwrap(), zero);
    assert!(linalg::cholesky(&zero, 3).is_none());
    assert_eq!(linalg::factor(&zero, 3).unwrap(), zero);
}

#[test]
fn the_eigenvalues_of_a_scaled_matrix_are_scaled_exactly() {
    // a power of two scales every value exactly, at the ends of the range of a double
    let a = [2.0, 1.0, 1.0, 3.0];
    let (values, vectors) = linalg::eigh(&a, 2).unwrap();
    for scale in [2f64.powi(1000), 2f64.powi(-1000)] {
        let scaled: Vec<f64> = a.iter().map(|v| v * scale).collect();
        let (found, found_vectors) = linalg::eigh(&scaled, 2).unwrap();
        assert_eq!(found, values.iter().map(|v| v * scale).collect::<Vec<_>>());
        assert_eq!(found_vectors, vectors);
    }
}

#[test]
fn the_linear_algebra_gives_the_same_bits_on_every_target() {
    // every output of every case, natively and in wasm
    let fixture = Fixture::load("linalg");
    let mut values = Vec::new();
    for name in SYMMETRIC {
        let case = fixture.case(name);
        let (k, a) = (case.input_usize("k"), case.input("a"));
        let (eigenvalues, vectors) = linalg::eigh(&a, k).unwrap();
        values.extend(eigenvalues);
        values.extend(vectors);
        values.extend(linalg::pinv_symmetric(&a, k).unwrap());
        values.extend(linalg::factor(&a, k).unwrap());
    }
    for name in ["gram", "gram_rank_deficient"] {
        let case = fixture.case(name);
        let (n, k, j) = (
            case.input_usize("n"),
            case.input_usize("k"),
            case.input("j"),
        );
        let gram = linalg::gram(&j, n, k).unwrap();
        values.extend(linalg::pinv_symmetric(&gram, k).unwrap());
        values.extend(gram);
    }
    assert_eq!(
        hash(&values),
        0xa77a_1698_534f_d118,
        "{} values",
        values.len()
    );
}

// --- metrics ----------------------------------------------------------------------------------

/// The outcome of a call of the python: the value, or the message of the ValueError it raised
/// (`{"error": <message>}`).
fn outcome(value: &Value) -> std::result::Result<&Value, &str> {
    match value.get("error") {
        Some(message) => Err(message.as_str().unwrap()),
        None => Ok(value),
    }
}

/// Holds an error of the crate to the one of the python, `{"error": <message>, "exception":
/// <class>}`: the same message and the kind of the class of its exception, a `ValueError`
/// where the fixture names no class.
fn check_error(error: &sbmlsim_stats::StatsError, expected: &Value, what: &str) {
    assert_eq!(error.message, expected["error"].as_str().unwrap(), "{what}");
    let class = expected
        .get("exception")
        .map_or("ValueError", |class| class.as_str().unwrap());
    assert_eq!(error.kind.name(), class, "{what}: {error}");
}

/// Holds a result of the crate to the outcome of the python: an error to the same message, a
/// value to the expected one, which it adds to `rows`.
fn check(
    rows: &mut Vec<(String, f64, f64)>,
    what: String,
    found: sbmlsim_stats::Result<f64>,
    expected: &Value,
) {
    match (found, outcome(expected)) {
        (Ok(found), Ok(expected)) => rows.push((what, found, number(expected))),
        (Err(error), Err(_)) => check_error(&error, expected, &what),
        (found, expected) => panic!("{what}: {found:?}, the python {expected:?}"),
    }
}

/// Holds the values of an elementwise function to the expected ones: equal lengths and every
/// value at `tolerance`.
fn check_all(name: &str, tolerance: f64, found: &[f64], expected: &[f64]) {
    assert_eq!(found.len(), expected.len(), "{name}: the number of values");
    let rows = found
        .iter()
        .zip(expected)
        .enumerate()
        .map(|(index, (&found, &expected))| (format!("{index}"), found, expected))
        .collect();
    assert_close(name, tolerance, rows);
}

#[test]
fn the_metrics_of_residuals_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    let mut rows = Vec::new();
    for name in fixture.cases("residuals/") {
        let case = fixture.case(&name);
        let x = case.input("x");
        let k = i64::try_from(case.input_usize("k")).unwrap();
        let n = i64::try_from(x.len()).unwrap();
        let expected = &case.value["expected"];
        rows.push((
            format!("{name} sse"),
            metrics::sse(&x),
            number(&expected["sse"]),
        ));
        let mse = metrics::mse(&x);
        check(
            &mut rows,
            format!("{name} mse"),
            mse.clone(),
            &expected["mse"],
        );
        let rmse = mse.clone().and_then(metrics::rmse_from_mse);
        check(&mut rows, format!("{name} rmse"), rmse, &expected["rmse"]);
        let aic = mse.clone().and_then(|mse| metrics::aic_from_mse(mse, n, k));
        check(&mut rows, format!("{name} aic"), aic, &expected["aic"]);
        let bic = mse.and_then(|mse| metrics::bic_from_mse(mse, n, k));
        check(&mut rows, format!("{name} bic"), bic, &expected["bic"]);
        rows.push((
            format!("{name} nrmse"),
            metrics::nrmse(&x),
            number(&expected["nrmse"]),
        ));
    }
    assert_close("residuals", KERNEL, rows);
}

/// The residuals `0..n` of `large_residuals` of `oracles/make_oracles.py`, bit for bit:
/// `(((i * 2654435761) % 1000003) / 1000003 - 0.5) * 2^(i % 31 - 15)`.
fn large_residuals(n: usize) -> Vec<f64> {
    (0..n as i64)
        .map(|i| {
            let a = (i * 2_654_435_761) % 1_000_003;
            // 2^e exactly, from its bits
            let power = f64::from_bits(((i % 31 - 15 + 1023) as u64) << 52);
            (a as f64 / 1_000_003.0 - 0.5) * power
        })
        .collect()
}

#[test]
fn the_metrics_of_many_residuals_are_the_pairwise_sums_of_numpy_bit_for_bit() {
    // more than a block of 128 and than the 8192 values of a buffer of numpy, which sums a
    // contiguous array pairwise as a whole: the fixture holds that a sum in blocks of 8192
    // would differ
    let fixture = Fixture::load("metrics");
    for name in fixture.cases("large_residuals/") {
        let case = fixture.case(&name);
        let x = large_residuals(case.input_usize("n"));
        assert_eq!(x[..4], case.required("first")[..], "{name}: the formula");
        let expected = &case.value["expected"];
        assert_eq!(metrics::sse(&x), number(&expected["sse"]), "{name}");
        assert_eq!(
            metrics::mse(&x).unwrap(),
            number(&expected["mse"]),
            "{name}"
        );
        assert_eq!(metrics::nrmse(&x), number(&expected["nrmse"]), "{name}");
    }
}

#[test]
fn the_metrics_of_an_mse_are_the_ones_of_sbmlsim_and_refuse_as_it_does() {
    let fixture = Fixture::load("metrics");
    let case = fixture.case("from_mse");
    let (mses, ns, ks) = (case.input("mse"), case.input_i64("n"), case.input_i64("k"));
    let expected = &case.value["expected"];
    let mut rows = Vec::new();
    for (index, ((&mse, &n), &k)) in mses.iter().zip(&ns).zip(&ks).enumerate() {
        let what = format!("mse={mse}, n={n}, k={k}");
        check(
            &mut rows,
            format!("rmse({what})"),
            metrics::rmse_from_mse(mse),
            &expected["rmse"][index],
        );
        check(
            &mut rows,
            format!("aic({what})"),
            metrics::aic_from_mse(mse, n, k),
            &expected["aic"][index],
        );
        check(
            &mut rows,
            format!("bic({what})"),
            metrics::bic_from_mse(mse, n, k),
            &expected["bic"][index],
        );
    }
    assert_close("from_mse", KERNEL, rows);
}

#[test]
fn r_squared_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    let mut rows = Vec::new();
    for name in fixture.cases("r_squared/") {
        let case = fixture.case(&name);
        let found = metrics::r_squared(&case.input("y"), &case.input("f"));
        check(&mut rows, name.clone(), found, &case.value["expected"]);
    }
    assert_close("r_squared", KERNEL, rows);
}

#[test]
fn the_fold_criteria_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    for name in fixture.cases("fold/") {
        let case = fixture.case(&name);
        let (y, f, fold) = (case.input("y"), case.input("f"), case.input_f64("fold"));
        let expected = &case.value["expected"];
        let counts = metrics::fold_counts(&y, &f, fold);
        let accepted = metrics::fold_accepted(&y, &f, fold);
        match outcome(&expected["counts"]) {
            Ok(counts_expected) => {
                let counts_expected = numbers(counts_expected);
                let (inside, n_ratio) = counts.unwrap();
                assert_eq!(
                    vec![inside as f64, n_ratio as f64],
                    counts_expected,
                    "{name}"
                );
                check_all(&name, 0.0, &accepted.unwrap(), &case.required("accepted"));
            }
            Err(message) => {
                assert_eq!(counts.unwrap_err().message, message, "{name}");
                // the criterion of a point refuses what the counts refuse
                assert_eq!(accepted.unwrap_err().message, message, "{name}");
            }
        }
    }
}

#[test]
fn the_uncertainty_criterion_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    for name in fixture.cases("sd/") {
        let case = fixture.case(&name);
        let found = metrics::sd_accepted(
            &case.input("y"),
            &case.input("f"),
            &case.input("sd"),
            case.input_f64("k"),
        )
        .unwrap();
        check_all(&name, 0.0, &found, &case.expected());
    }
    let error = metrics::sd_accepted(&[1.0, 2.0], &[1.0, 2.0], &[1.0], 2.0).unwrap_err();
    assert_eq!(
        error.message,
        "The uncertainty criterion requires as many predictions and standard deviations as \
         data points, but got '2' and '1' for '2'."
    );
}

#[test]
fn the_fold_errors_and_the_agreement_of_a_subset_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    let mut rows = Vec::new();
    let mut distribution = Vec::new();
    for name in fixture.cases("fold_errors/") {
        let case = fixture.case(&name);
        let (y, f) = (case.input("y"), case.input("f"));
        let expected = &case.value["expected"];
        let gmfe = metrics::gmfe(&y, &f).unwrap();
        rows.push((format!("{name} gmfe"), gmfe, number(&expected["gmfe"])));
        let afe = metrics::afe(&y, &f).unwrap();
        rows.push((format!("{name} afe"), afe, number(&expected["afe"])));
        let agreement = metrics::subset_agreement(&y, &f).unwrap();
        match (agreement, case.output("subset_agreement").unwrap()) {
            (None, None) => {}
            (Some((bias, limit, bias_ci, limit_ci)), Some(expected)) => {
                rows.push((format!("{name} bias"), bias, expected[0]));
                rows.push((format!("{name} limit"), limit, expected[1]));
                // with the quantile of the t distribution
                distribution.push((format!("{name} bias ci"), bias_ci, expected[2]));
                distribution.push((format!("{name} limit ci"), limit_ci, expected[3]));
            }
            (found, expected) => panic!("{name}: {found:?}, the python {expected:?}"),
        }
    }
    assert_close("fold_errors", KERNEL, rows);
    assert_close("subset_agreement", DISTRIBUTION, distribution);
    assert_eq!(metrics::MIN_POINTS_SUBSET_AGREEMENT, 10);
    // nine points with a ratio are too few, ten are enough
    let (y, f) = (vec![1.0; 10], vec![2.0; 10]);
    assert_eq!(metrics::subset_agreement(&y[..9], &f[..9]), Ok(None));
    assert!(metrics::subset_agreement(&y, &f).unwrap().is_some());
    let message = "The fold criterion requires as many predictions as data points, but got \
                   '1' for '2'.";
    for result in [
        metrics::gmfe(&[1.0, 2.0], &[1.0]),
        metrics::afe(&[1.0, 2.0], &[1.0]),
        metrics::subset_agreement(&[1.0, 2.0], &[1.0]).map(|_| 0.0),
    ] {
        assert_eq!(result.unwrap_err().message, message);
    }
}

#[test]
fn the_nrmse_of_an_empty_or_not_finite_residual_is_not_a_number() {
    assert!(metrics::nrmse(&[]).is_nan());
    assert!(metrics::nrmse(&[1.0, f64::NAN]).is_nan());
    assert!(metrics::nrmse(&[1.0, f64::NEG_INFINITY]).is_nan());
    assert_eq!(metrics::nrmse(&[3.0, -4.0]), 12.5_f64.sqrt());
}

#[test]
fn the_limits_of_guest_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("metrics");
    let case = fixture.case("guest_limits");
    let (low, high): (Vec<f64>, Vec<f64>) = case
        .input("ratio")
        .into_iter()
        .map(metrics::guest_limits)
        .unzip();
    check_all("guest low", KERNEL, &low, &case.required("low"));
    check_all("guest high", KERNEL, &high, &case.required("high"));
}

#[test]
fn within_guest_is_the_one_of_sbmlsim_and_refuses_as_it_does() {
    let fixture = Fixture::load("metrics");
    let mut rows = Vec::new();
    for name in fixture.cases("within_guest/") {
        if name == "within_guest/scalar" {
            // a scalar, which only python has
            continue;
        }
        let case = fixture.case(&name);
        let found = metrics::within_guest(&case.input("observed"), &case.input("predicted"));
        check(&mut rows, name.clone(), found, &case.value["expected"]);
    }
    assert_close("within_guest", KERNEL, rows);
}

#[test]
fn the_sum_is_the_pairwise_sum_of_numpy() {
    // the squares of 1 and 1000 times 1e-8: summed in order every 1e-16 is lost against the 1
    // and the sum is 1, the pairwise sum of numpy adds the small ones in blocks first
    let mut x = vec![1.0];
    x.extend(std::iter::repeat_n(1e-8, 1000));
    // np.sum(np.square(x)) of numpy 2.5.5
    assert_eq!(metrics::sse(&x), 1.000_000_000_000_098_8);
    x.reverse();
    assert_eq!(metrics::sse(&x), 1.000_000_000_000_100_1);
}

#[test]
fn the_metrics_give_the_same_bits_on_every_target() {
    let fixture = Fixture::load("metrics");
    let mut values = Vec::new();
    let value = |result: sbmlsim_stats::Result<f64>| result.unwrap_or(-1.0);
    for name in fixture.cases("residuals/") {
        let x = fixture.case(&name).input("x");
        let mse = metrics::mse(&x);
        values.extend([
            metrics::sse(&x),
            value(mse.clone()),
            value(mse.clone().and_then(metrics::rmse_from_mse)),
            value(mse.clone().and_then(|m| metrics::aic_from_mse(m, 10, 3))),
            value(mse.and_then(|m| metrics::bic_from_mse(m, 10, 3))),
            metrics::nrmse(&x),
        ]);
    }
    for name in fixture.cases("r_squared/") {
        let case = fixture.case(&name);
        values.push(value(metrics::r_squared(
            &case.input("y"),
            &case.input("f"),
        )));
    }
    for name in fixture.cases("fold_errors/") {
        let case = fixture.case(&name);
        let (y, f) = (case.input("y"), case.input("f"));
        values.push(metrics::gmfe(&y, &f).unwrap());
        values.push(metrics::afe(&y, &f).unwrap());
        if let Some((a, b, c, d)) = metrics::subset_agreement(&y, &f).unwrap() {
            values.extend([a, b, c, d]);
        }
    }
    for ratio in fixture.case("guest_limits").input("ratio") {
        let (low, high) = metrics::guest_limits(ratio);
        values.extend([low, high]);
    }
    for name in fixture.cases("within_guest/") {
        if name == "within_guest/scalar" {
            continue;
        }
        let case = fixture.case(&name);
        values.push(value(metrics::within_guest(
            &case.input("observed"),
            &case.input("predicted"),
        )));
    }
    assert_eq!(
        hash(&values),
        0x41c9_87a6_d6ae_0ae1,
        "{} values",
        values.len()
    );
}

// --- fisher and posterior ---------------------------------------------------------------------

/// The scales of a case by the names of `ParameterScaleType`.
fn scales(names: &[String]) -> Vec<Scale> {
    names
        .iter()
        .map(|name| match name.as_str() {
            "LINEAR" => Scale::Linear,
            "LOG" => Scale::Log,
            "LOG10" => Scale::Log10,
            _ => panic!("no scale {name}"),
        })
        .collect()
}

/// Holds every value of a `k x k` matrix found to the expected one at `tolerance` relative to
/// `sqrt(|e_ii e_jj|)` (the bound of a covariance), not a number to not a number and an
/// infinity to the same one; returns the largest error.
fn assert_matrix_close(
    name: &str,
    tolerance: f64,
    k: usize,
    found: &[f64],
    expected: &[f64],
) -> f64 {
    assert_eq!(found.len(), k * k, "{name}: the number of values");
    assert_eq!(
        expected.len(),
        k * k,
        "{name}: the number of expected values"
    );
    let mut worst = 0.0_f64;
    let mut failures = Vec::new();
    for i in 0..k {
        for j in 0..k {
            let (f, e) = (found[i * k + j], expected[i * k + j]);
            if f == e || (f.is_nan() && e.is_nan()) {
                continue;
            }
            let scale = expected[i * k + i].abs().sqrt() * expected[j * k + j].abs().sqrt();
            let error = (f - e).abs() / scale;
            worst = worst.max(error);
            if error.is_nan() || error > tolerance {
                failures.push(format!("{name}[{i}, {j}] = {f:e}, expected {e:e}"));
            }
        }
    }
    println!("{name}: largest error {worst:.3e} relative to sqrt(e_ii e_jj)");
    assert!(
        failures.is_empty(),
        "{} values of {name} off by more than {tolerance:e}:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
    worst
}

/// The rows of `assert_close` of a list of values found and expected.
fn rows(name: &str, found: &[f64], expected: &[f64]) -> Vec<(String, f64, f64)> {
    assert_eq!(found.len(), expected.len(), "{name}: the number of values");
    found
        .iter()
        .zip(expected)
        .enumerate()
        .map(|(i, (&f, &e))| (format!("{name}[{i}]"), f, e))
        .collect()
}

/// The statistics of the Fisher information of a case of `fisher.json`.
fn statistics_of(case: &Case<'_>) -> fisher::FisherStatistics {
    let k = case.input_usize("k");
    fisher::statistics(
        &case.input("information"),
        k,
        case.input_f64("cost"),
        case.input_usize("n"),
        case.input_bool("likelihood"),
        fisher::DEFAULT_RANK_TOLERANCE,
    )
    .unwrap_or_else(|e| panic!("{}: {e}", case.name))
}

#[test]
fn the_statistics_of_the_fisher_information_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("fisher");
    for name in fixture.cases("fisher/") {
        let case = fixture.case(&name);
        let k = case.input_usize("k");
        let found = statistics_of(&case);
        assert_close(
            &format!("{name} sigma2"),
            KERNEL,
            vec![(String::new(), found.sigma2, case.expected_f64("sigma2"))],
        );
        let spectrum = case.required("spectrum");
        assert_close_to(
            &format!("{name} spectrum"),
            KERNEL,
            largest(&spectrum),
            &found.spectrum,
            &spectrum,
        );
        assert_eq!(
            found.rank as f64,
            case.expected_f64("rank"),
            "{name}: the rank"
        );
        let condition = case.expected_f64("condition");
        let (cov, se, corr) = (
            case.required("covariance"),
            case.required("standard_errors"),
            case.required("correlation"),
        );
        // the nan of an error is the python's, in every case
        for (i, (f, e)) in found.standard_errors.iter().zip(&se).enumerate() {
            assert_eq!(
                f.is_nan(),
                e.is_nan(),
                "{name}: the standard error {i}, {f}"
            );
        }
        if found.rank < k {
            // the smallest eigenvalue is the noise of the rounding: both condition numbers
            // say rank deficient, infinite of an eigenvalue of zero
            assert!(
                condition >= 1e8 && found.condition >= 1e8,
                "{name}: {condition}, {}",
                found.condition
            );
            if spectrum[k - 1] == 0.0 {
                assert!(found.condition.is_infinite(), "{name}: {}", found.condition);
            }
        } else {
            assert_close(
                &format!("{name} condition"),
                KERNEL,
                vec![(String::new(), found.condition, condition)],
            );
        }
        assert_matrix_close(
            &format!("{name} covariance"),
            KERNEL,
            k,
            &found.covariance,
            &cov,
        );
        assert_close(
            &format!("{name} standard errors"),
            KERNEL,
            rows(&name, &found.standard_errors, &se),
        );
        assert_close_to(
            &format!("{name} correlation"),
            KERNEL,
            1.0,
            &found
                .correlation
                .iter()
                .map(|v| if v.is_nan() { 0.0 } else { *v })
                .collect::<Vec<_>>(),
            &corr
                .iter()
                .map(|v| if v.is_nan() { 0.0 } else { *v })
                .collect::<Vec<_>>(),
        );
        for (i, (f, e)) in found.correlation.iter().zip(&corr).enumerate() {
            assert_eq!(f.is_nan(), e.is_nan(), "{name}: the correlation {i}, {f}");
        }
    }
}

/// The jacobian of `large_jacobian` of `oracles/make_oracles.py`, row-major, bit for bit.
fn large_jacobian(rows: usize, k: usize) -> Vec<f64> {
    (0..(rows * k) as i64)
        .map(|x| ((((x * x) % 1_000_003 * 2_654_435_761 + x) % 1_000_003) % 19 - 9) as f64)
        .collect()
}

/// The statistics of the case `fisher_large/200` from its jacobian.
fn large_statistics(case: &Case<'_>) -> fisher::FisherStatistics {
    let (rows, k) = (case.input_usize("rows"), case.input_usize("k"));
    let information = linalg::gram(&large_jacobian(rows, k), rows, k).unwrap();
    fisher::statistics(
        &information,
        k,
        case.input_f64("cost"),
        case.input_usize("n"),
        false,
        fisher::DEFAULT_RANK_TOLERANCE,
    )
    .unwrap()
}

#[test]
fn the_statistics_of_an_information_of_200_parameters_are_the_ones_of_sbmlsim() {
    // the information of integers is exactly numpy's, the statistics compose the
    // decomposition of a 200 x 200 matrix
    let fixture = Fixture::load("fisher");
    let case = fixture.case("fisher_large/200");
    let k = case.input_usize("k");
    let found = large_statistics(&case);
    assert_eq!(found.rank as f64, case.expected_f64("rank"));
    let scalars = vec![
        (
            "sigma2".to_string(),
            found.sigma2,
            case.expected_f64("sigma2"),
        ),
        (
            "condition".to_string(),
            found.condition,
            case.expected_f64("condition"),
        ),
    ];
    assert_close("fisher_large/200", KERNEL, scalars);
    let spectrum = case.required("spectrum");
    assert_close_to(
        "fisher_large/200 spectrum",
        KERNEL,
        largest(&spectrum),
        &found.spectrum,
        &spectrum,
    );
    let se = case.required("standard_errors");
    assert_close(
        "fisher_large/200 standard errors",
        KERNEL,
        rows("se", &found.standard_errors, &se),
    );
    let (covariance, correlation) = (
        case.required("covariance_rows"),
        case.required("correlation_rows"),
    );
    for (r, row) in case.input_u64("rows_stored").into_iter().enumerate() {
        let row = usize::try_from(row).unwrap();
        for j in 0..k {
            // relative to sqrt(cov_ii cov_jj), the variances of the standard errors
            let scale = se[row] * se[j];
            let (f, e) = (found.covariance[row * k + j], covariance[r * k + j]);
            assert!(
                (f - e).abs() <= KERNEL * scale,
                "covariance[{row}, {j}] = {f:e}, expected {e:e}"
            );
            let (f, e) = (found.correlation[row * k + j], correlation[r * k + j]);
            assert!(
                (f - e).abs() <= KERNEL,
                "correlation[{row}, {j}] = {f:e}, expected {e:e}"
            );
        }
    }
}

#[test]
fn the_intervals_and_relative_errors_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("fisher");
    for name in fixture.cases("fisher/") {
        let case = fixture.case(&name);
        let (values, scales) = (case.input("values"), scales(&case.input_strings("scales")));
        let se = case.required("standard_errors");
        let intervals = fisher::intervals(
            &values,
            &se,
            &scales,
            case.input_f64("alpha"),
            case.input_option("dof"),
        )
        .unwrap();
        let (lower, upper): (Vec<f64>, Vec<f64>) = intervals.into_iter().unzip();
        assert_close(
            &format!("{name} lower"),
            KERNEL,
            rows(&name, &lower, &case.required("lower")),
        );
        assert_close(
            &format!("{name} upper"),
            KERNEL,
            rows(&name, &upper, &case.required("upper")),
        );
        let relative = fisher::relative_errors(&values, &se, &scales).unwrap();
        assert_close(
            &format!("{name} relative errors"),
            KERNEL,
            rows(&name, &relative, &case.required("relative_errors")),
        );
    }
}

#[test]
fn a_singular_information_has_no_errors_of_its_null_space_and_does_not_panic() {
    // a parameter the data do not depend on: rank 1 of 2, the condition infinite, the error
    // of the parameter not a number and its correlations not a number
    let s = fisher::statistics(&[4.0, 0.0, 0.0, 0.0], 2, 3.0, 12, false, 1e-8).unwrap();
    assert_eq!(s.rank, 1);
    assert_eq!(s.condition, f64::INFINITY);
    assert_eq!(s.spectrum, vec![4.0, 0.0]);
    assert_eq!(s.sigma2, 0.6);
    assert_eq!(s.covariance, vec![0.15, 0.0, 0.0, 0.0]);
    assert_eq!(s.standard_errors[0], 0.15_f64.sqrt());
    assert!(s.standard_errors[1].is_nan());
    // 0.15 / (sqrt(0.15) sqrt(0.15)), as numpy divides
    assert_eq!(s.correlation[0], 0.15 / (0.15_f64.sqrt() * 0.15_f64.sqrt()));
    assert!(s.correlation[1..].iter().all(|v| v.is_nan()));
    // no information at all: rank 0, every error not a number
    let s = fisher::statistics(&[0.0; 9], 3, 3.0, 12, true, 1e-8).unwrap();
    assert_eq!((s.rank, s.condition, s.sigma2), (0, f64::INFINITY, 1.0));
    assert_eq!(s.covariance, vec![0.0; 9]);
    assert!(s.standard_errors.iter().all(|v| v.is_nan()));
    // a perfect identifiable fit has errors of zero
    let s = fisher::statistics(&[2.0, 1.0, 1.0, 3.0], 2, 0.0, 12, false, 1e-8).unwrap();
    assert_eq!(s.standard_errors, vec![0.0, 0.0]);
    // without degrees of freedom the variance is not a number, for a likelihood one
    let s = fisher::statistics(&[2.0, 1.0, 1.0, 3.0], 2, 1.0, 2, false, 1e-8).unwrap();
    assert!(s.sigma2.is_nan() && s.standard_errors.iter().all(|v| v.is_nan()));
    let s = fisher::statistics(&[2.0, 1.0, 1.0, 3.0], 2, 1.0, 1, true, 1e-8).unwrap();
    assert_eq!(s.sigma2, 1.0);
    // a tolerance decides the rank, not the pseudo-inverse
    let s = fisher::statistics(&[1.0, 0.0, 0.0, 1e-9], 2, 1.0, 12, true, 1e-8).unwrap();
    assert_eq!((s.rank, s.condition), (1, 1.0 / 1e-9));
    assert_eq!(s.standard_errors[1], (1.0 / 1e-9_f64).sqrt());
}

#[test]
fn an_information_which_is_not_a_finite_symmetric_matrix_is_an_error() {
    let error = fisher::statistics(&[1.0, f64::NAN, f64::NAN, 1.0], 2, 1.0, 5, false, 1e-8);
    assert!(error.unwrap_err().message.contains("not finite"));
    let error = fisher::statistics(&[1.0, 2.0, 3.0], 2, 1.0, 5, false, 1e-8);
    assert!(error.unwrap_err().message.contains("needs 4 values"));
    let error = fisher::statistics(&[], 0, 1.0, 5, false, 1e-8);
    assert!(error.unwrap_err().message.contains("no parameters"));
}

#[test]
fn the_intervals_without_degrees_of_freedom_are_not_a_number() {
    let scales = [Scale::Linear, Scale::Log, Scale::Log10];
    let values = [1.0, 2.0, 3.0];
    let se = [0.1, 0.2, 0.3];
    for dof in [0.0, -3.0] {
        let intervals = fisher::intervals(&values, &se, &scales, 0.95, Some(dof)).unwrap();
        assert!(intervals.iter().all(|(l, u)| l.is_nan() && u.is_nan()));
    }
    // of the normal distribution for a likelihood, the t distribution otherwise
    let normal = fisher::intervals(&values, &se, &scales, 0.95, None).unwrap();
    let t = fisher::intervals(&values, &se, &scales, 0.95, Some(1e30)).unwrap();
    assert_eq!(normal, t);
    let z = special::normal_ppf(0.975);
    assert_eq!(normal[0], (1.0 - z * 0.1, 1.0 + z * 0.1));
    assert_eq!(
        fisher::relative_errors(&values, &se, &scales).unwrap()[2],
        std::f64::consts::LN_10 * 0.3
    );
}

#[test]
fn the_laplace_approximation_is_the_one_of_sbmlsim_and_names_its_weak_directions() {
    let fixture = Fixture::load("fisher");
    for name in fixture.cases("posterior/") {
        let case = fixture.case(&name);
        let k = case.input_usize("k");
        let precision = case.input("precision");
        let tolerance = case.input_f64("rank_tolerance");
        let pids: Vec<String> = case
            .input_strings("pids")
            .iter()
            .map(|pid| format!("'{pid}'"))
            .collect();
        let named = posterior::laplace_of(
            &precision,
            k,
            tolerance,
            Some(case.input_str("opid")),
            &pids,
        );
        if let Some(message) = case.error() {
            assert_eq!(named.unwrap_err().message, message, "{name}");
            // the message of the indices
            let indexed = posterior::laplace(&precision, k, tolerance)
                .unwrap_err()
                .message;
            assert!(
                indexed.starts_with("the data and the priors do not constrain"),
                "{indexed}"
            );
            continue;
        }
        let covariance = named.unwrap();
        assert_eq!(
            covariance,
            posterior::laplace(&precision, k, tolerance).unwrap()
        );
        assert_matrix_close(
            &format!("{name} covariance"),
            KERNEL,
            k,
            &covariance,
            &case.required("covariance"),
        );
        // the kernels of the posterior on its python correlation
        let r = case.required("correlation");
        assert_close_to(
            &format!("{name} rank correlation"),
            KERNEL,
            1.0,
            &posterior::rank_correlation(&r, k).unwrap(),
            &case.required("rank_correlation"),
        );
        assert_close_to(
            &format!("{name} normal correlation"),
            KERNEL,
            1.0,
            &posterior::normal_correlation(&r, k).unwrap(),
            &case.required("normal_correlation"),
        );
        let sd = case.required("standard_deviations");
        let scales = scales(&case.input_strings("scales"));
        let log_sd: Vec<f64> = scales
            .iter()
            .zip(&sd)
            .map(|(&s, &sd)| posterior::log_sd(s, sd))
            .collect();
        assert_close(
            &format!("{name} log sd"),
            KERNEL,
            rows(&name, &log_sd, &case.required("log_sd")),
        );
        let cv: Vec<f64> = scales
            .iter()
            .zip(&log_sd)
            .map(|(&s, &sigma)| {
                if s == Scale::Linear {
                    f64::NAN
                } else {
                    posterior::lognormal_cv(sigma)
                }
            })
            .collect();
        assert_close(
            &format!("{name} cv"),
            KERNEL,
            rows(&name, &cv, &case.required("cv")),
        );
        let values = case.input("values");
        for (level, label) in [(0.95, "95"), (0.5, "50")] {
            let (lower, upper): (Vec<f64>, Vec<f64>) =
                fisher::intervals(&values, &sd, &scales, level, None)
                    .unwrap()
                    .into_iter()
                    .unzip();
            assert_close(
                &format!("{name} lower{label}"),
                KERNEL,
                rows(&name, &lower, &case.required(&format!("lower{label}"))),
            );
            assert_close(
                &format!("{name} upper{label}"),
                KERNEL,
                rows(&name, &upper, &case.required(&format!("upper{label}"))),
            );
        }
    }
}

#[test]
fn the_weak_directions_are_named_by_their_indices_without_names() {
    // the data tell p1 and p2 only together, of the null vector (1, -2) / sqrt(5): the
    // larger component first
    let information = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let precision = [1.0, 0.0, 0.0, 0.0, 4.0, 2.0, 0.0, 2.0, 1.0];
    let error = posterior::laplace(&precision, 3, 1e-8).unwrap_err().message;
    assert_eq!(
        error,
        "the data and the priors do not constrain 1 direction(s) of the parameters, of \
         [[2, 1]]; the posterior is the uniform prior of the bounds there, which the Laplace \
         approximation cannot describe. Give these parameters priors and fit the objective \
         'posterior', or see the profile likelihood."
    );
    let names = ["'a'".to_string(), "'b'".to_string()];
    let error = posterior::laplace_of(&information, 3, 1e-8, Some("op"), &names).unwrap_err();
    assert!(error.message.contains("2 names"), "{error}");
    // the error of no parameters is numpy's
    assert_eq!(
        posterior::laplace(&[], 0, 1e-8).unwrap_err().message,
        "zero-size array to reduction operation maximum which has no identity"
    );
    assert!(posterior::laplace(&[1.0, 2.0], 2, 1e-8).is_err());
    assert!(posterior::laplace(&[1.0, f64::NAN, f64::NAN, 1.0], 2, 1e-8).is_err());
}

#[test]
fn the_rank_correlation_clips_and_propagates_not_a_number_as_sbmlsim() {
    let fixture = Fixture::load("fisher");
    for name in fixture.cases("rank_correlation/") {
        let case = fixture.case(&name);
        let found = posterior::rank_correlation(&case.input("r"), case.input_usize("k")).unwrap();
        assert_close(&name, KERNEL, rows(&name, &found, &case.expected()));
    }
    assert_eq!(posterior::lognormal_cv(0.0), 0.0);
    assert_eq!(
        posterior::log_sd(Scale::Log10, 1.0),
        std::f64::consts::LN_10
    );
    assert_eq!(posterior::log_sd(Scale::Linear, 0.5), 0.5);
}

#[test]
fn the_fisher_statistics_and_the_posterior_give_the_same_bits_on_every_target() {
    let fixture = Fixture::load("fisher");
    let mut values = Vec::new();
    for name in fixture.cases("fisher/") {
        let case = fixture.case(&name);
        let s = statistics_of(&case);
        values.extend([s.sigma2, s.rank as f64, s.condition]);
        values.extend(s.spectrum);
        values.extend(s.covariance);
        values.extend(s.correlation);
        let (v, scales) = (case.input("values"), scales(&case.input_strings("scales")));
        for (l, u) in fisher::intervals(
            &v,
            &s.standard_errors,
            &scales,
            0.95,
            case.input_option("dof"),
        )
        .unwrap()
        {
            values.extend([l, u]);
        }
        values.extend(fisher::relative_errors(&v, &s.standard_errors, &scales).unwrap());
        values.extend(s.standard_errors);
    }
    for name in fixture.cases("posterior/") {
        let case = fixture.case(&name);
        let k = case.input_usize("k");
        if let Ok(covariance) = posterior::laplace(&case.input("precision"), k, 1e-8) {
            values.extend(covariance);
            let r = case.required("correlation");
            values.extend(posterior::rank_correlation(&r, k).unwrap());
            values.extend(posterior::normal_correlation(&r, k).unwrap());
        }
    }
    for sigma in [0.0, 1e-8, 0.3, 1.0, 2.5] {
        values.push(posterior::lognormal_cv(sigma));
    }
    let s = large_statistics(&fixture.case("fisher_large/200"));
    values.extend([s.sigma2, s.condition]);
    values.extend(s.spectrum);
    values.extend(s.covariance);
    values.extend(s.standard_errors);
    assert_eq!(
        hash(&values),
        0x4710_98c5_1004_b448,
        "{} values",
        values.len()
    );
}

// --- uncertainty ------------------------------------------------------------------------------

/// The outcome of a call of the python with several values: the numbers (a boolean as 0 or
/// 1, an integer as its double), or the message of the error it raised
/// (`{"error": <message>, "exception": <class>}`).
fn values_of(value: &Value) -> std::result::Result<Vec<f64>, &str> {
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
fn check_values(
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

/// The probabilities of the quantiles and of their intervals of every input of the
/// statistics, `QUANTILE_QS` of `oracles/make_oracles.py`.
const QUANTILE_QS: [f64; 3] = [0.05, 0.5, 0.95];
/// The probabilities of the quantiles of every input, `QUANTILES_QS`.
const QUANTILES_QS: [f64; 6] = [0.0, 0.05, 0.25, 0.5, 0.95, 1.0];

/// The draws of a case of the statistics: its `x`, or the formula of `large_residuals` of
/// its `n`.
fn draws_of(case: &Case<'_>) -> Vec<f64> {
    if case.value["inputs"].get("x").is_some() {
        return case.input("x");
    }
    let x = large_residuals(case.input_usize("n"));
    assert_eq!(
        x[..4],
        case.input("first")[..],
        "{}: the formula",
        case.name
    );
    x
}

/// The mean and its error, the standard deviation and its error, the quantiles with their
/// intervals and the quantiles of the draws, as the cases of `statistics/` hold them.
fn statistics_rows(rows: &mut Vec<(String, f64, f64)>, name: &str, case: &Case<'_>) {
    let x = draws_of(case);
    let expected = &case.value["expected"];
    let (mean, se, n) = uncertainty::mean_se(&x);
    check_values(
        rows,
        &format!("{name} mean_se"),
        Ok(vec![mean, se, n as f64]),
        &expected["mean_se"],
    );
    let sd = uncertainty::sd_se(&x).map(|(sd, se, n)| vec![sd, se, n as f64]);
    check_values(rows, &format!("{name} sd_se"), sd, &expected["sd_se"]);
    for (index, q) in QUANTILE_QS.into_iter().enumerate() {
        let found = uncertainty::quantile_ci(&x, q, 0.95)
            .map(|(estimate, low, high, n)| vec![estimate, low, high, n as f64]);
        check_values(
            rows,
            &format!("{name} quantile_ci(q={q})"),
            found,
            &expected["quantile_ci"][index],
        );
    }
    check_values(
        rows,
        &format!("{name} quantiles"),
        uncertainty::quantiles(&x, &QUANTILES_QS),
        &expected["quantiles"],
    );
    for (index, q) in QUANTILES_QS.into_iter().enumerate() {
        let found = uncertainty::quantile(&x, q).map(|value| vec![value]);
        check_values(
            rows,
            &format!("{name} quantile(q={q})"),
            found,
            &expected["quantiles"][index],
        );
    }
}

#[test]
fn the_statistics_of_draws_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("uncertainty");
    let mut rows = Vec::new();
    for name in fixture.cases("statistics") {
        statistics_rows(&mut rows, &name, &fixture.case(&name));
    }
    assert_close("statistics", KERNEL, rows);
}

#[test]
fn the_mean_of_many_draws_is_the_pairwise_sum_of_numpy_bit_for_bit() {
    let fixture = Fixture::load("uncertainty");
    for name in fixture.cases("statistics_formula/") {
        let case = fixture.case(&name);
        let x = draws_of(&case);
        let expected = values_of(&case.value["expected"]["mean_se"]).unwrap();
        let (mean, se, n) = uncertainty::mean_se(&x);
        assert_eq!([mean, se, n as f64], expected[..], "{name}");
        let expected = values_of(&case.value["expected"]["sd_se"]).unwrap();
        assert_eq!(uncertainty::sd_se(&x).unwrap().0, expected[0], "{name}");
    }
}

#[test]
fn quantile_ci_refuses_and_returns_nan_as_sbmlsim() {
    // the probabilities 0 and 1, the levels 0, 1 and -1, and the arguments numpy (the
    // probability) and scipy (a level outside [-1, 1], a rank of not a number) refuse
    let fixture = Fixture::load("uncertainty");
    let mut rows = Vec::new();
    for name in fixture.cases("quantile_ci/") {
        let case = fixture.case(&name);
        let found = uncertainty::quantile_ci(
            &case.input("x"),
            case.input_f64("q"),
            case.input_f64("level"),
        )
        .map(|(estimate, low, high, n)| vec![estimate, low, high, n as f64]);
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    for name in fixture.cases("quantiles/") {
        let case = fixture.case(&name);
        let found = uncertainty::quantiles(&case.input("x"), &case.input("qs"));
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    assert_close("quantile_ci", KERNEL, rows);
}

/// The input and the output of `paired_draws` of `oracles/make_oracles.py`, bit for bit:
/// `x = large_residuals(n)`, `y = x * x + large_residuals(n + 13)[13..]`.
fn paired_draws(n: usize) -> (Vec<f64>, Vec<f64>) {
    let x = large_residuals(n);
    let shifted = large_residuals(n + 13);
    let y = x
        .iter()
        .zip(&shifted[13..])
        .map(|(x, s)| x * x + s)
        .collect();
    (x, y)
}

#[test]
fn the_first_order_index_from_given_data_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("uncertainty");
    let mut rows = Vec::new();
    for name in fixture.cases("first_order/") {
        let case = fixture.case(&name);
        let n_bins = case.input_option("n_bins").map(|bins| bins as usize);
        let found = uncertainty::first_order_given_data(&case.input("x"), &case.input("y"), n_bins)
            .map(|index| vec![index]);
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    for name in fixture.cases("first_order_formula/") {
        let case = fixture.case(&name);
        let (x, y) = paired_draws(case.input_usize("n"));
        assert_eq!(x[..4], case.input("first_x")[..], "{name}: the formula");
        assert_eq!(y[..4], case.input("first_y")[..], "{name}: the formula");
        let found = uncertainty::first_order_given_data(&x, &y, None).map(|index| vec![index]);
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    assert_close("first_order_given_data", KERNEL, rows);
}

#[test]
fn the_precision_of_an_output_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("uncertainty");
    let mut rows = Vec::new();
    let as_values = |result: sbmlsim_stats::Result<(bool, f64)>| {
        result.map(|(ok, w)| vec![f64::from(u8::from(ok)), w])
    };
    for name in fixture.cases("precise/") {
        let case = fixture.case(&name);
        let found = uncertainty::precise(&case.input("values"), case.input_f64("rel_tol"));
        check_values(&mut rows, &name, as_values(found), &case.value["expected"]);
    }
    for name in fixture.cases("precise_formula/") {
        let case = fixture.case(&name);
        let values = large_residuals(case.input_usize("n"));
        let found = uncertainty::precise(&values, case.input_f64("rel_tol"));
        check_values(&mut rows, &name, as_values(found), &case.value["expected"]);
    }
    assert_close("precise", KERNEL, rows);
}

#[test]
fn the_parameters_of_an_interval_are_the_ones_of_uncertain_interval() {
    // the median and the coefficient of variation of a log-normal, the mean and the standard
    // deviation of a normal, and the errors of python, numpy and the distributions
    let fixture = Fixture::load("uncertainty");
    let mut rows = Vec::new();
    for name in fixture.cases("interval/") {
        let case = fixture.case(&name);
        let found = uncertainty::interval_parameters(
            case.input_f64("low"),
            case.input_f64("high"),
            case.input_f64("level"),
            case.input_bool("lognormal"),
        )
        .map(|(center, spread)| vec![center, spread]);
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    // the quantile of the normal distribution is AS 241, not scipy's ndtri
    assert_close("interval_parameters", NORMAL, rows);
}

#[test]
fn the_fixtures_hold_every_class_of_exception_the_kinds_are_held_to() {
    // `check_error` holds the kind of every error to the class of the exception of the python,
    // and the fixtures of the uncertainty, the testing and the draws raise every class
    fn count<'a>(value: &'a Value, classes: &mut BTreeMap<&'a str, usize>) {
        match value {
            Value::Object(object) => {
                if let Some(class) = object.get("exception") {
                    *classes.entry(class.as_str().unwrap()).or_default() += 1;
                }
                object.values().for_each(|value| count(value, classes));
            }
            Value::Array(items) => items.iter().for_each(|value| count(value, classes)),
            _ => {}
        }
    }
    let fixtures = ["uncertainty", "testing", "draws"].map(Fixture::load);
    let mut classes = BTreeMap::new();
    for fixture in &fixtures {
        count(&fixture.value, &mut classes);
    }
    for class in [
        "ValueError",
        "OverflowError",
        "ZeroDivisionError",
        "IndexError",
    ] {
        assert!(classes.contains_key(class), "{class}: {classes:?}");
    }
    assert!(classes.values().sum::<usize>() >= 40, "{classes:?}");
}

#[test]
fn the_statistics_of_the_uncertainty_give_the_same_bits_on_every_target() {
    let fixture = Fixture::load("uncertainty");
    let mut values = Vec::new();
    let value = |result: sbmlsim_stats::Result<f64>| result.unwrap_or(-1.0);
    for name in fixture.cases("statistics") {
        let x = draws_of(&fixture.case(&name));
        let (mean, se, _) = uncertainty::mean_se(&x);
        values.extend([mean, se]);
        if let Ok((sd, se, _)) = uncertainty::sd_se(&x) {
            values.extend([sd, se]);
        }
        for q in QUANTILE_QS {
            let (estimate, low, high, _) = uncertainty::quantile_ci(&x, q, 0.95).unwrap();
            values.extend([estimate, low, high]);
        }
        values.extend(uncertainty::quantiles(&x, &QUANTILES_QS).unwrap());
    }
    for name in fixture.cases("first_order/") {
        let case = fixture.case(&name);
        let n_bins = case.input_option("n_bins").map(|bins| bins as usize);
        values.push(value(uncertainty::first_order_given_data(
            &case.input("x"),
            &case.input("y"),
            n_bins,
        )));
    }
    for n in [1000, 100_003] {
        let (x, y) = paired_draws(n);
        values.push(value(uncertainty::first_order_given_data(&x, &y, None)));
        values.push(value(uncertainty::precise(&x, 0.1).map(|(_, w)| w)));
    }
    for name in fixture.cases("interval/") {
        let case = fixture.case(&name);
        if let Ok((center, spread)) = uncertainty::interval_parameters(
            case.input_f64("low"),
            case.input_f64("high"),
            case.input_f64("level"),
            case.input_bool("lognormal"),
        ) {
            values.extend([center, spread]);
        }
    }
    assert_eq!(
        hash(&values),
        0xeaf6_ec6b_801b_ae21,
        "{} values",
        values.len()
    );
}

#[test]
fn more_bins_than_pairs_are_not_a_number_without_a_loop_over_the_bins() {
    // python cannot split into so many bins (a MemoryError of the list of their sizes); the
    // crate returns the value of any number of bins past the pairs at once
    let x: Vec<f64> = (0..10).map(f64::from).collect();
    let y: Vec<f64> = x.iter().map(|x| x * x).collect();
    let index = uncertainty::first_order_given_data(&x, &y, Some(usize::MAX));
    assert!(index.unwrap().is_nan());
    // the square of a bin past the largest double is refused before an empty bin
    let huge: Vec<f64> = x.iter().map(|x| x * 1e300).collect();
    let error = uncertainty::first_order_given_data(&x, &huge, Some(1000)).unwrap_err();
    assert_eq!(error.message, "(34, 'Numerical result out of range')");
}

// --- draws and correlation --------------------------------------------------------------------

/// The matrix of `large_correlation` of `oracles/make_oracles.py`, of a unit diagonal and the
/// coefficients `(((i + j) * 7 + i * j) % 19 - 9) / 10`, which are not positive semi-definite.
fn large_correlation(k: usize) -> Vec<f64> {
    let mut r = vec![0.0; k * k];
    for i in 0..k {
        for j in 0..k {
            r[i * k + j] = if i == j {
                1.0
            } else {
                ((((i + j) * 7 + i * j) % 19) as f64 - 9.0) / 10.0
            };
        }
    }
    r
}

/// The matrix of a case of `nearest/`: its input `m`, or the formula of `large_correlation`.
fn nearest_input(case: &Case<'_>) -> Vec<f64> {
    let k = case.input_usize("k");
    if case.value["inputs"].get("m").is_some() {
        case.input("m")
    } else {
        large_correlation(k)
    }
}

/// The nearest correlation of a case of `nearest/`.
fn nearest_of(case: &Case<'_>) -> Vec<f64> {
    correlation::nearest_correlation(
        &nearest_input(case),
        case.input_usize("k"),
        case.input_usize("max_iterations"),
        case.input_f64("tol"),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", case.name))
}

/// The tolerance of the nearest correlation against numpy, relative to the largest value of the
/// matrix: the eigendecompositions of the iterations are Jacobi's, not LAPACK's.
const NEAREST: f64 = 1e-12;

#[test]
fn the_nearest_correlation_is_the_one_of_higham_of_sbmlsim() {
    let fixture = Fixture::load("draws");
    let mut rows = Vec::new();
    for name in fixture.cases("nearest/") {
        let case = fixture.case(&name);
        let found = nearest_of(&case);
        let expected = case.expected();
        assert_eq!(found.len(), expected.len(), "{name}");
        // the off-diagonal zeros of numpy are rounding of the order of the largest value, 1
        for (index, (found, expected)) in found.into_iter().zip(expected).enumerate() {
            let (found, expected) = if expected.is_finite() && expected.abs() < 1e-3 {
                (found + 1.0, expected + 1.0)
            } else {
                (found, expected)
            };
            rows.push((format!("{name}[{index}]"), found, expected));
        }
    }
    assert_close("nearest_correlation", NEAREST, rows);
}

#[test]
#[ignore = "about 15 s in release, minutes in debug and in wasm: run it with `cargo test -p \
            sbmlsim-stats --release -- --ignored`; the binding test of python/tests/test_stats.py \
            runs the same case on every wheel"]
fn the_nearest_correlation_of_200_targets_is_the_one_of_sbmlsim() {
    // the 200 x 200 matrix of the formula, 100 iterations of eigendecompositions: every 97th
    // value and the sums of the rows against numpy, at 1e-12 of the largest value of the
    // matrix, 1 (Jacobi and LAPACK round differently, an entry of 1e-3 differs by a few 1e-12
    // of itself)
    let fixture = Fixture::load("draws");
    let case = fixture.case("nearest_large/200");
    let k = case.input_usize("k");
    let found = correlation::nearest_correlation(
        &large_correlation(k),
        k,
        case.input_usize("max_iterations"),
        case.input_f64("tol"),
    )
    .unwrap();
    let stride = case.input_usize("stride");
    let strided: Vec<f64> = found.iter().step_by(stride).copied().collect();
    let row_sums: Vec<f64> = found.chunks(k).map(|row| row.iter().sum()).collect();
    let mut worst = 0.0_f64;
    for (what, found) in [("strided", strided), ("row_sums", row_sums)] {
        let expected = numbers(&case.value["expected"][what]);
        assert_eq!(found.len(), expected.len(), "{what}");
        for (index, (found, expected)) in found.iter().zip(&expected).enumerate() {
            let error = (found - expected).abs();
            worst = worst.max(error);
            assert!(
                error <= 1e-12,
                "{what}[{index}] = {found:e}, expected {expected:e}"
            );
        }
    }
    println!("nearest_correlation of 200: largest absolute error {worst:.3e}");
    assert_eq!(
        hash(&found),
        0x8af4_f179_594f_93f9,
        "the bits of 200 x 200 values"
    );
}

#[test]
fn the_nearest_correlation_is_positive_semi_definite_with_a_unit_diagonal() {
    let k = 50;
    let r = correlation::nearest_correlation(&large_correlation(k), k, 100, 1e-10).unwrap();
    assert!(correlation::is_psd(&r, k, correlation::PSD_RTOL));
    for i in 0..k {
        assert_eq!(r[i * k + i], 1.0);
        for j in 0..k {
            assert_eq!(r[i * k + j], r[j * k + i]);
        }
    }
    // a slice of another size is refused
    assert!(correlation::nearest_correlation(&[1.0, 0.0, 0.0], 2, 100, 1e-10).is_err());
}

#[test]
fn is_psd_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("draws");
    assert_eq!(
        fixture.case("psd_rtol").value["expected"].as_f64().unwrap(),
        correlation::PSD_RTOL
    );
    for name in fixture.cases("is_psd/") {
        let case = fixture.case(&name);
        let found = correlation::is_psd(
            &case.input("m"),
            case.input_usize("k"),
            case.input_f64("rtol"),
        );
        assert_eq!(found, case.value["expected"].as_bool().unwrap(), "{name}");
    }
}

#[test]
fn the_validation_of_a_matrix_is_the_one_of_covariance_with_its_messages() {
    let fixture = Fixture::load("draws");
    for name in fixture.cases("validate/") {
        let case = fixture.case(&name);
        let targets = case.input_strings("targets");
        let targets: Vec<&str> = targets.iter().map(String::as_str).collect();
        let shape: Vec<usize> = case
            .input_u64("shape")
            .into_iter()
            .map(|n| usize::try_from(n).unwrap())
            .collect();
        let units = case.value["inputs"]["units"]
            .as_u64()
            .map(|n| usize::try_from(n).unwrap());
        let found = correlation::validate(
            &case.input("matrix"),
            &shape,
            &targets,
            case.input_str("kind") == "correlation",
            units,
        );
        match (&found, case.error()) {
            (Ok(()), None) => {}
            (Err(error), Some(_)) => check_error(error, &case.value["expected"], &name),
            _ => panic!("{name}: {found:?}, the python {:?}", case.error()),
        }
    }
}

/// The marginal of a case of `rho/`, as `_moments` gives it, on the scale of the covariance.
fn marginal(value: &Value, scale: &str) -> Marginal {
    let family = value[0].as_str().unwrap();
    let (mean, sd) = (number(&value[1]), number(&value[2]));
    match (family, scale) {
        ("normal", _) => Marginal::Normal { sd },
        (_, "log") => Marginal::Log { sd },
        _ => Marginal::LogNormal { mean, sigma: sd },
    }
}

#[test]
fn the_coefficient_of_the_normal_scores_is_the_one_of_normal_score_correlation() {
    let fixture = Fixture::load("draws");
    let mut rows = Vec::new();
    for name in fixture.cases("rho/") {
        let case = fixture.case(&name);
        let scale = case.input_str("scale");
        let found = correlation::normal_score_rho(
            case.input_f64("m"),
            marginal(&case.value["inputs"]["a"], scale),
            marginal(&case.value["inputs"]["b"], scale),
        )
        .map(|rho| vec![rho]);
        check_values(&mut rows, &name, found, &case.value["expected"]);
    }
    assert_close("normal_score_rho", KERNEL, rows);
    // marginals of two families have no coefficient, the python refuses them before
    let mixed =
        correlation::normal_score_rho(0.1, Marginal::Normal { sd: 1.0 }, Marginal::Log { sd: 1.0 });
    assert!(mixed.is_err());
}

/// The points of a case of `copula/` through the copula of its correlation.
fn copula_of(case: &Case<'_>) -> Vec<f64> {
    let mut u = case.input("u");
    draws::copula(
        &mut u,
        case.input_usize("n"),
        case.input_usize("d"),
        &case.input("r"),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", case.name));
    u
}

#[test]
fn the_copula_of_given_points_is_the_one_of_draw() {
    let fixture = Fixture::load("draws");
    let mut rows = Vec::new();
    for name in fixture.cases("copula/") {
        let case = fixture.case(&name);
        let found = copula_of(&case);
        let expected = case.expected();
        assert_eq!(found.len(), expected.len(), "{name}");
        for (index, (found, expected)) in found.into_iter().zip(expected).enumerate() {
            rows.push((format!("{name}[{index}]"), found, expected));
        }
    }
    assert_close("copula", KERNEL, rows);
}

#[test]
fn the_copula_of_the_identity_leaves_the_points_unchanged() {
    let fixture = Fixture::load("draws");
    for name in ["copula/identity", "copula/near_identity"] {
        let case = fixture.case(name);
        let found = copula_of(&case);
        let u = case.input("u");
        assert!(
            found
                .iter()
                .zip(&u)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{name}"
        );
    }
    // the sizes of the points and of the correlation are checked
    let mut u = vec![0.5; 6];
    assert!(draws::copula(&mut u, 2, 3, &[1.0, 0.0, 0.0, 1.0]).is_err());
    assert!(draws::copula(&mut u, 4, 2, &[1.0, 0.0, 0.0, 1.0]).is_err());
}

#[test]
fn the_draws_and_correlations_give_the_same_bits_on_every_target() {
    let fixture = Fixture::load("draws");
    let mut values = Vec::new();
    for name in fixture.cases("nearest/") {
        values.extend(nearest_of(&fixture.case(&name)));
    }
    for name in fixture.cases("copula/") {
        values.extend(copula_of(&fixture.case(&name)));
    }
    for name in fixture.cases("rho/") {
        let case = fixture.case(&name);
        let scale = case.input_str("scale");
        if let Ok(rho) = correlation::normal_score_rho(
            case.input_f64("m"),
            marginal(&case.value["inputs"]["a"], scale),
            marginal(&case.value["inputs"]["b"], scale),
        ) {
            values.push(rho);
        }
    }
    assert_eq!(
        hash(&values),
        0x4cb5_c456_9ee6_7877,
        "{} values",
        values.len()
    );
}

// --- testing ----------------------------------------------------------------------------------

/// The draws of the formula of `pattern` of `make_oracles.py`, whether every draw satisfies a
/// hypothesis: `((i * step + shift) % period) < ones` of the draw `i`.
fn pattern(n: u64, period: u64, ones: u64, step: u64, shift: u64) -> Vec<bool> {
    (0..n).map(|i| (i * step + shift) % period < ones).collect()
}

/// The draws of a case of `sequence/`: its list of booleans or the arguments of `pattern`.
fn satisfied_of(case: &Case<'_>) -> Vec<bool> {
    let inputs = &case.value["inputs"];
    match inputs.get("satisfied") {
        Some(list) => list
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_bool().unwrap())
            .collect(),
        None => {
            let get = |name: &str| inputs[name].as_u64().unwrap();
            pattern(
                get("n"),
                get("period"),
                get("ones"),
                get("step"),
                get("shift"),
            )
        }
    }
}

/// Every `stride`-th value from the first and the last value, `_strided` of the fixture.
fn strided(values: &[f64], stride: usize) -> Vec<f64> {
    let mut found: Vec<f64> = values.iter().step_by(stride).copied().collect();
    if !values.is_empty() && !(values.len() - 1).is_multiple_of(stride) {
        found.push(values[values.len() - 1]);
    }
    found
}

/// The p-values of `P >= p0`, `P < p0` and of the equivalence of a case of `sequence/`.
fn p_values_of(case: &Case<'_>) -> [Vec<f64>; 3] {
    let x = satisfied_of(case);
    let (p0, margin) = (case.input_f64("p0"), case.input_f64("margin"));
    [Side::Greater, Side::Less, Side::Equivalent]
        .map(|side| testing::p_values(&x, p0, side, margin))
}

/// The name of a decision of `_decide`.
fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::Accepted => "accepted",
        Decision::Rejected => "rejected",
        Decision::Equivalent => "equivalent",
        Decision::Undecided => "undecided",
    }
}

#[test]
fn the_constants_of_the_tests_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let case = fixture.case("constants");
    assert_eq!(case.expected_f64("margin"), testing::MARGIN);
    assert_eq!(case.expected_f64("max_needed"), testing::MAX_NEEDED as f64);
    assert_eq!(case.expected_f64("bisections"), testing::BISECTIONS as f64);
    assert_eq!(testing::MIXTURE, (0.5, 0.5));
}

#[test]
fn the_p_values_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let mut rows = Vec::new();
    for name in fixture.cases("sequence/") {
        let case = fixture.case(&name);
        let stride = case.input_usize("stride");
        let n = satisfied_of(&case).len();
        for (what, found) in ["p_greater", "p_less", "p_equivalent"]
            .into_iter()
            .zip(p_values_of(&case))
        {
            assert_eq!(found.len(), n, "{name} {what}");
            let expected = case.required(what);
            let found = strided(&found, stride);
            assert_eq!(found.len(), expected.len(), "{name} {what}");
            for (index, (found, expected)) in found.into_iter().zip(expected).enumerate() {
                // a p-value of 0 or 1 is the python's exactly
                if expected == 0.0 || expected == 1.0 {
                    assert_eq!(found, expected, "{name} {what}[{index}]");
                }
                rows.push((format!("{name} {what}[{index}]"), found, expected));
            }
        }
    }
    // the largest error, 8e-13 of a p-value of 500 draws, is the one of betaln of scipy, off
    // by 6e-15 of itself at (30.5, 464.5) where the crate's agrees with mpmath: the logarithm
    // of the mixture is a difference of terms of about 100
    assert_close("p_values", KERNEL, rows);
}

#[test]
fn the_decisions_and_their_draws_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let mut decided = 0;
    for name in fixture.cases("sequence/") {
        let case = fixture.case(&name);
        let alpha = case.input_f64("alpha");
        let [greater, less, equivalent] = p_values_of(&case);
        let (decision, n_decided) = testing::decision(&greater, &less, &equivalent, alpha);
        let expected = &case.value["expected"];
        assert_eq!(
            decision_name(decision),
            expected["decision"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            n_decided.map(|n| n as u64),
            expected["n_decided"].as_u64(),
            "{name}"
        );
        // the draws of a decision are the first ones of its p-values
        let first = match decision {
            Decision::Accepted => testing::decide(&greater, alpha),
            Decision::Rejected => testing::decide(&less, alpha),
            Decision::Equivalent => testing::decide(&equivalent, alpha),
            Decision::Undecided => None,
        };
        assert_eq!(first, n_decided, "{name}");
        decided += usize::from(n_decided.is_some());
    }
    assert!(decided > 50, "{decided} decided cases");
}

#[test]
fn the_confidence_sequence_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let mut rows = Vec::new();
    for name in fixture.cases("sequence/") {
        let case = fixture.case(&name);
        let (low, high) =
            testing::confidence_sequence(&satisfied_of(&case), case.input_f64("alpha")).unwrap();
        rows.push((format!("{name} low"), low, case.expected_f64("low")));
        rows.push((format!("{name} high"), high, case.expected_f64("high")));
    }
    assert_close("confidence_sequence", KERNEL, rows);
}

#[test]
fn the_confidence_sequence_refuses_an_alpha_as_python() {
    let fixture = Fixture::load("testing");
    for name in fixture.cases("sequence_alpha/") {
        let case = fixture.case(&name);
        let x = satisfied_of(&case);
        let found = testing::confidence_sequence(&x, case.input_f64("alpha"));
        let expected = &case.value["expected"];
        match (found, expected.get("error")) {
            (Ok((low, high)), None) => {
                let expected = numbers(expected);
                assert!(relative_error(low, expected[0]) <= KERNEL, "{name}");
                assert!(relative_error(high, expected[1]) <= KERNEL, "{name}");
            }
            (Err(error), Some(_)) => check_error(&error, expected, &name),
            (found, _) => panic!("{name}: {found:?}, the python {expected}"),
        }
    }
    // no draws, no logarithm: the whole interval
    assert_eq!(testing::confidence_sequence(&[], 0.0), Ok((0.0, 1.0)));
}

#[test]
fn the_draws_an_undecided_statement_needs_are_the_ones_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let case = fixture.case("needed");
    let (estimates, ns) = (case.input("estimate"), case.input_u64("n"));
    let (p0s, margins, alphas) = (case.input("p0"), case.input("margin"), case.input("alpha"));
    let expected = case.value["expected"].as_array().unwrap();
    for i in 0..estimates.len() {
        let found = testing::needed(estimates[i], ns[i], p0s[i], margins[i], alphas[i]).unwrap();
        assert_eq!(
            found,
            expected[i].as_u64(),
            "needed({}, {}, {}, {}, {})",
            estimates[i],
            ns[i],
            p0s[i],
            margins[i],
            alphas[i]
        );
    }
    for name in fixture.cases("sequence/") {
        let case = fixture.case(&name);
        let x = satisfied_of(&case);
        if x.is_empty() {
            continue;
        }
        let n = x.len() as u64;
        let k = x.iter().filter(|&&b| b).count() as f64;
        let found = testing::needed(
            k / n as f64,
            n,
            case.input_f64("p0"),
            case.input_f64("margin"),
            case.input_f64("alpha"),
        )
        .unwrap();
        assert_eq!(found, case.value["expected"]["needed"].as_u64(), "{name}");
    }
    // no draws: no number of draws, where the python's doubling would not end
    assert_eq!(testing::needed(0.5, 0, 0.5, 0.05, 0.05), Ok(None));
}

#[test]
fn the_draws_needed_refuse_an_estimate_as_python() {
    let fixture = Fixture::load("testing");
    for name in fixture.cases("needed_error/") {
        let case = fixture.case(&name);
        let found = testing::needed(
            case.input_f64("estimate"),
            case.input_usize("n") as u64,
            case.input_f64("p0"),
            case.input_f64("margin"),
            case.input_f64("alpha"),
        );
        let expected = &case.value["expected"];
        match (found, expected.get("error")) {
            (Ok(found), None) => assert_eq!(found, expected.as_u64(), "{name}"),
            (Err(error), Some(_)) => check_error(&error, expected, &name),
            (found, _) => panic!("{name}: {found:?}, the python {expected}"),
        }
    }
}

#[test]
fn the_family_level_is_the_one_of_sbmlsim() {
    let fixture = Fixture::load("testing");
    let case = fixture.case("family_level");
    let rows = case
        .input("level")
        .into_iter()
        .zip(case.input_i64("m"))
        .zip(case.expected())
        .map(|((level, m), expected)| {
            (
                format!("{level}, {m}"),
                testing::family_level(level, m),
                expected,
            )
        })
        .collect();
    assert_close("family_level", 0.0, rows);
}

/// The bands of a case of `bands/`.
fn bands_of(case: &Case<'_>) -> sbmlsim_stats::Result<Vec<f64>> {
    variability::bands(
        &case.input("distribution"),
        case.input_usize("rows"),
        case.input_usize("cols"),
        &case.input("qs"),
    )
}

#[test]
fn the_bands_are_the_quantiles_of_numpy_of_the_finite_values() {
    let fixture = Fixture::load("testing");
    let mut rows = Vec::new();
    for name in fixture.cases("bands/") {
        let case = fixture.case(&name);
        check_values(&mut rows, &name, bands_of(&case), &case.value["expected"]);
    }
    assert_close("bands", KERNEL, rows);
    // a distribution of another size is refused
    assert!(variability::bands(&[1.0, 2.0, 3.0], 2, 2, &[0.5]).is_err());
}

#[test]
fn the_tests_and_the_bands_give_the_same_bits_on_every_target() {
    let fixture = Fixture::load("testing");
    let mut values = Vec::new();
    for name in fixture.cases("sequence/") {
        let case = fixture.case(&name);
        for p in p_values_of(&case) {
            values.extend(p);
        }
        let (low, high) =
            testing::confidence_sequence(&satisfied_of(&case), case.input_f64("alpha")).unwrap();
        values.extend([low, high]);
    }
    for name in fixture.cases("bands/") {
        if let Ok(bands) = bands_of(&fixture.case(&name)) {
            values.extend(bands);
        }
    }
    assert_eq!(
        hash(&values),
        0x5631_5586_186d_8229,
        "{} values",
        values.len()
    );
}

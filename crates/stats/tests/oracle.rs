//! The functions of the crate against numpy and scipy: the values `oracles/make_oracles.py`
//! computed once and stored as a JSON fixture per module (`oracles/<module>.json`).
//!
//! A fixture holds its cases by name, every case its inputs and the expected values, the
//! numbers as the doubles they were and the numbers which JSON cannot hold as the strings
//! "nan", "inf" and "-inf". A test runs a function over the inputs of a case and holds every
//! value to the expected one at a relative tolerance; it prints the largest relative error it
//! found (`cargo test -p sbmlsim-stats -- --nocapture`).
#![cfg(not(target_family = "wasm"))]

use std::path::PathBuf;

use serde_json::Value;

use sbmlsim_stats::special;

/// The fixture of a module, `oracles/<module>.json`.
struct Fixture {
    module: String,
    value: Value,
}

impl Fixture {
    fn load(module: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("oracles")
            .join(format!("{module}.json"));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Self {
            module: module.to_string(),
            value,
        }
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

    /// The expected values, a list of numbers.
    fn expected(&self) -> Vec<f64> {
        numbers(&self.value["expected"])
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
            assert_eq!(
                special::t_ppf(1.0 - p, df),
                -special::t_ppf(1.0 - (1.0 - p), df)
            );
            let back = special::t_cdf(t, df);
            assert!((back - p).abs() <= 1e-12 * p, "{p}, {df}: {back}");
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

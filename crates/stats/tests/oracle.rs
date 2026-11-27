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

use serde_json::Value;

use sbmlsim_stats::{linalg, metrics, special};

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
        let found = linalg::gram(&j, n, k);
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
    let g = linalg::gram(&j, 3, 2);
    assert_eq!(g[0], 35.0);
    assert!(g[1].is_nan() && g[2].is_nan() && g[3].is_nan());
    assert_eq!(linalg::gram(&[], 0, 2), vec![0.0; 4]);
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
        assert!(error.0.contains("is not symmetric"), "{error}");
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
        assert!(error.0.contains("is not finite"), "{error}");
        assert!(linalg::eigvalsh(&a, 2).is_err());
        assert!(linalg::pinv_symmetric(&a, 2).is_err());
        assert!(linalg::factor(&a, 2).is_err());
        assert!(linalg::cholesky(&a, 2).is_none());
    }
    let error = linalg::eigh(&[1.0, 0.0, 0.0], 2).unwrap_err();
    assert!(error.0.contains("2 x 2"), "{error}");
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
        let gram = linalg::gram(&j, n, k);
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
        (Err(error), Err(message)) => assert_eq!(error.0, message, "{what}"),
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
                assert_eq!(counts.unwrap_err().0, message, "{name}");
                // the criterion of a point refuses what the counts refuse
                assert_eq!(accepted.unwrap_err().0, message, "{name}");
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
        error.0,
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
        assert_eq!(result.unwrap_err().0, message);
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

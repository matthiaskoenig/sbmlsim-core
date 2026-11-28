//! The functions of the special functions (`special`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use sbmlsim_stats::special;

use common::*;

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

/// Holds every value found to the expected one of scipy to the bit, the sign of a zero
/// included (the edges of a function, which are no rounding); two values below the smallest
/// normal double are the same, as in [`relative_error`], where scipy flushes a subnormal
/// value to zero (`gamma(-171.5)`).
fn assert_same(name: &str, rows: Vec<(String, f64, f64)>) {
    for (inputs, found, expected) in rows {
        let same = found.to_bits() == expected.to_bits()
            || (found.is_nan() && expected.is_nan())
            || (found.abs() < f64::MIN_POSITIVE
                && expected.abs() < f64::MIN_POSITIVE
                && (found != 0.0 || expected != 0.0 || found.signum() == expected.signum()));
        assert!(same, "{name}({inputs}) = {found:e}, expected {expected:e}");
    }
}

/// Holds every value of the incomplete gamma function of `a` at `x` to the expected one at
/// the tolerance [`GAMMA_INC`] times its condition, `max(1, |x - a| / 100)`: a relative
/// change of `x` by one rounding changes `P` and `Q` by about `|x - a|` roundings, so the
/// rounding of the exponent `a (ln λ - (λ - 1))` bounds every method far in the tails.
fn assert_close_conditioned(name: &str, a: &[f64], x: &[f64], found: &[f64], expected: &[f64]) {
    let mut worst = 0.0_f64;
    for (((&a, &x), &found), &expected) in a.iter().zip(x).zip(found).zip(expected) {
        let condition = ((x - a).abs() / 100.0).max(1.0);
        let error = relative_error(found, expected) / condition;
        worst = worst.max(error);
        assert!(
            error <= GAMMA_INC,
            "{name}({a}, {x}) = {found:e}, expected {expected:e}: {error:e} of the condition"
        );
    }
    println!("{name}: largest relative error per condition {worst:.3e}");
}

/// The rows of a function of one input of a case.
fn rows_of(case: &Case<'_>, input: &str, function: fn(f64) -> f64) -> Vec<(String, f64, f64)> {
    case.input(input)
        .into_iter()
        .zip(case.expected())
        .map(|(x, expected)| (format!("{x}"), function(x), expected))
        .collect()
}

/// The rows of a function of two inputs of a case.
fn rows_of_two(
    case: &Case<'_>,
    inputs: [&str; 2],
    function: fn(f64, f64) -> f64,
) -> Vec<(String, f64, f64)> {
    case.input(inputs[0])
        .into_iter()
        .zip(case.input(inputs[1]))
        .zip(case.expected())
        .map(|((x, y), expected)| (format!("{x}, {y}"), function(x, y), expected))
        .collect()
}

#[test]
fn erf_is_the_error_function_of_mpmath() {
    let fixture = Fixture::load("special");
    let case = fixture.case("erf");
    assert_close("erf", ERF, rows_of(&case, "x", special::erf));
    let case = fixture.case("erf_edges");
    assert_same("erf", rows_of(&case, "x", special::erf));
}

#[test]
fn gamma_is_the_gamma_function_of_mpmath() {
    let fixture = Fixture::load("special");
    let case = fixture.case("gamma");
    assert_close("gamma", GAMMA, rows_of(&case, "x", special::gamma));
    let case = fixture.case("gamma_edges");
    assert_same("gamma", rows_of(&case, "x", special::gamma));
    // the factorials are exact
    let mut factorial = 1.0;
    for n in 1..=22_u32 {
        factorial *= f64::from(n);
        assert_eq!(special::gamma(f64::from(n) + 1.0), factorial, "{n}!");
    }
}

#[test]
fn the_incomplete_gamma_function_is_the_one_of_mpmath() {
    let fixture = Fixture::load("special");
    let case = fixture.case("gamma_inc");
    let (a, x) = (case.input("a"), case.input("x"));
    for (name, function, expected) in [
        (
            "gamma_p",
            special::gamma_p as fn(f64, f64) -> f64,
            case.required("p"),
        ),
        ("gamma_q", special::gamma_q, case.required("q")),
    ] {
        let found: Vec<f64> = a.iter().zip(&x).map(|(&a, &x)| function(a, x)).collect();
        assert_close_conditioned(name, &a, &x, &found, &expected);
    }
    let case = fixture.case("gamma_inc_edges");
    let (a, x) = (case.input("a"), case.input("x"));
    for (name, function, expected) in [
        (
            "gamma_p",
            special::gamma_p as fn(f64, f64) -> f64,
            case.required("p"),
        ),
        ("gamma_q", special::gamma_q, case.required("q")),
    ] {
        let rows = a
            .iter()
            .zip(&x)
            .zip(expected)
            .map(|((&a, &x), expected)| (format!("{a}, {x}"), function(a, x), expected))
            .collect();
        assert_same(name, rows);
    }
}

#[test]
fn the_chi_squared_distribution_is_the_one_of_mpmath() {
    let fixture = Fixture::load("special");
    let case = fixture.case("chi2_cdf");
    let (x, df) = (case.input("x"), case.input("df"));
    let found: Vec<f64> = x
        .iter()
        .zip(&df)
        .map(|(&x, &df)| special::chi2_cdf(x, df))
        .collect();
    let half = |values: &[f64]| values.iter().map(|v| v / 2.0).collect::<Vec<f64>>();
    assert_close_conditioned("chi2_cdf", &half(&df), &half(&x), &found, &case.expected());
    let case = fixture.case("chi2_ppf");
    assert_close(
        "chi2_ppf",
        CHI2_PPF,
        rows_of_two(&case, ["p", "df"], special::chi2_ppf),
    );
}

#[test]
fn the_edges_of_the_chi_squared_and_the_t_distribution_are_the_ones_of_scipy() {
    let fixture = Fixture::load("special");
    for (name, inputs, function) in [
        (
            "chi2_cdf_edges",
            ["x", "df"],
            special::chi2_cdf as fn(f64, f64) -> f64,
        ),
        ("chi2_ppf_edges", ["p", "df"], special::chi2_ppf),
        ("t_cdf_edges", ["x", "df"], special::t_cdf),
        ("t_ppf_edges", ["p", "df"], special::t_ppf),
    ] {
        let case = fixture.case(name);
        let rows = rows_of_two(&case, inputs, function);
        if name.starts_with("t_") {
            // the normal distribution of infinitely many degrees of freedom, of AS 241 and
            // erf of libm, and scipy's t distribution differ in their last digit
            assert_close(name, NORMAL, rows);
        } else {
            assert_same(name, rows);
        }
    }
}

#[test]
fn the_chi_squared_quantile_inverts_the_distribution_function() {
    for df in [0.1, 0.5, 1.0, 2.0, 3.0, 7.5, 40.0, 1e3, 1e5, 1e7, 1e10] {
        for p in [1e-200, 1e-12, 0.01, 0.3, 0.5, 0.8, 0.99, 1.0 - 1e-9] {
            let x = special::chi2_ppf(p, df);
            let back = special::chi2_cdf(x, df);
            // the condition of the distribution function, the factor of a relative error of x
            let condition = (x / 2.0 - df / 2.0).abs().max(1.0);
            let error = if p < 0.5 {
                (back - p).abs() / p
            } else {
                ((1.0 - back) - (1.0 - p)).abs() / (1.0 - p)
            };
            assert!(error <= 1e-13 * condition, "{p}, {df}: {x} gives {back}");
        }
    }
}

#[test]
fn the_new_special_functions_give_the_same_bits_on_every_target() {
    // the values of the cases of the functions of 0.43.0, natively and in wasm
    let fixture = Fixture::load("special");
    let mut values = Vec::new();
    for name in ["erf", "erf_edges"] {
        values.extend(fixture.case(name).input("x").into_iter().map(special::erf));
    }
    for name in ["gamma", "gamma_edges"] {
        values.extend(
            fixture
                .case(name)
                .input("x")
                .into_iter()
                .map(special::gamma),
        );
    }
    for name in ["gamma_inc", "gamma_inc_edges"] {
        let case = fixture.case(name);
        for (a, x) in case.input("a").into_iter().zip(case.input("x")) {
            values.push(special::gamma_p(a, x));
            values.push(special::gamma_q(a, x));
        }
    }
    for name in ["chi2_cdf", "chi2_cdf_edges"] {
        let case = fixture.case(name);
        for (x, df) in case.input("x").into_iter().zip(case.input("df")) {
            values.push(special::chi2_cdf(x, df));
        }
    }
    for name in ["chi2_ppf", "chi2_ppf_edges"] {
        let case = fixture.case(name);
        for (p, df) in case.input("p").into_iter().zip(case.input("df")) {
            values.push(special::chi2_ppf(p, df));
        }
    }
    assert_eq!(hash(&values), 0, "{} values", values.len());
}

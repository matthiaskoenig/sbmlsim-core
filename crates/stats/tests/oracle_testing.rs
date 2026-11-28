//! The functions of the probability statements and the bands (`testing`, `variability`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use sbmlsim_stats::testing::{self, Decision, Side};
use sbmlsim_stats::variability;

use common::*;

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

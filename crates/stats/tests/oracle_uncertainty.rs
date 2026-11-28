//! The functions of the statistics of the draws (`uncertainty`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use std::collections::BTreeMap;

use serde_json::Value;

use sbmlsim_stats::uncertainty;

use common::*;

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

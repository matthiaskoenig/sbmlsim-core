//! The functions of the metrics of a fit (`metrics`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use serde_json::Value;

use sbmlsim_stats::metrics;

use common::*;

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

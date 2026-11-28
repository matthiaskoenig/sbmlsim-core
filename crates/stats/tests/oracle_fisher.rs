//! The functions of the Fisher information and the posterior (`fisher`, `posterior`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use sbmlsim_stats::{Scale, fisher, linalg, posterior, special};

use common::*;

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
        case.input_usize("n") as u64,
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
        case.input_usize("n") as u64,
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

#[test]
fn an_information_whose_pseudo_inverse_overflows_has_the_statistics_of_numpy() {
    // the pseudo-inverse of numpy of diag(1e-295, 1e-309) is [[nan, nan], [nan, inf]]: the
    // statistics are not a number and infinite, as before the decomposition was public
    let found = fisher::statistics(&[1e-295, 0.0, 0.0, 1e-309], 2, 1.0, 10, false, 1e-8).unwrap();
    assert!(found.covariance.iter().any(|v| !v.is_finite()));
    let (spectrum, inverse) = fisher::decomposition(&[1e-295, 0.0, 0.0, 1e-309], 2).unwrap();
    let derived =
        fisher::statistics_of_decomposition(&spectrum, &inverse, 2, 1.0, 10, false, 1e-8).unwrap();
    let bits = |values: &[f64]| -> Vec<u64> {
        values
            .iter()
            .map(|v| if v.is_nan() { f64::NAN } else { *v }.to_bits())
            .collect()
    };
    assert_eq!(bits(&found.covariance), bits(&derived.covariance));
    assert_eq!(bits(&found.standard_errors), bits(&derived.standard_errors));
}

#[test]
fn the_statistics_of_a_decomposition_of_lapack_are_the_ones_sbmlsim_derives_to_the_bit() {
    let fixture = Fixture::load("fisher_decomposition");
    for name in fixture.cases("decomposition/") {
        let case = fixture.case(&name);
        let k = case.input_usize("k");
        let found = fisher::statistics_of_decomposition(
            &case.input("spectrum"),
            &case.input("inverse"),
            k,
            case.input_f64("cost"),
            case.input_usize("n") as u64,
            case.input_bool("likelihood"),
            fisher::DEFAULT_RANK_TOLERANCE,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let same = |found: &[f64], expected: &[f64]| {
            found.len() == expected.len()
                && found
                    .iter()
                    .zip(expected)
                    .all(|(f, e)| f.to_bits() == e.to_bits() || (f.is_nan() && e.is_nan()))
        };
        assert!(
            same(&[found.sigma2], &[case.expected_f64("sigma2")]),
            "{name} sigma2"
        );
        assert!(
            same(&[found.condition], &[case.expected_f64("condition")]),
            "{name} condition"
        );
        assert_eq!(found.rank as f64, case.expected_f64("rank"), "{name} rank");
        assert_eq!(found.spectrum, case.input("spectrum"), "{name} spectrum");
        for (part, values) in [
            ("covariance", &found.covariance),
            ("standard_errors", &found.standard_errors),
            ("correlation", &found.correlation),
        ] {
            assert!(same(values, &case.required(part)), "{name} {part}");
        }
    }
}

#[test]
fn a_decomposition_which_does_not_fit_is_an_error() {
    let statistics = |spectrum: &[f64], inverse: &[f64], k: usize| {
        fisher::statistics_of_decomposition(spectrum, inverse, k, 1.0, 10, false, 1e-8)
            .map(|_| ())
            .unwrap_err()
            .message
    };
    assert_eq!(
        statistics(&[], &[], 0),
        "The Fisher information of no parameters has no statistics."
    );
    assert_eq!(
        statistics(&[2.0], &[0.5, 0.0], 1),
        "The decomposition of a Fisher information of 1 parameters needs 1 eigenvalues and \
         a pseudo-inverse of 1 x 1 values, not 1 and 2."
    );
    assert_eq!(
        statistics(&[1.0, 2.0], &[1.0, 0.0, 0.0, 0.5], 2),
        "The eigenvalues of a Fisher information must be largest first."
    );
    // a pseudo-inverse of LAPACK, which is not symmetric to the bit, is taken as it is
    let found = fisher::statistics_of_decomposition(
        &[2.0, 1.0],
        &[0.5, 0.1, 0.1 + 1e-17, 1.0],
        2,
        1.0,
        10,
        true,
        1e-8,
    )
    .unwrap();
    assert_eq!(found.covariance[2], 0.1 + 1e-17);
}

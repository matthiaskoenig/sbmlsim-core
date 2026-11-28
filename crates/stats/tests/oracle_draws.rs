//! The functions of the draws and the correlations (`draws`, `correlation`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use serde_json::Value;

use sbmlsim_stats::correlation::{self, Marginal};
use sbmlsim_stats::draws;

use common::*;

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

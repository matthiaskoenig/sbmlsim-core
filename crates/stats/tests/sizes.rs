//! The sizes of the arrays the functions make, counted without a wrap on every target: a size
//! past what an array holds (its bytes past the largest `isize`) is numpy's ValueError, before
//! any memory is asked for. The arguments here are sizes no memory holds, so a function which
//! allocated them would fail the tests by running out of memory, natively and in wasm, where a
//! `usize` is of 32 bits (`cargo test -p sbmlsim-stats --target wasm32-wasip1`).

use sbmlsim_stats::draws::{self, Design};
use sbmlsim_stats::{correlation, linalg, posterior};

/// The ValueError of numpy of an array whose bytes exceed the largest `isize`.
const ARRAY_TOO_BIG: &str =
    "array is too big; `arr.size * arr.dtype.itemsize` is larger than the maximum possible size.";

/// A dimension whose square wraps a `usize`, of 32 bits and of 64 bits.
const HALF: usize = 1 << (usize::BITS / 2);

#[test]
fn points_past_what_an_array_holds_are_refused() {
    for design in [Design::Sobol, Design::Lhs, Design::Random] {
        for (d, n) in [
            // a product which wraps a usize
            (2, usize::MAX / 2 + 1),
            (HALF, HALF),
            // a product a usize holds, whose bytes exceed the largest isize
            (1, usize::MAX / 8),
            (HALF / 2, HALF / 2),
            // no dimensions of more points than an array of no values holds
            (0, usize::MAX / 4),
        ] {
            let error = draws::unit_points(d, n, design, 1, 0).unwrap_err();
            assert_eq!(error.message, ARRAY_TOO_BIG, "{design:?} {d} {n}");
        }
    }
}

#[test]
fn a_gram_matrix_past_what_an_array_holds_is_refused() {
    // a jacobian of no residuals of many parameters
    for k in [HALF, HALF / 2, usize::MAX] {
        let error = linalg::gram(&[], 0, k).unwrap_err();
        assert_eq!(error.message, ARRAY_TOO_BIG, "{k}");
    }
    assert_eq!(linalg::gram(&[], 0, 2), Ok(vec![0.0; 4]));
    assert_eq!(
        linalg::gram(&[1.0, 2.0, 3.0], 2, 2).unwrap_err().message,
        "A jacobian of 2 x 2 needs 4 values, not 3."
    );
}

#[test]
fn a_copula_past_what_an_array_holds_is_refused() {
    let error = draws::copula(&mut [], usize::MAX / 2 + 1, 2, &[1.0, 0.0, 0.0, 1.0]).unwrap_err();
    assert_eq!(error.message, ARRAY_TOO_BIG);
    // no points of a correlation of many dimensions
    let error = draws::copula(&mut [], 0, HALF, &[]).unwrap_err();
    assert_eq!(error.message, ARRAY_TOO_BIG);
}

#[test]
fn a_matrix_of_more_values_than_a_usize_counts_is_refused() {
    // k * k wraps: no slice is of its values, and the error is no wrapped count of them
    assert!(linalg::cholesky(&[], HALF).is_none());
    for result in [
        linalg::eigh(&[], HALF).map(|(values, _)| values),
        linalg::eigvalsh(&[], HALF),
        linalg::pinv_symmetric(&[], HALF),
        linalg::factor(&[], HALF),
        posterior::rank_correlation(&[], HALF),
        posterior::normal_correlation(&[], HALF),
        posterior::laplace(&[], HALF, 1e-8),
        correlation::nearest_correlation(&[], HALF, 10, 1e-10),
    ] {
        assert_eq!(result.unwrap_err().message, ARRAY_TOO_BIG);
    }
    assert!(!correlation::is_psd(&[], HALF, 1e-8));
}

#[test]
fn lengths_which_do_not_fit_are_errors_not_panics() {
    assert_eq!(
        linalg::pinv_of_eigh(&[1.0], &[1.0, 0.0], 1)
            .unwrap_err()
            .message,
        "A matrix of 1 x 1 needs 1 values, not 2."
    );
    assert!(linalg::pinv_of_eigh(&[1.0, 2.0], &[1.0], 1).is_err());
    assert!(posterior::rank_correlation(&[1.0, 0.5], 2).is_err());
    assert!(posterior::normal_correlation(&[1.0, 0.5, 0.5], 2).is_err());
}

#[test]
#[cfg(target_pointer_width = "32")]
fn sizes_past_32_bits_are_refused_in_wasm() {
    // 70000^2 wraps a usize of 32 bits, not one of 64 bits (where the points are 39 GB)
    for design in [Design::Lhs, Design::Random] {
        let error = draws::unit_points(70_000, 70_000, design, 1, 0).unwrap_err();
        assert_eq!(error.message, ARRAY_TOO_BIG, "{design:?}");
    }
    assert_eq!(
        linalg::gram(&[], 0, 70_000).unwrap_err().message,
        ARRAY_TOO_BIG
    );
    // 2^29 doubles: a usize of 32 bits counts them, their bytes exceed its largest isize
    let error = draws::unit_points(1 << 14, 1 << 15, Design::Random, 1, 0).unwrap_err();
    assert_eq!(error.message, ARRAY_TOO_BIG);
}

//! The functions of the linear algebra (`linalg`) against numpy, scipy and sbmlsim: the values
//! `oracles/make_oracles.py` computed once and stored as JSON (`oracles/<module>.json`), see
//! `common`.
//!
//! The fixtures are compiled into the tests, so they run in wasm as well
//! (`cargo test -p sbmlsim-stats --target wasm32-wasip1`, in the continuous integration), and
//! a hash of the bits of every value found holds the wasm build to the native one.

mod common;

use sbmlsim_stats::linalg;

use common::*;

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

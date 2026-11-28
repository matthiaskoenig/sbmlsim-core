//! The random draws of the uncertainty analyses: the points of the unit cube of the three
//! designs and the Gaussian copula, held to their distributions and to their determinism.
//!
//! The streams are the ones of sbmlsim-core (the Sobol' sequence and xoshiro256++ of the crate
//! sbmlsim-sensitivity), not those of scipy and numpy, so the tests are statistical: uniform
//! marginals, one point per stratum of a Latin hypercube, the correlation of the normal scores
//! of the copula. A seed gives the same bits on every target, which a hash of a few draws holds
//! natively and in wasm (`cargo test -p sbmlsim-stats --target wasm32-wasip1`).

mod common;

use sbmlsim_stats::draws::{self, Design};
use sbmlsim_stats::special;

use common::hash;

const DESIGNS: [Design; 3] = [Design::Sobol, Design::Lhs, Design::Random];

/// The column `j` of a matrix of `d` columns, row-major.
fn column(points: &[f64], d: usize, j: usize) -> Vec<f64> {
    points.iter().skip(j).step_by(d).copied().collect()
}

/// The statistic of Kolmogorov and Smirnov of values against the uniform distribution.
fn ks_uniform(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len() as f64;
    sorted
        .iter()
        .enumerate()
        .map(|(i, &u)| (u - i as f64 / n).max((i + 1) as f64 / n - u))
        .fold(0.0, f64::max)
}

/// The Pearson correlation of two columns.
fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    sxy / (sxx * syy).sqrt()
}

/// The normal scores of points of the unit interval.
fn normal_scores(u: &[f64]) -> Vec<f64> {
    u.iter().map(|&u| special::normal_ppf(u)).collect()
}

#[test]
fn a_seed_gives_the_same_points_and_another_seed_others() {
    for design in DESIGNS {
        let a = draws::unit_points(3, 64, design, 42, 0).unwrap();
        let b = draws::unit_points(3, 64, design, 42, 0).unwrap();
        let c = draws::unit_points(3, 64, design, 43, 0).unwrap();
        assert_eq!(a.len(), 64 * 3);
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "{design:?}"
        );
        assert_ne!(a, c, "{design:?}");
        assert!(a.iter().all(|u| (0.0..1.0).contains(u)), "{design:?}");
    }
}

#[test]
fn sobol_and_random_points_continue_their_sequence_from_start() {
    for design in [Design::Sobol, Design::Random] {
        for (start, n) in [(0, 16), (5, 11), (64, 64), (1000, 24)] {
            let d = 4;
            let long = draws::unit_points(d, start + n, design, 7, 0).unwrap();
            let tail = draws::unit_points(d, n, design, 7, start as u64).unwrap();
            assert_eq!(tail[..], long[start * d..], "{design:?} {start}");
        }
    }
}

#[test]
fn random_points_jump_to_any_start() {
    // the jump of the stream past the doubles of the points before, `start * d` of them up to
    // a million, is the tail of the points drawn one after the other
    for (d, start) in [(1, 1), (3, 7), (2, 1000), (5, 200_000), (1, 1_000_000)] {
        let n = 3;
        let long = draws::unit_points(d, start + n, Design::Random, 11, 0).unwrap();
        let tail = draws::unit_points(d, n, Design::Random, 11, start as u64).unwrap();
        assert_eq!(tail[..], long[start * d..], "{d} {start}");
    }
    // a start past any loop: the points of `start` are the tail of the points of `start - 2`
    let start = 1 << 62;
    let earlier = draws::unit_points(3, 4, Design::Random, 5, start - 2).unwrap();
    let later = draws::unit_points(3, 2, Design::Random, 5, start).unwrap();
    assert_eq!(later[..], earlier[2 * 3..]);
    let last = draws::unit_points(2, 1, Design::Random, 5, u64::MAX).unwrap();
    assert!(last.iter().all(|u| (0.0..1.0).contains(u)));
}

#[test]
fn a_latin_hypercube_has_no_continuation() {
    let error = draws::unit_points(2, 4, Design::Lhs, 1, 2).unwrap_err();
    assert_eq!(
        error.message,
        "A Latin hypercube has no continuation, its points of more draws are another \
         hypercube: give the number of draws, or use 'sobol' or 'random' for more draws."
    );
}

#[test]
fn no_points_or_no_dimensions_are_no_values() {
    for design in DESIGNS {
        assert!(draws::unit_points(2, 0, design, 1, 0).unwrap().is_empty());
        assert!(draws::unit_points(0, 4, design, 1, 0).unwrap().is_empty());
    }
}

#[test]
fn the_points_of_every_design_are_uniform() {
    // the 1 % critical value of the statistic of Kolmogorov and Smirnov, 1.63 / sqrt(n); the
    // quasi-random designs are far closer
    let (n, d) = (4096, 5);
    for design in DESIGNS {
        let points = draws::unit_points(d, n, design, 2026, 0).unwrap();
        for j in 0..d {
            let statistic = ks_uniform(&column(&points, d, j));
            let bound = match design {
                Design::Random => 1.63 / (n as f64).sqrt(),
                Design::Sobol | Design::Lhs => 2.0 / n as f64,
            };
            assert!(statistic < bound, "{design:?} {j}: {statistic}");
        }
    }
}

#[test]
fn a_latin_hypercube_has_one_point_in_every_stratum() {
    for n in [1, 7, 1000, 4096] {
        let d = 3;
        let points = draws::unit_points(d, n, Design::Lhs, 11, 0).unwrap();
        for j in 0..d {
            let mut seen = vec![false; n];
            for u in column(&points, d, j) {
                let stratum = (0..n)
                    .find(|&p| p as f64 / n as f64 <= u && u < (p + 1) as f64 / n as f64)
                    .unwrap_or_else(|| panic!("{u} in no stratum of {n}"));
                assert!(!seen[stratum], "two points in the stratum {stratum} of {n}");
                seen[stratum] = true;
            }
        }
    }
}

#[test]
fn the_copula_correlates_the_normal_scores() {
    let (n, d) = (4096, 3);
    let r = [1.0, 0.7, 0.0, 0.7, 1.0, -0.3, 0.0, -0.3, 1.0];
    for design in DESIGNS {
        let mut u = draws::unit_points(d, n, design, 5, 0).unwrap();
        draws::copula(&mut u, n, d, &r).unwrap();
        assert!(u.iter().all(|u| (0.0..=1.0).contains(u)), "{design:?}");
        let z: Vec<Vec<f64>> = (0..d).map(|j| normal_scores(&column(&u, d, j))).collect();
        for (a, b) in [(0, 1), (0, 2), (1, 2)] {
            let (found, rho) = (pearson(&z[a], &z[b]), r[a * d + b]);
            // 0.02 of 0.7, three standard errors of a sample correlation of random points,
            // (1 - rho^2) / sqrt(n), of the others
            let tolerance = if rho == 0.7 {
                0.02
            } else {
                3.0 * (1.0 - rho * rho) / (n as f64).sqrt()
            };
            assert!(
                (found - rho).abs() < tolerance,
                "{design:?} ({a}, {b}): {found}"
            );
        }
        // the marginals stay uniform
        for j in 0..d {
            assert!(ks_uniform(&column(&u, d, j)) < 0.03, "{design:?} {j}");
        }
    }
}

#[test]
fn the_copula_of_a_singular_correlation_has_its_correlation() {
    // two variables which are one: the factor of the eigendecomposition, not of Cholesky
    let (n, d) = (4096, 3);
    let r = [1.0, 1.0, 0.5, 1.0, 1.0, 0.5, 0.5, 0.5, 1.0];
    let mut u = draws::unit_points(d, n, Design::Sobol, 9, 0).unwrap();
    draws::copula(&mut u, n, d, &r).unwrap();
    let z: Vec<Vec<f64>> = (0..d).map(|j| normal_scores(&column(&u, d, j))).collect();
    assert!((pearson(&z[0], &z[1]) - 1.0).abs() < 1e-9);
    assert!((pearson(&z[0], &z[2]) - 0.5).abs() < 0.02);
}

#[test]
fn the_draws_give_the_same_bits_on_every_target() {
    let mut values = Vec::new();
    for design in DESIGNS {
        values.extend(draws::unit_points(3, 16, design, 123, 0).unwrap());
    }
    values.extend(draws::unit_points(3, 8, Design::Sobol, 123, 1_000).unwrap());
    values.extend(draws::unit_points(3, 8, Design::Random, 123, 1_000).unwrap());
    let mut u = draws::unit_points(3, 16, Design::Sobol, 321, 0).unwrap();
    draws::copula(
        &mut u,
        16,
        3,
        &[1.0, 0.7, 0.0, 0.7, 1.0, -0.3, 0.0, -0.3, 1.0],
    )
    .unwrap();
    values.extend(u);
    assert_eq!(
        hash(&values),
        0x8562_a871_35e5_698d,
        "{} values",
        values.len()
    );
}

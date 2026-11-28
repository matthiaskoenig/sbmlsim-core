//! The virtual groups of subjects of a visual predictive check: the means of groups of subjects
//! resampled with replacement, held to their distributions and to their determinism.
//!
//! The resampling draws from xoshiro256++ of sbmlsim-core, not from the generator of numpy, so
//! the tests are statistical: a group of one subject is the subject itself, every subject is
//! drawn equally often, the mean of the group means is the mean of the subjects and their
//! variance the variance of a mean of independent draws, and the geometric mean of a group is
//! at most its mean. A seed gives the same bits on every target, which a hash holds natively
//! and in wasm (`cargo test -p sbmlsim-stats --target wasm32-wasip1`).

use sbmlsim_stats::variability;

/// The FNV-1a hash of the bits of the values, as the one of `tests/oracle.rs`.
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

/// The predictions of `subjects` subjects at `points` data points of a formula of integers,
/// positive: `1 + ((s * 7 + j * 3) % 11) / 4 + j` of the subject `s` at the point `j`.
fn predictions(subjects: usize, points: usize) -> Vec<f64> {
    (0..subjects)
        .flat_map(|s| {
            (0..points).map(move |j| 1.0 + ((s * 7 + j * 3) % 11) as f64 / 4.0 + j as f64)
        })
        .collect()
}

/// The column `j` of a matrix of `points` columns, row-major.
fn column(values: &[f64], points: usize, j: usize) -> Vec<f64> {
    values.iter().skip(j).step_by(points).copied().collect()
}

/// The mean and the variance (of the population) of values.
fn moments(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    (mean, variance)
}

#[test]
fn a_group_of_one_subject_is_the_subject_itself() {
    let values = predictions(13, 4);
    let (groups, rows) = variability::group_means(&values, 13, 4, 1, 500, false, 7).unwrap();
    assert_eq!(rows, 13);
    assert!(
        groups
            .iter()
            .zip(&values)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    let (geometric, rows) = variability::group_means(&values, 13, 4, 1, 500, true, 7).unwrap();
    assert_eq!(rows, 13);
    assert_eq!(geometric, groups);
}

#[test]
fn the_subjects_which_are_not_finite_everywhere_are_removed_first() {
    let mut values = predictions(6, 3);
    values[4] = f64::NAN; // subject 1
    values[9] = f64::INFINITY; // subject 3
    values[17] = f64::NEG_INFINITY; // subject 5
    let (kept, rows) = variability::group_means(&values, 6, 3, 1, 10, false, 1).unwrap();
    assert_eq!(rows, 3);
    let expected: Vec<f64> = [0, 2, 4]
        .iter()
        .flat_map(|&s| values[s * 3..s * 3 + 3].to_vec())
        .collect();
    assert_eq!(kept, expected);
    // the groups are drawn from the finite subjects only
    let (groups, rows) = variability::group_means(&values, 6, 3, 4, 300, false, 1).unwrap();
    assert_eq!(rows, 300);
    assert!(groups.iter().all(|v| v.is_finite()));
    // no finite subject: no group, as the python skips the subjects
    let none = vec![f64::NAN; 6];
    assert_eq!(
        variability::group_means(&none, 3, 2, 4, 10, false, 1).unwrap(),
        (vec![], 0)
    );
}

#[test]
fn every_subject_is_drawn_equally_often_with_replacement() {
    // subject s predicts 1 at the point s and 0 elsewhere: the group mean at a point is the
    // fraction of the members which are that subject
    let n = 10;
    let values: Vec<f64> = (0..n * n)
        .map(|i| f64::from(u8::from(i / n == i % n)))
        .collect();
    let (groups, size) = (4000, 5);
    let (means, rows) = variability::group_means(&values, n, n, size, groups, false, 3).unwrap();
    assert_eq!((rows, means.len()), (groups, groups * n));
    let mut counts = vec![0.0; n];
    for group in means.chunks(n) {
        // the members of a group are `size` subjects
        let members: f64 = group.iter().map(|m| m * size as f64).sum();
        assert!((members - size as f64).abs() < 1e-9);
        for (count, m) in counts.iter_mut().zip(group) {
            *count += m * size as f64;
        }
    }
    // chi-square of 9 degrees of freedom, below its 99.9 % quantile 27.88
    let expected = (groups * size) as f64 / n as f64;
    let chi2: f64 = counts
        .iter()
        .map(|c| (c - expected) * (c - expected) / expected)
        .sum();
    assert!(chi2 < 27.88, "chi2 {chi2}, counts {counts:?}");
    // with replacement: a group of one subject five times
    let repeated = means
        .chunks(n)
        .filter(|group| group.iter().any(|&m| m > 0.5))
        .count();
    assert!(repeated > 0);
}

#[test]
fn the_group_means_have_the_mean_of_the_subjects_and_the_variance_of_a_mean() {
    let (subjects, points, size, groups) = (200, 3, 5, 20000);
    let values = predictions(subjects, points);
    let (means, rows) =
        variability::group_means(&values, subjects, points, size, groups, false, 11).unwrap();
    assert_eq!(rows, groups);
    for j in 0..points {
        let (mean, variance) = moments(&column(&values, points, j));
        let (found, found_variance) = moments(&column(&means, points, j));
        // within four standard errors of the mean of the group means
        let se = (variance / (size * groups) as f64).sqrt();
        assert!(
            (found - mean).abs() < 4.0 * se,
            "point {j}: {found} vs {mean} ({se})"
        );
        // the variance of a mean of `size` independent draws, within 5 %: its relative
        // standard error is about sqrt(2 / groups), 1 %
        let ratio = found_variance / (variance / size as f64);
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "point {j}: variance ratio {ratio}"
        );
    }
}

#[test]
fn the_geometric_mean_of_a_group_is_the_mean_of_the_logarithms_and_at_most_its_mean() {
    let (subjects, points, size, groups) = (200, 3, 4, 20000);
    let values = predictions(subjects, points);
    let (geometric, _) =
        variability::group_means(&values, subjects, points, size, groups, true, 5).unwrap();
    // the same seed draws the same members
    let (arithmetic, _) =
        variability::group_means(&values, subjects, points, size, groups, false, 5).unwrap();
    for (g, a) in geometric.iter().zip(&arithmetic) {
        assert!(*g <= a * (1.0 + 1e-15), "{g} > {a}");
    }
    for j in 0..points {
        let logs: Vec<f64> = column(&values, points, j)
            .iter()
            .map(|v| libm::log(*v))
            .collect();
        let (mean, variance) = moments(&logs);
        let found: Vec<f64> = column(&geometric, points, j)
            .iter()
            .map(|v| libm::log(*v))
            .collect();
        let (found, _) = moments(&found);
        let se = (variance / (size * groups) as f64).sqrt();
        assert!(
            (found - mean).abs() < 4.0 * se,
            "point {j}: {found} vs {mean} ({se})"
        );
    }
    // a zero is a geometric mean of 0, a negative value one which is not a number, as numpy
    let values = [0.0, 1.0, -1.0];
    let (geometric, rows) = variability::group_means(&values, 3, 1, 50, 20, true, 2).unwrap();
    assert_eq!(rows, 20);
    assert!(
        geometric
            .iter()
            .all(|g| *g == 0.0 || g.is_nan() || *g == 1.0)
    );
    assert!(geometric.iter().any(|g| g.is_nan()));
}

#[test]
fn a_seed_gives_the_same_groups_and_another_seed_others() {
    let values = predictions(30, 2);
    let first = variability::group_means(&values, 30, 2, 3, 100, false, 42).unwrap();
    let again = variability::group_means(&values, 30, 2, 3, 100, false, 42).unwrap();
    let other = variability::group_means(&values, 30, 2, 3, 100, false, 43).unwrap();
    assert_eq!(first, again);
    assert_ne!(first.0, other.0);
    let mut drawn = first.0;
    drawn.extend(
        variability::group_means(&values, 30, 2, 7, 50, true, 0)
            .unwrap()
            .0,
    );
    assert_eq!(
        hash(&drawn),
        0x7571_f072_dcad_c55b,
        "{} values",
        drawn.len()
    );
}

#[test]
fn the_edges_of_the_groups() {
    let values = predictions(4, 2);
    // no group
    assert_eq!(
        variability::group_means(&values, 4, 2, 3, 0, false, 1).unwrap(),
        (vec![], 0)
    );
    // a group of no subject is not a number, the mean of numpy of nothing
    let (empty, rows) = variability::group_means(&values, 4, 2, 0, 3, false, 1).unwrap();
    assert_eq!(rows, 3);
    assert!(empty.iter().all(|v| v.is_nan()));
    // no data point
    assert_eq!(
        variability::group_means(&[], 4, 0, 1, 3, false, 1).unwrap(),
        (vec![], 4)
    );
    assert_eq!(
        variability::group_means(&[], 4, 0, 2, 3, false, 1).unwrap(),
        (vec![], 3)
    );
    // values of another size are refused
    assert!(variability::group_means(&values, 3, 2, 2, 3, false, 1).is_err());
}

/// The ValueError of numpy of an array whose bytes exceed the largest `isize`.
const ARRAY_TOO_BIG: &str =
    "array is too big; `arr.size * arr.dtype.itemsize` is larger than the maximum possible size.";

#[test]
fn groups_too_many_to_hold_are_refused_not_wrapped() {
    let values = predictions(4, 2);
    let half = 1usize << (usize::BITS / 2);
    // more members than a usize counts, members and statistics whose bytes exceed the largest
    // isize, for the means and the groups of no subject
    for (size, groups) in [
        (2, usize::MAX / 2 + 1),
        (0, usize::MAX / 2 + 1),
        (3, usize::MAX / 2 + 1),
        (0, usize::MAX / 4 + 1),
        (2, usize::MAX / 4 + 1),
        (half, half),
    ] {
        let error = variability::group_means(&values, 4, 2, size, groups, false, 1).unwrap_err();
        assert_eq!(error.message, ARRAY_TOO_BIG, "{size} {groups}");
    }
}

#[test]
fn groups_at_no_data_point_draw_nothing() {
    // no statistic and no draw, only the number of the rows, at once for any number of groups
    let groups = 1 << (usize::BITS - 6);
    for (subjects, size, rows) in [(3, 2, groups), (3, 1, 3), (3, 0, groups), (0, 2, 0)] {
        let found = variability::group_means(&[], subjects, 0, size, groups, false, 1);
        assert_eq!(found, Ok((Vec::new(), rows)), "{subjects} {size}");
    }
    assert_eq!(
        variability::group_means(&[], 3, 0, 2, 0, false, 1),
        Ok((Vec::new(), 0))
    );
    // rows whose bytes exceed the largest isize, and members past what an array holds
    for (size, groups) in [(2, usize::MAX / 4), (usize::MAX / 2, 3)] {
        let error = variability::group_means(&[], 3, 0, size, groups, false, 1).unwrap_err();
        assert_eq!(error.message, ARRAY_TOO_BIG, "{size} {groups}");
    }
}

#[test]
fn bands_too_many_to_hold_are_refused_not_wrapped() {
    // two quantiles at data points whose bytes exceed the largest isize, of no group
    let error = variability::bands(&[], 0, usize::MAX / 8, &[0.05, 0.95]).unwrap_err();
    assert_eq!(error.message, ARRAY_TOO_BIG);
}

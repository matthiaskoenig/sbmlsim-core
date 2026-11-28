//! The kernels of the visual predictive check of a variability, as `predictive_check` of
//! `sbmlsim.fit.variability` computes them: the statistic of virtual groups of subjects
//! ([`group_means`], `_group_means`) and the bands of the distribution of the groups at every
//! data point ([`bands`]).
//!
//! The groups are resampled from the stream xoshiro256++ of sbmlsim-core of a seed, not from
//! the generator of numpy: one seed gives the same groups on every target, and the groups match
//! the python in their distribution. The bands are numpy's linear quantiles of the finite
//! values of every data point.

use sbmlsim_sensitivity::random::Stream;

use crate::numpy::numpy_quantile_sorted;
use crate::size::{array_len, buffer, filled};
use crate::uncertainty::QUANTILES_RANGE;
use crate::{Result, StatsError};

/// The statistic of virtual groups of `size` subjects at every data point, `_group_means`.
///
/// `values` are the predictions of `subjects` subjects at `points` data points, a row per
/// subject (row-major). The subjects which are not finite at every data point are removed
/// first, as `predictive_check` removes them before it groups them. A group of one subject is
/// the subject itself: the finite subjects are returned as they are. Otherwise `groups` groups
/// of `size` subjects are drawn with replacement from the finite ones (the members of a group
/// after the ones of the group before, as `rng.integers(0, n, size=(groups, size))`), and the
/// statistic of a group at a data point is the mean of its members (`np.mean`) or, for
/// `geometric`, their geometric mean `exp(mean(log(x)))` (0 of a zero, not a number of a
/// negative value, as numpy computes it). A group of no subject is not a number. Without a
/// finite subject there is no group (the python skips such subjects).
///
/// Returns the statistics, a row per group (or per finite subject of groups of one) and a
/// column per data point, and the number of rows. Every call draws from the stream of its own
/// `seed`: a caller which groups several sets of subjects gives each its own seed.
///
/// # Errors
///
/// For `values` which are not `subjects x points` values, and the `ValueError` of numpy
/// (`array is too big`) for `groups` whose members (`groups x size`) or statistics (`groups x
/// points`) are past what an array holds, their bytes past the largest `isize`, counted without
/// a wrap. That is no check of the memory: a size which can be counted is reserved, and where
/// the system overcommits it runs out of memory as numpy would.
pub fn group_means(
    values: &[f64],
    subjects: usize,
    points: usize,
    size: usize,
    groups: usize,
    geometric: bool,
    seed: u64,
) -> Result<(Vec<f64>, usize)> {
    if subjects.checked_mul(points) != Some(values.len()) {
        return Err(StatsError::value(format!(
            "The predictions of {subjects} subjects at {points} data points are {subjects} x \
             {points} values, not {}.",
            values.len()
        )));
    }
    if points == 0 {
        // no data point: no statistic and no draw, only the number of the rows, which numpy
        // counts as it counts the rows of the members and of the statistics
        let rows = if size == 1 {
            subjects
        } else if subjects == 0 || groups == 0 {
            0
        } else {
            array_len::<i64>(&[groups, size])?;
            groups
        };
        array_len::<f64>(&[rows, points])?;
        return Ok((Vec::new(), rows));
    }
    // a row per finite subject, at most one per row of the values
    let finite: Vec<&[f64]> = values
        .chunks(points)
        .filter(|row| row.iter().all(|v| v.is_finite()))
        .collect();
    if size == 1 {
        return Ok((finite.concat(), finite.len()));
    }
    if finite.is_empty() || groups == 0 {
        return Ok((Vec::new(), 0));
    }
    // the members, `rng.integers(0, n, size=(groups, size))` of the python, and the statistics
    array_len::<i64>(&[groups, size])?;
    let statistics_count = array_len::<f64>(&[groups, points])?;
    let mut statistics = buffer(statistics_count)?;
    if size == 0 {
        // the mean of numpy of no member
        statistics.resize(statistics_count, f64::NAN);
        return Ok((statistics, groups));
    }
    // the members of a group after the ones of the group before, drawn as the group is summed
    let mut stream = Stream::new(seed);
    let mut sums = vec![0.0; points];
    for _ in 0..groups {
        // the sums along the members, in their order, divided by their number
        sums.fill(0.0);
        for _ in 0..size {
            let member = finite[stream.below(finite.len())];
            for (sum, &x) in sums.iter_mut().zip(member) {
                *sum += if geometric { libm::log(x) } else { x };
            }
        }
        statistics.extend(sums.iter().map(|sum| {
            let mean = sum / size as f64;
            if geometric { libm::exp(mean) } else { mean }
        }));
    }
    Ok((statistics, groups))
}

/// The bands of a distribution at every data point, the quantiles of `predictive_check`.
///
/// `distribution` is the statistic of `rows` groups at `cols` data points, a row per group
/// (row-major). The band of a data point (a column) is the quantiles `qs` of its finite values,
/// `np.quantile` of the linear method, `nan` for a column without a finite value. Returns a row
/// per data point and a column per quantile.
///
/// # Errors
///
/// For a distribution which is not `rows x cols` values, the ValueError of numpy
/// (`Quantiles must be in the range [0, 1]`) of a quantile outside `[0, 1]` or not a number,
/// where a column has a finite value (numpy is called for those only), and numpy's `array is
/// too big` for bands (`cols x qs.len()`) past what an array holds.
pub fn bands(distribution: &[f64], rows: usize, cols: usize, qs: &[f64]) -> Result<Vec<f64>> {
    if rows.checked_mul(cols) != Some(distribution.len()) {
        return Err(StatsError::value(format!(
            "The distribution of {rows} groups at {cols} data points is {rows} x {cols} values, \
             not {}.",
            distribution.len()
        )));
    }
    let mut bands = filled(array_len::<f64>(&[cols, qs.len()])?, f64::NAN)?;
    for j in 0..cols {
        let mut column: Vec<f64> = (0..rows)
            .map(|i| distribution[i * cols + j])
            .filter(|v| v.is_finite())
            .collect();
        if column.is_empty() {
            continue;
        }
        if !qs.iter().all(|q| (0.0..=1.0).contains(q)) {
            return Err(StatsError::value(QUANTILES_RANGE.to_owned()));
        }
        column.sort_by(f64::total_cmp);
        for (band, &q) in bands[j * qs.len()..(j + 1) * qs.len()].iter_mut().zip(qs) {
            *band = numpy_quantile_sorted(&column, q);
        }
    }
    Ok(bands)
}

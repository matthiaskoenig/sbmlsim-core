//! The bootstrap of the intervals of the estimators.

use crate::normal::normal_quantile;
use crate::random::Stream;
use crate::{Result, SensitivityError};

/// The resamples of a bootstrap.
#[derive(Debug, Clone, Copy)]
pub enum Resamples<'a> {
    /// `count` resamples drawn from the stream of `seed` ([`crate::random::Stream`]).
    Seeded {
        /// The number of resamples, at least one.
        count: usize,
        /// The seed of the stream.
        seed: u64,
    },
    /// Given resamples: the drawn indices, row-major, of the shape the estimator names (the
    /// points of a sample of Sobol, the starts of the windows of FAST, the trajectories of
    /// Morris). The resamples which SALib draws are given so, which is how the tests compare
    /// the intervals with SALib's.
    Given(&'a [usize]),
}

/// The bootstrap of the intervals of an estimator.
#[derive(Debug, Clone, Copy)]
pub struct Bootstrap<'a> {
    /// The level of the intervals, in `(0, 1)`.
    pub conf_level: f64,
    /// The resamples.
    pub resamples: Resamples<'a>,
}

impl Bootstrap<'_> {
    /// The factor of the standard deviation over the resamples: the quantile
    /// `0.5 + conf_level / 2` of the standard normal.
    pub(crate) fn z(&self) -> Result<f64> {
        if !(self.conf_level > 0.0 && self.conf_level < 1.0) {
            return Err(SensitivityError::new(format!(
                "The confidence level is in (0, 1), not {}.",
                self.conf_level
            )));
        }
        Ok(normal_quantile(0.5 + self.conf_level / 2.0))
    }

    /// The drawn indices of `groups` groups of resamples (the parameters of Morris and FAST,
    /// one group for Sobol), each resample `size` indices below `below`: the given ones, checked,
    /// or the ones of the seed, a group after the other, a resample after the other.
    ///
    /// Returns the indices, row-major `(groups, resamples, size)`, and the number of resamples
    /// per group.
    pub(crate) fn indices(
        &self,
        groups: usize,
        size: usize,
        below: usize,
        shape: &str,
    ) -> Result<(Vec<usize>, usize)> {
        match self.resamples {
            Resamples::Seeded { count, seed } => {
                if count == 0 {
                    return Err(SensitivityError::new(
                        "The number of resamples is at least 1, not 0.",
                    ));
                }
                let mut stream = Stream::new(seed);
                let total = groups * count * size;
                let indices = (0..total).map(|_| stream.below(below)).collect();
                Ok((indices, count))
            }
            Resamples::Given(indices) => {
                let per = groups * size;
                if per == 0 || indices.is_empty() || !indices.len().is_multiple_of(per) {
                    return Err(SensitivityError::new(format!(
                        "The given resamples are {shape}, not {} indices.",
                        indices.len()
                    )));
                }
                if let Some(index) = indices.iter().find(|&&index| index >= below) {
                    return Err(SensitivityError::new(format!(
                        "A given resample draws {index}, an index of {below} draws is below \
                         {below}."
                    )));
                }
                Ok((indices.to_vec(), indices.len() / per))
            }
        }
    }
}

/// How often every one of `n` rows is drawn by every resample of `size` drawn indices: a mean
/// over a resample is the mean of the rows weighted by these counts.
pub(crate) fn counts(indices: &[usize], size: usize, n: usize) -> Vec<f64> {
    let resamples = indices.len() / size;
    let mut counts = vec![0.0; resamples * n];
    for (r, drawn) in indices.chunks_exact(size).enumerate() {
        for &index in drawn {
            counts[r * n + index] += 1.0;
        }
    }
    counts
}

/// The weighted sums `counts @ values` of the resamples: a row of `columns` sums per resample,
/// over the `n` rows of `values` (`row(i)` the row `i`), each weighted by its count.
pub(crate) fn weighted_sums<'a>(
    counts: &[f64],
    n: usize,
    columns: usize,
    row: impl Fn(usize) -> &'a [f64],
) -> Vec<f64> {
    let resamples = counts.len() / n;
    let mut sums = vec![0.0; resamples * columns];
    for (r, weights) in counts.chunks_exact(n).enumerate() {
        let sum = &mut sums[r * columns..(r + 1) * columns];
        for (i, &weight) in weights.iter().enumerate() {
            if weight != 0.0 {
                for (s, v) in sum.iter_mut().zip(row(i)) {
                    *s += weight * v;
                }
            }
        }
    }
    sums
}

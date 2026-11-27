//! The reductions of numpy on doubles, to the last bits: the pairwise sum of `np.sum` and
//! `np.mean`, the variance of `np.var` and `np.std` and the linear quantile of `np.quantile`.

/// The block of the pairwise sum of numpy (`PW_BLOCKSIZE`), below which it sums in eight
/// lanes.
const PAIRWISE_BLOCK: usize = 128;

/// The sum of `value(start..start + n)` as numpy sums a contiguous array of doubles
/// (`np.add.reduce`, `pairwise_sum` of its loops): below eight values in order, up to a block
/// in eight lanes which are added as a tree, above that the halves (cut at a multiple of
/// eight) recursively.
fn pairwise(start: usize, n: usize, value: &impl Fn(usize) -> f64) -> f64 {
    if n < 8 {
        let mut sum = 0.0;
        for i in start..start + n {
            sum += value(i);
        }
        sum
    } else if n <= PAIRWISE_BLOCK {
        let mut lanes = [0.0; 8];
        for (j, lane) in lanes.iter_mut().enumerate() {
            *lane = value(start + j);
        }
        let mut i = 8;
        while i < n - n % 8 {
            for (j, lane) in lanes.iter_mut().enumerate() {
                *lane += value(start + i + j);
            }
            i += 8;
        }
        let mut sum = ((lanes[0] + lanes[1]) + (lanes[2] + lanes[3]))
            + ((lanes[4] + lanes[5]) + (lanes[6] + lanes[7]));
        for k in i..n {
            sum += value(start + k);
        }
        sum
    } else {
        let half = n / 2;
        let half = half - half % 8;
        pairwise(start, half, value) + pairwise(start + half, n - half, value)
    }
}

/// The sum of `value(0..n)` as `np.sum` of the array of the values: the reduction starts at
/// the identity 0 and adds the pairwise sum of all of them (numpy 2.5 over the whole of a
/// contiguous array, not in blocks of its buffer).
pub(crate) fn numpy_sum(n: usize, value: impl Fn(usize) -> f64) -> f64 {
    0.0 + pairwise(0, n, &value)
}

/// The mean of `value(0..n)` as `np.mean`: the sum divided by the count, `nan` of none.
pub(crate) fn numpy_mean(n: usize, value: impl Fn(usize) -> f64) -> f64 {
    numpy_sum(n, value) / n as f64
}

/// The variance of `values` as `np.var(values, ddof=ddof)`: the mean of numpy, the squares of
/// the deviations from it summed pairwise and divided by `n - ddof`.
pub(crate) fn numpy_var(values: &[f64], ddof: usize) -> f64 {
    let n = values.len();
    let mean = numpy_mean(n, |i| values[i]);
    let squares = numpy_sum(n, |i| {
        let deviation = values[i] - mean;
        deviation * deviation
    });
    squares / n.saturating_sub(ddof) as f64
}

/// The quantile `q` in `[0, 1]` of `sorted`, values in ascending order without a not a
/// number, as `np.quantile` of method `linear` interpolates it: at the virtual index
/// `(n - 1) q` between its neighbours, from the lower one below a weight of one half and from
/// the upper one above (`_lerp`), so an infinite neighbour gives an infinity or not a number
/// as numpy does. At the last index numpy takes the last value with the weight `(n - 1) q + 1`
/// (its previous index -1), so an infinite last value is not a number there. Not a number of
/// no values.
pub(crate) fn numpy_quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    let last = (n - 1) as f64;
    let virtual_index = last * q;
    let (previous, next, gamma) = if virtual_index >= last {
        (n - 1, n - 1, virtual_index + 1.0)
    } else {
        let floor = libm::floor(virtual_index);
        let previous = floor as usize;
        (previous, previous + 1, virtual_index - floor)
    };
    let (a, b) = (sorted[previous], sorted[next]);
    let difference = b - a;
    if gamma >= 0.5 {
        b - difference * (1.0 - gamma)
    } else {
        a + difference * gamma
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairwise_sum_of_few_values_is_the_sum_in_order() {
        assert_eq!(numpy_sum(0, |_| 1.0), 0.0);
        assert!(numpy_mean(0, |_| 1.0).is_nan());
        let values = [1.0, 2.0, 3.0];
        assert_eq!(numpy_sum(3, |i| values[i]), 6.0);
    }

    #[test]
    fn the_variance_and_the_quantile_of_few_values() {
        assert_eq!(numpy_var(&[1.0, 2.0, 3.0], 0), 2.0 / 3.0);
        assert_eq!(numpy_var(&[1.0, 2.0, 3.0], 1), 1.0);
        assert!(numpy_quantile_sorted(&[], 0.5).is_nan());
        assert_eq!(numpy_quantile_sorted(&[1.0, 2.0], 0.25), 1.25);
        assert_eq!(numpy_quantile_sorted(&[1.0, 2.0], 1.0), 2.0);
        // from the lower neighbour an infinity, from the upper one infinity less infinity
        assert_eq!(
            numpy_quantile_sorted(&[1.0, f64::INFINITY], 0.25),
            f64::INFINITY
        );
        assert!(numpy_quantile_sorted(&[1.0, f64::INFINITY], 0.5).is_nan());
        assert!(numpy_quantile_sorted(&[1.0, f64::INFINITY], 1.0).is_nan());
        assert!(numpy_quantile_sorted(&[f64::INFINITY], 0.0).is_nan());
    }
}

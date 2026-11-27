//! The extended FAST: the search curves of its design and the spectra of their values.

use std::f64::consts::PI;
use std::ops::Range;

use crate::bootstrap::Bootstrap;
use crate::random::Stream;
use crate::{
    Columns, EPSILON, Matrix, MatrixRef, Result, SensitivityError, by_columns, check_parameters,
    interval,
};

/// The elements an estimate of the spectra holds at once: the prefix sums of a bin are a row of
/// this many values per point, so the memory of an estimate does not grow with the elements.
const CHUNK: usize = 64;

/// Check the interference factor and the points per parameter of a FAST design.
fn check_points(n: usize, m: usize) -> Result<()> {
    if m == 0 {
        return Err(SensitivityError::new(
            "The interference factor m of a FAST design is at least 1, not 0.",
        ));
    }
    if n <= 4 * m * m {
        return Err(SensitivityError::new(format!(
            "The n of a FAST design is more than 4 m^2 = {}, not {n}.",
            4 * m * m
        )));
    }
    Ok(())
}

/// `numpy.linspace(start, stop, num)`: its values in its operations, its last value the stop.
fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return Vec::new();
    }
    if num == 1 {
        return vec![start];
    }
    let step = (stop - start) / (num - 1) as f64;
    let mut values: Vec<f64> = (0..num)
        .map(|k| {
            if step == 0.0 {
                start
            } else {
                k as f64 * step + start
            }
        })
        .collect();
    values[num - 1] = stop;
    values
}

/// The frequencies of a FAST design of `d` parameters, `n` points per parameter and the
/// interference factor `m`, those of `SALib.sample.fast_sampler`: the parameter of a block
/// runs at `omega = floor((n - 1) / (2 m))`, the others at the frequencies up to
/// `floor(omega / (2 m))`, spread over them or repeated.
fn frequencies(d: usize, n: usize, m: usize) -> Vec<usize> {
    let omega = (n - 1) / (2 * m);
    let mut frequencies = vec![omega];
    let highest = omega / (2 * m);
    if highest >= d - 1 {
        frequencies.extend(
            linspace(1.0, highest as f64, d - 1)
                .into_iter()
                .map(|f| f.floor() as usize),
        );
    } else {
        frequencies.extend((0..d - 1).map(|k| k % highest + 1));
    }
    frequencies
}

/// The phases of the search curves of a FAST design of `d` parameters drawn from the stream
/// of `seed`: `2 pi u` of a uniform `u` per block, in the order of the blocks.
#[must_use]
pub fn fast_phases(d: usize, seed: u64) -> Vec<f64> {
    let mut stream = Stream::new(seed);
    (0..d).map(|_| 2.0 * PI * stream.uniform()).collect()
}

/// The design of the extended FAST of `d` parameters, `n` points per parameter: a block of
/// `n` points per parameter, a row per point.
///
/// The search curves of `SALib.sample.fast_sampler.sample` with the phases of the stream of
/// `seed` ([`fast_phases`]), see [`fast_sample_with_phases`].
///
/// # Errors
///
/// See [`fast_sample_with_phases`].
pub fn fast_sample(d: usize, n: usize, m: usize, seed: u64) -> Result<Matrix> {
    fast_sample_with_phases(d, n, m, &fast_phases(d, seed))
}

/// The design of the extended FAST with given phases, a phase per block.
///
/// The block of the parameter `i` walks the search curve
/// `x_j = 0.5 + asin(sin(omega_j s + phi_i)) / pi` over `s = 2 pi k / n`, `k = 0..n`, with the
/// highest frequency for the parameter `i` and the lower frequencies for the others (in their
/// order), the curve and the frequencies of `SALib.sample.fast_sampler.sample` (Saltelli et al.
/// 1999), whose phases are random.
///
/// # Errors
///
/// If `d` is zero, `m` is zero, `n` is not more than `4 m^2` or there is not one phase per
/// parameter.
pub fn fast_sample_with_phases(d: usize, n: usize, m: usize, phases: &[f64]) -> Result<Matrix> {
    check_parameters(d)?;
    check_points(n, m)?;
    if phases.len() != d {
        return Err(SensitivityError::new(format!(
            "A FAST design of {d} parameters has a phase per parameter, not {}.",
            phases.len()
        )));
    }
    let frequencies = frequencies(d, n, m);
    let resolution = 2.0 * PI / n as f64;
    let mut points = Matrix::zeros(n * d, d);
    for (i, &phase) in phases.iter().enumerate() {
        // the highest frequency for the parameter of the block, the others in their order
        let mut lower = frequencies[1..].iter();
        let block: Vec<f64> = (0..d)
            .map(|j| {
                let frequency = if j == i {
                    frequencies[0]
                } else {
                    *lower.next().expect("a frequency per parameter")
                };
                frequency as f64
            })
            .collect();
        for k in 0..n {
            let s = resolution * k as f64;
            let row = &mut points.data[(i * n + k) * d..(i * n + k + 1) * d];
            for (x, &frequency) in row.iter_mut().zip(&block) {
                *x = 0.5 + (1.0 / PI) * libm::asin(libm::sin(frequency * s + phase));
            }
        }
    }
    Ok(points)
}

/// The indices of the extended FAST of the parameters, every column of the values estimated at
/// once.
#[derive(Debug, Clone, PartialEq)]
pub struct FastIndices {
    /// The first order indices `S1`, a row per parameter and a column per element.
    pub s1: Matrix,
    /// Their intervals `S1_conf`.
    pub s1_conf: Matrix,
    /// The total indices `ST`.
    pub st: Matrix,
    /// Their intervals `ST_conf`.
    pub st_conf: Matrix,
}

impl Columns for FastIndices {
    fn join(parts: Vec<Self>) -> Self {
        let join = |field: fn(&Self) -> &Matrix| {
            Matrix::join(&parts.iter().map(field).collect::<Vec<_>>())
        };
        Self {
            s1: join(|p| &p.s1),
            s1_conf: join(|p| &p.s1_conf),
            st: join(|p| &p.st),
            st_conf: join(|p| &p.st_conf),
        }
    }
}

/// The block of a parameter: `n` rows of the values from the row `first`.
#[derive(Clone, Copy)]
struct Block<'a> {
    y: MatrixRef<'a>,
    first: usize,
    n: usize,
}

/// The first and the total order of the windows of a block in the columns `range`: per start,
/// the window of `length` points from it, which wraps around the end of the block.
///
/// `S1` is the power of the output at the harmonics `1..m` of the frequency `omega` and `ST`
/// what is left of the power above the interferences below `omega / 2`, each over the power of
/// the spectrum up to the Nyquist frequency, the estimators of `SALib.analyze.fast` on the
/// transform of a real signal. The transform is needed at those bins only, and the transform of
/// a window at a bin is the difference of two prefix sums of the values of the block times the
/// twiddles of the bin, so a bootstrap of many windows costs little more than one transform of
/// the block. The power of the whole spectrum is the variance of the window less its Nyquist
/// bin (Parseval).
///
/// Returns `S1` and `ST`, a row of the columns per start; zero for a window without power.
fn window_orders(
    block: Block<'_>,
    range: Range<usize>,
    length: usize,
    starts: &[usize],
    m: usize,
    omega: usize,
) -> (Vec<f64>, Vec<f64>) {
    let width = range.len();
    let n = block.n;
    let extent = starts.iter().max().expect("a window") + length;
    // the values of the block centred on its mean, extended periodically
    let mut mean = vec![0.0; width];
    for j in 0..n {
        crate::add_row(&mut mean, &block.y.row(block.first + j)[range.clone()]);
    }
    for value in &mut mean {
        *value /= n as f64;
    }
    let mut x = vec![0.0; extent * width];
    for t in 0..extent {
        let row = &block.y.row(block.first + t % n)[range.clone()];
        for e in 0..width {
            x[t * width + e] = row[e] - mean[e];
        }
    }
    // the prefix sums of the values and of their squares
    let mut sums = vec![0.0; (extent + 1) * width];
    let mut squares = vec![0.0; (extent + 1) * width];
    for t in 0..extent {
        for e in 0..width {
            let v = x[t * width + e];
            sums[(t + 1) * width + e] = sums[t * width + e] + v;
            squares[(t + 1) * width + e] = squares[t * width + e] + v * v;
        }
    }
    let (cos, sin): (Vec<f64>, Vec<f64>) = (0..length)
        .map(|r| {
            let angle = 2.0 * PI * r as f64 / length as f64;
            (libm::cos(angle), libm::sin(angle))
        })
        .unzip();
    // the bins: the interferences, the harmonics and, for a window of an even length, the
    // Nyquist bin, which the power of the spectrum leaves out
    enum Kind {
        Interference,
        Harmonic,
        Nyquist,
    }
    let mut bins: Vec<(usize, Kind)> = (1..=omega / 2).map(|k| (k, Kind::Interference)).collect();
    bins.extend((1..=m).map(|p| (p * omega, Kind::Harmonic)));
    if length.is_multiple_of(2) {
        bins.push((length / 2, Kind::Nyquist));
    }
    let windows = starts.len();
    let mut first = vec![0.0; windows * width];
    let mut interference = vec![0.0; windows * width];
    let mut nyquist = vec![0.0; windows * width];
    let mut re = vec![0.0; (extent + 1) * width];
    let mut im = vec![0.0; (extent + 1) * width];
    let scale = (length * length) as f64;
    for (k, kind) in bins {
        let mut r = 0usize;
        for t in 0..extent {
            let (c, s) = (cos[r], sin[r]);
            for e in 0..width {
                let v = x[t * width + e];
                re[(t + 1) * width + e] = re[t * width + e] + v * c;
                im[(t + 1) * width + e] = im[t * width + e] - v * s;
            }
            r += k;
            if r >= length {
                r -= length;
            }
        }
        let target = match kind {
            Kind::Interference => &mut interference,
            Kind::Harmonic => &mut first,
            Kind::Nyquist => &mut nyquist,
        };
        for (q, &start) in starts.iter().enumerate() {
            let (low, high) = (start * width, (start + length) * width);
            for e in 0..width {
                let a = re[high + e] - re[low + e];
                let b = im[high + e] - im[low + e];
                target[q * width + e] += (a * a + b * b) / scale;
            }
        }
    }
    let mut s1 = vec![0.0; windows * width];
    let mut st = vec![0.0; windows * width];
    for (q, &start) in starts.iter().enumerate() {
        let (low, high) = (start * width, (start + length) * width);
        for e in 0..width {
            let sum = sums[high + e] - sums[low + e];
            let centred = squares[high + e] - squares[low + e] - sum * sum / length as f64;
            let whole = centred / length as f64 - nyquist[q * width + e];
            if whole > EPSILON {
                s1[q * width + e] = 2.0 * first[q * width + e] / whole;
                st[q * width + e] = 1.0 - 2.0 * interference[q * width + e] / whole;
            }
        }
    }
    (s1, st)
}

/// The indices of the extended FAST of the values of the points of a FAST design
/// ([`fast_sample`]).
///
/// The points of a parameter are the block of the design which walks the search curve at its
/// frequency `omega = floor((n - 1) / (2 m))`, see `SALib.analyze.fast`. An interval is
/// `z(conf_level)` times the standard deviation of the orders over windows of `ceil(n / 2)`
/// points of the block, which wrap around its end and whose frequency is the one of their own
/// length, from starts drawn per parameter and resample; given resamples are row-major
/// `(d, resamples)`, the starts of the windows. SALib documents these intervals as unreliable,
/// so they are indicative only.
///
/// The elements are spread over at most `threads` threads, and an index does not depend on
/// them.
///
/// # Errors
///
/// If the number of rows is no multiple of `d`, a block has not more than `4 m^2` points, or
/// the bootstrap is not valid.
pub fn fast_indices(
    y: MatrixRef<'_>,
    d: usize,
    m: usize,
    bootstrap: &Bootstrap<'_>,
    threads: usize,
) -> Result<FastIndices> {
    by_columns(y, threads, |part| fast_part(part, d, m, bootstrap))
}

/// The indices of the extended FAST of the elements of `y` on one thread, see
/// [`fast_indices`].
fn fast_part(
    y: MatrixRef<'_>,
    d: usize,
    m: usize,
    bootstrap: &Bootstrap<'_>,
) -> Result<FastIndices> {
    check_parameters(d)?;
    let rows = y.rows();
    if rows == 0 || !rows.is_multiple_of(d) {
        return Err(SensitivityError::new(format!(
            "The design of {d} parameters has a block of points per parameter, which {rows} \
             rows are no multiple of."
        )));
    }
    let n = rows / d;
    check_points(n, m)?;
    let z = bootstrap.z()?;
    let (starts, resamples) =
        bootstrap.indices(d, 1, n, "(d, resamples) of the starts of the windows")?;
    let omega = (n - 1) / (2 * m);
    let window = n.div_ceil(2);
    let inside_omega = (window - 1) / (2 * m);
    let elements = y.columns();
    let mut found = FastIndices {
        s1: Matrix::zeros(d, elements),
        s1_conf: Matrix::zeros(d, elements),
        st: Matrix::zeros(d, elements),
        st_conf: Matrix::zeros(d, elements),
    };
    for i in 0..d {
        let block = Block { y, first: i * n, n };
        let starts = &starts[i * resamples..(i + 1) * resamples];
        for low in (0..elements).step_by(CHUNK) {
            let range = low..(low + CHUNK).min(elements);
            let width = range.len();
            let (s1, st) = window_orders(block, range.clone(), n, &[0], m, omega);
            let (s1_boot, st_boot) =
                window_orders(block, range.clone(), window, starts, m, inside_omega);
            let at = i * elements + low;
            found.s1.data[at..at + width].copy_from_slice(&s1);
            found.st.data[at..at + width].copy_from_slice(&st);
            found.s1_conf.data[at..at + width]
                .copy_from_slice(&interval(&s1_boot, resamples, width, z));
            found.st_conf.data[at..at + width]
                .copy_from_slice(&interval(&st_boot, resamples, width, z));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resamples;

    #[test]
    fn the_frequencies_are_those_of_salib() {
        // SALib.sample.fast_sampler: omega[0] and omega[1:] for (d, n, m)
        assert_eq!(frequencies(3, 65, 4), [8, 1, 1]);
        assert_eq!(frequencies(4, 1025, 4), [128, 1, 8, 16]);
        assert_eq!(frequencies(5, 257, 2), [64, 1, 6, 11, 16]);
        assert_eq!(frequencies(4, 257, 4), [32, 1, 2, 4]);
        assert_eq!(frequencies(1, 65, 4), [8]);
    }

    #[test]
    fn a_block_walks_its_curve() {
        let points = fast_sample(3, 65, 4, 7).unwrap();
        assert_eq!((points.rows, points.columns), (195, 3));
        assert!(points.data.iter().all(|x| (0.0..=1.0).contains(x)));
        let phases = fast_phases(3, 7);
        let again = fast_sample_with_phases(3, 65, 4, &phases).unwrap();
        assert_eq!(points, again);
        // the first point of every block is the phase of the block on every curve
        for (i, phase) in phases.iter().enumerate() {
            let expected = 0.5 + (1.0 / PI) * libm::asin(libm::sin(*phase));
            for j in 0..3 {
                assert_eq!(points.get(i * 65, j), expected);
            }
        }
    }

    #[test]
    fn the_arguments_of_a_design_are_checked() {
        assert!(fast_sample(2, 64, 4, 1).is_err());
        assert!(fast_sample(2, 65, 0, 1).is_err());
        assert!(fast_sample(0, 65, 4, 1).is_err());
        assert!(fast_sample_with_phases(2, 65, 4, &[0.0]).is_err());
    }

    fn ishigami(points: &Matrix) -> Vec<f64> {
        (0..points.rows)
            .map(|i| {
                let x: Vec<f64> = points.row(i).iter().map(|u| u * 2.0 * PI - PI).collect();
                x[0].sin() + 7.0 * x[1].sin().powi(2) + 0.1 * x[2].powi(4) * x[0].sin()
            })
            .collect()
    }

    #[test]
    fn the_indices_of_the_ishigami_function() {
        let points = fast_sample(3, 1025, 4, 3).unwrap();
        let y = ishigami(&points);
        let found = fast_indices(
            MatrixRef::new(&y, points.rows, 1).unwrap(),
            3,
            4,
            &Bootstrap {
                conf_level: 0.95,
                resamples: Resamples::Seeded {
                    count: 100,
                    seed: 11,
                },
            },
            1,
        )
        .unwrap();
        for (j, s1) in [0.314, 0.442, 0.0].into_iter().enumerate() {
            assert!((found.s1.get(j, 0) - s1).abs() < 0.06, "{j}");
            assert!(found.s1_conf.get(j, 0) > 0.0);
        }
        assert!(found.st.get(0, 0) > found.st.get(2, 0));
    }

    /// The orders of a window computed by a direct transform of its values at every bin.
    fn direct(values: &[f64], m: usize, omega: usize) -> (f64, f64) {
        let n = values.len();
        let power: Vec<f64> = (1..n.div_ceil(2))
            .map(|k| {
                let (mut a, mut b) = (0.0, 0.0);
                for (j, v) in values.iter().enumerate() {
                    let angle = 2.0 * PI * ((k * j) % n) as f64 / n as f64;
                    a += v * angle.cos();
                    b -= v * angle.sin();
                }
                (a * a + b * b) / (n * n) as f64
            })
            .collect();
        let whole = 2.0 * power.iter().sum::<f64>();
        let first = 2.0 * (1..=m).map(|p| power[p * omega - 1]).sum::<f64>();
        let interference = 2.0 * power[..omega / 2].iter().sum::<f64>();
        (first / whole, 1.0 - interference / whole)
    }

    #[test]
    fn the_prefix_sums_give_the_transform_of_every_window() {
        for n in [65, 66, 257] {
            let points = fast_sample(2, n, 4, 5).unwrap();
            let y: Vec<f64> = (0..points.rows)
                .map(|i| {
                    let x = points.row(i);
                    100.0 + x[0] + 0.3 * x[1] * x[1] + 0.1 * (6.0 * x[0]).sin()
                })
                .collect();
            let window = n.div_ceil(2);
            let inside = (window - 1) / 8;
            let starts = [0, 3, n - 1, n / 2, 3];
            let given = [&starts[..], &starts[..]].concat();
            let found = fast_indices(
                MatrixRef::new(&y, points.rows, 1).unwrap(),
                2,
                4,
                &Bootstrap {
                    conf_level: 0.9,
                    resamples: Resamples::Given(&given),
                },
                1,
            )
            .unwrap();
            let block: Vec<f64> = y[..n].to_vec();
            let (s1, st) = direct(&block, 4, (n - 1) / 8);
            assert!((found.s1.get(0, 0) - s1).abs() < 1e-12, "{n}");
            assert!((found.st.get(0, 0) - st).abs() < 1e-12, "{n}");
            let block = Block {
                y: MatrixRef::new(&y, points.rows, 1).unwrap(),
                first: 0,
                n,
            };
            let (s1_boot, st_boot) = window_orders(block, 0..1, window, &starts, 4, inside);
            for (q, &start) in starts.iter().enumerate() {
                let values: Vec<f64> = (0..window).map(|t| y[(start + t) % n]).collect();
                let (s1, st) = direct(&values, 4, inside);
                assert!((s1_boot[q] - s1).abs() < 1e-12, "{n} {start}");
                assert!((st_boot[q] - st).abs() < 1e-12, "{n} {start}");
            }
        }
    }
}

//! The Sobol indices: the Sobol' sequence, the matrix of Saltelli on it and the estimators of
//! the first order, total and second order indices.

use std::sync::OnceLock;

use crate::bootstrap::{Bootstrap, counts, weighted_sums};
use crate::random::Stream;
use crate::{
    Columns, EPSILON, Matrix, MatrixRef, Result, SensitivityError, by_columns, check_parameters,
    interval, quotient,
};

/// The bits of a point of the Sobol' sequence, the default of `scipy.stats.qmc.Sobol`: a
/// coordinate is an integer of `BITS` bits times `2^-BITS`, and a sequence has at most
/// `2^BITS` points.
pub const BITS: u32 = 30;

/// The dimensions of the Sobol' sequence, those of the direction numbers the crate carries:
/// the first 2048 of the 21201 of Joe and Kuo, so a matrix of Saltelli, which takes two
/// dimensions per parameter, has at most 1024 parameters.
pub const MAX_DIMENSIONS: usize = 2048;

/// The direction numbers of Joe and Kuo, `new-joe-kuo-6.21201` up to the dimension 2048: a
/// line per dimension from the second, its degree `s`, the coefficients `a` of its primitive
/// polynomial and its initial direction numbers `m_1..m_s` (`data/LICENSE-joe-kuo`).
const JOE_KUO: &str = include_str!("../data/new-joe-kuo-6.2048");

/// The direction numbers of every dimension, `BITS` per dimension, each shifted to its bit.
fn directions() -> &'static [[u32; BITS as usize]] {
    static DIRECTIONS: OnceLock<Vec<[u32; BITS as usize]>> = OnceLock::new();
    DIRECTIONS.get_or_init(|| {
        let bits = BITS as usize;
        let mut all = Vec::with_capacity(MAX_DIMENSIONS);
        // the first dimension, whose direction numbers are all one: the van der Corput sequence
        all.push(std::array::from_fn(|j| 1u32 << (bits - 1 - j)));
        for line in JOE_KUO.lines().skip(1) {
            let fields: Vec<u64> = line
                .split_whitespace()
                .map(|field| field.parse().expect("a number of the direction numbers"))
                .collect();
            let (s, a) = (fields[1] as usize, fields[2]);
            let initial = &fields[3..3 + s];
            // the primitive polynomial with its leading and its constant term, as scipy reads it
            let polynomial = (1u64 << s) | (a << 1) | 1;
            let mut v = [0u64; BITS as usize];
            v[..s.min(bits)].copy_from_slice(&initial[..s.min(bits)]);
            // the recurrence of Bratley and Fox 1988, Algorithm 659
            for j in s..bits {
                let mut next = v[j - s];
                let mut power = 1u64;
                for k in 0..s {
                    power <<= 1;
                    if (polynomial >> (s - 1 - k)) & 1 == 1 {
                        next ^= power * v[j - k - 1];
                    }
                }
                v[j] = next;
            }
            all.push(std::array::from_fn(|j| {
                u32::try_from(v[j] << (bits - 1 - j)).expect("a direction number of BITS bits")
            }));
        }
        assert_eq!(
            all.len(),
            MAX_DIMENSIONS,
            "the dimensions of the direction numbers"
        );
        all
    })
}

/// Scramble the direction numbers of a dimension by a lower triangular matrix of unit diagonal:
/// the linear matrix scrambling of `scipy.stats.qmc.Sobol(scramble=True)` (LMS).
///
/// The row `p` of the matrix is an integer whose bits are its columns from the top, the column
/// `c` the bit `BITS - 1 - c`, and the bit `p` from the top of a scrambled direction number is
/// the parity of the direction number and that row.
fn scrambled(v: &[u32; BITS as usize], rows: &[u32; BITS as usize]) -> [u32; BITS as usize] {
    std::array::from_fn(|j| {
        rows.iter().enumerate().fold(0u32, |out, (p, row)| {
            out | (((row & v[j]).count_ones() & 1) << (BITS as usize - 1 - p))
        })
    })
}

/// Draw the scrambling of a dimension from the stream: its digital shift, `BITS` random bits,
/// and the rows of its lower triangular matrix of unit diagonal, whose bits left of the
/// diagonal are random (LMS+shift, as `scipy.stats.qmc.Sobol(scramble=True)` with the draws of
/// the stream).
fn scramble(v: &[u32; BITS as usize], stream: &mut Stream) -> ([u32; BITS as usize], u32) {
    let shift = stream.bits(BITS) as u32;
    let rows: [u32; BITS as usize] = std::array::from_fn(|p| {
        let p = p as u32;
        let diagonal = 1u32 << (BITS - 1 - p);
        // the columns left of the diagonal of the row p, the p bits above its bit
        let left = if p == 0 {
            0
        } else {
            (stream.bits(p) as u32) << (BITS - p)
        };
        diagonal | left
    });
    (scrambled(v, &rows), shift)
}

/// The points of the sequence of the direction numbers and the shifts of its dimensions, in
/// the order of its Gray code from the point `skip`: the state of a point is the shift and the
/// direction numbers of the bits of the Gray code of its index, so a point is the one before
/// it with the direction number of the lowest zero bit of the index before it.
fn points(directions: &[[u32; BITS as usize]], shifts: &[u32], n: usize, skip: u64) -> Matrix {
    let d = directions.len();
    let gray = skip ^ (skip >> 1);
    let mut quasi: Vec<u32> = directions
        .iter()
        .zip(shifts)
        .map(|(v, &shift)| {
            (0..BITS as usize)
                .filter(|&bit| (gray >> bit) & 1 == 1)
                .fold(shift, |state, bit| state ^ v[bit])
        })
        .collect();
    let scale = 1.0 / f64::from(1u32 << BITS);
    let mut out = Matrix::zeros(n, d);
    for i in 0..n {
        if i > 0 {
            let bit = (skip + i as u64 - 1).trailing_ones() as usize;
            for (state, v) in quasi.iter_mut().zip(directions) {
                *state ^= v[bit];
            }
        }
        for (k, &state) in quasi.iter().enumerate() {
            out.data[i * d + k] = f64::from(state) * scale;
        }
    }
    out
}

/// The points `skip..skip + n` of the Sobol' sequence of `d` dimensions, a row per point.
///
/// The sequence of `scipy.stats.qmc.Sobol(d, bits=30)` in the order of its Gray code: without
/// scrambling (`seed` `None`) bit for bit the one of scipy (`fast_forward(skip)` and
/// `random(n)`), whose first point is the origin; scrambled with the linear matrix scrambling
/// and the digital shift of scipy drawn from the stream of `seed`. The balance of the sequence
/// needs `n` to be a power of two.
///
/// # Errors
///
/// If `d` is not in `1..=MAX_DIMENSIONS`, `n` is zero or the sequence has fewer than
/// `skip + n` points (`2^BITS`).
pub fn sobol_sequence(d: usize, n: usize, seed: Option<u64>, skip: u64) -> Result<Matrix> {
    check_parameters(d)?;
    if d > MAX_DIMENSIONS {
        return Err(SensitivityError::new(format!(
            "The Sobol' sequence has at most {MAX_DIMENSIONS} dimensions, not {d}."
        )));
    }
    if n == 0 {
        return Err(SensitivityError::new(
            "A Sobol' sequence needs at least one point.",
        ));
    }
    let end = skip
        .checked_add(n as u64)
        .filter(|&end| end <= 1u64 << BITS);
    if end.is_none() {
        return Err(SensitivityError::new(format!(
            "The Sobol' sequence has 2^{BITS} points, not {skip} + {n}."
        )));
    }
    let base = &directions()[..d];
    let (directions, shifts): (Vec<[u32; BITS as usize]>, Vec<u32>) = match seed {
        None => (base.to_vec(), vec![0; d]),
        Some(seed) => {
            let mut stream = Stream::new(seed);
            base.iter().map(|v| scramble(v, &mut stream)).unzip()
        }
    };
    Ok(points(&directions, &shifts, n, skip))
}

/// The matrix of Saltelli of a base sample of `2 d` columns, the one of
/// `SALib.sample.sobol.sample`: for every row of the base, the row `A` (the first `d` columns),
/// the `d` rows `AB_j` (`A` with the column `j` of `B`), with the second order the `d` rows
/// `BA_j` (`B` with the column `j` of `A`), and the row `B` (the last `d` columns).
pub(crate) fn saltelli_points(base: &Matrix, d: usize, second_order: bool) -> Matrix {
    let n = base.rows;
    let step = if second_order { 2 * d + 2 } else { d + 2 };
    let mut points = Matrix::zeros(n * step, d);
    for i in 0..n {
        let row = base.row(i);
        let (a, b) = row.split_at(d);
        let mut put = |offset: usize, values: &[f64], swapped: Option<(usize, f64)>| {
            let target = &mut points.data[(i * step + offset) * d..(i * step + offset + 1) * d];
            target.copy_from_slice(values);
            if let Some((j, value)) = swapped {
                target[j] = value;
            }
        };
        put(0, a, None);
        for (j, &value) in b.iter().enumerate() {
            put(1 + j, a, Some((j, value)));
        }
        if second_order {
            for (j, &value) in a.iter().enumerate() {
                put(1 + d + j, b, Some((j, value)));
            }
        }
        put(step - 1, b, None);
    }
    points
}

/// The matrix of Saltelli of `d` parameters on the Sobol' sequence of `2 d` dimensions, the
/// design of [`sobol_indices`]: `n (d + 2)` points, `n (2 d + 2)` with the second order, in
/// the order of `SALib.sample.sobol.sample` (`A`, `AB_1..AB_d`, `BA_1..BA_d`, `B` per row of
/// the base sequence).
///
/// The base sequence is [`sobol_sequence`] of `2 d` dimensions, scrambled with the stream of
/// `seed` or not scrambled for `None`, and its first point is kept, as SALib keeps it
/// (`skip_values=0`); `skip` skips points of it as `skip_values` does.
///
/// # Errors
///
/// See [`sobol_sequence`]: `2 d` dimensions are at most `MAX_DIMENSIONS`.
pub fn saltelli(
    d: usize,
    n: usize,
    second_order: bool,
    seed: Option<u64>,
    skip: u64,
) -> Result<Matrix> {
    check_parameters(d)?;
    if 2 * d > MAX_DIMENSIONS {
        return Err(SensitivityError::new(format!(
            "The matrix of Saltelli has at most {} parameters, not {d}.",
            MAX_DIMENSIONS / 2
        )));
    }
    let base = sobol_sequence(2 * d, n, seed, skip)?;
    Ok(saltelli_points(&base, d, second_order))
}

/// The Sobol indices of the parameters, every column of the values estimated at once.
#[derive(Debug, Clone, PartialEq)]
pub struct SobolIndices {
    /// The first order indices `S1`, a row per parameter and a column per element.
    pub s1: Matrix,
    /// Their intervals `S1_conf`.
    pub s1_conf: Matrix,
    /// The total indices `ST`.
    pub st: Matrix,
    /// Their intervals `ST_conf`.
    pub st_conf: Matrix,
    /// The second order indices `S2` with the second order, a row per pair of parameters
    /// `(j, k)` (the row `j d + k`, so row-major `(d, d, elements)`) and a column per element:
    /// the upper triangle, not a number elsewhere.
    pub s2: Option<Matrix>,
    /// Their intervals `S2_conf`, as `s2`.
    pub s2_conf: Option<Matrix>,
}

impl Columns for SobolIndices {
    fn join(parts: Vec<Self>) -> Self {
        let join = |field: fn(&Self) -> Option<&Matrix>| -> Option<Matrix> {
            let matrices: Option<Vec<&Matrix>> = parts.iter().map(field).collect();
            matrices.map(|matrices| Matrix::join(&matrices))
        };
        Self {
            s1: join(|p| Some(&p.s1)).expect("every part has S1"),
            s1_conf: join(|p| Some(&p.s1_conf)).expect("every part has S1_conf"),
            st: join(|p| Some(&p.st)).expect("every part has ST"),
            st_conf: join(|p| Some(&p.st_conf)).expect("every part has ST_conf"),
            s2: join(|p| p.s2.as_ref()),
            s2_conf: join(|p| p.s2_conf.as_ref()),
        }
    }
}

/// The Sobol indices of the values of the points of a matrix of Saltelli ([`saltelli`]).
///
/// The values of an element are centred and scaled by their standard deviation, as SALib does,
/// since the estimators are biased for an output which is not centred; an element of no
/// standard deviation keeps its zeros and gets zero indices. `S1` is `mean(B (AB_j - A)) / var`
/// and `ST` is `0.5 mean((A - AB_j)^2) / var` of Saltelli et al. 2010, `S2` is
/// `mean(BA_j AB_k - A B) / var - S1_j - S1_k` of Saltelli 2002, each over the variance of the
/// pooled sample of `A` and `B`, and zero where the pooled sample has a range of at most
/// [`EPSILON`]. An interval is `z(conf_level)` times the standard deviation of the estimator
/// over the resamples of the `n` samples, each with the variance of its own resample; given
/// resamples are row-major `(resamples, n)`, the samples each resample draws.
///
/// The elements are spread over at most `threads` threads, and an index does not depend on
/// them.
///
/// # Errors
///
/// If the number of rows is no multiple of the points of a sample of the design (`d + 2`, or
/// `2 d + 2` with the second order), or the bootstrap is not valid.
pub fn sobol_indices(
    y: MatrixRef<'_>,
    d: usize,
    second_order: bool,
    bootstrap: &Bootstrap<'_>,
    threads: usize,
) -> Result<SobolIndices> {
    by_columns(y, threads, |part| {
        sobol_part(part, d, second_order, bootstrap)
    })
}

/// The Sobol indices of the elements of `y` on one thread, see [`sobol_indices`].
fn sobol_part(
    y: MatrixRef<'_>,
    d: usize,
    second_order: bool,
    bootstrap: &Bootstrap<'_>,
) -> Result<SobolIndices> {
    check_parameters(d)?;
    let step = if second_order { 2 * d + 2 } else { d + 2 };
    let rows = y.rows();
    if rows == 0 || !rows.is_multiple_of(step) {
        return Err(SensitivityError::new(format!(
            "The design of {d} parameters has {step} points per sample, which {rows} rows are \
             no multiple of; an analysis of the second order needs a design of the second order."
        )));
    }
    let z = bootstrap.z()?;
    let n = rows / step;
    let elements = y.columns();
    let (indices, resamples) = bootstrap.indices(1, n, n, "(resamples, n) of the n samples")?;
    let weights = counts(&indices, n, n);

    // the values centred and scaled per element
    let mut mean = vec![0.0; elements];
    for i in 0..rows {
        crate::add_row(&mut mean, y.row(i));
    }
    for m in &mut mean {
        *m /= rows as f64;
    }
    let mut scale = vec![0.0; elements];
    for i in 0..rows {
        for ((s, v), m) in scale.iter_mut().zip(y.row(i)).zip(&mean) {
            let deviation = v - m;
            *s += deviation * deviation;
        }
    }
    for s in &mut scale {
        *s = (*s / rows as f64).sqrt();
    }
    let mut values = vec![0.0; rows * elements];
    for i in 0..rows {
        for (e, v) in y.row(i).iter().enumerate() {
            values[i * elements + e] = if scale[e] > 0.0 {
                (v - mean[e]) / scale[e]
            } else {
                0.0
            };
        }
    }
    let row = |i: usize| &values[i * elements..(i + 1) * elements];
    let a = |i: usize| row(i * step);
    let b = |i: usize| row(i * step + step - 1);

    // the variance of the pooled sample of A and B and whether it has a range
    let mut pooled_mean = vec![0.0; elements];
    for i in 0..n {
        crate::add_row(&mut pooled_mean, a(i));
        crate::add_row(&mut pooled_mean, b(i));
    }
    for m in &mut pooled_mean {
        *m /= (2 * n) as f64;
    }
    let mut variance = vec![0.0; elements];
    let mut low = vec![f64::INFINITY; elements];
    let mut high = vec![f64::NEG_INFINITY; elements];
    for i in 0..n {
        for sample in [a(i), b(i)] {
            for e in 0..elements {
                let v = sample[e];
                let deviation = v - pooled_mean[e];
                variance[e] += deviation * deviation;
                low[e] = low[e].min(v);
                high[e] = high[e].max(v);
            }
        }
    }
    for v in &mut variance {
        *v /= (2 * n) as f64;
    }
    let usable: Vec<bool> = (0..elements).map(|e| high[e] - low[e] > EPSILON).collect();

    // the variance of every resample of the pooled sample, from its sums: the values are
    // centred and scaled, so the mean is small against the spread and the difference of the
    // two moments does not cancel
    let mut pairs = vec![0.0; n * elements];
    let mut squares = vec![0.0; n * elements];
    for i in 0..n {
        for e in 0..elements {
            let (va, vb) = (a(i)[e], b(i)[e]);
            pairs[i * elements + e] = va + vb;
            squares[i * elements + e] = va * va + vb * vb;
        }
    }
    let sums = weighted_sums(&weights, n, elements, |i| {
        &pairs[i * elements..(i + 1) * elements]
    });
    let sum_squares = weighted_sums(&weights, n, elements, |i| {
        &squares[i * elements..(i + 1) * elements]
    });
    let pooled = (2 * n) as f64;
    let variance_boot: Vec<f64> = sums
        .iter()
        .zip(&sum_squares)
        .map(|(s, q)| q / pooled - (s / pooled) * (s / pooled))
        .collect();

    let mut found = SobolIndices {
        s1: Matrix::zeros(d, elements),
        s1_conf: Matrix::zeros(d, elements),
        st: Matrix::zeros(d, elements),
        st_conf: Matrix::zeros(d, elements),
        s2: None,
        s2_conf: None,
    };
    // the estimates of every resample of a term, over the variance of the resample
    let resampled = |term: &[f64]| -> Vec<f64> {
        let sums = weighted_sums(&weights, n, elements, |i| {
            &term[i * elements..(i + 1) * elements]
        });
        sums.iter()
            .enumerate()
            .map(|(k, s)| quotient(s / n as f64, variance_boot[k], usable[k % elements]))
            .collect()
    };
    let estimate = |term: &[f64]| -> Vec<f64> {
        let mut sum = vec![0.0; elements];
        for i in 0..n {
            crate::add_row(&mut sum, &term[i * elements..(i + 1) * elements]);
        }
        (0..elements)
            .map(|e| quotient(sum[e] / n as f64, variance[e], usable[e]))
            .collect()
    };
    let mut first_boot: Vec<Vec<f64>> = Vec::new();
    let mut first = vec![0.0; n * elements];
    let mut total = vec![0.0; n * elements];
    for j in 0..d {
        for i in 0..n {
            let ab = row(i * step + 1 + j);
            for e in 0..elements {
                let (va, vb, vab) = (a(i)[e], b(i)[e], ab[e]);
                first[i * elements + e] = vb * (vab - va);
                total[i * elements + e] = 0.5 * (va - vab) * (va - vab);
            }
        }
        let s1 = estimate(&first);
        let st = estimate(&total);
        let s1_boot = resampled(&first);
        let st_boot = resampled(&total);
        let range = j * elements..(j + 1) * elements;
        found.s1.data[range.clone()].copy_from_slice(&s1);
        found.st.data[range.clone()].copy_from_slice(&st);
        found.s1_conf.data[range.clone()]
            .copy_from_slice(&interval(&s1_boot, resamples, elements, z));
        found.st_conf.data[range].copy_from_slice(&interval(&st_boot, resamples, elements, z));
        if second_order {
            first_boot.push(s1_boot);
        }
    }
    if !second_order {
        return Ok(found);
    }
    let mut s2 = vec![f64::NAN; d * d * elements];
    let mut s2_conf = vec![f64::NAN; d * d * elements];
    let mut closed = vec![0.0; n * elements];
    for j in 0..d {
        for k in j + 1..d {
            for i in 0..n {
                let ba = row(i * step + 1 + d + j);
                let ab = row(i * step + 1 + k);
                for e in 0..elements {
                    closed[i * elements + e] = ba[e] * ab[e] - a(i)[e] * b(i)[e];
                }
            }
            let pair = estimate(&closed);
            let pair_boot = resampled(&closed);
            let offset = (j * d + k) * elements;
            for e in 0..elements {
                s2[offset + e] = pair[e] - found.s1.get(j, e) - found.s1.get(k, e);
            }
            let differences: Vec<f64> = pair_boot
                .iter()
                .enumerate()
                .map(|(m, p)| p - first_boot[j][m] - first_boot[k][m])
                .collect();
            s2_conf[offset..offset + elements].copy_from_slice(&interval(
                &differences,
                resamples,
                elements,
                z,
            ));
        }
    }
    found.s2 = Some(Matrix {
        rows: d * d,
        columns: elements,
        data: s2,
    });
    found.s2_conf = Some(Matrix {
        rows: d * d,
        columns: elements,
        data: s2_conf,
    });
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resamples;

    #[test]
    fn the_first_points_are_those_of_scipy_without_scrambling() {
        // scipy.stats.qmc.Sobol(3, scramble=False).random(8) times 8
        let points = sobol_sequence(3, 8, None, 0).unwrap();
        let expected = [
            [0, 0, 0],
            [4, 4, 4],
            [6, 2, 2],
            [2, 6, 6],
            [3, 3, 5],
            [7, 7, 1],
            [5, 1, 7],
            [1, 5, 3],
        ];
        for (i, row) in expected.iter().enumerate() {
            for (k, &value) in row.iter().enumerate() {
                assert_eq!(points.get(i, k), f64::from(value) / 8.0, "{i} {k}");
            }
        }
    }

    #[test]
    fn a_skip_continues_the_sequence() {
        for seed in [None, Some(5)] {
            let all = sobol_sequence(5, 64, seed, 0).unwrap();
            let rest = sobol_sequence(5, 37, seed, 27).unwrap();
            assert_eq!(&all.data[27 * 5..], &rest.data[..]);
        }
    }

    #[test]
    fn a_scrambled_sequence_is_balanced() {
        // every interval of 1/n of every dimension holds one point of n = 2^m, and every
        // square of 1/8 x 1/8 of the first two dimensions one of 64: scrambling keeps the net
        for seed in [0, 1, 99] {
            let points = sobol_sequence(40, 64, Some(seed), 0).unwrap();
            for k in 0..40 {
                let mut seen = [false; 64];
                for i in 0..64 {
                    let cell = (points.get(i, k) * 64.0) as usize;
                    assert!(!seen[cell], "{seed} {k} {cell}");
                    seen[cell] = true;
                }
            }
            let mut seen = [false; 64];
            for i in 0..64 {
                let cell =
                    (points.get(i, 0) * 8.0) as usize * 8 + (points.get(i, 1) * 8.0) as usize;
                assert!(!seen[cell]);
                seen[cell] = true;
            }
            assert_ne!(points.get(0, 0), 0.0, "the first point is shifted");
        }
    }

    #[test]
    fn the_scrambling_is_that_of_scipy() {
        // the shifts and the rows of the matrices of the two dimensions which
        // scipy.stats.qmc.Sobol(2, scramble=True, rng=1) draws, and the first four points it
        // gives, times 2^30
        let shifts = [0x1250_9876, 0x0a68_9de8];
        let rows: [[u32; 30]; 2] = [
            [
                0x20000000, 0x30000000, 0x8000000, 0x2c000000, 0x2a000000, 0x27000000, 0x1a800000,
                0x23400000, 0x2b600000, 0x19500000, 0x38480000, 0x73c0000, 0x338e0000, 0x3cc30000,
                0x108c8000, 0x2a164000, 0x26ca2000, 0x13af5000, 0x30920800, 0xb633c00, 0x1202e600,
                0x10b09b00, 0x9b1ac80, 0x2e19df40, 0x1b864b60, 0x1d502750, 0x3b8ade98, 0x34b075a4,
                0x8c521f2, 0x132aa13f,
            ],
            [
                0x20000000, 0x30000000, 0x18000000, 0x3c000000, 0x36000000, 0x9000000, 0x19800000,
                0x30c00000, 0x20a00000, 0x6500000, 0x2c780000, 0x3e1c0000, 0x292e0000, 0x33410000,
                0xe768000, 0x16944000, 0x7842000, 0x18277000, 0xcf49800, 0x250af400, 0x30b94a00,
                0x25ed4500, 0x185ca780, 0x1fd32ec0, 0x1dc5fce0, 0x2987d190, 0x33ef4508, 0x2e168fd4,
                0x282d55a6, 0xd5d3d5f,
            ],
        ];
        let directions: Vec<[u32; 30]> = (0..2)
            .map(|k| scrambled(&directions()[k], &rows[k]))
            .collect();
        let found = points(&directions, &shifts, 4, 0);
        let expected = [
            [0x1250_9876, 0x0a68_9de8],
            [0x253b_f03a, 0x3c07_9af6],
            [0x35a0_6b07, 0x14ad_cf00],
            [0x02cb_034b, 0x22c2_c81e],
        ];
        for (i, row) in expected.iter().enumerate() {
            for (k, &value) in row.iter().enumerate() {
                assert_eq!(found.get(i, k), f64::from(value) / f64::from(1u32 << 30));
            }
        }
    }

    #[test]
    fn the_matrix_of_saltelli_crosses_the_columns() {
        let d = 3;
        let base = sobol_sequence(2 * d, 4, Some(2), 0).unwrap();
        for second_order in [false, true] {
            let points = saltelli(d, 4, second_order, Some(2), 0).unwrap();
            let step = if second_order { 2 * d + 2 } else { d + 2 };
            assert_eq!(points.rows, 4 * step);
            for i in 0..4 {
                let (a, b) = base.row(i).split_at(d);
                assert_eq!(points.row(i * step), a);
                assert_eq!(points.row(i * step + step - 1), b);
                for j in 0..d {
                    let ab = points.row(i * step + 1 + j);
                    for k in 0..d {
                        assert_eq!(ab[k], if k == j { b[k] } else { a[k] });
                    }
                    if second_order {
                        let ba = points.row(i * step + 1 + d + j);
                        for k in 0..d {
                            assert_eq!(ba[k], if k == j { a[k] } else { b[k] });
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_arguments_of_a_sequence_are_checked() {
        assert!(sobol_sequence(0, 4, None, 0).is_err());
        assert!(sobol_sequence(MAX_DIMENSIONS + 1, 4, None, 0).is_err());
        assert!(sobol_sequence(2, 0, None, 0).is_err());
        assert!(sobol_sequence(2, 2, None, (1 << BITS) - 1).is_err());
        assert!(saltelli(MAX_DIMENSIONS / 2 + 1, 4, false, None, 0).is_err());
        assert!(sobol_sequence(MAX_DIMENSIONS, 2, Some(1), 0).is_ok());
    }

    #[test]
    fn the_indices_of_the_ishigami_function() {
        let d = 3;
        let points = saltelli(d, 4096, true, Some(1), 0).unwrap();
        let pi = std::f64::consts::PI;
        let y: Vec<f64> = (0..points.rows)
            .map(|i| {
                let x: Vec<f64> = points.row(i).iter().map(|u| u * 2.0 * pi - pi).collect();
                x[0].sin() + 7.0 * x[1].sin().powi(2) + 0.1 * x[2].powi(4) * x[0].sin()
            })
            .collect();
        let found = sobol_indices(
            MatrixRef::new(&y, points.rows, 1).unwrap(),
            d,
            true,
            &Bootstrap {
                conf_level: 0.95,
                resamples: Resamples::Seeded {
                    count: 100,
                    seed: 1,
                },
            },
            1,
        )
        .unwrap();
        for (j, (s1, st)) in [(0.314, 0.558), (0.442, 0.442), (0.0, 0.244)]
            .into_iter()
            .enumerate()
        {
            assert!((found.s1.get(j, 0) - s1).abs() < 0.03, "{j}");
            assert!((found.st.get(j, 0) - st).abs() < 0.03, "{j}");
            assert!(found.s1_conf.get(j, 0) > 0.0 && found.s1_conf.get(j, 0) < 0.1);
        }
        let s2 = found.s2.unwrap();
        assert!((s2.get(2, 0) - 0.244).abs() < 0.05);
        assert!(s2.get(d, 0).is_nan());
    }
}

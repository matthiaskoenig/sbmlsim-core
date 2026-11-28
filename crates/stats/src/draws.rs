//! The draws of the variables of an uncertainty analysis: the points of the unit cube and the
//! Gaussian copula which correlates them, as `unit_points` and `draw` of
//! `sbmlsim.uncertainty.draws` (and `Variability.draw` of `sbmlsim.fit.variability`) give them.
//!
//! The points are quasi-random (the scrambled Sobol' sequence of sbmlsim-core, the direction
//! numbers of Joe and Kuo), a Latin hypercube or random (xoshiro256++ of sbmlsim-core). These
//! are deliberately not the streams of scipy and numpy: a seed gives the same points on every
//! target, natively and in wasm, and the draws are held to their distributions, not to the bits
//! of scipy. The copula of given points and a given correlation is the python's.
//!
//! Joe S, Kuo FY (2008). Constructing Sobol sequences with better two-dimensional projections.
//! SIAM J Sci Comput 30:2635-2654. Nelsen RB (2006). An Introduction to Copulas, 2nd ed.
//! Springer.

use sbmlsim_sensitivity::random::Stream;
use sbmlsim_sensitivity::sobol_sequence;

use crate::special::{normal_cdf, normal_ppf};
use crate::{Result, StatsError, linalg};

/// The distance of the points of the copula from the ends of the unit interval, `EPS` of
/// `sbmlsim.simulation.sampling.distributions`: the normal score of 0 and of 1 is infinite.
pub const EPS: f64 = 1e-12;

/// The design of the points of the unit cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Design {
    /// The scrambled Sobol' sequence, which continues from `start` (the default of the python).
    Sobol,
    /// A Latin hypercube, without a continuation.
    Lhs,
    /// Random points, which continue from `start`.
    Random,
}

/// `n` points of the unit cube of `d` dimensions, `n` rows of `d` columns, row-major, of the
/// stream of `seed`, `unit_points` of `sbmlsim.uncertainty.draws`.
///
/// - [`Design::Sobol`]: the points `start..start + n` of the Sobol' sequence of sbmlsim-core
///   (`sbmlsim_sensitivity::sobol_sequence`), scrambled by the stream of `seed`; its balance
///   needs a power of two of points, which the propagation draws.
/// - [`Design::Lhs`]: a Latin hypercube, every column a random permutation of the `n` strata
///   with a uniform point in its stratum (`scipy.stats.qmc.LatinHypercube`): the `n * d`
///   positions in the strata, row-major, then the permutation of every column, of the stream of
///   `seed`.
/// - [`Design::Random`]: the uniform doubles of the stream of `seed` row-major from the double
///   `start * d`, so the points from `start` are the tail of a longer draw. The stream jumps
///   ahead by the `start * d` doubles in a time logarithmic in them
///   (`sbmlsim_sensitivity::random::Stream::advance`), so any start costs about the same.
///
/// No points or no dimensions are no values, as the empty array of the python.
///
/// # Errors
///
/// A Latin hypercube after `start`, with the message of the python, and the errors of the
/// Sobol' sequence: more than its 2048 dimensions or `start + n` past its `2^30` points.
pub fn unit_points(d: usize, n: usize, design: Design, seed: u64, start: u64) -> Result<Vec<f64>> {
    if design == Design::Lhs && start > 0 {
        return Err(StatsError(
            "A Latin hypercube has no continuation, its points of more draws are another \
             hypercube: give the number of draws, or use 'sobol' or 'random' for more draws."
                .to_owned(),
        ));
    }
    if d == 0 || n == 0 {
        return Ok(Vec::new());
    }
    match design {
        Design::Sobol => sobol_sequence(d, n, Some(seed), start)
            .map(|points| points.data)
            .map_err(|error| StatsError(error.0)),
        Design::Lhs => Ok(latin_hypercube(d, n, seed)),
        Design::Random => {
            let mut stream = Stream::new(seed);
            stream.advance(u128::from(start) * d as u128);
            Ok((0..n * d).map(|_| stream.uniform()).collect())
        }
    }
}

/// A Latin hypercube of `n` points of `d` dimensions of the stream of `seed`: the point of the
/// stratum `p` of a column is `(p + u) / n`, kept below the end of its stratum, `(p + 1) / n`,
/// which the rounding of the sum could reach.
fn latin_hypercube(d: usize, n: usize, seed: u64) -> Vec<f64> {
    let mut stream = Stream::new(seed);
    let mut points: Vec<f64> = (0..n * d).map(|_| stream.uniform()).collect();
    let size = n as f64;
    let mut strata: Vec<usize> = Vec::with_capacity(n);
    for j in 0..d {
        strata.clear();
        strata.extend(0..n);
        stream.shuffle(&mut strata);
        for (i, &p) in strata.iter().enumerate() {
            let point = &mut points[i * d + j];
            let end = (p + 1) as f64 / size;
            let x = (p as f64 + *point) / size;
            *point = if x < end { x } else { previous(end) };
        }
    }
    points
}

/// The largest double below the positive double `x`.
fn previous(x: f64) -> f64 {
    f64::from_bits(x.to_bits() - 1)
}

/// Whether the `d x d` correlation `r` is the identity within the tolerances of `np.allclose`
/// (`|r - I| <= 1e-8 + 1e-5 |I|`), which the python draws without the copula.
fn is_identity(r: &[f64], d: usize) -> bool {
    r.iter().enumerate().all(|(index, &value)| {
        let identity = if index / d == index % d { 1.0 } else { 0.0 };
        (value - identity).abs() <= 1e-8 + 1e-5 * identity
    })
}

/// The Gaussian copula of the correlation `r` of the normal scores on the `n x d` points `u` of
/// the unit cube, in place, row-major, the copula of `draw` of `sbmlsim.uncertainty.draws`: the
/// normal scores of the points clipped to [`EPS`] from the ends, `z = Φ⁻¹(u)`, times the
/// transposed factor of the correlation ([`linalg::factor`], Cholesky, else of the
/// eigendecomposition of a singular correlation), `z Fᵀ`, and back to the unit interval,
/// `Φ(z)`. A correlation which is the identity within the tolerances of `np.allclose` leaves
/// the points unchanged, as the python does.
///
/// A point which is not a number makes its row not a number, as the product of numpy.
///
/// # Errors
///
/// A slice of points which is not of `n * d` values or a correlation which is not of `d * d`,
/// and the errors of [`linalg::factor`] of a correlation which is not finite or not symmetric.
pub fn copula(u: &mut [f64], n: usize, d: usize, r: &[f64]) -> Result<()> {
    if u.len() != n * d {
        return Err(StatsError(format!(
            "{n} points of {d} dimensions need {} values, not {}.",
            n * d,
            u.len()
        )));
    }
    if r.len() != d * d {
        return Err(StatsError(format!(
            "The correlation of {d} dimensions needs {} values, not {}.",
            d * d,
            r.len()
        )));
    }
    if is_identity(r, d) {
        return Ok(());
    }
    let factor = linalg::factor(r, d)?;
    let mut scores = vec![0.0; d];
    for row in u.chunks_mut(d) {
        for (score, &u) in scores.iter_mut().zip(row.iter()) {
            *score = normal_ppf(u.clamp(EPS, 1.0 - EPS));
        }
        for (j, value) in row.iter_mut().enumerate() {
            let mut z = 0.0;
            for (score, f) in scores.iter().zip(&factor[j * d..(j + 1) * d]) {
                z += score * f;
            }
            *value = normal_cdf(z);
        }
    }
    Ok(())
}

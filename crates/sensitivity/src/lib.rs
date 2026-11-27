//! The global sensitivity analysis of sbmlsim-core: the designs of the three global methods on
//! the unit cube and the estimators of their indices, over every element of an output at once.
//!
//! A global analysis draws the points of a design on the unit cube, which a caller maps through
//! the distributions of its parameters and simulates, and estimates the indices of every
//! parameter from the values of the points. The designs and the estimators are the ones of
//! SALib, which sbmlsim used before:
//!
//! - Sobol indices: the matrix of Saltelli on a scrambled Sobol' sequence ([`saltelli`],
//!   [`sobol_sequence`]) and the first order and total indices of Saltelli et al. 2010 with the
//!   second order of Saltelli 2002 ([`sobol_indices`]);
//! - the extended FAST: the search curves of Saltelli et al. 1999 ([`fast_sample`]) and their
//!   spectra ([`fast_indices`]);
//! - Morris: the trajectories of Morris 1991 on a grid of levels ([`morris_sample`]) and their
//!   elementary effects with the `mu_star` of Campolongo et al. 2007 ([`morris_effects`]).
//!
//! An estimator takes the values as a matrix of a row per point of the design and a column per
//! element (the time points of a timecourse, the conditions of a scan), and estimates every
//! column in the same pass. The intervals are the bootstrap of SALib: `z(conf_level)` times
//! the standard deviation of the estimator over resamples ([`Bootstrap`]).
//!
//! Every random draw comes from one generator, xoshiro256++ seeded through SplitMix64
//! ([`random`]), and every transcendental function from the crate libm, so a seed gives the
//! same points and the same intervals on every target, natively and in wasm, bit for bit. The
//! draws are not the ones of numpy, so the points of a seed are not the ones SALib draws for
//! it; the deterministic parts are SALib's bit for bit (the Sobol' sequence without scrambling
//! is the one of `scipy.stats.qmc.Sobol`), see the tests.
//!
//! References:
//!
//! - Saltelli, A. (2002). Making best use of model evaluations to compute sensitivity indices.
//!   Computer Physics Communications 145(2):280-297.
//! - Saltelli, A., Annoni, P., Azzini, I., Campolongo, F., Ratto, M., Tarantola, S. (2010).
//!   Variance based sensitivity analysis of model output. Design and estimator for the total
//!   sensitivity index. Computer Physics Communications 181(2):259-270.
//! - Saltelli, A., Tarantola, S., Chan, K. P.-S. (1999). A quantitative model-independent
//!   method for global sensitivity analysis of model output. Technometrics 41(1):39-56.
//! - Morris, M. D. (1991). Factorial sampling plans for preliminary computational experiments.
//!   Technometrics 33(2):161-174.
//! - Campolongo, F., Cariboni, J., Saltelli, A. (2007). An effective screening design for
//!   sensitivity analysis of large models. Environmental Modelling and Software
//!   22(10):1509-1518.
//! - Joe, S., Kuo, F. Y. (2008). Constructing Sobol sequences with better two-dimensional
//!   projections. SIAM Journal on Scientific Computing 30(5):2635-2654.
//! - Herman, J., Usher, W. (2017). SALib: An open-source Python library for sensitivity
//!   analysis. Journal of Open Source Software 2(9):97.

mod bootstrap;
mod fast;
mod morris;
mod normal;
pub mod random;
mod sobol;

pub use bootstrap::{Bootstrap, Resamples};
pub use fast::{FastIndices, fast_indices, fast_phases, fast_sample, fast_sample_with_phases};
pub use morris::{MorrisEffects, morris_delta, morris_effects, morris_sample};
pub use normal::normal_quantile;
pub use sobol::{BITS, MAX_DIMENSIONS, SobolIndices, saltelli, sobol_indices, sobol_sequence};

/// The fewest elements a thread of an estimator estimates, below which a thread costs more than
/// the elements it would take.
pub const ELEMENTS_PER_THREAD: usize = 16;

/// The smallest variance an estimator divides by, the guard of SALib against a sample without
/// variance: a smaller one gives a zero index.
pub const EPSILON: f64 = f64::EPSILON;

/// An error of the arguments of a design or an estimator.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct SensitivityError(pub String);

impl SensitivityError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The result of a design or an estimator.
pub type Result<T> = std::result::Result<T, SensitivityError>;

/// A matrix of doubles, row-major: the points of a design (a row per point and a column per
/// parameter) or an index (a row per parameter and a column per element).
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// The number of rows.
    pub rows: usize,
    /// The number of columns.
    pub columns: usize,
    /// The values, `rows * columns` of them, a row after the other.
    pub data: Vec<f64>,
}

impl Matrix {
    /// A matrix of `rows` rows of `columns` zeros.
    #[must_use]
    pub fn zeros(rows: usize, columns: usize) -> Self {
        Self {
            rows,
            columns,
            data: vec![0.0; rows * columns],
        }
    }

    /// The row `i`.
    #[must_use]
    pub fn row(&self, i: usize) -> &[f64] {
        &self.data[i * self.columns..(i + 1) * self.columns]
    }

    /// The value of the row `i` and the column `j`.
    #[must_use]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.data[i * self.columns + j]
    }

    /// The view of the matrix.
    #[must_use]
    pub fn view(&self) -> MatrixRef<'_> {
        MatrixRef {
            rows: self.rows,
            columns: self.columns,
            data: &self.data,
        }
    }
}

/// A view of a matrix of doubles, row-major: the values of the points of a design, a row per
/// point and a column per element, or the points themselves.
#[derive(Debug, Clone, Copy)]
pub struct MatrixRef<'a> {
    rows: usize,
    columns: usize,
    data: &'a [f64],
}

impl<'a> MatrixRef<'a> {
    /// A view of `data` as `rows` rows of `columns` values.
    ///
    /// # Errors
    ///
    /// If `data` does not have `rows * columns` values.
    pub fn new(data: &'a [f64], rows: usize, columns: usize) -> Result<Self> {
        if rows.checked_mul(columns) != Some(data.len()) {
            return Err(SensitivityError::new(format!(
                "{} values are no matrix of {rows} rows of {columns} columns.",
                data.len()
            )));
        }
        Ok(Self {
            rows,
            columns,
            data,
        })
    }

    /// The number of rows.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The number of columns.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The row `i`.
    #[must_use]
    pub fn row(&self, i: usize) -> &'a [f64] {
        &self.data[i * self.columns..(i + 1) * self.columns]
    }
}

impl Matrix {
    /// The matrix of the columns of the parts side by side, every part of the same rows.
    fn join(parts: &[&Matrix]) -> Matrix {
        let rows = parts.first().map_or(0, |part| part.rows);
        let columns = parts.iter().map(|part| part.columns).sum();
        let mut joined = Matrix::zeros(rows, columns);
        for i in 0..rows {
            let mut at = i * columns;
            for part in parts {
                joined.data[at..at + part.columns].copy_from_slice(part.row(i));
                at += part.columns;
            }
        }
        joined
    }
}

/// An estimate of the elements whose parts of the elements are joined side by side.
trait Columns: Sized {
    /// The estimate of the elements of the parts, in their order.
    fn join(parts: Vec<Self>) -> Self;
}

/// Estimate the elements of `y`, split into one contiguous range of columns per thread of at
/// most `threads`: every element is estimated alone, so the threads do not change a bit.
fn by_columns<T: Columns + Send>(
    y: MatrixRef<'_>,
    threads: usize,
    estimate: impl Fn(MatrixRef<'_>) -> Result<T> + Sync,
) -> Result<T> {
    let elements = y.columns;
    let threads = threads.min(elements.div_ceil(ELEMENTS_PER_THREAD)).max(1);
    if threads == 1 {
        return estimate(y);
    }
    let per_thread = elements.div_ceil(threads);
    let ranges: Vec<std::ops::Range<usize>> = (0..elements)
        .step_by(per_thread)
        .map(|low| low..(low + per_thread).min(elements))
        .collect();
    // the columns of a range as a matrix of their own
    let part = |range: &std::ops::Range<usize>| -> Result<T> {
        let width = range.len();
        let mut block = Vec::with_capacity(y.rows * width);
        for i in 0..y.rows {
            block.extend_from_slice(&y.row(i)[range.clone()]);
        }
        estimate(MatrixRef::new(&block, y.rows, width)?)
    };
    let parts: Vec<Result<T>> = std::thread::scope(|scope| {
        // a range whose thread cannot be spawned (a target without threads) runs here
        let handles: Vec<_> = ranges
            .iter()
            .map(|range| {
                let part = &part;
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || part(range))
                    .map_err(|_| range)
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| match handle {
                Ok(handle) => handle.join().expect("a thread of an estimator panicked"),
                Err(range) => part(range),
            })
            .collect()
    });
    Ok(T::join(parts.into_iter().collect::<Result<Vec<T>>>()?))
}

/// Check the number of parameters of a design.
fn check_parameters(d: usize) -> Result<()> {
    if d == 0 {
        return Err(SensitivityError::new(
            "A design needs at least one parameter.",
        ));
    }
    Ok(())
}

/// Add a row to the sums of the columns.
fn add_row(sum: &mut [f64], row: &[f64]) {
    for (s, v) in sum.iter_mut().zip(row) {
        *s += v;
    }
}

/// The standard deviation with one degree of freedom less of every column of `rows` rows of
/// `columns` values, row-major, times `z`: the interval of an estimator over its resamples.
/// Not a number for a single resample, as numpy's.
fn interval(values: &[f64], rows: usize, columns: usize, z: f64) -> Vec<f64> {
    let mut mean = vec![0.0; columns];
    for r in 0..rows {
        add_row(&mut mean, &values[r * columns..(r + 1) * columns]);
    }
    for m in &mut mean {
        *m /= rows as f64;
    }
    let mut squares = vec![0.0; columns];
    for r in 0..rows {
        for ((s, v), m) in squares
            .iter_mut()
            .zip(&values[r * columns..(r + 1) * columns])
            .zip(&mean)
        {
            let deviation = v - m;
            *s += deviation * deviation;
        }
    }
    squares
        .into_iter()
        .map(|s| z * (s / (rows as f64 - 1.0)).sqrt())
        .collect()
}

/// Divide by the variance of a sample where it has one, else give zero.
#[inline]
fn quotient(numerator: f64, variance: f64, usable: bool) -> f64 {
    if usable && variance > EPSILON {
        numerator / variance
    } else {
        0.0
    }
}

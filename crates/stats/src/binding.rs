//! The functions as the bindings take their arguments: arrays of any shape with their shape,
//! the signed integers of python and of JavaScript and the names of the scales, designs, sides
//! and families, checked with the messages of the python.
//!
//! The python bindings (`crates/python/src/stats.rs`) and the JavaScript ones
//! (`crates/wasm/src/stats.rs`, `js/stats.mjs`) convert their arguments into slices, shapes and
//! integers and call these, so a refused argument gives one message in both. What is
//! particular to a language stays in its binding: the conversion of its objects, the text of
//! an object which is not a name (python's `repr`, the JSON of JavaScript) and the class of
//! the exception of an error kind.

use crate::correlation::{self, Marginal};
use crate::draws::{self, Design};
use crate::fisher::{self, FisherStatistics};
use crate::metrics;
use crate::python::{python_repr, python_str_repr, shape_text};
use crate::testing::{self, Decision, Side};
use crate::uncertainty;
use crate::variability;
use crate::{ErrorKind, Result, Scale, StatsError, posterior};

impl Scale {
    /// The scale of a name of `sbmlsim.fit.options.ParameterScaleType` (`LINEAR`, `LOG`,
    /// `LOG10`) or of PEtab (`lin`, `log`, `log10`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "LINEAR" | "lin" => Some(Self::Linear),
            "LOG" | "log" => Some(Self::Log),
            "LOG10" | "log10" => Some(Self::Log10),
            _ => None,
        }
    }
}

impl Design {
    /// The design of its name, `sobol`, `lhs` or `random`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sobol" => Some(Self::Sobol),
            "lhs" => Some(Self::Lhs),
            "random" => Some(Self::Random),
            _ => None,
        }
    }
}

impl Side {
    /// The side of its name, `greater`, `less` or `equivalent`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "greater" => Some(Self::Greater),
            "less" => Some(Self::Less),
            "equivalent" => Some(Self::Equivalent),
            _ => None,
        }
    }
}

impl Decision {
    /// The name of the decision, `accepted`, `rejected`, `equivalent` or `undecided`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Equivalent => "equivalent",
            Self::Undecided => "undecided",
        }
    }
}

/// The text python gives a string, `repr`, by which a message names a string the binding was
/// given in place of a name.
#[must_use]
pub fn string_repr(text: &str) -> String {
    python_str_repr(text)
}

/// The error of the python of a scale which is none of the names, the object named by
/// `written` (its `repr` in python).
#[must_use]
pub fn unknown_scale(written: &str) -> StatsError {
    StatsError::value(format!(
        "The parameter scale {written} is none of 'LINEAR', 'LOG' and 'LOG10' (of PEtab 'lin', \
         'log' and 'log10')."
    ))
}

/// The error of the python of a design which is none of the names, the object named by
/// `written`.
#[must_use]
pub fn unknown_design(written: &str) -> StatsError {
    StatsError::value(format!(
        "The design {written} is none of 'sobol', 'lhs' and 'random'."
    ))
}

/// The error of the python of a side which is none of the names, the object named by
/// `written`.
#[must_use]
pub fn unknown_side(written: &str) -> StatsError {
    StatsError::value(format!(
        "The side {written} is none of 'greater', 'less' and 'equivalent'."
    ))
}

/// The marginal of a covariance on the scale `linear` or `log` from `(family, mean, sd)` of
/// `_moments` of `sbmlsim.uncertainty.draws`: `normal` or `log-normal`.
///
/// # Errors
///
/// A family which is neither.
pub fn marginal(family: &str, mean: f64, sd: f64, scale: &str) -> Result<Marginal> {
    match (family, scale) {
        ("normal", _) => Ok(Marginal::Normal { sd }),
        ("log-normal", "log") => Ok(Marginal::Log { sd }),
        ("log-normal", _) => Ok(Marginal::LogNormal { mean, sigma: sd }),
        _ => Err(StatsError::value(format!(
            "The family {family:?} of a marginal is none of 'normal' and 'log-normal'."
        ))),
    }
}

/// The number of rows of a square matrix of the shape, the error naming the matrix `what` for
/// another shape.
///
/// # Errors
///
/// A shape which is not of a square matrix.
pub fn square(shape: &[usize], what: &str) -> Result<usize> {
    match *shape {
        [rows, columns] if rows == columns => Ok(rows),
        _ => Err(StatsError::value(format!(
            "The {what} must be a square matrix, not of the shape {}.",
            shape_text(shape)
        ))),
    }
}

/// The rows and the columns of a matrix of the shape, the error `The {what}, not of the shape
/// (...).` for another shape, `what` naming the matrix and what it must be.
fn matrix(shape: &[usize], what: &str) -> Result<(usize, usize)> {
    match *shape {
        [rows, columns] => Ok((rows, columns)),
        _ => Err(StatsError::value(format!(
            "The {what}, not of the shape {}.",
            shape_text(shape)
        ))),
    }
}

/// The percentage of the predicted ratios within the limits of Guest et al. of ratios of any
/// shape, `within_guest` of `sbmlsim.fit.evaluation`: the error of observed ratios which are
/// not positive and finite writes them as `tolist` of numpy does, nested by their shape.
///
/// # Errors
///
/// Ratios of other shapes and an observed ratio which is not positive and finite.
pub fn within_guest(
    observed: &[f64],
    observed_shape: &[usize],
    predicted: &[f64],
    predicted_shape: &[usize],
) -> Result<f64> {
    // the python compares the shapes and writes the sizes
    if observed_shape != predicted_shape {
        return Err(StatsError::value(format!(
            "The observed and the predicted ratios have different lengths, {} and {}.",
            shape_len(observed_shape),
            shape_len(predicted_shape)
        )));
    }
    metrics::within_guest(observed, predicted).map_err(|error| {
        if observed_shape.len() == 1 {
            error
        } else {
            StatsError::value(format!(
                "The observed ratios must be positive and finite: {}.",
                nested_list(observed, observed_shape)
            ))
        }
    })
}

/// The number of the values of an array of the shape, 1 of a scalar.
fn shape_len(shape: &[usize]) -> usize {
    shape.iter().product()
}

/// The values of the shape as python writes `tolist` of numpy: nested lists, a number of no
/// dimension.
fn nested_list(values: &[f64], shape: &[usize]) -> String {
    match shape {
        [] => values.first().map_or_else(String::new, |&x| python_repr(x)),
        [_] => {
            let items: Vec<String> = values.iter().map(|&x| python_repr(x)).collect();
            format!("[{}]", items.join(", "))
        }
        [rows, rest @ ..] => {
            let size: usize = rest.iter().product();
            let items: Vec<String> = (0..*rows)
                .map(|i| nested_list(&values[i * size..(i + 1) * size], rest))
                .collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// The Fisher information `JᵀJ` of a jacobian of the shape `(residuals, parameters)` and its
/// number of parameters.
///
/// # Errors
///
/// A jacobian which is not a matrix, and the errors of [`crate::linalg::gram`].
pub fn gram(jacobian: &[f64], shape: &[usize]) -> Result<(Vec<f64>, usize)> {
    let (n, k) = matrix(
        shape,
        "jacobian must be a matrix of the residuals by the parameters",
    )?;
    Ok((crate::linalg::gram(jacobian, n, k)?, k))
}

/// The statistics of a Fisher information of the shape `(k, k)` of `n` data points (a
/// negative number has no degrees of freedom, as zero has) and its number of parameters.
///
/// # Errors
///
/// An information which is not a square matrix, and the errors of [`fisher::statistics`].
pub fn fisher_statistics(
    information: &[f64],
    shape: &[usize],
    cost: f64,
    n: i64,
    likelihood: bool,
    rank_tolerance: f64,
) -> Result<(FisherStatistics, usize)> {
    let k = square(shape, "Fisher information")?;
    let n = n.max(0).unsigned_abs();
    let statistics = fisher::statistics(information, k, cost, n, likelihood, rank_tolerance)?;
    Ok((statistics, k))
}

/// The covariance of the Laplace approximation of a precision of the shape `(k, k)` and its
/// number of parameters; the error of a weak direction names the parameters by `names` (as
/// the binding writes them), by their indices without.
///
/// # Errors
///
/// A precision which is not a square matrix, and the errors of [`posterior::laplace_of`].
pub fn posterior_laplace(
    precision: &[f64],
    shape: &[usize],
    names: Option<Vec<String>>,
    opid: Option<&str>,
    rank_tolerance: f64,
) -> Result<(Vec<f64>, usize)> {
    let k = square(shape, "precision")?;
    let names = names.unwrap_or_else(|| (0..k).map(|i| i.to_string()).collect());
    let covariance = posterior::laplace_of(precision, k, rank_tolerance, opid, &names)?;
    Ok((covariance, k))
}

/// [`posterior::rank_correlation`] of a correlation of the shape `(k, k)`, and `k`.
///
/// # Errors
///
/// A correlation which is not a square matrix.
pub fn rank_correlation(r: &[f64], shape: &[usize]) -> Result<(Vec<f64>, usize)> {
    let k = square(shape, "correlation")?;
    Ok((posterior::rank_correlation(r, k)?, k))
}

/// [`posterior::normal_correlation`] of a correlation of the shape `(k, k)`, and `k`.
///
/// # Errors
///
/// A correlation which is not a square matrix.
pub fn normal_correlation(r: &[f64], shape: &[usize]) -> Result<(Vec<f64>, usize)> {
    let k = square(shape, "correlation")?;
    Ok((posterior::normal_correlation(r, k)?, k))
}

/// [`uncertainty::first_order_given_data`] with a signed number of bins: a negative one is
/// refused as none, as numpy refuses it.
///
/// # Errors
///
/// The errors of [`uncertainty::first_order_given_data`].
pub fn first_order_given_data(x: &[f64], y: &[f64], n_bins: Option<i64>) -> Result<f64> {
    // more bins than a `usize` counts (wasm) are more bins than draws, which give the same
    let n_bins =
        n_bins.map(|bins| usize::try_from(bins).unwrap_or(if bins < 0 { 0 } else { usize::MAX }));
    uncertainty::first_order_given_data(x, y, n_bins)
}

/// `n` points of the unit cube of `d` dimensions of signed arguments, [`draws::unit_points`]:
/// a negative one is refused with the error of the python, in its order (scipy's of the
/// dimensions of a sampler, of `fast_forward`, an `OverflowError`, and numpy's of the shape).
///
/// # Errors
///
/// A negative argument, and the errors of [`draws::unit_points`].
pub fn unit_points(
    d: i64,
    n: i64,
    design: Design,
    seed: u64,
    start: i64,
) -> Result<(Vec<f64>, usize, usize)> {
    let dimension_error = || StatsError::value("d must be a non-negative integer value");
    let shape_error = || StatsError::value("negative dimensions are not allowed");
    match design {
        Design::Sobol if d < 0 => return Err(dimension_error()),
        Design::Sobol if start < 0 => {
            return Err(StatsError::new(
                ErrorKind::Overflow,
                "can't convert negative value to npy_uint32",
            ));
        }
        // a Latin hypercube refuses any start first, a negative one as well
        Design::Lhs if start == 0 && d < 0 => return Err(dimension_error()),
        Design::Random if d < 0 => return Err(shape_error()),
        // numpy advances its generator back by a negative number of doubles, the stream of
        // xoshiro256++ has no way back
        Design::Random if start < 0 => {
            return Err(StatsError::value(format!(
                "The start of random points must not be negative, not {start}."
            )));
        }
        _ => {}
    }
    if n < 0 && (design != Design::Lhs || start == 0) {
        return Err(shape_error());
    }
    // a size past a `usize` (of 32 bits in wasm) is past what an array holds, as numpy of 32
    // bits refuses it
    let (d, n) = (count(d)?, count(n)?);
    Ok((
        draws::unit_points(d, n, design, seed, start.unsigned_abs())?,
        n,
        d,
    ))
}

/// The size of a signed integer whose sign is checked, numpy's `array is too big` past a
/// `usize`.
fn count(x: i64) -> Result<usize> {
    usize::try_from(x.unsigned_abs()).map_err(|_| crate::size::too_big())
}

/// The Gaussian copula of a correlation of the shape `(d, d)` on points of the shape `(n, d)`,
/// in place, [`draws::copula`], and `(n, d)`.
///
/// # Errors
///
/// Points which are not a matrix, a correlation which is not a square matrix of `d` rows, and
/// the errors of [`draws::copula`].
pub fn copula(
    u: &mut [f64],
    u_shape: &[usize],
    r: &[f64],
    r_shape: &[usize],
) -> Result<(usize, usize)> {
    let (n, d) = matrix(
        u_shape,
        "points must be a matrix of the draws by the variables",
    )?;
    let k = square(r_shape, "correlation")?;
    if k != d {
        return Err(StatsError::value(format!(
            "The correlation of {k} variables does not fit points of {d} variables."
        )));
    }
    draws::copula(u, n, d, r)?;
    Ok((n, d))
}

/// The number of rows of a matrix of numpy's eigendecompositions, with the errors numpy
/// raises of another shape: `not_square` is the error of a matrix of two dimensions which is
/// not square.
fn eigen_matrix(shape: &[usize], not_square: impl FnOnce(usize, usize) -> String) -> Result<usize> {
    match *shape {
        [rows, columns] if rows == columns => Ok(rows),
        [rows, columns] => Err(StatsError::value(not_square(rows, columns))),
        ref shape if shape.len() < 2 => Err(StatsError::value(format!(
            "{}-dimensional array given. Array must be at least two-dimensional",
            shape.len()
        ))),
        ref shape => Err(StatsError::value(format!(
            "The matrix must be a square matrix, not of the shape {}.",
            shape_text(shape)
        ))),
    }
}

/// [`correlation::nearest_correlation`] of a matrix of the shape `(k, k)` and a signed number
/// of iterations (a negative one is none, as `range` of python), and `k`.
///
/// # Errors
///
/// A matrix which is not square, with the errors of numpy.
pub fn nearest_correlation(
    m: &[f64],
    shape: &[usize],
    max_iterations: i64,
    tol: f64,
) -> Result<(Vec<f64>, usize)> {
    // `y - correction` and `r + r.T` of the python: numpy broadcasts the shape against its
    // transpose
    let k = eigen_matrix(shape, |rows, columns| {
        format!(
            "operands could not be broadcast together with shapes ({rows},{columns}) \
             ({columns},{rows}) "
        )
    })?;
    // more iterations than a `usize` counts (wasm) are no limit, as are fewer
    let iterations =
        usize::try_from(max_iterations).unwrap_or(if max_iterations < 0 { 0 } else { usize::MAX });
    Ok((correlation::nearest_correlation(m, k, iterations, tol)?, k))
}

/// [`correlation::is_psd`] of a matrix of the shape `(k, k)`.
///
/// # Errors
///
/// A matrix which is not square or empty, with the errors of numpy.
pub fn is_psd(m: &[f64], shape: &[usize], rtol: f64) -> Result<bool> {
    let k = eigen_matrix(shape, |_, _| {
        "Last 2 dimensions of the array must be square".to_owned()
    })?;
    if k == 0 {
        return Err(StatsError::value(
            "zero-size array to reduction operation minimum which has no identity",
        ));
    }
    Ok(correlation::is_psd(m, k, rtol))
}

/// [`testing::needed`] of a signed number of draws.
///
/// # Errors
///
/// A negative number of draws (the python loops without an end on it), and the errors of
/// [`testing::needed`].
pub fn needed(estimate: f64, n: i64, p0: f64, margin: f64, alpha: f64) -> Result<Option<u64>> {
    let n = u64::try_from(n).map_err(|_| {
        StatsError::value(format!(
            "The number of draws must not be negative, not {n}."
        ))
    })?;
    testing::needed(estimate, n, p0, margin, alpha)
}

/// [`variability::group_means`] of predictions of the shape `(subjects, points)` and signed
/// sizes: the statistics, their rows and their columns.
///
/// # Errors
///
/// Predictions which are not a matrix, a negative size or number of groups (numpy's, which a
/// group of one does not reach) and the errors of [`variability::group_means`].
pub fn group_means(
    values: &[f64],
    shape: &[usize],
    size: i64,
    groups: i64,
    geometric: bool,
    seed: u64,
) -> Result<(Vec<f64>, usize, usize)> {
    let (subjects, points) = matrix(
        shape,
        "predictions must be a matrix of the subjects by the data points",
    )?;
    // `rng.integers(0, n, size=(groups, size))` of numpy, which a group of one does not call
    if size != 1 && (size < 0 || groups < 0) {
        return Err(StatsError::value("negative dimensions are not allowed"));
    }
    let (size, groups) = (count(size)?, count(groups)?);
    let (statistics, rows) =
        variability::group_means(values, subjects, points, size, groups, geometric, seed)?;
    Ok((statistics, rows, points))
}

/// [`variability::bands`] of a distribution of the shape `(groups, points)`: the bands, a row
/// per data point, and their rows and columns.
///
/// # Errors
///
/// A distribution which is not a matrix and the errors of [`variability::bands`].
pub fn bands(
    distribution: &[f64],
    shape: &[usize],
    qs: &[f64],
) -> Result<(Vec<f64>, usize, usize)> {
    let (rows, cols) = matrix(
        shape,
        "distribution must be a matrix of the groups by the data points",
    )?;
    Ok((
        variability::bands(distribution, rows, cols, qs)?,
        cols,
        qs.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(Scale::from_name("log10"), Some(Scale::Log10));
        assert_eq!(Scale::from_name("Log"), None);
        assert_eq!(Design::from_name("lhs"), Some(Design::Lhs));
        assert_eq!(Side::from_name("less"), Some(Side::Less));
        assert_eq!(Decision::Undecided.name(), "undecided");
        let error = unknown_scale("'x'");
        assert!(error.message.starts_with("The parameter scale 'x' is none"));
    }

    #[test]
    fn nested_lists_are_written_as_tolist() {
        assert_eq!(
            nested_list(&[1.0, -2.0, 3.0, 4.0], &[2, 2]),
            "[[1.0, -2.0], [3.0, 4.0]]"
        );
        assert_eq!(nested_list(&[0.5], &[]), "0.5");
        assert_eq!(nested_list(&[], &[0, 3]), "[]");
    }
}

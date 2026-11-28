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
use crate::size;
use crate::special;
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

/// A distribution function or a special function of one argument, which the bindings apply
/// to an array of any shape, value by value, as the ufuncs of scipy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unary {
    /// `norm_cdf`, `scipy.stats.norm.cdf`.
    NormCdf,
    /// `norm_ppf`, `scipy.stats.norm.ppf`.
    NormPpf,
    /// `erf`, `scipy.special.erf`.
    Erf,
    /// `gamma`, `scipy.special.gamma`.
    Gamma,
}

impl Unary {
    /// The function of its name in `sbmlsim_core.stats`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "norm_cdf" => Some(Self::NormCdf),
            "norm_ppf" => Some(Self::NormPpf),
            "erf" => Some(Self::Erf),
            "gamma" => Some(Self::Gamma),
            _ => None,
        }
    }

    /// The value of the function at `x`.
    #[must_use]
    pub fn at(self, x: f64) -> f64 {
        match self {
            Self::NormCdf => special::normal_cdf(x),
            Self::NormPpf => special::normal_ppf(x),
            Self::Erf => special::erf(x),
            Self::Gamma => special::gamma(x),
        }
    }
}

/// A distribution function of a value and the degrees of freedom, which the bindings apply
/// to two arrays broadcast against each other, as scipy broadcasts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binary {
    /// `t_cdf(x, df)`, `scipy.stats.t.cdf`.
    TCdf,
    /// `t_ppf(q, df)`, `scipy.stats.t.ppf`.
    TPpf,
    /// `chi2_cdf(x, df)`, `scipy.stats.chi2.cdf`.
    Chi2Cdf,
    /// `chi2_ppf(q, df)`, `scipy.stats.chi2.ppf`.
    Chi2Ppf,
}

impl Binary {
    /// The function of its name in `sbmlsim_core.stats`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "t_cdf" => Some(Self::TCdf),
            "t_ppf" => Some(Self::TPpf),
            "chi2_cdf" => Some(Self::Chi2Cdf),
            "chi2_ppf" => Some(Self::Chi2Ppf),
            _ => None,
        }
    }

    /// The value of the function at the value `x` (a quantile or a probability) and `df`
    /// degrees of freedom.
    #[must_use]
    pub fn at(self, x: f64, df: f64) -> f64 {
        match self {
            Self::TCdf => special::t_cdf(x, df),
            Self::TPpf => special::t_ppf(x, df),
            Self::Chi2Cdf => special::chi2_cdf(x, df),
            Self::Chi2Ppf => special::chi2_ppf(x, df),
        }
    }

    /// Whether the function is a quantile, whose error of shapes which do not broadcast
    /// names the degrees of freedom first, as scipy's.
    fn is_quantile(self) -> bool {
        matches!(self, Self::TPpf | Self::Chi2Ppf)
    }
}

/// The function applied to every value of an array of any shape, the values of the same
/// shape.
///
/// # Errors
///
/// An array the allocator refuses, numpy's error.
pub fn unary(function: Unary, x: &[f64]) -> Result<Vec<f64>> {
    let mut values = size::buffer(x.len())?;
    values.extend(x.iter().map(|&value| function.at(value)));
    Ok(values)
}

/// The function of two arrays broadcast against each other by the rules of numpy, the value
/// `x` of the shape `x_shape` and the degrees of freedom `df` of the shape `df_shape`: the
/// values and their shape.
///
/// # Errors
///
/// Shapes which do not broadcast, with the error of scipy, and a result past what an array
/// holds, numpy's.
pub fn binary(
    function: Binary,
    x: &[f64],
    x_shape: &[usize],
    df: &[f64],
    df_shape: &[usize],
) -> Result<(Vec<f64>, Vec<usize>)> {
    let Some(shape) = broadcast_shape(x_shape, df_shape) else {
        let (first, second) = if function.is_quantile() {
            (df_shape, x_shape)
        } else {
            (x_shape, df_shape)
        };
        return Err(StatsError::value(format!(
            "operands could not be broadcast together with shapes {} {} ",
            numpy_shape_text(first),
            numpy_shape_text(second)
        )));
    };
    for (values, shape, what) in [(x, x_shape, "values"), (df, df_shape, "degrees of freedom")] {
        if shape.iter().try_fold(1_usize, |len, &n| len.checked_mul(n)) != Some(values.len()) {
            return Err(StatsError::value(format!(
                "The {what} of the shape {} are not {} values.",
                shape_text(shape),
                values.len()
            )));
        }
    }
    let len = size::array_len::<f64>(&shape)?;
    let mut values = size::buffer(len)?;
    let x_strides = broadcast_strides(x_shape, shape.len());
    let df_strides = broadcast_strides(df_shape, shape.len());
    let mut index = vec![0_usize; shape.len()];
    let (mut at_x, mut at_df) = (0_usize, 0_usize);
    for _ in 0..len {
        values.push(function.at(x[at_x], df[at_df]));
        // the next index of the result in the order numpy ravels it, and the offsets of the
        // values it reads
        for axis in (0..shape.len()).rev() {
            index[axis] += 1;
            at_x += x_strides[axis];
            at_df += df_strides[axis];
            if index[axis] < shape[axis] {
                break;
            }
            at_x -= x_strides[axis] * shape[axis];
            at_df -= df_strides[axis] * shape[axis];
            index[axis] = 0;
        }
    }
    Ok((values, shape))
}

/// The shape two shapes broadcast to by the rules of numpy, aligned at their last
/// dimensions, every pair of dimensions equal or one of them one; `None` of shapes which do
/// not broadcast.
fn broadcast_shape(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let n = a.len().max(b.len());
    let dimension = |shape: &[usize], axis: usize| {
        (axis + shape.len())
            .checked_sub(n)
            .map_or(1, |at| shape[at])
    };
    (0..n)
        .map(|axis| match (dimension(a, axis), dimension(b, axis)) {
            (x, y) if x == y || y == 1 => Some(x),
            (1, y) => Some(y),
            _ => None,
        })
        .collect()
}

/// The strides (in values) of an array of the shape in the `n` dimensions of the shape it is
/// broadcast to: zero of a dimension it does not have or has once.
fn broadcast_strides(shape: &[usize], n: usize) -> Vec<usize> {
    let mut strides = vec![0; n];
    let mut stride = 1;
    for (axis, &dimension) in shape.iter().enumerate().rev() {
        if dimension != 1 {
            strides[n - shape.len() + axis] = stride;
        }
        stride *= dimension;
    }
    strides
}

/// The text numpy writes a shape with in its errors of broadcasting: `(2,3)`, `(3,)`, `()`.
fn numpy_shape_text(shape: &[usize]) -> String {
    match shape {
        [n] => format!("({n},)"),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
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

/// The decomposition of a Fisher information of the shape `(k, k)`, [`fisher::decomposition`]:
/// its eigenvalues, largest first, its pseudo-inverse and its number of parameters.
///
/// # Errors
///
/// An information which is not a square matrix, and the errors of
/// [`fisher::decomposition`].
pub fn fisher_decomposition(
    information: &[f64],
    shape: &[usize],
) -> Result<(Vec<f64>, Vec<f64>, usize)> {
    let k = square(shape, "Fisher information")?;
    let (spectrum, inverse) = fisher::decomposition(information, k)?;
    Ok((spectrum, inverse, k))
}

/// The statistics of a Fisher information of `n` data points (a negative number has no
/// degrees of freedom, as zero has) from its eigenvalues of the shape `(k,)` and its
/// pseudo-inverse of the shape `(k, k)`, [`fisher::statistics_of_decomposition`], and its
/// number of parameters.
///
/// # Errors
///
/// A pseudo-inverse which is not a square matrix, eigenvalues which are not a vector of one
/// value per parameter, and the errors of [`fisher::statistics_of_decomposition`].
#[allow(clippy::too_many_arguments)]
pub fn fisher_statistics_of_decomposition(
    spectrum: &[f64],
    spectrum_shape: &[usize],
    inverse: &[f64],
    inverse_shape: &[usize],
    cost: f64,
    n: i64,
    likelihood: bool,
    rank_tolerance: f64,
) -> Result<(FisherStatistics, usize)> {
    let k = square(inverse_shape, "pseudo-inverse of a Fisher information")?;
    if spectrum_shape != [k] {
        return Err(StatsError::value(format!(
            "The eigenvalues of a Fisher information of {k} parameters must be a vector of \
             {k} values, not of the shape {}.",
            shape_text(spectrum_shape)
        )));
    }
    let n = n.max(0).unsigned_abs();
    let statistics = fisher::statistics_of_decomposition(
        spectrum,
        inverse,
        k,
        cost,
        n,
        likelihood,
        rank_tolerance,
    )?;
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
    fn two_arrays_broadcast_as_numpy_broadcasts_them() {
        assert_eq!(broadcast_shape(&[2, 1], &[3]), Some(vec![2, 3]));
        assert_eq!(broadcast_shape(&[], &[4]), Some(vec![4]));
        assert_eq!(broadcast_shape(&[0], &[1]), Some(vec![0]));
        assert_eq!(broadcast_shape(&[3, 1, 4], &[2, 1]), Some(vec![3, 2, 4]));
        assert_eq!(broadcast_shape(&[2], &[3]), None);
        // a column of probabilities and a row of degrees of freedom: every pair, row-major
        let (values, shape) = binary(
            Binary::Chi2Ppf,
            &[0.5, 0.95],
            &[2, 1],
            &[1.0, 2.0, 3.0],
            &[3],
        )
        .unwrap();
        assert_eq!(shape, [2, 3]);
        let expected: Vec<f64> = [0.5, 0.95]
            .iter()
            .flat_map(|&p| [1.0, 2.0, 3.0].map(|df| special::chi2_ppf(p, df)))
            .collect();
        assert_eq!(values, expected);
        // the error of scipy, the degrees of freedom first of a quantile
        let error = binary(Binary::TPpf, &[0.1, 0.2], &[2], &[1.0; 6], &[2, 3]).unwrap_err();
        assert_eq!(
            error.message,
            "operands could not be broadcast together with shapes (2,3) (2,) "
        );
        let error = binary(Binary::TCdf, &[0.1, 0.2], &[2], &[1.0; 3], &[3]).unwrap_err();
        assert_eq!(
            error.message,
            "operands could not be broadcast together with shapes (2,) (3,) "
        );
        // values which are not of their shape are an error, not a panic
        let error = binary(Binary::TCdf, &[0.0], &[2], &[1.0], &[]).unwrap_err();
        assert_eq!(
            error.message,
            "The values of the shape (2,) are not 1 values."
        );
        // a result past what an array holds is numpy's error, before any memory is asked for:
        // empty arrays whose other dimensions broadcast past it, which numpy refuses as well
        let half = 1usize << (usize::BITS / 2);
        let error = binary(Binary::TCdf, &[], &[half, 1, 0], &[], &[1, half, 0])
            .map(|_| ())
            .unwrap_err();
        assert!(error.message.starts_with("array is too big"), "{error}");
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

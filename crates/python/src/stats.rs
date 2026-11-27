//! The statistics of the analyses (`sbmlsim-stats`), the kernels of numpy and scipy in
//! sbmlsim, on arrays of numpy; `sbmlsim_core.stats` names them.
//!
//! An array of any shape and of any type numpy converts to doubles is read as its values in
//! the order numpy ravels it (C order), borrowed where it is contiguous and copied where it is
//! not, as `np.asarray(values, dtype=float).ravel()` of the python. An error of the arguments
//! is a `ValueError` with the message of the python function the binding replaces.

use std::borrow::Cow;

use numpy::ndarray::{Array2, ArrayD, IxDyn};
use numpy::{
    AllowTypeChange, PyArray1, PyArray2, PyArrayDyn, PyArrayLikeDyn, PyUntypedArrayMethods,
};
use pyo3::exceptions::{PyIndexError, PyOverflowError, PyValueError, PyZeroDivisionError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyString};
use sbmlsim_stats::correlation::{self, Marginal};
use sbmlsim_stats::draws::{self, Design};
use sbmlsim_stats::testing::{self, Decision, Side};
use sbmlsim_stats::uncertainty::{self, Exception};
use sbmlsim_stats::{Scale, StatsError, fisher, linalg, metrics, posterior, variability};

/// The array of values of numpy the functions take, of any shape.
type Values<'py> = PyArrayLikeDyn<'py, f64, AllowTypeChange>;

/// The array of booleans of numpy the functions take, of any shape, as `np.asarray(values,
/// dtype=bool)` converts it: a value which is not zero, a not a number included, is true.
type Booleans<'py> = PyArrayLikeDyn<'py, bool, AllowTypeChange>;

/// The lower and the upper bounds of intervals.
type Bounds<'py> = (Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<f64>>);

/// The error of the arguments of a function, the `ValueError` of the python.
fn value_error(error: StatsError) -> PyErr {
    PyValueError::new_err(error.0)
}

/// A number of python as a double, with the text python writes it with.
///
/// The crate writes the double into a message as python writes a float, where the python
/// function wrote the number it was given: `'1.0'` for the int `1`, which python writes
/// `'1'`. The error of a number is the crate's with the number written as python writes the
/// object it was given (`str`, as an f-string does). Extracting it refuses what `float`
/// refuses, `None` included, with a `TypeError`.
pub(crate) struct Number {
    value: f64,
    /// The text of the double, as the crate writes it, and of the object given, where the
    /// two differ.
    written: Option<(String, String)>,
}

impl Number {
    /// The fold factor of a criterion without one, twofold.
    const TWOFOLD: Self = Self {
        value: 2.0,
        written: None,
    };

    /// The level of an interval without one, 95 %.
    const LEVEL_95: Self = Self {
        value: 0.95,
        written: None,
    };

    /// The message with the number as python writes the object given, where the crate wrote
    /// the double between `before` and `after`.
    fn written_in(&self, message: String, before: &str, after: &str) -> String {
        match &self.written {
            Some((double, original)) => message.replacen(
                &format!("{before}{double}{after}"),
                &format!("{before}{original}{after}"),
                1,
            ),
            None => message,
        }
    }

    /// The error of the crate with the number as python writes the object given.
    fn error(&self, error: StatsError) -> PyErr {
        match &self.written {
            Some((double, original)) => PyValueError::new_err(error.0.replacen(
                &format!("'{double}'"),
                &format!("'{original}'"),
                1,
            )),
            None => value_error(error),
        }
    }
}

impl<'a, 'py> FromPyObject<'a, 'py> for Number {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let value: f64 = object.extract()?;
        let double = PyFloat::new(object.py(), value).str()?.to_string();
        let original = object.str()?.to_string();
        Ok(Self {
            value,
            written: (double != original).then_some((double, original)),
        })
    }
}

/// The values of an array in the order numpy ravels it, borrowed where it is contiguous.
fn values<'a>(array: &'a Values<'_>) -> Cow<'a, [f64]> {
    let view = array.as_array();
    match view.to_slice() {
        Some(slice) => Cow::Borrowed(slice),
        None => Cow::Owned(view.iter().copied().collect()),
    }
}

/// Sum of Squared Errors (SSE) of the residuals, 0 of none.
#[pyfunction]
pub(crate) fn stats_sse(residuals: Values<'_>) -> f64 {
    metrics::sse(&values(&residuals))
}

/// Mean Squared Error (MSE) of the residuals.
#[pyfunction]
pub(crate) fn stats_mse(residuals: Values<'_>) -> PyResult<f64> {
    metrics::mse(&values(&residuals)).map_err(value_error)
}

/// Root Mean Squared Error (RMSE) from the mean squared error.
#[pyfunction]
pub(crate) fn stats_rmse_from_mse(mse: Number) -> PyResult<f64> {
    metrics::rmse_from_mse(mse.value).map_err(|error| mse.error(error))
}

/// Akaike Information Criterion (AIC) of a least squares fit from its mean squared error,
/// `n ln(MSE) + 2 k`, `-inf` for an MSE of 0.
#[pyfunction]
pub(crate) fn stats_aic_from_mse(mse: Number, n: i64, k: i64) -> PyResult<f64> {
    metrics::aic_from_mse(mse.value, n, k).map_err(|error| mse.error(error))
}

/// Bayesian Information Criterion (BIC) of a least squares fit from its mean squared error,
/// `n ln(MSE) + k ln(n)`, `-inf` for an MSE of 0.
#[pyfunction]
pub(crate) fn stats_bic_from_mse(mse: Number, n: i64, k: i64) -> PyResult<f64> {
    metrics::bic_from_mse(mse.value, n, k).map_err(|error| mse.error(error))
}

/// Coefficient of determination (R²) of a prediction, `nan` for fewer than two data points
/// and for data without variance.
#[pyfunction]
pub(crate) fn stats_r_squared(y_observed: Values<'_>, y_predicted: Values<'_>) -> PyResult<f64> {
    metrics::r_squared(&values(&y_observed), &values(&y_predicted)).map_err(value_error)
}

/// The number of data points whose prediction is within the fold factor and the number of
/// the points with a ratio, a positive and finite value and prediction.
#[pyfunction]
#[pyo3(signature = (y_observed, y_predicted, fold = Number::TWOFOLD))]
#[pyo3(text_signature = "(y_observed, y_predicted, fold=2.0)")]
pub(crate) fn stats_fold_counts(
    y_observed: Values<'_>,
    y_predicted: Values<'_>,
    fold: Number,
) -> PyResult<(usize, usize)> {
    metrics::fold_counts(&values(&y_observed), &values(&y_predicted), fold.value)
        .map_err(|error| fold.error(error))
}

/// The fold criterion of every data point: 1 within the fold factor, 0 outside and `nan`
/// without a ratio.
#[pyfunction]
#[pyo3(signature = (y_observed, y_predicted, fold = Number::TWOFOLD))]
#[pyo3(text_signature = "(y_observed, y_predicted, fold=2.0)")]
pub(crate) fn stats_fold_accepted<'py>(
    py: Python<'py>,
    y_observed: Values<'py>,
    y_predicted: Values<'py>,
    fold: Number,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let accepted = metrics::fold_accepted(&values(&y_observed), &values(&y_predicted), fold.value)
        .map_err(|error| fold.error(error))?;
    Ok(PyArray1::from_vec(py, accepted))
}

/// The uncertainty criterion of every data point: 1 within `k` standard deviations, 0
/// outside and `nan` without a positive and finite standard deviation or with a value or
/// prediction which is not finite.
#[pyfunction]
#[pyo3(signature = (y_observed, y_predicted, y_sd, k = 2.0))]
pub(crate) fn stats_sd_accepted<'py>(
    py: Python<'py>,
    y_observed: Values<'py>,
    y_predicted: Values<'py>,
    y_sd: Values<'py>,
    k: f64,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let accepted = metrics::sd_accepted(
        &values(&y_observed),
        &values(&y_predicted),
        &values(&y_sd),
        k,
    )
    .map_err(value_error)?;
    Ok(PyArray1::from_vec(py, accepted))
}

/// The geometric mean fold error `10**mean(|log10(f/y)|)` of the points with a ratio, `nan`
/// without one.
#[pyfunction]
pub(crate) fn stats_gmfe(y_observed: Values<'_>, y_predicted: Values<'_>) -> PyResult<f64> {
    metrics::gmfe(&values(&y_observed), &values(&y_predicted)).map_err(value_error)
}

/// The average fold error `10**mean(log10(f/y))` of the points with a ratio, `nan` without
/// one.
#[pyfunction]
pub(crate) fn stats_afe(y_observed: Values<'_>, y_predicted: Values<'_>) -> PyResult<f64> {
    metrics::afe(&values(&y_observed), &values(&y_predicted)).map_err(value_error)
}

/// The root mean square of the normalized residuals, `nan` without one or with one which is
/// not finite.
#[pyfunction]
pub(crate) fn stats_nrmse(nres: Values<'_>) -> f64 {
    metrics::nrmse(&values(&nres))
}

/// The lower and the upper limits of the predicted ratios of the observed ratios of the
/// criterion of Guest et al., arrays of the shape of the observed ratios.
#[pyfunction]
pub(crate) fn stats_guest_limits<'py>(
    py: Python<'py>,
    observed: Values<'py>,
) -> (Bound<'py, PyArrayDyn<f64>>, Bound<'py, PyArrayDyn<f64>>) {
    let (low, high): (Vec<f64>, Vec<f64>) = values(&observed)
        .iter()
        .map(|&ratio| metrics::guest_limits(ratio))
        .unzip();
    let shape = IxDyn(observed.shape());
    let array = |limits: Vec<f64>| {
        let limits =
            ArrayD::from_shape_vec(shape.clone(), limits).expect("a limit per observed ratio");
        PyArrayDyn::from_owned_array(py, limits)
    };
    (array(low), array(high))
}

/// The percentage (0 to 100) of the finite predicted ratios within the limits of Guest et al.,
/// `nan` without one.
#[pyfunction]
pub(crate) fn stats_within_guest(observed: Values<'_>, predicted: Values<'_>) -> PyResult<f64> {
    // the python compares the shapes and writes the sizes
    if observed.shape() != predicted.shape() {
        return Err(PyValueError::new_err(format!(
            "The observed and the predicted ratios have different lengths, {} and {}.",
            observed.len(),
            predicted.len()
        )));
    }
    match metrics::within_guest(&values(&observed), &values(&predicted)) {
        Ok(percentage) => Ok(percentage),
        Err(error) if observed.ndim() == 1 => Err(value_error(error)),
        // of equal shapes the error is of the observed ratios, which the python writes as
        // `tolist` gives them, a nested list of a matrix and a number of a scalar
        Err(_) => Err(PyValueError::new_err(format!(
            "The observed ratios must be positive and finite: {}.",
            observed.call_method0("tolist")?
        ))),
    }
}

/// The agreement of a subset of the data points of the points with a ratio (Bland and Altman
/// 1999): the bias, the half width of the limits of agreement and the half widths of the 95%
/// confidence intervals of the bias and of a limit, in decades; `None` for fewer than
/// `MIN_POINTS_SUBSET_AGREEMENT` points with a ratio.
#[pyfunction]
pub(crate) fn stats_subset_agreement(
    y_observed: Values<'_>,
    y_predicted: Values<'_>,
) -> PyResult<Option<(f64, f64, f64, f64)>> {
    metrics::subset_agreement(&values(&y_observed), &values(&y_predicted)).map_err(value_error)
}

/// The shape of an array as python writes the tuple, `(3,)` of one dimension.
fn shape_text(shape: &[usize]) -> String {
    match shape {
        [n] => format!("({n},)"),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The values of a square matrix, row-major, and its number of rows; an error which names the
/// matrix `what` of another shape.
fn square<'a>(array: &'a Values<'_>, what: &str) -> PyResult<(Cow<'a, [f64]>, usize)> {
    match array.shape() {
        &[rows, columns] if rows == columns => Ok((values(array), rows)),
        shape => Err(PyValueError::new_err(format!(
            "The {what} must be a square matrix, not of the shape {}.",
            shape_text(shape)
        ))),
    }
}

/// The `k x k` matrix of row-major values as an array of numpy.
fn matrix(py: Python<'_>, values: Vec<f64>, k: usize) -> Bound<'_, PyArray2<f64>> {
    let matrix = Array2::from_shape_vec((k, k), values).expect("k * k values of a matrix");
    PyArray2::from_owned_array(py, matrix)
}

/// A scale of a parameter: a name of `sbmlsim.fit.options.ParameterScaleType` (`LINEAR`,
/// `LOG`, `LOG10`), of PEtab (`lin`, `log`, `log10`) or a member of an enum of these names.
pub(crate) struct ScaleArg(Scale);

impl<'a, 'py> FromPyObject<'a, 'py> for ScaleArg {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let name: Option<String> = if object.is_instance_of::<PyString>() {
            object.extract().ok()
        } else {
            object
                .getattr("name")
                .ok()
                .and_then(|name| name.extract().ok())
        };
        match name.as_deref() {
            Some("LINEAR" | "lin") => Ok(Self(Scale::Linear)),
            Some("LOG" | "log") => Ok(Self(Scale::Log)),
            Some("LOG10" | "log10") => Ok(Self(Scale::Log10)),
            _ => Err(PyValueError::new_err(format!(
                "The parameter scale {} is none of 'LINEAR', 'LOG' and 'LOG10' (of PEtab \
                 'lin', 'log' and 'log10').",
                object.repr()?
            ))),
        }
    }
}

/// The values, the standard errors and the scales of the parameters, checked to be as many.
fn parameters(
    values: &Values<'_>,
    standard_errors: &Values<'_>,
    scales: Vec<ScaleArg>,
) -> PyResult<(Vec<f64>, Vec<f64>, Vec<Scale>)> {
    let (values, se) = (
        self::values(values).into_owned(),
        self::values(standard_errors).into_owned(),
    );
    let scales: Vec<Scale> = scales.into_iter().map(|ScaleArg(scale)| scale).collect();
    if values.len() != se.len() || values.len() != scales.len() {
        return Err(PyValueError::new_err(format!(
            "{} values, {} standard errors and {} scales",
            values.len(),
            se.len(),
            scales.len()
        )));
    }
    Ok((values, se, scales))
}

/// The Fisher information `JᵀJ` of a jacobian of the residuals by the parameters.
#[pyfunction]
pub(crate) fn stats_gram<'py>(
    py: Python<'py>,
    jacobian: Values<'py>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let &[n, k] = jacobian.shape() else {
        return Err(PyValueError::new_err(format!(
            "The jacobian must be a matrix of the residuals by the parameters, not of the \
             shape {}.",
            shape_text(jacobian.shape())
        )));
    };
    Ok(matrix(py, linalg::gram(&values(&jacobian), n, k), k))
}

/// The statistics of a Fisher information as `FisherInformation`: the variance of the
/// residuals, the eigenvalues, the rank, the condition number, the covariance, the standard
/// errors and the correlation.
#[pyfunction]
#[pyo3(signature = (information, cost, n, likelihood = false, rank_tolerance = fisher::DEFAULT_RANK_TOLERANCE))]
pub(crate) fn stats_fisher_statistics<'py>(
    py: Python<'py>,
    information: Values<'py>,
    cost: f64,
    n: i64,
    likelihood: bool,
    rank_tolerance: f64,
) -> PyResult<Bound<'py, PyDict>> {
    let (information, k) = square(&information, "Fisher information")?;
    // the python takes any integer, a negative number of points has no degrees of freedom
    // as zero has (`n <= k`)
    let n = usize::try_from(n.max(0)).unwrap_or(usize::MAX);
    let s = fisher::statistics(&information, k, cost, n, likelihood, rank_tolerance)
        .map_err(value_error)?;
    let dict = PyDict::new(py);
    dict.set_item("sigma2", s.sigma2)?;
    dict.set_item("spectrum", PyArray1::from_vec(py, s.spectrum))?;
    dict.set_item("rank", s.rank)?;
    dict.set_item("condition", s.condition)?;
    dict.set_item("covariance", matrix(py, s.covariance, k))?;
    dict.set_item("standard_errors", PyArray1::from_vec(py, s.standard_errors))?;
    dict.set_item("correlation", matrix(py, s.correlation, k))?;
    Ok(dict)
}

/// The lower and the upper bounds of the confidence intervals of the parameters in the units
/// of the model, of the t distribution of `dof` degrees of freedom, of the normal one without.
#[pyfunction]
#[pyo3(signature = (values, standard_errors, scales, level = 0.95, dof = None))]
pub(crate) fn stats_fisher_intervals<'py>(
    py: Python<'py>,
    values: Values<'py>,
    standard_errors: Values<'py>,
    scales: Vec<ScaleArg>,
    level: f64,
    dof: Option<f64>,
) -> PyResult<Bounds<'py>> {
    let (values, se, scales) = parameters(&values, &standard_errors, scales)?;
    let (lower, upper): (Vec<f64>, Vec<f64>) = fisher::intervals(&values, &se, &scales, level, dof)
        .into_iter()
        .unzip();
    Ok((PyArray1::from_vec(py, lower), PyArray1::from_vec(py, upper)))
}

/// The standard error of every parameter relative to its value, as `FisherInformation`.
#[pyfunction]
pub(crate) fn stats_fisher_relative_errors<'py>(
    py: Python<'py>,
    values: Values<'py>,
    standard_errors: Values<'py>,
    scales: Vec<ScaleArg>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let (values, se, scales) = parameters(&values, &standard_errors, scales)?;
    Ok(PyArray1::from_vec(
        py,
        fisher::relative_errors(&values, &se, &scales),
    ))
}

/// The covariance of the Laplace approximation of the posterior of a precision; the error of
/// `posterior_laplace` names the parameters of a weak direction by their pids, by their
/// indices without.
#[pyfunction]
#[pyo3(signature = (precision, pids = None, opid = None, rank_tolerance = fisher::DEFAULT_RANK_TOLERANCE))]
pub(crate) fn stats_posterior_laplace<'py>(
    py: Python<'py>,
    precision: Values<'py>,
    pids: Option<Vec<Bound<'py, PyAny>>>,
    opid: Option<String>,
    rank_tolerance: f64,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (precision, k) = square(&precision, "precision")?;
    // a pid as python writes it in a list, its repr
    let names = match pids {
        Some(pids) => pids
            .iter()
            .map(|pid| Ok(pid.repr()?.to_string()))
            .collect::<PyResult<Vec<String>>>()?,
        None => (0..k).map(|i| i.to_string()).collect(),
    };
    let covariance = posterior::laplace_of(&precision, k, rank_tolerance, opid.as_deref(), &names)
        .map_err(value_error)?;
    Ok(matrix(py, covariance, k))
}

/// The Spearman rank correlation of the normal of a correlation, `6 / π asin(r / 2)`.
#[pyfunction]
pub(crate) fn stats_rank_correlation<'py>(
    py: Python<'py>,
    r: Values<'py>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (r, k) = square(&r, "correlation")?;
    Ok(matrix(py, posterior::rank_correlation(&r, k), k))
}

/// The correlation of the normal scores of a posterior: the symmetric part, the diagonal 1.
#[pyfunction]
pub(crate) fn stats_normal_correlation<'py>(
    py: Python<'py>,
    r: Values<'py>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (r, k) = square(&r, "correlation")?;
    Ok(matrix(py, posterior::normal_correlation(&r, k), k))
}

/// The standard deviation of the natural logarithm of a standard deviation on a scale.
#[pyfunction]
pub(crate) fn stats_log_sd(scale: ScaleArg, sd: f64) -> f64 {
    posterior::log_sd(scale.0, sd)
}

/// The coefficient of variation of the log-normal of a standard deviation of the logarithm.
#[pyfunction]
pub(crate) fn stats_lognormal_cv(sigma: f64) -> f64 {
    posterior::lognormal_cv(sigma)
}

/// The error of a function of the uncertainty as the exception the python raised: a
/// `ValueError`, an `OverflowError`, a `ZeroDivisionError` or an `IndexError`.
fn uncertainty_error(error: StatsError) -> PyErr {
    match uncertainty::exception(&error) {
        Exception::Value => PyValueError::new_err(error.0),
        Exception::Overflow => PyOverflowError::new_err(error.0),
        Exception::ZeroDivision => PyZeroDivisionError::new_err(error.0),
        Exception::Index => PyIndexError::new_err(error.0),
    }
}

/// The mean, its standard error and the number of draws which are not `NaN`.
#[pyfunction]
pub(crate) fn stats_mean_se(x: Values<'_>) -> (f64, f64, usize) {
    uncertainty::mean_se(&values(&x))
}

/// The standard deviation, its standard error by the delta method and the number of draws
/// which are not `NaN`.
#[pyfunction]
pub(crate) fn stats_sd_se(x: Values<'_>) -> PyResult<(f64, f64, usize)> {
    uncertainty::sd_se(&values(&x)).map_err(uncertainty_error)
}

/// The quantile of the draws which are not `NaN`, numpy's linear one.
#[pyfunction]
pub(crate) fn stats_quantile(x: Values<'_>, q: f64) -> PyResult<f64> {
    uncertainty::quantile(&values(&x), q).map_err(uncertainty_error)
}

/// The quantiles of the draws which are not `NaN`, numpy's linear ones.
#[pyfunction]
pub(crate) fn stats_quantiles<'py>(
    py: Python<'py>,
    x: Values<'py>,
    qs: Values<'py>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let found = uncertainty::quantiles(&values(&x), &values(&qs)).map_err(uncertainty_error)?;
    Ok(PyArray1::from_vec(py, found))
}

/// A quantile, the ends of its distribution-free interval and the number of draws which are
/// not `NaN`.
#[pyfunction]
#[pyo3(signature = (x, q, level = 0.95))]
pub(crate) fn stats_quantile_ci(
    x: Values<'_>,
    q: f64,
    level: f64,
) -> PyResult<(f64, f64, f64, usize)> {
    uncertainty::quantile_ci(&values(&x), q, level).map_err(uncertainty_error)
}

/// The first-order index of an input from given draws, by binning.
#[pyfunction]
#[pyo3(signature = (x, y, n_bins = None))]
pub(crate) fn stats_first_order_given_data(
    x: Values<'_>,
    y: Values<'_>,
    n_bins: Option<i64>,
) -> PyResult<f64> {
    // a negative number of bins is refused as none, as numpy refuses it
    let n_bins = n_bins.map(|bins| usize::try_from(bins).unwrap_or(0));
    uncertainty::first_order_given_data(&values(&x), &values(&y), n_bins).map_err(uncertainty_error)
}

/// Whether the statistics of the draws of an output are precise enough, and the largest half
/// width relative to the standard deviation.
#[pyfunction]
pub(crate) fn stats_precise(values: Values<'_>, rel_tol: f64) -> PyResult<(bool, f64)> {
    uncertainty::precise(&self::values(&values), rel_tol).map_err(uncertainty_error)
}

/// The parameters of the distribution of a reported interval: the median and the coefficient
/// of variation of a log-normal, the mean and the standard deviation of a normal.
#[pyfunction]
#[pyo3(signature = (low, high, level = Number::LEVEL_95, *, lognormal))]
#[pyo3(text_signature = "(low, high, level=0.95, *, lognormal)")]
pub(crate) fn stats_interval_parameters(
    low: Number,
    high: Number,
    level: Number,
    lognormal: bool,
) -> PyResult<(f64, f64)> {
    uncertainty::interval_parameters(low.value, high.value, level.value, lognormal).map_err(
        |error| {
            let message = low.written_in(error.0, "[", ",");
            let message = high.written_in(message, ", ", "]");
            let message = level.written_in(message, "The level ", " of");
            uncertainty_error(StatsError(message))
        },
    )
}

/// The `rows x columns` matrix of row-major values as an array of numpy.
fn rectangle(
    py: Python<'_>,
    values: Vec<f64>,
    rows: usize,
    columns: usize,
) -> Bound<'_, PyArray2<f64>> {
    let matrix =
        Array2::from_shape_vec((rows, columns), values).expect("rows * columns values of a matrix");
    PyArray2::from_owned_array(py, matrix)
}

/// The seed of a stream: a non-negative integer, a negative one refused with the ValueError of
/// numpy.
pub(crate) struct Seed(u64);

impl<'a, 'py> FromPyObject<'a, 'py> for Seed {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        match object.extract::<u64>() {
            Ok(seed) => Ok(Self(seed)),
            Err(error) => match object.extract::<i64>() {
                Ok(seed) if seed < 0 => Err(PyValueError::new_err("expected non-negative integer")),
                _ => Err(error),
            },
        }
    }
}

/// A design of the points of the unit cube: `sobol`, `lhs` or `random`; any other object is
/// the ValueError of the python, raised when the points are drawn, so an explicit `None` is
/// refused as the python refuses it.
pub(crate) struct DesignArg(std::result::Result<Design, String>);

impl DesignArg {
    /// The design without one, the scrambled Sobol' sequence.
    const SOBOL: Self = Self(Ok(Design::Sobol));
}

impl<'a, 'py> FromPyObject<'a, 'py> for DesignArg {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        Ok(Self(match object.extract::<String>().ok().as_deref() {
            Some("sobol") => Ok(Design::Sobol),
            Some("lhs") => Ok(Design::Lhs),
            Some("random") => Ok(Design::Random),
            _ => Err(format!(
                "The design {} is none of 'sobol', 'lhs' and 'random'.",
                object.repr()?
            )),
        }))
    }
}

/// `n` points of the unit cube of `d` dimensions, `n` rows of `d` columns, of the design
/// `sobol` (the scrambled Sobol' sequence, from the point `start`), `lhs` (a Latin hypercube,
/// without a continuation) or `random` (from the point `start`).
#[pyfunction]
#[pyo3(signature = (d, n, *, design = DesignArg::SOBOL, seed, start = 0))]
#[pyo3(text_signature = "(d, n, *, design='sobol', seed, start=0)")]
pub(crate) fn stats_unit_points(
    py: Python<'_>,
    d: i64,
    n: i64,
    design: DesignArg,
    seed: Seed,
    start: i64,
) -> PyResult<Bound<'_, PyArray2<f64>>> {
    let design = design.0.map_err(PyValueError::new_err)?;
    // the errors of a negative argument in the order of the python: scipy's of the dimensions
    // of a sampler, of `fast_forward` (an OverflowError) and numpy's of the shape
    let dimension_error = || PyValueError::new_err("d must be a non-negative integer value");
    let shape_error = || PyValueError::new_err("negative dimensions are not allowed");
    match design {
        Design::Sobol if d < 0 => return Err(dimension_error()),
        Design::Sobol if start < 0 => {
            return Err(PyOverflowError::new_err(
                "can't convert negative value to npy_uint32",
            ));
        }
        // a Latin hypercube refuses any start first, a negative one as well
        Design::Lhs if start == 0 && d < 0 => return Err(dimension_error()),
        Design::Random if d < 0 => return Err(shape_error()),
        // numpy advances its generator back by a negative number of doubles, the stream of
        // xoshiro256++ has no way back
        Design::Random if start < 0 => {
            return Err(PyValueError::new_err(format!(
                "The start of random points must not be negative, not {start}."
            )));
        }
        _ => {}
    }
    if n < 0 && (design != Design::Lhs || start == 0) {
        return Err(shape_error());
    }
    let (d, n) = (d.unsigned_abs() as usize, n.unsigned_abs() as usize);
    let points =
        draws::unit_points(d, n, design, seed.0, start.unsigned_abs()).map_err(value_error)?;
    Ok(rectangle(py, points, n, d))
}

/// The Gaussian copula of the correlation `r` of the normal scores on the points `u` of the
/// unit cube, `n` rows of `d` columns: `Φ(Φ⁻¹(clip(u, EPS, 1 - EPS)) Fᵀ)` with the factor `F`
/// of `r`, the points themselves for a correlation which is the identity (`np.allclose`).
#[pyfunction]
pub(crate) fn stats_copula<'py>(
    py: Python<'py>,
    u: Values<'py>,
    r: Values<'py>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let &[n, d] = u.shape() else {
        return Err(PyValueError::new_err(format!(
            "The points must be a matrix of the draws by the variables, not of the shape {}.",
            shape_text(u.shape())
        )));
    };
    let (r, k) = square(&r, "correlation")?;
    if k != d {
        return Err(PyValueError::new_err(format!(
            "The correlation of {k} variables does not fit points of {d} variables."
        )));
    }
    let mut points = values(&u).into_owned();
    draws::copula(&mut points, n, d, &r).map_err(value_error)?;
    Ok(rectangle(py, points, n, d))
}

/// A marginal of a covariance as `_moments` of `sbmlsim.uncertainty.draws` gives it: its
/// family (`normal` or `log-normal`), its mean and its standard deviation (of the logarithm of
/// a log-normal).
pub(crate) struct Moments(String, f64, f64);

impl<'a, 'py> FromPyObject<'a, 'py> for Moments {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let (family, mean, sd): (String, f64, f64) = object.extract()?;
        Ok(Self(family, mean, sd))
    }
}

impl Moments {
    /// The marginal of the crate on the scale of the covariance.
    fn marginal(&self, scale: &str) -> PyResult<Marginal> {
        let Self(family, mean, sd) = self;
        match (family.as_str(), scale) {
            ("normal", _) => Ok(Marginal::Normal { sd: *sd }),
            ("log-normal", "log") => Ok(Marginal::Log { sd: *sd }),
            ("log-normal", _) => Ok(Marginal::LogNormal {
                mean: *mean,
                sigma: *sd,
            }),
            _ => Err(PyValueError::new_err(format!(
                "The family {family:?} of a marginal is none of 'normal' and 'log-normal'."
            ))),
        }
    }
}

/// The coefficient of the normal scores of the covariance `m` of the marginals `a` and `b`
/// (`(family, mean, sd)` of `_moments`) on the scale `linear` or `log`.
#[pyfunction]
#[pyo3(signature = (m, a, b, scale = "linear"))]
pub(crate) fn stats_normal_score_rho(m: f64, a: Moments, b: Moments, scale: &str) -> PyResult<f64> {
    correlation::normal_score_rho(m, a.marginal(scale)?, b.marginal(scale)?)
        .map_err(uncertainty_error)
}

/// The values of a matrix of the functions of numpy's eigendecompositions, with the
/// errors numpy raises of another shape: `stacked` is the error of a matrix of two
/// dimensions which is not square.
fn eigen_matrix<'a>(
    array: &'a Values<'_>,
    not_square: impl FnOnce(usize, usize) -> String,
) -> PyResult<(Cow<'a, [f64]>, usize)> {
    match *array.shape() {
        [rows, columns] if rows == columns => Ok((values(array), rows)),
        [rows, columns] => Err(PyValueError::new_err(not_square(rows, columns))),
        ref shape if shape.len() < 2 => Err(PyValueError::new_err(format!(
            "{}-dimensional array given. Array must be at least two-dimensional",
            shape.len()
        ))),
        ref shape => Err(PyValueError::new_err(format!(
            "The matrix must be a square matrix, not of the shape {}.",
            shape_text(shape)
        ))),
    }
}

/// The nearest correlation matrix in the Frobenius norm (Higham 2002), as
/// `sbmlsim.uncertainty.covariance.nearest_correlation`.
#[pyfunction]
#[pyo3(signature = (matrix, max_iterations = correlation::NEAREST_MAX_ITERATIONS as i64, tol = correlation::NEAREST_TOL))]
#[pyo3(text_signature = "(matrix, max_iterations=100, tol=1e-10)")]
pub(crate) fn stats_nearest_correlation<'py>(
    py: Python<'py>,
    matrix: Values<'py>,
    max_iterations: i64,
    tol: f64,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    // `y - correction` and `r + r.T` of the python: numpy broadcasts the shape against its
    // transpose
    let (m, k) = eigen_matrix(&matrix, |rows, columns| {
        format!(
            "operands could not be broadcast together with shapes ({rows},{columns}) \
             ({columns},{rows}) "
        )
    })?;
    // `range` of a negative number is no iteration
    let iterations = usize::try_from(max_iterations).unwrap_or(0);
    let nearest = correlation::nearest_correlation(&m, k, iterations, tol).map_err(value_error)?;
    Ok(self::matrix(py, nearest, k))
}

/// Whether a symmetric matrix is positive semi-definite, `_is_psd`: its smallest eigenvalue at
/// least `-rtol` times its largest one.
#[pyfunction]
#[pyo3(signature = (matrix, rtol = correlation::PSD_RTOL))]
pub(crate) fn stats_is_psd(matrix: Values<'_>, rtol: f64) -> PyResult<bool> {
    let (m, k) = eigen_matrix(&matrix, |_, _| {
        "Last 2 dimensions of the array must be square".to_owned()
    })?;
    if k == 0 {
        return Err(PyValueError::new_err(
            "zero-size array to reduction operation minimum which has no identity",
        ));
    }
    Ok(correlation::is_psd(&m, k, rtol))
}

/// Checks the matrix of a `Covariance` of the targets (`kind` `covariance` or `correlation`,
/// `units` the units of the targets of a covariance), as `Covariance._validate`.
#[pyfunction]
#[pyo3(signature = (targets, matrix, *, kind = "covariance", units = None))]
pub(crate) fn stats_validate_covariance(
    targets: Vec<Bound<'_, PyAny>>,
    matrix: Values<'_>,
    kind: &str,
    units: Option<Bound<'_, PyAny>>,
) -> PyResult<()> {
    let targets = targets
        .iter()
        .map(|target| Ok(target.str()?.to_string()))
        .collect::<PyResult<Vec<String>>>()?;
    let targets: Vec<&str> = targets.iter().map(String::as_str).collect();
    // `tuple(units)` of the python, of any iterable, whose number the validation compares
    // with the targets only for a covariance
    let units = units
        .map(|units| {
            units
                .try_iter()?
                .try_fold(0usize, |count, item| item.map(|_| count + 1))
        })
        .transpose()?;
    correlation::validate(
        &values(&matrix),
        matrix.shape(),
        &targets,
        kind == "correlation",
        units,
    )
    .map_err(value_error)
}

/// The booleans of an array in the order numpy ravels it, `np.asarray(x, dtype=bool).ravel()`.
fn booleans<'a>(array: &'a Booleans<'_>) -> Cow<'a, [bool]> {
    let view = array.as_array();
    match view.to_slice() {
        Some(slice) => Cow::Borrowed(slice),
        None => Cow::Owned(view.iter().copied().collect()),
    }
}

/// The alternative of a p-value: `greater`, `less` or `equivalent`.
pub(crate) struct SideArg(Side);

impl<'a, 'py> FromPyObject<'a, 'py> for SideArg {
    type Error = PyErr;

    fn extract(object: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        match object.extract::<String>().ok().as_deref() {
            Some("greater") => Ok(Self(Side::Greater)),
            Some("less") => Ok(Self(Side::Less)),
            Some("equivalent") => Ok(Self(Side::Equivalent)),
            _ => Err(PyValueError::new_err(format!(
                "The side {} is none of 'greater', 'less' and 'equivalent'.",
                object.repr()?
            ))),
        }
    }
}

/// The anytime-valid p-values of the side `greater` (`P > p0`), `less` (`P < p0`) or
/// `equivalent` (`|P - p0| < margin`) of a probability after every number of draws, from
/// whether every draw satisfies the hypothesis, `_Prefixes.p_values` and `_p_values`.
#[pyfunction]
#[pyo3(signature = (satisfied, p0, side, margin = testing::MARGIN))]
pub(crate) fn stats_p_values<'py>(
    py: Python<'py>,
    satisfied: Booleans<'py>,
    p0: f64,
    side: SideArg,
    margin: f64,
) -> Bound<'py, PyArray1<f64>> {
    PyArray1::from_vec(
        py,
        testing::p_values(&booleans(&satisfied), p0, side.0, margin),
    )
}

/// The confidence sequence of a probability after the draws at the level, its lower and its
/// upper end, `confidence_sequence`.
#[pyfunction]
#[pyo3(signature = (satisfied, level = 0.95))]
pub(crate) fn stats_confidence_sequence(
    satisfied: Booleans<'_>,
    level: f64,
) -> PyResult<(f64, f64)> {
    testing::confidence_sequence(&booleans(&satisfied), 1.0 - level).map_err(uncertainty_error)
}

/// The number of draws at which a p-value first is at most `alpha`, `None` if it never is.
#[pyfunction]
pub(crate) fn stats_decide(p_values: Values<'_>, alpha: f64) -> Option<usize> {
    testing::decide(&values(&p_values), alpha)
}

/// The decision of a statement from its p-values after every number of draws and the draws at
/// which it was first shown, `_decide`: `accepted`, `rejected`, `equivalent` or `undecided`
/// (without a number of draws).
#[pyfunction]
pub(crate) fn stats_decision(
    p_greater: Values<'_>,
    p_less: Values<'_>,
    p_equivalent: Values<'_>,
    alpha: f64,
) -> (&'static str, Option<usize>) {
    let (decision, n) = testing::decision(
        &values(&p_greater),
        &values(&p_less),
        &values(&p_equivalent),
        alpha,
    );
    let name = match decision {
        Decision::Accepted => "accepted",
        Decision::Rejected => "rejected",
        Decision::Equivalent => "equivalent",
        Decision::Undecided => "undecided",
    };
    (name, n)
}

/// The draws which decide a statement at its estimate, `_needed`: `None` beyond `MAX_NEEDED`.
#[pyfunction]
pub(crate) fn stats_needed(
    estimate: f64,
    n: u64,
    p0: f64,
    margin: f64,
    alpha: f64,
) -> PyResult<Option<u64>> {
    testing::needed(estimate, n, p0, margin, alpha).map_err(uncertainty_error)
}

/// The level of a statement with a threshold in a report of `m` of them (Bonferroni),
/// `family_level`.
#[pyfunction]
pub(crate) fn stats_family_level(level: f64, m: i64) -> f64 {
    testing::family_level(level, m)
}

/// The statistic of virtual groups of `size` subjects of the predictions (a row per subject, a
/// column per data point), `_group_means`: the finite subjects themselves for groups of one,
/// else `groups` groups resampled with replacement from the stream of the seed, their mean or
/// their geometric mean.
#[pyfunction]
#[pyo3(signature = (values, size, groups, *, geometric = false, seed))]
pub(crate) fn stats_group_means<'py>(
    py: Python<'py>,
    values: Values<'py>,
    size: i64,
    groups: i64,
    geometric: bool,
    seed: Seed,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let &[subjects, points] = values.shape() else {
        return Err(PyValueError::new_err(format!(
            "The predictions must be a matrix of the subjects by the data points, not of the \
             shape {}.",
            shape_text(values.shape())
        )));
    };
    // `rng.integers(0, n, size=(groups, size))` of numpy, which a group of one does not call
    if size != 1 && (size < 0 || groups < 0) {
        return Err(PyValueError::new_err("negative dimensions are not allowed"));
    }
    let (statistics, rows) = variability::group_means(
        &self::values(&values),
        subjects,
        points,
        size.unsigned_abs() as usize,
        groups.unsigned_abs() as usize,
        geometric,
        seed.0,
    )
    .map_err(value_error)?;
    Ok(rectangle(py, statistics, rows, points))
}

/// The bands of a distribution of groups (a row per group, a column per data point): the
/// quantiles `qs` of the finite values of every data point, numpy's linear ones, a row per
/// data point.
#[pyfunction]
pub(crate) fn stats_bands<'py>(
    py: Python<'py>,
    distribution: Values<'py>,
    qs: Values<'py>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let &[rows, cols] = distribution.shape() else {
        return Err(PyValueError::new_err(format!(
            "The distribution must be a matrix of the groups by the data points, not of the \
             shape {}.",
            shape_text(distribution.shape())
        )));
    };
    let qs = values(&qs);
    let bands = variability::bands(&values(&distribution), rows, cols, &qs).map_err(value_error)?;
    Ok(rectangle(py, bands, cols, qs.len()))
}

/// Adds the functions and the constants of the statistics to the module, by the names
/// `stats_<name>` and `STATS_<NAME>`.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "STATS_MIN_POINTS_SUBSET_AGREEMENT",
        metrics::MIN_POINTS_SUBSET_AGREEMENT,
    )?;
    m.add(
        "STATS_DEFAULT_RANK_TOLERANCE",
        fisher::DEFAULT_RANK_TOLERANCE,
    )?;
    m.add("STATS_PSD_RTOL", correlation::PSD_RTOL)?;
    m.add("STATS_EPS", draws::EPS)?;
    m.add("STATS_MARGIN", testing::MARGIN)?;
    m.add("STATS_MAX_NEEDED", testing::MAX_NEEDED)?;
    m.add("STATS_BISECTIONS", testing::BISECTIONS)?;
    m.add("STATS_MIXTURE", testing::MIXTURE)?;
    m.add_function(wrap_pyfunction!(stats_p_values, m)?)?;
    m.add_function(wrap_pyfunction!(stats_confidence_sequence, m)?)?;
    m.add_function(wrap_pyfunction!(stats_decide, m)?)?;
    m.add_function(wrap_pyfunction!(stats_decision, m)?)?;
    m.add_function(wrap_pyfunction!(stats_needed, m)?)?;
    m.add_function(wrap_pyfunction!(stats_family_level, m)?)?;
    m.add_function(wrap_pyfunction!(stats_group_means, m)?)?;
    m.add_function(wrap_pyfunction!(stats_bands, m)?)?;
    m.add_function(wrap_pyfunction!(stats_unit_points, m)?)?;
    m.add_function(wrap_pyfunction!(stats_copula, m)?)?;
    m.add_function(wrap_pyfunction!(stats_normal_score_rho, m)?)?;
    m.add_function(wrap_pyfunction!(stats_nearest_correlation, m)?)?;
    m.add_function(wrap_pyfunction!(stats_is_psd, m)?)?;
    m.add_function(wrap_pyfunction!(stats_validate_covariance, m)?)?;
    m.add_function(wrap_pyfunction!(stats_gram, m)?)?;
    m.add_function(wrap_pyfunction!(stats_fisher_statistics, m)?)?;
    m.add_function(wrap_pyfunction!(stats_fisher_intervals, m)?)?;
    m.add_function(wrap_pyfunction!(stats_fisher_relative_errors, m)?)?;
    m.add_function(wrap_pyfunction!(stats_posterior_laplace, m)?)?;
    m.add_function(wrap_pyfunction!(stats_rank_correlation, m)?)?;
    m.add_function(wrap_pyfunction!(stats_normal_correlation, m)?)?;
    m.add_function(wrap_pyfunction!(stats_log_sd, m)?)?;
    m.add_function(wrap_pyfunction!(stats_lognormal_cv, m)?)?;
    m.add_function(wrap_pyfunction!(stats_sse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_mse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_rmse_from_mse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_aic_from_mse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_bic_from_mse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_r_squared, m)?)?;
    m.add_function(wrap_pyfunction!(stats_fold_counts, m)?)?;
    m.add_function(wrap_pyfunction!(stats_fold_accepted, m)?)?;
    m.add_function(wrap_pyfunction!(stats_sd_accepted, m)?)?;
    m.add_function(wrap_pyfunction!(stats_gmfe, m)?)?;
    m.add_function(wrap_pyfunction!(stats_afe, m)?)?;
    m.add_function(wrap_pyfunction!(stats_nrmse, m)?)?;
    m.add_function(wrap_pyfunction!(stats_guest_limits, m)?)?;
    m.add_function(wrap_pyfunction!(stats_within_guest, m)?)?;
    m.add_function(wrap_pyfunction!(stats_subset_agreement, m)?)?;
    m.add_function(wrap_pyfunction!(stats_mean_se, m)?)?;
    m.add_function(wrap_pyfunction!(stats_sd_se, m)?)?;
    m.add_function(wrap_pyfunction!(stats_quantile, m)?)?;
    m.add_function(wrap_pyfunction!(stats_quantiles, m)?)?;
    m.add_function(wrap_pyfunction!(stats_quantile_ci, m)?)?;
    m.add_function(wrap_pyfunction!(stats_first_order_given_data, m)?)?;
    m.add_function(wrap_pyfunction!(stats_precise, m)?)?;
    m.add_function(wrap_pyfunction!(stats_interval_parameters, m)?)?;
    Ok(())
}

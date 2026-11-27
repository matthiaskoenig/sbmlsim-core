//! The statistics of the analyses (`sbmlsim-stats`), the kernels of numpy and scipy in
//! sbmlsim, on arrays of numpy; `sbmlsim_core.stats` names them.
//!
//! An array of any shape and of any type numpy converts to doubles is read as its values in
//! the order numpy ravels it (C order), borrowed where it is contiguous and copied where it is
//! not, as `np.asarray(values, dtype=float).ravel()` of the python. An error of the arguments
//! is a `ValueError` with the message of the python function the binding replaces.

use std::borrow::Cow;

use numpy::ndarray::{ArrayD, IxDyn};
use numpy::{AllowTypeChange, PyArray1, PyArrayDyn, PyArrayLikeDyn, PyUntypedArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyFloat;
use sbmlsim_stats::{StatsError, metrics};

/// The array of values of numpy the functions take, of any shape.
type Values<'py> = PyArrayLikeDyn<'py, f64, AllowTypeChange>;

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

/// Adds the functions and the constants of the statistics to the module, by the names
/// `stats_<name>` and `STATS_<NAME>`.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "STATS_MIN_POINTS_SUBSET_AGREEMENT",
        metrics::MIN_POINTS_SUBSET_AGREEMENT,
    )?;
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
    Ok(())
}

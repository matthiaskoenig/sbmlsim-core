//! The basis of a polynomial chaos expansion (`sbmlsim-chaos`): its multi-indices, its design
//! matrix and the surrogate an expansion is, on arrays of numpy.

use numpy::ndarray::{Array1, Array2};
use numpy::{AllowTypeChange, PyArray1, PyArray2, PyArrayLikeDyn, PyUntypedArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sbmlsim_batch::thread_count;
use sbmlsim_chaos::{Basis, ChaosError, expansion};

use crate::errors::batch_error;

/// The error of a basis, of points or of coefficients which do not fit it.
fn chaos_error(error: ChaosError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The multi-indices of a truncated basis of a polynomial chaos expansion, a row per term and
/// a column per input, ordered by their total degree and then by the inputs they read; the
/// first row is the constant term. The ones of `sbmlsim.sensitivity.chaos.expansion`.
#[pyfunction]
#[pyo3(signature = (d, degree, *, q=1.0, max_interaction=None))]
pub(crate) fn chaos_expansion(
    py: Python<'_>,
    d: i64,
    degree: i64,
    q: f64,
    max_interaction: Option<i64>,
) -> PyResult<Bound<'_, PyArray2<i64>>> {
    let inputs = usize::try_from(d)
        .ok()
        .filter(|&inputs| inputs >= 1)
        .ok_or_else(|| {
            PyValueError::new_err(format!("The basis needs at least one input, not {d}."))
        })?;
    let degree = u32::try_from(degree).map_err(|_| {
        PyValueError::new_err(format!("The degree must not be negative, not {degree}."))
    })?;
    let max_interaction = max_interaction
        .map(|cap| {
            usize::try_from(cap)
                .ok()
                .filter(|&cap| cap >= 1)
                .ok_or_else(|| {
                    PyValueError::new_err(format!(
                        "The interaction order must be at least 1, not {cap}."
                    ))
                })
        })
        .transpose()?;
    let basis = py
        .detach(|| expansion(inputs, degree, q, max_interaction))
        .map_err(chaos_error)?;
    let dense = basis.multi_indices().into_iter().map(i64::from).collect();
    let rows = Array2::from_shape_vec((basis.len(), inputs), dense)
        .expect("the multi-indices are rows of the inputs");
    Ok(PyArray2::from_owned_array(py, rows))
}

/// The basis of multi-indices of numpy, a two-dimensional array of degrees.
fn basis_of(multi_indices: &PyArrayLikeDyn<'_, i64, AllowTypeChange>) -> PyResult<Basis> {
    let shape = multi_indices.shape();
    let [_, inputs] = *shape else {
        return Err(PyValueError::new_err(
            "The points and the multi-indices are two-dimensional.",
        ));
    };
    let degrees = multi_indices
        .as_array()
        .iter()
        .map(|&degree| {
            u32::try_from(degree).map_err(|_| {
                PyValueError::new_err(format!(
                    "A degree of the multi-indices must not be negative, not {degree}."
                ))
            })
        })
        .collect::<PyResult<Vec<u32>>>()?;
    Basis::from_multi_indices(inputs, &degrees).map_err(chaos_error)
}

/// The points of numpy for a basis, a row of its inputs per point, in C order.
fn points_of(
    points: &PyArrayLikeDyn<'_, f64, AllowTypeChange>,
    basis: &Basis,
) -> PyResult<Vec<f64>> {
    let shape = points.shape();
    let [_, inputs] = *shape else {
        return Err(PyValueError::new_err(
            "The points and the multi-indices are two-dimensional.",
        ));
    };
    if inputs != basis.inputs() {
        return Err(PyValueError::new_err(format!(
            "The points have {inputs} inputs, the multi-indices {}.",
            basis.inputs()
        )));
    }
    Ok(points.as_array().iter().copied().collect())
}

/// The design matrix of a basis at points of the unit cube: a row per point and a column per
/// term, bitwise the one of `sbmlsim.sensitivity.chaos.design_matrix`.
#[pyfunction]
#[pyo3(signature = (u, multi_indices, *, threads=None))]
pub(crate) fn chaos_design_matrix<'py>(
    py: Python<'py>,
    u: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    multi_indices: PyArrayLikeDyn<'py, i64, AllowTypeChange>,
    threads: Option<i64>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let basis = basis_of(&multi_indices)?;
    let points = points_of(&u, &basis)?;
    let n = points.len() / basis.inputs();
    let matrix = py
        .detach(|| basis.design_matrix(&points, threads))
        .map_err(chaos_error)?;
    let matrix = Array2::from_shape_vec((n, basis.len()), matrix)
        .expect("the design matrix has a row per point");
    Ok(PyArray2::from_owned_array(py, matrix))
}

/// The values of a polynomial chaos expansion at points of the unit cube: a row per point and
/// a column per element for coefficients of a row per term and a column per element, and a
/// value per point for coefficients of a value per term, as the product of the design matrix
/// and the coefficients is (`PolynomialChaos.predict` of sbmlsim), without the design matrix.
#[pyfunction]
#[pyo3(signature = (u, multi_indices, coefficients, *, threads=None))]
pub(crate) fn chaos_predict<'py>(
    py: Python<'py>,
    u: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    multi_indices: PyArrayLikeDyn<'py, i64, AllowTypeChange>,
    coefficients: PyArrayLikeDyn<'py, f64, AllowTypeChange>,
    threads: Option<i64>,
) -> PyResult<Bound<'py, PyAny>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let basis = basis_of(&multi_indices)?;
    let points = points_of(&u, &basis)?;
    let n = points.len() / basis.inputs();
    let (terms, elements) = match *coefficients.shape() {
        [terms] => (terms, None),
        [terms, elements] => (terms, Some(elements)),
        _ => {
            return Err(PyValueError::new_err(
                "The coefficients are one value per term or a row per term.",
            ));
        }
    };
    if terms != basis.len() {
        return Err(PyValueError::new_err(format!(
            "The basis has {} terms, the coefficients {terms}.",
            basis.len()
        )));
    }
    let values: Vec<f64> = coefficients.as_array().iter().copied().collect();
    let width = elements.unwrap_or(1);
    let predicted = py
        .detach(|| basis.predict(&points, &values, width, threads))
        .map_err(chaos_error)?;
    Ok(match elements {
        Some(elements) => PyArray2::from_owned_array(
            py,
            Array2::from_shape_vec((n, elements), predicted)
                .expect("the prediction has a row per point"),
        )
        .into_any(),
        None => PyArray1::from_owned_array(py, Array1::from_vec(predicted)).into_any(),
    })
}

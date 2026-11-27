//! The thinning of a line of a figure (`sbmlsim-thin`), the core of
//! `sbmlsim_core.chart.thinning`, on arrays of numpy.

use std::borrow::Cow;

use numpy::{AllowTypeChange, PyArray1, PyArrayLike1, PyArrayLike2, PyUntypedArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// The points of a line which are kept, Ramer-Douglas-Peucker: the first and the last one,
/// the ones of `keep`, every point which is not finite and its neighbours, and the points
/// which keep every other one within `tolerance` of the segment between the points kept
/// around it, on every row of `y`. Bitwise the points of `kept_points` of
/// `sbmlsim_core.chart.thinning` on numpy before it called this.
#[pyfunction]
#[pyo3(signature = (x, y, tolerance, keep=None))]
pub(crate) fn thin_kept_points<'py>(
    py: Python<'py>,
    x: PyArrayLike1<'py, f64, AllowTypeChange>,
    y: PyArrayLike2<'py, f64, AllowTypeChange>,
    tolerance: f64,
    keep: Option<PyArrayLike1<'py, bool, AllowTypeChange>>,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let n = x.len();
    let [rows, points] = *y.shape() else {
        unreachable!("a two-dimensional array has two dimensions")
    };
    if rows == 0 || points != n {
        return Err(PyValueError::new_err(format!(
            "The y coordinates are one or more rows of the {n} points of the x coordinates, \
             not {rows} rows of {points}."
        )));
    }
    let mut kept = match keep {
        Some(keep) if keep.len() != n => {
            return Err(PyValueError::new_err(format!(
                "The points kept in any case are {} for {n} points.",
                keep.len()
            )));
        }
        Some(keep) => keep.as_array().to_vec(),
        None => vec![false; n],
    };
    let x = x.as_array();
    let x = x
        .as_slice()
        .map_or_else(|| Cow::Owned(x.to_vec()), Cow::Borrowed);
    let y = y.as_array();
    let columns: Vec<Cow<'_, [f64]>> = y
        .rows()
        .into_iter()
        .map(|row| {
            row.to_slice()
                .map_or_else(|| Cow::Owned(row.to_vec()), Cow::Borrowed)
        })
        .collect();
    let columns: Vec<&[f64]> = columns.iter().map(AsRef::as_ref).collect();
    py.detach(|| sbmlsim_thin::kept_points(&x, &columns, tolerance, &mut kept));
    Ok(PyArray1::from_vec(py, kept))
}

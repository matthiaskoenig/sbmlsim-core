//! The global sensitivity analysis (`sbmlsim-sensitivity`): the designs of Saltelli, the
//! extended FAST and Morris on the unit cube and the estimators of their indices, on arrays of
//! numpy; `sbmlsim_core.sensitivity` names them.

use std::borrow::Cow;

use crate::errors::batch_error;
use numpy::ndarray::{Array2, Array3};
use numpy::{
    AllowTypeChange, PyArray2, PyArray3, PyArrayLike2, PyArrayLikeDyn, PyUntypedArrayMethods,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use sbmlsim_batch::thread_count;
use sbmlsim_sensitivity::{
    Bootstrap, Matrix, MatrixRef, Resamples, SensitivityError, fast_indices, fast_sample,
    fast_sample_with_phases, morris_effects, morris_sample, saltelli, sobol_indices,
    sobol_sequence,
};

/// The number of resamples of a bootstrap without a number, the default of SALib.
const NUM_RESAMPLES: usize = 100;

/// The error of the arguments of a design or an estimator.
fn sensitivity_error(error: SensitivityError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// A count of python: a positive integer.
fn count(value: i64, what: &str) -> PyResult<usize> {
    usize::try_from(value)
        .ok()
        .filter(|&value| value >= 1)
        .ok_or_else(|| PyValueError::new_err(format!("{what} is a positive integer, not {value}.")))
}

/// A seed of python: an integer of `[0, 2^64)`.
fn seed_of(seed: &Bound<'_, PyAny>) -> PyResult<u64> {
    seed.extract::<u64>().map_err(|_| {
        PyValueError::new_err(format!(
            "A seed is an integer of [0, 2^64), not {}.",
            seed.repr()
                .map_or_else(|_| "?".to_owned(), |r| r.to_string())
        ))
    })
}

/// A matrix of numpy as a matrix of the estimators, borrowed where it is contiguous.
fn matrix<'a>(array: &'a PyArrayLike2<'_, f64, AllowTypeChange>) -> (Cow<'a, [f64]>, usize, usize) {
    let [rows, columns] = *array.shape() else {
        unreachable!("a two-dimensional array has two dimensions")
    };
    let view = array.as_array();
    let data = match view.to_slice() {
        Some(slice) => Cow::Borrowed(slice),
        None => Cow::Owned(view.iter().copied().collect()),
    };
    (data, rows, columns)
}

/// A matrix of the designs as an array of numpy.
fn array<'py>(py: Python<'py>, matrix: Matrix) -> Bound<'py, PyArray2<f64>> {
    let rows = Array2::from_shape_vec((matrix.rows, matrix.columns), matrix.data)
        .expect("a matrix has its rows of its columns");
    PyArray2::from_owned_array(py, rows)
}

/// The given resamples: an array of non-negative integers of `ndim` dimensions whose shape
/// passes `check`, flattened in C order.
fn given(
    resamples: &PyArrayLikeDyn<'_, i64, AllowTypeChange>,
    ndim: usize,
    shape: &str,
    check: impl Fn(&[usize]) -> bool,
) -> PyResult<Vec<usize>> {
    if resamples.ndim() != ndim || !check(resamples.shape()) {
        return Err(PyValueError::new_err(format!(
            "The resamples are an array {shape}, not of the shape {:?}.",
            resamples.shape()
        )));
    }
    resamples
        .as_array()
        .iter()
        .map(|&index| {
            usize::try_from(index).map_err(|_| {
                PyValueError::new_err(format!("A resample draws no negative index, not {index}."))
            })
        })
        .collect()
}

/// The resamples of a bootstrap: `num_resamples` of the stream of `seed`, or the given ones.
fn resamples<'a>(
    num_resamples: Option<i64>,
    seed: Option<&Bound<'_, PyAny>>,
    given: Option<&'a [usize]>,
) -> PyResult<Resamples<'a>> {
    match (seed, given) {
        (Some(seed), None) => Ok(Resamples::Seeded {
            count: num_resamples.map_or(Ok(NUM_RESAMPLES), |n| count(n, "num_resamples"))?,
            seed: seed_of(seed)?,
        }),
        (None, Some(given)) if num_resamples.is_none() => Ok(Resamples::Given(given)),
        (None, Some(_)) => Err(PyValueError::new_err(
            "The given resamples have their number: give num_resamples with a seed only.",
        )),
        (Some(_), Some(_)) => Err(PyValueError::new_err(
            "The resamples are drawn from a seed or given, not both.",
        )),
        (None, None) => Err(PyValueError::new_err(
            "The bootstrap needs a seed of its resamples or the resamples.",
        )),
    }
}

/// The points `skip..skip + n` of the Sobol' sequence of `d` dimensions, a row per point,
/// scrambled with the stream of `seed` or not scrambled for `None`.
#[pyfunction]
#[pyo3(signature = (d, n, *, seed, skip=0))]
pub(crate) fn sensitivity_sobol_sequence<'py>(
    py: Python<'py>,
    d: i64,
    n: i64,
    seed: Option<Bound<'py, PyAny>>,
    skip: u64,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (d, n) = (count(d, "d")?, count(n, "n")?);
    let seed = seed.as_ref().map(seed_of).transpose()?;
    let points = py
        .detach(|| sobol_sequence(d, n, seed, skip))
        .map_err(sensitivity_error)?;
    Ok(array(py, points))
}

/// The matrix of Saltelli of `d` parameters on the Sobol' sequence of `2 d` dimensions.
#[pyfunction]
#[pyo3(signature = (d, n, *, seed, second_order=false, skip=0))]
pub(crate) fn sensitivity_saltelli<'py>(
    py: Python<'py>,
    d: i64,
    n: i64,
    seed: Option<Bound<'py, PyAny>>,
    second_order: bool,
    skip: u64,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (d, n) = (count(d, "d")?, count(n, "n")?);
    let seed = seed.as_ref().map(seed_of).transpose()?;
    let points = py
        .detach(|| saltelli(d, n, second_order, seed, skip))
        .map_err(sensitivity_error)?;
    Ok(array(py, points))
}

/// The design of the extended FAST of `d` parameters, `n` points per parameter, with the
/// phases of the stream of `seed` or the given ones.
#[pyfunction]
#[pyo3(signature = (d, n, *, m=4, seed=None, phases=None))]
pub(crate) fn sensitivity_fast_sample<'py>(
    py: Python<'py>,
    d: i64,
    n: i64,
    m: i64,
    seed: Option<Bound<'py, PyAny>>,
    phases: Option<Vec<f64>>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (d, n, m) = (count(d, "d")?, count(n, "n")?, count(m, "m")?);
    let points = match (seed, phases) {
        (Some(seed), None) => {
            let seed = seed_of(&seed)?;
            py.detach(|| fast_sample(d, n, m, seed))
        }
        (None, Some(phases)) => py.detach(|| fast_sample_with_phases(d, n, m, &phases)),
        _ => {
            return Err(PyValueError::new_err(
                "A FAST design takes the seed of its phases or the phases, one of them.",
            ));
        }
    }
    .map_err(sensitivity_error)?;
    Ok(array(py, points))
}

/// The design of Morris of `d` parameters: `trajectories` trajectories of `d + 1` points on
/// the grid of `levels` levels of `[0, 1]`.
#[pyfunction]
#[pyo3(signature = (d, trajectories, *, levels=4, seed))]
pub(crate) fn sensitivity_morris_sample<'py>(
    py: Python<'py>,
    d: i64,
    trajectories: i64,
    levels: i64,
    seed: Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let (d, trajectories) = (count(d, "d")?, count(trajectories, "trajectories")?);
    let levels = count(levels, "levels")?;
    let seed = seed_of(&seed)?;
    let points = py
        .detach(|| morris_sample(d, trajectories, levels, seed))
        .map_err(sensitivity_error)?;
    Ok(array(py, points))
}

/// The indices of a row per parameter and a column per element into a dictionary.
fn put<'py>(found: &Bound<'py, PyDict>, key: &str, matrix: Matrix) -> PyResult<()> {
    found.set_item(key, array(found.py(), matrix))
}

/// The Sobol indices of the values of the points of a matrix of Saltelli, every column at once:
/// `S1`, `S1_conf`, `ST`, `ST_conf` of the shape `(d, elements)` and with the second order
/// `S2`, `S2_conf` of the shape `(d, d, elements)`.
#[pyfunction]
#[pyo3(signature = (
    y,
    d,
    *,
    second_order=false,
    conf_level=0.95,
    num_resamples=None,
    seed=None,
    resamples=None,
    threads=None
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sensitivity_sobol_indices<'py>(
    py: Python<'py>,
    y: PyArrayLike2<'py, f64, AllowTypeChange>,
    d: i64,
    second_order: bool,
    conf_level: f64,
    num_resamples: Option<i64>,
    seed: Option<Bound<'py, PyAny>>,
    resamples: Option<PyArrayLikeDyn<'py, i64, AllowTypeChange>>,
    threads: Option<i64>,
) -> PyResult<Bound<'py, PyDict>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let d = count(d, "d")?;
    let (data, rows, elements) = matrix(&y);
    let step = if second_order { 2 * d + 2 } else { d + 2 };
    let n = rows / step;
    let given = resamples
        .as_ref()
        .map(|r| given(r, 2, "(resamples, n) of the samples", |s| s[1] == n))
        .transpose()?;
    let bootstrap = Bootstrap {
        conf_level,
        resamples: self::resamples(num_resamples, seed.as_ref(), given.as_deref())?,
    };
    let values = MatrixRef::new(&data, rows, elements).map_err(sensitivity_error)?;
    let indices = py
        .detach(|| sobol_indices(values, d, second_order, &bootstrap, threads))
        .map_err(sensitivity_error)?;
    let found = PyDict::new(py);
    put(&found, "S1", indices.s1)?;
    put(&found, "S1_conf", indices.s1_conf)?;
    put(&found, "ST", indices.st)?;
    put(&found, "ST_conf", indices.st_conf)?;
    if let (Some(s2), Some(s2_conf)) = (indices.s2, indices.s2_conf) {
        for (key, values) in [("S2", s2), ("S2_conf", s2_conf)] {
            let values = Array3::from_shape_vec((d, d, elements), values.data)
                .expect("the second order is a matrix per element");
            found.set_item(key, PyArray3::from_owned_array(py, values))?;
        }
    }
    Ok(found)
}

/// The indices of the extended FAST of the values of the points of a FAST design, every column
/// at once: `S1`, `S1_conf`, `ST`, `ST_conf` of the shape `(d, elements)`.
#[pyfunction]
#[pyo3(signature = (
    y, d, *, m=4, conf_level=0.95, num_resamples=None, seed=None, resamples=None, threads=None
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sensitivity_fast_indices<'py>(
    py: Python<'py>,
    y: PyArrayLike2<'py, f64, AllowTypeChange>,
    d: i64,
    m: i64,
    conf_level: f64,
    num_resamples: Option<i64>,
    seed: Option<Bound<'py, PyAny>>,
    resamples: Option<PyArrayLikeDyn<'py, i64, AllowTypeChange>>,
    threads: Option<i64>,
) -> PyResult<Bound<'py, PyDict>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let (d, m) = (count(d, "d")?, count(m, "m")?);
    let (data, rows, elements) = matrix(&y);
    let given = resamples
        .as_ref()
        .map(|r| {
            given(r, 2, "(d, resamples) of the starts of the windows", |s| {
                s[0] == d
            })
        })
        .transpose()?;
    let bootstrap = Bootstrap {
        conf_level,
        resamples: self::resamples(num_resamples, seed.as_ref(), given.as_deref())?,
    };
    let values = MatrixRef::new(&data, rows, elements).map_err(sensitivity_error)?;
    let indices = py
        .detach(|| fast_indices(values, d, m, &bootstrap, threads))
        .map_err(sensitivity_error)?;
    let found = PyDict::new(py);
    put(&found, "S1", indices.s1)?;
    put(&found, "S1_conf", indices.s1_conf)?;
    put(&found, "ST", indices.st)?;
    put(&found, "ST_conf", indices.st_conf)?;
    Ok(found)
}

/// The elementary effects of the values of the trajectories of a Morris design, every column at
/// once: `mu`, `mu_star`, `sigma`, `mu_star_conf` of the shape `(d, elements)`.
#[pyfunction]
#[pyo3(signature = (
    cube,
    y,
    *,
    levels=4,
    conf_level=0.95,
    num_resamples=None,
    seed=None,
    resamples=None,
    threads=None
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sensitivity_morris_effects<'py>(
    py: Python<'py>,
    cube: PyArrayLike2<'py, f64, AllowTypeChange>,
    y: PyArrayLike2<'py, f64, AllowTypeChange>,
    levels: i64,
    conf_level: f64,
    num_resamples: Option<i64>,
    seed: Option<Bound<'py, PyAny>>,
    resamples: Option<PyArrayLikeDyn<'py, i64, AllowTypeChange>>,
    threads: Option<i64>,
) -> PyResult<Bound<'py, PyDict>> {
    let threads = thread_count(threads).map_err(batch_error)?;
    let levels = count(levels, "levels")?;
    let (points, rows, d) = matrix(&cube);
    let (data, value_rows, elements) = matrix(&y);
    let trajectories = rows / (d + 1);
    let given = resamples
        .as_ref()
        .map(|r| {
            given(
                r,
                3,
                "(d, resamples, trajectories) of the drawn trajectories",
                |s| s[0] == d && s[2] == trajectories,
            )
        })
        .transpose()?;
    let bootstrap = Bootstrap {
        conf_level,
        resamples: self::resamples(num_resamples, seed.as_ref(), given.as_deref())?,
    };
    let cube = MatrixRef::new(&points, rows, d).map_err(sensitivity_error)?;
    let values = MatrixRef::new(&data, value_rows, elements).map_err(sensitivity_error)?;
    let effects = py
        .detach(|| morris_effects(cube, values, levels, &bootstrap, threads))
        .map_err(sensitivity_error)?;
    let found = PyDict::new(py);
    put(&found, "mu", effects.mu)?;
    put(&found, "mu_star", effects.mu_star)?;
    put(&found, "sigma", effects.sigma)?;
    put(&found, "mu_star_conf", effects.mu_star_conf)?;
    Ok(found)
}

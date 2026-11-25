//! The figures (`sbmlsim-plot`): their SVG and PNG, their geometry, the pick of a point and the
//! colour scales, on the JSON text of a `figure/1` spec.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use sbmlsim_plot::{
    COLOR_SCALES, ColorScale, ColorScaleName, Figure, SpecError, geometry, parse, pick as pick_of,
};

/// The error of a spec which does not parse: the path of the field and what is wrong.
fn spec_error(error: SpecError) -> PyErr {
    PyValueError::new_err(format!("{}: {}", error.path, error.message))
}

/// Parses a spec, releasing the GIL.
fn figure_of(py: Python<'_>, spec: &str) -> PyResult<Figure> {
    py.detach(|| parse(spec)).map_err(spec_error)
}

/// The SVG of a figure given as the JSON text of a `figure/1` spec.
///
/// Raises `ValueError` with the path and message of an invalid spec.
#[pyfunction]
pub(crate) fn render_svg(py: Python<'_>, spec: &str) -> PyResult<String> {
    py.detach(|| parse(spec).map(|figure| sbmlsim_plot::render_svg(&figure)))
        .map_err(spec_error)
}

/// The PNG of a figure at `dpi`.
///
/// Raises `ValueError` for an invalid spec and `RuntimeError` for a figure which cannot be
/// rasterized.
#[pyfunction]
#[pyo3(signature = (spec, dpi=144.0))]
pub(crate) fn render_png<'py>(
    py: Python<'py>,
    spec: &str,
    dpi: f64,
) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
    let png = py
        .detach(|| {
            parse(spec)
                .map(|figure| sbmlsim_plot::render_png(&figure, dpi))
                .map_err(spec_error)
        })?
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(pyo3::types::PyBytes::new(py, &png))
}

/// The panels, scales and legend entries of a figure in its pixels.
#[pyfunction]
pub(crate) fn figure_geometry<'py>(py: Python<'py>, spec: &str) -> PyResult<Bound<'py, PyAny>> {
    let figure = figure_of(py, spec)?;
    let geometry = py.detach(|| geometry(&figure));
    pythonize::pythonize(py, &geometry).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// The data point nearest to the pixel `(x, y)` within `radius` pixels, or `None`.
#[pyfunction]
#[pyo3(signature = (spec, x, y, radius=8.0))]
pub(crate) fn pick<'py>(
    py: Python<'py>,
    spec: &str,
    x: f64,
    y: f64,
    radius: f64,
) -> PyResult<Bound<'py, PyAny>> {
    let figure = figure_of(py, spec)?;
    match py.detach(|| pick_of(&figure, x, y, radius)) {
        Some(hit) => {
            pythonize::pythonize(py, &hit).map_err(|e| PyRuntimeError::new_err(e.to_string()))
        }
        None => Ok(py.None().into_bound(py)),
    }
}

/// The colours (`#rrggbb`) of `values` on a named colour scale.
///
/// Raises `ValueError` for an unknown name.
#[pyfunction]
#[pyo3(signature = (name, low, high, values, center=None, log=false))]
pub(crate) fn color_scale(
    py: Python<'_>,
    name: &str,
    low: f64,
    high: f64,
    values: Vec<f64>,
    center: Option<f64>,
    log: bool,
) -> PyResult<Vec<String>> {
    let name = match name {
        "viridis" => ColorScaleName::Viridis,
        "magma" => ColorScaleName::Magma,
        "rdbu" => ColorScaleName::Rdbu,
        "coolwarm" => ColorScaleName::Coolwarm,
        "greys" => ColorScaleName::Greys,
        _ => {
            return Err(PyValueError::new_err(format!(
                "unknown color scale `{name}`, expected one of {}",
                COLOR_SCALES.join(", ")
            )));
        }
    };
    let scale = ColorScale {
        name,
        low,
        high,
        center,
        log,
    };
    Ok(py.detach(|| sbmlsim_plot::color_scale(&scale, &values)))
}

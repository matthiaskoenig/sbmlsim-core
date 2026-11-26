//! The figures (`sbmlsim-plot`): their SVG and PNG, their geometry, the pick of a point and the
//! colour scales, on the JSON text of a `figure/1` spec.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use sbmlsim_plot::{
    ColorScale, ColorScaleName, Figure, FontWeight, SpecError, geometry, parse, pick as pick_of,
    text_width_weighted,
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

/// The PNG of a figure at `dpi`, of the drawn size of the figure times `dpi / 72`.
///
/// Raises `ValueError` for an invalid spec and `RuntimeError` for a figure which cannot be
/// rasterized, e.g. an image of more than 64 Mpx.
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

/// The drawn size (`width`, `height`), panels, scales and legend entries of a figure in its
/// pixels; legends outside the plots make the drawn size larger than the size of the spec.
#[pyfunction]
pub(crate) fn figure_geometry<'py>(py: Python<'py>, spec: &str) -> PyResult<Bound<'py, PyAny>> {
    let figure = figure_of(py, spec)?;
    let layout = py.detach(|| geometry(&figure));
    pythonize::pythonize(py, &layout).map_err(|e| PyRuntimeError::new_err(e.to_string()))
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

/// The colours (`#rrggbb`) of `values` on a named colour scale, the values `low..high` on the
/// part `start..end` of the colour map.
///
/// Raises `ValueError` for an unknown name and for a part which is not `0 <= start < end <= 1`.
#[pyfunction]
#[pyo3(signature = (name, low, high, values, center=None, log=false, start=0.0, end=1.0))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn color_scale(
    py: Python<'_>,
    name: &str,
    low: f64,
    high: f64,
    values: Vec<f64>,
    center: Option<f64>,
    log: bool,
    start: f64,
    end: f64,
) -> PyResult<Vec<String>> {
    if !(start >= 0.0 && start < end && end <= 1.0) {
        return Err(PyValueError::new_err(format!(
            "start {start} and end {end} are no part of the colour map, expected 0 <= start < end <= 1"
        )));
    }
    let name: ColorScaleName = name.parse().map_err(PyValueError::new_err)?;
    let scale = ColorScale {
        name,
        low,
        high,
        center,
        log,
        start,
        end,
    };
    Ok(py.detach(|| sbmlsim_plot::color_scale(&scale, &values)))
}

/// The width in pixels of a text at a font size in pixels, in Inter Regular or Bold: the measure
/// the layout of the figures uses (since 0.28.0). The width of a text of several lines is the
/// width of its widest line.
#[pyfunction]
#[pyo3(signature = (text, size, bold=false))]
pub(crate) fn text_width(text: &str, size: f64, bold: bool) -> f64 {
    let weight = if bold {
        FontWeight::Bold
    } else {
        FontWeight::Normal
    };
    text_width_weighted(text, size, weight)
}

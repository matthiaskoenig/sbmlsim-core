//! The figures (`sbmlsim-plot`): their SVG and PNG, their geometry, the pick of a point and the
//! colour scales, on the JSON text of a `figure/1` spec.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use sbmlsim_plot::{
    ColorScale, ColorScaleName, ColorStop, Figure, FontFamily, FontWeight, SpecError, check_stops,
    geometry, parse, pick as pick_of, text_width_family,
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

/// The colour map of [`color_scale`]: a name, or stops `(position, "#rrggbb")` (since 0.31.0).
#[derive(FromPyObject)]
pub(crate) enum ColorMap {
    Name(String),
    Stops(Vec<(f64, String)>),
}

/// The colours (`#rrggbb`) of `values` on a colour scale, the values `low..high` on the
/// part `start..end` of the colour map: a named map, or a map of stops (since 0.31.0).
///
/// `gamma` (since 0.33.0) makes the scale a power scale, matplotlib's `PowerNorm`.
///
/// Raises `ValueError` for an unknown name, for stops which are no colour map, for a part
/// which is not `0 <= start < end <= 1` and for a `gamma` which is not positive.
#[pyfunction]
#[pyo3(signature = (name, low, high, values, center=None, log=false, start=0.0, end=1.0, gamma=None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn color_scale(
    py: Python<'_>,
    name: ColorMap,
    low: f64,
    high: f64,
    values: Vec<f64>,
    center: Option<f64>,
    log: bool,
    start: f64,
    end: f64,
    gamma: Option<f64>,
) -> PyResult<Vec<String>> {
    if gamma.is_some_and(|g| !(g > 0.0 && g.is_finite())) {
        return Err(PyValueError::new_err(format!(
            "gamma {} is no exponent of a power scale, expected a positive number",
            gamma.unwrap_or(f64::NAN)
        )));
    }
    if !(start >= 0.0 && start < end && end <= 1.0) {
        return Err(PyValueError::new_err(format!(
            "start {start} and end {end} are no part of the colour map, expected 0 <= start < end <= 1"
        )));
    }
    let (name, stops) = match name {
        ColorMap::Name(n) => (
            Some(n.parse::<ColorScaleName>().map_err(PyValueError::new_err)?),
            None,
        ),
        ColorMap::Stops(stops) => {
            let stops: Vec<ColorStop> = stops.into_iter().map(|(p, c)| ColorStop(p, c)).collect();
            if let Err((k, message)) = check_stops(&stops) {
                let at = k.map_or(String::new(), |k| format!("stop {k}: "));
                return Err(PyValueError::new_err(format!("{at}{message}")));
            }
            (None, Some(stops))
        }
    };
    let scale = ColorScale {
        name,
        stops,
        low,
        high,
        center,
        log,
        gamma,
        start,
        end,
    };
    Ok(py.detach(|| sbmlsim_plot::color_scale(&scale, &values)))
}

/// The width in pixels of a text at a font size in pixels, in Inter Regular or Bold: the measure
/// the layout of the figures uses (since 0.28.0). The width of a text of several lines is the
/// width of its widest line. `family` is `sans` (Inter) or `monospace`, a text element of that
/// family, 0.6 of the size per character (since 0.31.1).
///
/// Raises `ValueError` for another family.
#[pyfunction]
#[pyo3(signature = (text, size, bold=false, family="sans"))]
pub(crate) fn text_width(text: &str, size: f64, bold: bool, family: &str) -> PyResult<f64> {
    let weight = if bold {
        FontWeight::Bold
    } else {
        FontWeight::Normal
    };
    let family = match family {
        "sans" => FontFamily::Sans,
        "monospace" => FontFamily::Monospace,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown font family `{other}`, expected `sans` or `monospace`"
            )));
        }
    };
    Ok(text_width_family(text, size, weight, family))
}

/// The `png` of an image element of a figure (since 0.34.0): the base64 of the PNG of an RGB
/// or RGBA image, an array of shape `(rows, columns, 3)` or `(rows, columns, 4)` whose first
/// row is the top of the image, of integers from 0 to 255 or of numbers from 0 to 1, as
/// matplotlib's `imshow` reads an image.
///
/// Raises `ValueError` for another shape, for values out of their range and for an image of
/// more than 64 Mpx.
#[pyfunction]
pub(crate) fn image_png(py: Python<'_>, pixels: &Bound<'_, PyAny>) -> PyResult<String> {
    use numpy::PyUntypedArrayMethods;
    // the kind of the numbers of the array numpy makes of the pixels: integers or others
    let array = py.import("numpy")?.call_method1("asarray", (pixels,))?;
    let kind: String = array.getattr("dtype")?.getattr("kind")?.extract()?;
    let pixels: numpy::PyArrayLikeDyn<'_, f64, numpy::AllowTypeChange> = array.extract()?;
    let shape = pixels.shape().to_vec();
    let [rows, columns, channels] = shape[..] else {
        return Err(PyValueError::new_err(format!(
            "an image is an array of shape (rows, columns, 3 or 4), got {shape:?}"
        )));
    };
    if channels != 3 && channels != 4 {
        return Err(PyValueError::new_err(format!(
            "an image has 3 (RGB) or 4 (RGBA) channels, got {channels}"
        )));
    }
    // integers are the bytes of the channels, other numbers their shares of 255
    let integers = matches!(kind.as_str(), "u" | "i" | "b");
    let (scale, top) = if integers { (1.0, 255.0) } else { (255.0, 1.0) };
    let mut bytes = Vec::with_capacity(rows * columns * channels);
    for v in pixels.as_array().iter() {
        if !(0.0..=top).contains(v) {
            return Err(PyValueError::new_err(format!(
                "the value {v} of an image is not between 0 and {top}"
            )));
        }
        bytes.push((v * scale).round() as u8);
    }
    let (w, h) = (
        u32::try_from(columns).unwrap_or(u32::MAX),
        u32::try_from(rows).unwrap_or(u32::MAX),
    );
    py.detach(|| sbmlsim_plot::encode_png(w, h, channels == 4, &bytes))
        .map_err(PyValueError::new_err)
}

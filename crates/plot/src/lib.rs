//! The figures of sbmlsim: the figure spec `figure/1`, the JSON description of a figure which
//! the python package, the web app and the renderer share, with its parsing and validation, and
//! its renderer to SVG ([`render_svg`]).

mod colors;
mod error;
mod ink;
mod interact;
mod layout;
#[cfg(feature = "png")]
mod png;
mod render;
mod spec;
mod svg;
mod text;

pub use colors::{COLOR_SCALES, NO_VALUE_COLOR, color_scale};
pub use error::{RenderError, SpecError};
pub use interact::{
    FigurePlan, Geometry, LegendEntryGeometry, Pick, PixelRect, PlotGeometry, Scale, geometry,
    geometry_of, pick, pick_in,
};
#[cfg(feature = "png")]
pub use png::{MAX_PIXELS, render_png};
pub use render::render_svg;
pub use spec::*;
pub use text::text_width;

/// The contract id of the figure spec, the value of its `contract` field.
pub const FIGURE_CONTRACT: &str = "figure/1";

/// Parses a figure from JSON text and validates it.
///
/// Unknown fields are refused, as are a contract other than [`FIGURE_CONTRACT`] and every
/// inconsistency between fields (lengths, grid spans, axes, duplicate ids). The error carries
/// the path of the offending field, e.g. `plots[0].elements[1].y`.
pub fn parse(json: &str) -> Result<Figure, SpecError> {
    let mut de = serde_json::Deserializer::from_str(json);
    let figure: Figure = serde_path_to_error::deserialize(&mut de).map_err(|e| {
        let path = e.path().to_string();
        let message = match non_json_number(json, e.inner()) {
            Some(token) => format!(
                "NaN and Infinity are not JSON, a gap is written as `null`: found `{token}` at \
                 line {} column {}",
                e.inner().line(),
                e.inner().column()
            ),
            None => e.inner().to_string(),
        };
        SpecError::from_serde(path, message)
    })?;
    spec::validate(&figure)?;
    Ok(figure)
}

/// The token `NaN`, `Infinity` or `-Infinity` at the position of a syntax error, which
/// `json.dumps` of python and `JSON5` write but JSON has not.
fn non_json_number(json: &str, error: &serde_json::Error) -> Option<&'static str> {
    if error.classify() != serde_json::error::Category::Syntax || error.line() == 0 {
        return None;
    }
    let line = json.split('\n').nth(error.line() - 1)?;
    let at = line.as_bytes().get(error.column().checked_sub(1)?..)?;
    let minus = error.column() >= 2 && line.as_bytes()[error.column() - 2] == b'-';
    if at.starts_with(b"NaN") {
        Some(if minus { "-NaN" } else { "NaN" })
    } else if at.starts_with(b"Infinity") {
        Some(if minus { "-Infinity" } else { "Infinity" })
    } else {
        None
    }
}

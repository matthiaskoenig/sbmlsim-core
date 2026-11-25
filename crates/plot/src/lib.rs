//! The figures of sbmlsim: the figure spec `figure/1`, the JSON description of a figure which
//! the python package, the web app and the renderer share, with its parsing and validation, and
//! its renderer to SVG ([`render_svg`]).

mod error;
mod render;
mod spec;
mod svg;
mod text;

pub use error::SpecError;
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
        SpecError::from_serde(path, e.inner().to_string())
    })?;
    spec::validate(&figure)?;
    Ok(figure)
}

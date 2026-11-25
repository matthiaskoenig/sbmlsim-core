//! The PNG of a figure: the SVG of the renderer, rasterized with resvg.
//!
//! The font database holds the bundled Inter (Regular and Bold, version 4.001, the font the
//! layout measures text with) and nothing else, so the image does not depend on the fonts of
//! the machine, and every generic family of the SVG resolves to Inter.

use crate::error::RenderError;
use crate::interact::FigurePlan;
use crate::spec::Figure;
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

const REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");

/// The largest number of pixels of an image, 64 Mpx (`64 * 1024 * 1024`), whose pixmap of four
/// bytes per pixel is 256 MiB, so neither a dpi nor a figure grown by its legends can ask for
/// gigabytes. The size is the drawn size of the figure times `dpi / 72` on each side.
pub const MAX_PIXELS: f64 = 64.0 * 1024.0 * 1024.0;

/// The database of the bundled fonts, built once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(REGULAR.to_vec());
        db.load_font_data(BOLD.to_vec());
        let family = "Inter";
        db.set_sans_serif_family(family);
        db.set_serif_family(family);
        db.set_cursive_family(family);
        db.set_fantasy_family(family);
        db.set_monospace_family(family);
        Arc::new(db)
    })
    .clone()
}

fn error(message: impl Into<String>) -> RenderError {
    RenderError {
        message: message.into(),
    }
}

/// Rasterizes the SVG of a figure at `dpi` (72 is one pixel per unit of the figure).
pub(crate) fn rasterize(svg: &str, dpi: f64) -> Result<Vec<u8>, RenderError> {
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(error(format!("dpi must be a positive number, got {dpi}")));
    }
    let options = usvg::Options {
        fontdb: fonts(),
        font_family: "Inter".to_string(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| error(e.to_string()))?;
    let scale = (dpi / 72.0) as f32;
    let size = tree.size();
    let width = (f64::from(size.width()) * dpi / 72.0).round();
    let height = (f64::from(size.height()) * dpi / 72.0).round();
    if width < 1.0 || height < 1.0 {
        return Err(error(format!(
            "the image would be {width} by {height} pixels"
        )));
    }
    if width * height > MAX_PIXELS {
        return Err(error(format!(
            "the image would be {width} by {height} pixels at dpi {dpi}, more than the {} \
             pixels (64 Mpx) of an image",
            MAX_PIXELS as u64
        )));
    }
    let mut pixmap = tiny_skia::Pixmap::new(width as u32, height as u32)
        .ok_or_else(|| error("no pixmap of that size"))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| error(e.to_string()))
}

/// The PNG of a figure at `dpi`, of `width * dpi / 72` by `height * dpi / 72` pixels on white,
/// where `width` and `height` are the drawn size of the figure ([`crate::Geometry`]), which
/// legends outside a plot make larger than the size of the spec. An image of more than
/// [`MAX_PIXELS`] pixels is refused.
pub fn render_png(figure: &Figure, dpi: f64) -> Result<Vec<u8>, RenderError> {
    FigurePlan::new(figure).render_png(figure, dpi)
}

impl FigurePlan {
    /// The PNG of the figure drawn on this plan, see [`render_png`].
    pub fn render_png(&self, figure: &Figure, dpi: f64) -> Result<Vec<u8>, RenderError> {
        rasterize(&self.render_svg(figure), dpi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_holds_the_bundled_fonts_only() {
        assert_eq!(fonts().len(), 2);
    }
}

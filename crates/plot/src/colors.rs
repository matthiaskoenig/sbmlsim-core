//! The colour scales of the figures: matplotlib's colour maps as tables of 256 colours, and the
//! map from values to their colours ([`color_scale`]).
//!
//! A value becomes its share `t` of the scale, `(v - low) / (high - low)`, or of the decades
//! with `log`; with a `center` the values from `low` to `center` take the lower half of the
//! table and the values from `center` to `high` the upper half (matplotlib's `TwoSlopeNorm`).
//! The colour of `t` is the entry `floor(256 t)` of the table, the first one below the scale and
//! the last one above it, which is how matplotlib reads a colour map, so a value has the colour
//! matplotlib gives it. A scale may take a part `start..end` of the table (since 0.28.0): the
//! share `t`, held to `0..=1`, becomes `start + t (end - start)` before the table is read, so
//! the values `low..high` take exactly the colours of that part and a value beyond an end the
//! colour of that end. A value without a colour (NaN, or not positive on a log scale) takes
//! [`NO_VALUE_COLOR`]. The tables are constant data and the logarithm is `libm`'s, so the native
//! and the wasm build give the same colours.

use crate::spec::{ColorScale, ColorScaleName};

include!("colors_table.rs");

/// The names of the colour scales, the values of `name` of a scale of the spec.
pub const COLOR_SCALES: &[&str] = &["viridis", "magma", "rdbu", "coolwarm", "greys"];

/// The colour of a value which has none: NaN, or a value which is not positive on a log scale
/// (matplotlib's `lightgrey`, the colour of an undefined cell of the heatmaps of sbmlsim).
pub const NO_VALUE_COLOR: &str = "#d3d3d3";

/// The table of a colour scale.
fn table(name: ColorScaleName) -> &'static [u32; 256] {
    match name {
        ColorScaleName::Viridis => &VIRIDIS,
        ColorScaleName::Magma => &MAGMA,
        ColorScaleName::Rdbu => &RDBU,
        ColorScaleName::Coolwarm => &COOLWARM,
        ColorScaleName::Greys => &GREYS,
    }
}

/// The coordinate of a value on a scale: the value, or its `log10` on a log scale; NaN for a
/// value the scale cannot show.
fn coordinate(scale: &ColorScale, v: f64) -> f64 {
    if !scale.log {
        v
    } else if v > 0.0 {
        libm::log10(v)
    } else {
        f64::NAN
    }
}

/// The share of a scale a value lies at: 0 at `low`, 1 at `high`, 0.5 at `center`; outside of
/// `0..=1` beyond the ends, NaN for a value without a colour.
///
/// A scale whose ends are no span (a caller of [`color_scale`] may hand one in) puts every value
/// at its middle.
pub(crate) fn share(scale: &ColorScale, v: f64) -> f64 {
    let u = coordinate(scale, v);
    if u.is_nan() {
        return f64::NAN;
    }
    let (lo, hi) = (coordinate(scale, scale.low), coordinate(scale, scale.high));
    if !(lo.is_finite() && hi.is_finite() && hi > lo) {
        return 0.5;
    }
    match scale.center.map(|c| coordinate(scale, c)) {
        Some(c) if c.is_finite() && c >= lo && c <= hi => {
            if u < c {
                if c > lo {
                    0.5 * (u - lo) / (c - lo)
                } else {
                    0.0
                }
            } else if hi > c {
                0.5 + 0.5 * (u - c) / (hi - c)
            } else {
                1.0
            }
        }
        _ => (u - lo) / (hi - lo),
    }
}

/// The colour `#rrggbb` of a share of a scale, [`NO_VALUE_COLOR`] for NaN.
pub(crate) fn color_at(name: ColorScaleName, t: f64) -> String {
    if t.is_nan() {
        return NO_VALUE_COLOR.to_string();
    }
    let n = 256.0;
    let index = if t < 0.0 {
        0
    } else if t >= 1.0 {
        255
    } else {
        ((t * n).floor() as usize).min(255)
    };
    format!("#{:06x}", table(name)[index])
}

/// The share of the table of a scale a share of the scale reads: `start + t (end - start)`
/// with `t` held to `0..=1`; NaN stays NaN.
fn on_table(scale: &ColorScale, t: f64) -> f64 {
    if t.is_nan() {
        return f64::NAN;
    }
    scale.start + t.clamp(0.0, 1.0) * (scale.end - scale.start)
}

/// The colour `#rrggbb` of a value on a scale, [`NO_VALUE_COLOR`] for a value without one.
pub(crate) fn color_of(scale: &ColorScale, v: f64) -> String {
    color_at(scale.name, on_table(scale, share(scale, v)))
}

/// The colours `#rrggbb` of values on a colour scale, one per value.
///
/// The values `low..high` take the part `start..end` of the colour map. A value below `low` or
/// above `high` takes the colour of that end; NaN, and a value which is not positive on a log
/// scale, take [`NO_VALUE_COLOR`]. A scale whose ends are no span gives every value the middle
/// colour of its part. Used for the cells of a heatmap, a colour bar and the lines of a scan.
pub fn color_scale(scale: &ColorScale, values: &[f64]) -> Vec<String> {
    values.iter().map(|v| color_of(scale, *v)).collect()
}

/// Whether a dark text reads on a colour `#rrggbb`: the luminance of the colour (Rec. 709
/// weights of the encoded channels, as the heatmaps of sbmlsim) above 0.45.
pub(crate) fn dark_text_on(color: &str) -> bool {
    let channel = |k: usize| {
        color
            .get(1 + 2 * k..3 + 2 * k)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, |b| f64::from(b) / 255.0)
    };
    0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2) > 0.45
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_a_table() {
        for (name, text) in [
            (ColorScaleName::Viridis, "viridis"),
            (ColorScaleName::Magma, "magma"),
            (ColorScaleName::Rdbu, "rdbu"),
            (ColorScaleName::Coolwarm, "coolwarm"),
            (ColorScaleName::Greys, "greys"),
        ] {
            assert!(COLOR_SCALES.contains(&text));
            assert_eq!(table(name).len(), 256);
        }
    }

    #[test]
    fn text_contrast() {
        assert!(dark_text_on("#fde725"));
        assert!(!dark_text_on("#440154"));
        assert!(dark_text_on("#ffffff"));
    }
}

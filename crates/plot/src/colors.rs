//! The colour scales of the figures: matplotlib's colour maps as tables of 256 colours, and the
//! map from values to their colours ([`color_scale`]).
//!
//! The maps are the sequential and the diverging maps of matplotlib (since 0.30.0; viridis,
//! magma, rdbu, coolwarm and greys before), each named in lower case (`blues`, `purd`,
//! `ylorrd`) and with a reversed variant `<name>_r` whose colours are those of matplotlib's
//! `<name>_r` ([`ColorScaleName`], [`COLOR_SCALES`]).
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
//! [`NO_VALUE_COLOR`]. A scale of stops (since 0.31.0) is a map of 256 colours interpolated
//! between them as matplotlib's `LinearSegmentedColormap.from_list` ([`check_stops`]), read the
//! same way. The tables are constant data and the logarithm is `libm`'s, so the native
//! and the wasm build give the same colours.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::spec::{ColorScale, ColorStop};

include!("colors_table.rs");

impl ColorScaleName {
    /// The scale of a name of the spec, `None` for a name which is none.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The colour `0xrrggbb` at the entry `index` (0 to 255) of the scale.
    fn entry(self, index: usize) -> u32 {
        let (table, reversed) = self.table();
        table[if reversed { 255 - index } else { index }]
    }
}

/// The message of a name which is no colour scale, with the name in lower case when that is one
/// (matplotlib writes `Blues`, the spec `blues`).
pub(crate) fn unknown_scale(name: &str) -> String {
    let lower = name.to_lowercase();
    let hint = if ColorScaleName::from_name(&lower).is_some() {
        format!(", did you mean `{lower}`?")
    } else {
        String::new()
    };
    format!(
        "unknown color scale `{name}`, expected a sequential or diverging colour map of \
         matplotlib in lower case, or its reversal `<name>_r`, e.g. viridis, blues, rdbu or \
         purd_r{hint}"
    )
}

impl fmt::Display for ColorScaleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ColorScaleName {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::from_name(name).ok_or_else(|| unknown_scale(name))
    }
}

impl Serialize for ColorScaleName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for ColorScaleName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        name.parse().map_err(serde::de::Error::custom)
    }
}

/// The colour of a value which has none: NaN, or a value which is not positive on a log scale
/// (matplotlib's `lightgrey`, the colour of an undefined cell of the heatmaps of sbmlsim).
pub const NO_VALUE_COLOR: &str = "#d3d3d3";

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

/// Whether a text is a colour `#rrggbb`.
pub(crate) fn is_hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit())
}

/// The channels of a colour `#rrggbb` as shares of 255.
fn channels(c: &str) -> [f64; 3] {
    [1, 3, 5].map(|k| {
        c.get(k..k + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, |b| f64::from(b) / 255.0)
    })
}

/// Checks the stops of a colour map (since 0.31.0): at least two, the first at 0 and the last
/// at 1, the positions finite and not decreasing, the colours `#rrggbb`. The error names the
/// stop it is about, `None` for the stops as a whole.
pub fn check_stops(stops: &[ColorStop]) -> Result<(), (Option<usize>, String)> {
    if stops.len() < 2 {
        return Err((
            None,
            format!("{} stops, a colour map needs at least 2", stops.len()),
        ));
    }
    for (k, ColorStop(at, color)) in stops.iter().enumerate() {
        if !(at.is_finite() && (0.0..=1.0).contains(at)) {
            return Err((Some(k), format!("position {at} is not between 0 and 1")));
        }
        if k > 0 && *at < stops[k - 1].0 {
            return Err((
                Some(k),
                format!(
                    "position {at} is below the one before it, the positions must not decrease"
                ),
            ));
        }
        if !is_hex_color(color) {
            return Err((
                Some(k),
                format!("color `{color}` is not of the form #rrggbb"),
            ));
        }
    }
    if stops[0].0 != 0.0 {
        return Err((Some(0), "the first stop must be at 0".into()));
    }
    if stops[stops.len() - 1].0 != 1.0 {
        return Err((Some(stops.len() - 1), "the last stop must be at 1".into()));
    }
    Ok(())
}

/// The entry `index` (0 to 255) of the map of stops, as matplotlib's `_create_lookup_table`
/// computes the table of `LinearSegmentedColormap.from_list`: the colour at `index / 255`
/// interpolated linearly between the stops around it (the first stop at or after it, and the
/// one before), the first and the last entry the colours of the first and the last stop; each
/// channel written as matplotlib's `to_hex` writes it, `round(255 c)` with ties to even.
fn stop_entry(stops: &[ColorStop], index: usize) -> u32 {
    let last = stops.len() - 1;
    let rgb = if index == 0 {
        channels(&stops[0].1)
    } else if index >= 255 {
        channels(&stops[last].1)
    } else {
        let x = index as f64 / 255.0;
        // numpy's searchsorted on the left: the first stop at or after x, at least the second
        let k = stops.iter().position(|s| s.0 >= x).unwrap_or(last).max(1);
        let (a, b) = (&stops[k - 1], &stops[k]);
        let (ca, cb) = (channels(&a.1), channels(&b.1));
        let span = b.0 - a.0;
        let d = if span > 0.0 { (x - a.0) / span } else { 1.0 };
        [0, 1, 2].map(|c| (d * (cb[c] - ca[c]) + ca[c]).clamp(0.0, 1.0))
    };
    let byte = |v: f64| (v * 255.0).round_ties_even() as u32;
    (byte(rgb[0]) << 16) | (byte(rgb[1]) << 8) | byte(rgb[2])
}

/// The colour `#rrggbb` of a share of the map of a scale, [`NO_VALUE_COLOR`] for NaN.
pub(crate) fn color_at(scale: &ColorScale, t: f64) -> String {
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
    let entry = match (&scale.stops, scale.name) {
        (Some(stops), _) if stops.len() >= 2 => stop_entry(stops, index),
        (_, Some(name)) => name.entry(index),
        // a scale of neither, which validation refuses, is grey
        _ => 0x808080,
    };
    format!("#{entry:06x}")
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
    color_at(scale, on_table(scale, share(scale, v)))
}

/// The colours `#rrggbb` of values on a colour scale, one per value.
///
/// The map is the named one or the one of the stops of the scale (since 0.31.0).
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
        assert_eq!(COLOR_SCALES.len(), ColorScaleName::ALL.len());
        for (name, text) in ColorScaleName::ALL.into_iter().zip(COLOR_SCALES) {
            assert_eq!(name.name(), *text);
            assert_eq!(ColorScaleName::from_name(text), Some(name));
            assert_eq!(name.table().0.len(), 256);
        }
        // the names of 0.29 keep their scales
        for text in ["viridis", "magma", "rdbu", "coolwarm", "greys"] {
            assert!(COLOR_SCALES.contains(&text), "{text}");
        }
    }

    #[test]
    fn a_reversed_scale_reads_its_map_from_the_high_end() {
        let (blues, blues_r) = (ColorScaleName::Blues, ColorScaleName::BluesR);
        assert_eq!(blues.entry(0), blues_r.entry(255));
        assert_eq!(blues.entry(255), blues_r.entry(0));
        // matplotlib's `Blues` from white to dark blue
        assert_eq!(blues.entry(0), 0xf7fbff);
        assert_eq!(blues.entry(255), 0x08306b);
    }

    #[test]
    fn names_in_the_case_of_matplotlib_are_refused_with_a_hint() {
        let e = "Blues".parse::<ColorScaleName>().unwrap_err();
        assert!(e.contains("did you mean `blues`"), "{e}");
        let e = "jet".parse::<ColorScaleName>().unwrap_err();
        assert!(!e.contains("did you mean"), "{e}");
    }

    #[test]
    fn text_contrast() {
        assert!(dark_text_on("#fde725"));
        assert!(!dark_text_on("#440154"));
        assert!(dark_text_on("#ffffff"));
    }
}

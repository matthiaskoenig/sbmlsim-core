//! The types of the figure spec `figure/1` and the validation of what the types cannot say.
//!
//! Every object refuses unknown fields and uses snake case. A field with a default may be absent.

// a negated comparison also refuses NaN, which is intended
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::collections::HashSet;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::FIGURE_CONTRACT;
use crate::error::{SUB_PATH, SpecError};

fn yes() -> bool {
    true
}
fn one() -> u32 {
    1
}
fn one_f() -> f64 {
    1.0
}
fn space() -> f64 {
    0.3
}
fn two() -> f64 {
    2.0
}
fn six() -> f64 {
    6.0
}

/// A figure: a grid of plots with a title, fonts and legend settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Figure {
    /// The contract id, must be [`FIGURE_CONTRACT`].
    pub contract: String,
    /// Width of the figure in pixels.
    pub width: f64,
    /// Height of the figure in pixels.
    pub height: f64,
    /// Title above all plots.
    #[serde(default)]
    pub title: Option<String>,
    /// Font sizes.
    #[serde(default)]
    pub fonts: Fonts,
    /// The grid the plots are placed in.
    pub grid: Grid,
    /// Legend settings of the figure.
    #[serde(default)]
    pub legend: LegendSettings,
    /// The plots.
    pub plots: Vec<Plot>,
}

/// Font sizes in points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    /// Title font size, 16 by default.
    #[serde(default = "d_title")]
    pub title: f64,
    /// Axis label font size, 13 by default.
    #[serde(default = "d_label")]
    pub label: f64,
    /// Tick label font size, 11 by default.
    #[serde(default = "d_tick")]
    pub tick: f64,
    /// Legend font size, 11 by default.
    #[serde(default = "d_tick")]
    pub legend: f64,
}
fn d_title() -> f64 {
    16.0
}
fn d_label() -> f64 {
    13.0
}
fn d_tick() -> f64 {
    11.0
}
impl Default for Fonts {
    fn default() -> Self {
        Fonts {
            title: 16.0,
            label: 13.0,
            tick: 11.0,
            legend: 11.0,
        }
    }
}

/// The grid of rows and columns the plots are placed in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    /// Number of rows.
    pub rows: u32,
    /// Number of columns.
    pub columns: u32,
    /// Horizontal space between plots as a fraction of a plot, 0.3 by default.
    #[serde(default = "space")]
    pub wspace: f64,
    /// Vertical space between plots as a fraction of a plot, 0.3 by default.
    #[serde(default = "space")]
    pub hspace: f64,
}

/// Which plots share a legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendScope {
    /// One legend per plot.
    #[default]
    Plot,
    /// One legend per row of the grid.
    Row,
}

/// Where a legend is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendPosition {
    /// Inside the plot.
    #[default]
    Inside,
    /// Right of the plot.
    Outside,
}

/// Legend settings of a figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegendSettings {
    /// Whether a legend is per plot or per row.
    #[serde(default)]
    pub scope: LegendScope,
    /// Where the legend is placed.
    #[serde(default)]
    pub position: LegendPosition,
}

/// One plot of the grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plot {
    /// Id of the plot.
    pub id: String,
    /// Row of its top left cell, from 0.
    pub row: u32,
    /// Column of its top left cell, from 0.
    pub column: u32,
    /// Rows it spans, 1 by default.
    #[serde(default = "one")]
    pub row_span: u32,
    /// Columns it spans, 1 by default.
    #[serde(default = "one")]
    pub column_span: u32,
    /// Title of the plot.
    #[serde(default)]
    pub title: Option<String>,
    /// The x axis.
    pub x: Axis,
    /// The left y axis.
    pub y: Axis,
    /// The optional right y axis.
    #[serde(default)]
    pub y2: Option<Axis>,
    /// Whether the plot has a legend, true by default.
    #[serde(default = "yes")]
    pub legend: bool,
    /// A color bar.
    #[serde(default)]
    pub colorbar: Option<ColorBar>,
    /// Legends declared as lists of legend groups; absent: derived from the elements.
    #[serde(default)]
    pub declared_legends: Option<Vec<Vec<String>>>,
    /// The elements drawn in the plot.
    pub elements: Vec<Element>,
}

/// The scale of an axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisScale {
    /// Linear.
    #[default]
    Linear,
    /// Logarithmic.
    Log,
    /// Categories at the positions 0, 1, ...
    Category,
}

/// An interval of values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    /// Lower end.
    pub low: f64,
    /// Upper end.
    pub high: f64,
}

/// An axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axis {
    /// Label of the axis.
    #[serde(default)]
    pub label: Option<String>,
    /// Unit shown with the label.
    #[serde(default)]
    pub unit: Option<String>,
    /// Scale, linear by default.
    #[serde(default)]
    pub scale: AxisScale,
    /// Names of the categories of a category axis.
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    /// Exact lower end.
    #[serde(default)]
    pub min: Option<f64>,
    /// Exact upper end.
    #[serde(default)]
    pub max: Option<f64>,
    /// Values the axis must hold, margins applied.
    #[serde(default)]
    pub frame: Option<Range>,
    /// The decades shown below the largest value of a log axis (6 when absent).
    #[serde(default)]
    pub max_decades: Option<f64>,
    /// Reversed direction.
    #[serde(default)]
    pub reverse: bool,
    /// Grid lines.
    #[serde(default)]
    pub grid: bool,
    /// Whether the label is shown, true by default.
    #[serde(default = "yes")]
    pub label_visible: bool,
    /// Whether the ticks are shown, true by default.
    #[serde(default = "yes")]
    pub ticks_visible: bool,
    /// Rotation of the tick labels in degrees.
    #[serde(default)]
    pub tick_rotation: f64,
}

/// The side of the y axis an element belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YAxisRef {
    /// The left axis.
    #[default]
    Y,
    /// The right axis.
    Y2,
}

/// An element of a plot: the common fields and the data of its kind.
///
/// In JSON it is one flat object with `id`, `label`, `legend`, `axis`, `style`, `kind` and the
/// fields of its kind; unknown fields are refused.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Id, unique within the plot.
    pub id: String,
    /// Label in the legend.
    pub label: Option<String>,
    /// Legend group; absent: the id when a label is set.
    pub legend: Option<String>,
    /// The y axis the element is drawn against.
    pub axis: YAxisRef,
    /// Style.
    pub style: Style,
    /// What it draws.
    pub kind: ElementKind,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Common {
    id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    legend: Option<String>,
    #[serde(default)]
    axis: YAxisRef,
    #[serde(default)]
    style: Style,
}

const COMMON_KEYS: [&str; 5] = ["id", "label", "legend", "axis", "style"];

fn in_element<E: serde::de::Error>(path: &serde_path_to_error::Path, message: String) -> E {
    E::custom(format!("{SUB_PATH}{path}{SUB_PATH}{message}"))
}

impl<'de> Deserialize<'de> for Element {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let serde_json::Value::Object(mut map) = value else {
            return Err(D::Error::custom("expected an object"));
        };
        let mut common = serde_json::Map::new();
        for key in COMMON_KEYS {
            if let Some(v) = map.remove(key) {
                common.insert(key.to_string(), v);
            }
        }
        let c: Common = serde_path_to_error::deserialize(serde_json::Value::Object(common))
            .map_err(|e| in_element(e.path(), e.inner().to_string()))?;
        let given: Vec<String> = map.keys().cloned().collect();
        let kind: ElementKind = serde_path_to_error::deserialize(serde_json::Value::Object(map))
            .map_err(|e| in_element(e.path(), e.inner().to_string()))?;
        // serde cannot refuse unknown fields of the variants of a tagged enum: a field which
        // the kind does not write back is none of its fields
        if let Ok(serde_json::Value::Object(known)) = serde_json::to_value(&kind)
            && let Some(unknown) = given.iter().find(|k| !known.contains_key(*k))
        {
            return Err(D::Error::custom(format!("unknown field `{unknown}`")));
        }
        Ok(Element {
            id: c.id,
            label: c.label,
            legend: c.legend,
            axis: c.axis,
            style: c.style,
            kind,
        })
    }
}

impl Serialize for Element {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let common = Common {
            id: self.id.clone(),
            label: self.label.clone(),
            legend: self.legend.clone(),
            axis: self.axis,
            style: self.style.clone(),
        };
        let mut map = match serde_json::to_value(common).map_err(S::Error::custom)? {
            serde_json::Value::Object(m) => m,
            _ => return Err(S::Error::custom("not an object")),
        };
        match serde_json::to_value(&self.kind).map_err(S::Error::custom)? {
            serde_json::Value::Object(m) => map.extend(m),
            _ => return Err(S::Error::custom("not an object")),
        }
        serde_json::Value::Object(map).serialize(s)
    }
}

/// The data of an element, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementKind {
    /// A line and/or markers, with optional error bars.
    Curve {
        /// x values.
        x: Vec<f64>,
        /// y values.
        y: Vec<f64>,
        /// Error of x.
        #[serde(default)]
        xerr: Option<Vec<f64>>,
        /// Error of y.
        #[serde(default)]
        yerr: Option<Vec<f64>>,
    },
    /// A band between two curves with an optional median.
    Band {
        /// x values.
        x: Vec<f64>,
        /// Lower edge.
        low: Vec<f64>,
        /// Upper edge.
        high: Vec<f64>,
        /// Median line.
        #[serde(default)]
        median: Option<Vec<f64>>,
    },
    /// A filled area between two curves.
    Area {
        /// x values.
        x: Vec<f64>,
        /// Start of the area.
        from: Vec<f64>,
        /// End of the area.
        to: Vec<f64>,
    },
    /// Bars.
    Bar {
        /// Positions, or category indices.
        x: Vec<f64>,
        /// Heights.
        height: Vec<f64>,
        /// Errors.
        #[serde(default)]
        err: Option<Vec<f64>>,
        /// Horizontal bars.
        #[serde(default)]
        horizontal: bool,
        /// Width of a bar in axis units.
        #[serde(default)]
        width: Option<f64>,
    },
    /// Box plots.
    Box {
        /// Positions.
        positions: Vec<f64>,
        /// Statistics per position.
        stats: Vec<BoxStats>,
    },
    /// Violins.
    Violin {
        /// Positions.
        positions: Vec<f64>,
        /// Density per position.
        densities: Vec<Density>,
    },
    /// A histogram.
    Histogram {
        /// Bin edges, one more than counts.
        edges: Vec<f64>,
        /// Counts per bin.
        counts: Vec<f64>,
    },
    /// A heat map.
    Heatmap {
        /// Row names.
        rows: Vec<String>,
        /// Column names.
        columns: Vec<String>,
        /// Values, per row.
        values: Vec<Vec<f64>>,
        /// Text of the cells.
        #[serde(default)]
        text: Option<Vec<Vec<String>>>,
        /// Color scale.
        scale: ColorScale,
    },
    /// A text.
    Text {
        /// x position.
        x: f64,
        /// y position.
        y: f64,
        /// The text.
        text: String,
        /// Anchor, start by default.
        #[serde(default)]
        anchor: Anchor,
    },
    /// A horizontal or vertical line.
    Rule {
        /// Direction.
        orientation: Orientation,
        /// Position on the other axis.
        value: f64,
    },
    /// Filled polygons.
    Polygon {
        /// The outlines.
        paths: Vec<Vec<[f64; 2]>>,
    },
}

/// Text anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Text starts at the position.
    #[default]
    Start,
    /// Text is centered on the position.
    Middle,
    /// Text ends at the position.
    End,
}

/// Direction of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// A horizontal line.
    Horizontal,
    /// A vertical line.
    Vertical,
}

/// Statistics of a box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxStats {
    /// Lower whisker.
    pub low_whisker: f64,
    /// First quartile.
    pub q1: f64,
    /// Median.
    pub median: f64,
    /// Third quartile.
    pub q3: f64,
    /// Upper whisker.
    pub high_whisker: f64,
    /// Outliers.
    #[serde(default)]
    pub outliers: Vec<f64>,
}

/// A density curve of a violin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Density {
    /// Values where the density is given.
    pub values: Vec<f64>,
    /// Density at the values.
    pub density: Vec<f64>,
}

/// A color scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorScale {
    /// Name of the color map, e.g. `viridis` or `rdbu`.
    pub name: String,
    /// Value of the low end.
    pub low: f64,
    /// Value of the high end.
    pub high: f64,
    /// Value at the middle of a diverging map.
    #[serde(default)]
    pub center: Option<f64>,
    /// Logarithmic scale.
    #[serde(default)]
    pub log: bool,
}

/// A color bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorBar {
    /// The scale shown.
    pub scale: ColorScale,
    /// Label.
    #[serde(default)]
    pub label: Option<String>,
}

/// Line type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineType {
    /// Solid.
    #[default]
    Solid,
    /// Dashed.
    Dashed,
    /// Dotted.
    Dotted,
    /// Dash and dot.
    Dashdot,
    /// No line.
    None,
}

/// A line style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    /// Type, solid by default.
    #[serde(default, rename = "type")]
    pub kind: LineType,
    /// Width in pixels, 2 by default.
    #[serde(default = "two")]
    pub width: f64,
}
impl Default for Line {
    fn default() -> Self {
        Line {
            kind: LineType::Solid,
            width: 2.0,
        }
    }
}

/// Marker type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerType {
    /// No marker.
    #[default]
    None,
    /// Circle.
    Circle,
    /// Square.
    Square,
    /// Triangle.
    Triangle,
    /// Diamond.
    Diamond,
    /// Cross.
    Cross,
    /// Plus.
    Plus,
    /// X.
    X,
}

/// A marker style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    /// Type, none by default.
    #[serde(default, rename = "type")]
    pub kind: MarkerType,
    /// Size in pixels, 6 by default.
    #[serde(default = "six")]
    pub size: f64,
    /// Fill color.
    #[serde(default)]
    pub fill: Option<String>,
    /// Edge color.
    #[serde(default)]
    pub edge: Option<String>,
    /// Edge width, 1 by default.
    #[serde(default = "one_f")]
    pub edge_width: f64,
}
impl Default for Marker {
    fn default() -> Self {
        Marker {
            kind: MarkerType::None,
            size: 6.0,
            fill: None,
            edge: None,
            edge_width: 1.0,
        }
    }
}

/// A fill style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fill {
    /// Color `#rrggbb`.
    #[serde(default)]
    pub color: Option<String>,
    /// Opacity, 1 by default.
    #[serde(default = "one_f")]
    pub opacity: f64,
}

/// The style of an element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    /// Color `#rrggbb`; absent: the cycle by element position.
    #[serde(default)]
    pub color: Option<String>,
    /// Opacity, 1 by default.
    #[serde(default = "one_f")]
    pub opacity: f64,
    /// Line.
    #[serde(default)]
    pub line: Line,
    /// Marker.
    #[serde(default)]
    pub marker: Marker,
    /// Fill.
    #[serde(default)]
    pub fill: Option<Fill>,
}
impl Default for Style {
    fn default() -> Self {
        Style {
            color: None,
            opacity: 1.0,
            line: Line::default(),
            marker: Marker::default(),
            fill: None,
        }
    }
}

fn err<T>(path: String, message: impl Into<String>) -> Result<T, SpecError> {
    Err(SpecError::new(path, message))
}

fn same_len(path: &str, name: &str, n: usize, reference: &str, m: usize) -> Result<(), SpecError> {
    if n != m {
        return err(
            format!("{path}.{name}"),
            format!("{n} values, {reference} has {m}"),
        );
    }
    Ok(())
}

fn check_color(path: String, color: &Option<String>) -> Result<(), SpecError> {
    if let Some(c) = color {
        let ok =
            c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
        if !ok {
            return err(path, format!("color `{c}` is not of the form #rrggbb"));
        }
    }
    Ok(())
}

fn check_scale(path: &str, s: &ColorScale) -> Result<(), SpecError> {
    if !(s.low < s.high) {
        return err(format!("{path}.high"), "must be above low");
    }
    if s.log && s.low <= 0.0 {
        return err(format!("{path}.low"), "must be positive on a log scale");
    }
    Ok(())
}

fn check_axis(path: &str, a: &Axis) -> Result<(), SpecError> {
    if a.scale != AxisScale::Category && a.categories.is_some() {
        return err(
            format!("{path}.categories"),
            "only a category axis has categories",
        );
    }
    if a.scale == AxisScale::Category && a.categories.is_none() {
        return err(
            format!("{path}.categories"),
            "a category axis needs categories",
        );
    }
    if let (Some(lo), Some(hi)) = (a.min, a.max)
        && !(lo < hi)
    {
        return err(format!("{path}.max"), "must be above min");
    }
    if a.scale == AxisScale::Log {
        for (name, v) in [("min", a.min), ("max", a.max)] {
            if v.is_some_and(|v| v <= 0.0) {
                return err(format!("{path}.{name}"), "must be positive on a log axis");
            }
        }
        if let Some(r) = a.frame
            && r.low <= 0.0
        {
            return err(
                format!("{path}.frame.low"),
                "must be positive on a log axis",
            );
        }
    }
    if let Some(r) = a.frame
        && !(r.low <= r.high)
    {
        return err(format!("{path}.frame.high"), "must not be below low");
    }
    if a.max_decades.is_some_and(|d| !(d > 0.0)) {
        return err(format!("{path}.max_decades"), "must be positive");
    }
    Ok(())
}

fn check_kind(p: &str, kind: &ElementKind) -> Result<(), SpecError> {
    use ElementKind::*;
    match kind {
        Curve { x, y, xerr, yerr } => {
            same_len(p, "y", y.len(), "x", x.len())?;
            if let Some(e) = xerr {
                same_len(p, "xerr", e.len(), "x", x.len())?;
            }
            if let Some(e) = yerr {
                same_len(p, "yerr", e.len(), "x", x.len())?;
            }
        }
        Band {
            x,
            low,
            high,
            median,
        } => {
            same_len(p, "low", low.len(), "x", x.len())?;
            same_len(p, "high", high.len(), "x", x.len())?;
            if let Some(m) = median {
                same_len(p, "median", m.len(), "x", x.len())?;
            }
        }
        Area { x, from, to } => {
            same_len(p, "from", from.len(), "x", x.len())?;
            same_len(p, "to", to.len(), "x", x.len())?;
        }
        Bar {
            x,
            height,
            err: e,
            width,
            ..
        } => {
            same_len(p, "height", height.len(), "x", x.len())?;
            if let Some(e) = e {
                same_len(p, "err", e.len(), "x", x.len())?;
            }
            if width.is_some_and(|w| !(w > 0.0)) {
                return err(format!("{p}.width"), "must be positive");
            }
        }
        Box { positions, stats } => {
            same_len(p, "stats", stats.len(), "positions", positions.len())?;
        }
        Violin {
            positions,
            densities,
        } => {
            same_len(
                p,
                "densities",
                densities.len(),
                "positions",
                positions.len(),
            )?;
            for (i, d) in densities.iter().enumerate() {
                same_len(
                    &format!("{p}.densities[{i}]"),
                    "density",
                    d.density.len(),
                    "values",
                    d.values.len(),
                )?;
            }
        }
        Histogram { edges, counts } => {
            if edges.len() != counts.len() + 1 {
                return err(
                    format!("{p}.edges"),
                    format!(
                        "{} values, counts has {} so edges needs {}",
                        edges.len(),
                        counts.len(),
                        counts.len() + 1
                    ),
                );
            }
        }
        Heatmap {
            rows,
            columns,
            values,
            text,
            scale,
        } => {
            same_len(p, "values", values.len(), "rows", rows.len())?;
            for (i, r) in values.iter().enumerate() {
                same_len(
                    &format!("{p}.values[{i}]"),
                    "",
                    r.len(),
                    "columns",
                    columns.len(),
                )
                .map_err(|e| SpecError::new(format!("{p}.values[{i}]"), e.message))?;
            }
            if let Some(t) = text {
                same_len(p, "text", t.len(), "rows", rows.len())?;
                for (i, r) in t.iter().enumerate() {
                    if r.len() != columns.len() {
                        return err(
                            format!("{p}.text[{i}]"),
                            format!("{} values, columns has {}", r.len(), columns.len()),
                        );
                    }
                }
            }
            check_scale(&format!("{p}.scale"), scale)?;
        }
        Text { .. } | Rule { .. } | Polygon { .. } => {}
    }
    Ok(())
}

/// Checks the rules of a figure which the types cannot say.
pub(crate) fn validate(f: &Figure) -> Result<(), SpecError> {
    if f.contract != FIGURE_CONTRACT {
        return err(
            "contract".into(),
            format!("expected `{FIGURE_CONTRACT}`, got `{}`", f.contract),
        );
    }
    if !(f.width > 0.0) {
        return err("width".into(), "must be positive");
    }
    if !(f.height > 0.0) {
        return err("height".into(), "must be positive");
    }
    if f.grid.rows == 0 {
        return err("grid.rows".into(), "must be at least 1");
    }
    if f.grid.columns == 0 {
        return err("grid.columns".into(), "must be at least 1");
    }
    if f.grid.wspace < 0.0 {
        return err("grid.wspace".into(), "must not be negative");
    }
    if f.grid.hspace < 0.0 {
        return err("grid.hspace".into(), "must not be negative");
    }
    let mut plot_ids = HashSet::new();
    for (i, p) in f.plots.iter().enumerate() {
        let pp = format!("plots[{i}]");
        if !plot_ids.insert(p.id.as_str()) {
            return err(format!("{pp}.id"), format!("duplicate plot id `{}`", p.id));
        }
        if p.row >= f.grid.rows {
            return err(
                format!("{pp}.row"),
                format!("row {} is outside the grid of {} rows", p.row, f.grid.rows),
            );
        }
        if p.column >= f.grid.columns {
            return err(
                format!("{pp}.column"),
                format!(
                    "column {} is outside the grid of {} columns",
                    p.column, f.grid.columns
                ),
            );
        }
        if p.row_span == 0 || p.row + p.row_span > f.grid.rows {
            return err(
                format!("{pp}.row_span"),
                format!(
                    "span {} from row {} leaves the grid of {} rows",
                    p.row_span, p.row, f.grid.rows
                ),
            );
        }
        if p.column_span == 0 || p.column + p.column_span > f.grid.columns {
            return err(
                format!("{pp}.column_span"),
                format!(
                    "span {} from column {} leaves the grid of {} columns",
                    p.column_span, p.column, f.grid.columns
                ),
            );
        }
        check_axis(&format!("{pp}.x"), &p.x)?;
        check_axis(&format!("{pp}.y"), &p.y)?;
        if let Some(a) = &p.y2 {
            check_axis(&format!("{pp}.y2"), a)?;
        }
        if let Some(c) = &p.colorbar {
            check_scale(&format!("{pp}.colorbar.scale"), &c.scale)?;
        }
        let mut ids = HashSet::new();
        for (j, e) in p.elements.iter().enumerate() {
            let ep = format!("{pp}.elements[{j}]");
            if !ids.insert(e.id.as_str()) {
                return err(
                    format!("{ep}.id"),
                    format!("duplicate element id `{}`", e.id),
                );
            }
            if e.axis == YAxisRef::Y2 && p.y2.is_none() {
                return err(
                    format!("{ep}.axis"),
                    "the element uses y2 but the plot has no y2 axis",
                );
            }
            if !(0.0..=1.0).contains(&e.style.opacity) {
                return err(format!("{ep}.style.opacity"), "must be between 0 and 1");
            }
            if e.style.line.width < 0.0 {
                return err(format!("{ep}.style.line.width"), "must not be negative");
            }
            if e.style.marker.size < 0.0 {
                return err(format!("{ep}.style.marker.size"), "must not be negative");
            }
            check_color(format!("{ep}.style.color"), &e.style.color)?;
            check_color(format!("{ep}.style.marker.fill"), &e.style.marker.fill)?;
            check_color(format!("{ep}.style.marker.edge"), &e.style.marker.edge)?;
            if let Some(fill) = &e.style.fill {
                check_color(format!("{ep}.style.fill.color"), &fill.color)?;
                if !(0.0..=1.0).contains(&fill.opacity) {
                    return err(
                        format!("{ep}.style.fill.opacity"),
                        "must be between 0 and 1",
                    );
                }
            }
            check_kind(&ep, &e.kind)?;
        }
    }
    Ok(())
}

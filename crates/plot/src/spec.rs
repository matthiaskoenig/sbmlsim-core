//! The types of the figure spec `figure/1` and the validation of what the types cannot say.
//!
//! Every object refuses unknown fields and uses snake case. A field with a default may be absent.
//!
//! In every numeric data array JSON `null` is a gap, a value which is not there, and reads as
//! `f64::NAN`; a NaN is written as `null`.

// a negated comparison also refuses NaN, which is intended
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::collections::HashSet;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::FIGURE_CONTRACT;
// SUB_PATH is the U+0001 marker of the path inside an element, see `in_element`
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The x axis.
    pub x: Axis,
    /// The left y axis.
    pub y: Axis,
    /// The optional right y axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<Axis>,
    /// Whether the plot has a legend, true by default.
    #[serde(default = "yes")]
    pub legend: bool,
    /// A color bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbar: Option<ColorBar>,
    /// The legends the plot may show in some state of a page, each a list of entry labels
    /// (`declaredLegends` of sbmlsim-web): the legend takes the room of the largest of them, so
    /// the plot and the figure keep their size when entries come and go; a label without an
    /// element is not drawn. Absent: the room of the entries drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_legends: Option<Vec<Vec<String>>>,
    /// The order of the legend entries, as legend groups: the listed groups first in this
    /// order, the others after them in the order their elements first appear (the default
    /// order without it). A listed group without elements is ignored. It lets a writer draw
    /// elements under others (frozen runs under the live curves) and still list them last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_order: Option<Vec<String>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Unit shown with the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Scale, linear by default.
    #[serde(default)]
    pub scale: AxisScale,
    /// Names of the categories of a category axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    /// Exact lower end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Exact upper end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Values the axis must hold, margins applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Range>,
    /// The decades shown below the largest value of a log axis (6 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Id, unique within the figure.
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

impl Element {
    /// The legend group of the element: its `legend`, else its id when it has a label, else none.
    pub fn legend_group(&self) -> Option<&str> {
        match (&self.legend, &self.label) {
            (Some(g), _) => Some(g),
            (None, Some(_)) => Some(&self.id),
            (None, None) => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Common {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legend: Option<String>,
    #[serde(default)]
    axis: YAxisRef,
    #[serde(default)]
    style: Style,
}

const COMMON_KEYS: [&str; 5] = ["id", "label", "legend", "axis", "style"];

/// The fields of the kinds of an element besides the common ones, `None` for an unknown kind.
fn kind_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "curve" => &["x", "y", "xerr", "yerr"],
        "band" => &["x", "low", "high", "median"],
        "area" => &["x", "from", "to"],
        "bar" => &["x", "height", "err", "horizontal", "width"],
        "box" => &["positions", "stats"],
        "violin" => &["positions", "densities"],
        "histogram" => &["edges", "counts"],
        "heatmap" => &["rows", "columns", "values", "text", "scale"],
        "text" => &["x", "y", "text", "anchor"],
        "rule" => &["orientation", "value"],
        "polygon" => &["paths"],
        _ => return None,
    })
}

/// Serde helpers of the numeric data arrays: JSON `null` is a gap and reads as NaN, and a NaN
/// is written as `null`.
mod nums {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    fn gap(v: Option<f64>) -> f64 {
        v.unwrap_or(f64::NAN)
    }
    fn value(v: &f64) -> Option<f64> {
        if v.is_nan() { None } else { Some(*v) }
    }

    pub mod vec {
        use super::*;
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
            Ok(Vec::<Option<f64>>::deserialize(d)?
                .into_iter()
                .map(gap)
                .collect())
        }
        pub fn serialize<S: Serializer>(v: &[f64], s: S) -> Result<S::Ok, S::Error> {
            v.iter().map(value).collect::<Vec<_>>().serialize(s)
        }
    }

    pub mod opt_vec {
        use super::*;
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<f64>>, D::Error> {
            Ok(Option::<Vec<Option<f64>>>::deserialize(d)?
                .map(|v| v.into_iter().map(gap).collect()))
        }
        pub fn serialize<S: Serializer>(v: &Option<Vec<f64>>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(v) => super::vec::serialize(v, s),
                None => s.serialize_none(),
            }
        }
    }

    pub mod rows {
        use super::*;
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<f64>>, D::Error> {
            Ok(Vec::<Vec<Option<f64>>>::deserialize(d)?
                .into_iter()
                .map(|r| r.into_iter().map(gap).collect())
                .collect())
        }
        pub fn serialize<S: Serializer>(v: &[Vec<f64>], s: S) -> Result<S::Ok, S::Error> {
            v.iter()
                .map(|r| r.iter().map(value).collect::<Vec<_>>())
                .collect::<Vec<_>>()
                .serialize(s)
        }
    }
}

// An error inside an element carries the path inside it between two U+0001 characters in front
// of the message, which `SpecError::from_serde` (error.rs) takes apart and joins to the path of
// the element, since serde knows the path to the element only.
fn in_element<E: serde::de::Error>(path: &dyn std::fmt::Display, message: String) -> E {
    E::custom(format!("{SUB_PATH}{path}{SUB_PATH}{message}"))
}

impl<'de> Deserialize<'de> for Element {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let serde_json::Value::Object(mut map) = value else {
            return Err(D::Error::custom("expected an object"));
        };
        // serde cannot refuse unknown fields of the variants of a tagged enum, so the fields
        // are checked against the list of the kind before the typed parse
        if let Some(kind) = map.get("kind").and_then(|k| k.as_str())
            && let Some(fields) = kind_fields(kind)
            && let Some(unknown) = map.keys().find(|k| {
                *k != "kind" && !COMMON_KEYS.contains(&k.as_str()) && !fields.contains(&k.as_str())
            })
        {
            return Err(D::Error::custom(format!("unknown field `{unknown}`")));
        }
        // serde loses the path inside a tagged enum, so the name of the scale of a heatmap, the
        // one field of a kind which can be refused by its value alone, is checked here
        if let Some(n) = map
            .get("scale")
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
            && serde_json::from_value::<ColorScaleName>(n.into()).is_err()
        {
            return Err(in_element(
                &"scale.name",
                format!(
                    "unknown color scale `{n}`, expected one of viridis, magma, rdbu, coolwarm, greys"
                ),
            ));
        }
        let mut common = serde_json::Map::new();
        for key in COMMON_KEYS {
            if let Some(v) = map.remove(key) {
                common.insert(key.to_string(), v);
            }
        }
        let c: Common = serde_path_to_error::deserialize(serde_json::Value::Object(common))
            .map_err(|e| in_element(e.path(), e.inner().to_string()))?;
        let kind: ElementKind = serde_path_to_error::deserialize(serde_json::Value::Object(map))
            .map_err(|e| in_element(e.path(), e.inner().to_string()))?;
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
        #[serde(with = "nums::vec")]
        x: Vec<f64>,
        /// y values.
        #[serde(with = "nums::vec")]
        y: Vec<f64>,
        /// Error of x.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        xerr: Option<Vec<f64>>,
        /// Error of y.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        yerr: Option<Vec<f64>>,
    },
    /// A band between two curves with an optional median.
    Band {
        /// x values.
        #[serde(with = "nums::vec")]
        x: Vec<f64>,
        /// Lower edge.
        #[serde(with = "nums::vec")]
        low: Vec<f64>,
        /// Upper edge.
        #[serde(with = "nums::vec")]
        high: Vec<f64>,
        /// Median line.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        median: Option<Vec<f64>>,
    },
    /// A filled area between two curves.
    Area {
        /// x values.
        #[serde(with = "nums::vec")]
        x: Vec<f64>,
        /// Start of the area.
        #[serde(with = "nums::vec")]
        from: Vec<f64>,
        /// End of the area.
        #[serde(with = "nums::vec")]
        to: Vec<f64>,
    },
    /// Bars.
    Bar {
        /// Positions, or category indices.
        #[serde(with = "nums::vec")]
        x: Vec<f64>,
        /// Heights.
        #[serde(with = "nums::vec")]
        height: Vec<f64>,
        /// Errors.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        err: Option<Vec<f64>>,
        /// Horizontal bars.
        #[serde(default)]
        horizontal: bool,
        /// Width of a bar in axis units.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
    },
    /// Box plots.
    Box {
        /// Positions.
        #[serde(with = "nums::vec")]
        positions: Vec<f64>,
        /// Statistics per position.
        stats: Vec<BoxStats>,
    },
    /// Violins.
    Violin {
        /// Positions.
        #[serde(with = "nums::vec")]
        positions: Vec<f64>,
        /// Density per position.
        densities: Vec<Density>,
    },
    /// A histogram.
    Histogram {
        /// Bin edges, one more than counts.
        #[serde(with = "nums::vec")]
        edges: Vec<f64>,
        /// Counts per bin.
        #[serde(with = "nums::vec")]
        counts: Vec<f64>,
    },
    /// A heat map.
    Heatmap {
        /// Row names.
        rows: Vec<String>,
        /// Column names.
        columns: Vec<String>,
        /// Values, per row.
        #[serde(with = "nums::rows")]
        values: Vec<Vec<f64>>,
        /// Text of the cells.
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(with = "nums::vec")]
    pub values: Vec<f64>,
    /// Density at the values.
    #[serde(with = "nums::vec")]
    pub density: Vec<f64>,
}

/// The name of a color map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScaleName {
    /// Viridis.
    Viridis,
    /// Magma.
    Magma,
    /// Red to blue, diverging.
    Rdbu,
    /// Cool to warm, diverging.
    Coolwarm,
    /// Greys.
    Greys,
}

/// A color scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorScale {
    /// Name of the color map.
    pub name: ColorScaleName,
    /// Value of the low end.
    pub low: f64,
    /// Value of the high end.
    pub high: f64,
    /// Value at the middle of a diverging map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Edge color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    if let Some(c) = s.center
        && !(s.low..=s.high).contains(&c)
    {
        return err(format!("{path}.center"), "must be between low and high");
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
            for (i, b) in stats.iter().enumerate() {
                let order = [b.low_whisker, b.q1, b.median, b.q3, b.high_whisker];
                if !order.windows(2).all(|w| w[0] <= w[1]) {
                    return err(
                        format!("{p}.stats[{i}]"),
                        "needs low_whisker <= q1 <= median <= q3 <= high_whisker",
                    );
                }
            }
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
            if edges.is_empty() {
                return err(format!("{p}.edges"), "must not be empty");
            }
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
        Polygon { paths } => {
            if paths.is_empty() {
                return err(format!("{p}.paths"), "must not be empty");
            }
            for (i, ring) in paths.iter().enumerate() {
                if ring.len() < 3 {
                    return err(
                        format!("{p}.paths[{i}]"),
                        format!("{} points, a ring needs at least 3", ring.len()),
                    );
                }
            }
        }
        Text { .. } | Rule { .. } => {}
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
    for (name, v) in [
        ("title", f.fonts.title),
        ("label", f.fonts.label),
        ("tick", f.fonts.tick),
        ("legend", f.fonts.legend),
    ] {
        if !(v > 0.0) {
            return err(format!("fonts.{name}"), "must be positive");
        }
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
    // the id of an element names its group in the drawing, so it is unique in the figure
    let mut ids = HashSet::new();
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
        if let Some(order) = &p.legend_order {
            let mut seen = HashSet::new();
            for (k, g) in order.iter().enumerate() {
                if !seen.insert(g.as_str()) {
                    return err(
                        format!("{pp}.legend_order[{k}]"),
                        format!("duplicate legend group `{g}`"),
                    );
                }
            }
        }
        for (j, e) in p.elements.iter().enumerate() {
            let ep = format!("{pp}.elements[{j}]");
            if !ids.insert(e.id.as_str()) {
                return err(
                    format!("{ep}.id"),
                    format!("duplicate element id `{}` in the figure", e.id),
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

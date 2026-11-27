//! The types of the figure spec `figure/1` and the validation of what the types cannot say.
//!
//! Every object refuses unknown fields and uses snake case. A field with a default may be absent.
//!
//! In every numeric data array JSON `null` is a gap, a value which is not there, and reads as
//! `f64::NAN`; a NaN is written as `null`.

// a negated comparison also refuses NaN, which is intended
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::collections::{BTreeMap, HashSet};

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::FIGURE_CONTRACT;
pub use crate::colors::ColorScaleName;
// SUB_PATH is the U+0001 marker of the path inside an element, see `in_element`
use crate::error::{SUB_PATH, SpecError};

fn yes() -> bool {
    true
}
fn one() -> u32 {
    1
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_one(v: &f64) -> bool {
    *v == 1.0
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_true(v: &bool) -> bool {
    *v
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

/// The largest `width` and `height` of a figure in pixels.
pub const MAX_SIZE: f64 = 16384.0;

/// The largest number of `rows` and of `columns` of the grid of a figure.
pub const MAX_GRID: u32 = 64;

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
    /// Legends of the figure of the groups they list (since 0.33.0), right of the grid at its
    /// top or below it, each with a title, columns and a family of its own (matplotlib's
    /// `fig.legend(handles, labels, loc="outside right upper")` and `"outside lower center"`):
    /// the groups they list leave the legends of the plots. Two of a side are stacked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legends: Option<Vec<FigureLegend>>,
    /// The titles of the rows of the grid (since 0.33.0), one per row (an empty one for none),
    /// centred above the panels of each row as the title of a row of subfigures (matplotlib's
    /// `subfig.suptitle`), in the size and weight of the plot titles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_titles: Option<Vec<String>>,
    /// The plots.
    pub plots: Vec<Plot>,
}

/// The weight of a font: regular or bold (since 0.28.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    /// Inter Regular.
    #[default]
    Normal,
    /// Inter Bold.
    Bold,
}

impl FontWeight {
    fn is_normal(&self) -> bool {
        *self == FontWeight::Normal
    }
    fn is_bold(&self) -> bool {
        *self == FontWeight::Bold
    }
}

/// The family of the font of a text element (since 0.31.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    /// Inter, the font of every text of a figure.
    #[default]
    Sans,
    /// A monospace font: every character 0.6 of the font size wide, as the layout measures it;
    /// drawn with `ui-monospace, SFMono-Regular, Menlo, Consolas, Noto Sans Mono, monospace` in
    /// the SVG and with Noto Sans Mono in the PNG, so the columns of a table of text align.
    Monospace,
}

impl FontFamily {
    fn is_sans(&self) -> bool {
        *self == FontFamily::Sans
    }
}

fn bold() -> FontWeight {
    FontWeight::Bold
}

/// Font sizes in points and font weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    /// Title font size, 16 by default.
    #[serde(default = "d_title")]
    pub title: f64,
    /// Axis label font size, 13 by default.
    #[serde(default = "d_label")]
    pub label: f64,
    /// Tick label font size of the x axis, 11 by default, and of the y axes without `tick_y`.
    #[serde(default = "d_tick")]
    pub tick: f64,
    /// Legend font size, 11 by default.
    #[serde(default = "d_tick")]
    pub legend: f64,
    /// Tick label font size of the y axes and the colour bars, `tick` when absent (since
    /// 0.28.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_y: Option<f64>,
    /// Font size of the plot titles, `label` when absent (since 0.31.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_title: Option<f64>,
    /// Weight of the figure and plot titles, bold by default (since 0.28.0).
    #[serde(default = "bold", skip_serializing_if = "FontWeight::is_bold")]
    pub title_weight: FontWeight,
    /// Weight of the plot titles, `title_weight` when absent (since 0.33.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_title_weight: Option<FontWeight>,
    /// Font size of the labels of the colour bars, `label` when absent (since 0.33.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbar_label: Option<f64>,
    /// Weight of the labels of the colour bars, `label_weight` when absent (since 0.34.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbar_label_weight: Option<FontWeight>,
    /// Font size of the tick labels of the colour bars, `tick_y` (else `tick`) when absent
    /// (since 0.34.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbar_tick: Option<f64>,
    /// The family of the legend labels and titles, `sans` by default (since 0.33.0; matplotlib's
    /// `prop={"family": "monospace"}` of a legend); a legend may set its own.
    #[serde(default, skip_serializing_if = "FontFamily::is_sans")]
    pub legend_family: FontFamily,
    /// Weight of the axis titles and colour bar labels, normal by default (since 0.28.0).
    #[serde(default, skip_serializing_if = "FontWeight::is_normal")]
    pub label_weight: FontWeight,
    /// Weight of the tick labels, normal by default (since 0.28.0).
    #[serde(default, skip_serializing_if = "FontWeight::is_normal")]
    pub tick_weight: FontWeight,
    /// Weight of the legend labels, normal by default (since 0.28.0).
    #[serde(default, skip_serializing_if = "FontWeight::is_normal")]
    pub legend_weight: FontWeight,
}

impl Fonts {
    /// The tick label font size of the y axes and the colour bars: `tick_y`, else `tick`.
    pub fn y_tick(&self) -> f64 {
        self.tick_y.unwrap_or(self.tick)
    }

    /// The font size of the plot titles: `plot_title`, else `label`.
    pub fn plot_title_size(&self) -> f64 {
        self.plot_title.unwrap_or(self.label)
    }

    /// The weight of the plot titles: `plot_title_weight`, else `title_weight`.
    pub fn plot_title_weight(&self) -> FontWeight {
        self.plot_title_weight.unwrap_or(self.title_weight)
    }

    /// The font size of the labels of the colour bars: `colorbar_label`, else `label`.
    pub fn colorbar_label_size(&self) -> f64 {
        self.colorbar_label.unwrap_or(self.label)
    }

    /// The weight of the labels of the colour bars: `colorbar_label_weight`, else
    /// `label_weight`.
    pub fn colorbar_label_weight(&self) -> FontWeight {
        self.colorbar_label_weight.unwrap_or(self.label_weight)
    }

    /// The font size of the tick labels of the colour bars: `colorbar_tick`, else `tick_y`,
    /// else `tick`.
    pub fn colorbar_tick_size(&self) -> f64 {
        self.colorbar_tick.unwrap_or_else(|| self.y_tick())
    }
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
            tick_y: None,
            plot_title: None,
            title_weight: FontWeight::Bold,
            plot_title_weight: None,
            colorbar_label: None,
            colorbar_label_weight: None,
            colorbar_tick: None,
            legend_family: FontFamily::Sans,
            label_weight: FontWeight::Normal,
            tick_weight: FontWeight::Normal,
            legend_weight: FontWeight::Normal,
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
    /// One legend of the figure below the grid, centred, its entries in columns (since
    /// 0.31.0; matplotlib's `fig.legend(loc="lower center", ncol=n)`).
    Bottom,
}

/// Where a legend inside a panel goes (since 0.30.0): the locations of matplotlib's `loc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendLoc {
    /// The location of the least data among the others, in their order on a tie
    /// (matplotlib's `best`).
    #[default]
    Best,
    /// The upper right corner.
    UpperRight,
    /// The upper left corner.
    UpperLeft,
    /// The lower left corner.
    LowerLeft,
    /// The lower right corner.
    LowerRight,
    /// The middle of the right side, as `center_right`.
    Right,
    /// The middle of the left side.
    CenterLeft,
    /// The middle of the right side.
    CenterRight,
    /// The middle of the bottom side.
    LowerCenter,
    /// The middle of the top side.
    UpperCenter,
    /// The center of the panel.
    Center,
}

impl LegendLoc {
    /// The locations `best` chooses from, in the order of matplotlib, which wins a tie.
    pub const CANDIDATES: [LegendLoc; 9] = [
        LegendLoc::UpperRight,
        LegendLoc::UpperLeft,
        LegendLoc::LowerLeft,
        LegendLoc::LowerRight,
        LegendLoc::CenterLeft,
        LegendLoc::CenterRight,
        LegendLoc::LowerCenter,
        LegendLoc::UpperCenter,
        LegendLoc::Center,
    ];
}

/// Legend settings of a figure.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegendSettings {
    /// Whether a legend is per plot or per row.
    #[serde(default)]
    pub scope: LegendScope,
    /// Where the legend is placed.
    #[serde(default)]
    pub position: LegendPosition,
    /// Where a legend inside goes, `best` by default (since 0.30.0); a plot may set its own.
    #[serde(default)]
    pub loc: LegendLoc,
    /// The columns of the entries of the legend below the grid (since 0.31.0); absent, as many
    /// as fit into the width of the figure in one row, and the entries in rows of that many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// The title of the legend below the grid (since 0.31.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Where a legend of the figure goes (since 0.33.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FigureLegendPosition {
    /// Right of the grid, at its top.
    Right,
    /// Below the grid, centred on the figure.
    Bottom,
    /// Above the grid, below the title of the figure, centred on the figure (since 0.34.0;
    /// matplotlib's `fig.legend(loc="outside upper center")`): the figure grows by it.
    Top,
}

/// A legend of the figure of the groups it lists (since 0.33.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigureLegend {
    /// The legend groups of its entries, in their order; a group of no element is left out.
    pub groups: Vec<String>,
    /// Where it goes.
    pub position: FigureLegendPosition,
    /// Its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Its columns, 1 by default right of the grid; below it as many as fit in one row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// The family of its labels, the `legend_family` of the fonts by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<FontFamily>,
    /// The font size of its labels and title, the `legend` size of the fonts by default
    /// (since 0.34.0; matplotlib's `fontsize` of a legend).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// The weight of its labels, the `legend_weight` of the fonts by default (since 0.34.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<FontWeight>,
    /// Whether it has a frame, true by default (since 0.34.0; matplotlib's `frameon`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub frame: bool,
    /// The column of the grid it is centred under (`bottom`) or over (`top`), instead of the
    /// figure (since 0.34.0; matplotlib's `bbox_to_anchor` of a figure legend at the middle of
    /// the panels of a column): the panels of the plots of that column which span it alone,
    /// else of every plot over it. Legends of different columns sit side by side, each column
    /// stacking its own; the legends of the figure below them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// A legend of a plot besides its own (since 0.33.0): of the groups it lists, in its panel
/// (matplotlib's second legend of an axes, `ax.add_artist(legend)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlotLegend {
    /// The legend groups of its entries, in their order; they leave the legend of the plot.
    pub groups: Vec<String>,
    /// Its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Where it goes in the panel, `best` by default (avoiding the legends placed before it);
    /// with an `anchor` the side or corner of the legend at the anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<LegendLoc>,
    /// A point in fractions of the panel the legend is anchored at, see `Plot::legend_anchor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f64; 2]>,
    /// Its columns, 1 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// The family of its labels, the `legend_family` of the fonts by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<FontFamily>,
    /// The font size of its labels and title, the `legend` size of the fonts by default
    /// (since 0.34.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// The weight of its labels, the `legend_weight` of the fonts by default (since 0.34.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<FontWeight>,
    /// Whether it has a frame, true by default (since 0.34.0; matplotlib's `frameon`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub frame: bool,
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
    /// A second x axis on top of the panel with ticks and labels of its own (since 0.33.0;
    /// matplotlib's `twiny`): its limits its `min` and `max`, else those of the x axis; no
    /// element is drawn against it. The title of the plot goes above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x2: Option<Axis>,
    /// Whether the plot has a legend, true by default.
    #[serde(default = "yes")]
    pub legend: bool,
    /// Where the legend of the plot goes inside its panel (since 0.30.0); absent, the `loc` of
    /// the legend settings of the figure. A row legend inside takes the one of its host plot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_loc: Option<LegendLoc>,
    /// The title of the legend of the plot, drawn above its entries (since 0.31.0;
    /// matplotlib's `ax.legend(title=...)`). A row legend takes the first title of the plots
    /// of its row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_title: Option<String>,
    /// The columns of the legend of the plot, filled top down and then left to right
    /// (since 0.31.1; matplotlib's `ncol`), 1 by default. A row legend takes the first of the
    /// plots of its row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_columns: Option<u32>,
    /// A point in fractions of the panel (since 0.33.0) the legend of the plot is anchored at:
    /// its `legend_loc` (upper right without one) names the side or corner of the legend at the
    /// point, half a legend font size off it (matplotlib's `bbox_to_anchor=(x, y)` with `loc`);
    /// the legend may reach out of the panel. An anchor beyond the panel (a fraction below 0 or
    /// above 1, since 0.34.0) reserves the room of the part of the legend beyond the panel on
    /// that side, so the legend is not cut off by the figure; inside it takes no room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_anchor: Option<[f64; 2]>,
    /// The font size of the labels and the title of the legend of the plot, the `legend` size
    /// of the fonts by default (since 0.34.0; matplotlib's `fontsize` of a legend). A row
    /// legend takes the first of the plots of its row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_size: Option<f64>,
    /// The weight of the labels of the legend of the plot, the `legend_weight` of the fonts by
    /// default (since 0.34.0). A row legend takes the first of the plots of its row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_weight: Option<FontWeight>,
    /// The family of the labels and the title of the legend of the plot, the `legend_family`
    /// of the fonts by default (since 0.34.0). A row legend takes the first of the plots of its
    /// row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_family: Option<FontFamily>,
    /// Whether the legend of the plot has a frame, true by default (since 0.34.0; matplotlib's
    /// `frameon=False`): without one its entries stand on the panel. A row legend has none when
    /// a plot of its row has none.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub legend_frame: bool,
    /// Further legends of the plot in its panel (since 0.33.0), each of the groups it lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legends: Option<Vec<PlotLegend>>,
    /// The legend groups whose swatch draws the markers of all their elements side by side
    /// (since 0.33.0; matplotlib's `HandlerTuple`), e.g. one entry of the markers of several
    /// individuals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_tuples: Option<Vec<String>>,
    /// A color bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbar: Option<ColorBar>,
    /// Several color bars, stacked right of the panel in their order (since 0.28.0); refused
    /// together with `colorbar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorbars: Option<Vec<ColorBar>>,
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
    /// The swatches of legend groups by group (since 0.28.0): the swatch of a group drawn with
    /// this style instead of the style of its first element (or of the neutral one of a group
    /// of elements of different colours), its kind the kind of that element; a style without a
    /// colour takes the colour the swatch would have. Every key is a legend group of the plot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_swatches: Option<BTreeMap<String, Style>>,
    /// The aspect of the panel (since 0.30.0): `equal`, one unit of the coordinate of y as
    /// long as one of x, or a positive number, one unit of y that many times as long as one of x
    /// (matplotlib's `set_aspect` with `adjustable="datalim"`). The layout keeps the panel and
    /// widens the limits of one axis about their middle until the units have that ratio, so the
    /// data keeps its room; absent, the axes take their limits independently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<Aspect>,
    /// The frame of the panel (since 0.33.0): a line around all four sides of it in a colour
    /// and a width, instead of the lines of the left and bottom axes (matplotlib's four
    /// `spines` of one colour and width).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<PanelFrame>,
    /// Draws the plot as an inset in the panel of another plot (since 0.33.0), matplotlib's
    /// `ax.inset_axes([x, y, width, height])`: its panel at that rectangle of fractions of the
    /// panel of the host, with its own axes, ticks, labels and legend around it over the host,
    /// and a white panel. Its `row` and `column` are those of the host and its spans 1; it
    /// takes no room of the grid, and its host comes before it in `plots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inset: Option<Inset>,
    /// The elements drawn in the plot.
    pub elements: Vec<Element>,
}

/// The frame of a panel (since 0.33.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelFrame {
    /// The colour `#rrggbb`, the colour of the axis lines by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The width of the line in pixels, 1 by default; centred on the edge of the panel.
    #[serde(default = "one_f", skip_serializing_if = "is_one")]
    pub width: f64,
}

/// Where an inset plot goes (since 0.33.0): a rectangle of fractions of the panel of its host,
/// from its lower left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inset {
    /// The id of the host plot.
    pub plot: String,
    /// The left edge as a fraction of the width of the panel of the host.
    pub x: f64,
    /// The lower edge as a fraction of the height of the panel of the host.
    pub y: f64,
    /// The width as a fraction of the width of the panel of the host.
    pub width: f64,
    /// The height as a fraction of the height of the panel of the host.
    pub height: f64,
}

/// The aspect of a panel: the length of one unit of y as a multiple of the length of one unit
/// of x, on the coordinates of the axes (the value, its `log10` on a log axis, the position of
/// a category).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aspect {
    /// One unit of y is as long as one unit of x, written `"equal"`.
    Equal,
    /// One unit of y is this many times as long as one unit of x, a positive number.
    Ratio(f64),
}

impl Aspect {
    /// The ratio of the aspect: 1 for `equal`.
    pub fn ratio(&self) -> f64 {
        match self {
            Aspect::Equal => 1.0,
            Aspect::Ratio(r) => *r,
        }
    }
}

impl Serialize for Aspect {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Aspect::Equal => s.serialize_str("equal"),
            Aspect::Ratio(r) => s.serialize_f64(*r),
        }
    }
}

impl<'de> Deserialize<'de> for Aspect {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(t) if t == "equal" => Ok(Aspect::Equal),
            serde_json::Value::Number(n) => match n.as_f64() {
                Some(r) => Ok(Aspect::Ratio(r)),
                None => Err(D::Error::custom("expected `equal` or a positive number")),
            },
            other => Err(D::Error::custom(format!(
                "expected `equal` or a positive number, got {other}"
            ))),
        }
    }
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
    /// Whether the tick labels are shown, true by default, and the tick marks without
    /// `tick_marks_visible`.
    #[serde(default = "yes")]
    pub ticks_visible: bool,
    /// Whether the tick marks are shown (since 0.28.0); absent, they follow `ticks_visible`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_marks_visible: Option<bool>,
    /// Whether the line of the axis is shown, true by default (since 0.30.0).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub line_visible: bool,
    /// Rotation of the tick labels in degrees.
    #[serde(default)]
    pub tick_rotation: f64,
    /// The values of the ticks (since 0.31.0), instead of the ones the axis computes:
    /// matplotlib's `set_xticks`. A tick outside of the limits is not drawn; the ticks do not
    /// change the limits. Not on a category axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<Vec<f64>>,
    /// The labels of the `ticks`, one per tick (since 0.31.0); their numbers without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_labels: Option<Vec<String>>,
    /// The colour of the tick labels (since 0.33.0): one `#rrggbb` for all of them, or one per
    /// tick of `ticks` or per category of a category axis (matplotlib's
    /// `ticklabel.set_color`); the colour of the theme without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_label_color: Option<TickColors>,
    /// The weight of the tick labels of the axis, the `tick_weight` of the fonts by default
    /// (since 0.34.0; matplotlib's `fontweight` of the tick labels of one axis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_weight: Option<FontWeight>,
    /// The font size of the title of the axis, the `label` size of the fonts by default (since
    /// 0.34.0; e.g. the smaller labels of an inset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_size: Option<f64>,
}

/// The colours of the tick labels of an axis (since 0.33.0): one for all, or one per tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TickColors {
    /// One colour for every tick label.
    One(String),
    /// One colour per tick of the `ticks` of the axis.
    Each(Vec<String>),
}

impl TickColors {
    /// The colour of the tick label of the tick at `index`.
    pub fn of(&self, index: usize) -> Option<&str> {
        match self {
            TickColors::One(c) => Some(c),
            TickColors::Each(v) => v.get(index).map(String::as_str),
        }
    }
}

impl Axis {
    /// Whether the tick marks are shown: `tick_marks_visible`, else `ticks_visible`.
    pub fn marks_visible(&self) -> bool {
        self.tick_marks_visible.unwrap_or(self.ticks_visible)
    }

    /// The weight of the tick labels: `tick_weight`, else the one of the fonts.
    pub fn tick_weight_of(&self, fonts: &Fonts) -> FontWeight {
        self.tick_weight.unwrap_or(fonts.tick_weight)
    }

    /// The font size of the title: `label_size`, else the `label` size of the fonts.
    pub fn label_size_of(&self, fonts: &Fonts) -> f64 {
        self.label_size.unwrap_or(fonts.label)
    }
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
    /// What the positions of the element are in (since 0.33.0): the values of the axes, or
    /// fractions of the panel for a curve, a polygon, a text or an arrow, which then put
    /// nothing on the axes.
    pub coordinates: Coordinates,
    /// Whether the element is clipped to the panel, true by default (since 0.33.0): a curve,
    /// a polygon, a text or an arrow which is not is drawn over the whole figure (matplotlib's
    /// `clip_on=False`), e.g. the slashes of a broken axis or a text at the edge of the panel.
    pub clip: bool,
    /// The drawing order of the element (since 0.34.0; matplotlib's `zorder`): the elements of
    /// a plot are drawn by increasing `z`, those of the left and of the right y axis together,
    /// in their order on a tie (the elements of the left axis before those of the right one);
    /// 0 by default.
    pub z: Option<f64>,
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
    #[serde(default, skip_serializing_if = "Coordinates::is_data")]
    coordinates: Coordinates,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    clip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    z: Option<f64>,
}

const COMMON_KEYS: [&str; 8] = [
    "id",
    "label",
    "legend",
    "axis",
    "style",
    "coordinates",
    "clip",
    "z",
];

/// The fields of the kinds of an element besides the common ones, `None` for an unknown kind.
fn kind_fields(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "curve" => &["x", "y", "xerr", "yerr", "error_cap", "error_width"],
        "band" => &["x", "low", "high", "median"],
        "area" => &["x", "from", "to"],
        "bar" => &[
            "x",
            "height",
            "err",
            "horizontal",
            "width",
            "error_cap",
            "error_width",
            "base",
            "edge_color",
            "edge_width",
        ],
        "box" => &[
            "positions",
            "stats",
            "width",
            "median_color",
            "whisker_opacity",
            "median_opacity",
        ],
        "violin" => &["positions", "densities"],
        "histogram" => &["edges", "counts", "base", "edge_color", "edge_width"],
        "heatmap" => &[
            "rows", "columns", "values", "text", "scale", "x_edges", "y_edges",
        ],
        "text" => &[
            "x", "y", "text", "anchor", "size", "weight", "family", "rotation", "baseline", "box",
            "loc",
        ],
        "rule" => &["orientation", "value"],
        "contour" => &[
            "x",
            "y",
            "values",
            "levels",
            "filled",
            "scale",
            "labels",
            "label_size",
        ],
        "arrow" => &[
            "x",
            "y",
            "x_from",
            "y_from",
            "head",
            "head_length",
            "head_width",
            "body",
            "shrink",
            "text",
            "size",
            "weight",
            "anchor",
            "baseline",
        ],
        "polygon" => &["paths"],
        "image" => &["png", "extent", "origin", "interpolation"],
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

/// The errors of the values of an element along one axis: one error per value, which reaches
/// as far below the value as above it, or (since 0.33.0) the two sides apart, written
/// `{"low": [...], "high": [...]}`, matplotlib's `yerr` of shape `(2, n)`: the error below
/// each value and the one above it, a gap (`null`) a side which is not drawn, so a median with
/// its quantiles or an SD above the mean alone (a one-sided error bar) is drawn as matplotlib
/// draws it.
#[derive(Debug, Clone, PartialEq)]
pub enum Errors {
    /// One error per value, below and above it; its sign is ignored.
    Symmetric(Vec<f64>),
    /// The errors below and above every value apart, not negative, a gap a side not drawn.
    Sides {
        /// The error below every value.
        low: Vec<f64>,
        /// The error above every value.
        high: Vec<f64>,
    },
}

impl Errors {
    /// The number of values the errors are of.
    pub fn len(&self) -> usize {
        match self {
            Errors::Symmetric(e) => e.len(),
            Errors::Sides { low, .. } => low.len(),
        }
    }

    /// Whether there are no errors.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The error below and the error above the value `i`, each not negative or NaN for a side
    /// which is not drawn (a gap, or no error at `i`).
    pub fn at(&self, i: usize) -> (f64, f64) {
        let get = |v: &[f64]| v.get(i).copied().unwrap_or(f64::NAN);
        match self {
            Errors::Symmetric(e) => {
                let e = get(e).abs();
                (e, e)
            }
            Errors::Sides { low, high } => (get(low), get(high)),
        }
    }
}

impl Serialize for Errors {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let values = |v: &[f64]| -> Vec<Option<f64>> {
            v.iter()
                .map(|x| if x.is_nan() { None } else { Some(*x) })
                .collect()
        };
        match self {
            Errors::Symmetric(e) => values(e).serialize(s),
            Errors::Sides { low, high } => {
                let mut map = serde_json::Map::new();
                map.insert("low".into(), serde_json::json!(values(low)));
                map.insert("high".into(), serde_json::json!(values(high)));
                serde_json::Value::Object(map).serialize(s)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Errors {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let numbers = |v: serde_json::Value, side: &str| -> Result<Vec<f64>, D::Error> {
            let values: Vec<Option<f64>> = serde_json::from_value(v).map_err(|e| {
                D::Error::custom(if side.is_empty() {
                    format!("expected an array of numbers: {e}")
                } else {
                    format!("`{side}`: expected an array of numbers: {e}")
                })
            })?;
            Ok(values.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
        };
        match serde_json::Value::deserialize(d)? {
            v @ serde_json::Value::Array(_) => Ok(Errors::Symmetric(numbers(v, "")?)),
            serde_json::Value::Object(mut map) => {
                if let Some(k) = map.keys().find(|k| *k != "low" && *k != "high") {
                    return Err(D::Error::custom(format!(
                        "unknown field `{k}`, expected `low` and `high`"
                    )));
                }
                let mut side = |name: &str| match map.remove(name) {
                    Some(v) => numbers(v, name),
                    None => Err(D::Error::custom(format!("missing field `{name}`"))),
                };
                let low = side("low")?;
                let high = side("high")?;
                Ok(Errors::Sides { low, high })
            }
            other => Err(D::Error::custom(format!(
                "expected an array of errors or {{\"low\": [...], \"high\": [...]}}, got {other}"
            ))),
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
            && ColorScaleName::from_name(n).is_none()
        {
            return Err(in_element(&"scale.name", crate::colors::unknown_scale(n)));
        }
        // the family of a text likewise (since 0.31.1)
        if let Some(f) = map.get("family")
            && serde_json::from_value::<FontFamily>(f.clone()).is_err()
        {
            return Err(in_element(
                &"family",
                format!("unknown font family {f}, expected `sans` or `monospace`"),
            ));
        }
        // the weight of a text likewise (since 0.30.0)
        if let Some(w) = map.get("weight")
            && serde_json::from_value::<FontWeight>(w.clone()).is_err()
        {
            return Err(in_element(
                &"weight",
                format!("unknown font weight {w}, expected `normal` or `bold`"),
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
            coordinates: c.coordinates,
            clip: c.clip,
            z: c.z,
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
            coordinates: self.coordinates,
            clip: self.clip,
            z: self.z,
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
        /// Error of x: one per value, or the sides apart (since 0.33.0), see [`Errors`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        xerr: Option<Errors>,
        /// Error of y: one per value, or the sides apart (since 0.33.0), see [`Errors`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        yerr: Option<Errors>,
        /// The size of the caps of the error bars in pixels, matplotlib's `capsize`: a cap
        /// reaches that far to each side of its bar, so it is twice as long (since 0.37.0; as
        /// long as the size before), 0 (no caps) by default (since 0.30.0).
        #[serde(default, skip_serializing_if = "is_zero")]
        error_cap: f64,
        /// The width of the lines of the error bars in pixels, 1 by default (since 0.31.0;
        /// matplotlib's `elinewidth`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_width: Option<f64>,
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
        /// Errors of the heights: one per bar, or the sides apart (since 0.33.0), see
        /// [`Errors`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<Errors>,
        /// Horizontal bars.
        #[serde(default)]
        horizontal: bool,
        /// Width of a bar in axis units.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        /// The size of the caps of the error bars in pixels, matplotlib's `capsize`: a cap
        /// reaches that far to each side of its bar, so it is twice as long (since 0.37.0; as
        /// long as the size before), 0 (no caps) by default (since 0.30.0).
        #[serde(default, skip_serializing_if = "is_zero")]
        error_cap: f64,
        /// The width of the lines of the error bars in pixels, 1 by default (since 0.31.0;
        /// matplotlib's `elinewidth`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_width: Option<f64>,
        /// Where every bar starts, one per bar (since 0.33.0; matplotlib's `bottom`, `left` of
        /// horizontal bars): a bar runs from its base to its base and its height, its error bar
        /// at its end, so bars stack; 0 by default.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        base: Option<Vec<f64>>,
        /// The colour of the edges of the bars `#rrggbb` (since 0.34.0; matplotlib's
        /// `edgecolor`, e.g. white edges between neighbouring bars): with it or an
        /// `edge_width` every bar is outlined, in the colour of the element without one and
        /// in the width of the line of the style (1 without one) without an `edge_width`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge_color: Option<String>,
        /// The width of the edges of the bars in pixels (since 0.34.0; matplotlib's
        /// `linewidth` of bars), see `edge_color`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge_width: Option<f64>,
    },
    /// Box plots.
    Box {
        /// Positions.
        #[serde(with = "nums::vec")]
        positions: Vec<f64>,
        /// Statistics per position.
        stats: Vec<BoxStats>,
        /// The width of a box in units of the x axis (since 0.31.0; a share of the slot of a
        /// category on a category axis, whose categories are one unit apart): matplotlib's
        /// `widths`. Absent, half of the spacing of the positions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        /// The colour of the median line `#rrggbb` (since 0.33.0; matplotlib's `medianprops`),
        /// the colour of the element by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        median_color: Option<String>,
        /// The opacity of the whiskers and their caps (since 0.33.0; matplotlib's `alpha` of
        /// `whiskerprops` and `capprops`), the opacity of the style by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        whisker_opacity: Option<f64>,
        /// The opacity of the median line (since 0.34.0; matplotlib's `alpha` of
        /// `medianprops`, e.g. seaborn's opaque median in a translucent box), the opacity of the
        /// style by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        median_opacity: Option<f64>,
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
        /// Where the bar of every bin starts, one per bin (since 0.33.0; matplotlib's `bottom`):
        /// the bar runs from its base to its base and its count, so histograms stack; 0 by
        /// default.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        base: Option<Vec<f64>>,
        /// The colour of the edges of the bins `#rrggbb` (since 0.34.0; matplotlib's
        /// `edgecolor`, e.g. white edges between neighbouring bins): with it or an
        /// `edge_width` every bin is outlined, in the colour of the element without one and
        /// in the width of the line of the style (1 without one) without an `edge_width`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge_color: Option<String>,
        /// The width of the edges of the bins in pixels (since 0.34.0; matplotlib's
        /// `linewidth` of bars), see `edge_color`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge_width: Option<f64>,
    },
    /// A heat map.
    Heatmap {
        /// Row names; may be absent with `y_edges` (since 0.30.0).
        #[serde(default)]
        rows: Vec<String>,
        /// Column names; may be absent with `x_edges` (since 0.30.0).
        #[serde(default)]
        columns: Vec<String>,
        /// Values, per row.
        #[serde(with = "nums::rows")]
        values: Vec<Vec<f64>>,
        /// Text of the cells.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<Vec<Vec<String>>>,
        /// Color scale.
        scale: ColorScale,
        /// The edges of the columns on the x axis, one more than columns, strictly monotonic
        /// (since 0.30.0): column `j` spans `x_edges[j]..x_edges[j + 1]` of a numeric x axis;
        /// absent, the columns are the categories of the x axis.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        x_edges: Option<Vec<f64>>,
        /// The edges of the rows on the y axis, one more than rows, strictly monotonic (since
        /// 0.30.0): row `i` spans `y_edges[i]..y_edges[i + 1]` of a numeric y axis; absent, the
        /// rows are the categories of the y axis.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "nums::opt_vec"
        )]
        y_edges: Option<Vec<f64>>,
    },
    /// A text.
    Text {
        /// x position; required but with `loc`, which places the text instead.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        /// y position; required but with `loc`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        /// The text.
        text: String,
        /// The horizontal anchor, start by default (matplotlib's `ha`; of the box around the
        /// rotated text for a rotated one).
        #[serde(default)]
        anchor: Anchor,
        /// Font size, the `tick` size of the fonts by default (since 0.30.0).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<f64>,
        /// Font weight, normal by default (since 0.30.0).
        #[serde(default, skip_serializing_if = "FontWeight::is_normal")]
        weight: FontWeight,
        /// Font family, `sans` by default (since 0.31.1).
        #[serde(default, skip_serializing_if = "FontFamily::is_sans")]
        family: FontFamily,
        /// The rotation in degrees counterclockwise about the position (since 0.33.0;
        /// matplotlib's `rotation`), 0 by default; the anchors align the box around the
        /// rotated text, as matplotlib's default `rotation_mode`.
        #[serde(default, skip_serializing_if = "is_zero")]
        rotation: f64,
        /// The vertical anchor (since 0.33.0; matplotlib's `va`); absent, the text is centred
        /// on its position as before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline: Option<VerticalAnchor>,
        /// A box behind the text (since 0.33.0; matplotlib's `bbox`).
        #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
        text_box: Option<TextBox>,
        /// A location in the panel instead of a position (since 0.33.0), one of the locations
        /// of a legend but `best`: the text and its box (a white one with a black edge without
        /// a `box`) in that corner, as matplotlib's `AnchoredText` (`pad` 0.4 and `borderpad`
        /// 0.5 font sizes).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loc: Option<LegendLoc>,
    },
    /// An arrow from a tail to a head, with an optional text at its tail (since 0.33.0):
    /// matplotlib's `annotate(text, xy=(x, y), xytext=(x_from, y_from), arrowprops=...)`, a
    /// `FancyArrowPatch` or a `FancyArrow`. A line in the line of the style with a head, or
    /// with a `body` a filled arrow of that width; its colour is the text colour without one
    /// of the style, as matplotlib's black.
    Arrow {
        /// x of the head.
        x: f64,
        /// y of the head.
        y: f64,
        /// x of the tail.
        x_from: f64,
        /// y of the tail.
        y_from: f64,
        /// The head of an arrow of a line: `open` (matplotlib's `->`), `filled` (`-|>`, the
        /// default) or `none` (`-`); an arrow with a body has a filled head.
        #[serde(default, skip_serializing_if = "ArrowHead::is_filled")]
        head: ArrowHead,
        /// The length of the head in pixels: 0.4 of the `tick` font size for an arrow of a
        /// line (matplotlib's `head_length` of 0.4 of the size of an annotation), three body
        /// widths for an arrow with a body (`headlength` of 12 points over a `width` of 4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head_length: Option<f64>,
        /// The width of the head across the arrow in pixels: as long as the head of an arrow
        /// of a line, three body widths for an arrow with a body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head_width: Option<f64>,
        /// The width of the body of a filled arrow in pixels (matplotlib's `width` of the
        /// `arrowprops` of an annotation); absent, the arrow is a line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<f64>,
        /// The share of the length of the arrow left out at each end, 0 by default
        /// (matplotlib's `shrink`); an arrow of a line ends 2 pixels before its ends anyway
        /// (`shrinkA` and `shrinkB`).
        #[serde(default, skip_serializing_if = "is_zero")]
        shrink: f64,
        /// A text at the tail, the arrow starting at its edge.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// The font size of the text, the `tick` size of the fonts by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<f64>,
        /// The weight of the text.
        #[serde(default, skip_serializing_if = "FontWeight::is_normal")]
        weight: FontWeight,
        /// The horizontal anchor of the text, start by default.
        #[serde(default)]
        anchor: Anchor,
        /// The vertical anchor of the text, its baseline by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        baseline: Option<VerticalAnchor>,
    },
    /// Contours of a grid of values (since 0.33.0): lines at levels, with their labels inline
    /// (matplotlib's `contour` and `clabel`), or the bands between the levels filled
    /// (`contourf`). The colour of a line is the one of its level on the scale, of a band the
    /// one of its middle; without a scale the colour of the element.
    Contour {
        /// The coordinates of the columns of the grid on the x axis, strictly monotonic.
        #[serde(with = "nums::vec")]
        x: Vec<f64>,
        /// The coordinates of the rows of the grid on the y axis, strictly monotonic.
        #[serde(with = "nums::vec")]
        y: Vec<f64>,
        /// The values, per row; a gap is no value, the cells around it are not drawn.
        #[serde(with = "nums::rows")]
        values: Vec<Vec<f64>>,
        /// The levels, strictly increasing; at least two for filled bands.
        levels: Vec<f64>,
        /// Whether the bands between the levels are filled instead of lines drawn.
        #[serde(default, skip_serializing_if = "is_false")]
        filled: bool,
        /// The colour scale of the levels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<ColorScale>,
        /// The labels of the lines, one per level (an empty one for none), drawn on the lines
        /// as matplotlib's `clabel(inline=True)`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        labels: Option<Vec<String>>,
        /// The font size of the labels, the `tick` size of the fonts by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label_size: Option<f64>,
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
    /// A raster image over a rectangle of the values of the axes (since 0.34.0), matplotlib's
    /// `imshow(image, extent=(x0, x1, y0, y1), origin=...)`, e.g. the scan of a figure of a
    /// study under its digitized data: the PNG embedded in the SVG as a `data:` URI, stretched
    /// over the rectangle, at the opacity of the style. The axes stick to the edges of the
    /// rectangle, as matplotlib's limits of an image; the `aspect` of the plot keeps the
    /// proportions of the image (matplotlib's default `aspect="equal"` of `imshow`).
    Image {
        /// The PNG, base64 (the standard alphabet with its padding, no line breaks).
        png: String,
        /// The rectangle `[x0, x1, y0, y1]`: the left and the right edge, the bottom and the
        /// top edge, in the values of the axes; an edge left of (below) the other mirrors the
        /// image, as matplotlib's `extent`.
        extent: [f64; 4],
        /// Which edge the first row of the image goes at: `upper` (the default, matplotlib's
        /// default `origin`) at `y1`, `lower` at `y0`.
        #[serde(default, skip_serializing_if = "ImageOrigin::is_upper")]
        origin: ImageOrigin,
        /// How the pixels are resampled: `auto` (the default), `smooth` (bilinear) or
        /// `nearest` (each pixel a square, matplotlib's `interpolation="nearest"`).
        #[serde(default, skip_serializing_if = "Interpolation::is_auto")]
        interpolation: Interpolation,
    },
}

/// The edge of an image its first row goes at (since 0.34.0), matplotlib's `origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageOrigin {
    /// The first row at the top edge, `y1`.
    #[default]
    Upper,
    /// The first row at the bottom edge, `y0`.
    Lower,
}

impl ImageOrigin {
    fn is_upper(&self) -> bool {
        *self == ImageOrigin::Upper
    }
}

/// How the pixels of an image are resampled (since 0.34.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// As matplotlib's default `interpolation="auto"`: `nearest` for an image drawn at least
    /// three times its size along both sides, `smooth` for one drawn smaller or less enlarged.
    #[default]
    Auto,
    /// Bilinear, the default of a browser.
    Smooth,
    /// The nearest pixel, every pixel a square (`image-rendering: pixelated`).
    Nearest,
}

impl Interpolation {
    fn is_auto(&self) -> bool {
        *self == Interpolation::Auto
    }
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

/// The head of an arrow of a line (since 0.33.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowHead {
    /// Two strokes, matplotlib's `->`.
    Open,
    /// A filled triangle, matplotlib's `-|>`.
    #[default]
    Filled,
    /// No head, matplotlib's `-`.
    None,
}

impl ArrowHead {
    fn is_filled(&self) -> bool {
        *self == ArrowHead::Filled
    }
}

/// The vertical anchor of a text (since 0.33.0), matplotlib's `va`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAnchor {
    /// The baseline of the last line at the position.
    Baseline,
    /// The middle of the text at the position.
    Center,
    /// The top of the text at the position.
    Top,
    /// The bottom of the text at the position.
    Bottom,
}

/// The shape of the box of a text (since 0.33.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBoxStyle {
    /// A rectangle, matplotlib's `boxstyle="square"`.
    #[default]
    Square,
    /// A rectangle with corners rounded by the padding, matplotlib's `boxstyle="round"`.
    Round,
}

impl TextBoxStyle {
    fn is_square(&self) -> bool {
        *self == TextBoxStyle::Square
    }
}

/// The box behind a text (since 0.33.0), matplotlib's `bbox` of a text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBox {
    /// The fill `#rrggbb`, or `none`; white by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// The opacity of the fill, 1 by default.
    #[serde(default = "one_f", skip_serializing_if = "is_one")]
    pub opacity: f64,
    /// The colour of the edge `#rrggbb`, or `none`, the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// The width of the edge in pixels, 1 by default.
    #[serde(default = "one_f", skip_serializing_if = "is_one")]
    pub edge_width: f64,
    /// The padding between the text and the edge in pixels, 0.3 font sizes by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    /// The shape, `square` by default.
    #[serde(default, skip_serializing_if = "TextBoxStyle::is_square")]
    pub style: TextBoxStyle,
}

/// What the positions of an element are in (since 0.33.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coordinates {
    /// The values of the axes.
    #[default]
    Data,
    /// Fractions of the panel, 0 at its left (bottom) edge and 1 at its right (top) edge,
    /// matplotlib's `transform=ax.transAxes`; beyond 0 and 1 outside of the panel.
    Axes,
}

impl Coordinates {
    fn is_data(&self) -> bool {
        *self == Coordinates::Data
    }
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

/// A stop of a colour map built from colours at positions (since 0.31.0): a position in
/// `0..=1` and its colour `#rrggbb`, written `[position, "#rrggbb"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorStop(pub f64, pub String);

/// A color scale: a named colour map or a map of stops, and the values it spans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorScale {
    /// Name of the color map: a sequential or diverging map of matplotlib in lower case, or
    /// its reversal `<name>_r` (since 0.30.0; viridis, magma, rdbu, coolwarm, greys before).
    /// Required before 0.31.0; since then a scale has either a name or `stops`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ColorScaleName>,
    /// The colour map as colours at positions (since 0.31.0), matplotlib's
    /// `LinearSegmentedColormap.from_list(name, [(0.0, c0), (0.3, c1), ..., (1.0, cn)])`: at
    /// least two stops, the first at 0 and the last at 1, the positions not decreasing (two
    /// stops at one position are a step); the 256 colours of the map interpolated linearly in
    /// sRGB between them, as matplotlib's. Refused together with `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stops: Option<Vec<ColorStop>>,
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
    /// The exponent of a power scale (since 0.33.0), matplotlib's `PowerNorm(gamma)`: the
    /// share `t` of a value between `low` and `high` takes the colour at `t^gamma`; not on a
    /// log scale or with a `center`. matplotlib's `CenteredNorm` is a `center` between ends
    /// equally far from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamma: Option<f64>,
    /// The share of the color map `low` takes, 0 by default (since 0.28.0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// The share of the color map `high` takes, 1 by default (since 0.28.0): the values
    /// `low..high` take the part `start..end` of the map.
    #[serde(default = "one_f", skip_serializing_if = "is_one")]
    pub end: f64,
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
    /// The values of the ticks, computed from the scale without them (since 0.28.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<Vec<f64>>,
    /// The labels of the ticks, one per tick; the numbers of the ticks without them (since
    /// 0.28.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_labels: Option<Vec<String>>,
    /// One band of one color per integer from `low` to `high`, its tick at the middle of the
    /// band (since 0.28.0).
    #[serde(default, skip_serializing_if = "is_false")]
    pub categorical: bool,
    /// Lines across the bar at values of the scale (since 0.31.0), e.g. at the default of a
    /// scan (matplotlib's `cbar.ax.axhline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marks: Option<Vec<ColorBarMark>>,
    /// Where the bar goes (since 0.31.0), right of its panel by default.
    #[serde(default, skip_serializing_if = "ColorBarPlacement::is_right")]
    pub placement: ColorBarPlacement,
    /// The location of an inset bar in its panel (since 0.31.0), one of the locations of a
    /// legend but `best`, `upper_right` by default; only with the placement `inset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<LegendLoc>,
    /// The length of the bar as a share of what it runs along (since 0.31.0), matplotlib's
    /// `shrink`: of the height of its panel right of it (1 by default), of the width of the
    /// panels of its row above them (1 by default), of the width of its panel inset (0.5 by
    /// default); the bar centred on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// The thickness of the bar in pixels (since 0.31.0), 12 by default; of a strip 0.03 of
    /// the height of its panel, of a bar in the panel its width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f64>,
    /// Triangles at the ends of the bar for the values beyond them (since 0.33.0; matplotlib's
    /// `extend`), in the colours of the ends, 0.05 of the bar long, the bar shorter by them.
    #[serde(default, skip_serializing_if = "ColorBarExtend::is_neither")]
    pub extend: ColorBarExtend,
    /// The rows of the grid a bar right of its panel spans from the row of its plot (since
    /// 0.33.0), 1 by default: matplotlib's `fig.colorbar(..., ax=axes[:, -1])`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    /// The side of a horizontal bar its ticks, tick labels and label go (since 0.34.0;
    /// matplotlib's `ticklocation`): above a bar above a row and below the others by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_side: Option<TickSide>,
}

/// The side of a horizontal colour bar its ticks and labels go (since 0.34.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickSide {
    /// Above the bar.
    Top,
    /// Below the bar.
    Bottom,
}

/// The ends of a colour bar with a triangle (since 0.33.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorBarExtend {
    /// None.
    #[default]
    Neither,
    /// The low end.
    Min,
    /// The high end.
    Max,
    /// Both ends.
    Both,
}

impl ColorBarExtend {
    fn is_neither(&self) -> bool {
        *self == ColorBarExtend::Neither
    }

    /// Whether the low end and the high end have a triangle.
    pub fn ends(&self) -> (bool, bool) {
        match self {
            ColorBarExtend::Neither => (false, false),
            ColorBarExtend::Min => (true, false),
            ColorBarExtend::Max => (false, true),
            ColorBarExtend::Both => (true, true),
        }
    }
}

/// Where a colour bar goes (since 0.31.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorBarPlacement {
    /// Vertical, right of the panel of its plot and the bars before it, its low end at the
    /// bottom.
    #[default]
    Right,
    /// Horizontal, above the panels of the row of its plot and centred over them, its ticks
    /// and its label above it (matplotlib's `fig.colorbar(..., location="top")` over the axes
    /// of a row); the row gets the room. One bar of a row goes above it.
    Top,
    /// Horizontal, inside the panel of its plot at `loc`, its ticks and its label below it
    /// (matplotlib's `ax.inset_axes` with a horizontal colour bar); a legend at `best` avoids
    /// it.
    Inset,
    /// A strip of the colours along the top of the panel of its plot, as wide as the panel,
    /// without ticks, labels and outline (since 0.33.0; an inset image of a colour range over a
    /// panel); a legend at `best` avoids it.
    Strip,
    /// Vertical in place of the panel of its plot, which draws nothing else (since 0.33.0; a
    /// figure of a colour bar alone, matplotlib's `fig.colorbar(..., cax=ax)`): as wide as the
    /// panel without a thickness, its ticks and its label right of it.
    Panel,
    /// Horizontal, below the panels of the row of its plot and their axes, centred under them,
    /// its ticks and its label below it (since 0.34.0; matplotlib's `fig.colorbar(...,
    /// location="bottom")` under the axes of a row, below the grid for its last row); the row
    /// gets the room. One bar of a row goes below it.
    Bottom,
}

impl ColorBarPlacement {
    fn is_right(&self) -> bool {
        *self == ColorBarPlacement::Right
    }
}

/// A line across a colour bar at a value of its scale (since 0.31.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorBarMark {
    /// The value, between the ends of the scale.
    pub value: f64,
    /// The colour `#rrggbb`, black by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The width of the line in pixels, 2 by default.
    #[serde(default = "two")]
    pub width: f64,
}

impl Plot {
    /// The color bars of the plot, of `colorbar` or of `colorbars`, in their order.
    pub fn color_bars(&self) -> &[ColorBar] {
        match (&self.colorbar, &self.colorbars) {
            (Some(b), _) => std::slice::from_ref(b),
            (None, Some(bars)) => bars,
            (None, None) => &[],
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    /// Triangle pointing up, matplotlib's `^`.
    Triangle,
    /// Diamond, a square on its corner, matplotlib's `D` (as wide as high since 0.31.0, as
    /// `thin_diamond` before).
    Diamond,
    /// Cross.
    Cross,
    /// Plus.
    Plus,
    /// X.
    X,
    /// Triangle pointing down, matplotlib's `v` (since 0.31.0).
    TriangleDown,
    /// Triangle pointing left, matplotlib's `<` (since 0.31.0).
    TriangleLeft,
    /// Triangle pointing right, matplotlib's `>` (since 0.31.0).
    TriangleRight,
    /// Star of five points, matplotlib's `*` (since 0.31.0).
    Star,
    /// Filled X, matplotlib's `X` (since 0.31.0).
    XFilled,
    /// Filled plus, matplotlib's `P` (since 0.31.0).
    PlusFilled,
    /// Pentagon, matplotlib's `p` (since 0.31.0).
    Pentagon,
    /// Hexagon with a corner at the top, matplotlib's `h` (since 0.31.0).
    Hexagon,
    /// Thin diamond, matplotlib's `d` (since 0.31.0).
    ThinDiamond,
    /// Octagon with a flat top, matplotlib's `8` (since 0.33.0).
    Octagon,
    /// Three spokes, one down, matplotlib's `1` (since 0.33.0).
    TriDown,
    /// Three spokes, one up, matplotlib's `2` (since 0.33.0).
    TriUp,
    /// Three spokes, one to the left, matplotlib's `3` (since 0.33.0).
    TriLeft,
    /// Three spokes, one to the right, matplotlib's `4` (since 0.33.0).
    TriRight,
}

impl MarkerType {
    /// Whether the marker is lines (cross, plus, x, the tri markers), which have no fill.
    pub fn is_line(&self) -> bool {
        matches!(
            self,
            MarkerType::Cross
                | MarkerType::Plus
                | MarkerType::X
                | MarkerType::TriDown
                | MarkerType::TriUp
                | MarkerType::TriLeft
                | MarkerType::TriRight
        )
    }
}

/// Which part of a marker its fill fills (since 0.31.0), matplotlib's `fillstyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStyle {
    /// The whole marker.
    #[default]
    Full,
    /// The left half; the right half takes `fill_alt`.
    Left,
    /// The right half; the left half takes `fill_alt`.
    Right,
    /// The bottom half; the top half takes `fill_alt`.
    Bottom,
    /// The top half; the bottom half takes `fill_alt`.
    Top,
}

impl FillStyle {
    fn is_full(&self) -> bool {
        *self == FillStyle::Full
    }
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
    /// Fill color, or `none` for a hollow marker (since 0.28.0), whose edge is drawn without
    /// a fill; absent, the color of the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Edge color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// Edge width, 1 by default.
    #[serde(default = "one_f")]
    pub edge_width: f64,
    /// Which half the fill fills (since 0.31.0), the whole marker by default; the edge goes
    /// around both halves and between them, as matplotlib draws a half filled marker. A marker
    /// of lines has no fill and ignores it.
    #[serde(default, skip_serializing_if = "FillStyle::is_full")]
    pub fill_style: FillStyle,
    /// The fill of the other half of a half filled marker (since 0.31.0), `#rrggbb` or `none`,
    /// the default (matplotlib's `markerfacecoloralt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_alt: Option<String>,
}
impl Default for Marker {
    fn default() -> Self {
        Marker {
            kind: MarkerType::None,
            size: 6.0,
            fill: None,
            edge: None,
            edge_width: 1.0,
            fill_style: FillStyle::Full,
            fill_alt: None,
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
    /// Line; absent, the default line, which a bar does not draw: a bar draws the line of its
    /// style as its outline only when the style gives one (since 0.28.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Line>,
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
            line: None,
            marker: Marker::default(),
            fill: None,
        }
    }
}

impl Style {
    /// The line of the style: the line it gives, else the default line.
    pub fn line_or_default(&self) -> Line {
        self.line.unwrap_or_default()
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

fn check_style(path: &str, style: &Style) -> Result<(), SpecError> {
    if !(0.0..=1.0).contains(&style.opacity) {
        return err(format!("{path}.opacity"), "must be between 0 and 1");
    }
    if style.line_or_default().width < 0.0 {
        return err(format!("{path}.line.width"), "must not be negative");
    }
    if style.marker.size < 0.0 {
        return err(format!("{path}.marker.size"), "must not be negative");
    }
    check_color(format!("{path}.color"), &style.color)?;
    // a hollow marker has no fill
    if style.marker.fill.as_deref() != Some("none") {
        check_color(format!("{path}.marker.fill"), &style.marker.fill)?;
    }
    check_color(format!("{path}.marker.edge"), &style.marker.edge)?;
    if style.marker.fill_alt.as_deref() != Some("none") {
        check_color(format!("{path}.marker.fill_alt"), &style.marker.fill_alt)?;
    }
    if let Some(fill) = &style.fill {
        check_color(format!("{path}.fill.color"), &fill.color)?;
        if !(0.0..=1.0).contains(&fill.opacity) {
            return err(format!("{path}.fill.opacity"), "must be between 0 and 1");
        }
    }
    Ok(())
}

fn check_scale(path: &str, s: &ColorScale) -> Result<(), SpecError> {
    match (&s.name, &s.stops) {
        (Some(_), Some(_)) => {
            return err(
                format!("{path}.stops"),
                "refused together with name, a scale has either",
            );
        }
        (None, None) => return err(format!("{path}.name"), "a scale needs a name or stops"),
        (None, Some(stops)) => {
            if let Err((k, message)) = crate::colors::check_stops(stops) {
                let p = match k {
                    Some(k) => format!("{path}.stops[{k}]"),
                    None => format!("{path}.stops"),
                };
                return err(p, message);
            }
        }
        (Some(_), None) => {}
    }
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
    if let Some(g) = s.gamma {
        if !(g > 0.0 && g.is_finite()) {
            return err(format!("{path}.gamma"), "must be positive");
        }
        if s.log || s.center.is_some() {
            return err(
                format!("{path}.gamma"),
                "a power scale is linear and has no center",
            );
        }
    }
    if !(s.start >= 0.0 && s.start < 1.0) {
        return err(format!("{path}.start"), "must be at least 0 and below 1");
    }
    if !(s.end > s.start && s.end <= 1.0) {
        return err(format!("{path}.end"), "must be above start and at most 1");
    }
    Ok(())
}

/// The most bands of a categorical colour bar.
const MAX_BANDS: usize = 256;

fn check_colorbar(path: &str, b: &ColorBar) -> Result<(), SpecError> {
    let s = &b.scale;
    check_scale(&format!("{path}.scale"), s)?;
    let integer = |v: f64| v.is_finite() && v.fract() == 0.0;
    if b.categorical {
        if s.log {
            return err(
                format!("{path}.categorical"),
                "a categorical bar needs a linear scale",
            );
        }
        if !integer(s.low) {
            return err(
                format!("{path}.scale.low"),
                "must be an integer on a categorical bar",
            );
        }
        if !integer(s.high) {
            return err(
                format!("{path}.scale.high"),
                "must be an integer on a categorical bar",
            );
        }
        if s.high - s.low >= MAX_BANDS as f64 {
            return err(
                format!("{path}.scale.high"),
                format!("a categorical bar has at most {MAX_BANDS} bands"),
            );
        }
    }
    if let Some(ticks) = &b.ticks {
        for (k, t) in ticks.iter().enumerate() {
            let p = format!("{path}.ticks[{k}]");
            if !(t.is_finite() && (s.low..=s.high).contains(t)) {
                return err(p, format!("{t} is not between low and high"));
            }
            if b.categorical && !integer(*t) {
                return err(p, format!("{t} is no integer of a categorical bar"));
            }
        }
    }
    match (&b.ticks, &b.tick_labels) {
        // a categorical bar without ticks takes the integers from low to high, one per band
        (None, Some(l)) if b.categorical => {
            let bands = (s.high - s.low) as usize + 1;
            same_len(path, "tick_labels", l.len(), "bands", bands)
        }
        (None, Some(_)) => err(format!("{path}.tick_labels"), "tick labels need ticks"),
        (Some(t), Some(l)) => same_len(path, "tick_labels", l.len(), "ticks", t.len()),
        _ => Ok(()),
    }?;
    for (k, m) in b.marks.iter().flatten().enumerate() {
        let p = format!("{path}.marks[{k}]");
        if !(m.value.is_finite() && (s.low..=s.high).contains(&m.value)) {
            return err(
                format!("{p}.value"),
                format!("{} is not between low and high", m.value),
            );
        }
        check_color(format!("{p}.color"), &m.color)?;
        if !(m.width >= 0.0 && m.width.is_finite()) {
            return err(
                format!("{p}.width"),
                "must be a number of pixels, not negative",
            );
        }
    }
    match b.loc {
        Some(_) if b.placement != ColorBarPlacement::Inset => {
            return err(format!("{path}.loc"), "only an inset bar has a location");
        }
        Some(LegendLoc::Best) => {
            return err(
                format!("{path}.loc"),
                "an inset bar takes a fixed location, not best",
            );
        }
        _ => {}
    }
    if let Some(r) = b.rows
        && (r == 0 || b.placement != ColorBarPlacement::Right)
    {
        return err(
            format!("{path}.rows"),
            "a bar right of its panel spans at least 1 row",
        );
    }
    if let Some(l) = b.length
        && !(l > 0.0 && l <= 1.0)
    {
        return err(format!("{path}.length"), "must be above 0 and at most 1");
    }
    if let Some(t) = b.thickness
        && !(t > 0.0 && t.is_finite())
    {
        return err(format!("{path}.thickness"), "must be positive");
    }
    if b.tick_side.is_some()
        && !matches!(
            b.placement,
            ColorBarPlacement::Top | ColorBarPlacement::Bottom | ColorBarPlacement::Inset
        )
    {
        return err(
            format!("{path}.tick_side"),
            "only a horizontal bar with ticks (top, bottom, inset) has a side of its ticks",
        );
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
    if a.label_size.is_some_and(|v| !(v > 0.0 && v.is_finite())) {
        return err(format!("{path}.label_size"), "must be positive");
    }
    if let Some(ticks) = &a.ticks {
        if a.scale == AxisScale::Category {
            return err(
                format!("{path}.ticks"),
                "a category axis has its categories as ticks",
            );
        }
        for (k, t) in ticks.iter().enumerate() {
            if !t.is_finite() {
                return err(format!("{path}.ticks[{k}]"), "must be a finite number");
            }
            if a.scale == AxisScale::Log && *t <= 0.0 {
                return err(
                    format!("{path}.ticks[{k}]"),
                    "must be positive on a log axis",
                );
            }
        }
    }
    match &a.tick_label_color {
        Some(TickColors::One(c)) => {
            check_color(format!("{path}.tick_label_color"), &Some(c.clone()))?;
        }
        Some(TickColors::Each(colors)) => {
            let (n, of) = match (&a.ticks, &a.categories) {
                (Some(t), _) => (t.len(), "ticks"),
                (None, Some(c)) => (c.len(), "categories"),
                (None, None) => {
                    return err(
                        format!("{path}.tick_label_color"),
                        "a colour per tick label needs ticks or categories",
                    );
                }
            };
            same_len(path, "tick_label_color", colors.len(), of, n)?;
            for (k, c) in colors.iter().enumerate() {
                check_color(format!("{path}.tick_label_color[{k}]"), &Some(c.clone()))?;
            }
        }
        None => {}
    }
    match (&a.ticks, &a.tick_labels) {
        (None, Some(_)) => err(format!("{path}.tick_labels"), "tick labels need ticks"),
        (Some(t), Some(l)) => same_len(path, "tick_labels", l.len(), "ticks", t.len()),
        _ => Ok(()),
    }
}

fn check_error_width(p: &str, width: Option<f64>) -> Result<(), SpecError> {
    if width.is_some_and(|w| !(w >= 0.0 && w.is_finite())) {
        return err(
            format!("{p}.error_width"),
            "must be a number of pixels, not negative",
        );
    }
    Ok(())
}

/// The errors of `n` values: as many as the values, and the sides apart not negative.
fn check_errors(p: &str, name: &str, e: &Errors, n: usize) -> Result<(), SpecError> {
    match e {
        Errors::Symmetric(v) => same_len(p, name, v.len(), "x", n),
        Errors::Sides { low, high } => {
            for (side, v) in [("low", low), ("high", high)] {
                same_len(&format!("{p}.{name}"), side, v.len(), "x", n)?;
                if let Some(k) = v.iter().position(|e| *e < 0.0 || e.is_infinite()) {
                    return err(
                        format!("{p}.{name}.{side}[{k}]"),
                        "must be a finite number, not negative, or null for no error",
                    );
                }
            }
            Ok(())
        }
    }
}

/// The edges of bars and bins (since 0.34.0): a colour and a width not negative.
fn check_edge(p: &str, color: &Option<String>, width: Option<f64>) -> Result<(), SpecError> {
    check_color(format!("{p}.edge_color"), color)?;
    if width.is_some_and(|w| !(w >= 0.0 && w.is_finite())) {
        return err(
            format!("{p}.edge_width"),
            "must be a number of pixels, not negative",
        );
    }
    Ok(())
}

fn check_cap(p: &str, cap: f64) -> Result<(), SpecError> {
    if !(cap >= 0.0 && cap.is_finite()) {
        return err(
            format!("{p}.error_cap"),
            "must be a number of pixels, not negative",
        );
    }
    Ok(())
}

/// The edges of the cells of a heatmap along one axis: at least two finite values, strictly
/// increasing or strictly decreasing, as many as the names of that axis and one, when it has
/// names.
fn check_edges(
    p: &str,
    name: &str,
    edges: &[f64],
    names: &[String],
    names_field: &str,
) -> Result<(), SpecError> {
    let path = format!("{p}.{name}");
    if edges.len() < 2 {
        return err(path, "needs at least 2 values, the edges of one cell");
    }
    if let Some(k) = edges.iter().position(|e| !e.is_finite()) {
        return err(format!("{path}[{k}]"), "an edge must be a finite number");
    }
    let rising = edges.windows(2).all(|w| w[0] < w[1]);
    let falling = edges.windows(2).all(|w| w[0] > w[1]);
    if !(rising || falling) {
        return err(path, "must be strictly increasing or strictly decreasing");
    }
    if !names.is_empty() && names.len() + 1 != edges.len() {
        return err(
            path,
            format!(
                "{} values, {names_field} has {} so {name} needs {}",
                edges.len(),
                names.len(),
                names.len() + 1
            ),
        );
    }
    Ok(())
}

/// The rows and the columns of a heatmap: the edges less one along an axis with edges, else
/// the names.
pub(crate) fn heatmap_size(
    rows: &[String],
    columns: &[String],
    x_edges: Option<&[f64]>,
    y_edges: Option<&[f64]>,
) -> (usize, usize) {
    (
        y_edges.map_or(rows.len(), |e| e.len().saturating_sub(1)),
        x_edges.map_or(columns.len(), |e| e.len().saturating_sub(1)),
    )
}

fn check_kind(p: &str, kind: &ElementKind) -> Result<(), SpecError> {
    use ElementKind::*;
    match kind {
        Curve {
            x,
            y,
            xerr,
            yerr,
            error_cap,
            error_width,
        } => {
            check_cap(p, *error_cap)?;
            check_error_width(p, *error_width)?;
            same_len(p, "y", y.len(), "x", x.len())?;
            if let Some(e) = xerr {
                check_errors(p, "xerr", e, x.len())?;
            }
            if let Some(e) = yerr {
                check_errors(p, "yerr", e, x.len())?;
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
            error_cap,
            error_width,
            base,
            edge_color,
            edge_width,
            ..
        } => {
            check_edge(p, edge_color, *edge_width)?;
            check_cap(p, *error_cap)?;
            check_error_width(p, *error_width)?;
            same_len(p, "height", height.len(), "x", x.len())?;
            if let Some(e) = e {
                check_errors(p, "err", e, x.len())?;
            }
            if width.is_some_and(|w| !(w > 0.0)) {
                return err(format!("{p}.width"), "must be positive");
            }
            if let Some(b) = base {
                same_len(p, "base", b.len(), "x", x.len())?;
            }
        }
        Box {
            positions,
            stats,
            width,
            median_color,
            whisker_opacity,
            median_opacity,
        } => {
            check_color(format!("{p}.median_color"), median_color)?;
            for (name, o) in [
                ("whisker_opacity", whisker_opacity),
                ("median_opacity", median_opacity),
            ] {
                if o.is_some_and(|o| !(0.0..=1.0).contains(&o)) {
                    return err(format!("{p}.{name}"), "must be between 0 and 1");
                }
            }
            same_len(p, "stats", stats.len(), "positions", positions.len())?;
            if width.is_some_and(|w| !(w > 0.0 && w.is_finite())) {
                return err(format!("{p}.width"), "must be positive");
            }
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
        Histogram {
            edges,
            counts,
            base,
            edge_color,
            edge_width,
        } => {
            check_edge(p, edge_color, *edge_width)?;
            if let Some(b) = base {
                same_len(p, "base", b.len(), "counts", counts.len())?;
            }
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
            x_edges,
            y_edges,
        } => {
            if let Some(e) = x_edges {
                check_edges(p, "x_edges", e, columns, "columns")?;
            }
            if let Some(e) = y_edges {
                check_edges(p, "y_edges", e, rows, "rows")?;
            }
            let (n_rows, n_columns) =
                heatmap_size(rows, columns, x_edges.as_deref(), y_edges.as_deref());
            // the counts are named by what gives them: the names, or the edges without names
            let rows_by = if y_edges.is_some() && rows.is_empty() {
                "y_edges gives"
            } else {
                "rows has"
            };
            let columns_by = if x_edges.is_some() && columns.is_empty() {
                "x_edges gives"
            } else {
                "columns has"
            };
            if values.len() != n_rows {
                return err(
                    format!("{p}.values"),
                    format!("{} values, {rows_by} {n_rows}", values.len()),
                );
            }
            for (i, r) in values.iter().enumerate() {
                if r.len() != n_columns {
                    return err(
                        format!("{p}.values[{i}]"),
                        format!("{} values, {columns_by} {n_columns}", r.len()),
                    );
                }
            }
            if let Some(t) = text {
                if t.len() != n_rows {
                    return err(
                        format!("{p}.text"),
                        format!("{} values, {rows_by} {n_rows}", t.len()),
                    );
                }
                for (i, r) in t.iter().enumerate() {
                    if r.len() != n_columns {
                        return err(
                            format!("{p}.text[{i}]"),
                            format!("{} values, {columns_by} {n_columns}", r.len()),
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
        Text {
            x,
            y,
            size,
            rotation,
            text_box,
            loc,
            ..
        } => {
            if size.is_some_and(|v| !(v > 0.0 && v.is_finite())) {
                return err(format!("{p}.size"), "must be positive");
            }
            if !rotation.is_finite() {
                return err(
                    format!("{p}.rotation"),
                    "must be a finite number of degrees",
                );
            }
            match loc {
                Some(LegendLoc::Best) => {
                    return err(
                        format!("{p}.loc"),
                        "a text takes a fixed location, not best",
                    );
                }
                Some(_) => {
                    for (name, v) in [("x", x), ("y", y)] {
                        if v.is_some() {
                            return err(
                                format!("{p}.{name}"),
                                "refused together with loc, which places the text",
                            );
                        }
                    }
                }
                None => {
                    for (name, v) in [("x", x), ("y", y)] {
                        if v.is_none() {
                            return err(format!("{p}.{name}"), "a text needs x and y, or a loc");
                        }
                    }
                }
            }
            if let Some(b) = text_box {
                if b.fill.as_deref() != Some("none") {
                    check_color(format!("{p}.box.fill"), &b.fill)?;
                }
                if b.edge.as_deref() != Some("none") {
                    check_color(format!("{p}.box.edge"), &b.edge)?;
                }
                if !(0.0..=1.0).contains(&b.opacity) {
                    return err(format!("{p}.box.opacity"), "must be between 0 and 1");
                }
                if !(b.edge_width >= 0.0 && b.edge_width.is_finite()) {
                    return err(format!("{p}.box.edge_width"), "must not be negative");
                }
                if b.padding.is_some_and(|v| !(v >= 0.0 && v.is_finite())) {
                    return err(format!("{p}.box.padding"), "must not be negative");
                }
            }
        }
        Rule { .. } => {}
        Image { png, extent, .. } => {
            crate::image::png_size(png).map_err(|m| SpecError::new(format!("{p}.png"), m))?;
            if let Some(k) = extent.iter().position(|v| !v.is_finite()) {
                return err(format!("{p}.extent[{k}]"), "must be a finite number");
            }
            if extent[0] == extent[1] || extent[2] == extent[3] {
                return err(
                    format!("{p}.extent"),
                    "the edges of each side must differ, the image needs an area",
                );
            }
        }
        Contour {
            x,
            y,
            values,
            levels,
            filled,
            scale,
            labels,
            label_size,
        } => {
            for (name, v) in [("x", x), ("y", y)] {
                if v.len() < 2 {
                    return err(format!("{p}.{name}"), "needs at least 2 values");
                }
                if let Some(k) = v.iter().position(|e| !e.is_finite()) {
                    return err(format!("{p}.{name}[{k}]"), "must be a finite number");
                }
                let rising = v.windows(2).all(|w| w[0] < w[1]);
                let falling = v.windows(2).all(|w| w[0] > w[1]);
                if !(rising || falling) {
                    return err(
                        format!("{p}.{name}"),
                        "must be strictly increasing or strictly decreasing",
                    );
                }
            }
            same_len(p, "values", values.len(), "y", y.len())?;
            for (i, r) in values.iter().enumerate() {
                same_len(&format!("{p}.values[{i}]"), "", r.len(), "x", x.len())
                    .map_err(|e| SpecError::new(format!("{p}.values[{i}]"), e.message))?;
            }
            if levels.is_empty() || (*filled && levels.len() < 2) {
                return err(
                    format!("{p}.levels"),
                    if *filled {
                        "filled bands need at least 2 levels"
                    } else {
                        "needs at least 1 level"
                    },
                );
            }
            if let Some(k) = levels.iter().position(|v| !v.is_finite()) {
                return err(format!("{p}.levels[{k}]"), "must be a finite number");
            }
            if !levels.windows(2).all(|w| w[0] < w[1]) {
                return err(format!("{p}.levels"), "must be strictly increasing");
            }
            if let Some(s) = scale {
                check_scale(&format!("{p}.scale"), s)?;
            }
            if let Some(l) = labels {
                if *filled {
                    return err(format!("{p}.labels"), "filled bands have no labels");
                }
                same_len(p, "labels", l.len(), "levels", levels.len())?;
            }
            if label_size.is_some_and(|v| !(v > 0.0 && v.is_finite())) {
                return err(format!("{p}.label_size"), "must be positive");
            }
        }
        Arrow {
            x,
            y,
            x_from,
            y_from,
            head_length,
            head_width,
            body,
            shrink,
            size,
            ..
        } => {
            for (name, v) in [("x", x), ("y", y), ("x_from", x_from), ("y_from", y_from)] {
                if !v.is_finite() {
                    return err(format!("{p}.{name}"), "must be a finite number");
                }
            }
            for (name, v) in [
                ("head_length", head_length),
                ("head_width", head_width),
                ("body", body),
            ] {
                if v.is_some_and(|v| !(v >= 0.0 && v.is_finite())) {
                    return err(
                        format!("{p}.{name}"),
                        "must be a number of pixels, not negative",
                    );
                }
            }
            if !(0.0..0.5).contains(shrink) {
                return err(format!("{p}.shrink"), "must be at least 0 and below 0.5");
            }
            if size.is_some_and(|v| !(v > 0.0 && v.is_finite())) {
                return err(format!("{p}.size"), "must be positive");
            }
        }
    }
    Ok(())
}

/// The font size of a legend (since 0.34.0): positive.
fn check_legend_size(path: &str, size: Option<f64>) -> Result<(), SpecError> {
    if size.is_some_and(|v| !(v > 0.0 && v.is_finite())) {
        return err(path.to_string(), "must be positive");
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
    for (name, v) in [("width", f.width), ("height", f.height)] {
        if !(v > 0.0) {
            return err(name.into(), "must be positive");
        }
        if v > MAX_SIZE {
            return err(
                name.into(),
                format!("must be at most {MAX_SIZE} pixels, got {v}"),
            );
        }
    }
    for (name, v) in [
        ("title", f.fonts.title),
        ("label", f.fonts.label),
        ("tick", f.fonts.tick),
        ("legend", f.fonts.legend),
        ("tick_y", f.fonts.y_tick()),
        ("plot_title", f.fonts.plot_title_size()),
        ("colorbar_label", f.fonts.colorbar_label_size()),
        ("colorbar_tick", f.fonts.colorbar_tick_size()),
    ] {
        if !(v > 0.0) {
            return err(format!("fonts.{name}"), "must be positive");
        }
    }
    for (name, v) in [("grid.rows", f.grid.rows), ("grid.columns", f.grid.columns)] {
        if v == 0 {
            return err(name.into(), "must be at least 1");
        }
        if v > MAX_GRID {
            return err(name.into(), format!("must be at most {MAX_GRID}, got {v}"));
        }
    }
    if f.grid.wspace < 0.0 {
        return err("grid.wspace".into(), "must not be negative");
    }
    if f.grid.hspace < 0.0 {
        return err("grid.hspace".into(), "must not be negative");
    }
    if f.legend.columns == Some(0) {
        return err("legend.columns".into(), "must be at least 1");
    }
    let mut listed = HashSet::new();
    for (k, l) in f.legends.iter().flatten().enumerate() {
        if l.columns == Some(0) {
            return err(format!("legends[{k}].columns"), "must be at least 1");
        }
        check_legend_size(&format!("legends[{k}].size"), l.size)?;
        if let Some(c) = l.column {
            if l.position == FigureLegendPosition::Right {
                return err(
                    format!("legends[{k}].column"),
                    "only a legend above or below the grid is centred on a column",
                );
            }
            if c >= f.grid.columns {
                return err(
                    format!("legends[{k}].column"),
                    format!(
                        "column {c} is outside the grid of {} columns",
                        f.grid.columns
                    ),
                );
            }
        }
        for (j, g) in l.groups.iter().enumerate() {
            if !listed.insert(g.as_str()) {
                return err(
                    format!("legends[{k}].groups[{j}]"),
                    format!("`{g}` is in a legend of the figure already"),
                );
            }
        }
    }
    if let Some(t) = &f.row_titles {
        same_len("", "row_titles", t.len(), "grid.rows", f.grid.rows as usize)
            .map_err(|e| SpecError::new("row_titles", e.message))?;
    }
    // one bar goes above a row, one below it
    let mut top_rows: BTreeMap<u32, String> = BTreeMap::new();
    let mut bottom_rows: BTreeMap<u32, String> = BTreeMap::new();
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
        if let Some(inset) = &p.inset {
            let ip = format!("{pp}.inset");
            let host = f.plots[..i].iter().find(|h| h.id == inset.plot);
            let Some(host) = host else {
                return err(
                    format!("{ip}.plot"),
                    format!(
                        "`{}` is no plot before this one, the host of an inset comes before it",
                        inset.plot
                    ),
                );
            };
            if host.inset.is_some() {
                return err(format!("{ip}.plot"), "an inset is no host of an inset");
            }
            if p.row != host.row || p.column != host.column || p.row_span != 1 || p.column_span != 1
            {
                return err(
                    format!("{pp}.row"),
                    "an inset has the row and the column of its host and spans 1",
                );
            }
            for (name, v) in [
                ("x", inset.x),
                ("y", inset.y),
                ("width", inset.width),
                ("height", inset.height),
            ] {
                if !v.is_finite() {
                    return err(format!("{ip}.{name}"), "must be a finite number");
                }
            }
            if !(inset.width > 0.0 && inset.height > 0.0) {
                return err(format!("{ip}.width"), "width and height must be positive");
            }
            if p.color_bars().iter().any(|b| {
                matches!(
                    b.placement,
                    ColorBarPlacement::Top | ColorBarPlacement::Bottom
                )
            }) {
                return err(
                    format!("{pp}.colorbar"),
                    "an inset has no bar above or below a row",
                );
            }
        }
        for (k, b) in p.color_bars().iter().enumerate() {
            let bp = if p.colorbar.is_some() {
                format!("{pp}.colorbar")
            } else {
                format!("{pp}.colorbars[{k}]")
            };
            check_colorbar(&bp, b)?;
            if let Some(r) = b.rows
                && p.row + r > f.grid.rows
            {
                return err(
                    format!("{bp}.rows"),
                    format!(
                        "{r} rows from row {} leave the grid of {} rows",
                        p.row, f.grid.rows
                    ),
                );
            }
        }
        if let Some(fr) = &p.frame {
            check_color(format!("{pp}.frame.color"), &fr.color)?;
            if !(fr.width >= 0.0 && fr.width.is_finite()) {
                return err(format!("{pp}.frame.width"), "must not be negative");
            }
        }
        if let Some(Aspect::Ratio(r)) = p.aspect
            && !(r > 0.0 && r.is_finite())
        {
            return err(
                format!("{pp}.aspect"),
                "must be `equal` or a positive number",
            );
        }
        check_axis(&format!("{pp}.x"), &p.x)?;
        check_axis(&format!("{pp}.y"), &p.y)?;
        if let Some(a) = &p.y2 {
            check_axis(&format!("{pp}.y2"), a)?;
        }
        if let Some(a) = &p.x2 {
            check_axis(&format!("{pp}.x2"), a)?;
            if a.scale == AxisScale::Category {
                return err(
                    format!("{pp}.x2.scale"),
                    "a top axis is linear or log, its labels the tick_labels of its ticks",
                );
            }
        }
        if let Some(c) = &p.colorbar {
            check_colorbar(&format!("{pp}.colorbar"), c)?;
        }
        if let Some(bars) = &p.colorbars {
            if p.colorbar.is_some() {
                return err(
                    format!("{pp}.colorbars"),
                    "refused together with colorbar, which is one of them",
                );
            }
            for (k, c) in bars.iter().enumerate() {
                check_colorbar(&format!("{pp}.colorbars[{k}]"), c)?;
            }
        }
        for (k, c) in p.color_bars().iter().enumerate() {
            let (rows, side, row) = match c.placement {
                ColorBarPlacement::Top => (&mut top_rows, "above", p.row),
                ColorBarPlacement::Bottom => (&mut bottom_rows, "below", p.row + p.row_span - 1),
                _ => continue,
            };
            let here = if p.colorbar.is_some() {
                format!("{pp}.colorbar.placement")
            } else {
                format!("{pp}.colorbars[{k}].placement")
            };
            if let Some(first) = rows.insert(row, here.clone()) {
                return err(
                    here,
                    format!("row {row} has a bar {side} it already, {first}"),
                );
            }
        }
        if p.legend_columns == Some(0) {
            return err(format!("{pp}.legend_columns"), "must be at least 1");
        }
        check_legend_size(&format!("{pp}.legend_size"), p.legend_size)?;
        if let Some([x, y]) = p.legend_anchor
            && !(x.is_finite() && y.is_finite())
        {
            return err(format!("{pp}.legend_anchor"), "must be two finite numbers");
        }
        let mut listed = HashSet::new();
        for (k, l) in p.legends.iter().flatten().enumerate() {
            let lp = format!("{pp}.legends[{k}]");
            if l.columns == Some(0) {
                return err(format!("{lp}.columns"), "must be at least 1");
            }
            check_legend_size(&format!("{lp}.size"), l.size)?;
            if let Some([x, y]) = l.anchor
                && !(x.is_finite() && y.is_finite())
            {
                return err(format!("{lp}.anchor"), "must be two finite numbers");
            }
            for (j, g) in l.groups.iter().enumerate() {
                if !listed.insert(g.as_str()) {
                    return err(
                        format!("{lp}.groups[{j}]"),
                        format!("`{g}` is in a legend of the plot already"),
                    );
                }
            }
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
            check_style(&format!("{ep}.style"), &e.style)?;
            check_kind(&ep, &e.kind)?;
            if e.z.is_some_and(|z| !z.is_finite()) {
                return err(format!("{ep}.z"), "must be a finite number");
            }
            let placeable = matches!(
                e.kind,
                ElementKind::Curve { .. }
                    | ElementKind::Polygon { .. }
                    | ElementKind::Text { .. }
                    | ElementKind::Arrow { .. }
            );
            if e.coordinates != Coordinates::Data && !placeable {
                return err(
                    format!("{ep}.coordinates"),
                    "only a curve, a polygon, a text and an arrow take fractions of the panel",
                );
            }
            if !e.clip && !placeable {
                return err(
                    format!("{ep}.clip"),
                    "only a curve, a polygon, a text and an arrow are drawn unclipped",
                );
            }
        }
        if let Some(swatches) = &p.legend_swatches {
            for (group, style) in swatches {
                let sp = format!("{pp}.legend_swatches.{group}");
                if !p.elements.iter().any(|e| e.legend_group() == Some(group)) {
                    return err(sp, format!("`{group}` is no legend group of the plot"));
                }
                check_style(&sp, style)?;
            }
        }
    }
    Ok(())
}

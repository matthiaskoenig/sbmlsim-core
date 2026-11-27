//! The layout of a figure: where its title, the panels of its plots and its legends go, in the
//! pixels of the figure.
//!
//! Every plot is prepared first: its axes are resolved and the room around its panel is
//! measured, the tick labels and titles with the Inter widths and the bands Charton reserves
//! for its axes and the plot title by a chart of margins 0 (decision (a) of the spike). The
//! grid then aligns the panels: the panels of a column start and end at the same x, the panels
//! of a row at the same y, every column gets the room of the widest left axis of the plots
//! which start in it and of the widest right side of the plots which end in it, and the space
//! between two panels is `wspace` (`hspace`) times a panel, grown where the axes between them
//! need more. A plot over several columns or rows spans their panels and the space between
//! them. The ticks are resolved twice, first for the panels of a plain division of the figure,
//! then for the panels this gives, and the second layout is the layout of the figure.
//!
//! A legend takes the room of the largest legend it may show (its declared legends, see
//! `legend::room`), so a page which draws other entries in another state keeps its plot and
//! figure; its frame is that room, the entries drawn now from its top.
//!
//! The legends follow the legend settings of the figure. Inside, a legend is placed at the
//! location of its plot (of a row: of its rightmost plot with a legend), its `legend_loc` or
//! the `loc` of the figure: a fixed location of matplotlib, or `best`, the one of matplotlib's
//! locations whose frame has the least badness of matplotlib over the data (`crate::ink`, since
//! 0.33.0; the least ink before), the first in their
//! order on a tie; a legend which does not fit into that panel goes outside instead, one which
//! fits stays inside however much data it covers, as in matplotlib. Outside, the legend of a
//! plot is right of its
//! panel and its right axis, and the column gets the room of its widest legend; the legend of
//! a row is right of the grid at the top of the panels of its row. The panels keep their size
//! and the figure grows by the room of the legends: right by their width, and down by the part
//! of a legend which reaches below the room of its last row, which that row gets before the
//! next one, so a tall legend never reaches into the row below it.
//!
//! Planning a figure resolves its axes twice and probes Charton's bands of every plot, a few
//! tenths of a millisecond. A caller which needs the geometry more than once (the geometry and
//! the pick of a figure, the pick on every pointer move of a page) plans the figure once with
//! [`plan`] and keeps the [`Plan`]: [`Plan::layout`] is the geometry, the pick reads the
//! `Layout` it is given, and `render::render_planned` draws the figure on the same plan, so the
//! drawing and the geometry come from one layout.

use crate::ink::Ink;
use crate::render::axis::{ResolvedAxis, Transform};
use crate::render::legend::{self, Entry, Shape};
use crate::render::{self, Prepared, Sides, heatmap};
use crate::spec::{
    AxisScale, ColorBarPlacement, Figure, FigureLegendPosition, Fonts, LegendLoc, LegendPosition,
    LegendScope, Plot,
};
use crate::text::extra_lines;

/// The padding at the edges of the figure and the least gap between the axes of two
/// neighbouring plots, in pixels.
pub(crate) const PAD: f64 = 8.0;
/// The gap between a plot and a legend outside of it, in pixels.
const LEGEND_GAP: f64 = 10.0;
/// The gap between a legend inside and the edges of its panel, in pixels.
const LEGEND_INSET: f64 = 6.0;
/// The least width and height of the panel of a plot, in pixels: of the span of the plot, not
/// of every column and row it spans (since 0.30.0).
const MIN_PANEL: f64 = 20.0;
/// The room an estimate of the panels gives the axes (left and right, top and bottom).
const ESTIMATE_ROOM: (f64, f64) = (80.0, 60.0);

/// A rectangle in the pixels of the figure.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Rect {
    /// The right edge.
    pub(crate) fn right(&self) -> f64 {
        self.x + self.width
    }

    /// The bottom edge.
    pub(crate) fn bottom(&self) -> f64 {
        self.y + self.height
    }
}

/// How an axis maps its values onto its side of the panel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScaleInfo {
    /// Linear, logarithmic or the positions of categories (a heatmap's rows and columns).
    pub kind: AxisScale,
    /// The value at the start of the axis (left, bottom), before a reversal.
    pub low: f64,
    /// The value at the end of the axis.
    pub high: f64,
    /// Whether the axis runs from `high` to `low`.
    pub reverse: bool,
}

impl ScaleInfo {
    /// The scale of a resolved axis.
    fn of(a: &ResolvedAxis) -> Self {
        ScaleInfo {
            kind: match a.transform {
                Transform::Linear => AxisScale::Linear,
                Transform::Log => AxisScale::Log,
                Transform::Category => AxisScale::Category,
            },
            low: a.low,
            high: a.high,
            reverse: a.reverse,
        }
    }
}

/// The panel of a plot and its scales.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlotBox {
    /// The id of the plot.
    pub id: String,
    /// The panel: the rectangle the data is drawn in.
    pub rect: Rect,
    /// The x axis.
    pub x: ScaleInfo,
    /// The left y axis.
    pub y: ScaleInfo,
    /// The right y axis.
    pub y2: Option<ScaleInfo>,
}

/// Whose legend a legend is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LegendOwner {
    /// The legend of the plot of this id.
    Plot(String),
    /// The legend of a row of the grid.
    Row(u32),
    /// The legend of the figure below the grid (since 0.31.0).
    Figure,
}

/// One entry of a legend.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntryBox {
    /// The legend group of the entry.
    pub legend: String,
    /// The label.
    pub label: String,
    /// The rectangle of the entry, its swatch and its label.
    pub rect: Rect,
}

/// A legend: its frame and its entries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegendBox {
    /// Whose legend it is.
    pub owner: LegendOwner,
    /// The frame.
    pub rect: Rect,
    /// The entries, from the top.
    pub entries: Vec<EntryBox>,
}

/// The boxes of a figure, in the pixels of the figure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// The width of the figure: its width, grown by the legends outside.
    pub width: f64,
    /// The height of the figure: its height, grown by a legend outside taller than it.
    pub height: f64,
    /// The plots, in the order of the spec.
    pub plots: Vec<PlotBox>,
    /// The legends.
    pub legends: Vec<LegendBox>,
    /// The band of the title over the panels, `None` without a title.
    pub title: Option<Rect>,
}

/// A colour bar above or below a row: the plot and the bar of it, and the rectangle of the bar
/// itself.
pub(crate) type RowBar = (usize, usize, Rect);

/// A figure laid out, with what drawing it needs beyond the boxes.
pub(crate) struct Plan {
    /// The colour bars above the rows (since 0.31.0) and below them (since 0.34.0).
    pub row_bars: Vec<RowBar>,
    /// The bands of the titles of the rows with their titles (since 0.33.0).
    pub row_titles: Vec<(Rect, String)>,
    /// The title and the columns of every legend of the layout (since 0.31.0).
    pub shapes: Vec<Shape>,
    /// The boxes.
    pub layout: Layout,
    /// Every plot prepared, in the order of the spec.
    pub prepared: Vec<Prepared>,
    /// The canvas of every plot: its panel with the room of its column and row around it.
    pub canvases: Vec<Rect>,
    /// The entries of every legend of the layout.
    pub entries: Vec<Vec<Entry>>,
}

/// A legend at an anchor (since 0.34.0): the anchor in fractions of its panel, its location,
/// its room and its font size.
type AnchoredLegend = ([f64; 2], LegendLoc, (f64, f64), f64);

/// A legend before it is placed: its owner, where it goes, its entries and the room it takes.
type Pending = (LegendOwner, Place, Vec<Entry>, (f64, f64), Shape);

/// Where a legend goes.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// Inside the panel of the plot at this position, at this location (see [`loc_rect`]).
    Inside(usize, LegendLoc),
    /// Right of the plot at this position.
    RightOfPlot(usize),
    /// Right of the grid, at the top of this row.
    RightOfGrid(usize),
    /// Below the grid, centred on the figure (since 0.31.0) or on a column (since 0.34.0).
    Below(Option<u32>),
    /// Above the grid, centred on the figure or on a column (since 0.34.0).
    Above(Option<u32>),
    /// In or around the panel of the plot at this position, at an anchor in fractions of its
    /// panel by a location (since 0.33.0).
    Anchored(usize, [f64; 2], LegendLoc),
    /// Right of the grid at its top, a legend of the figure (since 0.33.0).
    FigureRight,
}

/// The extent of the panels along one direction of the grid.
struct Track {
    /// The size of one panel.
    size: f64,
    /// Where the panel of every column (row) starts.
    starts: Vec<f64>,
    /// The pixels the grid reaches beyond the stretch it fills, which the figure grows by: 0
    /// unless the panels of the plots need more than the stretch holds (see [`track`]).
    overflow: f64,
}

impl Track {
    /// The start and the length of the panels from `first` over `span` columns (rows).
    fn span(&self, first: u32, span: u32) -> (f64, f64) {
        let (a, b) = (first as usize, (first + span - 1) as usize);
        (self.starts[a], self.starts[b] + self.size - self.starts[a])
    }
}

/// Lays out one direction of the grid: `before` and `after` are the room of every column
/// (row) before and after its panel, `start` and `length` the stretch of the figure the grid
/// fills, `space` the space between two panels as a share of a panel and `spans` the columns
/// (rows) every plot spans.
///
/// The gap between two neighbouring panels is the larger of `space` times a panel and the room
/// of the axes between them with [`PAD`]; the size of a panel is the one which fills the
/// stretch. The stretch as a function of the size is increasing and piecewise linear, so the
/// size is found on the piece where the gaps which `space` decides are the ones below it.
///
/// The panel of every plot is at least [`MIN_PANEL`] long over its span, its cells and the
/// gaps between them: a grid of many thin cells fits its stretch as long as the plots span
/// enough of them. Only a grid whose plots cannot have that least panel in the stretch reaches
/// beyond it, by its `overflow`, which the figure grows by, so no plot is cut off.
fn track(
    start: f64,
    length: f64,
    before: &[f64],
    after: &[f64],
    space: f64,
    spans: &[u32],
) -> Track {
    let n = before.len();
    let needs: Vec<f64> = (0..n.saturating_sub(1))
        .map(|c| after[c] + before[c + 1] + PAD)
        .collect();
    let available = length - before[0] - after[n - 1];
    let mut sorted = needs.clone();
    sorted.sort_by(f64::total_cmp);
    let m = sorted.len();
    let space = space.max(0.0);
    let mut size = None;
    for k in 0..=m {
        let fixed: f64 = sorted[k..].iter().sum();
        let s = (available - fixed) / (n as f64 + k as f64 * space);
        let below = k == 0 || space * s >= sorted[k - 1];
        let above = k == m || space * s <= sorted[k];
        if below && above {
            size = Some(s);
            break;
        }
    }
    let total: f64 = sorted.iter().sum();
    // a span of k cells is k cells and k - 1 gaps of at least `space` cells long
    let least = spans
        .iter()
        .map(|k| {
            let k = f64::from((*k).max(1));
            MIN_PANEL / (k + (k - 1.0) * space)
        })
        .fold(0.0, f64::max);
    let size = size.unwrap_or((available - total) / n as f64).max(least);
    let mut starts = Vec::with_capacity(n);
    let mut at = start + before[0];
    starts.push(at);
    for need in &needs {
        at += size + (space * size).max(*need);
        starts.push(at);
    }
    // beyond the stretch by more than the rounding of the sums
    let beyond = at + size + after[n - 1] - start - length;
    Track {
        size,
        starts,
        overflow: if beyond > 1e-6 { beyond } else { 0.0 },
    }
}

/// The room of every column and row of the grid around its panels.
struct Room {
    /// Left of the panels of a column.
    left: Vec<f64>,
    /// Right of the panels of a column.
    right: Vec<f64>,
    /// Above the panels of a row.
    top: Vec<f64>,
    /// Below the panels of a row.
    bottom: Vec<f64>,
    /// The part of `bottom` the colour bar below a row takes, after the room of the axes of
    /// the row (since 0.34.0).
    bars_below: Vec<f64>,
}

/// The room of the columns and rows: the largest depth of the plots which start or end there,
/// above a row the room of the colour bar above it (since 0.31.0) and below a row the room of
/// the colour bar below it (since 0.34.0); `bars` are the rooms of the bars above and below the
/// row of every plot.
fn room(figure: &Figure, depths: &[Sides], bars: &[(f64, f64)]) -> Room {
    let (cols, rows) = (figure.grid.columns as usize, figure.grid.rows as usize);
    let mut r = Room {
        left: vec![0.0; cols],
        right: vec![0.0; cols],
        top: vec![0.0; rows],
        bottom: vec![0.0; rows],
        bars_below: vec![0.0; rows],
    };
    // an inset takes no room of the grid (since 0.33.0)
    for (p, d) in figure
        .plots
        .iter()
        .zip(depths)
        .filter(|(p, _)| p.inset.is_none())
    {
        let (c, row) = (p.column as usize, p.row as usize);
        let (c_end, r_end) = (
            c + p.column_span as usize - 1,
            row + p.row_span as usize - 1,
        );
        r.left[c] = r.left[c].max(d.left);
        r.right[c_end] = r.right[c_end].max(d.right);
        r.top[row] = r.top[row].max(d.top);
        r.bottom[r_end] = r.bottom[r_end].max(d.bottom);
    }
    let mut above = vec![0.0f64; rows];
    for (p, (top, bottom)) in figure
        .plots
        .iter()
        .zip(bars)
        .filter(|(p, _)| p.inset.is_none())
    {
        above[p.row as usize] = above[p.row as usize].max(*top);
        let end = (p.row + p.row_span - 1) as usize;
        r.bars_below[end] = r.bars_below[end].max(*bottom);
    }
    for (top, b) in r.top.iter_mut().zip(above) {
        *top += b;
    }
    for (bottom, b) in r.bottom.iter_mut().zip(&r.bars_below) {
        *bottom += b;
    }
    // the title of a row above everything of the row (since 0.33.0)
    for (top, t) in r.top.iter_mut().zip(figure.row_titles.iter().flatten()) {
        *top += row_title_band(figure, t);
    }
    r
}

/// The height of the band of the title of a row (since 0.33.0): one and a half plot title
/// sizes and the lines of a title of several lines after its first, 0 without a title.
fn row_title_band(figure: &Figure, title: &str) -> f64 {
    if title.is_empty() {
        return 0.0;
    }
    let size = figure.fonts.plot_title_size();
    1.5 * size + extra_lines(title, size)
}

/// The height of the title band of a figure: twice the title font and the lines of a title
/// of several lines after its first, 0 without a title.
fn title_band(figure: &Figure) -> f64 {
    match &figure.title {
        Some(t) => 2.0 * figure.fonts.title + extra_lines(t, figure.fonts.title),
        None => 0.0,
    }
}

/// The panels of the plots for the depths of their plots, with the room of the grid and the
/// pixels the grid reaches beyond the width and the height of the figure; `above` is the room
/// of the legends of the figure above the grid (since 0.34.0), which the figure grows by.
fn grid(
    figure: &Figure,
    depths: &[Sides],
    bars: &[(f64, f64)],
    above: f64,
) -> (Vec<Rect>, Room, (f64, f64)) {
    let band = title_band(figure);
    let r = room(figure, depths, bars);
    let grid_plots = || figure.plots.iter().filter(|p| p.inset.is_none());
    let column_spans: Vec<u32> = grid_plots().map(|p| p.column_span).collect();
    let row_spans: Vec<u32> = grid_plots().map(|p| p.row_span).collect();
    let columns = track(
        PAD,
        figure.width - 2.0 * PAD,
        &r.left,
        &r.right,
        figure.grid.wspace,
        &column_spans,
    );
    let rows = track(
        band + above + PAD,
        figure.height - band - 2.0 * PAD,
        &r.top,
        &r.bottom,
        figure.grid.hspace,
        &row_spans,
    );
    let mut panels: Vec<Rect> = figure
        .plots
        .iter()
        .map(|p| {
            let (x, width) = columns.span(p.column, p.column_span);
            let (y, height) = rows.span(p.row, p.row_span);
            Rect {
                x,
                y,
                width,
                height,
            }
        })
        .collect();
    place_insets(figure, &mut panels);
    (panels, r, (columns.overflow, rows.overflow))
}

/// The panel of an inset (since 0.33.0): its rectangle of fractions of the panel of its host.
fn inset_panel(inset: &crate::spec::Inset, host: Rect) -> Rect {
    Rect {
        x: host.x + inset.x * host.width,
        y: host.y + (1.0 - inset.y - inset.height) * host.height,
        width: inset.width * host.width,
        height: inset.height * host.height,
    }
}

/// Puts the panel of every inset into the panel of its host.
fn place_insets(figure: &Figure, panels: &mut [Rect]) {
    for (i, p) in figure.plots.iter().enumerate() {
        let Some(inset) = &p.inset else { continue };
        if let Some(h) = figure.plots.iter().position(|q| q.id == inset.plot) {
            panels[i] = inset_panel(inset, panels[h]);
        }
    }
}

/// The panels of a plain division of the figure into its grid, for the first resolution of the
/// ticks.
fn estimate(figure: &Figure) -> Vec<(f64, f64)> {
    let band = title_band(figure);
    let cw = (figure.width - 2.0 * PAD) / f64::from(figure.grid.columns);
    let ch = (figure.height - band - 2.0 * PAD) / f64::from(figure.grid.rows);
    let mut sizes: Vec<(f64, f64)> = figure
        .plots
        .iter()
        .map(|p| {
            let right = if p.y2.is_some() { ESTIMATE_ROOM.0 } else { 0.0 };
            let top = match &p.title {
                Some(t) => {
                    2.0 * figure.fonts.plot_title_size()
                        + extra_lines(t, figure.fonts.plot_title_size())
                }
                None => 0.0,
            };
            (
                (f64::from(p.column_span) * cw - ESTIMATE_ROOM.0 - right).max(MIN_PANEL),
                (f64::from(p.row_span) * ch - ESTIMATE_ROOM.1 - top).max(MIN_PANEL),
            )
        })
        .collect();
    // an inset its share of the panel of its host
    for (i, p) in figure.plots.iter().enumerate() {
        let Some(inset) = &p.inset else { continue };
        if let Some(h) = figure.plots.iter().position(|q| q.id == inset.plot) {
            let (w, hgt) = sizes[h];
            sizes[i] = (
                (inset.width * w).max(MIN_PANEL),
                (inset.height * hgt).max(MIN_PANEL),
            );
        }
    }
    sizes
}

/// Rounds a coordinate to the two decimals of the SVG, so a nested `<svg>` starts where it is
/// written.
pub(crate) fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Lays a figure out.
pub(crate) fn plan(figure: &Figure) -> Plan {
    let fonts = &figure.fonts;
    let prepare_all = |sizes: &[(f64, f64)]| -> Vec<Prepared> {
        figure
            .plots
            .iter()
            .zip(sizes)
            .map(|(p, size)| render::prepare(figure, p, *size))
            .collect()
    };
    let bars_of = |prepared: &[Prepared]| -> Vec<(f64, f64)> {
        prepared.iter().map(|p| (p.top_bar, p.bottom_bar)).collect()
    };
    let estimated = estimate(figure);
    let first = prepare_all(&estimated);

    // the entries of the legends, of the colours of the elements, which do not depend on the
    // panels
    let mut plot_entries: Vec<Vec<Entry>> = figure
        .plots
        .iter()
        .zip(&first)
        .map(|(p, prep)| {
            if p.legend {
                legend::plot_entries(p, &prep.colors, fonts)
            } else {
                Vec::new()
            }
        })
        .collect();
    // the legends of the figure take their groups out of the legends of the plots, the first
    // entry of a group of all plots (since 0.33.0)
    let mut figure_lists: Vec<Vec<Entry>> = Vec::new();
    for fl in figure.legends.iter().flatten() {
        let mut list = Vec::new();
        for g in &fl.groups {
            let mut found = None;
            for entries in plot_entries.iter_mut() {
                while let Some(k) = entries.iter().position(|e| &e.legend == g) {
                    let e = entries.remove(k);
                    found.get_or_insert(e);
                }
            }
            list.extend(found);
        }
        figure_lists.push(list);
    }
    // the further legends of a plot take their groups out of its legend (since 0.33.0)
    let mut extra_lists: Vec<Vec<Vec<Entry>>> = Vec::with_capacity(figure.plots.len());
    for (p, entries) in figure.plots.iter().zip(plot_entries.iter_mut()) {
        let mut lists = Vec::new();
        for pl in p.legends.iter().flatten() {
            let mut list = Vec::new();
            for g in &pl.groups {
                if let Some(k) = entries.iter().position(|e| &e.legend == g) {
                    list.push(entries.remove(k));
                }
            }
            lists.push(list);
        }
        extra_lists.push(lists);
    }
    let plot_shape = |p: &Plot| Shape {
        title: p.legend_title.clone(),
        columns: p.legend_columns.unwrap_or(1).max(1) as usize,
        family: p.legend_family.unwrap_or(fonts.legend_family),
        size: p.legend_size.unwrap_or(fonts.legend),
        weight: p.legend_weight.unwrap_or(fonts.legend_weight),
        title_weight: fonts.legend_weight,
        frame: p.legend_frame,
    };
    let extra_shape = |pl: &crate::spec::PlotLegend| Shape {
        title: pl.title.clone(),
        columns: pl.columns.unwrap_or(1).max(1) as usize,
        family: pl.family.unwrap_or(fonts.legend_family),
        size: pl.size.unwrap_or(fonts.legend),
        weight: pl.weight.unwrap_or(fonts.legend_weight),
        title_weight: fonts.legend_weight,
        frame: pl.frame,
    };
    // the legends of the figure with their shapes and rooms, the columns of one below or
    // above the grid as many as fit into the width of the figure without columns given
    let figure_legends: Vec<(Shape, Option<(f64, f64)>)> = figure
        .legends
        .iter()
        .flatten()
        .zip(&figure_lists)
        .map(|(fl, list)| {
            let base = Shape {
                title: fl.title.clone(),
                family: fl.family.unwrap_or(fonts.legend_family),
                size: fl.size.unwrap_or(fonts.legend),
                weight: fl.weight.unwrap_or(fonts.legend_weight),
                frame: fl.frame,
                ..Shape::of(fonts)
            };
            let columns = match (fl.columns, fl.position) {
                (Some(c), _) => c as usize,
                (None, FigureLegendPosition::Right) => 1,
                (None, FigureLegendPosition::Bottom | FigureLegendPosition::Top) => {
                    fitting_columns(list, &base, figure.width)
                }
            };
            let shape = Shape { columns, ..base };
            let size = legend::room(list, &[], &shape);
            (shape, size)
        })
        .collect();
    // the room of the legends of the figure above the grid (since 0.34.0)
    let above: Vec<(Option<u32>, (f64, f64))> = figure
        .legends
        .iter()
        .flatten()
        .zip(&figure_legends)
        .filter(|(fl, _)| fl.position == FigureLegendPosition::Top)
        .filter_map(|(fl, (_, size))| Some((fl.column, (*size)?)))
        .collect();
    let above_room = stack_extent(&above);

    // the legends at an anchor beyond their panel reserve the room of their part beyond it
    // (since 0.34.0): every anchored legend of a plot with its anchor, location and room
    let scope_plot = figure.legend.scope == LegendScope::Plot
        && figure.legend.position != LegendPosition::Bottom;
    let anchored: Vec<Vec<AnchoredLegend>> = figure
        .plots
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut out = Vec::new();
            if scope_plot && let Some(at) = p.legend_anchor {
                let shape = plot_shape(p);
                if let Some(size) = legend::room(&plot_entries[i], &declared(p), &shape) {
                    let loc = p.legend_loc.unwrap_or(LegendLoc::UpperRight);
                    out.push((at, loc, size, shape.size));
                }
            }
            for (pl, list) in p.legends.iter().flatten().zip(&extra_lists[i]) {
                let shape = extra_shape(pl);
                if let (Some(at), Some(size)) = (pl.anchor, legend::room(list, &[], &shape)) {
                    let loc = pl.loc.unwrap_or(LegendLoc::Best);
                    out.push((at, loc, size, shape.size));
                }
            }
            out
        })
        .collect();
    let depths_of = |prepared: &[Prepared], sizes: &[(f64, f64)]| -> Vec<Sides> {
        prepared
            .iter()
            .zip(sizes)
            .zip(&anchored)
            .map(|((prep, size), legends)| {
                let mut d = prep.depth;
                for (at, loc, room, font) in legends {
                    let beyond = beyond_panel(*size, *at, *room, *loc, *font);
                    d.left = d.left.max(beyond.left);
                    d.right = d.right.max(beyond.right);
                    d.top = d.top.max(beyond.top);
                    d.bottom = d.bottom.max(beyond.bottom);
                }
                d
            })
            .collect()
    };
    let (panels, _, _) = grid(
        figure,
        &depths_of(&first, &estimated),
        &bars_of(&first),
        above_room,
    );
    let sizes: Vec<(f64, f64)> = panels.iter().map(|r| (r.width, r.height)).collect();
    let mut prepared = prepare_all(&sizes);
    let (mut panels, room, (grid_wider, grid_taller)) = grid(
        figure,
        &depths_of(&prepared, &sizes),
        &bars_of(&prepared),
        above_room,
    );

    let family = figure.fonts.legend_family;
    let end_column = |p: &Plot| (p.column + p.column_span - 1) as usize;
    let end_row = |p: &Plot| (p.row + p.row_span - 1) as usize;
    let fits = |panel: &Rect, (w, h): (f64, f64)| {
        w + 2.0 * LEGEND_INSET <= panel.width && h + 2.0 * LEGEND_INSET <= panel.height
    };
    let inside = figure.legend.position == LegendPosition::Inside;
    // the location of a legend inside the panel of the plot at `i`: the `legend_loc` of the
    // plot or the `loc` of the figure, `best` resolved by the data; none when it does not fit
    // into the panel
    let loc_of = |i: usize, size: (f64, f64)| -> Option<LegendLoc> {
        let plot = &figure.plots[i];
        // the legend of an inset stays in its panel
        if plot.inset.is_none() && !(inside && fits(&panels[i], size)) {
            return None;
        }
        match plot.legend_loc.unwrap_or(figure.legend.loc) {
            LegendLoc::Best => {
                let panel = (panels[i].width, panels[i].height);
                Some(best_loc(figure, i, &prepared[i], panel, size))
            }
            loc => Some(loc),
        }
    };
    // every legend with where it goes and the room it takes (the largest of the legends it may
    // show, `legend::room`); a legend inside which does not fit into its panel goes outside
    // instead
    let mut pending: Vec<Pending> = Vec::new();
    let below = figure.legend.position == LegendPosition::Bottom;
    match figure.legend.scope {
        // one legend of the figure below the grid, of the entries of all plots
        _ if below => {
            let orders: Vec<&[String]> = figure
                .plots
                .iter()
                .filter_map(|p| p.legend_order.as_deref())
                .collect();
            let all_declared: Vec<&[String]> = figure.plots.iter().flat_map(declared).collect();
            let list = legend::merge_ordered(plot_entries, &orders);
            let base = Shape {
                title: figure.legend.title.clone(),
                ..Shape::of(fonts)
            };
            // the columns given, else as many as fit into the width of the figure
            let columns = match figure.legend.columns {
                Some(c) => c as usize,
                None => fitting_columns(&list, &base, figure.width),
            };
            let shape = Shape { columns, ..base };
            if let Some(size) = legend::room(&list, &all_declared, &shape) {
                pending.push((LegendOwner::Figure, Place::Below(None), list, size, shape));
            }
        }
        LegendScope::Plot => {
            for (i, list) in plot_entries.into_iter().enumerate() {
                let shape = plot_shape(&figure.plots[i]);
                let Some(size) = legend::room(&list, &declared(&figure.plots[i]), &shape) else {
                    continue;
                };
                let place = match (figure.plots[i].legend_anchor, loc_of(i, size)) {
                    // a legend at an anchor (since 0.33.0)
                    (Some(at), _) => Place::Anchored(
                        i,
                        at,
                        figure.plots[i].legend_loc.unwrap_or(LegendLoc::UpperRight),
                    ),
                    (None, Some(loc)) => Place::Inside(i, loc),
                    (None, None) => Place::RightOfPlot(i),
                };
                pending.push((
                    LegendOwner::Plot(figure.plots[i].id.clone()),
                    place,
                    list,
                    size,
                    shape,
                ));
            }
        }
        LegendScope::Row => {
            let rows = figure.grid.rows as usize;
            let mut lists_of_row: Vec<Vec<Vec<Entry>>> = vec![Vec::new(); rows];
            let mut declared_of_row: Vec<Vec<&[String]>> = vec![Vec::new(); rows];
            let mut inset_lists = Vec::new();
            for (i, (p, list)) in figure.plots.iter().zip(plot_entries).enumerate() {
                // an inset keeps its own legend in its panel
                if p.inset.is_some() {
                    inset_lists.push((i, list));
                    continue;
                }
                lists_of_row[p.row as usize].push(list);
                declared_of_row[p.row as usize].extend(declared(p));
            }
            for (i, list) in inset_lists {
                let shape = plot_shape(&figure.plots[i]);
                if let Some(size) = legend::room(&list, &declared(&figure.plots[i]), &shape)
                    && let Some(loc) = loc_of(i, size)
                {
                    pending.push((
                        LegendOwner::Plot(figure.plots[i].id.clone()),
                        Place::Inside(i, loc),
                        list,
                        size,
                        shape,
                    ));
                }
            }
            for (r, lists) in lists_of_row.into_iter().enumerate() {
                let orders: Vec<&[String]> = figure
                    .plots
                    .iter()
                    .filter(|p| p.row as usize == r && p.inset.is_none())
                    .filter_map(|p| p.legend_order.as_deref())
                    .collect();
                let list = legend::merge_ordered(lists, &orders);
                // the first title, columns and font of the plots of the row, no frame when a
                // plot of the row has none
                let of_row = || {
                    figure
                        .plots
                        .iter()
                        .filter(|p| p.row as usize == r && p.inset.is_none())
                };
                let shape = Shape {
                    title: of_row().find_map(|p| p.legend_title.clone()),
                    columns: of_row().find_map(|p| p.legend_columns).unwrap_or(1).max(1) as usize,
                    family: of_row().find_map(|p| p.legend_family).unwrap_or(family),
                    size: of_row().find_map(|p| p.legend_size).unwrap_or(fonts.legend),
                    weight: of_row()
                        .find_map(|p| p.legend_weight)
                        .unwrap_or(fonts.legend_weight),
                    title_weight: fonts.legend_weight,
                    frame: of_row().all(|p| p.legend_frame),
                };
                let Some(size) = legend::room(&list, &declared_of_row[r], &shape) else {
                    continue;
                };
                // the rightmost plot of the row with a legend holds a legend inside
                let host = figure
                    .plots
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.row as usize == r && p.legend && p.inset.is_none())
                    .max_by_key(|(i, p)| (p.column + p.column_span, usize::MAX - i))
                    .map(|(i, _)| i);
                let place = match host.and_then(|i| Some((i, loc_of(i, size)?))) {
                    Some((i, loc)) => Place::Inside(i, loc),
                    None => Place::RightOfGrid(r),
                };
                pending.push((LegendOwner::Row(r as u32), place, list, size, shape));
            }
        }
    }

    // the further legends of the plots in their panels (since 0.33.0), at their location or
    // anchor, `best` avoiding the legends of the panel before them
    for (i, lists) in extra_lists.into_iter().enumerate() {
        let p = &figure.plots[i];
        for (pl, list) in p.legends.iter().flatten().zip(lists) {
            let shape = extra_shape(pl);
            let Some(size) = legend::room(&list, &[], &shape) else {
                continue;
            };
            let place = match (pl.anchor, pl.loc.unwrap_or(LegendLoc::Best)) {
                (Some(at), loc) => Place::Anchored(i, at, loc),
                (None, LegendLoc::Best) => {
                    let origin = Rect {
                        width: panels[i].width,
                        height: panels[i].height,
                        ..Rect::default()
                    };
                    // the legends of the panel placed so far
                    let taken: Vec<Rect> = pending
                        .iter()
                        .filter_map(|(_, place, _, sz, sh)| match place {
                            Place::Inside(j, loc) if *j == i => Some(loc_rect(origin, *sz, *loc)),
                            Place::Anchored(j, at, loc) if *j == i => {
                                Some(anchor_rect(origin, *at, *sz, *loc, sh.size))
                            }
                            _ => None,
                        })
                        .collect();
                    let panel = (panels[i].width, panels[i].height);
                    Place::Inside(
                        i,
                        best_loc_avoiding(figure, i, &prepared[i], panel, size, &taken),
                    )
                }
                (None, loc) => Place::Inside(i, loc),
            };
            pending.push((LegendOwner::Plot(p.id.clone()), place, list, size, shape));
        }
    }
    // the legends of the figure (since 0.33.0)
    for ((fl, list), (shape, size)) in figure
        .legends
        .iter()
        .flatten()
        .zip(figure_lists)
        .zip(figure_legends)
    {
        let Some(size) = size else {
            continue;
        };
        let place = match fl.position {
            FigureLegendPosition::Right => Place::FigureRight,
            FigureLegendPosition::Bottom => Place::Below(fl.column),
            FigureLegendPosition::Top => Place::Above(fl.column),
        };
        pending.push((LegendOwner::Figure, place, list, size, shape));
    }

    // the legends right of a plot: the column gets the room of its widest one, the columns
    // right of it move by it
    let columns = figure.grid.columns as usize;
    let mut extra = vec![0.0f64; columns];
    for (_, place, _, size, _) in &pending {
        if let Place::RightOfPlot(i) = place {
            let c = end_column(&figure.plots[*i]);
            extra[c] = extra[c].max(LEGEND_GAP + size.0);
        }
    }
    let shift: Vec<f64> = (0..columns)
        .map(|c| extra[..c].iter().sum::<f64>())
        .collect();
    for (p, r) in figure.plots.iter().zip(panels.iter_mut()) {
        let (a, b) = (shift[p.column as usize], shift[end_column(p)]);
        r.x += a;
        r.width += b - a;
    }
    place_insets(figure, &mut panels);
    let mut width = figure.width + grid_wider + extra.iter().sum::<f64>();
    // the legends right of the grid
    let grid_right = figure
        .plots
        .iter()
        .zip(&panels)
        .filter(|(p, _)| p.inset.is_none())
        .map(|(p, r)| r.right() + room.right[end_column(p)])
        .fold(0.0, f64::max);
    let widest = pending
        .iter()
        .filter(|(_, place, _, _, _)| matches!(place, Place::RightOfGrid(_)))
        .map(|(_, _, _, size, _)| size.0)
        .fold(0.0, f64::max);
    if widest > 0.0 {
        width += LEGEND_GAP + widest;
    }
    // the legends of the figure right of the grid, right of the legends of its rows
    let figure_right_x = grid_right
        + if widest > 0.0 {
            LEGEND_GAP + widest
        } else {
            0.0
        }
        + LEGEND_GAP;
    let widest_figure = pending
        .iter()
        .filter(|(_, place, _, _, _)| matches!(place, Place::FigureRight))
        .map(|(_, _, _, size, _)| size.0)
        .fold(0.0, f64::max);
    if widest_figure > 0.0 {
        width += LEGEND_GAP + widest_figure;
    }
    let grid_top = figure
        .plots
        .iter()
        .zip(&panels)
        .filter(|(p, _)| p.inset.is_none())
        .map(|(_, r)| r.y)
        .fold(f64::INFINITY, f64::min);
    // the legends outside, at the top of their plot or row, and the last row of the grid they
    // reach down to
    let mut outside: Vec<Option<(Rect, usize, usize)>> = pending
        .iter()
        .map(|(_, place, _, size, _)| {
            let (w, h) = *size;
            match place {
                Place::Inside(..)
                | Place::Below(_)
                | Place::Above(_)
                | Place::Anchored(..)
                | Place::FigureRight => None,
                Place::RightOfPlot(i) => {
                    let p = &figure.plots[*i];
                    let rect = Rect {
                        x: panels[*i].right() + room.right[end_column(p)] + LEGEND_GAP,
                        y: panels[*i].y,
                        width: w,
                        height: h,
                    };
                    Some((rect, p.row as usize, end_row(p)))
                }
                Place::RightOfGrid(r) => {
                    let of_row = || {
                        figure
                            .plots
                            .iter()
                            .zip(&panels)
                            .filter(|(p, _)| p.row as usize == *r && p.inset.is_none())
                    };
                    let top = of_row()
                        .map(|(_, rect)| rect.y)
                        .fold(f64::INFINITY, f64::min);
                    let last = of_row().map(|(p, _)| end_row(p)).max().unwrap_or(*r);
                    let rect = Rect {
                        x: grid_right + LEGEND_GAP,
                        y: top,
                        width: w,
                        height: h,
                    };
                    Some((rect, *r, last))
                }
            }
        })
        .collect();
    // a legend outside which reaches below the room of its last row gives that row the room
    // of its overflow: the rows below move down, the figure grows, the panels keep their size
    let rows = figure.grid.rows as usize;
    let mut row_bottom = vec![f64::NEG_INFINITY; rows];
    for (p, r) in figure.plots.iter().zip(&panels) {
        if p.inset.is_some() {
            continue;
        }
        let e = end_row(p);
        row_bottom[e] = row_bottom[e].max(r.bottom() + room.bottom[e]);
    }
    let mut overflow = vec![0.0f64; rows];
    for (rect, _, last) in outside.iter().flatten() {
        overflow[*last] = overflow[*last].max(rect.bottom() - row_bottom[*last]);
    }
    let down: Vec<f64> = (0..rows)
        .map(|r| overflow[..r].iter().sum::<f64>())
        .collect();
    for (p, r) in figure.plots.iter().zip(panels.iter_mut()) {
        let (a, b) = (down[p.row as usize], down[end_row(p)]);
        r.y += a;
        r.height += b - a;
    }
    place_insets(figure, &mut panels);
    for (rect, first, _) in outside.iter_mut().flatten() {
        // a legend moves with the first row of its plot or row
        rect.y += down[*first];
    }
    let mut height = figure.height + above_room + grid_taller + overflow.iter().sum::<f64>();
    let grid_bottom = figure
        .plots
        .iter()
        .zip(&panels)
        .filter(|(p, _)| p.inset.is_none())
        .map(|(p, r)| r.bottom() + room.bottom[end_row(p)])
        .fold(0.0, f64::max);
    // the middle of the panels of a column, for a legend of the figure under or over it (since
    // 0.34.0): of the plots which span that column alone, else of every plot over it
    let column_middle = |c: u32| -> f64 {
        let span = |alone: bool| {
            figure
                .plots
                .iter()
                .zip(&panels)
                .filter(|(p, _)| {
                    p.inset.is_none()
                        && if alone {
                            p.column == c && p.column_span == 1
                        } else {
                            p.column <= c && c < p.column + p.column_span
                        }
                })
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, r), (_, rect)| {
                    (l.min(rect.x), r.max(rect.right()))
                })
        };
        let (l, r) = match span(true) {
            (l, r) if l < r => (l, r),
            _ => span(false),
        };
        if l < r { (l + r) / 2.0 } else { width / 2.0 }
    };
    // the legends below the grid, centred on the figure or on a column and stacked: the
    // figure grows by them (since 0.31.0); the legends above the grid likewise into the room
    // the grid left them (since 0.34.0); the legends of the figure right of the grid stacked
    // from the top of the grid (since 0.33.0)
    let stacked = |above: bool| -> Vec<(Option<u32>, (f64, f64))> {
        pending
            .iter()
            .filter_map(|(_, place, _, size, _)| match place {
                Place::Below(c) if !above => Some((*c, *size)),
                Place::Above(c) if above => Some((*c, *size)),
                _ => None,
            })
            .collect()
    };
    let place_stack = |items: &[(Option<u32>, (f64, f64))], start: f64, centred_first: bool| {
        stack_rects(items, start, centred_first, &|c, w| {
            let middle = c.map_or(width / 2.0, column_middle);
            (middle - w / 2.0).clamp(0.0, (width - w).max(0.0))
        })
    };
    let below_rects = place_stack(&stacked(false), grid_bottom + LEGEND_GAP, false);
    if let Some(end) = below_rects.iter().map(Rect::bottom).reduce(f64::max) {
        height = height.max(end + PAD);
    }
    let above_rects = place_stack(&stacked(true), title_band(figure) + PAD, true);
    let mut right_rects = Vec::new();
    let mut right_y = grid_top;
    for (_, place, _, (w, h), _) in &pending {
        if let Place::FigureRight = place {
            let rect = Rect {
                x: figure_right_x,
                y: right_y,
                width: *w,
                height: *h,
            };
            right_y = rect.bottom() + LEGEND_GAP;
            height = height.max(rect.bottom() + PAD);
            right_rects.push(rect);
        }
    }
    let (mut below_rects, mut above_rects, mut right_rects) = (
        below_rects.into_iter(),
        above_rects.into_iter(),
        right_rects.into_iter(),
    );
    // the legends inside, in the panels where they end up
    let mut legends = Vec::with_capacity(pending.len());
    let mut entries = Vec::with_capacity(pending.len());
    for ((owner, place, list, size, shape), out) in pending.into_iter().zip(outside) {
        // the frame is the room of the legend, which holds the largest legend it may show, so
        // nothing moves when entries come and go; a legend which draws nothing now is no frame
        let far = matches!(place, Place::Anchored(_, at, _) if beyond(at));
        let rect = match (place, out) {
            (_, Some((rect, _, _))) => rect,
            (Place::Inside(i, loc), None) => loc_rect(panels[i], size, loc),
            (Place::Anchored(i, at, loc), None) => {
                anchor_rect(panels[i], at, size, loc, shape.size)
            }
            (Place::Below(_), None) => match below_rects.next() {
                Some(r) => r,
                None => continue,
            },
            (Place::Above(_), None) => match above_rects.next() {
                Some(r) => r,
                None => continue,
            },
            (Place::FigureRight, None) => match right_rects.next() {
                Some(r) => r,
                None => continue,
            },
            (Place::RightOfPlot(_) | Place::RightOfGrid(_), None) => continue,
        };
        if list.is_empty() {
            continue;
        }
        // a legend anchored beyond its panel which reaches beyond the figure grows it, so
        // nothing is cut off (since 0.34.0)
        if far {
            width = width.max(rect.right() + PAD);
            height = height.max(rect.bottom() + PAD);
        }
        legends.push((owner, rect, list, shape));
    }

    // a plot with an aspect has the limits of the panel it ends up with: one whose panel is
    // not the one it was prepared for (the room of the axes changed after the first
    // preparation, or a legend outside widened a column of its span) is prepared again for its
    // panel, in the room the layout gave it
    for ((p, r), prep) in figure.plots.iter().zip(&panels).zip(prepared.iter_mut()) {
        let (w, h) = prep.panel;
        let moved = (r.width - w).abs() > 1e-6 || (r.height - h).abs() > 1e-6;
        if (p.aspect.is_some() || p.inset.is_some()) && moved {
            *prep = render::prepare(figure, p, (r.width, r.height));
        }
    }
    let canvases: Vec<Rect> = figure
        .plots
        .iter()
        .zip(&panels)
        .zip(&prepared)
        .map(|((p, r), prep)| {
            // an inset's canvas is its panel with the room of its own axes
            if p.inset.is_some() {
                let d = prep.depth;
                let (x, y) = (round2(r.x - d.left), round2(r.y - d.top));
                return Rect {
                    x,
                    y,
                    width: r.right() + d.right - x,
                    height: r.bottom() + d.bottom - y,
                };
            }
            let (c, row) = (p.column as usize, p.row as usize);
            let r_end = (p.row + p.row_span - 1) as usize;
            let x = round2(r.x - room.left[c]);
            let y = round2(r.y - room.top[row]);
            Rect {
                x,
                y,
                width: r.right() + room.right[end_column(p)] - x,
                height: r.bottom() + room.bottom[r_end] - y,
            }
        })
        .collect();
    let plots = figure
        .plots
        .iter()
        .zip(&panels)
        .zip(&prepared)
        .map(|((p, r), prep)| PlotBox {
            id: p.id.clone(),
            rect: *r,
            x: ScaleInfo::of(&prep.xa),
            y: ScaleInfo::of(&prep.ya),
            y2: prep.y2a.as_ref().map(ScaleInfo::of),
        })
        .collect();
    let title = figure.title.as_ref().map(|_| {
        let left = panels.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let right = panels.iter().map(Rect::right).fold(0.0, f64::max);
        let (left, right) = if left < right {
            (left, right)
        } else {
            (0.0, width)
        };
        Rect {
            x: left,
            y: 0.0,
            width: right - left,
            height: title_band(figure),
        }
    });
    let mut boxes = Vec::with_capacity(legends.len());
    let mut shapes = Vec::with_capacity(legends.len());
    for (owner, rect, list, shape) in legends {
        let rects = legend::entry_rects(rect, &list, &shape);
        boxes.push(LegendBox {
            owner,
            rect,
            entries: list
                .iter()
                .zip(rects)
                .map(|(e, r)| EntryBox {
                    legend: e.legend.clone(),
                    label: e.label.clone(),
                    rect: r,
                })
                .collect(),
        });
        entries.push(list);
        shapes.push(shape);
    }
    // the bars above the rows, over the panels of their row (since 0.31.0), and below the
    // rows, under the panels of their row and their axes (since 0.34.0)
    let mut row_bars = Vec::new();
    for (i, (p, prep)) in figure.plots.iter().zip(&prepared).enumerate() {
        for (k, l) in prep.bars.iter().enumerate() {
            let row = match l.placement {
                ColorBarPlacement::Top => p.row,
                ColorBarPlacement::Bottom => p.row + p.row_span - 1,
                _ => continue,
            };
            // the plots of the row: which start in it for a bar above, end in it for one below
            let of_row = || {
                figure
                    .plots
                    .iter()
                    .zip(&panels)
                    .filter(|(q, _)| {
                        q.inset.is_none()
                            && if l.placement == ColorBarPlacement::Top {
                                q.row == row
                            } else {
                                q.row + q.row_span - 1 == row
                            }
                    })
                    .map(|(_, r)| r)
            };
            let left = of_row().map(|r| r.x).fold(f64::INFINITY, f64::min);
            let right = of_row().map(Rect::right).fold(f64::NEG_INFINITY, f64::max);
            let r = row as usize;
            let edge = if l.placement == ColorBarPlacement::Top {
                of_row().map(|q| q.y).fold(f64::INFINITY, f64::min) - room.top[r]
            } else {
                of_row().map(Rect::bottom).fold(f64::NEG_INFINITY, f64::max) + room.bottom[r]
                    - room.bars_below[r]
            };
            row_bars.push((i, k, heatmap::row_bar(l, (left, right), edge)));
        }
    }
    // the bands of the titles of the rows, over the panels of their rows (since 0.33.0)
    let mut row_titles = Vec::new();
    for (r, t) in figure.row_titles.iter().flatten().enumerate() {
        if t.is_empty() {
            continue;
        }
        let of_row = || {
            figure
                .plots
                .iter()
                .zip(&panels)
                .filter(|(q, _)| q.row as usize == r && q.inset.is_none())
                .map(|(_, rect)| rect)
        };
        let left = of_row().map(|q| q.x).fold(f64::INFINITY, f64::min);
        let right = of_row().map(Rect::right).fold(f64::NEG_INFINITY, f64::max);
        let top = of_row().map(|q| q.y).fold(f64::INFINITY, f64::min) - room.top[r];
        if left.is_finite() && right.is_finite() && top.is_finite() {
            row_titles.push((
                Rect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: row_title_band(figure, t),
                },
                t.clone(),
            ));
        }
    }
    Plan {
        shapes,
        row_titles,
        row_bars,
        layout: Layout {
            width,
            height,
            plots,
            legends: boxes,
            title,
        },
        prepared,
        canvases,
        entries,
    }
}

/// The columns of a legend above or below the grid without columns given: as many as fit into
/// the width of the figure in one row.
fn fitting_columns(list: &[Entry], base: &Shape, width: f64) -> usize {
    (1..=list.len().max(1))
        .rev()
        .find(|c| {
            let shape = Shape {
                columns: *c,
                ..base.clone()
            };
            legend::size(list, &shape).0 <= width - 2.0 * PAD
        })
        .unwrap_or(1)
}

/// The frames of the legends of the figure above or below the grid (since 0.34.0), each of a
/// column (`Some`) or of the figure, with its room: the legends of a column stacked from
/// `start` in their order, each column its own stack, the legends of the figure stacked before
/// (`centred_first`) or after them, [`LEGEND_GAP`] between two; `x_of` gives the left edge of a
/// legend of a column or of the figure and a width.
fn stack_rects(
    items: &[(Option<u32>, (f64, f64))],
    start: f64,
    centred_first: bool,
    x_of: &dyn Fn(Option<u32>, f64) -> f64,
) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); items.len()];
    let centred = |rects: &mut Vec<Rect>, from: f64| -> f64 {
        let mut y = from;
        for (k, (c, (w, h))) in items.iter().enumerate() {
            if c.is_none() {
                rects[k] = Rect {
                    x: x_of(None, *w),
                    y,
                    width: *w,
                    height: *h,
                };
                y += h + LEGEND_GAP;
            }
        }
        y
    };
    let columns = |rects: &mut Vec<Rect>, from: f64| -> f64 {
        let mut ends: Vec<(u32, f64)> = Vec::new();
        for (k, (c, (w, h))) in items.iter().enumerate() {
            let Some(c) = c else { continue };
            let at = match ends.iter_mut().find(|(col, _)| col == c) {
                Some(end) => end,
                None => {
                    ends.push((*c, from));
                    let last = ends.len() - 1;
                    &mut ends[last]
                }
            };
            rects[k] = Rect {
                x: x_of(Some(*c), *w),
                y: at.1,
                width: *w,
                height: *h,
            };
            at.1 += h + LEGEND_GAP;
        }
        ends.iter().map(|(_, y)| *y).fold(from, f64::max)
    };
    if centred_first {
        let y = centred(&mut rects, start);
        columns(&mut rects, y);
    } else {
        let y = columns(&mut rects, start);
        centred(&mut rects, y);
    }
    rects
}

/// The room the legends of a stack take (see [`stack_rects`]), the gap after the last one
/// included; 0 without one.
fn stack_extent(items: &[(Option<u32>, (f64, f64))]) -> f64 {
    stack_rects(items, 0.0, true, &|_, _| 0.0)
        .iter()
        .map(|r| r.bottom() + LEGEND_GAP)
        .fold(0.0, f64::max)
}

/// The room a legend anchored beyond its panel takes beyond each side of a panel of `panel`
/// size (since 0.34.0): on the side of an anchor below 0 or above 1 the part of the legend
/// beyond the panel, nothing on a side the anchor is within.
fn beyond_panel(
    (w, h): (f64, f64),
    at: [f64; 2],
    size: (f64, f64),
    loc: LegendLoc,
    font: f64,
) -> Sides {
    let origin = Rect {
        width: w,
        height: h,
        ..Rect::default()
    };
    let r = anchor_rect(origin, at, size, loc, font);
    Sides {
        left: if at[0] < 0.0 { (-r.x).max(0.0) } else { 0.0 },
        right: if at[0] > 1.0 {
            (r.right() - w).max(0.0)
        } else {
            0.0
        },
        top: if at[1] > 1.0 { (-r.y).max(0.0) } else { 0.0 },
        bottom: if at[1] < 0.0 {
            (r.bottom() - h).max(0.0)
        } else {
            0.0
        },
    }
}

/// Whether an anchor is beyond its panel (since 0.34.0).
fn beyond(at: [f64; 2]) -> bool {
    at.iter().any(|v| *v < 0.0 || *v > 1.0)
}

/// The declared legends of a plot, none for a plot without a legend.
fn declared(p: &Plot) -> Vec<&[String]> {
    match (&p.declared_legends, p.legend) {
        (Some(lists), true) => lists.iter().map(Vec::as_slice).collect(),
        _ => Vec::new(),
    }
}

/// The location `best` of a legend of `size` inside the panel of a plot of `panel` size: of
/// matplotlib's locations the one of the least badness of matplotlib (since 0.33.0: the
/// vertices, boxes and lines of the data under its frame, `Ink`; the area of ink before), the
/// first in the order of [`LegendLoc::CANDIDATES`] on a tie, as matplotlib's `best`. A location
/// whose frame overlaps an inset colour bar of the plot (since 0.31.0) or the panel of an inset
/// plot (since 0.33.0) loses to every one which does not, the least overlap first.
fn best_loc(
    figure: &Figure,
    i: usize,
    prep: &Prepared,
    panel: (f64, f64),
    size: (f64, f64),
) -> LegendLoc {
    best_loc_avoiding(figure, i, prep, panel, size, &[])
}

/// [`best_loc`] avoiding the rectangles `taken` of the panel as it avoids an inset (the other
/// legends of the panel, since 0.33.0).
fn best_loc_avoiding(
    figure: &Figure,
    i: usize,
    prep: &Prepared,
    panel: (f64, f64),
    size: (f64, f64),
    taken: &[Rect],
) -> LegendLoc {
    let plot = &figure.plots[i];
    let ink = Ink::of_plot(plot, prep, &figure.fonts, panel);
    let origin = Rect {
        width: panel.0,
        height: panel.1,
        ..Rect::default()
    };
    let mut insets = inset_rects(plot, prep, &figure.fonts, origin);
    // the panels of the inset plots of the plot (since 0.33.0)
    insets.extend(
        figure
            .plots
            .iter()
            .filter_map(|q| q.inset.as_ref())
            .filter(|inset| inset.plot == plot.id)
            .map(|inset| inset_panel(inset, origin)),
    );
    insets.extend_from_slice(taken);
    let mut best = (LegendLoc::UpperRight, (f64::INFINITY, usize::MAX));
    for loc in LegendLoc::CANDIDATES {
        let r = loc_rect(origin, size, loc);
        let covered: f64 = insets.iter().map(|i| overlap(&r, i)).sum();
        let badness = ink.badness(&r);
        if (covered, badness) < best.1 {
            best = (loc, (covered, badness));
        }
    }
    best.0
}

/// The rectangles of the inset colour bars of a plot, with their ticks and labels, in its
/// panel `panel`.
pub(crate) fn inset_rects(plot: &Plot, prep: &Prepared, fonts: &Fonts, panel: Rect) -> Vec<Rect> {
    plot.color_bars()
        .iter()
        .zip(&prep.bars)
        .filter_map(|(b, l)| match l.placement {
            ColorBarPlacement::Inset => Some(l.inset_rect(b, panel, fonts)),
            // a strip along the top of the panel (since 0.33.0)
            ColorBarPlacement::Strip => Some(l.strip_rect(b, panel)),
            _ => None,
        })
        .collect()
}

/// The area two rectangles share.
fn overlap(a: &Rect, b: &Rect) -> f64 {
    let w = a.right().min(b.right()) - a.x.max(b.x);
    let h = a.bottom().min(b.bottom()) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// The frame of a legend of `size` anchored at a point `at` in fractions of a panel (since
/// 0.33.0): its side or corner `loc` at the point, half its font size `font` off it on the side
/// of the legend, as matplotlib places a legend in a `bbox_to_anchor` of a point with its
/// `borderaxespad`.
pub(crate) fn anchor_rect(
    panel: Rect,
    at: [f64; 2],
    (w, h): (f64, f64),
    loc: LegendLoc,
    font: f64,
) -> Rect {
    let pad = 0.5 * font;
    let (px, py) = (
        panel.x + at[0] * panel.width,
        panel.y + (1.0 - at[1]) * panel.height,
    );
    let (left, center, right) = (px + pad, px - w / 2.0, px - pad - w);
    let (top, middle, bottom) = (py + pad, py - h / 2.0, py - pad - h);
    let (x, y) = match loc {
        LegendLoc::Best | LegendLoc::UpperRight => (right, top),
        LegendLoc::UpperLeft => (left, top),
        LegendLoc::LowerLeft => (left, bottom),
        LegendLoc::LowerRight => (right, bottom),
        LegendLoc::Right | LegendLoc::CenterRight => (right, middle),
        LegendLoc::CenterLeft => (left, middle),
        LegendLoc::LowerCenter => (center, bottom),
        LegendLoc::UpperCenter => (center, top),
        LegendLoc::Center => (center, middle),
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// The frame of a legend of `size` at a location of a panel, `LEGEND_INSET` from its edges;
/// `best` is resolved before (see [`best_loc`]) and stands for the upper right here.
pub(crate) fn loc_rect(panel: Rect, (w, h): (f64, f64), loc: LegendLoc) -> Rect {
    let (left, right) = (panel.x + LEGEND_INSET, panel.right() - LEGEND_INSET - w);
    let (top, bottom) = (panel.y + LEGEND_INSET, panel.bottom() - LEGEND_INSET - h);
    let (center, middle) = (
        panel.x + (panel.width - w) / 2.0,
        panel.y + (panel.height - h) / 2.0,
    );
    let (x, y) = match loc {
        LegendLoc::Best | LegendLoc::UpperRight => (right, top),
        LegendLoc::UpperLeft => (left, top),
        LegendLoc::LowerLeft => (left, bottom),
        LegendLoc::LowerRight => (right, bottom),
        LegendLoc::Right | LegendLoc::CenterRight => (right, middle),
        LegendLoc::CenterLeft => (left, middle),
        LegendLoc::LowerCenter => (center, bottom),
        LegendLoc::UpperCenter => (center, top),
        LegendLoc::Center => (center, middle),
    };
    Rect {
        x: x.max(panel.x),
        y: y.max(panel.y),
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    /// The panels of a drawing in the pixels of the figure: the clip of the first plot group of
    /// every nested `<svg>`, moved by its `x` and `y`.
    fn drawn_panels(svg: &str) -> Vec<(String, Rect)> {
        let doc = Document::parse(svg).unwrap();
        let num = |n: roxmltree::Node, a: &str| n.attribute(a).unwrap().parse::<f64>().unwrap();
        doc.descendants()
            .filter(|n| n.has_tag_name("svg") && n.attribute("data-plot").is_some())
            .map(|plot| {
                let g = plot
                    .descendants()
                    .find(|n| n.attribute("class") == Some("charton-plot"))
                    .unwrap();
                let id = g
                    .attribute("clip-path")
                    .unwrap()
                    .trim_start_matches("url(#")
                    .trim_end_matches(')');
                let rect = doc
                    .descendants()
                    .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(id))
                    .and_then(|c| c.first_element_child())
                    .unwrap();
                (
                    plot.attribute("data-plot").unwrap().to_string(),
                    Rect {
                        x: num(plot, "x") + num(rect, "x"),
                        y: num(plot, "y") + num(rect, "y"),
                        width: num(rect, "width"),
                        height: num(rect, "height"),
                    },
                )
            })
            .collect()
    }

    /// The fixture `tests/specs/<name>.json`.
    fn fixture(name: &str) -> Figure {
        let path = format!("{}/tests/specs/{name}.json", env!("CARGO_MANIFEST_DIR"));
        crate::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The guard of the two-pass placement: the panel Charton draws is the panel of the layout.
    /// It rests on Charton laying a panel out as its canvas less its margins less the bands of
    /// its axes and title (`resolve_scene` of Charton 0.9.1), which the probe measures; a Charton
    /// upgrade which changes that fails here first. Keep it.
    #[test]
    fn panels_land_on_the_layout() {
        for name in [
            "grid",
            "every_kind",
            "benchmark",
            "rotated",
            "curve",
            "styles",
        ] {
            let figure = fixture(name);
            let lay = plan(&figure).layout;
            let drawn = drawn_panels(&crate::render_svg(&figure));
            assert_eq!(drawn.len(), lay.plots.len(), "{name}");
            for ((id, d), b) in drawn.iter().zip(&lay.plots) {
                assert_eq!(id, &b.id);
                let r = b.rect;
                for (a, e) in [
                    (d.x, r.x),
                    (d.y, r.y),
                    (d.width, r.width),
                    (d.height, r.height),
                ] {
                    assert!(
                        (a - e).abs() <= 0.011,
                        "{name} {id}: drawn {d:?}, laid out {r:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_track_fills_its_stretch() {
        // two panels with room 30 left and 20 right of each and a space of 0.3 of a panel
        let t = track(10.0, 500.0, &[30.0, 30.0], &[20.0, 20.0], 0.3, &[1, 1]);
        let gap = (0.3 * t.size).max(20.0 + 30.0 + PAD);
        assert!((30.0 + 2.0 * t.size + gap + 20.0 - 500.0).abs() < 1e-9);
        assert_eq!(t.starts[0], 40.0);
        assert!((t.starts[1] - (40.0 + t.size + gap)).abs() < 1e-9);
        // a wide space decides the gap, a narrow one leaves it to the axes
        let wide = track(0.0, 1000.0, &[10.0, 10.0], &[10.0, 10.0], 1.0, &[1]);
        assert!((wide.starts[1] - wide.starts[0] - 2.0 * wide.size).abs() < 1e-9);
        let narrow = track(0.0, 1000.0, &[10.0, 10.0], &[10.0, 10.0], 0.0, &[1]);
        assert!((narrow.starts[1] - narrow.starts[0] - narrow.size - 28.0).abs() < 1e-9);
        assert_eq!(t.overflow + wide.overflow + narrow.overflow, 0.0);
    }

    /// The end of the last panel of a track and the room after it.
    fn track_end(t: &Track, after: f64) -> f64 {
        t.starts[t.starts.len() - 1] + t.size + after
    }

    #[test]
    fn thin_cells_fit_when_the_plots_span_enough_of_them() {
        // 32 columns of a space of 1.2 in 1209 pixels: a cell is 17.5 pixels, below the least
        // panel, but the plots span 8 columns each, so the grid fills its stretch
        let (before, after) = (vec![0.0; 32], vec![0.0; 32]);
        let t = track(8.0, 1209.0, &before, &after, 1.2, &[8, 8, 8, 8]);
        assert!(t.size < MIN_PANEL);
        assert_eq!(t.overflow, 0.0);
        assert!((track_end(&t, 0.0) - (8.0 + 1209.0)).abs() < 1e-9);
        // a plot of one cell needs the least panel: the grid reaches beyond its stretch and
        // says by how much
        let one = track(8.0, 1209.0, &before, &after, 1.2, &[1, 8]);
        assert_eq!(one.size, MIN_PANEL);
        assert!((track_end(&one, 0.0) - (8.0 + 1209.0) - one.overflow).abs() < 1e-9);
        assert!(one.overflow > 0.0);
    }

    #[test]
    fn a_wide_grid_fits_the_figure() {
        let mut plots = Vec::new();
        for k in 0..4 {
            plots.push(format!(
                r#"{{"id": "p{k}", "row": 0, "column": {c}, "column_span": 8,
                    "x": {{}}, "y": {{}}, "elements": [{{"id": "c{k}", "kind": "curve",
                    "x": [0, 1], "y": [0, 1]}}]}}"#,
                c = 8 * k
            ));
        }
        let spec = format!(
            r#"{{"contract": "figure/1", "width": 1225, "height": 300,
                "grid": {{"rows": 1, "columns": 32, "wspace": 1.2}}, "plots": [{}]}}"#,
            plots.join(",")
        );
        let figure = crate::parse(&spec).unwrap();
        let lay = plan(&figure).layout;
        assert_eq!(lay.width, 1225.0);
        for b in &lay.plots {
            assert!(b.rect.right() <= 1225.0 - PAD + 1e-9, "{b:?}");
        }
    }

    #[test]
    fn a_span_of_thin_rows_keeps_its_plot_in_the_figure() {
        // a heatmap over 3 of 5 thin rows and a panel over 2 of them, as a 3:2 split
        let spec = r#"{"contract": "figure/1", "width": 400, "height": 200,
            "grid": {"rows": 5, "columns": 1},
            "plots": [
                {"id": "a", "row": 0, "column": 0, "row_span": 3, "x": {"label": "t"},
                 "y": {"label": "z"}, "elements": []},
                {"id": "b", "row": 3, "column": 0, "row_span": 2, "x": {"label": "t"},
                 "y": {"label": "f"}, "elements": []}]}"#;
        let figure = crate::parse(spec).unwrap();
        let plan = plan(&figure);
        assert_eq!(plan.layout.height, 200.0);
        for c in &plan.canvases {
            assert!(c.bottom() <= 200.0 + 1e-9, "{c:?}");
        }
        // the rows are thinner than the least panel, the spans are not
        let a = plan.layout.plots[0].rect;
        assert!(a.height >= MIN_PANEL, "{a:?}");
    }
}

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
//! figure; the frame of the entries drawn now sits at the top of that room (at its bottom in a
//! lower corner of a panel), as wide as the room.
//!
//! The legends follow the legend settings of the figure. Inside, a legend is placed in the
//! corner of the panel of its plot (of a row: of its rightmost plot) which covers the fewest
//! data, the upper right one on a tie; a legend which does not fit into that panel goes outside
//! instead. Outside, the legend of a plot is right of its panel and its right axis, and the
//! column gets the room of its widest legend; the legend of a row is right of the grid at the
//! top of the panels of its row. The panels keep their size and the figure grows by the room of
//! the legends: right by their width, and down by the part of a legend which reaches below the
//! room of its last row, which that row gets before the next one, so a tall legend never
//! reaches into the row below it.
//!
//! Planning a figure resolves its axes twice and probes Charton's bands of every plot, a few
//! tenths of a millisecond. A caller which needs the geometry more than once (the geometry and
//! the pick of a figure, the pick on every pointer move of a page) plans the figure once with
//! [`plan`] and keeps the [`Plan`]: [`Plan::layout`] is the geometry, the pick reads the
//! `Layout` it is given, and `render::render_planned` draws the figure on the same plan, so the
//! drawing and the geometry come from one layout. [`layout`] is the plan's layout for a caller
//! which needs it once.

use crate::render::axis::{ResolvedAxis, Transform};
use crate::render::legend::{self, Entry};
use crate::render::{self, Prepared, Sides};
use crate::spec::{
    AxisScale, ElementKind, Figure, LegendPosition, LegendScope, LineType, MarkerType, Orientation,
    Plot, YAxisRef,
};

/// The padding at the edges of the figure and the least gap between the axes of two
/// neighbouring plots, in pixels.
pub(crate) const PAD: f64 = 8.0;
/// The gap between a plot and a legend outside of it, in pixels.
const LEGEND_GAP: f64 = 10.0;
/// The gap between a legend inside and the edges of its panel, in pixels.
const LEGEND_INSET: f64 = 6.0;
/// The least width and height of a panel, in pixels.
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
#[allow(dead_code, reason = "the geometry and the pick of a figure read it")]
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
#[allow(dead_code, reason = "the geometry and the pick of a figure read it")]
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
}

/// One entry of a legend.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code, reason = "the geometry and the pick of a figure read it")]
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
    #[allow(dead_code, reason = "the geometry and the pick of a figure read it")]
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

/// The layout of a figure.
#[allow(dead_code, reason = "the geometry and the pick of a figure read it")]
pub(crate) fn layout(figure: &Figure) -> Layout {
    plan(figure).layout
}

/// A figure laid out, with what drawing it needs beyond the boxes.
pub(crate) struct Plan {
    /// The boxes.
    pub layout: Layout,
    /// Every plot prepared, in the order of the spec.
    pub prepared: Vec<Prepared>,
    /// The canvas of every plot: its panel with the room of its column and row around it.
    pub canvases: Vec<Rect>,
    /// The entries of every legend of the layout.
    pub entries: Vec<Vec<Entry>>,
}

/// A legend before it is placed: its owner, where it goes, its entries and the room it takes.
type Pending = (LegendOwner, Place, Vec<Entry>, (f64, f64));

/// Where a legend goes.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// Inside the panel of the plot at this position.
    Inside(usize),
    /// Right of the plot at this position.
    RightOfPlot(usize),
    /// Right of the grid, at the top of this row.
    RightOfGrid(usize),
}

/// The extent of the panels along one direction of the grid.
struct Track {
    /// The size of one panel.
    size: f64,
    /// Where the panel of every column (row) starts.
    starts: Vec<f64>,
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
/// fills and `space` the space between two panels as a share of a panel.
///
/// The gap between two neighbouring panels is the larger of `space` times a panel and the room
/// of the axes between them with [`PAD`]; the size of a panel is the one which fills the
/// stretch. The stretch as a function of the size is increasing and piecewise linear, so the
/// size is found on the piece where the gaps which `space` decides are the ones below it.
fn track(start: f64, length: f64, before: &[f64], after: &[f64], space: f64) -> Track {
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
    let size = size
        .unwrap_or((available - total) / n as f64)
        .max(MIN_PANEL);
    let mut starts = Vec::with_capacity(n);
    let mut at = start + before[0];
    starts.push(at);
    for need in &needs {
        at += size + (space * size).max(*need);
        starts.push(at);
    }
    Track { size, starts }
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
}

/// The room of the columns and rows: the largest depth of the plots which start or end there.
fn room(figure: &Figure, depths: &[Sides]) -> Room {
    let (cols, rows) = (figure.grid.columns as usize, figure.grid.rows as usize);
    let mut r = Room {
        left: vec![0.0; cols],
        right: vec![0.0; cols],
        top: vec![0.0; rows],
        bottom: vec![0.0; rows],
    };
    for (p, d) in figure.plots.iter().zip(depths) {
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
    r
}

/// The height of the title band of a figure: twice the title font, 0 without a title.
fn title_band(figure: &Figure) -> f64 {
    if figure.title.is_some() {
        2.0 * figure.fonts.title
    } else {
        0.0
    }
}

/// The panels of the plots for the depths of their plots, with the room of the grid.
fn grid(figure: &Figure, depths: &[Sides]) -> (Vec<Rect>, Room) {
    let band = title_band(figure);
    let r = room(figure, depths);
    let columns = track(
        PAD,
        figure.width - 2.0 * PAD,
        &r.left,
        &r.right,
        figure.grid.wspace,
    );
    let rows = track(
        band + PAD,
        figure.height - band - 2.0 * PAD,
        &r.top,
        &r.bottom,
        figure.grid.hspace,
    );
    let panels = figure
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
    (panels, r)
}

/// The panels of a plain division of the figure into its grid, for the first resolution of the
/// ticks.
fn estimate(figure: &Figure) -> Vec<(f64, f64)> {
    let band = title_band(figure);
    let cw = (figure.width - 2.0 * PAD) / f64::from(figure.grid.columns);
    let ch = (figure.height - band - 2.0 * PAD) / f64::from(figure.grid.rows);
    figure
        .plots
        .iter()
        .map(|p| {
            let right = if p.y2.is_some() { ESTIMATE_ROOM.0 } else { 0.0 };
            let top = if p.title.is_some() {
                2.0 * figure.fonts.label
            } else {
                0.0
            };
            (
                (f64::from(p.column_span) * cw - ESTIMATE_ROOM.0 - right).max(MIN_PANEL),
                (f64::from(p.row_span) * ch - ESTIMATE_ROOM.1 - top).max(MIN_PANEL),
            )
        })
        .collect()
}

/// Rounds a coordinate to the two decimals of the SVG, so a nested `<svg>` starts where it is
/// written.
fn round2(v: f64) -> f64 {
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
    let depths_of =
        |prepared: &[Prepared]| -> Vec<Sides> { prepared.iter().map(|p| p.depth).collect() };
    let first = prepare_all(&estimate(figure));
    let (panels, _) = grid(figure, &depths_of(&first));
    let sizes: Vec<(f64, f64)> = panels.iter().map(|r| (r.width, r.height)).collect();
    let prepared = prepare_all(&sizes);
    let (mut panels, room) = grid(figure, &depths_of(&prepared));

    let plot_entries: Vec<Vec<Entry>> = figure
        .plots
        .iter()
        .zip(&prepared)
        .map(|(p, prep)| {
            if p.legend {
                legend::plot_entries(p, &prep.colors, fonts)
            } else {
                Vec::new()
            }
        })
        .collect();
    let end_column = |p: &Plot| (p.column + p.column_span - 1) as usize;
    let end_row = |p: &Plot| (p.row + p.row_span - 1) as usize;
    let fits = |panel: &Rect, (w, h): (f64, f64)| {
        w + 2.0 * LEGEND_INSET <= panel.width && h + 2.0 * LEGEND_INSET <= panel.height
    };
    let inside = figure.legend.position == LegendPosition::Inside;
    // every legend with where it goes and the room it takes (the largest of the legends it may
    // show, `legend::room`); a legend inside which does not fit into its panel goes outside
    // instead
    let mut pending: Vec<Pending> = Vec::new();
    match figure.legend.scope {
        LegendScope::Plot => {
            for (i, list) in plot_entries.into_iter().enumerate() {
                let Some(size) = legend::room(&list, &declared(&figure.plots[i]), fonts) else {
                    continue;
                };
                let place = if inside && fits(&panels[i], size) {
                    Place::Inside(i)
                } else {
                    Place::RightOfPlot(i)
                };
                pending.push((
                    LegendOwner::Plot(figure.plots[i].id.clone()),
                    place,
                    list,
                    size,
                ));
            }
        }
        LegendScope::Row => {
            let rows = figure.grid.rows as usize;
            let mut lists_of_row: Vec<Vec<Vec<Entry>>> = vec![Vec::new(); rows];
            let mut declared_of_row: Vec<Vec<&[String]>> = vec![Vec::new(); rows];
            for (p, list) in figure.plots.iter().zip(plot_entries) {
                lists_of_row[p.row as usize].push(list);
                declared_of_row[p.row as usize].extend(declared(p));
            }
            for (r, lists) in lists_of_row.into_iter().enumerate() {
                let list = legend::merge(lists);
                let Some(size) = legend::room(&list, &declared_of_row[r], fonts) else {
                    continue;
                };
                // the rightmost plot of the row with a legend holds a legend inside
                let host = figure
                    .plots
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.row as usize == r && p.legend)
                    .max_by_key(|(i, p)| (p.column + p.column_span, usize::MAX - i))
                    .map(|(i, _)| i);
                let place = match host {
                    Some(i) if inside && fits(&panels[i], size) => Place::Inside(i),
                    _ => Place::RightOfGrid(r),
                };
                pending.push((LegendOwner::Row(r as u32), place, list, size));
            }
        }
    }

    // the legends right of a plot: the column gets the room of its widest one, the columns
    // right of it move by it
    let columns = figure.grid.columns as usize;
    let mut extra = vec![0.0f64; columns];
    for (_, place, _, size) in &pending {
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
    let mut width = figure.width + extra.iter().sum::<f64>();
    // the legends right of the grid
    let grid_right = figure
        .plots
        .iter()
        .zip(&panels)
        .map(|(p, r)| r.right() + room.right[end_column(p)])
        .fold(0.0, f64::max);
    let widest = pending
        .iter()
        .filter(|(_, place, _, _)| matches!(place, Place::RightOfGrid(_)))
        .map(|(_, _, _, size)| size.0)
        .fold(0.0, f64::max);
    if widest > 0.0 {
        width += LEGEND_GAP + widest;
    }
    // the legends outside, at the top of their plot or row, and the last row of the grid they
    // reach down to
    let mut outside: Vec<Option<(Rect, usize, usize)>> = pending
        .iter()
        .map(|(_, place, _, size)| {
            let (w, h) = *size;
            match place {
                Place::Inside(_) => None,
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
                            .filter(|(p, _)| p.row as usize == *r)
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
    for (rect, first, _) in outside.iter_mut().flatten() {
        // a legend moves with the first row of its plot or row
        rect.y += down[*first];
    }
    let height = figure.height + overflow.iter().sum::<f64>();
    // the legends inside, in the panels where they end up
    let mut legends = Vec::with_capacity(pending.len());
    let mut entries = Vec::with_capacity(pending.len());
    for ((owner, place, list, size), out) in pending.into_iter().zip(outside) {
        // the room of the legend, in which the frame of the entries drawn now sits at the top,
        // at the bottom in a lower corner of a panel
        let (room, lower) = match (place, out) {
            (_, Some((rect, _, _))) => (rect, false),
            (Place::Inside(i), None) | (Place::RightOfPlot(i), None) => {
                let rect = corner(&figure.plots[i], &prepared[i], panels[i], size);
                (rect, rect.y > panels[i].y + LEGEND_INSET)
            }
            (Place::RightOfGrid(_), None) => continue,
        };
        if list.is_empty() {
            continue;
        }
        let height = legend::size(&list, fonts).1;
        let rect = Rect {
            y: if lower {
                room.bottom() - height
            } else {
                room.y
            },
            height,
            ..room
        };
        legends.push((owner, rect, list));
    }

    let canvases: Vec<Rect> = figure
        .plots
        .iter()
        .zip(&panels)
        .map(|(p, r)| {
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
    for (owner, rect, list) in legends {
        let rects = legend::entry_rects(rect, list.len(), fonts);
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
    }
    Plan {
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

/// The declared legends of a plot, none for a plot without a legend.
fn declared(p: &Plot) -> Vec<&[String]> {
    match (&p.declared_legends, p.legend) {
        (Some(lists), true) => lists.iter().map(Vec::as_slice).collect(),
        _ => Vec::new(),
    }
}

/// The frame of a legend of `size` inside the panel of a plot: the corner which covers the
/// fewest data, the upper right, the upper left, the lower right and the lower left in this
/// order on a tie.
fn corner(plot: &Plot, prep: &Prepared, panel: Rect, (w, h): (f64, f64)) -> Rect {
    let (left, right) = (panel.x + LEGEND_INSET, panel.right() - LEGEND_INSET - w);
    let (top, bottom) = (panel.y + LEGEND_INSET, panel.bottom() - LEGEND_INSET - h);
    let candidates =
        [(right, top), (left, top), (right, bottom), (left, bottom)].map(|(x, y)| Rect {
            x: x.max(panel.x),
            y: y.max(panel.y),
            width: w,
            height: h,
        });
    let shapes = data_shapes(plot, prep, panel);
    let mut best = (usize::MAX, candidates[0]);
    for c in candidates {
        let hits = shapes.hits(&c);
        if hits < best.0 {
            best = (hits, c);
        }
    }
    best.1
}

/// The data of a plot in pixels: the segments of its lines and edges and its single points.
#[derive(Default)]
struct Shapes {
    segments: Vec<[(f64, f64); 2]>,
    points: Vec<(f64, f64)>,
}

impl Shapes {
    /// Adds the segments between consecutive drawable points.
    fn polyline(&mut self, points: impl IntoIterator<Item = (f64, f64)>) {
        let mut last: Option<(f64, f64)> = None;
        for p in points {
            let ok = p.0.is_finite() && p.1.is_finite();
            if let (true, Some(a)) = (ok, last) {
                self.segments.push([a, p]);
            }
            last = ok.then_some(p);
        }
    }

    /// Adds a point.
    fn point(&mut self, p: (f64, f64)) {
        if p.0.is_finite() && p.1.is_finite() {
            self.points.push(p);
        }
    }

    /// How many segments and points lie in a rectangle.
    fn hits(&self, r: &Rect) -> usize {
        let inside = |(x, y): (f64, f64)| x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom();
        self.points.iter().filter(|p| inside(**p)).count()
            + self
                .segments
                .iter()
                .filter(|[a, b]| crosses(*a, *b, r))
                .count()
    }
}

/// Whether the segment from `a` to `b` meets a rectangle (Liang and Barsky).
fn crosses(a: (f64, f64), b: (f64, f64), r: &Rect) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, a.0 - r.x),
        (dx, r.right() - a.0),
        (-dy, a.1 - r.y),
        (dy, r.bottom() - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// The data of the elements of a plot in pixels of its panel.
fn data_shapes(plot: &Plot, prep: &Prepared, panel: Rect) -> Shapes {
    let xa = &prep.xa;
    let px = |v: f64| panel.x + panel.width * xa.share(xa.u(v));
    let mut s = Shapes::default();
    for e in &plot.elements {
        let ya = match (e.axis, &prep.y2a) {
            (YAxisRef::Y2, Some(a)) => a,
            _ => &prep.ya,
        };
        let py = |v: f64| panel.y + panel.height * (1.0 - ya.share(ya.u(v)));
        let at = |x: f64, y: f64| (px(x), py(y));
        let line = |xs: &[f64], ys: &[f64], s: &mut Shapes| {
            s.polyline(xs.iter().zip(ys).map(|(x, y)| at(*x, *y)));
        };
        match &e.kind {
            ElementKind::Curve { x, y, .. } => {
                let lined = e.style.line.kind != LineType::None && e.style.line.width > 0.0;
                if lined {
                    line(x, y, &mut s);
                }
                if !lined || e.style.marker.kind != MarkerType::None {
                    for (a, b) in x.iter().zip(y) {
                        s.point(at(*a, *b));
                    }
                }
            }
            ElementKind::Band {
                x,
                low,
                high,
                median,
            } => {
                line(x, low, &mut s);
                line(x, high, &mut s);
                if let Some(m) = median {
                    line(x, m, &mut s);
                }
            }
            ElementKind::Area { x, from, to } => {
                line(x, from, &mut s);
                line(x, to, &mut s);
            }
            ElementKind::Bar {
                x,
                height,
                horizontal,
                ..
            } => {
                for (p, v) in x.iter().zip(height) {
                    let (a, b) = if *horizontal {
                        (at(0.0, *p), at(*v, *p))
                    } else {
                        (at(*p, 0.0), at(*p, *v))
                    };
                    s.polyline([a, b]);
                    s.point(b);
                }
            }
            ElementKind::Box { positions, stats } => {
                for (p, b) in positions.iter().zip(stats) {
                    s.polyline([at(*p, b.low_whisker), at(*p, b.high_whisker)]);
                    for v in &b.outliers {
                        s.point(at(*p, *v));
                    }
                }
            }
            ElementKind::Violin {
                positions,
                densities,
            } => {
                for (p, d) in positions.iter().zip(densities) {
                    s.polyline(d.values.iter().map(|v| at(*p, *v)));
                }
            }
            ElementKind::Histogram { edges, counts } => {
                s.polyline(
                    edges
                        .windows(2)
                        .zip(counts)
                        .flat_map(|(w, c)| [at(w[0], *c), at(w[1], *c)]),
                );
            }
            ElementKind::Text { x, y, .. } => s.point(at(*x, *y)),
            ElementKind::Rule { orientation, value } => match orientation {
                Orientation::Horizontal => {
                    let y = py(*value);
                    s.polyline([(panel.x, y), (panel.right(), y)]);
                }
                Orientation::Vertical => {
                    let x = px(*value);
                    s.polyline([(x, panel.y), (x, panel.bottom())]);
                }
            },
            ElementKind::Polygon { paths } => {
                for ring in paths {
                    s.polyline(ring.iter().chain(ring.first()).map(|[x, y]| at(*x, *y)));
                }
            }
            // a heatmap covers its whole panel, every corner alike
            ElementKind::Heatmap { .. } => {}
        }
    }
    s
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
        for name in ["grid", "every_kind", "benchmark", "rotated", "curve"] {
            let figure = fixture(name);
            let lay = layout(&figure);
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
        let t = track(10.0, 500.0, &[30.0, 30.0], &[20.0, 20.0], 0.3);
        let gap = (0.3 * t.size).max(20.0 + 30.0 + PAD);
        assert!((30.0 + 2.0 * t.size + gap + 20.0 - 500.0).abs() < 1e-9);
        assert_eq!(t.starts[0], 40.0);
        assert!((t.starts[1] - (40.0 + t.size + gap)).abs() < 1e-9);
        // a wide space decides the gap, a narrow one leaves it to the axes
        let wide = track(0.0, 1000.0, &[10.0, 10.0], &[10.0, 10.0], 1.0);
        assert!((wide.starts[1] - wide.starts[0] - 2.0 * wide.size).abs() < 1e-9);
        let narrow = track(0.0, 1000.0, &[10.0, 10.0], &[10.0, 10.0], 0.0);
        assert!((narrow.starts[1] - narrow.starts[0] - narrow.size - 28.0).abs() < 1e-9);
    }

    #[test]
    fn a_corner_without_data() {
        let crossing = |a, b, r: Rect| crosses(a, b, &r);
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        assert!(crossing((-5.0, 5.0), (15.0, 5.0), r));
        assert!(crossing((2.0, 2.0), (3.0, 3.0), r));
        assert!(!crossing((-5.0, -1.0), (15.0, -1.0), r));
        assert!(!crossing((11.0, 0.0), (20.0, 10.0), r));
    }
}

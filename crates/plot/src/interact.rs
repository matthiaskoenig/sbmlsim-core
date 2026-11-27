//! The geometry of a figure and the pick of a point: what a page needs to map the pointer
//! onto the data and to find the legend entries.
//!
//! Both read the layout the renderer drew with: a figure is planned once ([`FigurePlan`]) and
//! the geometry and every pick read that plan, so nothing is recomputed per pointer move.
//! [`geometry`] and [`pick`] plan the figure themselves for a caller which needs them once.
//! The pixel of a value goes through the resolved axes of the plot (linear, `log10` through
//! `libm`, the positions of categories, reversed axes), so the result is the same natively
//! and in wasm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::layout::{self, LegendOwner, Plan, Rect};
use crate::render::axis::ResolvedAxis;
use crate::render::heatmap::cell_span;
use crate::render::stats_marks::{BAR_WIDTH, VIOLIN_WIDTH, base_of, box_body, spacing};
use crate::spec::{
    AxisScale, ElementKind, Errors, Figure, LineType, Orientation, YAxisRef, heatmap_size,
};

/// A figure laid out once: the plan the geometry and the pick read.
pub struct FigurePlan(Plan);

impl FigurePlan {
    /// Lays a figure out.
    pub fn new(figure: &Figure) -> Self {
        FigurePlan(layout::plan(figure))
    }

    /// The SVG of the figure drawn on this plan.
    pub fn render_svg(&self, figure: &Figure) -> String {
        crate::render::render_planned(figure, &self.0)
    }
}

/// A rectangle in the pixels of the figure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl From<Rect> for PixelRect {
    fn from(r: Rect) -> Self {
        PixelRect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// How an axis maps its values onto its side of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    /// `linear`, `log` or `category`.
    pub kind: AxisScale,
    /// The value at the start of the axis (left, bottom), before a reversal.
    pub low: f64,
    /// The value at the end of the axis.
    pub high: f64,
    /// Whether the axis runs from `high` to `low`.
    pub reverse: bool,
}

impl From<&layout::ScaleInfo> for Scale {
    fn from(s: &layout::ScaleInfo) -> Self {
        Scale {
            kind: s.kind,
            low: s.low,
            high: s.high,
            reverse: s.reverse,
        }
    }
}

/// The panel of a plot and its scales.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlotGeometry {
    /// The id of the plot.
    pub id: String,
    /// The panel, the rectangle the data is drawn in.
    pub rect: PixelRect,
    /// The x axis.
    pub x: Scale,
    /// The left y axis.
    pub y: Scale,
    /// The right y axis.
    pub y2: Option<Scale>,
}

/// The rectangle of a legend entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegendEntryGeometry {
    /// The legend group of the entry.
    pub legend: String,
    /// The id of the plot whose legend it is, `row:<n>` for the legend of a row, or `figure`
    /// for the legend below the grid (since 0.31.0).
    pub owner: String,
    /// The rectangle of its swatch and label.
    pub rect: PixelRect,
}

/// Where the panels and the legend entries of a figure are, in its pixels, and how large it is
/// drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// The width of the figure as drawn, the `width` of its SVG (to its two decimals): the
    /// `width` of the spec, grown by the legends right of the plots and of the rows, which an
    /// inside legend too large for its panel becomes as well, and by a grid whose plots cannot
    /// have their least panel in it (since 0.30.0). The PNG is `width * dpi / 72` pixels wide.
    pub width: f64,
    /// The height of the figure as drawn, the `height` of its SVG (to its two decimals): the
    /// `height` of the spec, grown by a legend outside which is taller than its row, and by a
    /// grid whose plots cannot have their least panel in it (since 0.30.0).
    pub height: f64,
    /// The plots, in the order of the spec.
    pub plots: Vec<PlotGeometry>,
    /// The entries of the legends drawn, from the top of each legend.
    pub legend_entries: Vec<LegendEntryGeometry>,
}

/// A data point under the pointer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    /// The id of the plot.
    pub plot: String,
    /// The id of the element.
    pub element: String,
    /// The legend group of the element.
    pub legend: Option<String>,
    /// The position of the point in the data of the element (the cell of a heatmap in row
    /// order).
    pub index: usize,
    /// The data at the point: `x`, `y`, `xerr`, `yerr` of a curve, `x`, `low`, `high` (and
    /// `median`) of a band, `x`, `from`, `to` of an area, `x`, `height`, `err` of a bar, `x`
    /// and the five numbers of a box, `x` of a violin, `low`, `high`, `count` of a histogram
    /// bin, `value` of a heatmap cell (with `x_low`, `x_high` along `x_edges` and `y_low`,
    /// `y_high` along `y_edges`, since 0.30.0), `x`, `y` of a text, `value` of a rule.
    pub values: BTreeMap<String, f64>,
    /// The label of the element; `row / column` for a heatmap cell, the name it has along an
    /// axis of names alone for a heatmap with edges, none without names (since 0.30.0).
    pub label: Option<String>,
}

/// The geometry of a figure.
pub fn geometry(figure: &Figure) -> Geometry {
    geometry_of(&FigurePlan::new(figure))
}

/// The geometry of a figure planned already (its plan is of that figure, whatever it is).
pub fn geometry_of(plan: &FigurePlan) -> Geometry {
    let l = &plan.0.layout;
    let plots = l
        .plots
        .iter()
        .map(|p| PlotGeometry {
            id: p.id.clone(),
            rect: p.rect.into(),
            x: (&p.x).into(),
            y: (&p.y).into(),
            y2: p.y2.as_ref().map(Scale::from),
        })
        .collect();
    let legend_entries = l
        .legends
        .iter()
        .flat_map(|lb| {
            let owner = match &lb.owner {
                LegendOwner::Plot(id) => id.clone(),
                LegendOwner::Row(r) => format!("row:{r}"),
                LegendOwner::Figure => "figure".to_string(),
            };
            lb.entries.iter().map(move |e| LegendEntryGeometry {
                legend: e.legend.clone(),
                owner: owner.clone(),
                rect: e.rect.into(),
            })
        })
        .collect();
    Geometry {
        width: layout::round2(l.width),
        height: layout::round2(l.height),
        plots,
        legend_entries,
    }
}

/// The data point nearest to the pixel `(x, y)` within `radius` pixels, `None` without one.
pub fn pick(figure: &Figure, x: f64, y: f64, radius: f64) -> Option<Pick> {
    pick_in(figure, &FigurePlan::new(figure), x, y, radius)
}

/// [`pick`] on a figure planned already.
///
/// `plan` must be the plan of the same figure (`FigurePlan::new(figure)`); a plan of another
/// figure (other plot ids) gives `None`. The points of every plot whose panel (grown by
/// `radius`) holds the pixel are measured; a point is picked only where it is drawn, inside the
/// panel. The nearest wins, the element drawn last on a tie. A hidden element is the caller's
/// filter, not the crate's.
///
/// The hit area of a point is its pixel for a curve, a text and a heatmap cell (the cell), the
/// segments of the line of a curve which draws one (a segment picks its nearer end), the
/// vertical segment between the edges at its x for a band and an area, the rectangle of a bar,
/// of a histogram bin and, for a violin, the bounding rectangle of its density (its widest
/// width over its range of values), the rectangle of a box together with its whisker line, and
/// the line of a rule. A point in a rectangle has distance 0.
pub fn pick_in(figure: &Figure, plan: &FigurePlan, x: f64, y: f64, radius: f64) -> Option<Pick> {
    let boxes = &plan.0.layout.plots;
    if !(x.is_finite() && y.is_finite() && radius >= 0.0)
        || boxes.len() != figure.plots.len()
        || boxes.iter().zip(&figure.plots).any(|(b, p)| b.id != p.id)
    {
        return None;
    }
    let mut best = Best {
        limit: radius,
        at: (x, y),
        plot: 0,
        element: 0,
        hit: None,
    };
    for (pi, ((plot, pb), prep)) in figure
        .plots
        .iter()
        .zip(boxes)
        .zip(&plan.0.prepared)
        .enumerate()
    {
        let r = pb.rect;
        if x < r.x - radius || x > r.right() + radius || y < r.y - radius || y > r.bottom() + radius
        {
            continue;
        }
        let map = Map {
            rect: r,
            xa: &prep.xa,
            ya: &prep.ya,
            y2a: prep.y2a.as_ref(),
        };
        for (ei, e) in plot.elements.iter().enumerate() {
            let ya = match e.axis {
                YAxisRef::Y => Some(map.ya),
                YAxisRef::Y2 => map.y2a,
            };
            let Some(ya) = ya else { continue };
            // the marks in fractions of the panel are no data (since 0.33.0)
            if e.coordinates == crate::spec::Coordinates::Axes {
                continue;
            }
            best.plot = pi;
            best.element = ei;
            let line = e.style.line_or_default();
            let lined = line.kind != LineType::None && line.width > 0.0;
            measure(&e.kind, lined, &Map { ya, ..map }, &mut best);
        }
    }
    let hit = best.hit?;
    let plot = &figure.plots[hit.plot];
    let e = &plot.elements[hit.element];
    Some(Pick {
        plot: plot.id.clone(),
        element: e.id.clone(),
        legend: e.legend_group().map(str::to_string),
        index: hit.index,
        values: hit.values,
        label: hit.label.or_else(|| e.label.clone()),
    })
}

/// The best point so far.
struct Hit {
    plot: usize,
    element: usize,
    index: usize,
    values: BTreeMap<String, f64>,
    label: Option<String>,
}

/// The search: the pointer, the largest distance still of interest (the radius, then the
/// distance of the best point), and the plot and element being measured.
struct Best {
    limit: f64,
    at: (f64, f64),
    plot: usize,
    element: usize,
    hit: Option<Hit>,
}

impl Best {
    /// Offers a point at a pixel distance; its data is built only when it is the best so far.
    fn offer(
        &mut self,
        distance: f64,
        index: usize,
        build: impl FnOnce() -> (BTreeMap<String, f64>, Option<String>),
    ) {
        if distance <= self.limit {
            self.limit = distance;
            let (values, label) = build();
            self.hit = Some(Hit {
                plot: self.plot,
                element: self.element,
                index,
                values,
                label,
            });
        }
    }
}

/// The panel with the axes a point of an element is measured on.
#[derive(Clone, Copy)]
struct Map<'a> {
    rect: Rect,
    xa: &'a ResolvedAxis,
    ya: &'a ResolvedAxis,
    y2a: Option<&'a ResolvedAxis>,
}

impl Map<'_> {
    /// The pixel x of a coordinate of the x axis.
    fn px(&self, u: f64) -> f64 {
        self.rect.x + self.xa.share(u) * self.rect.width
    }

    /// The pixel y of a coordinate of the y axis.
    fn py(&self, u: f64) -> f64 {
        self.rect.bottom() - self.ya.share(u) * self.rect.height
    }

    /// The pixel of a data point, `None` where the axes cannot show it or it lies outside
    /// the panel (a point beyond the ends of an axis is not drawn).
    fn point(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (px, py) = (self.px(self.xa.u(x)), self.py(self.ya.u(y)));
        self.inside(px, py).then_some((px, py))
    }

    fn inside(&self, px: f64, py: f64) -> bool {
        const EPS: f64 = 1e-6;
        let r = self.rect;
        px.is_finite()
            && py.is_finite()
            && px >= r.x - EPS
            && px <= r.right() + EPS
            && py >= r.y - EPS
            && py <= r.bottom() + EPS
    }
}

/// The error below and above the value `i` of errors, NaN for none.
fn e_at(e: &Option<Errors>, i: usize) -> (f64, f64) {
    e.as_ref().map_or((f64::NAN, f64::NAN), |e| e.at(i))
}

/// The pairs of a finite value.
fn finite(pairs: Vec<(String, f64)>) -> impl Iterator<Item = (String, f64)> {
    pairs.into_iter().filter(|(_, v)| v.is_finite())
}

fn values(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .filter(|(_, v)| v.is_finite())
        .map(|(k, v)| ((*k).to_string(), *v))
        .collect()
}

/// The distance of a pixel to the segment `a`-`b`.
fn to_segment(at: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    segment_foot(at, a, b).0
}

/// The distance of a pixel to a segment and the point of the segment nearest to it.
fn segment_foot((px, py): (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, (f64, f64)) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let foot = (a.0 + t * dx, a.1 + t * dy);
    ((px - foot.0).hypot(py - foot.1), foot)
}

/// Offers the segments of the line of a curve: a segment joins two consecutive points whose
/// pixels are finite (a gap or a value a log axis cannot show breaks the line, as it is drawn),
/// is measured where it is drawn (its nearest point inside the panel) and picks its nearer end,
/// an end inside the panel before one outside, so a pointer on the line between two sparse
/// points picks the curve.
fn curve_segments(
    x: &[f64],
    y: &[f64],
    m: &Map,
    best: &mut Best,
    build: impl Fn(usize) -> (BTreeMap<String, f64>, Option<String>),
) {
    let at = best.at;
    let pixel = |i: usize| {
        let p = (m.px(m.xa.u(x[i])), m.py(m.ya.u(y[i])));
        (p.0.is_finite() && p.1.is_finite()).then_some(p)
    };
    for i in 1..x.len().min(y.len()) {
        let (Some(a), Some(b)) = (pixel(i - 1), pixel(i)) else {
            continue;
        };
        if at.0 < a.0.min(b.0) - best.limit || at.0 > a.0.max(b.0) + best.limit {
            continue;
        }
        let (distance, foot) = segment_foot(at, a, b);
        if distance > best.limit || !m.inside(foot.0, foot.1) {
            continue;
        }
        let rank = |p: (f64, f64)| (!m.inside(p.0, p.1), (at.0 - p.0).hypot(at.1 - p.1));
        let end = if rank(b) < rank(a) { i } else { i - 1 };
        best.offer(distance, end, || build(end));
    }
}

/// The distance of a pixel to a rectangle given by two opposite corners, 0 inside.
fn to_rect((px, py): (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = (a.0.min(b.0) - px).max(0.0).max(px - a.0.max(b.0));
    let dy = (a.1.min(b.1) - py).max(0.0).max(py - a.1.max(b.1));
    dx.hypot(dy)
}

/// Whether a rectangle (two opposite corners) overlaps the panel.
fn overlaps(m: &Map, a: (f64, f64), b: (f64, f64)) -> bool {
    let r = m.rect;
    [a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite())
        && a.0.max(b.0) >= r.x
        && a.0.min(b.0) <= r.right()
        && a.1.max(b.1) >= r.y
        && a.1.min(b.1) <= r.bottom()
}

/// Offers the points of an element to the search; `lined` when the element draws its line.
fn measure(kind: &ElementKind, lined: bool, m: &Map, best: &mut Best) {
    let at = best.at;
    let no_label = None::<String>;
    // the distance to the rectangle of an area-like mark, corners in data, `None` off the panel
    let area = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let (a, b) = (
            (m.px(m.xa.u(x0)), m.py(m.ya.u(y0))),
            (m.px(m.xa.u(x1)), m.py(m.ya.u(y1))),
        );
        overlaps(m, a, b).then(|| to_rect(at, a, b))
    };
    // the error of a symmetric error bar under its name, the sides of one of two sides under
    // `<name>_low` and `<name>_high` (since 0.33.0)
    let errors = |name: &'static str, e: &Option<Errors>, i: usize| -> Vec<(String, f64)> {
        match e {
            Some(Errors::Symmetric(_)) => vec![(name.to_string(), e_at(e, i).1)],
            Some(Errors::Sides { .. }) => {
                let (lo, hi) = e_at(e, i);
                vec![(format!("{name}_low"), lo), (format!("{name}_high"), hi)]
            }
            None => Vec::new(),
        }
    };
    let err = |e: &Option<Vec<f64>>, i: usize| {
        e.as_ref()
            .and_then(|e| e.get(i))
            .copied()
            .unwrap_or(f64::NAN)
    };
    match kind {
        ElementKind::Curve {
            x, y, xerr, yerr, ..
        } => {
            for (i, (xv, yv)) in x.iter().zip(y).enumerate() {
                let px = m.px(m.xa.u(*xv));
                if !near(px, at.0, best.limit) {
                    continue;
                }
                let Some(p) = m.point(*xv, *yv) else { continue };
                best.offer((at.0 - p.0).hypot(at.1 - p.1), i, || {
                    let mut v = values(&[("x", *xv), ("y", *yv)]);
                    v.extend(finite(errors("xerr", xerr, i)));
                    v.extend(finite(errors("yerr", yerr, i)));
                    (v, no_label.clone())
                });
            }
            if lined {
                curve_segments(x, y, m, best, |i| {
                    let mut v = values(&[("x", x[i]), ("y", y[i])]);
                    v.extend(finite(errors("xerr", xerr, i)));
                    v.extend(finite(errors("yerr", yerr, i)));
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Band {
            x,
            low,
            high,
            median,
        } => {
            for (i, ((xv, lo), hi)) in x.iter().zip(low).zip(high).enumerate() {
                let px = m.px(m.xa.u(*xv));
                if !near(px, at.0, best.limit) {
                    continue;
                }
                let (Some(a), Some(b)) = (m.point(*xv, *lo), m.point(*xv, *hi)) else {
                    continue;
                };
                best.offer(to_segment(at, a, b), i, || {
                    let v = values(&[
                        ("x", *xv),
                        ("low", *lo),
                        ("high", *hi),
                        ("median", err(median, i)),
                    ]);
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Area { x, from, to } => {
            for (i, ((xv, f), t)) in x.iter().zip(from).zip(to).enumerate() {
                let px = m.px(m.xa.u(*xv));
                if !near(px, at.0, best.limit) {
                    continue;
                }
                let (Some(a), Some(b)) = (m.point(*xv, *f), m.point(*xv, *t)) else {
                    continue;
                };
                best.offer(to_segment(at, a, b), i, || {
                    let v = values(&[
                        ("x", *xv),
                        ("from", *f),
                        ("to", *t),
                        ("low", *f),
                        ("high", *t),
                    ]);
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Bar {
            x,
            height,
            err: errs,
            horizontal,
            width,
            base: bases,
            ..
        } => {
            // the bars run along the axis of their positions and across the axis of heights
            let (along, across) = if *horizontal {
                (m.ya, m.xa)
            } else {
                (m.xa, m.ya)
            };
            let w = width.unwrap_or(BAR_WIDTH * spacing(x));
            let zero = base_of(across);
            for (i, (p, h)) in x.iter().zip(height).enumerate() {
                // a bar of a base runs from it (since 0.33.0)
                let from = bases.as_ref().map_or(0.0, |b| b[i]);
                let base = match across.u(from) {
                    u if u.is_finite() && bases.is_some() => u,
                    _ => zero,
                };
                let (a, b, top) = (
                    along.u(p - w / 2.0),
                    along.u(p + w / 2.0),
                    across.u(from + *h),
                );
                let (c0, c1) = if *horizontal {
                    ((m.px(base), m.py(a)), (m.px(top), m.py(b)))
                } else {
                    ((m.px(a), m.py(base)), (m.px(b), m.py(top)))
                };
                if !overlaps(m, c0, c1) {
                    continue;
                }
                best.offer(to_rect(at, c0, c1), i, || {
                    let mut v = values(&[("x", *p), ("height", *h)]);
                    if bases.is_some() {
                        v.insert("base".into(), from);
                    }
                    v.extend(finite(errors("err", errs, i)));
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Box {
            positions,
            stats,
            width,
            ..
        } => {
            let w = crate::render::stats_marks::box_width(positions, *width);
            for (i, (p, b)) in positions.iter().zip(stats).enumerate() {
                let (q1, q3) = box_body(m.ya, b.q1, b.q3);
                let (a, c) = (
                    (m.px(m.xa.u(p - w / 2.0)), m.py(q1)),
                    (m.px(m.xa.u(p + w / 2.0)), m.py(q3)),
                );
                let Some(r) = overlaps(m, a, c).then(|| to_rect(at, a, c)) else {
                    continue;
                };
                // the whisker line belongs to the box
                let whisker = match (m.point(*p, b.low_whisker), m.point(*p, b.high_whisker)) {
                    (Some(lo), Some(hi)) => to_segment(at, lo, hi),
                    _ => f64::INFINITY,
                };
                best.offer(r.min(whisker), i, || {
                    let v = values(&[
                        ("x", *p),
                        ("low_whisker", b.low_whisker),
                        ("q1", b.q1),
                        ("median", b.median),
                        ("q3", b.q3),
                        ("high_whisker", b.high_whisker),
                    ]);
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Violin {
            positions,
            densities,
        } => {
            let half = VIOLIN_WIDTH * spacing(positions) / 2.0;
            for (i, (p, d)) in positions.iter().zip(densities).enumerate() {
                let (lo, hi) = d
                    .values
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                        (a.min(v), b.max(v))
                    });
                if lo > hi {
                    continue;
                }
                if let Some(r) = area(p - half, lo, p + half, hi) {
                    best.offer(r, i, || (values(&[("x", *p)]), no_label.clone()));
                }
            }
        }
        ElementKind::Histogram {
            edges,
            counts,
            base: bases,
        } => {
            let zero = base_of(m.ya);
            for (i, (c, e)) in counts.iter().zip(edges.windows(2)).enumerate() {
                let from = bases.as_ref().map_or(0.0, |b| b[i]);
                let base = match m.ya.u(from) {
                    u if u.is_finite() && bases.is_some() => u,
                    _ => zero,
                };
                let (a, b) = (
                    (m.px(m.xa.u(e[0])), m.py(base)),
                    (m.px(m.xa.u(e[1])), m.py(m.ya.u(from + *c))),
                );
                if !overlaps(m, a, b) {
                    continue;
                }
                best.offer(to_rect(at, a, b), i, || {
                    let v = values(&[("low", e[0]), ("high", e[1]), ("count", *c)]);
                    (v, no_label.clone())
                });
            }
        }
        ElementKind::Heatmap {
            rows,
            columns,
            values: cells,
            x_edges,
            y_edges,
            ..
        } => {
            let (x_edges, y_edges) = (x_edges.as_deref(), y_edges.as_deref());
            let (n_rows, n_columns) = heatmap_size(rows, columns, x_edges, y_edges);
            for (i, row) in cells.iter().enumerate().take(n_rows) {
                for (j, v) in row.iter().enumerate().take(n_columns) {
                    if !v.is_finite() {
                        continue;
                    }
                    let ((x0, x1), (y0, y1)) = (cell_span(x_edges, j), cell_span(y_edges, i));
                    let Some(d) = area(x0, y0, x1, y1) else {
                        continue;
                    };
                    best.offer(d, i * n_columns + j, || {
                        // the names of the cell, and the edges along an axis with edges
                        let names: Vec<&str> = [rows.get(i), columns.get(j)]
                            .into_iter()
                            .flatten()
                            .map(String::as_str)
                            .collect();
                        let label = (!names.is_empty()).then(|| names.join(" / "));
                        let mut pairs = vec![("value", *v)];
                        if x_edges.is_some() {
                            pairs.extend([("x_low", x0), ("x_high", x1)]);
                        }
                        if y_edges.is_some() {
                            pairs.extend([("y_low", y0), ("y_high", y1)]);
                        }
                        (values(&pairs), label)
                    });
                }
            }
        }
        // a text at a location and an arrow are no data
        ElementKind::Text { x: None, .. }
        | ElementKind::Text { y: None, .. }
        | ElementKind::Arrow { .. }
        | ElementKind::Contour { .. } => {}
        ElementKind::Text {
            x: Some(x),
            y: Some(y),
            ..
        } => {
            if let Some(p) = m.point(*x, *y) {
                best.offer((at.0 - p.0).hypot(at.1 - p.1), 0, || {
                    (values(&[("x", *x), ("y", *y)]), no_label.clone())
                });
            }
        }
        ElementKind::Rule { orientation, value } => {
            let (dist, on_panel) = match orientation {
                Orientation::Horizontal => {
                    let py = m.py(m.ya.u(*value));
                    ((at.1 - py).abs(), m.inside(m.rect.x, py))
                }
                Orientation::Vertical => {
                    let px = m.px(m.xa.u(*value));
                    ((at.0 - px).abs(), m.inside(px, m.rect.y))
                }
            };
            if on_panel {
                best.offer(dist, 0, || (values(&[("value", *value)]), no_label.clone()));
            }
        }
        ElementKind::Polygon { .. } => {}
    }
}

/// Whether a pixel coordinate is within `limit` of the pointer's (false for NaN).
fn near(p: f64, at: f64, limit: f64) -> bool {
    (p - at).abs() <= limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const TWO_LEGENDS: &str = r#"{"contract": "figure/1", "width": 600, "height": 400,
        "grid": {"rows": 1, "columns": 2},
        "plots": [
          {"id": "p", "row": 0, "column": 0, "x": {}, "y": {}, "elements": [
            {"id": "a", "label": "A", "kind": "curve", "x": [0, 1], "y": [0, 1]},
            {"id": "b", "label": "B", "kind": "curve", "x": [0, 1], "y": [1, 0]}]},
          {"id": "q", "row": 0, "column": 1, "x": {}, "y": {}, "elements": [
            {"id": "c", "label": "C", "kind": "curve", "x": [0, 1], "y": [0, 2]}]}]}"#;

    #[test]
    fn geometry_matches_layout() {
        let f = parse(TWO_LEGENDS).unwrap_or_else(|e| panic!("{e}"));
        let plan = layout::plan(&f);
        let g = geometry(&f);
        assert_eq!(g.plots.len(), plan.layout.plots.len());
        for (gp, lp) in g.plots.iter().zip(&plan.layout.plots) {
            assert_eq!(gp.id, lp.id);
            assert_eq!(gp.rect, PixelRect::from(lp.rect));
            assert_eq!(gp.x, Scale::from(&lp.x));
            assert_eq!(gp.y, Scale::from(&lp.y));
            assert_eq!(gp.y2, lp.y2.as_ref().map(Scale::from));
        }
    }

    #[test]
    fn legend_entry_rects() {
        let f = parse(TWO_LEGENDS).unwrap_or_else(|e| panic!("{e}"));
        let plan = layout::plan(&f);
        let g = geometry(&f);
        let n: usize = plan.layout.legends.iter().map(|l| l.entries.len()).sum();
        assert_eq!(g.legend_entries.len(), n);
        assert!(n >= 3);
        let mut seen = g.legend_entries.iter();
        for lb in &plan.layout.legends {
            let frame = lb.rect;
            for e in &lb.entries {
                let r = seen.next().unwrap().rect;
                assert_eq!(r, PixelRect::from(e.rect));
                assert!(
                    r.x >= frame.x - 1e-6 && r.y >= frame.y - 1e-6,
                    "{r:?} in {frame:?}"
                );
                assert!(r.x + r.width <= frame.right() + 1e-6);
                assert!(r.y + r.height <= frame.bottom() + 1e-6);
            }
        }
    }
}

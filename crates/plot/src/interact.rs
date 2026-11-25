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
use crate::render::stats_marks::{BAR_WIDTH, BOX_WIDTH, VIOLIN_WIDTH, base_of, spacing};
use crate::spec::{AxisScale, ElementKind, Figure, Orientation, YAxisRef};

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
    /// The id of the plot whose legend it is, or `row:<n>` for the legend of a row.
    pub owner: String,
    /// The rectangle of its swatch and label.
    pub rect: PixelRect,
}

/// Where the panels and the legend entries of a figure are, in its pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
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
    /// bin, `value` of a heatmap cell, `x`, `y` of a text, `value` of a rule.
    pub values: BTreeMap<String, f64>,
    /// The label of the element; `row / column` for a heatmap cell.
    pub label: Option<String>,
}

/// The geometry of a figure.
pub fn geometry(figure: &Figure) -> Geometry {
    geometry_of(&FigurePlan::new(figure))
}

/// The geometry of a figure planned already.
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
            };
            lb.entries.iter().map(move |e| LegendEntryGeometry {
                legend: e.legend.clone(),
                owner: owner.clone(),
                rect: e.rect.into(),
            })
        })
        .collect();
    Geometry {
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
/// The points of every plot whose panel (grown by `radius`) holds the pixel are measured; a
/// point is picked only where it is drawn, inside the panel. The nearest wins, the element
/// drawn last on a tie. A hidden element is the caller's filter, not the crate's.
pub fn pick_in(figure: &Figure, plan: &FigurePlan, x: f64, y: f64, radius: f64) -> Option<Pick> {
    if !(x.is_finite() && y.is_finite() && radius >= 0.0) {
        return None;
    }
    let mut best: Option<(f64, Pick)> = None;
    for ((plot, pb), prep) in figure
        .plots
        .iter()
        .zip(&plan.0.layout.plots)
        .zip(&plan.0.prepared)
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
        for e in &plot.elements {
            let ya = match e.axis {
                YAxisRef::Y => Some(map.ya),
                YAxisRef::Y2 => map.y2a,
            };
            let Some(ya) = ya else { continue };
            let m = Map { ya, ..map };
            for c in candidates(&e.kind, &m, (x, y)) {
                if c.distance <= radius && best.as_ref().is_none_or(|(d, _)| c.distance <= *d) {
                    let label = c.label.or_else(|| e.label.clone());
                    best = Some((
                        c.distance,
                        Pick {
                            plot: plot.id.clone(),
                            element: e.id.clone(),
                            legend: e.legend_group().map(str::to_string),
                            index: c.index,
                            values: c.values,
                            label,
                        },
                    ));
                }
            }
        }
    }
    best.map(|(_, p)| p)
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

/// A point of an element near the pointer.
struct Candidate {
    distance: f64,
    index: usize,
    values: BTreeMap<String, f64>,
    label: Option<String>,
}

fn values(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs
        .iter()
        .filter(|(_, v)| v.is_finite())
        .map(|(k, v)| ((*k).to_string(), *v))
        .collect()
}

/// The distance of a pixel to the segment `a`-`b`.
fn to_segment((px, py): (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (px - (a.0 + t * dx)).hypot(py - (a.1 + t * dy))
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

/// The points of an element with their distance to the pointer.
fn candidates(kind: &ElementKind, m: &Map, at: (f64, f64)) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut add = |distance: f64, index: usize, v: BTreeMap<String, f64>, label: Option<String>| {
        if distance.is_finite() {
            out.push(Candidate {
                distance,
                index,
                values: v,
                label,
            });
        }
    };
    // the distance to the rectangle of an area-like mark, `(x0, y0)` and `(x1, y1)` in data
    let area = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let (a, b) = (
            (m.px(m.xa.u(x0)), m.py(m.ya.u(y0))),
            (m.px(m.xa.u(x1)), m.py(m.ya.u(y1))),
        );
        overlaps(m, a, b).then(|| to_rect(at, a, b))
    };
    match kind {
        ElementKind::Curve { x, y, xerr, yerr } => {
            for (i, (xv, yv)) in x.iter().zip(y).enumerate() {
                let Some(p) = m.point(*xv, *yv) else { continue };
                let err = |e: &Option<Vec<f64>>| e.as_ref().and_then(|e| e.get(i)).copied();
                let v = values(&[
                    ("x", *xv),
                    ("y", *yv),
                    ("xerr", err(xerr).unwrap_or(f64::NAN)),
                    ("yerr", err(yerr).unwrap_or(f64::NAN)),
                ]);
                add((at.0 - p.0).hypot(at.1 - p.1), i, v, None);
            }
        }
        ElementKind::Band {
            x,
            low,
            high,
            median,
        } => {
            for (i, ((xv, lo), hi)) in x.iter().zip(low).zip(high).enumerate() {
                let (a, b) = (m.point(*xv, *lo), m.point(*xv, *hi));
                let (Some(a), Some(b)) = (a, b) else { continue };
                let med = median.as_ref().and_then(|v| v.get(i)).copied();
                let v = values(&[
                    ("x", *xv),
                    ("low", *lo),
                    ("high", *hi),
                    ("median", med.unwrap_or(f64::NAN)),
                ]);
                add(to_segment(at, a, b), i, v, None);
            }
        }
        ElementKind::Area { x, from, to } => {
            for (i, ((xv, f), t)) in x.iter().zip(from).zip(to).enumerate() {
                let (a, b) = (m.point(*xv, *f), m.point(*xv, *t));
                let (Some(a), Some(b)) = (a, b) else { continue };
                let v = values(&[
                    ("x", *xv),
                    ("from", *f),
                    ("to", *t),
                    ("low", *f),
                    ("high", *t),
                ]);
                add(to_segment(at, a, b), i, v, None);
            }
        }
        ElementKind::Bar {
            x,
            height,
            err,
            horizontal,
            width,
        } => {
            let (along, across) = if *horizontal {
                (m.ya, m.xa)
            } else {
                (m.xa, m.ya)
            };
            let w = width.unwrap_or(BAR_WIDTH * spacing(x));
            let base = base_of(across);
            for (i, (p, h)) in x.iter().zip(height).enumerate() {
                let (a, b, top) = (along.u(p - w / 2.0), along.u(p + w / 2.0), across.u(*h));
                let (c0, c1) = if *horizontal {
                    ((m.px(base), m.py(a)), (m.px(top), m.py(b)))
                } else {
                    ((m.px(a), m.py(base)), (m.px(b), m.py(top)))
                };
                if !overlaps(m, c0, c1) {
                    continue;
                }
                let e = err.as_ref().and_then(|e| e.get(i)).copied();
                let v = values(&[("x", *p), ("height", *h), ("err", e.unwrap_or(f64::NAN))]);
                add(to_rect(at, c0, c1), i, v, None);
            }
        }
        ElementKind::Box { positions, stats } => {
            let w = BOX_WIDTH * spacing(positions);
            for (i, (p, b)) in positions.iter().zip(stats).enumerate() {
                let Some(r) = area(p - w / 2.0, b.q1, p + w / 2.0, b.q3) else {
                    continue;
                };
                let v = values(&[
                    ("x", *p),
                    ("low_whisker", b.low_whisker),
                    ("q1", b.q1),
                    ("median", b.median),
                    ("q3", b.q3),
                    ("high_whisker", b.high_whisker),
                ]);
                // the whiskers belong to the box: the nearest of the box and its whisker line
                let whisker = {
                    let (lo, hi) = (m.point(*p, b.low_whisker), m.point(*p, b.high_whisker));
                    match (lo, hi) {
                        (Some(lo), Some(hi)) => to_segment(at, lo, hi),
                        _ => f64::INFINITY,
                    }
                };
                add(r.min(whisker), i, v, None);
            }
        }
        ElementKind::Violin {
            positions,
            densities,
        } => {
            let half = VIOLIN_WIDTH * spacing(positions) / 2.0;
            for (i, (p, d)) in positions.iter().zip(densities).enumerate() {
                let vals: Vec<f64> = d.values.iter().copied().filter(|v| v.is_finite()).collect();
                let (lo, hi) = vals
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                        (a.min(*v), b.max(*v))
                    });
                if lo > hi {
                    continue;
                }
                if let Some(r) = area(p - half, lo, p + half, hi) {
                    add(r, i, values(&[("x", *p)]), None);
                }
            }
        }
        ElementKind::Histogram { edges, counts } => {
            let base = base_of(m.ya);
            for (i, (c, e)) in counts.iter().zip(edges.windows(2)).enumerate() {
                let (a, b) = (
                    (m.px(m.xa.u(e[0])), m.py(base)),
                    (m.px(m.xa.u(e[1])), m.py(m.ya.u(*c))),
                );
                if !overlaps(m, a, b) {
                    continue;
                }
                let v = values(&[("low", e[0]), ("high", e[1]), ("count", *c)]);
                add(to_rect(at, a, b), i, v, None);
            }
        }
        ElementKind::Heatmap {
            rows,
            columns,
            values: cells,
            ..
        } => {
            for (i, row) in cells.iter().enumerate().take(rows.len()) {
                for (j, v) in row.iter().enumerate().take(columns.len()) {
                    if !v.is_finite() {
                        continue;
                    }
                    let (r, c) = (i as f64, j as f64);
                    let Some(d) = area(c - 0.5, r - 0.5, c + 0.5, r + 0.5) else {
                        continue;
                    };
                    let label = format!("{} / {}", rows[i], columns[j]);
                    add(
                        d,
                        i * columns.len() + j,
                        values(&[("value", *v)]),
                        Some(label),
                    );
                }
            }
        }
        ElementKind::Text { x, y, .. } => {
            if let Some(p) = m.point(*x, *y) {
                add(
                    (at.0 - p.0).hypot(at.1 - p.1),
                    0,
                    values(&[("x", *x), ("y", *y)]),
                    None,
                );
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
                add(dist, 0, values(&[("value", *value)]), None);
            }
        }
        ElementKind::Polygon { .. } => {}
    }
    out
}

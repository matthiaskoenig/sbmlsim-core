//! The marks of the elements of a plot as layers of Charton: curves, bands, areas, rules and
//! text.
//!
//! The crate prepares what Charton draws: every value becomes the coordinate of its axis
//! (`ResolvedAxis::u`), a value the axis cannot show splits a line, a band or an area into
//! runs (one path or polygon per run, never a NaN), a band and an area are closed polygons
//! (Charton's area has no lower edge), error bars are open paths of one subpath per point (no
//! caps, matplotlib's default), and a rule is a line from end to end of the other axis. Every
//! layer is recorded with the element it draws, so the writer groups the layers of one element.
//!
//! The marker of a curve is drawn by the writer at every point Charton places, so every shape of
//! the spec is drawn the same way at the same size.

use charton::error::ChartonError;
use charton::prelude::*;

use super::axis::ResolvedAxis;
use crate::spec::{Element, ElementKind, Fonts, LineType, MarkerType, Orientation, Style};
use crate::svg::{LayerInfo, MarkerGlyph, theme};

/// The opacity of a band or an area without a fill of its own (`band-opacity` of sbmlsim-web).
pub(crate) const BAND_OPACITY: f64 = 0.18;
/// The width of an error bar in pixels (`data-width` of sbmlsim-web).
pub(crate) const ERROR_WIDTH: f64 = 1.0;

/// The layers of a plot and what each of them draws.
pub(crate) struct Layers {
    /// The chart of the layers, its first layer the scaffold which sets the scales.
    pub chart: LayeredChart,
    /// What each layer draws, in the order of the layers.
    pub info: Vec<LayerInfo>,
}

impl Layers {
    /// Adds a layer of an element.
    fn push(&mut self, layer: impl Into<LayeredChart>, info: LayerInfo) {
        let chart = std::mem::take(&mut self.chart);
        self.chart = chart.and(layer);
        self.info.push(info);
    }
}

/// A dataset of columns.
fn dataset(columns: Vec<(&str, ColumnVector)>) -> Result<Dataset, ChartonError> {
    columns.to_dataset()
}

/// The runs of consecutive indices for which `ok` holds.
fn runs(n: usize, ok: impl Fn(usize) -> bool) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !ok(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && ok(i) {
            i += 1;
        }
        out.push(start..i);
    }
    out
}

/// The dash pattern of a line type at a width, empty for a solid line.
fn dash(kind: LineType, width: f64) -> Vec<f64> {
    let w = width.max(1.0);
    match kind {
        LineType::Dashed => vec![3.7 * w, 1.6 * w],
        LineType::Dotted => vec![w, 1.65 * w],
        LineType::Dashdot => vec![6.4 * w, 1.6 * w, w, 1.6 * w],
        LineType::Solid | LineType::None => Vec::new(),
    }
}

/// The paint of a line: colour, width, dash and opacity.
#[derive(Clone)]
struct Stroke {
    color: String,
    width: f64,
    dash: Vec<f64>,
    opacity: f64,
}

impl Stroke {
    /// The stroke of the line of a style, `None` for a style without a line.
    fn of_line(style: &Style, color: &str) -> Option<Stroke> {
        (style.line.kind != LineType::None && style.line.width > 0.0).then(|| Stroke {
            color: color.to_string(),
            width: style.line.width,
            dash: dash(style.line.kind, style.line.width),
            opacity: style.opacity,
        })
    }
}

/// The name of a path group or subpath: Charton groups by a discrete column.
fn group_name(k: usize) -> String {
    k.to_string()
}

/// The open paths through points, one path group per run, as a layer.
fn path_layer(
    runs_of: &[(Vec<f64>, Vec<f64>)],
    stroke: &Stroke,
    subpaths: bool,
) -> Result<Option<Chart<MarkPath>>, ChartonError> {
    let (mut xs, mut ys, mut gs, mut ss) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (k, (x, y)) in runs_of.iter().enumerate() {
        if x.len() < 2 {
            continue;
        }
        xs.extend_from_slice(x);
        ys.extend_from_slice(y);
        // one path with a subpath per run, or a path per run
        let (g, s) = if subpaths { (0, k) } else { (k, 0) };
        gs.extend(std::iter::repeat_n(group_name(g), x.len()));
        ss.extend(std::iter::repeat_n(group_name(s), x.len()));
    }
    if xs.is_empty() {
        return Ok(None);
    }
    let ds = dataset(vec![
        ("x", xs.into_column()),
        ("y", ys.into_column()),
        ("g", gs.into_column()),
        ("s", ss.into_column()),
    ])?;
    let s = stroke.clone();
    let chart = Chart::build(&ds)?
        .mark_path()?
        .configure_path(move |p| {
            p.with_stroke(s.color.as_str())
                .with_stroke_width(s.width)
                .with_opacity(s.opacity)
                .with_dash(s.dash)
        })
        .encode((
            alt::x("x"),
            alt::y("y"),
            alt::path_group("g").with_subpath("s"),
        ))?;
    Ok(Some(chart))
}

/// The runs of the finite points of a line through `(x, y)` in coordinates.
fn line_runs(xu: &[f64], yu: &[f64]) -> Vec<(Vec<f64>, Vec<f64>)> {
    runs(xu.len(), |i| xu[i].is_finite() && yu[i].is_finite())
        .into_iter()
        .map(|r| (xu[r.clone()].to_vec(), yu[r].to_vec()))
        .collect()
}

/// The closed outlines between two edges, one polygon per run of finite values.
fn polygon_layer(
    xu: &[f64],
    lo: &[f64],
    hi: &[f64],
    fill: &str,
    opacity: f64,
) -> Result<Option<Chart<MarkPolygon>>, ChartonError> {
    let (mut px, mut py, mut pg) = (Vec::new(), Vec::new(), Vec::new());
    let ok = |i: usize| xu[i].is_finite() && lo[i].is_finite() && hi[i].is_finite();
    for (k, r) in runs(xu.len(), ok).into_iter().enumerate() {
        if r.len() < 2 {
            continue;
        }
        for i in r.clone() {
            px.push(xu[i]);
            py.push(hi[i]);
            pg.push(group_name(k));
        }
        for i in r.rev() {
            px.push(xu[i]);
            py.push(lo[i]);
            pg.push(group_name(k));
        }
    }
    if px.is_empty() {
        return Ok(None);
    }
    let ds = dataset(vec![
        ("x", px.into_column()),
        ("y", py.into_column()),
        ("g", pg.into_column()),
    ])?;
    let fill = fill.to_string();
    let chart = Chart::build(&ds)?
        .mark_polygon()?
        .configure_polygon(move |p| {
            p.with_fill(fill.as_str())
                .with_opacity(opacity)
                .with_stroke("none")
                .with_stroke_width(0.0)
        })
        .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    Ok(Some(chart))
}

/// The fill colour and opacity of a band or an area.
fn fill_of(style: &Style, color: &str) -> (String, f64) {
    match &style.fill {
        Some(f) => (
            f.color.clone().unwrap_or_else(|| color.to_string()),
            f.opacity * style.opacity,
        ),
        None => (color.to_string(), BAND_OPACITY * style.opacity),
    }
}

/// The error bars of a curve along one axis: a segment per point from `v - e` to `v + e`.
///
/// `along` is the axis of the error, `across` the coordinates of the other axis; an end the axis
/// cannot show (below 0 on a logarithmic axis) is the lower end of the axis.
fn error_runs(
    values: &[f64],
    errors: &[f64],
    across: &[f64],
    along: &ResolvedAxis,
    horizontal: bool,
) -> Vec<(Vec<f64>, Vec<f64>)> {
    let start = along.u(along.low);
    let mut out = Vec::new();
    for i in 0..values.len() {
        let (v, e, c) = (values[i], errors[i], across[i]);
        if !(v.is_finite() && e.is_finite() && c.is_finite() && along.u(v).is_finite()) {
            continue;
        }
        let end = |w: f64| {
            let u = along.u(w);
            if u.is_finite() { u } else { start }
        };
        let (a, b) = (end(v - e.abs()), end(v + e.abs()));
        if horizontal {
            out.push((vec![a, b], vec![c, c]));
        } else {
            out.push((vec![c, c], vec![a, b]));
        }
    }
    out
}

/// The data of a curve: x, y and the errors of x and of y.
type CurveData<'a> = (&'a [f64], &'a [f64], Option<&'a [f64]>, Option<&'a [f64]>);

/// The layers of a curve: its line, its error bars and its markers.
fn curve_layers(
    layers: &mut Layers,
    index: usize,
    data: CurveData<'_>,
    style: &Style,
    color: &str,
    axes: (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let (x, y, xerr, yerr) = data;
    let (xa, ya) = axes;
    let xu: Vec<f64> = x.iter().map(|v| xa.u(*v)).collect();
    let yu: Vec<f64> = y.iter().map(|v| ya.u(*v)).collect();
    let info = || LayerInfo {
        element: Some(index),
        marker: None,
    };
    if let Some(stroke) = Stroke::of_line(style, color)
        && let Some(l) = path_layer(&line_runs(&xu, &yu), &stroke, false)?
    {
        layers.push(l, info());
    }
    let bar = Stroke {
        color: color.to_string(),
        width: ERROR_WIDTH,
        dash: Vec::new(),
        opacity: style.opacity,
    };
    if let Some(e) = xerr
        && let Some(l) = path_layer(&error_runs(x, e, &yu, xa, true), &bar, true)?
    {
        layers.push(l, info());
    }
    if let Some(e) = yerr
        && let Some(l) = path_layer(&error_runs(y, e, &xu, ya, false), &bar, true)?
    {
        layers.push(l, info());
    }
    let m = &style.marker;
    if m.kind != MarkerType::None && m.size > 0.0 {
        let (px, py): (Vec<f64>, Vec<f64>) = xu
            .iter()
            .zip(&yu)
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| (*a, *b))
            .unzip();
        if !px.is_empty() {
            let ds = dataset(vec![("x", px.into_column()), ("y", py.into_column())])?;
            let radius = m.size / 2.0;
            let chart = Chart::build(&ds)?
                .mark_point()?
                .configure_point(move |p| p.with_size(radius))
                .encode((alt::x("x"), alt::y("y")))?;
            let glyph = MarkerGlyph {
                kind: m.kind,
                size: m.size,
                fill: m.fill.clone().unwrap_or_else(|| color.to_string()),
                edge: m.edge.clone().unwrap_or_else(|| color.to_string()),
                edge_width: m.edge_width,
                opacity: style.opacity,
            };
            layers.push(
                chart,
                LayerInfo {
                    element: Some(index),
                    marker: Some(glyph),
                },
            );
        }
    }
    Ok(())
}

/// Builds the layers of the elements of a plot.
///
/// `colors` is the colour of every element (its own or the default), `fonts` the font sizes of
/// the figure. Only elements of the left y axis are drawn; the kinds of Task 4 and the right
/// axis of Task 5 get their group and no layer yet.
pub(crate) fn build(
    elements: &[Element],
    colors: &[String],
    fonts: &Fonts,
    xa: &ResolvedAxis,
    ya: &ResolvedAxis,
) -> Result<Layers, ChartonError> {
    let (x0, x1) = xa.domain();
    let (y0, y1) = ya.domain();
    // the scaffold: the corners of the domain, drawn for the scales and written by no one
    let scaffold = Chart::build(&dataset(vec![
        ("x", vec![x0, x1].into_column()),
        ("y", vec![y0, y1].into_column()),
    ])?)?
    .mark_point()?
    .encode((alt::x("x"), alt::y("y")))?;
    let mut layers = Layers {
        chart: scaffold.into(),
        info: vec![LayerInfo {
            element: None,
            marker: None,
        }],
    };
    for (index, e) in elements.iter().enumerate() {
        if e.axis != crate::spec::YAxisRef::Y {
            continue;
        }
        let color = colors[index].as_str();
        let style = &e.style;
        let info = || LayerInfo {
            element: Some(index),
            marker: None,
        };
        match &e.kind {
            ElementKind::Curve { x, y, xerr, yerr } => {
                curve_layers(
                    &mut layers,
                    index,
                    (x, y, xerr.as_deref(), yerr.as_deref()),
                    style,
                    color,
                    (xa, ya),
                )?;
            }
            ElementKind::Band {
                x,
                low,
                high,
                median,
            } => {
                let xu: Vec<f64> = x.iter().map(|v| xa.u(*v)).collect();
                let lo: Vec<f64> = low.iter().map(|v| ya.u(*v)).collect();
                let hi: Vec<f64> = high.iter().map(|v| ya.u(*v)).collect();
                let (fill, opacity) = fill_of(style, color);
                if let Some(l) = polygon_layer(&xu, &lo, &hi, &fill, opacity)? {
                    layers.push(l, info());
                }
                if let Some(m) = median
                    && let Some(stroke) = Stroke::of_line(style, color)
                {
                    let mu: Vec<f64> = m.iter().map(|v| ya.u(*v)).collect();
                    if let Some(l) = path_layer(&line_runs(&xu, &mu), &stroke, false)? {
                        layers.push(l, info());
                    }
                }
            }
            ElementKind::Area { x, from, to } => {
                let xu: Vec<f64> = x.iter().map(|v| xa.u(*v)).collect();
                let lo: Vec<f64> = from.iter().map(|v| ya.u(*v)).collect();
                let hi: Vec<f64> = to.iter().map(|v| ya.u(*v)).collect();
                let (fill, opacity) = fill_of(style, color);
                if let Some(l) = polygon_layer(&xu, &lo, &hi, &fill, opacity)? {
                    layers.push(l, info());
                }
            }
            ElementKind::Rule { orientation, value } => {
                let Some(stroke) = Stroke::of_line(style, color) else {
                    continue;
                };
                let line = match orientation {
                    Orientation::Horizontal => {
                        let v = ya.u(*value);
                        (vec![x0, x1], vec![v, v])
                    }
                    Orientation::Vertical => {
                        let v = xa.u(*value);
                        (vec![v, v], vec![y0, y1])
                    }
                };
                if line.0.iter().chain(&line.1).all(|v| v.is_finite())
                    && let Some(l) = path_layer(&[line], &stroke, false)?
                {
                    layers.push(l, info());
                }
            }
            ElementKind::Text { x, y, text, anchor } => {
                let (u, v) = (xa.u(*x), ya.u(*y));
                if !(u.is_finite() && v.is_finite()) {
                    continue;
                }
                let ds = dataset(vec![
                    ("x", vec![u].into_column()),
                    ("y", vec![v].into_column()),
                    ("t", vec![text.clone()].into_column()),
                ])?;
                let anchor = match anchor {
                    crate::spec::Anchor::Start => "start",
                    crate::spec::Anchor::Middle => "middle",
                    crate::spec::Anchor::End => "end",
                };
                let (c, size, opacity) = (color.to_string(), fonts.tick, style.opacity);
                let chart = Chart::build(&ds)?
                    .mark_text()?
                    .configure_text(move |t| {
                        t.with_color(c.as_str())
                            .with_size(size)
                            .with_anchor(anchor)
                            .with_opacity(opacity)
                    })
                    .encode((alt::x("x"), alt::y("y"), alt::text("t")))?;
                layers.push(chart, info());
            }
            // the kinds of Task 4: a group without marks
            ElementKind::Bar { .. }
            | ElementKind::Box { .. }
            | ElementKind::Violin { .. }
            | ElementKind::Histogram { .. }
            | ElementKind::Heatmap { .. }
            | ElementKind::Polygon { .. } => {}
        }
    }
    Ok(layers)
}

/// The values an element puts on the x axis and on its y axis.
pub(crate) fn extent(e: &Element, xs: &mut Vec<f64>, ys: &mut Vec<f64>) {
    let with_errors = |out: &mut Vec<f64>, v: &[f64], err: Option<&[f64]>| {
        out.extend_from_slice(v);
        if let Some(err) = err {
            for (a, b) in v.iter().zip(err) {
                out.push(a - b.abs());
                out.push(a + b.abs());
            }
        }
    };
    match &e.kind {
        ElementKind::Curve { x, y, xerr, yerr } => {
            with_errors(xs, x, xerr.as_deref());
            with_errors(ys, y, yerr.as_deref());
        }
        ElementKind::Band {
            x,
            low,
            high,
            median,
        } => {
            xs.extend_from_slice(x);
            ys.extend_from_slice(low);
            ys.extend_from_slice(high);
            if let Some(m) = median {
                ys.extend_from_slice(m);
            }
        }
        ElementKind::Area { x, from, to } => {
            xs.extend_from_slice(x);
            ys.extend_from_slice(from);
            ys.extend_from_slice(to);
        }
        ElementKind::Text { x, y, .. } => {
            xs.push(*x);
            ys.push(*y);
        }
        ElementKind::Rule { orientation, value } => match orientation {
            Orientation::Horizontal => ys.push(*value),
            Orientation::Vertical => xs.push(*value),
        },
        _ => {}
    }
}

/// Whether an element without a colour of its own takes the next colour of the cycle; a rule
/// and a text are drawn in the text colour instead.
pub(crate) fn cycled(e: &Element) -> bool {
    !matches!(e.kind, ElementKind::Rule { .. } | ElementKind::Text { .. })
}

/// The default colour of an element which is not cycled.
pub(crate) const UNCYCLED: &str = theme::FG;

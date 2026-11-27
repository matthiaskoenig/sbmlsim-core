//! The marks of the elements of a plot as layers of Charton: curves, bands, areas, rules and
//! text here, the statistical kinds and polygons in [`super::stats_marks`] and heatmaps in
//! [`super::heatmap`].
//!
//! The crate prepares what Charton draws: every value becomes the coordinate of its axis
//! (`ResolvedAxis::u`), a value the axis cannot show splits a line, a band or an area into
//! runs (one path or polygon per run, never a NaN), a band and an area are closed polygons
//! (Charton's area has no lower edge), error bars are open paths of one subpath per point (no
//! caps by default, as matplotlib; with `error_cap` a cap of that length in pixels across each
//! end, since 0.30.0), and a rule is a line from end to end of the other axis. Every
//! layer is recorded with the element it draws, so the writer groups the layers of one element.
//!
//! The marker of a curve is drawn by the writer at every point Charton places, so every shape of
//! the spec is drawn the same way at the same size.

use charton::error::ChartonError;
use charton::prelude::*;

use super::axis::ResolvedAxis;
use super::cut::{Span, XEnds, cut_runs, linear};
use super::{heatmap, stats_marks};
use crate::spec::{
    Coordinates, Element, ElementKind, Errors, Fonts, Line, LineType, MarkerType, Orientation,
    Style,
};
use crate::svg::{LayerInfo, MarkerGlyph, RawSvg, theme};

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
    pub(crate) fn push(&mut self, layer: impl Into<LayeredChart>, info: LayerInfo) {
        let chart = std::mem::take(&mut self.chart);
        self.chart = chart.and(layer);
        self.info.push(info);
    }
}

/// The pixels of a panel (since 0.33.0): from its upper left corner, of the coordinates of its
/// axes, of values or of fractions of it.
#[derive(Clone, Copy)]
pub(crate) struct Pixels<'a> {
    /// The x axis.
    pub xa: &'a ResolvedAxis,
    /// The y axis.
    pub ya: &'a ResolvedAxis,
    /// The width of the panel.
    pub w: f64,
    /// The height of the panel.
    pub h: f64,
}

impl<'a> Pixels<'a> {
    /// The pixels of a panel of `(w, h)` with these axes.
    pub(crate) fn new((xa, ya): (&'a ResolvedAxis, &'a ResolvedAxis), (w, h): (f64, f64)) -> Self {
        Pixels { xa, ya, w, h }
    }

    /// The pixel of coordinates of the axes.
    pub(crate) fn of_u(&self, u: f64, v: f64) -> (f64, f64) {
        (self.w * self.xa.share(u), self.h * (1.0 - self.ya.share(v)))
    }

    /// The pixel of a position in values or in fractions of the panel.
    pub(crate) fn at(&self, c: Coordinates, x: f64, y: f64) -> (f64, f64) {
        match c {
            Coordinates::Data => self.of_u(self.xa.u(x), self.ya.u(y)),
            Coordinates::Axes => (self.w * x, self.h * (1.0 - y)),
        }
    }
}

/// Adds a layer the crate draws itself (since 0.33.0): a point at a corner of the domain for
/// Charton, which the writer replaces by the SVG.
pub(crate) fn raw_layer(
    layers: &mut Layers,
    index: usize,
    raw: RawSvg,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let (x0, _) = xa.domain();
    let (y0, _) = ya.domain();
    let chart = Chart::build(&dataset(vec![
        ("x", vec![x0].into_column()),
        ("y", vec![y0].into_column()),
    ])?)?
    .mark_point()?
    .encode((alt::x("x"), alt::y("y")))?;
    layers.push(
        chart,
        LayerInfo {
            element: Some(index),
            marker: None,
            raw: Some(raw),
        },
    );
    Ok(())
}

/// A dataset of columns.
pub(crate) fn dataset(columns: Vec<(&str, ColumnVector)>) -> Result<Dataset, ChartonError> {
    columns.to_dataset()
}

/// The dash pattern of a line type at a width, empty for a solid line.
pub(crate) fn dash(kind: LineType, width: f64) -> Vec<f64> {
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
pub(crate) struct Stroke {
    /// Colour `#rrggbb`.
    pub color: String,
    /// Width in pixels.
    pub width: f64,
    /// Dash pattern, empty for a solid line.
    pub dash: Vec<f64>,
    /// Opacity.
    pub opacity: f64,
}

impl Stroke {
    /// The stroke of the line of a style, `None` for a style without a line.
    pub(crate) fn of_line(style: &Style, color: &str) -> Option<Stroke> {
        Self::of(style.line_or_default(), style, color)
    }

    /// The stroke of a line in the colour and the opacity of a style, `None` for no line.
    pub(crate) fn of(line: Line, style: &Style, color: &str) -> Option<Stroke> {
        (line.kind != LineType::None && line.width > 0.0).then(|| Stroke {
            color: color.to_string(),
            width: line.width,
            dash: dash(line.kind, line.width),
            opacity: style.opacity,
        })
    }
}

/// The name of a path group or subpath: Charton groups by a discrete column.
pub(crate) fn group_name(k: usize) -> String {
    k.to_string()
}

/// The open paths through points, one path group per run, as a layer.
pub(crate) fn path_layer(
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

/// The runs of the finite points of a line through `(x, y)` in coordinates, cut to the x
/// domain `span` (see [`super::cut`]).
fn line_runs(xu: &[f64], yu: &[f64], span: Span) -> Vec<(Vec<f64>, Vec<f64>)> {
    cut_runs(xu, &[yu], span, linear)
        .into_iter()
        .map(|(x, mut columns)| (x, columns.swap_remove(0)))
        .collect()
}

/// The closed outlines between two edges, one polygon per run of finite values inside the x
/// domain `span`.
fn polygon_layer(
    xu: &[f64],
    (lo, hi): (&[f64], &[f64]),
    span: Span,
    fill: &str,
    opacity: f64,
) -> Result<Option<Chart<MarkPolygon>>, ChartonError> {
    let (mut px, mut py, mut pg) = (Vec::new(), Vec::new(), Vec::new());
    for (k, (x, edges)) in cut_runs(xu, &[lo, hi], span, linear)
        .into_iter()
        .enumerate()
    {
        if x.len() < 2 {
            continue;
        }
        for (a, b) in x.iter().zip(&edges[1]) {
            px.push(*a);
            py.push(*b);
            pg.push(group_name(k));
        }
        for (a, b) in x.iter().zip(&edges[0]).rev() {
            px.push(*a);
            py.push(*b);
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
pub(crate) fn fill_of(style: &Style, color: &str) -> (String, f64) {
    match &style.fill {
        Some(f) => (
            f.color.clone().unwrap_or_else(|| color.to_string()),
            f.opacity * style.opacity,
        ),
        None => (color.to_string(), BAND_OPACITY * style.opacity),
    }
}

/// The error bars of a curve along one axis: a segment per point from `v - low` to
/// `v + high`, and with a `cap` a segment across each end which the axis shows, `cap`
/// coordinates of the other axis long (0 for none).
///
/// `along` is the axis of the error, `across` the coordinates of the other axis; an end the axis
/// cannot show (below 0 on a logarithmic axis) is the lower end of the axis, and has no cap. A
/// side which is not drawn (a gap of errors of two sides, since 0.33.0) leaves its half of the
/// bar out, and its cap: the bar runs from the value to the other end, a one-sided error bar.
/// A symmetric error of 0 has no caps; a side of 0 has its cap at the value, as matplotlib.
/// `keep` says which points have error bars at all.
pub(crate) fn error_runs(
    values: &[f64],
    errors: &Errors,
    across: &[f64],
    along: &ResolvedAxis,
    horizontal: bool,
    cap: f64,
    keep: &dyn Fn(usize) -> bool,
) -> Vec<(Vec<f64>, Vec<f64>)> {
    let start = along.u(along.low);
    let sides = matches!(errors, Errors::Sides { .. });
    let mut out = Vec::new();
    let mut segment = |a: (f64, f64), b: (f64, f64)| {
        if horizontal {
            out.push((vec![a.0, b.0], vec![a.1, b.1]));
        } else {
            out.push((vec![a.1, b.1], vec![a.0, b.0]));
        }
    };
    for i in 0..values.len().min(across.len()) {
        let (v, c) = (values[i], across[i]);
        let (lo, hi) = errors.at(i);
        if !keep(i)
            || (lo.is_nan() && hi.is_nan())
            || !(v.is_finite() && c.is_finite() && along.u(v).is_finite())
        {
            continue;
        }
        let end = |w: f64| along.u(w);
        let (a, b) = (
            if lo.is_nan() { end(v) } else { end(v - lo) },
            if hi.is_nan() { end(v) } else { end(v + hi) },
        );
        segment(
            (if a.is_finite() { a } else { start }, c),
            (if b.is_finite() { b } else { start }, c),
        );
        let capped = |e: f64| !e.is_nan() && (sides || e != 0.0);
        if cap > 0.0 {
            let half = cap / 2.0;
            for (u, e) in [(a, lo), (b, hi)] {
                if u.is_finite() && capped(e) {
                    segment((u, c - half), (u, c + half));
                }
            }
        }
    }
    out
}

/// The coordinates of the axis of `along` one pixel of a panel of `pixels` along it spans.
pub(crate) fn per_pixel(along: &ResolvedAxis, pixels: f64) -> f64 {
    let (lo, hi) = along.domain();
    if pixels > 0.0 {
        (hi - lo) / pixels
    } else {
        0.0
    }
}

/// The x domain of a plot as the interval its series are cut to: nothing of a curve, a band or
/// an area is drawn beyond an end of the x axis.
fn domain_span(xa: &ResolvedAxis) -> Span {
    let (lo, hi) = xa.domain();
    Span { lo, hi }
}

/// The data of a curve: x, y and the errors of x and of y.
type CurveData<'a> = (&'a [f64], &'a [f64], Option<&'a Errors>, Option<&'a Errors>);

/// The error bars of a curve: the length of their caps in pixels and the panel in pixels.
struct ErrorCaps {
    /// The length of a cap in pixels, 0 for none.
    cap: f64,
    /// The width of the lines of the error bars in pixels.
    width: f64,
    /// The width and the height of the panel in pixels.
    panel: (f64, f64),
}

/// The layers of a curve: its line, its error bars and its markers.
fn curve_layers(
    layers: &mut Layers,
    index: usize,
    data: CurveData<'_>,
    caps: ErrorCaps,
    style: &Style,
    color: &str,
    axes: (&ResolvedAxis, &ResolvedAxis),
) -> Result<(), ChartonError> {
    let (x, y, xerr, yerr) = data;
    let (xa, ya) = axes;
    let span = domain_span(xa);
    let xu: Vec<f64> = x.iter().map(|v| xa.u(*v)).collect();
    let yu: Vec<f64> = y.iter().map(|v| ya.u(*v)).collect();
    let info = || LayerInfo {
        element: Some(index),
        marker: None,
        raw: None,
    };
    if let Some(stroke) = Stroke::of_line(style, color)
        && let Some(l) = path_layer(&line_runs(&xu, &yu, span), &stroke, false)?
    {
        layers.push(l, info());
    }
    let bar = Stroke {
        color: color.to_string(),
        width: caps.width,
        dash: Vec::new(),
        opacity: style.opacity,
    };
    // a point beyond an end of the x axis has no error bars and no marker
    let inside = |i: usize| span.holds(xu[i]);
    // a cap across an error of x runs along y and one across an error of y along x
    let (w, h) = caps.panel;
    let (x_cap, y_cap) = (caps.cap * per_pixel(ya, h), caps.cap * per_pixel(xa, w));
    if let Some(e) = xerr
        && let Some(l) = path_layer(&error_runs(x, e, &yu, xa, true, x_cap, &inside), &bar, true)?
    {
        layers.push(l, info());
    }
    if let Some(e) = yerr
        && let Some(l) = path_layer(
            &error_runs(y, e, &xu, ya, false, y_cap, &inside),
            &bar,
            true,
        )?
    {
        layers.push(l, info());
    }
    let m = &style.marker;
    if m.kind != MarkerType::None && m.size > 0.0 {
        let (px, py): (Vec<f64>, Vec<f64>) = xu
            .iter()
            .zip(&yu)
            .filter(|(a, b)| a.is_finite() && b.is_finite() && span.holds(**a))
            .map(|(a, b)| (*a, *b))
            .unzip();
        if !px.is_empty() {
            let ds = dataset(vec![("x", px.into_column()), ("y", py.into_column())])?;
            let radius = m.size / 2.0;
            let chart = Chart::build(&ds)?
                .mark_point()?
                .configure_point(move |p| p.with_size(radius))
                .encode((alt::x("x"), alt::y("y")))?;
            let glyph = MarkerGlyph::of(m, color, style.opacity);
            layers.push(
                chart,
                LayerInfo {
                    element: Some(index),
                    marker: Some(glyph),
                    raw: None,
                },
            );
        }
    }
    Ok(())
}

/// Builds the layers of the elements of a plot.
///
/// `colors` is the colour of every element (its own or the default), `fonts` the font sizes of
/// the figure and `panel` the expected width and height of the panel in pixels, which the text
/// of the cells of a heatmap is fitted into. Only the elements of the y axis `which` are drawn,
/// against `ya`: the primary chart of a plot draws the left axis, its overlay the right one.
pub(crate) fn build(
    elements: &[Element],
    colors: &[String],
    fonts: &Fonts,
    (xa, ya): (&ResolvedAxis, &ResolvedAxis),
    panel: (f64, f64),
    which: crate::spec::YAxisRef,
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
            raw: None,
        }],
    };
    for (index, e) in elements.iter().enumerate() {
        if e.axis != which {
            continue;
        }
        let color = colors[index].as_str();
        let style = &e.style;
        let info = || LayerInfo {
            element: Some(index),
            marker: None,
            raw: None,
        };
        match &e.kind {
            _ if super::annot::raw_marks(e) => {
                let px = Pixels::new((xa, ya), panel);
                let ew = match &e.kind {
                    ElementKind::Curve { error_width, .. } => error_width.unwrap_or(ERROR_WIDTH),
                    _ => ERROR_WIDTH,
                };
                if let Some(raw) = super::annot::marks_layer(e, color, &px, ew) {
                    raw_layer(&mut layers, index, raw, (xa, ya))?;
                }
            }
            ElementKind::Curve {
                x,
                y,
                xerr,
                yerr,
                error_cap,
                error_width,
            } => {
                curve_layers(
                    &mut layers,
                    index,
                    (x, y, xerr.as_ref(), yerr.as_ref()),
                    ErrorCaps {
                        cap: *error_cap,
                        width: error_width.unwrap_or(ERROR_WIDTH),
                        panel,
                    },
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
                let span = domain_span(xa);
                if let Some(l) = polygon_layer(&xu, (&lo, &hi), span, &fill, opacity)? {
                    layers.push(l, info());
                }
                if let Some(m) = median
                    && let Some(stroke) = Stroke::of_line(style, color)
                {
                    let mu: Vec<f64> = m.iter().map(|v| ya.u(*v)).collect();
                    if let Some(l) = path_layer(&line_runs(&xu, &mu, span), &stroke, false)? {
                        layers.push(l, info());
                    }
                }
            }
            ElementKind::Area { x, from, to } => {
                let xu: Vec<f64> = x.iter().map(|v| xa.u(*v)).collect();
                let lo: Vec<f64> = from.iter().map(|v| ya.u(*v)).collect();
                let hi: Vec<f64> = to.iter().map(|v| ya.u(*v)).collect();
                let (fill, opacity) = fill_of(style, color);
                let span = domain_span(xa);
                if let Some(l) = polygon_layer(&xu, (&lo, &hi), span, &fill, opacity)? {
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
            ElementKind::Text { .. } if super::annot::rich_text(e) => {
                let px = Pixels::new((xa, ya), panel);
                if let Some(raw) = super::annot::text_layer(e, color, fonts, &px) {
                    raw_layer(&mut layers, index, raw, (xa, ya))?;
                }
            }
            ElementKind::Contour {
                x,
                y,
                values,
                levels,
                filled,
                scale,
                labels,
                label_size,
            } => {
                let px = Pixels::new((xa, ya), panel);
                let grid = super::contour::Grid { x, y, values };
                let look = super::contour::Look {
                    levels,
                    filled: *filled,
                    scale: scale.as_ref(),
                    labels: labels.as_deref(),
                    label_size: *label_size,
                    style,
                    color,
                };
                let raw = super::contour::contour_svg(&grid, &look, fonts, &px);
                raw_layer(&mut layers, index, raw, (xa, ya))?;
            }
            ElementKind::Arrow { .. } => {
                let px = Pixels::new((xa, ya), panel);
                if let Some(raw) = super::annot::arrow_layer(e, color, fonts, &px) {
                    raw_layer(&mut layers, index, raw, (xa, ya))?;
                }
            }
            ElementKind::Image {
                png,
                extent,
                origin,
                interpolation,
            } => {
                let px = Pixels::new((xa, ya), panel);
                if let Some(svg) = image_svg(png, *extent, *origin, *interpolation, style, &px) {
                    let raw = RawSvg {
                        clipped: svg,
                        unclipped: String::new(),
                    };
                    raw_layer(&mut layers, index, raw, (xa, ya))?;
                }
            }
            ElementKind::Text {
                x,
                y,
                text,
                anchor,
                size,
                weight,
                family,
                ..
            } => {
                let (u, v) = (xa.u(x.unwrap_or(f64::NAN)), ya.u(y.unwrap_or(f64::NAN)));
                if !(u.is_finite() && v.is_finite()) {
                    continue;
                }
                let monospace = *family == crate::spec::FontFamily::Monospace;
                // a dot operator of a monospace text is the middle dot, which every monospace
                // font has, so the columns of its lines stay aligned
                let text = if monospace {
                    text.replace('\u{22c5}', "\u{b7}")
                } else {
                    text.clone()
                };
                let ds = dataset(vec![
                    ("x", vec![u].into_column()),
                    ("y", vec![v].into_column()),
                    ("t", vec![text].into_column()),
                ])?;
                let anchor = match anchor {
                    crate::spec::Anchor::Start => "start",
                    crate::spec::Anchor::Middle => "middle",
                    crate::spec::Anchor::End => "end",
                };
                let (c, opacity) = (color.to_string(), style.opacity);
                let size = size.unwrap_or(fonts.tick);
                let weight = match weight {
                    crate::spec::FontWeight::Normal => "normal",
                    crate::spec::FontWeight::Bold => "bold",
                };
                let chart = Chart::build(&ds)?
                    .mark_text()?
                    .configure_text(move |t| {
                        let t = t
                            .with_color(c.as_str())
                            .with_size(size)
                            .with_weight(weight)
                            .with_anchor(anchor)
                            .with_opacity(opacity);
                        if monospace {
                            t.with_font_family(crate::svg::MONO_FAMILY)
                        } else {
                            t
                        }
                    })
                    .encode((alt::x("x"), alt::y("y"), alt::text("t")))?;
                layers.push(chart, info());
            }
            ElementKind::Bar {
                x,
                height,
                err,
                horizontal,
                width,
                error_cap,
                error_width,
                base,
                edge_color,
                edge_width,
            } => stats_marks::bar_layers(
                &mut layers,
                index,
                stats_marks::Bars {
                    x,
                    height,
                    base: base.as_deref(),
                    err: err.as_ref(),
                    horizontal: *horizontal,
                    width: *width,
                    error_cap: *error_cap,
                    error_width: error_width.unwrap_or(ERROR_WIDTH),
                    panel,
                    edge: (edge_color.as_deref(), *edge_width),
                },
                style,
                color,
                (xa, ya),
            )?,
            ElementKind::Box {
                positions,
                stats,
                width,
                median_color,
                whisker_opacity,
                median_opacity,
            } => stats_marks::box_layers(
                &mut layers,
                index,
                stats_marks::Boxes {
                    positions,
                    stats,
                    width: *width,
                    median_color: median_color.as_deref(),
                    whisker_opacity: *whisker_opacity,
                    median_opacity: *median_opacity,
                },
                style,
                color,
                (xa, ya),
            )?,
            ElementKind::Violin {
                positions,
                densities,
            } => stats_marks::violin_layers(
                &mut layers,
                index,
                (positions, densities),
                style,
                color,
                (xa, ya),
            )?,
            ElementKind::Histogram {
                edges,
                counts,
                base,
                edge_color,
                edge_width,
            } => stats_marks::histogram_layers(
                &mut layers,
                index,
                (edges, counts, base.as_deref()),
                (edge_color.as_deref(), *edge_width),
                style,
                color,
                (xa, ya),
            )?,
            ElementKind::Polygon { paths } => {
                stats_marks::polygon_layers(&mut layers, index, paths, style, color, (xa, ya))?;
            }
            ElementKind::Heatmap {
                rows,
                columns,
                values,
                text,
                scale,
                x_edges,
                y_edges,
            } => heatmap::heatmap_layers(
                &mut layers,
                index,
                heatmap::Cells::of(
                    (rows, columns),
                    values,
                    text.as_deref(),
                    scale,
                    (x_edges.as_deref(), y_edges.as_deref()),
                ),
                style,
                fonts,
                (xa, ya),
                panel,
            )?,
        }
    }
    Ok(layers)
}

/// The `<image>` of an image element in the pixels of its panel (since 0.34.0), `None` for an
/// extent the axes cannot show (an edge at or below 0 on a log axis).
///
/// The image is stretched over the rectangle of its corners (`preserveAspectRatio="none"`, as
/// matplotlib fills the `extent`), its first row at the top or the bottom edge by its origin;
/// an extent or an axis which runs the other way mirrors it, by a transform about its centre.
fn image_svg(
    png: &str,
    [x0, x1, y0, y1]: [f64; 4],
    origin: crate::spec::ImageOrigin,
    interpolation: crate::spec::Interpolation,
    style: &Style,
    px: &Pixels,
) -> Option<String> {
    use crate::svg::{attr_num, attr_opacity, num};
    // the pixels of the left and the right edge, and of the edges of the first and the last row
    let (first, last) = match origin {
        crate::spec::ImageOrigin::Upper => (y1, y0),
        crate::spec::ImageOrigin::Lower => (y0, y1),
    };
    let (left, top) = px.at(Coordinates::Data, x0, first);
    let (right, bottom) = px.at(Coordinates::Data, x1, last);
    if ![left, top, right, bottom].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x, w) = (left.min(right), (right - left).abs());
    let (y, h) = (top.min(bottom), (bottom - top).abs());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut out = String::with_capacity(png.len() + 256);
    out.push_str("<image class=\"sbmlsim-image-raster\"");
    attr_num(&mut out, "x", x);
    attr_num(&mut out, "y", y);
    attr_num(&mut out, "width", w);
    attr_num(&mut out, "height", h);
    out.push_str(" preserveAspectRatio=\"none\"");
    // matplotlib's `auto`: the pixels of an image drawn three times its size or more as
    // squares, else smoothed
    let nearest = match interpolation {
        crate::spec::Interpolation::Nearest => true,
        crate::spec::Interpolation::Smooth => false,
        crate::spec::Interpolation::Auto => crate::image::png_size(png)
            .is_ok_and(|(pw, ph)| w >= 3.0 * f64::from(pw) && h >= 3.0 * f64::from(ph)),
    };
    // `pixelated` is a value of CSS only (resvg ignores it in the attribute); the attribute of
    // SVG 1.1 for a page which refuses inline styles
    if nearest {
        out.push_str(" image-rendering=\"optimizeSpeed\" style=\"image-rendering:pixelated\"");
    }
    attr_opacity(&mut out, "opacity", style.opacity);
    // a mirror about the centre of the image along each side which runs the other way
    let (fx, fy) = (right < left, bottom < top);
    if fx || fy {
        let (sx, sy) = (if fx { -1.0 } else { 1.0 }, if fy { -1.0 } else { 1.0 });
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        out.push_str(" transform=\"matrix(");
        for (k, v) in [sx, 0.0, 0.0, sy, cx - sx * cx, cy - sy * cy]
            .iter()
            .enumerate()
        {
            if k > 0 {
                out.push(' ');
            }
            num(&mut out, *v);
        }
        out.push_str(")\"");
    }
    out.push_str(" href=\"data:image/png;base64,");
    // the base64 of the spec, checked to be of its alphabet, which needs no escaping
    out.push_str(png);
    out.push_str("\"/>");
    Some(out)
}

/// The values the elements of a plot put on its axes.
#[derive(Debug, Default)]
pub(crate) struct Extent {
    /// Values on the x axis.
    pub xs: Vec<f64>,
    /// Values on the y axis.
    pub ys: Vec<f64>,
    /// Values the x axis sticks to: the base of horizontal bars.
    pub x_sticky: Vec<f64>,
    /// Values the y axis sticks to: the base of vertical bars and of a histogram.
    pub y_sticky: Vec<f64>,
}

/// Adds the values an element puts on the x axis and on its y axis.
///
/// `ends` are the ends the spec gives the x axis with whether the y axis of the element is
/// logarithmic: a curve, a band or an area then puts only its values inside the ends on the
/// axes, and the values where it crosses an end, since nothing beyond them is drawn; a text, a
/// vertical bar and a vertex of a polygon only inside them.
pub(crate) fn extent(e: &Element, out: &mut Extent, ends: Option<(XEnds, bool)>) {
    // an element in fractions of the panel (since 0.33.0) and a text at a location put nothing
    // on the axes
    if e.coordinates == Coordinates::Axes
        || matches!(e.kind, ElementKind::Text { loc: Some(_), .. })
    {
        return;
    }
    if let Some((ends, log_y)) = ends
        && cut_extent(e, out, ends, log_y)
    {
        return;
    }
    let with_errors = |out: &mut Vec<f64>, v: &[f64], err: Option<&Errors>| {
        out.extend_from_slice(v);
        if let Some(err) = err {
            push_errors(out, v, err, |_| true);
        }
    };
    let (xs, ys) = (&mut out.xs, &mut out.ys);
    match &e.kind {
        ElementKind::Curve {
            x, y, xerr, yerr, ..
        } => {
            with_errors(xs, x, xerr.as_ref());
            with_errors(ys, y, yerr.as_ref());
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
            if let (Some(x), Some(y)) = (x, y) {
                xs.push(*x);
                ys.push(*y);
            }
        }
        ElementKind::Arrow {
            x,
            y,
            x_from,
            y_from,
            ..
        } => {
            xs.extend([*x, *x_from]);
            ys.extend([*y, *y_from]);
        }
        // the axes stick to the ends of the grid of contours, as matplotlib's tight limits of
        // a `ContourSet`
        // the axes stick to the edges of an image, as matplotlib's limits of an image (since
        // 0.34.0)
        ElementKind::Image { extent, .. } => {
            out.xs.extend_from_slice(&extent[..2]);
            out.ys.extend_from_slice(&extent[2..]);
            out.x_sticky.extend_from_slice(&extent[..2]);
            out.y_sticky.extend_from_slice(&extent[2..]);
        }
        ElementKind::Contour { x, y, .. } => {
            for (values, out_v, sticky) in [
                (x, &mut out.xs, &mut out.x_sticky),
                (y, &mut out.ys, &mut out.y_sticky),
            ] {
                if let (Some(a), Some(b)) = (values.first(), values.last()) {
                    out_v.extend([*a, *b]);
                    sticky.extend([*a, *b]);
                }
            }
        }
        ElementKind::Rule { orientation, value } => match orientation {
            Orientation::Horizontal => ys.push(*value),
            Orientation::Vertical => xs.push(*value),
        },
        ElementKind::Bar {
            x,
            height,
            err,
            horizontal,
            width,
            base,
            ..
        } => {
            let w = width.unwrap_or(stats_marks::BAR_WIDTH * stats_marks::spacing(x));
            let (positions, values, sticky) = if *horizontal {
                (&mut out.ys, &mut out.xs, &mut out.x_sticky)
            } else {
                (&mut out.xs, &mut out.ys, &mut out.y_sticky)
            };
            for p in x {
                positions.push(p - w / 2.0);
                positions.push(p + w / 2.0);
            }
            // a bar from its base (0 by default) to its top, the axis sticking to its base
            let tops = stats_marks::tops(height, base.as_deref());
            with_errors(values, &tops, err.as_ref());
            let bases = stats_marks::bases(base.as_deref(), height.len());
            values.extend_from_slice(&bases);
            sticky.extend_from_slice(&bases);
        }
        ElementKind::Box {
            positions,
            stats,
            width,
            ..
        } => {
            let half = stats_marks::box_width(positions, *width) / 2.0;
            for p in positions {
                xs.push(p - half);
                xs.push(p + half);
            }
            for b in stats {
                ys.extend([b.low_whisker, b.q1, b.median, b.q3, b.high_whisker]);
                ys.extend_from_slice(&b.outliers);
            }
        }
        ElementKind::Violin {
            positions,
            densities,
        } => {
            let half = stats_marks::VIOLIN_WIDTH * stats_marks::spacing(positions) / 2.0;
            for p in positions {
                xs.push(p - half);
                xs.push(p + half);
            }
            for d in densities {
                ys.extend_from_slice(&d.values);
            }
        }
        ElementKind::Histogram {
            edges,
            counts,
            base,
            ..
        } => {
            xs.extend_from_slice(edges);
            ys.extend(stats_marks::tops(counts, base.as_deref()));
            let bases = stats_marks::bases(base.as_deref(), counts.len());
            ys.extend_from_slice(&bases);
            out.y_sticky.extend_from_slice(&bases);
        }
        ElementKind::Heatmap {
            rows,
            columns,
            x_edges,
            y_edges,
            ..
        } => {
            // the cells along an axis with edges span them, and the axis sticks to them as
            // matplotlib's `pcolormesh` does; along a category axis they span its categories
            match x_edges {
                Some(e) => {
                    xs.extend_from_slice(e);
                    out.x_sticky.extend_from_slice(e);
                }
                None => xs.extend([-0.5, columns.len() as f64 - 0.5]),
            }
            match y_edges {
                Some(e) => {
                    ys.extend_from_slice(e);
                    out.y_sticky.extend_from_slice(e);
                }
                None => ys.extend([-0.5, rows.len() as f64 - 0.5]),
            }
        }
        ElementKind::Polygon { paths } => {
            for [x, y] in paths.iter().flatten() {
                xs.push(*x);
                ys.push(*y);
            }
        }
    }
}

/// Adds the ends of the error bars of the values `i` with `keep(i)` to the values of an axis:
/// the value less the error below it and the value plus the error above it, a side which is
/// not drawn not.
fn push_errors(out: &mut Vec<f64>, values: &[f64], err: &Errors, keep: impl Fn(usize) -> bool) {
    for (i, v) in values.iter().enumerate() {
        if !keep(i) {
            continue;
        }
        let (lo, hi) = err.at(i);
        if !lo.is_nan() {
            out.push(v - lo);
        }
        if !hi.is_nan() {
            out.push(v + hi);
        }
    }
}

/// Adds the values of an element inside the given ends of the x axis; false for an element
/// whose extent is taken as without the ends.
fn cut_extent(e: &Element, out: &mut Extent, ends: XEnds, log_y: bool) -> bool {
    let inside = |x: &[f64], v: &[f64]| -> Vec<f64> {
        x.iter()
            .zip(v)
            .filter(|(x, _)| ends.holds(**x))
            .map(|(_, v)| *v)
            .collect()
    };
    match &e.kind {
        ElementKind::Curve {
            x, y, xerr, yerr, ..
        } => {
            out.xs.extend(inside(x, x));
            let holds = |i: usize| x.get(i).is_some_and(|v| ends.holds(*v));
            if let Some(err) = xerr {
                push_errors(&mut out.xs, x, err, holds);
            }
            out.ys.extend(ends.values(x, &[y], log_y));
            if let Some(err) = yerr {
                push_errors(&mut out.ys, y, err, holds);
            }
        }
        ElementKind::Band {
            x,
            low,
            high,
            median,
        } => {
            out.xs.extend(inside(x, x));
            out.ys.extend(ends.values(x, &[low, high], log_y));
            if let Some(m) = median {
                out.ys.extend(ends.values(x, &[m], log_y));
            }
        }
        ElementKind::Area { x, from, to } => {
            out.xs.extend(inside(x, x));
            out.ys.extend(ends.values(x, &[from, to], log_y));
        }
        // a text, a vertical bar and a vertex of a polygon beyond an end are cut by the panel,
        // so they put nothing on the axes; the edges of a polygon which cross an end are cut
        // there by the panel and not interpolated
        ElementKind::Text {
            x: Some(x),
            y: Some(y),
            ..
        } => {
            if ends.holds(*x) {
                out.xs.push(*x);
                out.ys.push(*y);
            }
        }
        ElementKind::Arrow {
            x,
            y,
            x_from,
            y_from,
            ..
        } => {
            for (a, b) in [(x, y), (x_from, y_from)] {
                if ends.holds(*a) {
                    out.xs.push(*a);
                    out.ys.push(*b);
                }
            }
        }
        ElementKind::Bar {
            x,
            height,
            err,
            horizontal: false,
            width,
            base,
            ..
        } => {
            let w = width.unwrap_or(stats_marks::BAR_WIDTH * stats_marks::spacing(x));
            let tops = stats_marks::tops(height, base.as_deref());
            let bases = stats_marks::bases(base.as_deref(), height.len());
            for (k, p) in x.iter().enumerate() {
                if !ends.holds(*p) {
                    continue;
                }
                out.xs.extend([p - w / 2.0, p + w / 2.0]);
                out.ys.extend([tops[k], bases[k]]);
                out.y_sticky.push(bases[k]);
                if let Some(e) = err {
                    push_errors(&mut out.ys, &tops, e, |i| i == k);
                }
            }
            if base.is_none() {
                out.ys.push(0.0);
                out.y_sticky.push(0.0);
            }
        }
        ElementKind::Polygon { paths } => {
            for [x, y] in paths.iter().flatten() {
                if ends.holds(*x) {
                    out.xs.push(*x);
                    out.ys.push(*y);
                }
            }
        }
        // a rule spans its axis (a horizontal one has no x, a vertical one no y); a horizontal
        // bar, a box, a violin, a histogram and a heatmap take their extent as without the
        // ends, the panel cuts what is beyond them
        _ => return false,
    }
    true
}

/// Whether an element without a colour of its own takes the next colour of the cycle; a rule,
/// a text and an arrow are drawn in the text colour instead, and an image has its own colours
/// (since 0.34.0).
pub(crate) fn cycled(e: &Element) -> bool {
    !matches!(
        e.kind,
        ElementKind::Rule { .. }
            | ElementKind::Text { .. }
            | ElementKind::Arrow { .. }
            | ElementKind::Image { .. }
    )
}

/// The default colour of an element which is not cycled.
pub(crate) const UNCYCLED: &str = theme::FG;

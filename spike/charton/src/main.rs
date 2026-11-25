//! Spike: can Charton 0.9.1 draw every element kind of `figure/1`?
//!
//! One function per element kind builds a figure and returns its SVG. `main`
//! writes every SVG into `out/`, times each one with 200 repetitions and runs
//! the probes of the questions of FINDINGS.md (geometry through a custom
//! backend, the right y axis, the data-element attributes).

use charton::coordinate::Rect;
use charton::core::layer::{
    CircleConfig, GradientRectConfig, LineConfig, PathConfig, RectConfig, RenderBackend, TextConfig,
};
use charton::core::semantic::{Element, Group, Role};
use charton::error::ChartonError;
use charton::prelude::*;
use std::fmt::Write as _;
use std::time::Instant;

type R<T> = Result<T, Box<dyn std::error::Error>>;

const W: u32 = 500;
const H: u32 = 400;

fn ds(cols: Vec<(&str, ColumnVector)>) -> R<Dataset> {
    Ok(cols.to_dataset()?)
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
        .collect()
}

fn ylin(field: &str, log: bool) -> charton::encode::y::Y {
    if log {
        alt::y(field).with_scale(Scale::Log)
    } else {
        alt::y(field)
    }
}

// ---------------------------------------------------------------- curve
/// A curve with x and y error bars and a NaN gap. The y error bars are
/// Charton's error bar mark, the x error bars a path mark with one path group
/// per point (Charton's error bar is vertical only, `coord_flip` is per chart).
fn curve() -> R<LayeredChart> {
    let t = linspace(0.0, 24.0, 200);
    let y: Vec<f64> = t
        .iter()
        .map(|&x| {
            if (10.0..12.0).contains(&x) {
                f64::NAN
            } else {
                4.0 * (-x / 6.0).exp() * x
            }
        })
        .collect();
    let line = Chart::build(&ds(vec![
        ("time", t.into_column()),
        ("conc", y.into_column()),
    ])?)?
    .mark_line()?
    .configure_line(|l| {
        l.with_color("#1f77b4")
            .with_stroke_width(2.0)
            .with_dash(vec![6.0, 3.0])
    })
    .encode((alt::x("time"), alt::y("conc")))?;

    let td: Vec<f64> = (0..12).map(|i| 1.0 + 2.0 * i as f64).collect();
    let yd: Vec<f64> = td
        .iter()
        .map(|&x| 4.0 * (-x / 6.0).exp() * x * 1.05)
        .collect();
    let ye: Vec<f64> = yd.iter().map(|y| 0.12 * y + 0.1).collect();
    let xe: Vec<f64> = td.iter().map(|_| 0.6).collect();
    let lo: Vec<f64> = yd.iter().zip(&ye).map(|(y, e)| y - e).collect();
    let hi: Vec<f64> = yd.iter().zip(&ye).map(|(y, e)| y + e).collect();
    let pts = Chart::build(&ds(vec![
        ("time", td.clone().into_column()),
        ("conc", yd.clone().into_column()),
    ])?)?
    .mark_point()?
    .configure_point(|p| {
        p.with_color("#ff7f0e")
            .with_shape(PointShape::Square)
            .with_size(4.0)
            .with_stroke("black")
            .with_stroke_width(1.0)
    })
    .encode((alt::x("time"), alt::y("conc")))?;
    let yerr = Chart::build(&ds(vec![
        ("time", td.clone().into_column()),
        ("lo", lo.into_column()),
        ("hi", hi.into_column()),
    ])?)?
    .mark_errorbar()?
    .configure_errorbar(|e| {
        e.with_color("#ff7f0e")
            .with_stroke_width(1.0)
            .with_cap_length(3.0)
    })
    .encode((alt::x("time"), alt::y("lo"), alt::y2("hi")))?;
    // x error bars: two rows per point, grouped by point
    let mut px = Vec::new();
    let mut py = Vec::new();
    let mut pg = Vec::new();
    for (i, ((x, y), e)) in td.iter().zip(&yd).zip(&xe).enumerate() {
        px.extend([x - e, x + e]);
        py.extend([*y, *y]);
        pg.extend([format!("p{i}"), format!("p{i}")]);
    }
    let xerr = Chart::build(&ds(vec![
        ("x", px.into_column()),
        ("y", py.into_column()),
        ("g", pg.into_column()),
    ])?)?
    .mark_path()?
    .configure_path(|p| p.with_stroke("#ff7f0e").with_stroke_width(1.0))
    .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;

    Ok(line
        .and(xerr)
        .and(yerr)
        .and(pts)
        .with_size(W, H)
        .with_title("curve: x/y errors, NaN gap 10..12")
        .with_x_label("time [hr]")
        .with_y_label("concentration [mg/l]"))
}

// ---------------------------------------------------------------- band
/// The closed outline of the area between `lo` and `hi`, one path group per
/// run of finite values: Charton's area mark has no y2 (its baseline is 0 or
/// a stack), so a band and a shaded area are polygons the crate builds, and a
/// NaN splits them into runs.
fn band_polygon(x: &[f64], lo: &[f64], hi: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<String>) {
    let (mut px, mut py, mut pg) = (Vec::new(), Vec::new(), Vec::new());
    let mut run = 0;
    let mut i = 0;
    while i < x.len() {
        if !(lo[i].is_finite() && hi[i].is_finite()) {
            i += 1;
            continue;
        }
        let start = i;
        while i < x.len() && lo[i].is_finite() && hi[i].is_finite() {
            i += 1;
        }
        let g = format!("r{run}");
        run += 1;
        for k in start..i {
            px.push(x[k]);
            py.push(hi[k]);
            pg.push(g.clone());
        }
        for k in (start..i).rev() {
            px.push(x[k]);
            py.push(lo[k]);
            pg.push(g.clone());
        }
    }
    (px, py, pg)
}

/// A band (low..high) with a median line.
fn band() -> R<LayeredChart> {
    let t = linspace(0.0, 24.0, 300);
    let med: Vec<f64> = t.iter().map(|&x| 4.0 * (-x / 6.0).exp() * x).collect();
    let lo: Vec<f64> = med.iter().map(|m| m * 0.7).collect();
    let hi: Vec<f64> = med.iter().map(|m| m * 1.4).collect();
    let (px, py, pg) = band_polygon(&t, &lo, &hi);
    let area = Chart::build(&ds(vec![
        ("x", px.into_column()),
        ("y", py.into_column()),
        ("g", pg.into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|a| {
        a.with_fill("#2ca02c")
            .with_opacity(0.25)
            .with_stroke("none")
            .with_stroke_width(0.0)
    })
    .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    let line = Chart::build(&ds(vec![
        ("time", t.into_column()),
        ("med", med.into_column()),
    ])?)?
    .mark_line()?
    .configure_line(|l| l.with_color("#2ca02c").with_stroke_width(1.5))
    .encode((alt::x("time"), alt::y("med")))?;
    Ok(area
        .and(line)
        .with_size(W, H)
        .with_title("band (polygon) with median")
        .with_x_label("time [hr]")
        .with_y_label("conc [mg/l]"))
}

// ---------------------------------------------------------------- area
/// A shaded area (from, to) with a NaN gap, behind a line.
fn area() -> R<LayeredChart> {
    let t = linspace(0.0, 10.0, 101);
    let from: Vec<f64> = t.iter().map(|&x| 0.1 * x).collect();
    let to: Vec<f64> = t
        .iter()
        .map(|&x| {
            if (4.0..5.0).contains(&x) {
                f64::NAN
            } else {
                x.sin().abs() + 0.2 + 0.1 * x
            }
        })
        .collect();
    let (px, py, pg) = band_polygon(&t, &from, &to);
    let a = Chart::build(&ds(vec![
        ("x", px.into_column()),
        ("y", py.into_column()),
        ("g", pg.into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|a| {
        a.with_fill("#9467bd")
            .with_opacity(0.3)
            .with_stroke("none")
            .with_stroke_width(0.0)
    })
    .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    let l = Chart::build(&ds(vec![("t", t.into_column()), ("to", to.into_column())])?)?
        .mark_line()?
        .configure_line(|l| l.with_color("#9467bd"))
        .encode((alt::x("t"), alt::y("to")))?;
    Ok(a.and(l)
        .with_size(W, H)
        .with_title("area from/to (polygon), NaN gap 4..5")
        .with_x_label("t")
        .with_y_label("y"))
}

// ---------------------------------------------------------------- bar
/// A categorical bar with error bars.
fn bar() -> R<LayeredChart> {
    let cat = vec!["oral", "iv", "renal", "hepatic"];
    let h = vec![3.2, 5.1, 1.4, 2.8];
    let lo: Vec<f64> = h.iter().map(|v| v - 0.4).collect();
    let hi: Vec<f64> = h.iter().map(|v| v + 0.5).collect();
    let b = Chart::build(&ds(vec![
        ("cat", cat.clone().into_column()),
        ("h", h.into_column()),
    ])?)?
    .mark_bar()?
    .configure_bar(|b| {
        b.with_color("#8c564b")
            .with_stroke("black")
            .with_stroke_width(0.8)
    })
    .encode((alt::x("cat"), alt::y("h")))?;
    let e = Chart::build(&ds(vec![
        ("cat", cat.into_column()),
        ("lo", lo.into_column()),
        ("hi", hi.into_column()),
    ])?)?
    .mark_errorbar()?
    .configure_errorbar(|e| {
        e.with_color("black")
            .with_stroke_width(1.2)
            .with_cap_length(6.0)
    })
    .encode((alt::x("cat"), alt::y("lo"), alt::y2("hi")))?;
    Ok(b.and(e)
        .with_size(W, H)
        .with_title("bar with errors")
        .with_x_label("route")
        .with_y_label("AUC [mg*hr/l]"))
}

// ---------------------------------------------------------------- box
/// A box plot from given statistics: floating bars (q1..q3), whisker rules,
/// median ticks and outlier points. Charton's own boxplot computes its
/// statistics from raw data, so it cannot take the five numbers.
fn boxes() -> R<LayeredChart> {
    let cat = vec!["A", "B", "C"];
    let (lo, q1, med, q3, hi) = (
        vec![1.0, 2.0, 0.5],
        vec![2.0, 3.1, 1.5],
        vec![2.6, 3.9, 2.2],
        vec![3.3, 4.6, 3.0],
        vec![4.5, 6.0, 4.1],
    );
    let boxes = Chart::build(&ds(vec![
        ("cat", cat.clone().into_column()),
        ("q1", q1.clone().into_column()),
        ("q3", q3.clone().into_column()),
    ])?)?
    .mark_bar()?
    .configure_bar(|b| {
        b.with_color("#aec7e8")
            .with_stroke("black")
            .with_stroke_width(1.0)
            .with_width(0.5)
    })
    .encode((alt::x("cat"), alt::y("q1"), alt::y2("q3")))?;
    // whiskers as rules: lo..q1 and q3..hi
    let mut wc = Vec::new();
    let mut w1 = Vec::new();
    let mut w2 = Vec::new();
    for i in 0..3 {
        wc.extend([cat[i], cat[i]]);
        w1.extend([lo[i], q3[i]]);
        w2.extend([q1[i], hi[i]]);
    }
    let whisk = Chart::build(&ds(vec![
        ("cat", wc.into_column()),
        ("a", w1.into_column()),
        ("b", w2.into_column()),
    ])?)?
    .mark_rule()?
    .configure_rule(|r| r.with_color("black").with_stroke_width(1.0))
    .encode((alt::x("cat"), alt::y("a"), alt::y2("b")))?;
    // median: a path per box at the numeric category positions (the tick
    // mark draws a dot here), on the same axis as the bars
    let mut mx = Vec::new();
    let mut my = Vec::new();
    let mut mg = Vec::new();
    let mut ml = Vec::new();
    for i in 0..3 {
        mx.extend([i as f64 - 0.25, i as f64 + 0.25]);
        my.extend([med[i], med[i]]);
        mg.extend([cat[i], cat[i]]);
        ml.extend([cat[i], cat[i]]);
    }
    let median = Chart::build(&ds(vec![
        ("x", mx.into_column()),
        ("med", my.into_column()),
        ("g", mg.into_column()),
        ("cat", ml.into_column()),
    ])?)?
    .mark_path()?
    .configure_path(|p| p.with_stroke("#d62728").with_stroke_width(2.0))
    .encode((
        alt::x("x").with_category_labels("cat"),
        alt::y("med"),
        alt::path_group("g"),
    ))?;
    let out = Chart::build(&ds(vec![
        ("cat", vec!["A", "B", "C", "C"].into_column()),
        ("v", vec![5.2, 0.9, 5.0, 0.1].into_column()),
    ])?)?
    .mark_point()?
    .configure_point(|p| {
        p.with_color("none")
            .with_stroke("black")
            .with_stroke_width(1.0)
            .with_size(3.0)
    })
    .encode((alt::x("cat"), alt::y("v")))?;
    Ok(boxes
        .and(whisk)
        .and(median)
        .and(out)
        .with_size(W, H)
        .with_title("box from given stats")
        .with_x_label("group")
        .with_y_label("value"))
}

// ---------------------------------------------------------------- violin
/// A violin from a given density outline: a polygon per category, the x
/// position `index +- density * half_width`, labelled with the categories.
fn violin() -> R<LayeredChart> {
    let cats = ["A", "B", "C"];
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut g = Vec::new();
    let mut lab = Vec::new();
    for (i, c) in cats.iter().enumerate() {
        let ys = linspace(-3.0, 3.0, 60);
        let mu = i as f64 * 0.5;
        let dens: Vec<f64> = ys
            .iter()
            .map(|v| (-(v - mu).powi(2) / (2.0 * (1.0 + 0.3 * i as f64))).exp())
            .collect();
        let max = dens.iter().cloned().fold(0.0, f64::max);
        // right side up, left side down: one closed outline
        for (v, d) in ys.iter().zip(&dens) {
            x.push(i as f64 + 0.4 * d / max);
            y.push(*v);
        }
        for (v, d) in ys.iter().zip(&dens).rev() {
            x.push(i as f64 - 0.4 * d / max);
            y.push(*v);
        }
        for _ in 0..(2 * ys.len()) {
            g.push(c.to_string());
            lab.push(c.to_string());
        }
    }
    let v = Chart::build(&ds(vec![
        ("x", x.into_column()),
        ("y", y.into_column()),
        ("g", g.into_column()),
        ("cat", lab.into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|p| {
        p.with_fill("#17becf")
            .with_opacity(0.6)
            .with_stroke("black")
            .with_stroke_width(1.0)
    })
    .encode((
        alt::x("x").with_category_labels("cat"),
        alt::y("y"),
        alt::path_group("g"),
    ))?;
    Ok(v.with_size(W, H)
        .with_title("violin from given density")
        .with_x_label("group")
        .with_y_label("value"))
}

// ---------------------------------------------------------------- histogram
/// A histogram from given bin edges and counts: one closed polygon per bin
/// (Charton's histogram bins raw data; a bar has no x2 for a continuous bin).
fn histogram() -> R<LayeredChart> {
    let edges = linspace(0.0, 10.0, 21);
    let counts: Vec<f64> = (0..20)
        .map(|i| 30.0 * (-((i as f64 - 8.0) / 4.0).powi(2)).exp())
        .collect();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut g = Vec::new();
    for i in 0..20 {
        let (a, b, c) = (edges[i], edges[i + 1], counts[i]);
        x.extend([a, b, b, a]);
        y.extend([0.0, 0.0, c, c]);
        for _ in 0..4 {
            g.push(format!("b{i}"));
        }
    }
    let h = Chart::build(&ds(vec![
        ("x", x.into_column()),
        ("y", y.into_column()),
        ("g", g.into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|p| {
        p.with_fill("#7f7f7f")
            .with_opacity(0.8)
            .with_stroke("white")
            .with_stroke_width(0.8)
    })
    .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    Ok(h.with_size(W, H)
        .with_title("histogram from given bins")
        .with_x_label("value")
        .with_y_label("count"))
}

// ---------------------------------------------------------------- heatmap
/// A heatmap with cell text and a diverging scale centred at 0. Charton has
/// no diverging colour map and no custom map, so the crate resolves the cell
/// colours (RdBu around 0) and draws the cells as polygons with an explicit
/// categorical colour per cell (a custom palette indexed in category order).
fn heatmap() -> R<LayeredChart> {
    let rows = ["CL", "Vd", "ka", "fu"];
    let cols = ["AUC", "Cmax", "tmax", "thalf", "MRT"];
    let vals: Vec<f64> = (0..20)
        .map(|k| ((k as f64) * 0.7).sin() * 0.9 - 0.05 * (k as f64 - 10.0) / 10.0)
        .collect();
    let vmax = vals.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut g = Vec::new();
    let mut tx = Vec::new();
    let mut ty = Vec::new();
    let mut tt = Vec::new();
    let mut palette = Vec::new();
    for (r, _) in rows.iter().enumerate() {
        for (c, _) in cols.iter().enumerate() {
            let k = r * cols.len() + c;
            let v = vals[k];
            let (cx, cy) = (c as f64, r as f64);
            x.extend([cx - 0.5, cx + 0.5, cx + 0.5, cx - 0.5]);
            y.extend([cy - 0.5, cy - 0.5, cy + 0.5, cy + 0.5]);
            let id = format!("c{k:03}");
            for _ in 0..4 {
                g.push(id.clone());
            }
            palette.push(rdbu(v / vmax));
            tx.push(cx);
            ty.push(cy);
            tt.push(format!("{v:.2}"));
        }
    }
    let xl: Vec<String> = (0..x.len())
        .map(|i| cols[((x[i] + 0.5).floor() as usize).min(4)].to_string())
        .collect();
    let cells = Chart::build(&ds(vec![
        ("x", x.into_column()),
        ("y", y.into_column()),
        ("g", g.clone().into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|p| p.with_stroke("white").with_stroke_width(1.0))
    .encode((
        alt::x("x"),
        alt::y("y"),
        alt::path_group("g"),
        alt::color("g"),
    ))?;
    let _ = xl;
    let text = Chart::build(&ds(vec![
        ("x", tx.into_column()),
        ("y", ty.into_column()),
        ("t", tt.into_column()),
    ])?)?
    .mark_text()?
    .configure_text(|t| t.with_color("black").with_size(11.0).with_anchor("middle"))
    .encode((alt::x("x"), alt::y("y"), alt::text("t")))?;
    let palette: Vec<&str> = palette.iter().map(|s| s.as_str()).collect();
    Ok(cells
        .and(text)
        .with_size(W, H)
        .with_title("heatmap, diverging RdBu around 0, cell text")
        .with_x_ticks(
            cols.iter()
                .enumerate()
                .map(|(i, c)| (i as f64, c.to_string()))
                .collect::<Vec<_>>(),
        )
        .with_y_ticks(
            rows.iter()
                .enumerate()
                .map(|(i, c)| (i as f64, c.to_string()))
                .collect::<Vec<_>>(),
        )
        .configure_theme(|t| {
            t.with_palette(palette)
                .with_show_legend(false)
                .with_grid(false)
        })
        .with_x_label("parameter")
        .with_y_label("output"))
}

/// RdBu (the 11 stops of ColorBrewer), `t` in [-1, 1], 0 white.
fn rdbu(t: f64) -> String {
    const S: [(u8, u8, u8); 11] = [
        (103, 0, 31),
        (178, 24, 43),
        (214, 96, 77),
        (244, 165, 130),
        (253, 219, 199),
        (247, 247, 247),
        (209, 229, 240),
        (146, 197, 222),
        (67, 147, 195),
        (33, 102, 172),
        (5, 48, 97),
    ];
    // high values red: reverse so that +1 -> index 0
    let u = ((1.0 - t.clamp(-1.0, 1.0)) / 2.0) * 10.0;
    let i = (u.floor() as usize).min(9);
    let f = u - i as f64;
    let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (S[i], S[i + 1]);
    format!("#{:02x}{:02x}{:02x}", m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

// ---------------------------------------------------------------- text, rule, polygon
/// Text annotations at data positions over points.
fn text() -> R<LayeredChart> {
    let x = vec![1.0, 2.0, 3.0, 4.0];
    let y = vec![2.0, 3.5, 1.0, 4.0];
    let t = vec!["Cmax", "tmax = 2 hr", "trough", "steady state"];
    let p = Chart::build(&ds(vec![
        ("x", x.clone().into_column()),
        ("y", y.clone().into_column()),
    ])?)?
    .mark_point()?
    .encode((alt::x("x"), alt::y("y")))?;
    let tx = Chart::build(&ds(vec![
        ("x", x.into_column()),
        ("y", y.into_column()),
        ("t", t.into_column()),
    ])?)?
    .mark_text()?
    .configure_text(|c| c.with_color("#d62728").with_size(12.0).with_anchor("start"))
    .encode((alt::x("x"), alt::y("y"), alt::text("t")))?;
    Ok(p.and(tx)
        .with_size(W, H)
        .with_title("text annotations")
        .with_x_label("x")
        .with_y_label("y"))
}

/// Rules: an identity line and horizontal bias and limit lines (Bland-Altman
/// style). A rule is vertical only (x, y..y2); horizontal and diagonal rules
/// are two-point lines, which carry a dash where a rule cannot.
fn rule() -> R<LayeredChart> {
    let x = vec![0.5, 1.2, 2.0, 3.1, 4.0, 5.2];
    let y = vec![0.6, 1.0, 2.3, 2.9, 4.4, 5.0];
    let pts = Chart::build(&ds(vec![("x", x.into_column()), ("y", y.into_column())])?)?
        .mark_point()?
        .encode((alt::x("x"), alt::y("y")))?;
    let ident = Chart::build(&ds(vec![
        ("x", vec![0.0, 6.0].into_column()),
        ("y", vec![0.0, 6.0].into_column()),
    ])?)?
    .mark_line()?
    .configure_line(|l| l.with_color("black").with_stroke_width(1.0))
    .encode((alt::x("x"), alt::y("y")))?;
    let mut lines = Vec::new();
    for (level, dash) in [(0.8, vec![4.0, 2.0]), (1.0, vec![]), (1.25, vec![4.0, 2.0])] {
        let d = dash.clone();
        lines.push(
            Chart::build(&ds(vec![
                ("x", vec![0.0, 6.0].into_column()),
                ("y", vec![level * 4.0, level * 4.0].into_column()),
            ])?)?
            .mark_line()?
            .configure_line(move |l| l.with_color("#d62728").with_dash(d))
            .encode((alt::x("x"), alt::y("y")))?,
        );
    }
    let vrule = Chart::build(&ds(vec![
        ("x", vec![3.0].into_column()),
        ("y", vec![0.0].into_column()),
        ("y2", vec![6.0].into_column()),
    ])?)?
    .mark_rule()?
    .configure_rule(|r| r.with_color("#2ca02c").with_stroke_width(2.0))
    .encode((alt::x("x"), alt::y("y"), alt::y2("y2")))?;
    let mut c = pts.and(ident).and(vrule);
    for l in lines {
        c = c.and(l);
    }
    Ok(c.with_size(W, H)
        .with_title("rules: identity, levels (dashed), vertical")
        .with_x_label("observed")
        .with_y_label("predicted"))
}

/// A closed polygon (a hexagon, the lobule) with fill and stroke.
fn polygon() -> R<LayeredChart> {
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut g = Vec::new();
    for k in 0..6 {
        let a = std::f64::consts::PI / 3.0 * k as f64;
        x.push(a.cos());
        y.push(a.sin());
        g.push("hex");
    }
    let p = Chart::build(&ds(vec![
        ("x", x.into_column()),
        ("y", y.into_column()),
        ("g", g.into_column()),
    ])?)?
    .mark_polygon()?
    .configure_polygon(|p| {
        p.with_fill("#fdd0a2")
            .with_stroke("#a63603")
            .with_stroke_width(2.0)
    })
    .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?;
    Ok(p.with_size(W, W)
        .with_title("polygon (hexagon)")
        .with_x_domain(-1.2, 1.2)
        .with_y_domain(-1.2, 1.2)
        .with_x_label("x")
        .with_y_label("y"))
}

// ---------------------------------------------------------------- colour bar
/// The lines of a scan with 11 points coloured by a continuous field:
/// Charton draws the colour bar of a continuous colour encoding itself.
fn colorbar() -> R<LayeredChart> {
    let t = linspace(0.0, 24.0, 120);
    let mut tt = Vec::new();
    let mut yy = Vec::new();
    let mut dose = Vec::new();
    for k in 0..11 {
        let d = 10.0 + 10.0 * k as f64;
        for &x in &t {
            tt.push(x);
            yy.push(d / 20.0 * (-x / 6.0).exp() * x);
            dose.push(d);
        }
    }
    let l = Chart::build(&ds(vec![
        ("time", tt.into_column()),
        ("conc", yy.into_column()),
        ("dose", dose.into_column()),
    ])?)?
    .mark_line()?
    .encode((alt::x("time"), alt::y("conc"), alt::color("dose")))?;
    Ok(l.with_size(W, H)
        .with_title("scan over 11 doses, colour bar")
        .with_x_label("time [hr]")
        .with_y_label("conc [mg/l]")
        .configure_theme(|t| t.with_color_map(ColorMap::Viridis)))
}

// ---------------------------------------------------------------- log axes
/// Log x and log y. Charton's line, area, path, polygon and text marks refuse
/// a log x (and path, polygon and text a log y), so log y is Charton's
/// `Scale::Log` (named on every layer) and log x is log10 of the data on a
/// linear axis with explicit ticks `10^k` written by the crate.
fn log_axes() -> R<LayeredChart> {
    let x: Vec<f64> = (0..200)
        .map(|i| 10f64.powf(-2.0 + 4.0 * i as f64 / 199.0))
        .collect();
    let y: Vec<f64> = x.iter().map(|v| 5.0 / (1.0 + v.powf(1.5))).collect();
    let xd = vec![0.03, 0.3, 3.0, 30.0];
    let yd: Vec<f64> = xd
        .iter()
        .map(|v: &f64| 5.0 / (1.0 + v.powf(1.5)) * 1.1)
        .collect();
    let lx: Vec<f64> = x.iter().map(|v| v.log10()).collect();
    let lxd: Vec<f64> = xd.iter().map(|v| v.log10()).collect();
    let l = Chart::build(&ds(vec![("x", lx.into_column()), ("y", y.into_column())])?)?
        .mark_line()?
        .encode((alt::x("x"), ylin("y", true)))?;
    let p = Chart::build(&ds(vec![
        ("x", lxd.into_column()),
        ("y", yd.into_column()),
    ])?)?
    .mark_point()?
    .configure_point(|p| p.with_color("#d62728"))
    .encode((alt::x("x"), ylin("y", true)))?;
    let ticks: Vec<(f64, String)> = (-2..=2)
        .map(|k| (k as f64, format!("{}", 10f64.powi(k))))
        .collect();
    Ok(l.and(p)
        .with_size(W, H)
        .with_title("log y (Scale::Log), log x (log10 + ticks)")
        .with_x_domain(-2.0, 2.0)
        .with_x_ticks(ticks)
        .with_x_label("dose [mg]")
        .with_y_label("response"))
}

// ---------------------------------------------------------------- categorical
/// A categorical x axis with points and a line through the categories, the
/// tick labels rotated.
fn categorical() -> R<LayeredChart> {
    let c = vec![
        "Smith2001",
        "Jones2004",
        "Lee2010",
        "Meyer2015",
        "Kowalski2020",
    ];
    let v = vec![1.2, 3.4, 2.2, 4.0, 2.9];
    let p = Chart::build(&ds(vec![
        ("study", c.into_column()),
        ("v", v.into_column()),
    ])?)?
    .mark_point()?
    .configure_point(|p| p.with_size(5.0))
    .encode((alt::x("study"), alt::y("v")))?;
    Ok(p.with_size(W, H)
        .with_title("categorical x axis, rotated labels")
        .with_x_label("study")
        .with_y_label("clearance [ml/min]")
        .configure_theme(|t| t.with_x_tick_label_angle(-45.0)))
}

// ---------------------------------------------------------------- right axis
/// The left chart of a plot with a right y axis.
fn right_primary() -> R<LayeredChart> {
    let t = linspace(0.0, 24.0, 200);
    let y: Vec<f64> = t.iter().map(|&x| 4.0 * (-x / 6.0).exp() * x).collect();
    Ok(Chart::build(&ds(vec![
        ("time", t.into_column()),
        ("conc", y.into_column()),
    ])?)?
    .mark_line()?
    .configure_line(|l| l.with_color("#1f77b4").with_stroke_width(2.0))
    .encode((alt::x("time"), alt::y("conc")))?
    .with_size(W, H)
    .with_x_domain(0.0, 24.0)
    .with_x_expand(Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    })
    .with_title("right y axis (overlay + axis drawn by the crate)")
    .with_x_label("time [hr]")
    .with_y_label("conc [mg/l]")
    .configure_theme(|t| t.with_right_margin(0.12)))
}

/// The layers of the right y axis: a second chart without axes and grid, its
/// margins set so its panel is the panel of the primary chart.
fn right_overlay(panel: &Rect) -> R<LayeredChart> {
    let t = linspace(0.0, 24.0, 200);
    let y: Vec<f64> = t
        .iter()
        .map(|&x| 1000.0 * (1.0 - (-x / 8.0).exp()))
        .collect();
    let (w, h) = (W as f64, H as f64);
    Ok(Chart::build(&ds(vec![
        ("time", t.into_column()),
        ("amount", y.into_column()),
    ])?)?
    .mark_line()?
    .configure_line(|l| {
        l.with_color("#d62728")
            .with_stroke_width(2.0)
            .with_dash(vec![5.0, 3.0])
    })
    .encode((alt::x("time"), alt::y("amount")))?
    .with_size(W, H)
    .with_x_domain(0.0, 24.0)
    .with_x_expand(Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    })
    .with_y_domain(0.0, 1000.0)
    .with_y_expand(Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    })
    .with_margins(
        panel.y / h,
        (w - panel.x - panel.width) / w,
        (h - panel.y - panel.height) / h,
        panel.x / w,
    )
    .configure_theme(|t| {
        t.with_show_axes(false)
            .with_grid(false)
            .with_show_legend(false)
            .with_background_color("none")
    }))
}

/// Composes the primary chart, the overlay and a right axis drawn by the crate.
fn right_axis() -> R<String> {
    let primary = right_primary()?;
    let geo = geometry(&primary)?;
    let panel = geo.panel.ok_or("no panel")?;
    let overlay = right_overlay(&panel)?;
    let ogeo = geometry(&overlay)?;
    let opanel = ogeo.panel.ok_or("no overlay panel")?;
    let a = primary.to_svg_with(&SvgOptions::default().with_id_prefix("p"))?;
    let b = overlay.to_svg_with(&SvgOptions::default().with_id_prefix("o"))?;
    let mut svg = String::new();
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">"#
    )?;
    svg.push_str(&strip_xml(&a));
    svg.push_str(&strip_xml(&b));
    // right axis: spine, ticks, labels, title (nice ticks of 0..1000)
    let x0 = panel.x + panel.width;
    write!(
        svg,
        r##"<g class="plot-axis-right"><line x1="{x0}" y1="{}" x2="{x0}" y2="{}" stroke="#333" stroke-width="1"/>"##,
        panel.y,
        panel.y + panel.height
    )?;
    for k in 0..=5 {
        let v = 200.0 * k as f64;
        let y = panel.y + panel.height * (1.0 - v / 1000.0);
        write!(
            svg,
            r##"<line x1="{x0}" y1="{y}" x2="{}" y2="{y}" stroke="#333"/><text x="{}" y="{y}" font-size="11" dominant-baseline="central" fill="#333" font-family="Inter, sans-serif">{v}</text>"##,
            x0 + 5.0,
            x0 + 8.0
        )?;
    }
    let (tx, ty) = (x0 + 45.0, panel.y + panel.height / 2.0);
    write!(
        svg,
        r##"<text x="{tx}" y="{ty}" font-size="12" text-anchor="middle" fill="#d62728" transform="rotate(90 {tx} {ty})" font-family="Inter, sans-serif">amount [mg]</text></g></svg>"##
    )?;
    if std::env::var("SHOW_PANELS").is_ok() {
        println!(
            "right axis: primary panel {:?}, overlay panel {:?}",
            rect(&panel),
            rect(&opanel)
        );
    }
    Ok(svg)
}

// ---------------------------------------------------------------- grid
/// A 2x2 grid of independent plots (different datasets, scales and axes)
/// composed by the crate as nested <svg> elements, with one legend per row
/// drawn by the crate.
fn grid() -> R<String> {
    let (cw, ch) = (360u32, 280u32);
    let legend_w = 130u32;
    let mut cells: Vec<LayeredChart> = Vec::new();
    // (0,0) curve, (0,1) log, (1,0) bar, (1,1) band
    cells.push(
        curve()?
            .with_size(cw, ch)
            .with_title("A")
            .configure_theme(|t| t.with_show_legend(false)),
    );
    cells.push(
        log_axes()?
            .with_size(cw, ch)
            .with_title("B")
            .configure_theme(|t| t.with_show_legend(false)),
    );
    cells.push(
        bar()?
            .with_size(cw, ch)
            .with_title("C")
            .configure_theme(|t| t.with_show_legend(false)),
    );
    cells.push(
        band()?
            .with_size(cw, ch)
            .with_title("D")
            .configure_theme(|t| t.with_show_legend(false)),
    );
    // align the panels: a first pass measures the axis depth of every chart
    // with margins of 0 (the recording backend, no SVG), then each chart gets
    // margins which put its panel at the same box as the others of its column
    // (left, right) and row (top, bottom)
    let (w, h) = (cw as f64, ch as f64);
    let pad = 6.0;
    let mut depth = Vec::new();
    for c in &cells {
        let c0 = c.clone().with_margins(0.0, 0.0, 0.0, 0.0);
        let p = geometry(&c0)?.panel.ok_or("no panel")?;
        depth.push((p.y, w - p.x - p.width, h - p.y - p.height, p.x));
    }
    let mut cells2 = Vec::new();
    for (i, c) in cells.into_iter().enumerate() {
        let (r, col) = (i / 2, i % 2);
        let top = (0..2).map(|k| depth[r * 2 + k].0).fold(0.0, f64::max) + pad;
        let bottom = (0..2).map(|k| depth[r * 2 + k].2).fold(0.0, f64::max) + pad;
        let left = (0..2).map(|k| depth[k * 2 + col].3).fold(0.0, f64::max) + pad;
        let right = pad;
        let d = depth[i];
        cells2.push(c.with_margins(
            (top - d.0) / h,
            (right - d.1) / w,
            (bottom - d.2) / h,
            (left - d.3) / w,
        ));
    }
    let cells = cells2;
    for (i, c) in cells.iter().enumerate() {
        if let Some(p) = geometry(c)?.panel {
            if std::env::var("SHOW_PANELS").is_ok() {
                println!("grid cell {i}: panel {:?}", rect(&p));
            }
        }
    }
    let (fw, fh) = (2 * cw + legend_w, 2 * ch + 30);
    let mut svg = String::new();
    write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{fw}" height="{fh}" viewBox="0 0 {fw} {fh}"><rect width="100%" height="100%" fill="white"/>"##
    )?;
    write!(
        svg,
        r##"<text x="{}" y="20" font-size="16" text-anchor="middle" font-family="Inter, sans-serif">2x2 grid of independent plots, row legends</text>"##,
        fw / 2
    )?;
    for (i, c) in cells.iter().enumerate() {
        let (r, col) = (i / 2, i % 2);
        let s = c.to_svg_with(&SvgOptions::default().with_id_prefix(format!("g{i}")))?;
        write!(
            svg,
            r#"<svg x="{}" y="{}" width="{cw}" height="{ch}">"#,
            col as u32 * cw,
            30 + r as u32 * ch
        )?;
        svg.push_str(&strip_outer(&s));
        svg.push_str("</svg>");
    }
    let rows = [
        vec![
            ("model", "#1f77b4", "line", true),
            ("data", "#ff7f0e", "marker", false),
        ],
        vec![
            ("AUC", "#8c564b", "rect", false),
            ("median", "#2ca02c", "line", false),
            ("5-95 %", "#2ca02c", "band", false),
        ],
    ];
    for (r, entries) in rows.iter().enumerate() {
        let (x, y0) = (2 * cw + 10, 30 + r as u32 * ch + 40);
        write!(svg, r#"<g class="plot-legend" data-row="{r}">"#)?;
        for (k, (label, color, kind, dashed)) in entries.iter().enumerate() {
            let y = y0 + 20 * k as u32;
            match *kind {
                "line" => write!(
                    svg,
                    r#"<line x1="{x}" y1="{y}" x2="{}" y2="{y}" stroke="{color}" stroke-width="2"{}/>"#,
                    x + 24,
                    if *dashed {
                        r#" stroke-dasharray="6 3""#
                    } else {
                        ""
                    }
                )?,
                "marker" => write!(
                    svg,
                    r#"<rect x="{}" y="{}" width="8" height="8" fill="{color}" stroke="black"/>"#,
                    x + 8,
                    y - 4
                )?,
                "band" => write!(
                    svg,
                    r#"<rect x="{x}" y="{}" width="24" height="10" fill="{color}" fill-opacity="0.25"/>"#,
                    y - 5
                )?,
                _ => write!(
                    svg,
                    r#"<rect x="{x}" y="{}" width="24" height="10" fill="{color}" stroke="black" stroke-width="0.8"/>"#,
                    y - 5
                )?,
            }
            write!(
                svg,
                r##"<text x="{}" y="{y}" font-size="12" dominant-baseline="central" font-family="Inter, sans-serif" fill="#333">{label}</text>"##,
                x + 30
            )?;
        }
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    Ok(svg)
}

/// Drops an XML declaration, keeps the <svg> element (placed at 0,0).
fn strip_xml(s: &str) -> String {
    match s.find("<svg") {
        Some(i) => s[i..].to_string(),
        None => s.to_string(),
    }
}

/// The content of the outer <svg> of a chart, for nesting in another <svg>.
fn strip_outer(s: &str) -> String {
    let s = strip_xml(s);
    let start = s.find('>').map(|i| i + 1).unwrap_or(0);
    let end = s.rfind("</svg>").unwrap_or(s.len());
    s[start..end].to_string()
}

// ---------------------------------------------------------------- data-element
/// Adds `data-element` to every mark group by its layer index: Charton writes
/// `data-layer="N"` on the group of the marks of layer N.
fn tag_elements(svg: &str, ids: &[&str]) -> String {
    let mut out = svg.to_string();
    for (i, id) in ids.iter().enumerate().rev() {
        let needle = format!(r#"data-layer="{i}""#);
        out = out.replace(&needle, &format!(r#"data-layer="{i}" data-element="{id}""#));
    }
    out
}

// ---------------------------------------------------------------- geometry
/// What a recording backend learns of a chart without writing SVG: the panel
/// (the clip of the plot group), the groups per role and the primitives.
#[derive(Default)]
struct Geometry {
    panel: Option<Rect>,
    groups: Vec<String>,
    primitives: usize,
    texts: Vec<(f32, f32, String, f32, String)>,
}

struct Recorder {
    geo: Geometry,
}

impl RenderBackend for Recorder {
    fn draw_circle(&mut self, _: CircleConfig) {
        self.geo.primitives += 1;
    }
    fn draw_rect(&mut self, _: RectConfig) {
        self.geo.primitives += 1;
    }
    fn draw_path(&mut self, _: PathConfig) {
        self.geo.primitives += 1;
    }
    fn draw_text(&mut self, c: TextConfig) -> Result<(), ChartonError> {
        self.geo.primitives += 1;
        self.geo
            .texts
            .push((c.x, c.y, c.text, c.font_size, c.text_anchor));
        Ok(())
    }
    fn draw_line(&mut self, _: LineConfig) {
        self.geo.primitives += 1;
    }
    fn draw_gradient_rect(&mut self, _: GradientRectConfig) {
        self.geo.primitives += 1;
    }
    fn begin_group(&mut self, g: &Group<'_>) {
        if matches!(g.role, Role::Plot) && self.geo.panel.is_none() {
            self.geo.panel = g.clip;
        }
        self.geo.groups.push(format!(
            "{:?} layer={:?} mark={:?}",
            g.role, g.layer, g.mark
        ));
    }
    fn begin_element(&mut self, _: Element<'_>) {}
}

fn geometry(chart: &LayeredChart) -> R<Geometry> {
    let mut rec = Recorder {
        geo: Geometry::default(),
    };
    let mut c = chart.clone();
    c.render(&mut rec)?;
    Ok(rec.geo)
}

fn rect(r: &Rect) -> (f64, f64, f64, f64) {
    (r.x, r.y, r.width, r.height)
}

// ---------------------------------------------------------------- benchmark
/// The ten colours of the series cycle.
const TAB10: [&str; 10] = [
    "#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2", "#7f7f7f",
    "#bcbd22", "#17becf",
];

/// The tick label of the decade `10^k`.
fn decade_label(k: i32) -> String {
    if (-3..=3).contains(&k) {
        format!("{}", 10f64.powi(k))
    } else {
        format!("1e{k}")
    }
}

/// A width of a text in pixels: a rough stand-in for the Inter table of
/// decision (d), enough to size the legend column of the spike.
fn text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * 0.6 * size
}

/// The benchmark figure of the design (8 curves of 500 points, 20 points with
/// x and y error bars, log y), built as decisions (a) to (e) say the crate
/// builds it: log y is log10 of the data on a linear axis with the decade
/// ticks of the crate, the x error bars a path group per point, the y error
/// bars Charton's error bar on the log10 bounds, Charton's legend off and the
/// crate's legend (with `data-legend`) in a right margin the crate reserves,
/// the panel placed with the two-pass margins, every mark group tagged with
/// `data-element`, and the theme tokens in a scoped `<style>` of
/// `var(--sbmlsim-<token>, default)`.
fn bench() -> R<String> {
    let n = 500;
    let t: Vec<f64> = (0..n).map(|i| 0.01 + i as f64 * 24.0 / n as f64).collect();
    let mut layers: Vec<LayeredChart> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut labels: Vec<(String, &str, bool)> = Vec::new();
    let (mut lmin, mut lmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in 0..8 {
        let ly: Vec<f64> = t
            .iter()
            .map(|x| ((1.0 + k as f64) * (-x / 5.0).exp() * x + 1e-3).log10())
            .collect();
        for v in &ly {
            lmin = lmin.min(*v);
            lmax = lmax.max(*v);
        }
        let color = TAB10[k];
        layers.push(
            Chart::build(&ds(vec![
                ("time", t.clone().into_column()),
                ("y", ly.into_column()),
            ])?)?
            .mark_line()?
            .configure_line(move |l| l.with_color(color).with_stroke_width(1.5))
            .encode((alt::x("time"), alt::y("y")))?
            .into(),
        );
        ids.push(format!("c{k}"));
        labels.push((format!("dose {} mg", 10 * (k + 1)), color, false));
    }
    let td: Vec<f64> = (0..20).map(|i| 0.5 + i as f64).collect();
    let yd: Vec<f64> = td
        .iter()
        .map(|x| 3.0 * (-x / 5.0).exp() * x + 1e-3)
        .collect();
    let lyd: Vec<f64> = yd.iter().map(|y| y.log10()).collect();
    let llo: Vec<f64> = yd.iter().map(|y| (y * 0.8).log10()).collect();
    let lhi: Vec<f64> = yd.iter().map(|y| (y * 1.2).log10()).collect();
    let (mut px, mut py, mut pg) = (Vec::new(), Vec::new(), Vec::new());
    for (i, (x, y)) in td.iter().zip(&lyd).enumerate() {
        px.extend([x - 0.5, x + 0.5]);
        py.extend([*y, *y]);
        pg.extend([format!("p{i}"), format!("p{i}")]);
    }
    let data = "#000000";
    layers.push(
        Chart::build(&ds(vec![
            ("x", px.into_column()),
            ("y", py.into_column()),
            ("g", pg.into_column()),
        ])?)?
        .mark_path()?
        .configure_path(move |p| p.with_stroke(data).with_stroke_width(1.0))
        .encode((alt::x("x"), alt::y("y"), alt::path_group("g")))?
        .into(),
    );
    ids.push("data-xerr".into());
    layers.push(
        Chart::build(&ds(vec![
            ("time", td.clone().into_column()),
            ("lo", llo.into_column()),
            ("hi", lhi.into_column()),
        ])?)?
        .mark_errorbar()?
        .configure_errorbar(move |e| e.with_color(data).with_cap_length(3.0))
        .encode((alt::x("time"), alt::y("lo"), alt::y2("hi")))?
        .into(),
    );
    ids.push("data-yerr".into());
    layers.push(
        Chart::build(&ds(vec![
            ("time", td.into_column()),
            ("y", lyd.into_column()),
        ])?)?
        .mark_point()?
        .configure_point(move |p| p.with_color(data).with_size(3.5))
        .encode((alt::x("time"), alt::y("y")))?
        .into(),
    );
    ids.push("data".into());
    labels.push(("data (mean +- SD)".into(), data, true));

    let (k0, k1) = (lmin.floor() as i32, lmax.ceil() as i32);
    let ticks: Vec<(f64, String)> = (k0..=k1).map(|k| (k as f64, decade_label(k))).collect();
    let mut it = layers.into_iter();
    let mut chart = it.next().ok_or("no layer")?;
    for l in it {
        chart = chart.and(l);
    }
    let zero = Expansion {
        mult: (0.0, 0.0),
        add: (0.0, 0.0),
    };
    let chart = chart
        .with_size(W, H)
        .with_x_label("time [hr]")
        .with_y_label("concentration [mg/l]")
        .with_x_domain(0.0, 24.0)
        .with_x_expand(zero)
        .with_y_domain(k0 as f64, k1 as f64)
        .with_y_expand(zero)
        .with_y_ticks(ticks)
        .configure_theme(|t| t.with_show_legend(false));

    // the legend column, then the panel: pass 1 measures the axis depth
    let (w, h) = (W as f64, H as f64);
    let (fs, row, swatch, gap, pad) = (12.0, 18.0, 24.0, 6.0, 8.0);
    let legend_w = swatch
        + gap
        + labels
            .iter()
            .map(|(l, _, _)| text_width(l, fs))
            .fold(0.0, f64::max);
    let p0 = geometry(&chart.clone().with_margins(0.0, 0.0, 0.0, 0.0))?
        .panel
        .ok_or("no panel")?;
    // the depth of the right side (none: the y axis is on the left)
    let dr = w - p0.x - p0.width;
    let right = legend_w + 2.0 * pad;
    // the title band of the figure is the crate's: Charton centres a title
    // over the canvas inside its margins, which the legend column narrows
    let title_band = 26.0;
    let chart = chart.with_margins((pad + title_band) / h, (right - dr) / w, pad / h, pad / w);
    let panel = geometry(&chart)?.panel.ok_or("no panel")?;

    let svg = chart.to_svg_with(&SvgOptions::default().with_id_prefix("bench"))?;
    let ids: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let mut svg =
        tag_elements(&svg, &ids).replacen(r#"class="charton""#, r#"id="bench" class="charton""#, 1);
    let style = concat!(
        "<style>",
        "#bench .charton-tick-label text{fill:var(--sbmlsim-fg-soft, rgb(91,100,114))}",
        "#bench .charton-axis-title text,#bench .plot-figure-title text,#bench .plot-legend text{fill:var(--sbmlsim-fg, rgb(27,31,36))}",
        "#bench .charton-mark-line path{stroke-width:var(--sbmlsim-curve-width, 1.8px)}",
        "</style>"
    );
    let at = svg.find("<defs>").ok_or("no defs")?;
    svg.insert_str(at, style);
    // the legend of the crate, one group per entry carrying its element
    let mut lg = String::new();
    let (x, y0) = (panel.x + panel.width + pad, panel.y + 4.0);
    lg.push_str(r#"<g class="plot-legend">"#);
    for (k, (label, color, marker)) in labels.iter().enumerate() {
        let y = y0 + row * k as f64 + row / 2.0;
        let element = ids.get(k).copied().unwrap_or("data");
        let element = if *marker { "data" } else { element };
        write!(
            lg,
            r#"<g class="plot-legend-entry" data-legend="{element}">"#
        )?;
        if *marker {
            write!(
                lg,
                r#"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{color}"/><circle cx="{:.2}" cy="{y:.2}" r="3.5" fill="{color}"/>"#,
                x + swatch / 2.0,
                y - 6.0,
                x + swatch / 2.0,
                y + 6.0,
                x + swatch / 2.0
            )?;
        } else {
            write!(
                lg,
                r#"<line x1="{x:.2}" y1="{y:.2}" x2="{:.2}" y2="{y:.2}" stroke="{color}" stroke-width="1.8"/>"#,
                x + swatch
            )?;
        }
        write!(
            lg,
            r#"<text x="{:.2}" y="{y:.2}" font-size="{fs}" dominant-baseline="central" font-family="Inter, sans-serif" fill="rgb(27,31,36)">{label}</text></g>"#,
            x + swatch + gap
        )?;
    }
    lg.push_str("</g>");
    write!(
        lg,
        r#"<g class="plot-figure-title"><text x="{:.2}" y="{:.2}" font-size="16" font-weight="bold" text-anchor="middle" dominant-baseline="central" font-family="Inter, sans-serif" fill="rgb(27,31,36)">benchmark: 8 x 500, 20 points with x/y errors, log y</text></g>"#,
        w / 2.0,
        pad + title_band / 2.0
    )?;
    let end = svg.rfind("</svg>").ok_or("no end")?;
    svg.insert_str(end, &lg);
    Ok(svg)
}

// ---------------------------------------------------------------- main
type Build = fn() -> R<String>;

fn svg_of(f: fn() -> R<LayeredChart>) -> R<String> {
    Ok(f()?.to_svg()?)
}

macro_rules! chart_fig {
    ($name:ident) => {
        (stringify!($name), (|| svg_of($name)) as Build)
    };
}

fn main() -> R<()> {
    std::fs::create_dir_all("out")?;
    let figs: Vec<(&str, Build)> = vec![
        chart_fig!(curve),
        chart_fig!(band),
        chart_fig!(area),
        chart_fig!(bar),
        chart_fig!(boxes),
        chart_fig!(violin),
        chart_fig!(histogram),
        chart_fig!(heatmap),
        chart_fig!(text),
        chart_fig!(rule),
        chart_fig!(polygon),
        chart_fig!(colorbar),
        chart_fig!(log_axes),
        chart_fig!(categorical),
        ("right_axis", right_axis as Build),
        ("grid", grid as Build),
        ("bench", bench as Build),
    ];
    let reps = std::env::var("REPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200usize);
    println!("| figure | ms/figure | bytes |");
    println!("|---|---:|---:|");
    for (name, f) in &figs {
        let svg = f()?;
        std::fs::write(format!("out/{name}.svg"), &svg)?;
        let s = Instant::now();
        for _ in 0..reps {
            std::hint::black_box(f()?);
        }
        println!(
            "| {name} | {:.3} | {} |",
            s.elapsed().as_secs_f64() * 1e3 / reps as f64,
            svg.len()
        );
    }

    // geometry through a recording backend, and its cost
    let c = curve()?;
    let g = geometry(&c)?;
    println!(
        "curve panel {:?}, {} primitives, groups:",
        g.panel.as_ref().map(rect),
        g.primitives
    );
    for grp in g
        .groups
        .iter()
        .filter(|s| s.starts_with("Mark") || s.starts_with("Plot") || s.starts_with("Legend"))
    {
        println!("  {grp}");
    }
    let s = Instant::now();
    for _ in 0..reps {
        std::hint::black_box(geometry(&c)?);
    }
    println!(
        "geometry(curve): {:.3} ms",
        s.elapsed().as_secs_f64() * 1e3 / reps as f64
    );
    let s = Instant::now();
    for _ in 0..reps {
        std::hint::black_box(c.to_svg()?);
    }
    println!(
        "to_svg(curve) without building: {:.3} ms",
        s.elapsed().as_secs_f64() * 1e3 / reps as f64
    );

    // the texts Charton laid out (for the text measure comparison)
    let g = geometry(&categorical()?)?;
    for (x, y, t, size, anchor) in g.texts.iter().take(12) {
        println!("  text {t:?} at ({x:.1},{y:.1}) size {size} anchor {anchor}");
    }

    // data-element on the mark groups
    let tagged = tag_elements(&c.to_svg()?, &["model", "data-xerr", "data-yerr", "data"]);
    std::fs::write("out/curve_tagged.svg", &tagged)?;
    println!("tagged groups: {}", tagged.matches("data-element=").count());

    // PNG through Charton's raster backend
    #[cfg(feature = "png")]
    {
        let s = Instant::now();
        for _ in 0..20 {
            std::hint::black_box(curve()?.to_png()?);
        }
        println!(
            "to_png(curve): {:.2} ms",
            s.elapsed().as_secs_f64() * 1e3 / 20.0
        );
        std::fs::write("out/curve_charton.png", curve()?.to_png()?)?;
    }
    Ok(())
}

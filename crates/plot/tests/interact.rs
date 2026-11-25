//! The geometry of a figure and the pick of a point.

use sbmlsim_plot::{Geometry, Scale, geometry, parse, pick};

/// A figure of one plot as JSON text.
fn figure(x: &str, y: &str, extra: &str, elements: &str) -> sbmlsim_plot::Figure {
    parse(&format!(
        r#"{{"contract": "figure/1", "width": 600, "height": 400,
            "grid": {{"rows": 1, "columns": 1}},
            "plots": [{{"id": "p", "row": 0, "column": 0, "x": {x}, "y": {y} {extra},
                        "elements": [{elements}]}}]}}"#
    ))
    .unwrap_or_else(|e| panic!("{e}"))
}

/// The pixel of a value on an axis of a panel side `[start, start + size]`; `flip` for y.
fn axis_pixel(s: &Scale, v: f64, start: f64, size: f64, flip: bool) -> f64 {
    let t = |v: f64| match s.kind {
        sbmlsim_plot::AxisScale::Log => v.log10(),
        _ => v,
    };
    let mut share = (t(v) - t(s.low)) / (t(s.high) - t(s.low));
    if s.reverse {
        share = 1.0 - share;
    }
    if flip {
        start + size - share * size
    } else {
        start + share * size
    }
}

/// The pixel of a point of the plot `p`, on the left or right y axis.
fn pixel(g: &Geometry, x: f64, y: f64, right: bool) -> (f64, f64) {
    let p = &g.plots[0];
    let ys = if right { p.y2.as_ref().unwrap() } else { &p.y };
    (
        axis_pixel(&p.x, x, p.rect.x, p.rect.width, false),
        axis_pixel(ys, y, p.rect.y, p.rect.height, true),
    )
}

const LIN: &str = r#"{"label": "t"}"#;
const CURVE: &str = r#"{"id": "c", "label": "conc", "kind": "curve",
    "x": [0, 1, 2, 3, 4], "y": [1, 4, 2, 8, 5], "yerr": [0.1, 0.2, 0.3, 0.4, 0.5]}"#;

#[test]
fn pick_curve_point() {
    let f = figure(LIN, LIN, "", CURVE);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 3.0, 8.0, false);
    let p = pick(&f, px + 1.0, py - 1.0, 5.0).expect("a pick");
    assert_eq!(
        (p.plot.as_str(), p.element.as_str(), p.index),
        ("p", "c", 3)
    );
    assert_eq!(p.values["x"], 3.0);
    assert_eq!(p.values["y"], 8.0);
    assert_eq!(p.values["yerr"], 0.4);
    assert!(!p.values.contains_key("xerr"));
    assert_eq!(p.label.as_deref(), Some("conc"));
    assert_eq!(p.legend.as_deref(), Some("c"));
}

#[test]
fn pick_log_axis() {
    let f = figure(LIN, r#"{"scale": "log"}"#, "", CURVE);
    let g = geometry(&f);
    assert_eq!(serde_json::to_value(g.plots[0].y).unwrap()["kind"], "log");
    let (px, py) = pixel(&g, 3.0, 8.0, false);
    let p = pick(&f, px, py, 3.0).expect("a pick");
    assert_eq!((p.index, p.values["y"]), (3, 8.0));
}

#[test]
fn pick_misses() {
    let f = figure(LIN, LIN, "", CURVE);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 3.0, 8.0, false);
    assert!(pick(&f, px + 30.0, py + 30.0, 5.0).is_none());
    assert!(pick(&f, -50.0, -50.0, 5.0).is_none());
}

#[test]
fn pick_nearest_of_two() {
    let two = r#"{"id": "a", "kind": "curve", "x": [0, 1], "y": [0, 1]},
        {"id": "b", "kind": "curve", "x": [0, 1], "y": [0.05, 1.5]}"#;
    let f = figure(LIN, LIN, "", two);
    let g = geometry(&f);
    let (ax, ay) = pixel(&g, 1.0, 1.0, false);
    let (_, by) = pixel(&g, 1.0, 1.5, false);
    // nearer to b's point than to a's
    let p = pick(&f, ax, (ay + by) / 2.0 - 1.0, 100.0).unwrap();
    assert_eq!(p.element, "b");
    let p = pick(&f, ax, (ay + by) / 2.0 + 1.0, 100.0).unwrap();
    assert_eq!(p.element, "a");
}

#[test]
fn pick_heatmap_cell() {
    let hm = r#"{"id": "h", "label": "cells", "kind": "heatmap",
        "rows": ["r0", "r1"], "columns": ["c0", "c1", "c2"],
        "values": [[1, 2, 3], [4, 5, 6]], "scale": {"name": "viridis", "low": 1, "high": 6}}"#;
    let f = figure(LIN, LIN, "", hm);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 2.0, 1.0, false);
    let p = pick(&f, px, py, 0.0).expect("a cell");
    assert_eq!(p.index, 5);
    assert_eq!(p.values["value"], 6.0);
    assert_eq!(p.label.as_deref(), Some("r1 / c2"));
}

#[test]
fn pick_box() {
    let bx = r#"{"id": "b", "kind": "box", "positions": [1, 2],
        "stats": [{"low_whisker": 0, "q1": 1, "median": 2, "q3": 3, "high_whisker": 4},
                  {"low_whisker": 1, "q1": 2, "median": 3, "q3": 4, "high_whisker": 6}]}"#;
    let f = figure(LIN, LIN, "", bx);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 2.0, 3.0, false);
    let p = pick(&f, px, py, 0.0).expect("inside the box");
    assert_eq!(p.index, 1);
    for (k, v) in [
        ("low_whisker", 1.0),
        ("q1", 2.0),
        ("median", 3.0),
        ("q3", 4.0),
        ("high_whisker", 6.0),
    ] {
        assert_eq!(p.values[k], v, "{k}");
    }
}

#[test]
fn pick_respects_y2() {
    let two = r#"{"id": "l", "kind": "curve", "x": [0, 1, 2], "y": [1, 2, 3]},
        {"id": "r", "axis": "y2", "kind": "curve", "x": [0, 1, 2], "y": [100, 400, 200]}"#;
    let f = figure(LIN, LIN, r#", "y2": {"label": "u"}"#, two);
    let g = geometry(&f);
    assert!(g.plots[0].y2.is_some());
    let (px, py) = pixel(&g, 1.0, 400.0, true);
    let p = pick(&f, px, py, 3.0).expect("a pick");
    assert_eq!(
        (p.element.as_str(), p.index, p.values["y"]),
        ("r", 1, 400.0)
    );
}

#[test]
fn geometry_matches_layout() {
    let f = figure(LIN, LIN, "", CURVE);
    let g = geometry(&f);
    let svg = sbmlsim_plot::render_svg(&f);
    assert!(svg.contains("<svg"));
    let p = &g.plots[0];
    assert_eq!(p.id, "p");
    assert!(p.rect.x > 0.0 && p.rect.y > 0.0);
    assert!(p.rect.x + p.rect.width <= 600.0 && p.rect.y + p.rect.height <= 400.0);
    assert!(p.x.low <= 0.0 && p.x.high >= 4.0 && !p.x.reverse);
    // the geometry of a plan equals the geometry computed alone
    let plan = sbmlsim_plot::FigurePlan::new(&f);
    assert_eq!(sbmlsim_plot::geometry_of(&plan), g);
    // and serializes in snake_case
    let json = serde_json::to_value(&g).unwrap();
    assert!(json["legend_entries"].is_array());
}

#[test]
fn legend_entry_rects() {
    let two = r#"{"id": "a", "label": "A", "kind": "curve", "x": [0, 1], "y": [0, 1]},
        {"id": "b", "label": "B", "kind": "curve", "x": [0, 1], "y": [1, 0]}"#;
    let f = figure(LIN, LIN, "", two);
    let g = geometry(&f);
    assert_eq!(g.legend_entries.len(), 2);
    let names: Vec<&str> = g.legend_entries.iter().map(|e| e.legend.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    for e in &g.legend_entries {
        assert!(e.rect.width > 0.0 && e.rect.height > 0.0);
        assert_eq!(e.owner, "p");
    }
    // the entries do not overlap
    let (a, b) = (&g.legend_entries[0].rect, &g.legend_entries[1].rect);
    assert!(a.y + a.height <= b.y + 1e-6 || b.y + b.height <= a.y + 1e-6);
}

//! The geometry of a figure and the pick of a point.

use sbmlsim_plot::{FigurePlan, Geometry, Scale, geometry, parse, pick, pick_in};

mod support;

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
fn pick_vertical_bar() {
    let bar = r#"{"id": "b", "kind": "bar", "x": [1, 2, 3], "height": [2, 5, 3], "err": [0.5, 0.5, 0.5]}"#;
    let f = figure(LIN, LIN, "", bar);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 2.0, 2.5, false);
    let p = pick(&f, px, py, 0.0).expect("inside the bar");
    assert_eq!(p.index, 1);
    assert_eq!((p.values["height"], p.values["err"]), (5.0, 0.5));
    // in the gap between two bars there is nothing
    let (qx, qy) = pixel(&g, 2.5, 1.0, false);
    assert!(pick(&f, qx, qy, 0.0).is_none());
}

#[test]
fn pick_horizontal_bar() {
    let bar =
        r#"{"id": "b", "kind": "bar", "horizontal": true, "x": [1, 2, 3], "height": [2, 5, 3]}"#;
    let f = figure(LIN, LIN, "", bar);
    let g = geometry(&f);
    // positions on y, heights on x
    let (px, py) = pixel(&g, 2.5, 3.0, false);
    let p = pick(&f, px, py, 0.0).expect("inside the bar of position 3");
    assert_eq!((p.index, p.values["height"], p.values["x"]), (2, 3.0, 3.0));
    // beyond the end of the shortest bar, at its position: no pick
    let (qx, qy) = pixel(&g, 4.5, 1.0, false);
    assert!(pick(&f, qx, qy, 0.0).is_none());
}

#[test]
fn pick_histogram_bin() {
    let h = r#"{"id": "h", "kind": "histogram", "edges": [0, 1, 2, 3], "counts": [2, 7, 4]}"#;
    let f = figure(LIN, LIN, "", h);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 1.5, 3.0, false);
    let p = pick(&f, px, py, 0.0).expect("inside the bin");
    assert_eq!(p.index, 1);
    assert_eq!(
        (p.values["low"], p.values["high"], p.values["count"]),
        (1.0, 2.0, 7.0)
    );
}

#[test]
fn pick_rule() {
    let rules = r#"{"id": "hr", "kind": "rule", "orientation": "horizontal", "value": 3},
        {"id": "vr", "kind": "rule", "orientation": "vertical", "value": 1}"#;
    let f = figure(LIN, LIN, "", &format!("{rules}, {CURVE}"));
    let g = geometry(&f);
    let r = g.plots[0].rect;
    let (_, py) = pixel(&g, 0.0, 3.0, false);
    // between the curve points, close to the horizontal rule only
    let p = pick(&f, r.x + r.width * 0.9, py + 1.0, 3.0).expect("the rule");
    assert_eq!((p.element.as_str(), p.values["value"]), ("hr", 3.0));
    let (vx, _) = pixel(&g, 1.0, 0.0, false);
    let p = pick(&f, vx + 1.0, r.y + 2.0, 3.0).expect("the rule");
    assert_eq!((p.element.as_str(), p.values["value"]), ("vr", 1.0));
}

#[test]
fn pick_band() {
    let band = r#"{"id": "b", "kind": "band", "x": [0, 1, 2], "low": [0, 1, 2],
        "high": [2, 4, 3], "median": [1, 2, 2.5]}"#;
    let f = figure(LIN, LIN, "", band);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 1.0, 2.5, false);
    let p = pick(&f, px, py, 1.0).expect("inside the band at x=1");
    assert_eq!(p.index, 1);
    assert_eq!(
        (p.values["low"], p.values["high"], p.values["median"]),
        (1.0, 4.0, 2.0)
    );
}

#[test]
fn pick_text() {
    let t = r#"{"id": "t", "label": "note", "kind": "text", "x": 2, "y": 3, "text": "hello"}"#;
    let f = figure(LIN, LIN, "", &format!("{t}, {CURVE}"));
    let g = geometry(&f);
    let (px, py) = pixel(&g, 2.0, 3.0, false);
    let p = pick(&f, px, py, 2.0).expect("the text");
    assert_eq!((p.element.as_str(), p.values["y"]), ("t", 3.0));
    assert_eq!(p.label.as_deref(), Some("note"));
}

#[test]
fn pick_on_a_category_axis() {
    let bar = r#"{"id": "b", "kind": "bar", "x": [0, 1, 2], "height": [4, 9, 6]}"#;
    let f = figure(
        r#"{"scale": "category", "categories": ["a", "b", "c"]}"#,
        LIN,
        "",
        bar,
    );
    let g = geometry(&f);
    assert_eq!(
        serde_json::to_value(g.plots[0].x).unwrap()["kind"],
        "category"
    );
    let (px, py) = pixel(&g, 2.0, 3.0, false);
    let p = pick(&f, px, py, 0.0).expect("the third bar");
    assert_eq!((p.index, p.values["height"]), (2, 6.0));
}

#[test]
fn pick_on_a_reversed_axis() {
    let f = figure(r#"{"reverse": true}"#, r#"{"reverse": true}"#, "", CURVE);
    let g = geometry(&f);
    assert!(g.plots[0].x.reverse && g.plots[0].y.reverse);
    let (px, py) = pixel(&g, 3.0, 8.0, false);
    let p = pick(&f, px, py, 2.0).expect("a pick");
    assert_eq!((p.index, p.values["y"]), (3, 8.0));
    // the reversal puts the first point on the other side than the last
    let (x0, _) = pixel(&g, 0.0, 1.0, false);
    let (x4, _) = pixel(&g, 4.0, 5.0, false);
    assert!(x0 > x4);
}

#[test]
fn pick_y2_bar() {
    let two = r#"{"id": "l", "kind": "curve", "x": [0, 1, 2], "y": [1, 2, 3]},
        {"id": "b", "axis": "y2", "kind": "bar", "x": [0, 1, 2], "height": [100, 400, 200]}"#;
    let f = figure(LIN, LIN, r#", "y2": {"label": "u"}"#, two);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 1.0, 300.0, true);
    let p = pick(&f, px, py, 0.0).expect("inside the y2 bar");
    assert_eq!(
        (p.element.as_str(), p.index, p.values["height"]),
        ("b", 1, 400.0)
    );
}

#[test]
fn a_plan_of_another_figure_picks_nothing() {
    let f = figure(LIN, LIN, "", CURVE);
    let other = parse(&support::spec_text("grid")).unwrap();
    let plan = FigurePlan::new(&other);
    let g = geometry(&f);
    let (px, py) = pixel(&g, 3.0, 8.0, false);
    assert!(pick_in(&f, &plan, px, py, 5.0).is_none());
}

#[test]
fn the_plan_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<FigurePlan>();
}

/// The cost of a pick on the benchmark figure; run with
/// `cargo test --release -p sbmlsim-plot --test interact -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn pick_cost_on_the_benchmark() {
    let f = parse(&support::spec_text("benchmark")).unwrap();
    let plan = FigurePlan::new(&f);
    let g = geometry(&f);
    let r = g.plots[0].rect;
    let mut times = Vec::new();
    for k in 0..200 {
        let (x, y) = (r.x + r.width * (k as f64 / 200.0), r.y + r.height * 0.5);
        let t = std::time::Instant::now();
        std::hint::black_box(pick_in(&f, &plan, x, y, 8.0));
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(f64::total_cmp);
    println!("pick median {:.4} ms, max {:.4} ms", times[100], times[199]);
    assert!(times[100] < 0.1);
}

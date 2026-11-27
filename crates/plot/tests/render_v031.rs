//! The fields of `figure/1` since 0.31.0 (#115): colour scales of stops, the marks, the
//! placement, the length and the thickness of a colour bar, the ticks of an axis, tick labels
//! of several lines and with powers of ten, the multiplier of an axis, the markers of
//! matplotlib and their half fills, the width of a box and of error bars, the title and the
//! columns of a legend, the legend below the grid, the swatch of a band with its curve and the
//! size of the plot titles.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{
    ColorScale, ColorStop, Figure, FontFamily, FontWeight, check_stops, color_scale, geometry,
    parse, render_svg, text_width, text_width_family,
};
use support::{assert_golden, spec_text, used_markers};

/// Parses a figure of JSON text.
fn spec(json: &str) -> Figure {
    parse(json).unwrap_or_else(|e| panic!("{e}"))
}

/// A figure of one plot with the given axes, extra plot fields and elements, as JSON text.
fn figure(x: &str, y: &str, extra: &str, elements: &str) -> String {
    format!(
        r#"{{"contract": "figure/1", "width": 500, "height": 400,
            "grid": {{"rows": 1, "columns": 1}},
            "plots": [{{"id": "p", "row": 0, "column": 0, "x": {x}, "y": {y}, {extra}
                        "elements": [{elements}]}}]}}"#
    )
}

/// A curve from 0 to 1.
const CURVE: &str = r#"{"id": "c", "kind": "curve", "label": "c", "x": [0, 1], "y": [0, 1]}"#;

/// The groups of a class.
fn groups<'a>(doc: &'a Document<'a>, class: &str) -> Vec<Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| {
            n.has_tag_name("g")
                && n.attribute("class")
                    .is_some_and(|c| c.split(' ').any(|k| k == class))
        })
        .collect()
}

/// A number attribute of a node.
fn num(n: Node, name: &str) -> f64 {
    n.attribute(name)
        .unwrap_or_else(|| panic!("no {name}"))
        .parse()
        .unwrap()
}

/// The text of a node and its descendants.
fn text(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

/// The error of a spec which does not parse: its path and message.
fn refused(json: &str) -> (String, String) {
    let e = parse(json).expect_err("the spec is refused");
    (e.path, e.message)
}

#[test]
fn golden_v031() {
    assert_golden("v031", &render_svg(&spec(&spec_text("v031"))));
}

#[test]
fn golden_v031_legend() {
    assert_golden("v031_legend", &render_svg(&spec(&spec_text("v031_legend"))));
}

/// A scale of stops.
fn stops_scale(stops: &[(f64, &str)]) -> ColorScale {
    ColorScale {
        name: None,
        stops: Some(
            stops
                .iter()
                .map(|(p, c)| ColorStop(*p, c.to_string()))
                .collect(),
        ),
        low: 0.0,
        high: 1.0,
        center: None,
        log: false,
        gamma: None,
        start: 0.0,
        end: 1.0,
    }
}

#[test]
fn a_scale_of_stops_has_the_colours_of_matplotlib() {
    // matplotlib 3.11.2: to_hex(LinearSegmentedColormap.from_list(name, stops)(v))
    let s = stops_scale(&[
        (0.0, "#006d77"),
        (0.3, "#2a9d4b"),
        (0.6, "#74c69d"),
        (1.0, "#d8f3dc"),
    ]);
    let values = [0.0, 0.1, 0.3, 0.45, 0.6, 0.77, 0.999, 1.0];
    assert_eq!(
        color_scale(&s, &values),
        [
            "#006d77", "#0e7d69", "#2a9d4b", "#4fb274", "#74c69d", "#9fd9b8", "#d8f3dc", "#d8f3dc"
        ]
    );
    // two stops at one position are a step
    let step = stops_scale(&[
        (0.0, "#000000"),
        (0.5, "#ff0000"),
        (0.5, "#0000ff"),
        (1.0, "#ffffff"),
    ]);
    assert_eq!(
        color_scale(&step, &[0.25, 0.49, 0.5, 0.51, 0.75]),
        ["#800000", "#fa0000", "#0101ff", "#0505ff", "#8181ff"]
    );
}

#[test]
fn stops_which_are_no_colour_map_are_refused() {
    let stop = |p: f64, c: &str| ColorStop(p, c.to_string());
    assert!(check_stops(&[stop(0.0, "#000000"), stop(1.0, "#ffffff")]).is_ok());
    assert_eq!(check_stops(&[stop(0.0, "#000000")]).unwrap_err().0, None);
    assert_eq!(
        check_stops(&[stop(0.1, "#000000"), stop(1.0, "#ffffff")])
            .unwrap_err()
            .0,
        Some(0)
    );
    let e = check_stops(&[
        stop(0.0, "#000000"),
        stop(0.6, "#ff0000"),
        stop(0.4, "#00ff00"),
        stop(1.0, "#ffffff"),
    ])
    .unwrap_err();
    assert_eq!(e.0, Some(2));
    let e = check_stops(&[stop(0.0, "black"), stop(1.0, "#ffffff")]).unwrap_err();
    assert!(e.1.contains("#rrggbb"), "{}", e.1);
    // in a spec, with the path of the stop
    let bar = |scale: &str| {
        figure(
            "{}",
            "{}",
            &format!(r#""colorbar": {{"scale": {scale}}},"#),
            CURVE,
        )
    };
    let (path, _) = refused(&bar(
        r##"{"stops": [[0, "#000000"], [0.9, "#ffffff"]], "low": 0, "high": 1}"##,
    ));
    assert_eq!(path, "plots[0].colorbar.scale.stops[1]");
    let (path, _) = refused(&bar(
        r##"{"name": "viridis", "stops": [[0, "#000000"], [1, "#ffffff"]], "low": 0, "high": 1}"##,
    ));
    assert_eq!(path, "plots[0].colorbar.scale.stops");
    let (path, _) = refused(&bar(r#"{"low": 0, "high": 1}"#));
    assert_eq!(path, "plots[0].colorbar.scale.name");
}

#[test]
fn the_fields_of_a_colour_bar_are_checked() {
    let bar = |fields: &str| {
        figure(
            "{}",
            "{}",
            &format!(
                r#""colorbar": {{"scale": {{"name": "viridis", "low": 0, "high": 1}}, {fields}}},"#
            ),
            CURVE,
        )
    };
    let cases = [
        (
            r#""marks": [{"value": 2}]"#,
            "plots[0].colorbar.marks[0].value",
        ),
        (
            r#""marks": [{"value": 0.5, "color": "red"}]"#,
            "plots[0].colorbar.marks[0].color",
        ),
        (r#""loc": "upper_left""#, "plots[0].colorbar.loc"),
        (
            r#""placement": "inset", "loc": "best""#,
            "plots[0].colorbar.loc",
        ),
        (r#""length": 0"#, "plots[0].colorbar.length"),
        (r#""length": 1.5"#, "plots[0].colorbar.length"),
        (r#""thickness": -1"#, "plots[0].colorbar.thickness"),
    ];
    for (fields, path) in cases {
        assert_eq!(refused(&bar(fields)).0, path, "{fields}");
    }
    // one bar above a row
    let two = r#"{"contract": "figure/1", "width": 500, "height": 400,
        "grid": {"rows": 1, "columns": 2},
        "plots": [
          {"id": "a", "row": 0, "column": 0, "x": {}, "y": {}, "elements": [],
           "colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "top"}},
          {"id": "b", "row": 0, "column": 1, "x": {}, "y": {}, "elements": [],
           "colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "top"}}]}"#;
    let (path, message) = refused(two);
    assert_eq!(path, "plots[1].colorbar.placement");
    assert!(message.contains("plots[0].colorbar.placement"), "{message}");
}

#[test]
fn the_ticks_of_an_axis_are_checked() {
    let cases = [
        (
            r#"{"scale": "category", "categories": ["a"], "ticks": [0]}"#,
            "plots[0].x.ticks",
        ),
        (r#"{"tick_labels": ["a"]}"#, "plots[0].x.tick_labels"),
        (
            r#"{"ticks": [0, 1], "tick_labels": ["a"]}"#,
            "plots[0].x.tick_labels",
        ),
        (
            r#"{"scale": "log", "ticks": [0, 1]}"#,
            "plots[0].x.ticks[0]",
        ),
    ];
    for (x, path) in cases {
        assert_eq!(refused(&figure(x, "{}", "", CURVE)).0, path, "{x}");
    }
}

/// The tick labels of an axis of a drawing, the text of each.
fn tick_labels(doc: &Document, axis: &str) -> Vec<String> {
    doc.descendants()
        .filter(|n| n.attribute("data-axis") == Some(axis))
        .flat_map(|a| a.descendants().collect::<Vec<_>>())
        .filter(|n| {
            n.has_tag_name("g")
                && n.attribute("class")
                    .is_some_and(|c| c.contains("charton-tick-label"))
        })
        .flat_map(|g| g.children().filter(|t| t.has_tag_name("text")))
        .map(text)
        .collect()
}

#[test]
fn an_axis_takes_its_ticks_and_their_labels() {
    let x = r#"{"min": 0, "max": 144, "ticks": [0, 24, 48, 72, 96, 120, 144, 168]}"#;
    let svg = render_svg(&spec(&figure(x, "{}", "", CURVE)));
    let doc = Document::parse(&svg).unwrap();
    // the tick beyond the limits is not drawn
    assert_eq!(
        tick_labels(&doc, "x"),
        ["0", "24", "48", "72", "96", "120", "144"]
    );
    let x = r#"{"ticks": [0, 0.5, 1], "tick_labels": ["start", "half", "end"]}"#;
    let svg = render_svg(&spec(&figure(x, "{}", "", CURVE)));
    let doc = Document::parse(&svg).unwrap();
    assert_eq!(tick_labels(&doc, "x"), ["start", "half", "end"]);
}

#[test]
fn the_decades_of_a_log_axis_are_powers_of_ten() {
    let curve = r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0.000001, 0.1]}"#;
    let y = r#"{"scale": "log", "min": 0.000001, "max": 0.1}"#;
    let svg = render_svg(&spec(&figure("{}", y, "", curve)));
    let doc = Document::parse(&svg).unwrap();
    assert_eq!(
        tick_labels(&doc, "y"),
        [
            "10\u{2212}6",
            "10\u{2212}5",
            "10\u{2212}4",
            "10\u{2212}3",
            "10\u{2212}2",
            "10\u{2212}1"
        ]
    );
    // the exponent a superscript: smaller and raised, the measure of the layout
    let sup = doc
        .descendants()
        .find(|n| n.has_tag_name("tspan") && n.text() == Some("\u{2212}4"))
        .unwrap();
    assert!((num(sup, "font-size") - 0.7 * 11.0).abs() < 1e-9);
    assert!(num(sup, "dy") < 0.0);
}

#[test]
fn large_values_of_a_linear_axis_have_a_multiplier() {
    let curve = r#"{"id": "c", "kind": "curve", "x": [1e6, 9e6], "y": [0, 1]}"#;
    let svg = render_svg(&spec(&figure("{}", "{}", "", curve)));
    let doc = Document::parse(&svg).unwrap();
    let labels = tick_labels(&doc, "x");
    assert!(
        labels.iter().all(|l| l.parse::<f64>().unwrap() < 10.0),
        "{labels:?}"
    );
    let m = groups(&doc, "sbmlsim-axis-multiplier");
    assert_eq!(m.len(), 1);
    assert_eq!(text(m[0]), "\u{d7}106");
}

#[test]
fn a_tick_label_of_a_colour_bar_has_its_lines() {
    let bar = r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1},
        "ticks": [0, 1], "tick_labels": ["0\nfasted", "1\nfed"]},"#;
    let svg = render_svg(&spec(&figure("{}", "{}", bar, CURVE)));
    let doc = Document::parse(&svg).unwrap();
    let labels = groups(&doc, "sbmlsim-colorbar-tick-labels")[0];
    let lines: Vec<&str> = labels
        .descendants()
        .filter(|n| n.has_tag_name("tspan"))
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(lines, ["0", "fasted", "1", "fed"]);
}

/// The lines of the marks of the colour bars of a drawing.
fn marks(doc: &Document) -> Vec<(f64, f64, f64, f64, String)> {
    groups(doc, "sbmlsim-colorbar-marks")
        .into_iter()
        .flat_map(|g| g.children().filter(|n| n.has_tag_name("line")))
        .map(|l| {
            (
                num(l, "x1"),
                num(l, "y1"),
                num(l, "x2"),
                num(l, "y2"),
                l.attribute("stroke").unwrap().to_string(),
            )
        })
        .collect()
}

/// The rectangle of the bar of the first colour bar of a drawing: its outline.
fn bar_rect(doc: &Document) -> (f64, f64, f64, f64) {
    let bar = groups(doc, "sbmlsim-colorbar")[0];
    let outline = bar
        .children()
        .find(|n| n.has_tag_name("rect") && n.attribute("fill") == Some("none"))
        .unwrap();
    (
        num(outline, "x"),
        num(outline, "y"),
        num(outline, "width"),
        num(outline, "height"),
    )
}

#[test]
fn a_mark_is_a_line_across_the_bar_at_its_value() {
    let bar = r##""colorbar": {"scale": {"name": "viridis", "low": 0.1, "high": 10, "log": true},
        "marks": [{"value": 1}, {"value": 10, "color": "#ff0000", "width": 4}]},"##;
    let svg = render_svg(&spec(&figure("{}", "{}", bar, CURVE)));
    let doc = Document::parse(&svg).unwrap();
    let (x, y, w, h) = bar_rect(&doc);
    let m = marks(&doc);
    assert_eq!(m.len(), 2);
    // across the bar, the middle of a log scale from 0.1 to 10 at 1
    assert_eq!((m[0].0, m[0].2), (x, x + w));
    assert!((m[0].1 - (y + h / 2.0)).abs() < 0.01, "{m:?}");
    assert_eq!(m[0].4, "#000000");
    assert!((m[1].1 - y).abs() < 0.01);
    assert_eq!(m[1].4, "#ff0000");
}

#[test]
fn a_bar_above_a_row_spans_its_length_of_the_panels_centred() {
    let json = r##"{"contract": "figure/1", "width": 800, "height": 400,
        "grid": {"rows": 1, "columns": 2},
        "plots": [
          {"id": "a", "row": 0, "column": 0, "x": {}, "y": {}, "elements": [],
           "colorbar": {"scale": {"stops": [[0, "#000000"], [1, "#ffffff"]], "low": 0, "high": 1},
                        "placement": "top", "length": 0.4, "label": "scan"}},
          {"id": "b", "row": 0, "column": 1, "x": {}, "y": {}, "elements": []}]}"##;
    let f = spec(json);
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let g = geometry(&f);
    let (a, b) = (&g.plots[0].rect, &g.plots[1].rect);
    let (x, y, w, h) = bar_rect(&doc);
    let span = b.x + b.width - a.x;
    assert!((w - 0.4 * span).abs() < 0.01, "{w} of {span}");
    assert!((x + w / 2.0 - (a.x + span / 2.0)).abs() < 0.01);
    // above the panels, horizontal, its gradient from the left
    assert!(y + h < a.y);
    assert!(w > h);
    let gradient = doc
        .descendants()
        .find(|n| n.has_tag_name("linearGradient"))
        .unwrap();
    assert_eq!(gradient.attribute("x2"), Some("1"));
    assert_eq!(gradient.attribute("y2"), Some("0"));
    // a figure without the bar has its panels higher up
    let plain = spec(&json.replace(r#""placement": "top""#, r#""placement": "inset""#));
    assert!(geometry(&plain).plots[0].rect.y < a.y);
}

#[test]
fn a_legend_at_best_avoids_an_inset_bar() {
    // a curve along the bottom: the upper right, the first location of best, is free
    let low = r#"{"id": "c", "kind": "curve", "label": "c", "x": [0, 1], "y": [0, 0]}"#;
    let y = r#"{"min": 0, "max": 1}"#;
    let legend_x = |extra: &str| {
        let f = spec(&figure("{}", y, extra, low));
        geometry(&f).legend_entries[0].rect.x
    };
    let free = legend_x("");
    let inset = legend_x(
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "inset"},"#,
    );
    assert!(inset < free, "{inset} {free}");
    // the inset bar in the upper right of the panel
    let f = spec(&figure(
        "{}",
        y,
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "inset"},"#,
        low,
    ));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let panel = &geometry(&f).plots[0].rect;
    let (x, y, w, _) = bar_rect(&doc);
    // in the pixels of the figure: the bar is drawn in the canvas of its plot
    let canvas = doc
        .descendants()
        .find(|n| n.attribute("data-plot") == Some("p") && n.has_tag_name("svg"))
        .unwrap();
    let (x, y) = (x + num(canvas, "x"), y + num(canvas, "y"));
    assert!(x > panel.x + panel.width / 2.0 - 1.0 - w / 2.0 && x + w < panel.x + panel.width);
    assert!(
        y > panel.y && y < panel.y + panel.height / 3.0,
        "{y} {panel:?}"
    );
}

#[test]
fn half_filled_markers_draw_both_halves() {
    let curve = r##"{"id": "m", "kind": "curve", "x": [0.5], "y": [0.5],
        "style": {"color": "#00aa00", "line": {"type": "none"},
                  "marker": {"type": "diamond", "size": 10, "fill_style": "left",
                             "fill_alt": "#ffffff"}}}"##;
    let svg = render_svg(&spec(&figure("{}", "{}", "", curve)));
    let doc = Document::parse(&svg).unwrap();
    let group = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("m"))
        .unwrap();
    let fills: Vec<&str> = used_markers(&doc, group)
        .iter()
        .filter(|m| m.shape.has_tag_name("path"))
        .filter_map(|m| m.shape.attribute("fill"))
        .collect();
    assert_eq!(fills, ["#00aa00", "#ffffff"]);
    assert!(
        refused(&figure("{}", "{}", "", &curve.replace("#ffffff", "white")))
            .0
            .ends_with("marker.fill_alt")
    );
}

#[test]
fn a_diamond_is_as_wide_as_high() {
    let marker = |kind: &str| {
        let curve = format!(
            r#"{{"id": "m", "kind": "curve", "x": [0.5], "y": [0.5],
                "style": {{"line": {{"type": "none"}}, "marker": {{"type": "{kind}", "size": 10}}}}}}"#
        );
        let svg = render_svg(&spec(&figure("{}", "{}", "", &curve)));
        let doc = Document::parse(&svg).unwrap();
        let group = doc
            .descendants()
            .find(|n| n.attribute("data-element") == Some("m"))
            .unwrap();
        let d = used_markers(&doc, group)[0]
            .shape
            .attribute("d")
            .unwrap()
            .to_string();
        let v: Vec<f64> = d
            .replace(['M', 'L', 'Z'], " ")
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        let (xs, ys): (Vec<f64>, Vec<f64>) = v.chunks(2).map(|p| (p[0], p[1])).unzip();
        let extent = |s: &[f64]| {
            s.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - s.iter().copied().fold(f64::INFINITY, f64::min)
        };
        (extent(&xs), extent(&ys))
    };
    // matplotlib's `D`, a square of the side of the size on its corner, and its `d`
    let (w, h) = marker("diamond");
    assert!(
        (w - h).abs() < 0.02 && (h - 10.0 * 2f64.sqrt()).abs() < 0.02,
        "{w} {h}"
    );
    let (w, h) = marker("thin_diamond");
    assert!((w / h - 0.6).abs() < 0.01, "{w} {h}");
}

#[test]
fn a_box_and_error_bars_take_their_widths() {
    let elements = r#"{"id": "b", "kind": "box", "positions": [0, 1], "width": 0.2,
          "stats": [{"low_whisker": 0, "q1": 1, "median": 2, "q3": 3, "high_whisker": 4},
                    {"low_whisker": 0, "q1": 1, "median": 2, "q3": 3, "high_whisker": 4}]},
        {"id": "e", "kind": "curve", "x": [0.5], "y": [2], "yerr": [1], "error_width": 3}"#;
    let x = r#"{"min": -0.5, "max": 1.5}"#;
    let f = spec(&figure(x, "{}", "", elements));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let panel = &geometry(&f).plots[0].rect;
    let ring = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("b"))
        .unwrap()
        .descendants()
        .find(|n| n.has_tag_name("path"))
        .unwrap();
    let v: Vec<f64> = ring
        .attribute("d")
        .unwrap()
        .split('Z')
        .next()
        .unwrap()
        .replace(['M', 'L'], " ")
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let xs: Vec<f64> = v.chunks(2).map(|p| p[0]).collect();
    let width = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - xs.iter().copied().fold(f64::INFINITY, f64::min);
    // 0.2 of an axis of 2 units
    assert!((width - 0.1 * panel.width).abs() < 0.02, "{width}");
    let bar = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("e"))
        .unwrap()
        .descendants()
        .find(|n| n.has_tag_name("path") && n.attribute("fill") == Some("none"))
        .unwrap();
    assert_eq!(bar.attribute("stroke-width"), Some("3"));
}

#[test]
fn a_legend_has_its_title() {
    let svg = render_svg(&spec(&figure(
        "{}",
        "{}",
        r#""legend_title": "3 day","#,
        CURVE,
    )));
    let doc = Document::parse(&svg).unwrap();
    let title = doc
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-legend-title"))
        .unwrap();
    assert_eq!(text(title), "3 day");
    let entry = doc
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-legend-entry"))
        .unwrap();
    let label = entry
        .descendants()
        .find(|n| n.has_tag_name("text"))
        .unwrap();
    assert!(num(label, "y") > num(title, "y") + 11.0);
}

#[test]
fn the_legend_below_the_grid_has_its_columns() {
    let f = spec(&spec_text("v031_legend"));
    let g = geometry(&f);
    let entries: Vec<_> = g
        .legend_entries
        .iter()
        .filter(|e| e.owner == "figure")
        .collect();
    assert_eq!(entries.len(), 6);
    // three columns of two rows, filled top down
    let (x0, x1) = (entries[0].rect.x, entries[2].rect.x);
    assert_eq!(entries[1].rect.x, x0);
    assert!(x1 > x0);
    assert!(entries[1].rect.y > entries[0].rect.y);
    assert_eq!(entries[2].rect.y, entries[0].rect.y);
    // below the panels, the figure grown by it
    let bottom = g
        .plots
        .iter()
        .map(|p| p.rect.y + p.rect.height)
        .fold(0.0, f64::max);
    assert!(entries[0].rect.y > bottom);
    assert!(g.height > 300.0);
}

#[test]
fn a_group_of_a_band_and_a_curve_draws_both_in_its_swatch() {
    let elements = r##"{"id": "band", "kind": "band", "legend": "m", "x": [0, 1], "low": [0, 0],
          "high": [1, 1], "style": {"color": "#00aa00"}},
        {"id": "line", "kind": "curve", "legend": "m", "label": "model", "x": [0, 1],
          "y": [0.5, 0.5], "style": {"color": "#00aa00"}}"##;
    let svg = render_svg(&spec(&figure("{}", "{}", "", elements)));
    let doc = Document::parse(&svg).unwrap();
    let swatch = groups(&doc, "sbmlsim-legend-swatch")[0];
    let kinds: Vec<&str> = swatch
        .children()
        .filter(|n| n.is_element())
        .map(|n| n.tag_name().name())
        .collect();
    assert_eq!(kinds, ["rect", "path"]);
}

#[test]
fn the_plot_titles_have_their_size() {
    let svg = render_svg(&spec(
        &figure("{}", "{}", r#""title": "panel","#, CURVE).replace(
            r#""width": 500,"#,
            r#""width": 500, "fonts": {"plot_title": 17},"#,
        ),
    ));
    let doc = Document::parse(&svg).unwrap();
    let title = doc
        .descendants()
        .find(|n| n.has_tag_name("text") && n.text() == Some("panel"))
        .unwrap();
    assert_eq!(num(title, "font-size"), 17.0);
}

#[test]
fn golden_v031_text() {
    assert_golden("v031_text", &render_svg(&spec(&spec_text("v031_text"))));
}

#[test]
fn a_legend_inside_fills_its_columns() {
    let f = spec(&spec_text("v031_text"));
    let g = geometry(&f);
    let panel = &g.plots[0].rect;
    let entries: Vec<_> = g
        .legend_entries
        .iter()
        .filter(|e| e.owner == "units")
        .collect();
    assert_eq!(entries.len(), 6);
    // two columns of three rows, filled top down, inside the panel
    assert_eq!(entries[1].rect.x, entries[0].rect.x);
    assert_eq!(entries[2].rect.x, entries[0].rect.x);
    assert!(entries[3].rect.x > entries[0].rect.x + entries[0].rect.width);
    assert_eq!(entries[3].rect.y, entries[0].rect.y);
    for e in &entries {
        assert!(e.rect.x >= panel.x && e.rect.x + e.rect.width <= panel.x + panel.width);
    }
    // a legend of one column is taller than of two
    let one = spec(&spec_text("v031_text").replace(r#""legend_columns": 2,"#, ""));
    let one = geometry(&one);
    let last = |g: &sbmlsim_plot::Geometry| {
        g.legend_entries
            .iter()
            .filter(|e| e.owner == "units")
            .map(|e| e.rect.y)
            .fold(0.0, f64::max)
    };
    assert!(last(&one) > last(&g));
    let (path, _) = refused(&figure("{}", "{}", r#""legend_columns": 0,"#, CURVE));
    assert_eq!(path, "plots[0].legend_columns");
}

#[test]
fn a_monospace_text_has_its_family_and_measure() {
    let element = r#"{"id": "t", "kind": "text", "x": 0.5, "y": 0.5, "text": "mg⋅l  1.0",
        "family": "monospace"}"#;
    let svg = render_svg(&spec(&figure("{}", "{}", "", element)));
    let doc = Document::parse(&svg).unwrap();
    let node = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("t"))
        .unwrap()
        .descendants()
        .find(|n| n.has_tag_name("text"))
        .unwrap();
    let family = node.attribute("font-family").unwrap();
    assert!(family.starts_with("ui-monospace") && family.ends_with("monospace"));
    // the dot operator as the middle dot of every monospace font
    // nine characters of 0.6 of the size
    assert_eq!(text(node), "mg\u{b7}l \u{a0}1.0");
    assert_eq!(
        text_width_family(
            "mg\u{22c5}l  1.0",
            10.0,
            FontWeight::Normal,
            FontFamily::Monospace
        ),
        9.0 * 0.6 * 10.0
    );
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        &element.replace("monospace", "serif"),
    ));
    assert_eq!(path, "plots[0].elements[0].family");
}

#[test]
fn the_signs_of_units_are_measured() {
    // the dot operator of Noto Sans Math, which the PNG draws it with, and the signs of Inter
    for sign in [
        "\u{22c5}", "\u{b7}", "\u{d7}", "\u{2212}", "\u{b5}", "\u{b0}", "\u{b1}",
    ]
    .into_iter()
    .chain([
        "\u{2264}", "\u{2265}", "\u{b9}", "\u{b2}", "\u{b3}", "\u{2074}", "\u{207b}",
    ]) {
        let w = text_width(sign, 10.0);
        // a sign outside of the table takes the advance of the digit zero
        assert_ne!(w, text_width("0", 10.0), "{sign}");
        assert!(w > 0.0, "{sign}");
    }
}

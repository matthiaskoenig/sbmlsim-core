//! The fields of `figure/1` since 0.30.0: a hidden axis line, error-bar caps, the aspect of a
//! panel, a heatmap over the edges of its cells, the size and the weight of a text element and
//! its spaces, the colour maps of matplotlib and a grid of thin cells which fits its figure.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{
    COLOR_SCALES, ColorScale, ColorScaleName, Figure, geometry, parse, pick, render_svg,
};
use support::{assert_golden, spec_text};

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

/// The `d` of the paths of the error bars of an element: its paths without a fill.
fn error_paths(doc: &Document, id: &str) -> Vec<String> {
    let group = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some(id))
        .unwrap();
    group
        .descendants()
        .filter(|n| n.has_tag_name("path") && n.attribute("fill") == Some("none"))
        .map(|n| n.attribute("d").unwrap().to_string())
        .collect()
}

/// The segments of a path of `M x y L x y` subpaths.
fn segments(d: &str) -> Vec<[f64; 4]> {
    d.split('M')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let v: Vec<f64> = s
                .replace('L', " ")
                .split_whitespace()
                .map(|n| n.parse().unwrap())
                .collect();
            [v[0], v[1], v[2], v[3]]
        })
        .collect()
}

#[test]
fn golden_v030() {
    assert_golden("v030", &render_svg(&spec(&spec_text("v030"))));
}

#[test]
fn an_axis_line_may_be_hidden() {
    let curve = r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1], "axis": "y"},
        {"id": "d", "kind": "curve", "x": [0, 1], "y": [1, 0], "axis": "y2"}"#;
    let shown = render_svg(&spec(&figure("{}", "{}", r#""y2": {},"#, curve)));
    let doc = Document::parse(&shown).unwrap();
    assert_eq!(groups(&doc, "charton-axis-line").len(), 3);
    let hidden = render_svg(&spec(&figure(
        r#"{"line_visible": false}"#,
        "{}",
        r#""y2": {"line_visible": false},"#,
        curve,
    )));
    let doc = Document::parse(&hidden).unwrap();
    // the line of y is left, its ticks and the ticks of x are kept
    let lines = groups(&doc, "charton-axis-line");
    assert_eq!(lines.len(), 1);
    let axis = lines[0].parent().unwrap();
    assert_eq!(axis.attribute("data-axis"), Some("y"));
    for name in ["x", "y2"] {
        let axis = doc
            .descendants()
            .find(|n| n.attribute("data-axis") == Some(name))
            .unwrap();
        assert!(
            axis.descendants()
                .any(|n| n.attribute("class") == Some("charton-tick")),
            "{name}"
        );
    }
}

#[test]
fn error_bars_have_caps_of_their_length() {
    let element = |cap: &str| {
        format!(
            r#"{{"id": "c", "kind": "curve", "x": [1, 2, 3], "y": [2, 3, 2],
                "xerr": [0.1, 0.1, 0.1], "yerr": [0.5, 0.2, 0], {cap}
                "style": {{"line": {{"type": "none"}}, "marker": {{"type": "circle"}}}}}},
               {{"id": "b", "kind": "bar", "x": [1, 2], "height": [1, 2], "err": [0.3, 0.3],
                 {cap} "horizontal": true}}"#
        )
    };
    let plain = render_svg(&spec(&figure("{}", "{}", "", &element(""))));
    let doc = Document::parse(&plain).unwrap();
    // an error of x and one of y per point, a segment each, none for the errors of 0
    let paths = error_paths(&doc, "c");
    let count: usize = paths.iter().map(|d| segments(d).len()).sum();
    assert_eq!(count, 6);
    let capped = render_svg(&spec(&figure(
        "{}",
        "{}",
        "",
        &element(r#""error_cap": 8,"#),
    )));
    let doc = Document::parse(&capped).unwrap();
    let paths = error_paths(&doc, "c");
    let (x_bars, y_bars) = (segments(&paths[0]), segments(&paths[1]));
    // the errors of x: a bar and two caps per point; the errors of y the same but the error of
    // 0, which has a bar and no caps
    assert_eq!(x_bars.len(), 9);
    assert_eq!(y_bars.len(), 7);
    // a cap across an error of y is horizontal and 8 pixels long, at the end of its bar
    let [x1, y1, x2, y2] = y_bars[1];
    assert!((y1 - y2).abs() < 0.011 && ((x2 - x1).abs() - 8.0).abs() < 0.011);
    let bar = y_bars[0];
    assert!((y1 - bar[1]).abs() < 0.011, "{y_bars:?}");
    // a cap across an error of x is vertical
    let [x1, y1, x2, y2] = x_bars[1];
    assert!((x1 - x2).abs() < 0.011 && ((y2 - y1).abs() - 8.0).abs() < 0.011);
    // the caps of horizontal bars are vertical
    let bars = segments(&error_paths(&doc, "b")[0]);
    assert_eq!(bars.len(), 6);
    let [x1, y1, x2, y2] = bars[1];
    assert!((x1 - x2).abs() < 0.011 && ((y2 - y1).abs() - 8.0).abs() < 0.011);
}

/// The pixels of one unit of y over the pixels of one unit of x of a plot of a geometry.
fn aspect_of(g: &sbmlsim_plot::Geometry, k: usize) -> f64 {
    let p = &g.plots[k];
    let per_x = p.rect.width / (p.x.high - p.x.low);
    let per_y = p.rect.height / (p.y.high - p.y.low);
    per_y / per_x
}

#[test]
fn an_aspect_widens_one_axis() {
    let hexagon = r#"{"id": "h", "kind": "polygon", "paths": [[[1, 0], [0.5, 0.866],
        [-0.5, 0.866], [-1, 0], [-0.5, -0.866], [0.5, -0.866]]]}"#;
    let free = geometry(&spec(&figure("{}", "{}", "", hexagon)));
    let equal = geometry(&spec(&figure("{}", "{}", r#""aspect": "equal","#, hexagon)));
    assert!((aspect_of(&free, 0) - 1.0).abs() > 0.05);
    assert!((aspect_of(&equal, 0) - 1.0).abs() < 1e-9);
    // the panel keeps its size, the limits of x are widened about their middle
    assert_eq!(free.plots[0].rect, equal.plots[0].rect);
    let (x, y) = (&equal.plots[0].x, &equal.plots[0].y);
    assert!(x.high - x.low > free.plots[0].x.high - free.plots[0].x.low);
    assert!((x.low + x.high).abs() < 1e-9);
    assert_eq!((y.low, y.high), (free.plots[0].y.low, free.plots[0].y.high));
    // a number is the length of a unit of y in units of x
    let two = geometry(&spec(&figure("{}", "{}", r#""aspect": 2,"#, hexagon)));
    assert!((aspect_of(&two, 0) - 2.0).abs() < 1e-9);
    // given ends are widened as well
    let ends = r#"{"min": -1, "max": 1}"#;
    let fixed = geometry(&spec(&figure(ends, ends, r#""aspect": "equal","#, hexagon)));
    assert!((aspect_of(&fixed, 0) - 1.0).abs() < 1e-9);
}

#[test]
fn an_aspect_holds_on_the_panel_the_layout_ends_with() {
    // the plot below spans the column of a plot with a legend outside, which widens the span
    // after the plots were prepared
    let json = r#"{"contract": "figure/1", "width": 600, "height": 500,
        "grid": {"rows": 2, "columns": 2}, "legend": {"position": "outside"},
        "plots": [
            {"id": "a", "row": 0, "column": 0, "x": {}, "y": {},
             "elements": [{"id": "c", "kind": "curve", "label": "a long legend label",
                           "x": [0, 1], "y": [0, 1]}]},
            {"id": "b", "row": 1, "column": 0, "column_span": 2, "x": {}, "y": {},
             "aspect": "equal", "legend": false,
             "elements": [{"id": "d", "kind": "curve", "x": [0, 1], "y": [0, 1]}]}]}"#;
    let g = geometry(&spec(json));
    assert!((aspect_of(&g, 1) - 1.0).abs() < 1e-9, "{:?}", g.plots[1]);
}

#[test]
fn a_heatmap_over_edges_keeps_a_numeric_axis() {
    let cells = r#"{"id": "h", "kind": "heatmap", "x_edges": [0, 1, 3, 10],
        "y_edges": [0, 0.5, 1], "values": [[1, 2, 3], [4, 5, 6]],
        "scale": {"name": "blues", "low": 1, "high": 6}}"#;
    let f = spec(&figure("{}", "{}", "", cells));
    let g = geometry(&f);
    let p = &g.plots[0];
    // the axes stick to the outer edges, as matplotlib's pcolormesh
    assert_eq!(p.x.kind, sbmlsim_plot::AxisScale::Linear);
    assert_eq!((p.x.low, p.x.high), (0.0, 10.0));
    assert_eq!((p.y.low, p.y.high), (0.0, 1.0));
    // the cell of row 1 and column 2 spans 3..10 and 0.5..1
    let (px, py) = (
        p.rect.x + p.rect.width * 0.65,
        p.rect.y + p.rect.height * 0.25,
    );
    let hit = pick(&f, px, py, 0.0).expect("a cell");
    assert_eq!(hit.index, 5);
    assert_eq!(hit.values["value"], 6.0);
    assert_eq!((hit.values["x_low"], hit.values["x_high"]), (3.0, 10.0));
    assert_eq!((hit.values["y_low"], hit.values["y_high"]), (0.5, 1.0));
    assert_eq!(hit.label, None);
    // the cells are drawn in their colours over their edges, 6 of them
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let group = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("h"))
        .unwrap();
    let rings: usize = group
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .map(|n| n.attribute("d").unwrap().matches('M').count())
        .sum();
    assert_eq!(rings, 6);
    // edges along one axis and names along the other: the names are the categories of y
    let mixed = r#"{"id": "h", "kind": "heatmap", "x_edges": [0, 1, 3, 10],
        "rows": ["a", "b"], "values": [[1, 2, 3], [4, 5, 6]],
        "scale": {"name": "viridis", "low": 1, "high": 6}}"#;
    let f = spec(&figure("{}", "{}", "", mixed));
    let g = geometry(&f);
    assert_eq!(g.plots[0].x.kind, sbmlsim_plot::AxisScale::Linear);
    assert_eq!(g.plots[0].y.kind, sbmlsim_plot::AxisScale::Category);
    let p = &g.plots[0];
    let hit = pick(
        &f,
        p.rect.x + p.rect.width * 0.05,
        p.rect.y + p.rect.height * 0.25,
        0.0,
    )
    .expect("a cell");
    assert_eq!(hit.label.as_deref(), Some("b"));
    assert!(!hit.values.contains_key("y_low"));
}

#[test]
fn a_text_has_its_size_weight_and_spaces() {
    let texts = r#"{"id": "t", "kind": "text", "x": 0, "y": 0, "text": "A  Circuit",
        "size": 18, "weight": "bold"},
        {"id": "u", "kind": "text", "x": 1, "y": 1, "text": "plain"}"#;
    let svg = render_svg(&spec(&figure("{}", "{}", "", texts)));
    let doc = Document::parse(&svg).unwrap();
    let text = |id: &str| {
        doc.descendants()
            .find(|n| n.attribute("data-element") == Some(id))
            .and_then(|g| g.descendants().find(|n| n.has_tag_name("text")))
            .unwrap()
    };
    let t = text("t");
    assert_eq!(t.attribute("font-size"), Some("18"));
    assert_eq!(t.attribute("font-weight"), Some("bold"));
    // the second space of the run is a no-break space, which SVG does not collapse
    assert_eq!(t.text(), Some("A \u{a0}Circuit"));
    let u = text("u");
    assert_eq!(u.attribute("font-size"), Some("11"));
    assert_eq!(u.attribute("font-weight"), None);
}

/// A scale of a name from 0 to 1.
fn unit_scale(name: ColorScaleName) -> ColorScale {
    ColorScale {
        name: Some(name),
        stops: None,
        low: 0.0,
        high: 1.0,
        center: None,
        log: false,
        start: 0.0,
        end: 1.0,
    }
}

#[test]
fn the_maps_of_matplotlib() {
    // the ends of matplotlib's colormaps[name](0) and (255)
    for (name, low, high) in [
        ("blues", "#f7fbff", "#08306b"),
        ("reds", "#fff5f0", "#67000d"),
        ("oranges", "#fff5eb", "#7f2704"),
        ("purples", "#fcfbfd", "#3f007d"),
        ("greens", "#f7fcf5", "#00441b"),
        ("purd", "#f7f4f9", "#67001f"),
        ("ylorrd", "#ffffcc", "#800026"),
        ("ylgnbu", "#ffffd9", "#081d58"),
        ("blues_r", "#08306b", "#f7fbff"),
        ("greys", "#ffffff", "#000000"),
        ("viridis_r", "#fde725", "#440154"),
    ] {
        let s = unit_scale(ColorScaleName::from_name(name).unwrap());
        assert_eq!(
            sbmlsim_plot::color_scale(&s, &[0.0, 1.0]),
            [low, high],
            "{name}"
        );
    }
    // every map and its reversal, the names of 0.29 among them
    assert_eq!(COLOR_SCALES.len(), 108);
    for name in [
        "viridis", "magma", "rdbu", "coolwarm", "greys", "purd_r", "seismic",
    ] {
        assert!(COLOR_SCALES.contains(&name), "{name}");
    }
    // a colour bar and a heatmap read the names of the spec
    let cells = r#"{"id": "h", "kind": "heatmap", "rows": ["a"], "columns": ["b"],
        "values": [[1]], "scale": {"name": "ylorrd_r", "low": 0, "high": 2}}"#;
    let bar = r#""colorbar": {"scale": {"name": "purd", "low": 0, "high": 1}},"#;
    assert!(parse(&figure("{}", "{}", bar, cells)).is_ok());
}

#[test]
fn a_name_of_matplotlib_in_its_case_is_refused_with_a_hint() {
    let cells = r#"{"id": "h", "kind": "heatmap", "rows": ["a"], "columns": ["b"],
        "values": [[1]], "scale": {"name": "Blues", "low": 0, "high": 2}}"#;
    let e = parse(&figure("{}", "{}", "", cells)).unwrap_err();
    assert_eq!(e.path, "plots[0].elements[0].scale.name");
    assert!(e.message.contains("did you mean `blues`"), "{e}");
    let bar = r#""colorbar": {"scale": {"name": "PuRd", "low": 0, "high": 1}},"#;
    let e = parse(&figure("{}", "{}", bar, "")).unwrap_err();
    assert_eq!(e.path, "plots[0].colorbar.scale.name");
    assert!(e.message.contains("did you mean `purd`"), "{e}");
}

#[test]
fn a_wide_grid_of_large_spans_fits_its_figure() {
    // 32 columns of a space of 1.2 and 4 plots of 8 columns each: every plot lies in the
    // figure, which keeps its width
    let plots: Vec<String> = (0..4)
        .map(|k| {
            format!(
                r#"{{"id": "p{k}", "row": 0, "column": {c}, "column_span": 8,
                    "x": {{"label": "x"}}, "y": {{"label": "y"}}, "elements": [{{"id": "c{k}",
                    "kind": "curve", "x": [0, 1], "y": [0, 1]}}]}}"#,
                c = 8 * k
            )
        })
        .collect();
    let json = format!(
        r#"{{"contract": "figure/1", "width": 1225, "height": 300,
            "grid": {{"rows": 1, "columns": 32, "wspace": 1.2}}, "plots": [{}]}}"#,
        plots.join(",")
    );
    let g = geometry(&spec(&json));
    assert_eq!(g.width, 1225.0);
    let last = &g.plots[3].rect;
    assert!(last.x + last.width < 1225.0, "{last:?}");
    // a grid whose plots cannot have their least panel grows the figure, nothing is cut off
    let single: Vec<String> = (0..64)
        .map(|k| {
            format!(
                r#"{{"id": "q{k}", "row": 0, "column": {k}, "x": {{}}, "y": {{}},
                    "elements": []}}"#
            )
        })
        .collect();
    let json = format!(
        r#"{{"contract": "figure/1", "width": 400, "height": 200,
            "grid": {{"rows": 1, "columns": 64}}, "plots": [{}]}}"#,
        single.join(",")
    );
    let g = geometry(&spec(&json));
    let last = &g.plots[63].rect;
    assert!(g.width > 400.0);
    assert!(last.x + last.width <= g.width, "{last:?} in {}", g.width);
    assert!(last.width >= 20.0 - 1e-9);
}

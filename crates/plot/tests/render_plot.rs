//! The rendering of one plot: axes, curves, bands, areas, rules and text.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{parse, render_svg, text_width};
use support::{assert_golden, spec_text};

/// Renders a figure of JSON text.
fn render(json: &str) -> String {
    render_svg(&parse(json).unwrap_or_else(|e| panic!("{e}")))
}

/// A figure of one plot with the given axes and elements, as JSON text.
fn figure(x: &str, y: &str, elements: &str) -> String {
    format!(
        r#"{{"contract": "figure/1", "width": 500, "height": 400,
            "grid": {{"rows": 1, "columns": 1}},
            "plots": [{{"id": "p", "row": 0, "column": 0, "x": {x}, "y": {y},
                        "elements": [{elements}]}}]}}"#
    )
}

/// The groups of an element.
fn element_groups<'a>(doc: &'a Document<'a>, id: &str) -> Vec<Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| n.has_tag_name("g") && n.attribute("data-element") == Some(id))
        .collect()
}

/// The one group of an element.
fn element_group<'a>(doc: &'a Document<'a>, id: &str) -> Node<'a, 'a> {
    let groups = element_groups(doc, id);
    assert_eq!(groups.len(), 1, "groups of element {id}");
    groups[0]
}

/// Whether a node is inside a group of a class.
fn inside(node: Node, class: &str) -> bool {
    node.ancestors().any(|a| {
        a.attribute("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == class))
    })
}

/// The tick labels of the axis `x` or `y` as numbers.
fn tick_values(doc: &Document, axis: &str) -> Vec<f64> {
    doc.descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter(|n| inside(*n, "charton-tick-label"))
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute("data-axis") == Some(axis))
        })
        .map(|n| {
            let t = n.text().unwrap_or_default();
            t.parse::<f64>()
                .unwrap_or_else(|e| panic!("tick label `{t}`: {e}"))
        })
        .collect()
}

/// Every number of every attribute of the drawing which carries coordinates.
fn coordinates(svg: &str) -> Vec<String> {
    let doc = Document::parse(svg).unwrap();
    let mut out = Vec::new();
    for n in doc.descendants() {
        for a in n.attributes() {
            if matches!(
                a.name(),
                "x" | "y"
                    | "x1"
                    | "x2"
                    | "y1"
                    | "y2"
                    | "cx"
                    | "cy"
                    | "r"
                    | "d"
                    | "points"
                    | "width"
                    | "height"
                    | "transform"
            ) {
                out.push(a.value().to_string());
            }
        }
    }
    out
}

const LINEAR: &str = r#"{"label": "time", "unit": "hr"}"#;

#[test]
fn every_element_is_a_group() {
    let json = figure(
        LINEAR,
        r#"{"label": "c"}"#,
        r#"{"id": "c", "kind": "curve", "label": "c", "x": [0, 1, 2], "y": [1, 2, 3],
            "xerr": [0.1, 0.1, 0.1], "yerr": [0.2, 0.2, 0.2],
            "style": {"marker": {"type": "circle"}}},
           {"id": "b", "kind": "band", "x": [0, 1, 2], "low": [0, 1, 2], "high": [2, 3, 4],
            "median": [1, 2, 3]},
           {"id": "a", "kind": "area", "x": [0, 1, 2], "from": [0, 0, 0], "to": [1, 1, 1]},
           {"id": "h", "kind": "rule", "orientation": "horizontal", "value": 2},
           {"id": "v", "kind": "rule", "orientation": "vertical", "value": 1},
           {"id": "t", "kind": "text", "x": 1, "y": 2, "text": "note"}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    for id in ["c", "b", "a", "h", "v", "t"] {
        let g = element_group(&doc, id);
        assert!(
            g.descendants().any(|n| n.is_element() && n != g),
            "element {id} draws nothing"
        );
    }
    // the fixture of every kind: one group per element of every kind, and the
    // grid, whose elements of a right y axis are written once
    for name in ["every_kind", "grid"] {
        let every = parse(&spec_text(name)).unwrap();
        let svg = render_svg(&every);
        let doc = Document::parse(&svg).unwrap();
        for plot in &every.plots {
            for e in &plot.elements {
                element_group(&doc, &e.id);
            }
        }
    }
}

#[test]
fn legend_group_attribute() {
    let json = figure(
        LINEAR,
        LINEAR,
        r#"{"id": "labelled", "kind": "curve", "label": "dose 1", "x": [0, 1], "y": [0, 1]},
           {"id": "grouped", "kind": "curve", "label": "dose 2", "legend": "doses",
            "x": [0, 1], "y": [1, 0]},
           {"id": "plain", "kind": "curve", "x": [0, 1], "y": [0.5, 0.5]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    assert_eq!(
        element_group(&doc, "labelled").attribute("data-legend"),
        Some("labelled")
    );
    assert_eq!(
        element_group(&doc, "grouped").attribute("data-legend"),
        Some("doses")
    );
    assert_eq!(element_group(&doc, "plain").attribute("data-legend"), None);
}

#[test]
fn nan_splits_a_curve() {
    let json = figure(
        LINEAR,
        LINEAR,
        r#"{"id": "c", "kind": "curve", "x": [0, 1, 2, 3, 4], "y": [1, 2, null, 3, 4]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let g = element_group(&doc, "c");
    let paths: Vec<&str> = g
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .map(|n| n.attribute("d").unwrap())
        .collect();
    assert_eq!(paths.len(), 2, "{paths:?}");
    for d in paths {
        assert_eq!(d.matches('M').count(), 1, "{d}");
        assert_eq!(d.matches('L').count(), 1, "{d}");
    }
}

#[test]
fn log_axis_drops_nonpositive() {
    let json = figure(
        LINEAR,
        r#"{"scale": "log"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1, 2, 3], "y": [0, -1, 1, 10],
            "style": {"marker": {"type": "circle"}}}"#,
    );
    let svg = render(&json);
    for c in coordinates(&svg) {
        let lower = c.to_lowercase();
        assert!(
            !lower.contains("nan") && !lower.contains("inf"),
            "coordinate `{c}`"
        );
    }
    let doc = Document::parse(&svg).unwrap();
    let g = element_group(&doc, "c");
    let markers = g.descendants().filter(|n| n.has_tag_name("circle")).count();
    assert_eq!(markers, 2);
    // the line runs between the two drawn points only
    let paths: Vec<&str> = g
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .map(|n| n.attribute("d").unwrap())
        .collect();
    assert_eq!(paths.len(), 1, "{paths:?}");
}

#[test]
fn log_axis_max_decades() {
    let ys: Vec<String> = (-12..=2).map(|k| format!("1e{k}")).collect();
    let xs: Vec<String> = (0..ys.len()).map(|i| i.to_string()).collect();
    let json = figure(
        LINEAR,
        r#"{"scale": "log", "max_decades": 6}"#,
        &format!(
            r#"{{"id": "c", "kind": "curve", "x": [{}], "y": [{}]}}"#,
            xs.join(","),
            ys.join(",")
        ),
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let ticks = tick_values(&doc, "y");
    assert!(!ticks.is_empty());
    let lowest = ticks.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        lowest >= 1e-4 * (1.0 - 1e-12),
        "lowest tick {lowest}: {ticks:?}"
    );
    let highest = ticks.iter().copied().fold(0.0, f64::max);
    assert!(highest >= 100.0 * (1.0 - 1e-12), "highest tick {highest}");
}

#[test]
fn constant_data_has_a_range() {
    let json = figure(
        LINEAR,
        LINEAR,
        r#"{"id": "c", "kind": "curve", "x": [0, 1, 2], "y": [5, 5, 5]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let ticks = tick_values(&doc, "y");
    let low = ticks.iter().copied().fold(f64::INFINITY, f64::min);
    let high = ticks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(low < 5.0 && 5.0 < high, "{ticks:?}");
    // and on a log axis
    let json = figure(
        LINEAR,
        r#"{"scale": "log"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1, 2], "y": [5, 5, 5]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let ticks = tick_values(&doc, "y");
    let low = ticks.iter().copied().fold(f64::INFINITY, f64::min);
    let high = ticks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(low < 5.0 && 5.0 < high, "{ticks:?}");
}

#[test]
fn exact_ends_of_an_axis() {
    let json = figure(
        r#"{"min": 0, "max": 24}"#,
        LINEAR,
        r#"{"id": "c", "kind": "curve", "x": [1, 30], "y": [1, 2]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let ticks = tick_values(&doc, "x");
    assert_eq!(ticks.first(), Some(&0.0), "{ticks:?}");
    assert!(ticks.iter().all(|t| *t <= 24.0), "{ticks:?}");
}

#[test]
fn axis_title_clears_the_inter_width_of_the_tick_labels() {
    // tick labels like 123456.8, which Charton's estimate measures 15 % short
    let json = figure(
        LINEAR,
        r#"{"label": "amount", "unit": "mmol"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [123456.7, 123457.9]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let in_y = |n: &Node| n.ancestors().any(|a| a.attribute("data-axis") == Some("y"));
    let number = |n: &Node, a: &str| n.attribute(a).unwrap().parse::<f64>().unwrap();
    let labels: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("text") && inside(*n, "charton-tick-label") && in_y(n))
        .collect();
    assert!(labels.iter().any(|n| n.text().unwrap().len() >= 8));
    // the left edge of the widest label in Inter: the labels end at their x
    let left = labels
        .iter()
        .map(|n| number(n, "x") - text_width(n.text().unwrap(), 11.0))
        .fold(f64::INFINITY, f64::min);
    let title = doc
        .descendants()
        .find(|n| n.has_tag_name("text") && inside(*n, "charton-axis-title") && in_y(n))
        .unwrap();
    assert_eq!(title.text(), Some("amount [mmol]"));
    // the title is rotated about its anchor, so its right edge is half a line right of it
    let right = number(&title, "x") + 13.0 / 2.0;
    let gap = left - right;
    assert!(
        (gap - 10.0).abs() < 0.02,
        "gap of {gap} between the labels and the title"
    );
    // and the title is inside the figure
    assert!(number(&title, "x") - 13.0 / 2.0 >= 0.0);
}

#[test]
fn hidden_parts_and_grid() {
    let json = figure(
        r#"{"label": "time", "ticks_visible": false, "grid": true}"#,
        r#"{"label": "conc", "label_visible": false}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 10], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let texts_of = |axis: &str, class: &str| -> Vec<String> {
        doc.descendants()
            .filter(|n| n.has_tag_name("text") && inside(*n, class))
            .filter(|n| {
                n.ancestors()
                    .any(|a| a.attribute("data-axis") == Some(axis))
            })
            .map(|n| n.text().unwrap_or_default().to_string())
            .collect()
    };
    assert!(texts_of("x", "charton-tick-label").is_empty());
    assert_eq!(texts_of("x", "charton-axis-title"), ["time"]);
    assert!(!texts_of("y", "charton-tick-label").is_empty());
    assert!(texts_of("y", "charton-axis-title").is_empty());
    // vertical grid lines at the ticks of x, none of y
    let grid: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("line") && inside(*n, "charton-grid"))
        .collect();
    assert!(!grid.is_empty());
    assert!(grid.iter().all(|l| l.attribute("x1") == l.attribute("x2")));
}

#[test]
fn reversed_axis_runs_backwards() {
    let json = figure(
        r#"{"reverse": true}"#,
        LINEAR,
        r#"{"id": "c", "kind": "curve", "x": [0, 10], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let d = element_group(&doc, "c")
        .descendants()
        .find(|n| n.has_tag_name("path"))
        .and_then(|n| n.attribute("d"))
        .unwrap()
        .to_string();
    let xs: Vec<f64> = d
        .split(['M', 'L'])
        .filter(|p| !p.is_empty())
        .map(|p| p.split(' ').next().unwrap().parse().unwrap())
        .collect();
    assert!(xs[0] > xs[1], "{d}");
}

#[test]
fn empty_and_single_point() {
    let json = figure(
        LINEAR,
        LINEAR,
        r#"{"id": "empty", "kind": "curve", "label": "empty", "x": [], "y": []},
           {"id": "gaps", "kind": "curve", "x": [0, 1], "y": [null, null]},
           {"id": "one", "kind": "curve", "x": [1], "y": [2],
            "style": {"marker": {"type": "square"}}},
           {"id": "band", "kind": "band", "x": [], "low": [], "high": []},
           {"id": "area", "kind": "area", "x": [1], "from": [0], "to": [1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    for id in ["empty", "gaps", "band", "area"] {
        let g = element_group(&doc, id);
        assert!(
            !g.descendants().any(|n| n.has_tag_name("path")),
            "{id} draws a path"
        );
    }
    assert_eq!(
        element_group(&doc, "empty").attribute("data-legend"),
        Some("empty")
    );
    let one = element_group(&doc, "one");
    assert_eq!(
        one.descendants()
            .filter(|n| n.is_element() && *n != one && !n.has_tag_name("g"))
            .count(),
        1
    );
    // a figure of nothing at all
    let svg = render(&figure(LINEAR, LINEAR, ""));
    Document::parse(&svg).unwrap();
}

#[test]
fn text_is_escaped() {
    let text = "<a & µ° m²> \"q\"";
    let quoted = serde_json_string(text);
    let json = figure(
        &format!(r#"{{"label": {quoted}, "unit": "µmol/l"}}"#),
        LINEAR,
        &format!(
            r#"{{"id": "t", "kind": "text", "x": 1, "y": 1, "text": {quoted}}},
               {{"id": "c", "kind": "curve", "label": {quoted}, "legend": {quoted},
                 "x": [0, 2], "y": [0, 2]}}"#
        ),
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let texts: Vec<&str> = doc
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter_map(|n| n.text())
        .collect();
    assert!(texts.contains(&text), "{texts:?}");
    assert!(
        texts.contains(&format!("{text} [µmol/l]").as_str()),
        "{texts:?}"
    );
    assert_eq!(
        element_group(&doc, "c").attribute("data-legend"),
        Some(text)
    );
}

/// A string as JSON text.
fn serde_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn semantic_classes() {
    let svg = render(&spec_text("curve"));
    assert!(!svg.contains("var("), "a var() in the drawing");
    let doc = Document::parse(&svg).unwrap();
    for axis in ["x", "y"] {
        let g = doc
            .descendants()
            .find(|n| n.has_tag_name("g") && n.attribute("data-axis") == Some(axis))
            .unwrap_or_else(|| panic!("no group of the axis {axis}"));
        let class = g.attribute("class").unwrap_or_default();
        assert!(
            class.split_whitespace().any(|c| c == "charton-axis"),
            "{class}"
        );
    }
    let marks: Vec<Node> = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name("g")
                && n.attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|one| one == "charton-mark"))
        })
        .collect();
    assert!(!marks.is_empty());
    for m in marks {
        assert!(
            m.ancestors().any(|a| a.attribute("data-element").is_some()),
            "a mark outside of an element group"
        );
    }
}

#[test]
fn same_spec_same_bytes() {
    let a = render(&spec_text("benchmark"));
    let b = render(&spec_text("benchmark"));
    assert_eq!(a, b);
}

#[test]
fn golden_curve() {
    assert_golden("curve", &render(&spec_text("curve")));
}

#[test]
fn golden_benchmark() {
    assert_golden("benchmark", &render(&spec_text("benchmark")));
}

#[test]
fn golden_rotated() {
    assert_golden("rotated", &render(&spec_text("rotated")));
}

#[test]
fn golden_kinds() {
    assert_golden("kinds", &render(&spec_text("kinds")));
}

#[test]
fn golden_axes() {
    assert_golden("axes", &render(&spec_text("axes")));
}

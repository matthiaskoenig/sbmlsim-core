//! The rendering of one plot: axes, curves, bands, areas, rules and text.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{FontWeight, parse, render_svg, text_width, text_width_weighted};
use support::{assert_golden, spec_text, used_markers};

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
        .map(|n| label_value(n))
        .collect()
}

/// The number a tick label writes: plain, `10` with a superscript exponent, or a mantissa
/// times that (since 0.31.0), the minus sign U+2212.
fn label_value(n: Node) -> f64 {
    let base: String = n
        .children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect();
    let sup: Option<String> = n
        .children()
        .find(|c| c.has_tag_name("tspan"))
        .and_then(|c| c.text())
        .map(|t| t.replace('\u{2212}', "-"));
    let number = |t: &str| {
        t.parse::<f64>()
            .unwrap_or_else(|e| panic!("tick label `{t}`: {e}"))
    };
    match sup {
        None => number(&base),
        Some(e) => {
            let power = 10f64.powf(number(&e));
            match base.split_once('\u{d7}') {
                Some((m, _)) => number(m) * power,
                None => power,
            }
        }
    }
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
    let markers = used_markers(&doc, g).len();
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

/// A figure of [`figure`] with the fonts of a JSON object.
fn with_fonts(json: &str, fonts: &str) -> String {
    json.replacen("\"grid\"", &format!("\"fonts\": {fonts}, \"grid\""), 1)
}

/// The panel of the one plot of a drawing: the clip of its plot group in the figure.
fn the_panel(doc: &Document) -> (f64, f64, f64, f64) {
    let number = |n: Node, a: &str| n.attribute(a).unwrap().parse::<f64>().unwrap();
    let plot = doc
        .descendants()
        .find(|n| n.has_tag_name("svg") && n.attribute("data-plot").is_some())
        .unwrap();
    let g = plot
        .descendants()
        .find(|n| n.attribute("class") == Some("charton-plot"))
        .unwrap();
    let id = g
        .attribute("clip-path")
        .unwrap()
        .trim_start_matches("url(#")
        .trim_end_matches(')');
    let rect = doc
        .descendants()
        .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(id))
        .and_then(|c| c.first_element_child())
        .unwrap();
    (
        number(plot, "x") + number(rect, "x"),
        number(plot, "y") + number(rect, "y"),
        number(rect, "width"),
        number(rect, "height"),
    )
}

/// The texts of a class on an axis.
fn texts_on<'a>(doc: &'a Document<'a>, axis: &str, class: &str) -> Vec<Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| n.has_tag_name("text") && inside(*n, class))
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute("data-axis") == Some(axis))
        })
        .collect()
}

#[test]
fn bold_text_is_wider() {
    // the measure: Inter Bold is wider than Inter Regular, and a text of several lines is as
    // wide as its widest line
    let regular = text_width("concentration 1234", 11.0);
    assert_eq!(
        text_width_weighted("concentration 1234", 11.0, FontWeight::Normal),
        regular
    );
    assert!(text_width_weighted("concentration 1234", 11.0, FontWeight::Bold) > regular * 1.03);
    assert_eq!(text_width("ab\nabcd", 11.0), text_width("abcd", 11.0));
    // bold tick labels take more room left of the panel, which moves right
    let json = figure(
        LINEAR,
        r#"{"label": "amount", "unit": "mmol"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [123456.7, 123457.9]}"#,
    );
    let normal = render(&json);
    let bold = render(&with_fonts(
        &json,
        r#"{"tick_weight": "bold", "label_weight": "bold", "title_weight": "normal"}"#,
    ));
    let (dn, db) = (
        Document::parse(&normal).unwrap(),
        Document::parse(&bold).unwrap(),
    );
    assert!(the_panel(&db).0 > the_panel(&dn).0 + 1.0);
    for axis in ["x", "y"] {
        let ticks = texts_on(&db, axis, "charton-tick-label");
        assert!(!ticks.is_empty());
        assert!(
            ticks
                .iter()
                .all(|t| t.attribute("font-weight") == Some("bold"))
        );
        assert!(
            texts_on(&dn, axis, "charton-tick-label")
                .iter()
                .all(|t| t.attribute("font-weight").is_none())
        );
        let titles = texts_on(&db, axis, "charton-axis-title");
        assert_eq!(titles[0].attribute("font-weight"), Some("bold"));
    }
    // the bold labels end where the regular ones end, and the title keeps its gap of 10 to
    // the widest of them, measured in Inter Bold
    let left = texts_on(&db, "y", "charton-tick-label")
        .iter()
        .map(|n| {
            n.attribute("x").unwrap().parse::<f64>().unwrap()
                - text_width_weighted(n.text().unwrap(), 11.0, FontWeight::Bold)
        })
        .fold(f64::INFINITY, f64::min);
    let title = texts_on(&db, "y", "charton-axis-title")[0];
    let right = title.attribute("x").unwrap().parse::<f64>().unwrap() + 13.0 / 2.0;
    assert!((left - right - 10.0).abs() < 0.02, "{}", left - right);
}

/// The lines of a text: the texts of its `tspan`s with their `x` and `y`, or its own text at its
/// `x` and `y`.
fn lines_of(n: Node) -> Vec<(String, f64, f64)> {
    let number = |n: Node, a: &str| n.attribute(a).unwrap().parse::<f64>().unwrap();
    let spans: Vec<Node> = n.children().filter(|c| c.has_tag_name("tspan")).collect();
    if spans.is_empty() {
        return vec![(
            n.text().unwrap_or_default().to_string(),
            number(n, "x"),
            number(n, "y"),
        )];
    }
    spans
        .iter()
        .map(|s| {
            (
                s.text().unwrap_or_default().to_string(),
                number(*s, "x"),
                number(*s, "y"),
            )
        })
        .collect()
}

#[test]
fn multi_line_axis_label() {
    let one = figure(
        r#"{"label": "time", "unit": "hr"}"#,
        r#"{"label": "concentration", "unit": "mM"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let two = figure(
        r#"{"label": "time\nafter the dose", "unit": "hr"}"#,
        r#"{"label": "concentration\nin plasma", "unit": "mM"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let (s1, s2) = (render(&one), render(&two));
    let (d1, d2) = (Document::parse(&s1).unwrap(), Document::parse(&s2).unwrap());
    let (p1, p2) = (the_panel(&d1), the_panel(&d2));
    // a second line of the label font: 1.2 times 13 pixels more room below and left
    assert!((p2.0 - p1.0 - 15.6).abs() < 0.02, "{p1:?} {p2:?}");
    assert!((p1.3 - p2.3 - 15.6).abs() < 0.02, "{p1:?} {p2:?}");
    // the x title: the first line where the one line was, the second below it
    let x1 = lines_of(texts_on(&d1, "x", "charton-axis-title")[0]);
    let x2 = lines_of(texts_on(&d2, "x", "charton-axis-title")[0]);
    assert_eq!(x1.len(), 1);
    assert_eq!(
        x2.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["time", "after the dose [hr]"]
    );
    let below = p2.1 + p2.3 - (p1.1 + p1.3);
    assert!((x2[0].2 - (x1[0].2 + below)).abs() < 0.02);
    assert!((x2[1].2 - x2[0].2 - 15.6).abs() < 0.02);
    // the y title, rotated: the last line nearest the axis, at the distance of the one line,
    // the first one further out
    let y1 = lines_of(texts_on(&d1, "y", "charton-axis-title")[0]);
    let y2 = lines_of(texts_on(&d2, "y", "charton-axis-title")[0]);
    assert_eq!(
        y2.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["concentration", "in plasma [mM]"]
    );
    let y_title = texts_on(&d2, "y", "charton-axis-title")[0];
    let x_of = |n: Node| n.attribute("x").unwrap().parse::<f64>().unwrap();
    // the rotation about the anchor turns a line lower in the text into one further right
    assert!(
        y_title
            .attribute("transform")
            .is_some_and(|t| t.starts_with("rotate(-90"))
    );
    let anchor = x_of(y_title);
    assert!((p2.0 - anchor - (p1.0 - y1[0].1)).abs() < 0.02);
    let anchor_y = y_title.attribute("y").unwrap().parse::<f64>().unwrap();
    assert!((y2[1].1 - anchor).abs() < 0.02 && (y2[1].2 - anchor_y).abs() < 0.02);
    assert!((y2[0].1 - anchor).abs() < 0.02 && (anchor_y - y2[0].2 - 15.6).abs() < 0.02);
}

#[test]
fn tick_marks_and_tick_sizes() {
    let json = figure(
        r#"{"label": "time", "ticks_visible": false, "tick_marks_visible": true}"#,
        r#"{"label": "c", "tick_marks_visible": false}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 10], "y": [0, 1]}"#,
    );
    let svg = render(&with_fonts(&json, r#"{"tick": 9, "tick_y": 14}"#));
    let doc = Document::parse(&svg).unwrap();
    let marks = |axis: &str| {
        doc.descendants()
            .filter(|n| n.has_tag_name("path") && inside(*n, "charton-tick"))
            .filter(|n| {
                n.ancestors()
                    .any(|a| a.attribute("data-axis") == Some(axis))
            })
            .count()
    };
    // x: marks without labels; y: labels without marks
    assert!(marks("x") > 0);
    assert!(texts_on(&doc, "x", "charton-tick-label").is_empty());
    assert_eq!(marks("y"), 0);
    let labels = texts_on(&doc, "y", "charton-tick-label");
    assert!(!labels.is_empty());
    assert!(
        labels
            .iter()
            .all(|n| n.attribute("font-size") == Some("14"))
    );
    // without the field the marks follow the labels, as before 0.28.0
    let old = render(&figure(
        r#"{"label": "time", "ticks_visible": false}"#,
        r#"{"label": "c"}"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 10], "y": [0, 1]}"#,
    ));
    let old = Document::parse(&old).unwrap();
    assert!(!old.descendants().any(|n| inside(n, "charton-tick")
        && n.ancestors().any(|a| a.attribute("data-axis") == Some("x"))));
    // the x labels take the x size
    let svg = render(&with_fonts(
        &figure(
            LINEAR,
            LINEAR,
            r#"{"id": "c", "kind": "curve", "x": [0, 10], "y": [0, 1]}"#,
        ),
        r#"{"tick": 9, "tick_y": 14}"#,
    ));
    let doc = Document::parse(&svg).unwrap();
    assert!(
        texts_on(&doc, "x", "charton-tick-label")
            .iter()
            .all(|n| n.attribute("font-size") == Some("9"))
    );
    assert!(
        texts_on(&doc, "y", "charton-tick-label")
            .iter()
            .all(|n| n.attribute("font-size") == Some("14"))
    );
}

#[test]
fn multi_line_text_element() {
    let json = figure(
        LINEAR,
        LINEAR,
        r#"{"id": "t", "kind": "text", "x": 1, "y": 1, "text": "first\nsecond", "anchor": "middle"},
           {"id": "c", "kind": "curve", "x": [0, 2], "y": [0, 2]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let text = element_group(&doc, "t")
        .descendants()
        .find(|n| n.has_tag_name("text"))
        .unwrap();
    let lines = lines_of(text);
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
    // the lines centred on the position, 1.2 times the font size apart
    let y = text.attribute("y").unwrap().parse::<f64>().unwrap();
    assert!((lines[1].2 - lines[0].2 - 13.2).abs() < 0.02);
    assert!(((lines[0].2 + lines[1].2) / 2.0 - y).abs() < 0.02);
}

#[test]
fn a_tick_label_with_a_newline_is_lines() {
    let json = figure(
        r#"{"scale": "category", "categories": ["a\nb", "c"]}"#,
        "{}",
        r#"{"id": "e", "kind": "curve", "x": [0, 1], "y": [1, 2]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let lines: Vec<(&str, f64)> = doc
        .descendants()
        .filter(|n| n.has_tag_name("tspan"))
        .map(|n| {
            (
                n.text().unwrap_or_default(),
                n.attribute("y").unwrap().parse().unwrap(),
            )
        })
        .collect();
    // the lines of the label of the bottom axis down from its tick, a line height apart
    assert_eq!(lines.len(), 2, "{svg}");
    assert_eq!((lines[0].0, lines[1].0), ("a", "b"));
    assert!((lines[1].1 - lines[0].1 - 13.2).abs() < 0.02);
}

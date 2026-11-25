//! What the panels of the web app rely on (`PlotSpec` of `plot.ts` of sbmlsim-web): the frame
//! of the value axis (`yLimits`), its exact ends (`yEnds`), the end of the x axis (`xEnd`), the
//! declared legends (`declaredLegends`), the frozen runs and the sizes of the text
//! (`fontSizes`).
//!
//! The geometry is read back from the SVG as in `render_figure.rs`: a plot is a nested `<svg>`,
//! its panel the clip rectangle of its first `charton-plot` group.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{parse, render_svg};
use serde_json::{Value, json};
use support::{assert_golden, spec_text};

/// Renders a figure of JSON.
fn render(spec: &Value) -> String {
    render_svg(&parse(&spec.to_string()).unwrap_or_else(|e| panic!("{e}")))
}

/// A rectangle in the pixels of the figure.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.width
    }
    fn bottom(&self) -> f64 {
        self.y + self.height
    }
}

/// A number attribute of a node.
fn number(n: Node, name: &str) -> f64 {
    n.attribute(name)
        .unwrap_or_else(|| panic!("no attribute {name} on {:?}", n.tag_name()))
        .parse()
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Whether a node carries a class.
fn has_class(n: Node, class: &str) -> bool {
    n.attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

/// The size of the figure.
fn figure_size(doc: &Document) -> (f64, f64) {
    let root = doc.root_element();
    (number(root, "width"), number(root, "height"))
}

/// The offset of a node in the figure: the `x` and `y` of the nested `<svg>`s around it.
fn offset(n: Node) -> (f64, f64) {
    n.ancestors()
        .filter(|a| a.has_tag_name("svg") && a.parent_element().is_some())
        .fold((0.0, 0.0), |(x, y), a| {
            (x + number(a, "x"), y + number(a, "y"))
        })
}

/// The rectangle of a clip path, by its reference `url(#id)`.
fn clip_rect(doc: &Document, reference: &str) -> Rect {
    let id = reference.trim_start_matches("url(#").trim_end_matches(')');
    let rect = doc
        .descendants()
        .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(id))
        .and_then(|c| c.children().find(|n| n.has_tag_name("rect")))
        .expect("the clip rectangle");
    Rect {
        x: number(rect, "x"),
        y: number(rect, "y"),
        width: number(rect, "width"),
        height: number(rect, "height"),
    }
}

/// The panel of the plot `p` in the figure: the clip of its first `charton-plot` group.
fn panel(doc: &Document) -> Rect {
    let g = doc
        .descendants()
        .find(|n| has_class(*n, "charton-plot"))
        .expect("a panel");
    let r = clip_rect(doc, g.attribute("clip-path").expect("a clipped panel"));
    let (dx, dy) = offset(g);
    Rect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    }
}

/// The legends of the figure: the frame and the labels of the entries of each.
fn legends(doc: &Document) -> Vec<(Rect, Vec<String>)> {
    doc.descendants()
        .filter(|n| n.has_tag_name("g") && has_class(*n, "sbmlsim-legend"))
        .map(|g| {
            let frame = g
                .descendants()
                .find(|n| n.has_tag_name("rect") && has_class(*n, "sbmlsim-legend-frame"))
                .expect("a legend frame");
            let (dx, dy) = offset(frame);
            let labels = g
                .descendants()
                .filter(|n| n.has_tag_name("text") && has_class(*n, "sbmlsim-legend-label"))
                .map(|t| t.text().unwrap_or_default().to_string())
                .collect();
            let rect = Rect {
                x: number(frame, "x") + dx,
                y: number(frame, "y") + dy,
                width: number(frame, "width"),
                height: number(frame, "height"),
            };
            (rect, labels)
        })
        .collect()
}

/// The labels of the ticks of an axis (`x` or `y`).
fn tick_labels(doc: &Document, axis: &str) -> Vec<String> {
    doc.descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute("data-axis") == Some(axis))
                && n.ancestors().any(|a| has_class(a, "charton-tick-label"))
        })
        .map(|n| n.text().unwrap_or_default().to_string())
        .collect()
}

/// The group of an element.
fn element<'a>(doc: &'a Document<'a>, id: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| n.has_tag_name("g") && n.attribute("data-element") == Some(id))
        .unwrap_or_else(|| panic!("no group of {id}"))
}

/// The vertices of the paths of an element group, in the figure.
fn vertices(doc: &Document, id: &str) -> Vec<(f64, f64)> {
    let g = element(doc, id);
    g.descendants()
        .filter(|n| n.has_tag_name("path"))
        .flat_map(|path| {
            let (dx, dy) = offset(path);
            let d = path.attribute("d").unwrap().to_string();
            d.split(['M', 'L'])
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    let mut it = s.trim_end_matches('Z').split_whitespace();
                    let x: f64 = it.next().unwrap().parse().unwrap();
                    let y: f64 = it.next().unwrap().parse().unwrap();
                    (x + dx, y + dy)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The markers of an element group: its circles.
fn markers(doc: &Document, id: &str) -> Vec<(f64, f64)> {
    element(doc, id)
        .descendants()
        .filter(|n| n.has_tag_name("circle"))
        .map(|c| {
            let (dx, dy) = offset(c);
            (number(c, "cx") + dx, number(c, "cy") + dy)
        })
        .collect()
}

/// A panel of the web app: a live curve, the study data `studies` (label, x, y) as markers, a
/// frozen run of the live curve and the given axes.
fn panel_spec(studies: &[(&str, &[f64], &[f64])], x: Value, y: Value, extra: Value) -> Value {
    let mut elements = vec![
        json!({
            "id": "frozen-1", "kind": "curve", "label": "frozen at 4 mg", "legend": "frozen-1",
            "x": [0, 1, 2, 3, 4], "y": [0, 2, 3, 2.5, 2],
            "style": {"color": "#440154", "line": {"type": "dashed"}, "opacity": 0.85}
        }),
        json!({
            "id": "live", "kind": "curve", "label": "glimepiride",
            "x": [0, 1, 2, 3, 4], "y": [0, 4, 6, 5, 4],
            "style": {"color": "#440154"}
        }),
    ];
    for (k, (label, xs, ys)) in studies.iter().enumerate() {
        elements.push(json!({
            "id": format!("study-{k}"), "kind": "curve", "label": label,
            "x": xs, "y": ys,
            "style": {"color": "#440154", "line": {"type": "none"}, "marker": {"type": "circle"}}
        }));
    }
    let mut plot = json!({
        "id": "p", "row": 0, "column": 0, "x": x, "y": y, "elements": elements
    });
    if let (Value::Object(p), Value::Object(e)) = (&mut plot, extra) {
        p.extend(e);
    }
    json!({
        "contract": "figure/1", "width": 480, "height": 300,
        "grid": {"rows": 1, "columns": 1},
        "legend": {"scope": "plot", "position": "outside"},
        "plots": [plot]
    })
}

/// The axes of a panel: time in hours and a concentration.
fn axes() -> (Value, Value) {
    (
        json!({"label": "time", "unit": "hr"}),
        json!({"label": "glimepiride", "unit": "nM"}),
    )
}

#[test]
fn declared_legends_fix_the_box() {
    let declared = json!({"declared_legends": [
        ["Study A, 1 mg"],
        ["Study B, 4 mg (a much longer label)", "Study C, 8 mg"]
    ]});
    let (x, y) = axes();
    let one = render(&panel_spec(
        &[("Study A, 1 mg", &[1.0, 2.0], &[3.0, 5.0])],
        x.clone(),
        y.clone(),
        declared.clone(),
    ));
    let other = render(&panel_spec(
        &[("Study C, 8 mg", &[1.0, 3.0], &[4.0, 5.5])],
        x,
        y,
        declared,
    ));
    let (a, b) = (
        Document::parse(&one).unwrap(),
        Document::parse(&other).unwrap(),
    );
    // the plot, the figure and the legend stay where they are whichever list is drawn
    assert_eq!(panel(&a), panel(&b));
    assert_eq!(figure_size(&a), figure_size(&b));
    let (la, lb) = (legends(&a), legends(&b));
    assert_eq!(la.len(), 1);
    assert_eq!(lb.len(), 1);
    assert_eq!(
        (la[0].0.x, la[0].0.y, la[0].0.width),
        (lb[0].0.x, lb[0].0.y, lb[0].0.width)
    );
    // a declared label without an element is not drawn
    assert_eq!(
        la[0].1,
        ["frozen at 4 mg", "glimepiride", "Study A, 1 mg"].map(String::from)
    );
    assert_eq!(
        lb[0].1,
        ["frozen at 4 mg", "glimepiride", "Study C, 8 mg"].map(String::from)
    );
    // the legend is as wide as the widest declared label needs, and the figure holds it
    let (w, _) = figure_size(&a);
    assert!(la[0].0.right() <= w + 1e-9, "{:?} past {w}", la[0].0);
    let undeclared = {
        let (x, y) = axes();
        render(&panel_spec(
            &[("Study A, 1 mg", &[1.0, 2.0], &[3.0, 5.0])],
            x,
            y,
            json!({}),
        ))
    };
    let u = Document::parse(&undeclared).unwrap();
    assert!(
        legends(&u)[0].0.width < la[0].0.width,
        "the declared lists widen the legend"
    );
}

#[test]
fn declared_legends_reserve_the_rows_of_the_longest_list() {
    // a legend inside: the box which is checked against the panel holds the most rows a
    // declared list has, so a panel whose list of the moment is short keeps the place it gets
    // with the longest list
    let labels: Vec<String> = (0..12).map(|k| format!("Study {k}")).collect();
    let declared = json!({"declared_legends": [labels]});
    let (x, y) = axes();
    let mut spec = panel_spec(
        &[("Study 0", &[1.0], &[3.0])],
        x.clone(),
        y.clone(),
        declared,
    );
    spec["legend"]["position"] = json!("inside");
    let mut plain = panel_spec(&[("Study 0", &[1.0], &[3.0])], x, y, json!({}));
    plain["legend"]["position"] = json!("inside");
    let (with, without) = (render(&spec), render(&plain));
    let (a, b) = (
        Document::parse(&with).unwrap(),
        Document::parse(&without).unwrap(),
    );
    let (la, lb) = (legends(&a), legends(&b));
    // fifteen rows do not fit into the panel, so the legend goes outside and the figure widens
    let p = panel(&a);
    assert!(la[0].0.x >= p.right(), "{:?} inside {p:?}", la[0].0);
    assert!(lb[0].0.x < panel(&b).right(), "three rows fit inside");
    assert_eq!(la[0].1, lb[0].1);
}

#[test]
fn frame_holds_still() {
    let frame = |data: &[f64], log: bool| {
        let (x, _) = axes();
        let mut y =
            json!({"label": "glimepiride", "unit": "nM", "frame": {"low": 0.5, "high": 12}});
        if log {
            y["scale"] = json!("log");
        }
        render(&panel_spec(
            &[("Study A", &[1.0, 2.0], data)],
            x,
            y,
            json!({}),
        ))
    };
    for log in [false, true] {
        let small = frame(&[0.6, 0.7], log);
        let large = frame(&[9.0, 30.0], log);
        let (a, b) = (
            Document::parse(&small).unwrap(),
            Document::parse(&large).unwrap(),
        );
        assert_eq!(tick_labels(&a, "y"), tick_labels(&b, "y"), "log {log}");
        assert_eq!(panel(&a), panel(&b), "log {log}");
    }
}

#[test]
fn ends_cut() {
    let (x, _) = axes();
    let y = json!({"label": "glimepiride", "unit": "nM", "min": 0, "max": 2});
    let svg = render(&panel_spec(
        &[("Study A", &[1.0, 2.0], &[1.0, 5.0])],
        x,
        y,
        json!({}),
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    // the axis ends at its declared ends
    let ticks = tick_labels(&doc, "y");
    assert_eq!(ticks.first().map(String::as_str), Some("0"));
    assert_eq!(ticks.last().map(String::as_str), Some("2"));
    // every element is drawn inside a group clipped to the panel
    for id in ["live", "frozen-1", "study-0"] {
        let g = element(&doc, id);
        let clip = g
            .ancestors()
            .find_map(|a| a.attribute("clip-path"))
            .unwrap_or_else(|| panic!("{id} is not clipped"));
        let r = clip_rect(&doc, clip);
        let (dx, dy) = offset(g);
        let r = Rect {
            x: r.x + dx,
            y: r.y + dy,
            ..r
        };
        assert_eq!(r, p, "{id}");
    }
    // the curve which reaches past the upper end still runs to the end of the x axis
    let live = vertices(&doc, "live");
    assert!(
        live.iter().any(|(_, y)| *y < p.y),
        "the curve passes the end"
    );
}

#[test]
fn x_end_drops_data() {
    let (_, y) = axes();
    let x = json!({"label": "time", "unit": "hr", "max": 2.5});
    let mut spec = panel_spec(&[("Study A", &[1.0, 3.0], &[3.0, 4.0])], x, y, json!({}));
    // the live curve reaches far up beyond the end of the x axis
    spec["plots"][0]["elements"][1]["y"] = json!([0, 4, 6, 50, 80]);
    spec["plots"][0]["elements"][1]["style"]["marker"] = json!({"type": "circle"});
    let svg = render(&spec);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    // the axis ends at the end
    assert_eq!(
        tick_labels(&doc, "x").last().map(String::as_str),
        Some("2.5")
    );
    // the point of the study beyond the end is no point of the figure
    assert_eq!(markers(&doc, "study-0").len(), 1);
    // the curves end at the end of the axis, with no vertex and no marker beyond it
    for id in ["live", "frozen-1"] {
        let v = vertices(&doc, id);
        let right = v.iter().map(|(x, _)| *x).fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (right - p.right()).abs() < 0.011,
            "{id} ends at {right}, the panel at {}",
            p.right()
        );
    }
    assert_eq!(markers(&doc, "live").len(), 3, "the markers of 0, 1 and 2");
    // the value axis is the one of what is drawn: the values beyond the end (50 and 80) are not
    // on it, the 28 where the live curve crosses the end is
    let top: f64 = tick_labels(&doc, "y").last().unwrap().parse().unwrap();
    assert!(top < 40.0, "the y axis reaches {top}");
    let end = vertices(&doc, "live")
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    assert!(end.1 >= p.y && end.1 <= p.bottom(), "{end:?} outside {p:?}");
}

#[test]
fn font_sizes_from_spec() {
    let (x, _) = axes();
    let y = json!({"label": "glimepiride", "unit": "nM"});
    let mut spec = panel_spec(&[("Study A", &[1.0], &[3.0])], x, y, json!({}));
    spec["title"] = json!("Glimepiride");
    spec["fonts"] = json!({"title": 19, "label": 15, "tick": 9, "legend": 12});
    let svg = render(&spec);
    assert!(!svg.contains("var("), "no CSS variable in the file");
    let doc = Document::parse(&svg).unwrap();
    let sizes = |class: &str| -> Vec<f64> {
        doc.descendants()
            .filter(|n| n.has_tag_name("text"))
            .filter(|n| n.ancestors().any(|a| has_class(a, class)))
            .map(|n| number(n, "font-size"))
            .collect()
    };
    for (class, size, count) in [
        ("sbmlsim-figure-title", 19.0, 1),
        ("charton-axis-title", 15.0, 2),
        ("charton-tick-label", 9.0, 4),
        ("sbmlsim-legend-label", 12.0, 3),
    ] {
        let found = sizes(class);
        assert!(found.len() >= count, "{class}: {found:?}");
        assert!(found.iter().all(|s| *s == size), "{class}: {found:?}");
    }
}

#[test]
fn golden_webapp() {
    let svg = render_svg(&parse(&spec_text("webapp")).unwrap());
    assert_golden("webapp", &svg);
}

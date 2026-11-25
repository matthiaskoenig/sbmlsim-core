//! The composition of a figure: the grid of plots with its spans, the title band, the legends
//! of a plot and of a row inside and outside, and the right y axis.
//!
//! The geometry is read back from the SVG: a plot is a nested `<svg class="sbmlsim-plot">` at
//! its place in the figure, its panel the clip rectangle of its first `charton-plot` group, and
//! a legend a `<g class="sbmlsim-legend">` with its frame `rect.sbmlsim-legend-frame` and an
//! entry group `g.sbmlsim-legend-entry` per entry, all in the pixels of the figure.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{parse, render_svg, text_width};
use support::{assert_golden, spec_text};

/// Renders a figure of JSON text.
fn render(json: &str) -> String {
    render_svg(&parse(json).unwrap_or_else(|e| panic!("{e}")))
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
    /// Whether two rectangles share area.
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    /// Whether `o` lies inside this rectangle.
    fn holds(&self, o: &Rect) -> bool {
        o.x >= self.x - 1e-6
            && o.y >= self.y - 1e-6
            && o.right() <= self.right() + 1e-6
            && o.bottom() <= self.bottom() + 1e-6
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

/// The nested `<svg>` of a plot.
fn plot_node<'a>(doc: &'a Document<'a>, id: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| n.has_tag_name("svg") && n.attribute("data-plot") == Some(id))
        .unwrap_or_else(|| panic!("no plot {id}"))
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

/// The panel of a plot in the figure: the clip of its first `charton-plot` group.
fn panel(doc: &Document, id: &str) -> Rect {
    let plot = plot_node(doc, id);
    let g = plot
        .descendants()
        .find(|n| has_class(*n, "charton-plot"))
        .unwrap_or_else(|| panic!("no panel of {id}"));
    let r = clip_rect(doc, g.attribute("clip-path").expect("a clipped panel"));
    let (dx, dy) = offset(g);
    Rect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    }
}

/// A legend of the figure: its frame and its entries as (legend group, label).
#[derive(Debug)]
struct Legend {
    frame: Rect,
    entries: Vec<(String, String)>,
}

/// The legends of the figure in their order.
fn legends(doc: &Document) -> Vec<Legend> {
    doc.descendants()
        .filter(|n| n.has_tag_name("g") && has_class(*n, "sbmlsim-legend"))
        .map(|g| {
            let frame = g
                .descendants()
                .find(|n| n.has_tag_name("rect") && has_class(*n, "sbmlsim-legend-frame"))
                .expect("a legend frame");
            let (dx, dy) = offset(frame);
            let entries = g
                .descendants()
                .filter(|n| n.has_tag_name("g") && has_class(*n, "sbmlsim-legend-entry"))
                .map(|e| {
                    let label = e
                        .descendants()
                        .find(|t| t.has_tag_name("text"))
                        .and_then(|t| t.text())
                        .unwrap_or_default()
                        .to_string();
                    (
                        e.attribute("data-legend").unwrap_or_default().to_string(),
                        label,
                    )
                })
                .collect();
            Legend {
                frame: Rect {
                    x: number(frame, "x") + dx,
                    y: number(frame, "y") + dy,
                    width: number(frame, "width"),
                    height: number(frame, "height"),
                },
                entries,
            }
        })
        .collect()
}

/// The texts of the axis `axis` of a plot with their positions in the figure.
fn axis_texts(doc: &Document, plot: &str, axis: &str) -> Vec<(String, f64, f64)> {
    let node = plot_node(doc, plot);
    node.descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute("data-axis") == Some(axis))
                && n.ancestors().any(|a| has_class(a, "charton-tick-label"))
        })
        .map(|n| {
            let (dx, dy) = offset(n);
            (
                n.text().unwrap_or_default().to_string(),
                number(n, "x") + dx,
                number(n, "y") + dy,
            )
        })
        .collect()
}

/// The points of the first path of an element group, in the figure.
fn path_points(doc: &Document, element: &str) -> Vec<(f64, f64)> {
    let g = doc
        .descendants()
        .find(|n| n.has_tag_name("g") && n.attribute("data-element") == Some(element))
        .unwrap_or_else(|| panic!("no group of {element}"));
    let path = g
        .descendants()
        .find(|n| n.has_tag_name("path"))
        .unwrap_or_else(|| panic!("no path in {element}"));
    let (dx, dy) = offset(path);
    let d = path.attribute("d").unwrap();
    d.split(['M', 'L'])
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let mut it = s.trim_end_matches('Z').split_whitespace();
            let x: f64 = it.next().unwrap().parse().unwrap();
            let y: f64 = it.next().unwrap().parse().unwrap();
            (x + dx, y + dy)
        })
        .collect()
}

/// A curve element as JSON.
fn curve(id: &str, extra: &str, x: &str, y: &str) -> String {
    format!(r#"{{"id": "{id}", "kind": "curve", {extra} "x": {x}, "y": {y}}}"#)
}

/// A plot as JSON.
fn plot(id: &str, row: u32, column: u32, extra: &str, elements: &[String]) -> String {
    format!(
        r#"{{"id": "{id}", "row": {row}, "column": {column}, {extra}
            "x": {{"label": "time", "unit": "hr"}}, "y": {{"label": "c", "unit": "mM"}},
            "elements": [{}]}}"#,
        elements.join(", ")
    )
}

/// A figure as JSON.
fn figure(
    width: f64,
    height: f64,
    extra: &str,
    rows: u32,
    columns: u32,
    plots: &[String],
) -> String {
    format!(
        r#"{{"contract": "figure/1", "width": {width}, "height": {height}, {extra}
            "grid": {{"rows": {rows}, "columns": {columns}}}, "plots": [{}]}}"#,
        plots.join(", ")
    )
}

/// A plot `a` at (0, 0), `b` at (0, 1) and `c` at (1, 0) over both columns.
fn spanning_grid(extra: &str) -> String {
    let c = |id: &str| curve(id, r#""label": "l","#, "[0, 1, 2]", "[1, 3, 2]");
    figure(
        700.0,
        500.0,
        extra,
        2,
        2,
        &[
            plot("a", 0, 0, "", &[c("a_l")]),
            plot("b", 0, 1, r#""title": "b","#, &[c("b_l")]),
            plot("c", 1, 0, r#""column_span": 2,"#, &[c("c_l")]),
        ],
    )
}

#[test]
fn grid_cells_do_not_overlap() {
    let svg = render(&spanning_grid(""));
    let doc = Document::parse(&svg).unwrap();
    let (w, h) = figure_size(&doc);
    let figure = Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
    };
    let rects: Vec<Rect> = ["a", "b", "c"].iter().map(|id| panel(&doc, id)).collect();
    for (i, r) in rects.iter().enumerate() {
        assert!(figure.holds(r), "panel {i} {r:?} outside of the figure");
        assert!(r.width > 100.0 && r.height > 100.0, "panel {i} {r:?}");
        for (j, o) in rects.iter().enumerate().skip(i + 1) {
            assert!(!r.overlaps(o), "panels {i} {r:?} and {j} {o:?} overlap");
        }
    }
    let (a, b, c) = (rects[0], rects[1], rects[2]);
    // the panels of a column start together, the spanning plot ends with the last column
    assert!((a.x - c.x).abs() < 0.01, "{a:?} {c:?}");
    assert!((b.right() - c.right()).abs() < 0.01, "{b:?} {c:?}");
    // the panels of a row share their top and bottom, though only b has a title
    assert!((a.y - b.y).abs() < 0.01 && (a.bottom() - b.bottom()).abs() < 0.01);
    // the tick labels of b's y axis start right of a's panel, the x axis of a above c's panel
    for (text, x, _) in axis_texts(&doc, "b", "y") {
        assert!(x - text_width(&text, 11.0) > a.right(), "{text} at {x}");
    }
    for (text, _, y) in axis_texts(&doc, "a", "x") {
        assert!(y + 11.0 < c.y, "{text} at {y}");
    }
}

#[test]
fn title_band_above_plots() {
    let svg = render(&spanning_grid(r#""title": "Figure title","#));
    let doc = Document::parse(&svg).unwrap();
    let title = doc
        .descendants()
        .find(|n| n.has_tag_name("g") && has_class(*n, "sbmlsim-figure-title"))
        .and_then(|g| g.descendants().find(|n| n.has_tag_name("text")))
        .expect("the figure title");
    assert_eq!(title.text(), Some("Figure title"));
    // the title is centred in a band of twice its font size
    let band = 2.0 * 16.0;
    assert!((number(title, "y") - band / 2.0).abs() < 1e-9);
    for id in ["a", "b", "c"] {
        let p = panel(&doc, id);
        assert!(p.y > band, "panel of {id} at {}", p.y);
        // nothing of a plot, its title included, reaches into the band
        for t in plot_node(&doc, id)
            .descendants()
            .filter(|n| n.has_tag_name("text"))
        {
            let (_, dy) = offset(t);
            let y = number(t, "y") + dy;
            assert!(y - 8.0 > band, "{:?} of {id} at {y}", t.text());
        }
    }
}

/// A figure of one plot with labelled curves and legend settings.
fn one_plot(legend: &str, elements: &[String]) -> String {
    figure(
        500.0,
        400.0,
        &format!(r#""legend": {legend},"#),
        1,
        1,
        &[plot("p", 0, 0, "", elements)],
    )
}

#[test]
fn legend_inside() {
    let elements = [
        curve("one", r#""label": "dose 1","#, "[0, 1, 2]", "[0, 2, 1]"),
        curve("two", r#""label": "dose 2","#, "[0, 1, 2]", "[0, 1, 0.5]"),
    ];
    let svg = render(&one_plot(
        r#"{"scope": "plot", "position": "inside"}"#,
        &elements,
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let found = legends(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        p.holds(&found[0].frame),
        "{:?} not in {p:?}",
        found[0].frame
    );
    assert_eq!(
        found[0].entries,
        [
            ("one".to_string(), "dose 1".to_string()),
            ("two".to_string(), "dose 2".to_string())
        ]
    );
}

#[test]
fn legend_inside_avoids_data() {
    // a dense rising line fills the lower left and the upper right corner
    let n = 200;
    let x: Vec<String> = (0..=n)
        .map(|i| format!("{}", f64::from(i) / f64::from(n)))
        .collect();
    let xs = format!("[{}]", x.join(", "));
    let svg = render(&one_plot(
        r#"{"position": "inside"}"#,
        &[curve("rise", r#""label": "rising line","#, &xs, &xs)],
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    assert!(p.holds(&frame));
    // the upper left corner is free
    assert!(frame.right() < p.x + p.width / 2.0, "{frame:?} in {p:?}");
    assert!(frame.bottom() < p.y + p.height / 2.0, "{frame:?} in {p:?}");
}

#[test]
fn legend_outside_widens() {
    let elements = [
        curve(
            "one",
            r#""label": "a long label of dose 1","#,
            "[0, 1]",
            "[0, 2]",
        ),
        curve("two", r#""label": "dose 2","#, "[0, 1]", "[0, 1]"),
    ];
    let inside = render(&one_plot(r#"{"position": "inside"}"#, &elements));
    let outside = render(&one_plot(r#"{"position": "outside"}"#, &elements));
    let (din, dout) = (
        Document::parse(&inside).unwrap(),
        Document::parse(&outside).unwrap(),
    );
    let (pin, pout) = (panel(&din, "p"), panel(&dout, "p"));
    let legend = legends(&dout)[0].frame;
    let ((win, hin), (wout, hout)) = (figure_size(&din), figure_size(&dout));
    assert_eq!((win, hin), (500.0, 400.0));
    assert_eq!(hout, 400.0);
    // the figure grows by the legend and a gap, the plot keeps its panel
    assert!(
        wout - win >= legend.width && wout - win <= legend.width + 20.0,
        "{wout} {legend:?}"
    );
    assert!((pin.width - pout.width).abs() < 0.01, "{pin:?} {pout:?}");
    assert!((pin.height - pout.height).abs() < 0.01, "{pin:?} {pout:?}");
    assert!(
        legend.x > pout.right() && legend.right() <= wout,
        "{legend:?} {pout:?}"
    );
    assert!((legend.y - pout.y).abs() < 0.01, "{legend:?} {pout:?}");
}

#[test]
fn row_legend_merges() {
    let dose = |id: &str| {
        curve(
            id,
            r#""label": "dose", "legend": "dose","#,
            "[0, 1]",
            "[0, 1]",
        )
    };
    let data = curve(
        "b_data",
        r#""label": "data", "style": {"line": {"type": "none"}, "marker": {"type": "circle"}},"#,
        "[0, 1]",
        "[0.2, 0.8]",
    );
    // the same group of another style in another plot is an entry of its own
    let dashed = curve(
        "e_dashed",
        r##""label": "dose", "legend": "dose", "style": {"color": "#d62728", "line": {"type": "dashed"}},"##,
        "[0, 1]",
        "[1, 0]",
    );
    let json = figure(
        1000.0,
        500.0,
        r#""legend": {"scope": "row", "position": "outside"},"#,
        2,
        3,
        &[
            plot("a", 0, 0, "", &[dose("a_dose")]),
            plot("b", 0, 1, "", &[dose("b_dose"), data]),
            plot("e", 0, 2, "", &[dashed]),
            plot("c", 1, 0, r#""legend": false,"#, &[dose("c_dose")]),
            plot(
                "d",
                1,
                1,
                "",
                &[curve("d_other", r#""label": "other","#, "[0, 1]", "[1, 0]")],
            ),
        ],
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let found = legends(&doc);
    assert_eq!(found.len(), 2, "{found:?}");
    let entry = |g: &str, l: &str| (g.to_string(), l.to_string());
    assert_eq!(
        found[0].entries,
        [
            entry("dose", "dose"),
            entry("b_data", "data"),
            entry("dose", "dose")
        ]
    );
    assert_eq!(found[1].entries, [entry("d_other", "other")]);
    // right of the grid, at the top of the panels of their row, aligned
    let (e, d) = (panel(&doc, "e"), panel(&doc, "d"));
    assert!(found[0].frame.x > e.right());
    assert!((found[0].frame.x - found[1].frame.x).abs() < 0.01);
    assert!((found[0].frame.y - e.y).abs() < 0.01 && (found[1].frame.y - d.y).abs() < 0.01);
}

#[test]
fn legend_group_one_entry() {
    let elements = [
        curve("first", r#""legend": "run 1","#, "[0, 1]", "[0, 1]"),
        curve(
            "second",
            r#""label": "run 1", "legend": "run 1","#,
            "[0, 1]",
            "[1, 2]",
        ),
        curve(
            "third",
            r#""label": "other", "legend": "run 1","#,
            "[0, 1]",
            "[2, 3]",
        ),
        curve("fourth", r#""label": "run 2","#, "[0, 1]", "[3, 4]"),
    ];
    let svg = render(&one_plot(r#"{"position": "outside"}"#, &elements));
    let doc = Document::parse(&svg).unwrap();
    let found = legends(&doc);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].entries,
        [
            ("run 1".to_string(), "run 1".to_string()),
            ("fourth".to_string(), "run 2".to_string())
        ]
    );
    // the swatch is the one of the element whose label the entry shows
    let entry = doc
        .descendants()
        .find(|n| {
            has_class(*n, "sbmlsim-legend-entry") && n.attribute("data-legend") == Some("run 1")
        })
        .unwrap();
    let swatch = entry
        .descendants()
        .find(|n| n.has_tag_name("path") || n.has_tag_name("line"))
        .expect("a swatch line");
    let second = doc
        .descendants()
        .find(|n| n.attribute("data-element") == Some("second"))
        .and_then(|g| g.descendants().find(|n| n.has_tag_name("path")))
        .unwrap();
    assert_eq!(swatch.attribute("stroke"), second.attribute("stroke"));
}

#[test]
fn right_axis() {
    let json = r##"{"contract": "figure/1", "width": 600, "height": 400,
        "grid": {"rows": 1, "columns": 1},
        "plots": [{"id": "p", "row": 0, "column": 0,
            "x": {"label": "time", "unit": "hr", "min": 0, "max": 10},
            "y": {"label": "c", "unit": "mM", "min": 0, "max": 1},
            "y2": {"label": "amount", "unit": "mg", "min": 0, "max": 1000},
            "elements": [
              {"id": "left", "kind": "curve", "x": [0, 10], "y": [0, 1]},
              {"id": "right", "kind": "curve", "axis": "y2", "x": [0, 5, 10], "y": [0, 500, 1000],
               "style": {"color": "#d62728"}}
            ]}]}"##;
    let svg = render(json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    // the right axis: ticks right of the panel, from 0 at its bottom to 1000 at its top
    let ticks = axis_texts(&doc, "p", "y2");
    assert!(ticks.len() >= 3, "{ticks:?}");
    for (text, x, _) in &ticks {
        assert!(*x > p.right(), "{text} at {x}, panel {p:?}");
    }
    let at = |label: &str| ticks.iter().find(|t| t.0 == label).map(|t| t.2);
    assert!(
        (at("0").expect("tick 0") - p.bottom()).abs() < 0.01,
        "{ticks:?}"
    );
    assert!(
        (at("1000").expect("tick 1000") - p.y).abs() < 0.01,
        "{ticks:?}"
    );
    let title = plot_node(&doc, "p")
        .descendants()
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute("data-axis") == Some("y2"))
        })
        .find(|n| n.has_tag_name("text") && n.text() == Some("amount [mg]"));
    assert!(title.is_some(), "no title of the right axis");
    // the element of the right axis is drawn once, scaled by its axis
    let groups = doc
        .descendants()
        .filter(|n| n.attribute("data-element") == Some("right"))
        .count();
    assert_eq!(groups, 1);
    let points = path_points(&doc, "right");
    assert_eq!(points.len(), 3, "{points:?}");
    for ((x, y), (ex, ey)) in points.iter().zip([
        (p.x, p.bottom()),
        (p.x + p.width / 2.0, p.y + p.height / 2.0),
        (p.right(), p.y),
    ]) {
        assert!(
            (x - ex).abs() < 0.02 && (y - ey).abs() < 0.02,
            "{points:?} in {p:?}"
        );
    }
    // the element of the left axis is scaled by the left axis
    let left = path_points(&doc, "left");
    assert!((left[1].1 - p.y).abs() < 0.02, "{left:?} in {p:?}");
}

#[test]
fn golden_grid() {
    assert_golden("grid", &render(&spec_text("grid")));
}

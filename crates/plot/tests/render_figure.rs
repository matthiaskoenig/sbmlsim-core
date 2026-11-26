//! The composition of a figure: the grid of plots with its spans, the title band, the legends
//! of a plot and of a row inside and outside, and the right y axis.
//!
//! The geometry is read back from the SVG: a plot is a nested `<svg class="sbmlsim-plot">` at
//! its place in the figure, its panel the clip rectangle of its first `charton-plot` group, and
//! a legend a `<g class="sbmlsim-legend">` with its frame `rect.sbmlsim-legend-frame` and an
//! entry group `g.sbmlsim-legend-entry` per entry, all in the pixels of the figure.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{geometry, parse, render_svg, text_width};
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

/// The numbers as a JSON list.
fn list(values: impl IntoIterator<Item = f64>) -> String {
    let v: Vec<String> = values.into_iter().map(|a| format!("{a}")).collect();
    format!("[{}]", v.join(", "))
}

/// Whether a polyline of the figure meets a rectangle, sampled every half pixel.
fn polyline_meets(points: &[(f64, f64)], r: &Rect) -> bool {
    points.windows(2).any(|w| {
        let ((ax, ay), (bx, by)) = (w[0], w[1]);
        let n = (((bx - ax).hypot(by - ay)) * 2.0).ceil().max(1.0) as usize;
        (0..=n).any(|k| {
            let t = k as f64 / n as f64;
            let (x, y) = (ax + t * (bx - ax), ay + t * (by - ay));
            x > r.x && x < r.right() && y > r.y && y < r.bottom()
        })
    })
}

#[test]
fn legend_avoids_a_band_interior() {
    // a band fills the right half of the panel from its bottom to its top, its edges on the
    // edges of the panel; a line crosses the upper left corner; the lower left one is free
    let x = list((0..=20).map(|i| 5.0 + f64::from(i) * 0.25));
    let band = format!(
        r#"{{"id": "band", "kind": "band", "label": "range", "x": {x}, "low": {}, "high": {}}}"#,
        list((0..=20).map(|_| 0.0)),
        list((0..=20).map(|_| 10.0)),
    );
    let line = curve("line", r#""label": "mean","#, "[0, 1.5, 3]", "[10, 8, 6]");
    let p = format!(
        r#"{{"id": "p", "row": 0, "column": 0,
            "x": {{"label": "time", "min": 0, "max": 10}},
            "y": {{"label": "c", "min": 0, "max": 10}},
            "elements": [{band}, {line}]}}"#
    );
    let svg = render(&figure(
        500.0,
        400.0,
        r#""legend": {"position": "inside"},"#,
        1,
        1,
        &[p],
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    assert!(p.holds(&frame), "{frame:?} not in {p:?}");
    // the lower left corner
    assert!(frame.right() < p.x + p.width / 2.0, "{frame:?} in {p:?}");
    assert!(frame.y > p.y + p.height / 2.0, "{frame:?} in {p:?}");
}

#[test]
fn legend_avoids_dense_curves() {
    // the shape of the HCTZ panels: curves rise from the lower left and fill the upper right,
    // data points of the highest one with error bars reach up into the upper left corner; the
    // lower right corner, below the plateaus, is free
    let t: Vec<f64> = (0..=96).map(|i| f64::from(i) * 0.25).collect();
    let mut elements = Vec::new();
    for (k, a) in [8.0f64, 9.0, 10.0].into_iter().enumerate() {
        let y = t.iter().map(|v| a * (1.0 - (-0.15 * v).exp()));
        elements.push(curve(
            &format!("dose{k}"),
            &format!(r#""label": "dose {a} mg","#),
            &list(t.iter().copied()),
            &list(y),
        ));
    }
    let td: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let yd = td.map(|v| 10.0 * (1.0 - (-0.15 * v).exp()));
    elements.push(format!(
        r#"{{"id": "data", "kind": "curve", "label": "study", "style": {{"line": {{"type": "none"}},
            "marker": {{"type": "circle", "size": 6}}}}, "x": {}, "y": {}, "yerr": {}}}"#,
        list(td),
        list(yd),
        list(td.map(|_| 5.0)),
    ));
    let p = format!(
        r#"{{"id": "p", "row": 0, "column": 0,
            "x": {{"label": "time", "unit": "hr", "min": 0, "max": 24}},
            "y": {{"label": "urine", "unit": "mg", "min": 0, "max": 11}},
            "elements": [{}]}}"#,
        elements.join(", ")
    );
    let svg = render(&figure(
        500.0,
        400.0,
        r#""legend": {"position": "inside"},"#,
        1,
        1,
        &[p],
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    assert!(p.holds(&frame), "{frame:?} not in {p:?}");
    // the lower right corner, where no curve passes
    assert!(frame.x > p.x + p.width / 2.0, "{frame:?} in {p:?}");
    assert!(frame.y > p.y + p.height / 2.0, "{frame:?} in {p:?}");
    for k in 0..3 {
        let line = path_points(&doc, &format!("dose{k}"));
        assert!(!polyline_meets(&line, &frame), "dose{k} under {frame:?}");
    }
}

#[test]
fn crowded_panel_keeps_legend_inside() {
    // lines every tenth of the height cross every location; the legend stays inside at the
    // location of the least data, as matplotlib's `best` does (before 0.30.0 it went outside)
    let mut elements: Vec<String> = (0..=10)
        .map(|k| {
            let v = f64::from(k);
            curve(&format!("l{k}"), "", "[0, 10]", &format!("[{v}, {v}]"))
        })
        .collect();
    elements.push(curve("named", r#""label": "dose","#, "[0, 10]", "[0, 10]"));
    let svg = render(&figure(
        500.0,
        400.0,
        r#""legend": {"position": "inside"},"#,
        1,
        1,
        &[plot("p", 0, 0, "", &elements)],
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    assert!(p.holds(&frame), "{frame:?} not in {p:?}");
    assert_eq!(figure_size(&doc).0, 500.0);
}

#[test]
fn legend_too_large_for_its_panel_goes_outside() {
    let elements: Vec<String> = (0..40)
        .map(|k| {
            curve(
                &format!("c{k}"),
                &format!(r#""label": "dose {k}","#),
                "[0, 1]",
                "[0, 1]",
            )
        })
        .collect();
    let svg = render(&one_plot(r#"{"position": "inside"}"#, &elements));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    assert!(frame.x > p.right(), "{frame:?} not right of {p:?}");
}

/// The frame of the legend of a plot of a rising line, the legend at `loc` of the figure and
/// the plot setting `extra`, with its panel.
fn located(loc: &str, extra: &str) -> (Rect, Rect) {
    let elements = [curve(
        "rise",
        r#""label": "rising line","#,
        "[0, 1]",
        "[0, 1]",
    )];
    let svg = render(&figure(
        500.0,
        400.0,
        &format!(r#""legend": {{"position": "inside", "loc": "{loc}"}},"#),
        1,
        1,
        &[plot("p", 0, 0, extra, &elements)],
    ));
    let doc = Document::parse(&svg).unwrap();
    (legends(&doc)[0].frame, panel(&doc, "p"))
}

#[test]
fn legend_loc_is_respected() {
    // every location of matplotlib puts the legend there, data under it or not
    let close = |a: f64, b: f64| (a - b).abs() < 1.0;
    for loc in [
        "upper_right",
        "upper_left",
        "lower_left",
        "lower_right",
        "right",
        "center_left",
        "center_right",
        "lower_center",
        "upper_center",
        "center",
    ] {
        let (f, p) = located(loc, "");
        assert!(p.holds(&f), "{loc}: {f:?} not in {p:?}");
        let (cx, cy) = (f.x + f.width / 2.0, f.y + f.height / 2.0);
        let (px, py) = (p.x + p.width / 2.0, p.y + p.height / 2.0);
        let horizontal = if loc.ends_with("left") {
            f.x < px - f.width / 2.0 - 1.0
        } else if loc.ends_with("right") {
            f.right() > px + f.width / 2.0 + 1.0
        } else {
            close(cx, px)
        };
        let vertical = if loc.starts_with("upper") {
            f.bottom() < py
        } else if loc.starts_with("lower") {
            f.y > py
        } else {
            close(cy, py)
        };
        assert!(horizontal && vertical, "{loc}: {f:?} in {p:?}");
    }
    // the rising line runs through the lower left and the upper right corner: an explicit
    // location keeps the legend on the data
    let (f, p) = located("lower_left", "");
    assert!(
        f.x < p.x + 10.0 && f.bottom() > p.bottom() - 10.0,
        "{f:?} in {p:?}"
    );
}

#[test]
fn plot_legend_loc_overrides_the_figure() {
    let (f, p) = located("upper_right", r#""legend_loc": "lower_center","#);
    assert!(f.y > p.y + p.height / 2.0, "{f:?} in {p:?}");
    assert!(
        ((f.x + f.width / 2.0) - (p.x + p.width / 2.0)).abs() < 1.0,
        "{f:?} in {p:?}"
    );
}

#[test]
fn best_takes_the_order_of_matplotlib_on_a_tie() {
    // a line through the middle: the four corners are free, and the upper right one is the
    // first location of matplotlib
    let elements = [curve("flat", r#""label": "flat","#, "[0, 1]", "[0.5, 0.5]")];
    let svg = render(&one_plot(r#"{"position": "inside"}"#, &elements));
    let doc = Document::parse(&svg).unwrap();
    let (f, p) = (legends(&doc)[0].frame, panel(&doc, "p"));
    assert!(
        f.right() > p.right() - 10.0 && f.y < p.y + 10.0,
        "{f:?} in {p:?}"
    );
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
        // the elements of a group in one colour (a group of several colours has a neutral
        // swatch, `render_webapp::mixed_colour_group_has_a_neutral_swatch`)
        curve(
            "first",
            r##""legend": "run 1", "style": {"color": "#2a6f97"},"##,
            "[0, 1]",
            "[0, 1]",
        ),
        curve(
            "second",
            r##""label": "run 1", "legend": "run 1",
                "style": {"color": "#2a6f97", "line": {"type": "dashed", "width": 3}},"##,
            "[0, 1]",
            "[1, 2]",
        ),
        curve(
            "third",
            r##""label": "other", "legend": "run 1", "style": {"color": "#2a6f97"},"##,
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
    for name in ["stroke", "stroke-width", "stroke-dasharray"] {
        assert_eq!(swatch.attribute(name), second.attribute(name), "{name}");
    }
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
fn tall_legend_grows_its_row() {
    let labels: Vec<String> = (1..=11)
        .map(|k| {
            curve(
                &format!("{{p}}_{k}"),
                &format!(r#""label": "dose {k}","#),
                "[0, 1]",
                "[0, 1]",
            )
        })
        .collect();
    let of = |p: &str| -> Vec<String> { labels.iter().map(|e| e.replace("{p}", p)).collect() };
    let json = figure(
        500.0,
        320.0,
        r#""legend": {"position": "outside"}, "fonts": {"legend": 10},"#,
        2,
        1,
        &[plot("a", 0, 0, "", &of("a")), plot("b", 1, 0, "", &of("b"))],
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let found = legends(&doc);
    assert_eq!(found.len(), 2);
    let (a, b) = (panel(&doc, "a"), panel(&doc, "b"));
    let (la, lb) = (found[0].frame, found[1].frame);
    assert!(
        la.height > a.height,
        "the legend {la:?} is not taller than its panel {a:?}"
    );
    for (what, r) in [("panel", b), ("legend", lb)] {
        assert!(
            !la.overlaps(&r),
            "the legend of row 0 {la:?} overlaps the {what} of row 1 {r:?}"
        );
    }
    // nothing of the plot of row 1 reaches up into the legend of row 0
    for t in plot_node(&doc, "b")
        .descendants()
        .filter(|n| n.has_tag_name("text"))
    {
        let (_, dy) = offset(t);
        assert!(
            number(t, "y") + dy > la.bottom(),
            "{:?} above {la:?}",
            t.text()
        );
    }
    let (_, h) = figure_size(&doc);
    assert!(h > 320.0, "the figure keeps its height {h}");
    assert!(lb.bottom() <= h, "{lb:?} below the figure of height {h}");
    // the panels keep their size
    assert!((a.height - b.height).abs() < 0.01, "{a:?} {b:?}");
}

#[test]
fn legend_too_large_inside_goes_outside() {
    let labels: Vec<String> = (1..=12)
        .map(|k| {
            curve(
                &format!("c{k}"),
                &format!(r#""label": "dose {k}","#),
                "[0, 1]",
                "[0, 1]",
            )
        })
        .collect();
    let svg = render(&figure(
        400.0,
        250.0,
        r#""legend": {"position": "inside"},"#,
        1,
        1,
        &[plot("p", 0, 0, "", &labels)],
    ));
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let frame = legends(&doc)[0].frame;
    let (w, _) = figure_size(&doc);
    assert!(frame.x > p.right(), "{frame:?} not right of {p:?}");
    assert!(
        w > 400.0 && frame.right() <= w,
        "{frame:?} in a figure of width {w}"
    );
}

#[test]
fn swatch_shows_error_bars() {
    let svg = render(&one_plot(
        r#"{"position": "outside"}"#,
        &[curve(
            "data",
            r#""label": "data", "yerr": [0.1, 0.1], "style": {"line": {"type": "none"}, "marker": {"type": "circle"}},"#,
            "[0, 1]",
            "[1, 2]",
        )],
    ));
    let doc = Document::parse(&svg).unwrap();
    let swatch = doc
        .descendants()
        .find(|n| has_class(*n, "sbmlsim-legend-swatch"))
        .expect("a swatch");
    let vertical = swatch
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .any(|n| {
            let d = n.attribute("d").unwrap();
            let pts: Vec<f64> = d
                .split(['M', 'L', ' '])
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            pts.len() == 4 && pts[0] == pts[2] && pts[1] != pts[3]
        });
    assert!(vertical, "no error bar in the swatch");
    assert!(
        swatch.descendants().any(|n| n.has_tag_name("circle")),
        "no marker in the swatch"
    );
}

#[test]
fn rotated_right_axis_labels_clear_the_ticks() {
    // a label rotated by 90 degrees about the middle of its start reaches half its line height
    // back towards the axis; it starts that much further out, past the tick and its padding
    let json = r#"{"contract": "figure/1", "width": 500, "height": 300,
        "grid": {"rows": 1, "columns": 1},
        "plots": [{"id": "p", "row": 0, "column": 0, "x": {}, "y": {},
            "y2": {"label": "amount", "tick_rotation": 90},
            "elements": [{"id": "r", "kind": "curve", "axis": "y2", "x": [0, 1], "y": [0, 100]}]}]}"#;
    let svg = render(json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc, "p");
    let ticks = axis_texts(&doc, "p", "y2");
    assert!(!ticks.is_empty());
    for (text, x, _) in ticks {
        // tick 5, padding 3, half the tick font of 11
        assert!(
            (x - (p.right() + 5.0 + 3.0 + 5.5)).abs() < 0.02,
            "{text} at {x}, panel {p:?}"
        );
    }
}

#[test]
fn golden_grid() {
    assert_golden("grid", &render(&spec_text("grid")));
}

#[test]
fn golden_legends() {
    assert_golden("legends", &render(&spec_text("legends")));
}

/// The fields of 0.28.0: font weights, the y tick size, hollow markers, texts of several
/// lines, outlined bars, tick marks without labels and the swatch of a legend group.
#[test]
fn golden_styles() {
    assert_golden("styles", &render(&spec_text("styles")));
}

/// The colour bars of 0.28.0: start and end of a scale, ticks and their labels, categorical
/// bands, bars stacked beside an outside legend and a right axis, a log bar with start and end.
#[test]
fn golden_v028() {
    assert_golden("v028", &render(&spec_text("v028")));
}

/// The lines of a text: the texts of its `tspan`s with their `y` in the text, or its own text.
fn text_lines(n: Node) -> Vec<(String, f64)> {
    let spans: Vec<Node> = n.children().filter(|c| c.has_tag_name("tspan")).collect();
    if spans.is_empty() {
        return vec![(n.text().unwrap_or_default().to_string(), number(n, "y"))];
    }
    spans
        .iter()
        .map(|s| (s.text().unwrap_or_default().to_string(), number(*s, "y")))
        .collect()
}

#[test]
fn multi_line_legend_entry_grows_the_legend() {
    let elements = |second: &str| {
        [
            curve("one", r#""label": "dose 1","#, "[0, 1, 2]", "[0, 2, 1]"),
            curve(
                "two",
                &format!(r#""label": "{second}","#),
                "[0, 1, 2]",
                "[0, 1, 0.5]",
            ),
            curve("three", r#""label": "dose 3","#, "[0, 1, 2]", "[1, 1, 1]"),
        ]
    };
    let one = one_plot(r#"{"position": "outside"}"#, &elements("dose 2"));
    let two = one_plot(
        r#"{"position": "outside"}"#,
        &elements(r"dose 2\nwith a long second line"),
    );
    let (s1, s2) = (render(&one), render(&two));
    let (d1, d2) = (Document::parse(&s1).unwrap(), Document::parse(&s2).unwrap());
    let (l1, l2) = (legends(&d1)[0].frame, legends(&d2)[0].frame);
    // one more line of the legend font: 1.2 times 11 pixels taller, and as wide as the widest
    // line
    assert!((l2.height - l1.height - 13.2).abs() < 0.02, "{l1:?} {l2:?}");
    assert!(
        (l2.width
            - l1.width
            - (text_width("with a long second line", 11.0) - text_width("dose 3", 11.0)))
        .abs()
            < 0.02,
        "{l1:?} {l2:?}"
    );
    // the entries of the geometry: the second one taller, the third one below it
    let g = geometry(&parse(&two).unwrap());
    let e = &g.legend_entries;
    assert_eq!(e.len(), 3);
    assert!((e[1].rect.height - e[0].rect.height - 13.2).abs() < 1e-9);
    assert!((e[2].rect.y - (e[1].rect.y + e[1].rect.height)).abs() < 1e-9);
    assert!((e[2].rect.height - e[0].rect.height).abs() < 1e-9);
    // the label: two lines centred on its entry, the swatch at the middle of the entry
    let entry = d2
        .descendants()
        .filter(|n| has_class(*n, "sbmlsim-legend-entry"))
        .nth(1)
        .unwrap();
    let label = entry
        .descendants()
        .find(|n| has_class(*n, "sbmlsim-legend-label"))
        .unwrap();
    let lines = text_lines(label);
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["dose 2", "with a long second line"]
    );
    let middle = e[1].rect.y + e[1].rect.height / 2.0;
    assert!(((lines[0].1 + lines[1].1) / 2.0 - middle).abs() < 0.02);
    assert!((lines[1].1 - lines[0].1 - 13.2).abs() < 0.02);
    let swatch = entry
        .descendants()
        .find(|n| has_class(*n, "sbmlsim-legend-swatch"))
        .unwrap();
    let transform = swatch.attribute("transform").unwrap();
    let cy: f64 = transform
        .trim_end_matches(')')
        .rsplit(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!((cy - middle).abs() < 0.02, "{transform} {middle}");
}

#[test]
fn outside_legend_with_multi_line_entries_keeps_rows_apart() {
    // six entries of three lines: a legend taller than its row, which grows the row
    let labels: Vec<String> = (1..=6)
        .map(|k| {
            curve(
                &format!("{{p}}_{k}"),
                &format!(r#""label": "dose {k}\nof the study\nin mg","#),
                "[0, 1]",
                "[0, 1]",
            )
        })
        .collect();
    let of = |p: &str| -> Vec<String> { labels.iter().map(|e| e.replace("{p}", p)).collect() };
    for scope in ["plot", "row"] {
        let json = figure(
            500.0,
            320.0,
            &format!(r#""legend": {{"position": "outside", "scope": "{scope}"}},"#),
            2,
            1,
            &[plot("a", 0, 0, "", &of("a")), plot("b", 1, 0, "", &of("b"))],
        );
        let svg = render(&json);
        let doc = Document::parse(&svg).unwrap();
        let found = legends(&doc);
        assert_eq!(found.len(), 2, "{scope}");
        let (a, b) = (panel(&doc, "a"), panel(&doc, "b"));
        let (la, lb) = (found[0].frame, found[1].frame);
        // six entries of three lines each
        assert!(
            (la.height - (12.0 + 6.0 * (1.6 * 11.0 + 2.0 * 13.2))).abs() < 0.02,
            "{scope}: {la:?}"
        );
        assert!(la.height > a.height, "{scope}: {la:?} {a:?}");
        for (what, r) in [("panel", b), ("legend", lb)] {
            assert!(
                !la.overlaps(&r),
                "{scope}: the legend of row 0 {la:?} overlaps the {what} of row 1 {r:?}"
            );
        }
        for t in plot_node(&doc, "b")
            .descendants()
            .filter(|n| n.has_tag_name("text"))
        {
            let (_, dy) = offset(t);
            assert!(number(t, "y") + dy > la.bottom(), "{scope}: {:?}", t.text());
        }
        let (_, h) = figure_size(&doc);
        assert!(
            lb.bottom() <= h,
            "{scope}: {lb:?} below the figure of height {h}"
        );
    }
}

#[test]
fn legend_swatches() {
    // the lines of a scan in one legend group: mixed colours, drawn neutral without a swatch
    // of their own; the swatch of the group in the middle colour of the map with it
    let lines: Vec<String> = ["#440154", "#21918c", "#fde725"]
        .iter()
        .enumerate()
        .map(|(k, c)| {
            curve(
                &format!("s{k}"),
                &format!(r#""label": "scan", "legend": "scan", "style": {{"color": "{c}"}},"#),
                "[0, 1]",
                &format!("[{k}, {}]", k + 1),
            )
        })
        .collect();
    let swatch_paint = |json: &str| {
        let svg = render(json);
        let doc = Document::parse(&svg).unwrap();
        let swatch = doc
            .descendants()
            .find(|n| has_class(*n, "sbmlsim-legend-swatch"))
            .expect("a swatch");
        let path = swatch
            .descendants()
            .find(|n| n.has_tag_name("path"))
            .map(|p| {
                (
                    p.attribute("stroke").unwrap().to_string(),
                    p.attribute("stroke-width").unwrap().to_string(),
                    p.attribute("stroke-dasharray").is_some(),
                )
            });
        let marker = swatch
            .descendants()
            .find(|n| n.has_tag_name("circle"))
            .map(|c| c.attribute("fill").unwrap().to_string());
        (path, marker)
    };
    let neutral = swatch_paint(&one_plot(r#"{"position": "outside"}"#, &lines));
    assert_eq!(neutral.0.unwrap().0, "#3b424d");
    let styled = figure(
        500.0,
        400.0,
        r#""legend": {"position": "outside"},"#,
        1,
        1,
        &[plot(
            "p",
            0,
            0,
            r##""legend_swatches": {"scan": {"color": "#21918c",
                "line": {"type": "dashed", "width": 3}, "marker": {"type": "circle"}}},"##,
            &lines,
        )],
    );
    let (path, marker) = swatch_paint(&styled);
    assert_eq!(path, Some(("#21918c".to_string(), "3".to_string(), true)));
    assert_eq!(marker.as_deref(), Some("#21918c"));
    // a key which is no legend group of the plot is refused with its path
    let wrong = styled.replace(
        "\"legend_swatches\": {\"scan\"",
        "\"legend_swatches\": {\"scans\"",
    );
    let e = parse(&wrong).unwrap_err().to_string();
    assert!(e.starts_with("plots[0].legend_swatches.scans:"), "{e}");
}

#[test]
fn titles_of_lines_and_weights() {
    // the figure title: bold by default, normal with its weight; two lines grow its band
    let plain = spanning_grid(r#""title": "Figure title","#);
    let two = spanning_grid(
        r#""title": "Figure title\nand its subtitle", "fonts": {"title_weight": "normal"},"#,
    );
    let (s1, s2) = (render(&plain), render(&two));
    let (d1, d2) = (Document::parse(&s1).unwrap(), Document::parse(&s2).unwrap());
    let title = |doc: &Document| {
        doc.descendants()
            .find(|n| n.has_tag_name("g") && has_class(*n, "sbmlsim-figure-title"))
            .and_then(|g| g.descendants().find(|n| n.has_tag_name("text")))
            .map(|t| {
                (
                    t.attribute("font-weight").map(str::to_string),
                    text_lines(t),
                )
            })
            .unwrap()
    };
    let (t1, t2) = (title(&d1), title(&d2));
    assert_eq!(t1.0.as_deref(), Some("bold"));
    assert_eq!(t2.0, None);
    assert_eq!(t2.1.len(), 2);
    // the band: twice the font plus a line of 1.2 times 16 pixels, the lines centred in it
    let band = 2.0 * 16.0 + 19.2;
    assert!(((t2.1[0].1 + t2.1[1].1) / 2.0 - band / 2.0).abs() < 0.02);
    assert!((t2.1[1].1 - t2.1[0].1 - 19.2).abs() < 0.02);
    let (a1, a2) = (panel(&d1, "a"), panel(&d2, "a"));
    assert!((a2.y - a1.y - 19.2).abs() < 0.02, "{a1:?} {a2:?}");
    // a plot title of two lines: the last line where the one line was, the first above it,
    // the panel a line lower; its weight is the title weight
    let plot_title = |doc: &Document, id: &str| {
        plot_node(doc, id)
            .descendants()
            .find(|n| has_class(*n, "charton-title"))
            .and_then(|g| g.descendants().find(|n| n.has_tag_name("text")))
            .map(|t| {
                let (_, dy) = offset(t);
                (
                    t.attribute("font-weight").map(str::to_string),
                    text_lines(t)
                        .into_iter()
                        .map(|(l, y)| (l, y + dy))
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap()
    };
    let single = figure(
        500.0,
        400.0,
        "",
        1,
        1,
        &[plot(
            "p",
            0,
            0,
            r#""title": "plasma","#,
            &[curve("c", "", "[0, 1]", "[0, 1]")],
        )],
    );
    let double = single.replace(
        "\"title\": \"plasma\"",
        "\"title\": \"plasma\\nof the arm\"",
    );
    let normal = double.replacen(
        "\"grid\"",
        r#""fonts": {"title_weight": "normal"}, "grid""#,
        1,
    );
    let (s1, s2, s3) = (render(&single), render(&double), render(&normal));
    let (d1, d2, d3) = (
        Document::parse(&s1).unwrap(),
        Document::parse(&s2).unwrap(),
        Document::parse(&s3).unwrap(),
    );
    let (p1, p2) = (panel(&d1, "p"), panel(&d2, "p"));
    let lh = 1.2 * 13.0;
    assert!((p2.y - p1.y - lh).abs() < 0.02, "{p1:?} {p2:?}");
    let (t1, t2, t3) = (
        plot_title(&d1, "p"),
        plot_title(&d2, "p"),
        plot_title(&d3, "p"),
    );
    assert_eq!(t1.0.as_deref(), Some("bold"));
    assert_eq!(t2.0.as_deref(), Some("bold"));
    assert_eq!(t3.0, None);
    assert_eq!(t2.1.len(), 2);
    // the distance of the last line to the panel is the one of the one line
    assert!(((p2.y - t2.1[1].1) - (p1.y - t1.1[0].1)).abs() < 0.02);
    assert!((t2.1[1].1 - t2.1[0].1 - lh).abs() < 0.02);
}

#[test]
fn right_axis_of_lines_marks_and_weights() {
    let json = r##"{"contract": "figure/1", "width": 600, "height": 400,
        "grid": {"rows": 1, "columns": 1},
        "plots": [{"id": "p", "row": 0, "column": 0,
            "x": {"label": "time", "min": 0, "max": 10},
            "y": {"label": "c", "min": 0, "max": 1},
            "y2": {"label": "amount", "unit": "mg", "min": 0, "max": 1000},
            "elements": [
              {"id": "left", "kind": "curve", "label": "left", "x": [0, 10], "y": [0, 1]},
              {"id": "right", "kind": "curve", "axis": "y2", "x": [0, 10], "y": [0, 1000]}
            ]}]}"##;
    let two = json.replace("\"label\": \"amount\"", "\"label\": \"amount\\nin urine\"");
    let styled = two
        .replace(
            "\"unit\": \"mg\",",
            "\"unit\": \"mg\", \"ticks_visible\": false, \"tick_marks_visible\": true,",
        )
        .replacen(
            "\"grid\"",
            r#""fonts": {"label_weight": "bold", "legend_weight": "bold", "tick_y": 9}, "grid""#,
            1,
        );
    let (s1, s2, s3) = (render(json), render(&two), render(&styled));
    let (d1, d2, d3) = (
        Document::parse(&s1).unwrap(),
        Document::parse(&s2).unwrap(),
        Document::parse(&s3).unwrap(),
    );
    let (p1, p2) = (panel(&d1, "p"), panel(&d2, "p"));
    // a second line of the title takes 1.2 times 13 pixels more right of the panel
    assert!(
        ((p1.right() - p2.right()) - 15.6).abs() < 0.02,
        "{p1:?} {p2:?}"
    );
    let title = |doc: &Document| {
        plot_node(doc, "p")
            .descendants()
            .filter(|n| {
                n.ancestors()
                    .any(|a| a.attribute("data-axis") == Some("y2"))
                    && n.ancestors().any(|a| has_class(a, "charton-axis-title"))
            })
            .find(|n| n.has_tag_name("text"))
            .map(|t| {
                (
                    number(t, "y"),
                    t.attribute("font-weight").map(str::to_string),
                    text_lines(t),
                )
            })
            .unwrap()
    };
    // the first line next to the axis, the second further out: lower in the rotated text
    let (y, weight, lines) = title(&d2);
    assert_eq!(weight, None);
    assert_eq!(lines.len(), 2);
    assert!((lines[0].1 - y).abs() < 0.02 && (lines[1].1 - y - 15.6).abs() < 0.02);
    assert_eq!(title(&d3).1.as_deref(), Some("bold"));
    // marks without labels on the right axis, the room of the marks alone
    assert!(axis_texts(&d3, "p", "y2").is_empty());
    let marks = plot_node(&d3, "p")
        .descendants()
        .filter(|n| {
            n.has_tag_name("path")
                && n.ancestors()
                    .any(|a| a.attribute("data-axis") == Some("y2"))
                && n.ancestors().any(|a| has_class(a, "charton-tick"))
        })
        .count();
    assert!(marks >= 3);
    // the left tick labels of the y size and a bold legend
    let y_ticks: Vec<Node> = plot_node(&d3, "p")
        .descendants()
        .filter(|n| {
            n.has_tag_name("text")
                && n.ancestors().any(|a| a.attribute("data-axis") == Some("y"))
                && n.ancestors().any(|a| has_class(a, "charton-tick-label"))
        })
        .collect();
    assert!(!y_ticks.is_empty());
    assert!(
        y_ticks
            .iter()
            .all(|n| n.attribute("font-size") == Some("9"))
    );
    let label = d3
        .descendants()
        .find(|n| has_class(*n, "sbmlsim-legend-label"))
        .unwrap();
    assert_eq!(label.attribute("font-weight"), Some("bold"));
    let (l2, l3) = (legends(&d2)[0].frame, legends(&d3)[0].frame);
    assert!(l3.width > l2.width, "{l2:?} {l3:?}");
}

/// A figure of one plot with a heatmap whose row labels and cell text are given.
fn heatmap_figure(row: &str, cell: &str) -> String {
    let heat = format!(
        r#"{{"id": "h", "kind": "heatmap", "rows": ["{row}"], "columns": ["x", "y"],
            "values": [[1, 2]], "text": [["{cell}", "{cell}"]],
            "scale": {{"name": "viridis", "low": 1, "high": 2}}}}"#
    );
    figure(
        160.0,
        300.0,
        "",
        1,
        1,
        &[format!(
            r#"{{"id": "p", "row": 0, "column": 0, "x": {{}}, "y": {{}}, "elements": [{heat}]}}"#
        )],
    )
}

/// The font sizes of the texts of a figure, in the order of the document.
fn font_sizes(svg: &str) -> Vec<String> {
    let doc = Document::parse(svg).unwrap();
    doc.descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter_map(|n| n.attribute("font-size").map(str::to_string))
        .collect()
}

/// A tick label of several lines (since 0.31.0) is as wide as its widest line, its lines are
/// written as `<tspan>`s; a cell text is one line in SVG, so a newline in it is measured as the
/// space it renders as.
#[test]
fn newline_in_a_tick_label_breaks_it_and_in_a_cell_text_is_one_line() {
    let long = "wide label here";
    let broken = render(&heatmap_figure(&long.replace(' ', "\\n"), "ww ww"));
    let spaced = render(&heatmap_figure(long, "ww ww"));
    let widest = render(&heatmap_figure("wide", "ww ww"));
    let (a, b, c) = (
        Document::parse(&broken).unwrap(),
        Document::parse(&spaced).unwrap(),
        Document::parse(&widest).unwrap(),
    );
    // the left axis is as wide as the widest line
    assert!(panel(&a, "p").x < panel(&b, "p").x);
    assert_eq!(panel(&a, "p").x, panel(&c, "p").x);
    let lines: Vec<&str> = a
        .descendants()
        .filter(|n| n.has_tag_name("tspan"))
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(lines, ["wide", "label", "here"]);
    // the cell text of one size whether its words are broken or not
    let cell = |c: &str| font_sizes(&render(&heatmap_figure("r", c)));
    assert_eq!(cell("wwwww\\nwwwww"), cell("wwwww wwwww"));
    assert_ne!(cell("wwwww\\nwwwww"), cell("wwwww"));
}

/// The panel of a figure with a category axis rotated to the vertical, whose label is given.
fn category_panel(label: &str) -> Rect {
    let bar = r#"{"id": "b", "kind": "bar", "x": [0, 1], "height": [1, 2]}"#;
    let plot = format!(
        r#"{{"id": "p", "row": 0, "column": 0,
            "x": {{"scale": "category", "categories": ["{label}", "b"], "tick_rotation": 90}},
            "y": {{}}, "elements": [{bar}]}}"#
    );
    let svg = render(&figure(400.0, 300.0, "", 1, 1, &[plot]));
    panel(&Document::parse(&svg).unwrap(), "p")
}

#[test]
fn newline_in_a_category_label_reserves_its_widest_line() {
    let broken = category_panel("a long label\\nshort");
    assert_eq!(broken, category_panel("a long label"));
    assert!(broken.height > category_panel("a long label short").height);
}

/// A categorical colour bar with labels and no ticks has a tick at the middle of every band.
#[test]
fn categorical_bar_labels_without_ticks() {
    let bar = r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 2},
        "categorical": true, "tick_labels": ["lo", "mid", "hi"]},"#;
    let svg = render(&figure(
        400.0,
        300.0,
        "",
        1,
        1,
        &[plot("p", 0, 0, bar, &[curve("c", "", "[0, 1]", "[0, 1]")])],
    ));
    let doc = Document::parse(&svg).unwrap();
    let labels = doc
        .descendants()
        .find(|n| has_class(*n, "sbmlsim-colorbar-tick-labels"))
        .unwrap();
    let texts: Vec<_> = labels.children().filter_map(|n| n.text()).collect();
    assert_eq!(texts, ["lo", "mid", "hi"]);
}

#[test]
fn row_legend_follows_the_legend_order_of_its_plots() {
    let one = curve("a_one", r#""label": "one","#, "[0, 1]", "[1, 0]");
    let two = curve("b_two", r#""label": "two","#, "[0, 1]", "[0, 1]");
    let three = curve("c_three", r#""label": "three","#, "[0, 1]", "[0.5, 0.5]");
    let json = figure(
        900.0,
        300.0,
        r#""legend": {"scope": "row", "position": "outside"},"#,
        1,
        3,
        &[
            plot("a", 0, 0, "", &[one]),
            plot("b", 0, 1, r#""legend_order": ["b_two"],"#, &[two]),
            plot(
                "c",
                0,
                2,
                r#""legend_order": ["c_three", "b_two"],"#,
                &[three],
            ),
        ],
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let found = legends(&doc);
    assert_eq!(found.len(), 1);
    let groups: Vec<&str> = found[0].entries.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(groups, ["b_two", "c_three", "a_one"]);
}

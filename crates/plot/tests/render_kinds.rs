//! The rendering of bars, boxes, violins, histograms, heatmaps, polygons and the colour
//! scales with their colour bar.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{
    COLOR_SCALES, ColorScale, ColorScaleName, NO_VALUE_COLOR, color_scale, parse, render_svg,
};
use support::{assert_golden, spec_text, used_markers};

/// Renders a figure of JSON text.
fn render(json: &str) -> String {
    render_svg(&parse(json).unwrap_or_else(|e| panic!("{e}")))
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

/// A scale of a name between two values.
fn scale(name: ColorScaleName, low: f64, high: f64) -> ColorScale {
    ColorScale {
        name: Some(name),
        stops: None,
        low,
        high,
        center: None,
        log: false,
        gamma: None,
        start: 0.0,
        end: 1.0,
    }
}

/// The one group of an element.
fn element_group<'a>(doc: &'a Document<'a>, id: &str) -> Node<'a, 'a> {
    let groups: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("g") && n.attribute("data-element") == Some(id))
        .collect();
    assert_eq!(groups.len(), 1, "groups of element {id}");
    groups[0]
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// The panel of the plot a node is drawn in: the clip rectangle of its plot group, the node
/// itself or one around it.
fn panel_of(doc: &Document, node: Node) -> Rect {
    let clip = node
        .ancestors()
        .find_map(|a| a.attribute("clip-path"))
        .expect("a clipped plot group");
    let id = clip.trim_start_matches("url(#").trim_end_matches(')');
    let rect = doc
        .descendants()
        .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some(id))
        .and_then(|c| c.children().find(|n| n.has_tag_name("rect")))
        .expect("the clip rectangle");
    let a = |name: &str| rect.attribute(name).unwrap().parse::<f64>().unwrap();
    Rect {
        x: a("x"),
        y: a("y"),
        width: a("width"),
        height: a("height"),
    }
}

/// The panel of the one plot of a drawing, from its first clipped group.
fn panel(doc: &Document) -> Rect {
    let clipped = doc
        .descendants()
        .find(|n| n.attribute("clip-path").is_some())
        .expect("a clipped group");
    panel_of(doc, clipped)
}

/// The pixel of a value on the y axis of a panel whose axis runs linearly from `low` to `high`.
fn y_pixel(p: Rect, low: f64, high: f64, v: f64) -> f64 {
    p.y + p.height * (1.0 - (v - low) / (high - low))
}

/// The pixel of a value on the x axis of a panel whose axis runs linearly from `low` to `high`.
fn x_pixel(p: Rect, low: f64, high: f64, v: f64) -> f64 {
    p.x + p.width * (v - low) / (high - low)
}

/// One subpath of a path: its points and whether it is closed.
#[derive(Debug, Clone)]
struct Subpath {
    points: Vec<(f64, f64)>,
    closed: bool,
}

/// The subpaths of the `d` of a path of `M`, `L` and `Z` commands.
fn subpaths(d: &str) -> Vec<Subpath> {
    let mut out: Vec<Subpath> = Vec::new();
    let mut numbers = String::new();
    let mut command = ' ';
    let flush = |command: char, numbers: &mut String, out: &mut Vec<Subpath>| {
        let v: Vec<f64> = numbers
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        numbers.clear();
        match command {
            'M' => out.push(Subpath {
                points: vec![(v[0], v[1])],
                closed: false,
            }),
            'L' => out.last_mut().unwrap().points.push((v[0], v[1])),
            _ => {}
        }
    };
    for c in d.chars() {
        match c {
            'M' | 'L' | 'Z' => {
                flush(command, &mut numbers, &mut out);
                if c == 'Z' {
                    out.last_mut().unwrap().closed = true;
                }
                command = c;
            }
            c => numbers.push(c),
        }
    }
    flush(command, &mut numbers, &mut out);
    out
}

/// The paths inside a node.
fn paths<'a>(node: Node<'a, 'a>) -> Vec<Node<'a, 'a>> {
    node.descendants()
        .filter(|n| n.has_tag_name("path"))
        .collect()
}

/// The filled paths inside a node: the paths whose fill is a colour.
fn filled<'a>(node: Node<'a, 'a>) -> Vec<Node<'a, 'a>> {
    paths(node)
        .into_iter()
        .filter(|p| p.attribute("fill").is_some_and(|f| f != "none"))
        .collect()
}

/// The open stroked paths inside a node.
fn open_lines<'a>(node: Node<'a, 'a>) -> Vec<Node<'a, 'a>> {
    paths(node)
        .into_iter()
        .filter(|p| p.attribute("fill") == Some("none"))
        .filter(|p| !p.attribute("d").unwrap_or_default().contains('Z'))
        .collect()
}

/// The texts inside a node.
fn texts(node: Node) -> Vec<String> {
    node.descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(|n| n.text().unwrap_or_default().to_string())
        .collect()
}

/// The vertical span of the points of a subpath.
fn y_span(s: &Subpath) -> (f64, f64) {
    let ys = s.points.iter().map(|p| p.1);
    (
        ys.clone().fold(f64::INFINITY, f64::min),
        ys.fold(f64::NEG_INFINITY, f64::max),
    )
}

/// The horizontal span of the points of a subpath.
fn x_span(s: &Subpath) -> (f64, f64) {
    let xs = s.points.iter().map(|p| p.0);
    (
        xs.clone().fold(f64::INFINITY, f64::min),
        xs.fold(f64::NEG_INFINITY, f64::max),
    )
}

/// Whether two pixels agree within the rounding of the drawing.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.05
}

/// Asserts that two pixels agree within the rounding of the drawing.
fn assert_near(a: f64, b: f64, what: &str) {
    assert!(near(a, b), "{what}: {a} is not {b}");
}

#[test]
fn viridis_ends() {
    let s = scale(ColorScaleName::Viridis, 0.0, 1.0);
    assert_eq!(color_scale(&s, &[0.0, 1.0]), ["#440154", "#fde725"]);
    // outside of the scale: its ends; no value: the colour of no value
    assert_eq!(
        color_scale(&s, &[-5.0, 7.0, f64::NAN]),
        ["#440154", "#fde725", NO_VALUE_COLOR]
    );
    for name in ["viridis", "magma", "rdbu", "coolwarm", "greys"] {
        assert!(COLOR_SCALES.contains(&name), "{name}");
    }
    // the ends of every table, matplotlib's
    let ends = [
        (ColorScaleName::Magma, "#000004", "#fcfdbf"),
        (ColorScaleName::Rdbu, "#67001f", "#053061"),
        (ColorScaleName::Coolwarm, "#3b4cc0", "#b40426"),
        (ColorScaleName::Greys, "#ffffff", "#000000"),
    ];
    for (name, low, high) in ends {
        assert_eq!(
            color_scale(&scale(name, 0.0, 1.0), &[0.0, 1.0]),
            [low, high]
        );
    }
}

#[test]
fn diverging_center() {
    let s = ColorScale {
        center: Some(0.0),
        ..scale(ColorScaleName::Rdbu, -1.0, 3.0)
    };
    let plain = scale(ColorScaleName::Rdbu, 0.0, 1.0);
    let middle = color_scale(&plain, &[0.5]);
    // matplotlib's RdBu(0.5)
    assert_eq!(middle, ["#f6f7f7"]);
    assert_eq!(color_scale(&s, &[0.0]), middle);
    assert_eq!(color_scale(&s, &[-1.0, 3.0]), ["#67001f", "#053061"]);
    // the two halves: -0.5 is half way down, 1.5 half way up
    assert_eq!(
        color_scale(&s, &[-0.5, 1.5]),
        color_scale(&plain, &[0.25, 0.75])
    );
}

#[test]
fn log_and_degenerate_scales() {
    let s = ColorScale {
        log: true,
        ..scale(ColorScaleName::Magma, 1.0, 100.0)
    };
    // 10 is half way between 1 and 100 in decades; a value which is not positive has no colour
    assert_eq!(
        color_scale(&s, &[1.0, 10.0, 100.0, 0.0, -1.0]),
        [
            "#000004",
            "#b73779",
            "#fcfdbf",
            NO_VALUE_COLOR,
            NO_VALUE_COLOR
        ]
    );
    // a scale of one value, which a caller of the function may hand in: the middle colour
    let one = scale(ColorScaleName::Viridis, 2.0, 2.0);
    assert_eq!(color_scale(&one, &[2.0, 2.0, 5.0]), ["#21918c"; 3]);
}

#[test]
fn constant_heatmap() {
    let json = figure(
        "{}",
        "{}",
        "",
        r#"{"id": "h", "kind": "heatmap", "rows": ["a", "b", "c"], "columns": ["x", "y"],
            "values": [[2, 2], [2, 2], [2, 2]],
            "scale": {"name": "viridis", "low": 0, "high": 4}}"#,
    );
    let svg = render(&json);
    for bad in ["NaN", "inf"] {
        assert!(!svg.contains(bad), "{bad} in the drawing");
    }
    let doc = Document::parse(&svg).unwrap();
    let cells = filled(element_group(&doc, "h"));
    assert_eq!(cells.len(), 6, "one cell per value");
    let fills: Vec<&str> = cells.iter().filter_map(|c| c.attribute("fill")).collect();
    let middle = color_scale(&scale(ColorScaleName::Viridis, 0.0, 4.0), &[2.0]);
    assert!(fills.iter().all(|f| *f == middle[0]), "{fills:?}");
}

#[test]
fn heatmap_cell_text() {
    let json = figure(
        "{}",
        "{}",
        "",
        r#"{"id": "h", "kind": "heatmap", "rows": ["CL", "Vd"], "columns": ["AUC", "Cmax", "tmax"],
            "values": [[-1, 0, 1], [0.5, null, -0.5]],
            "text": [["-1", "0", "1"], ["0.5", "<n/a>", "-0.5"]],
            "scale": {"name": "rdbu", "low": -1, "high": 1, "center": 0}}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let mut got = texts(element_group(&doc, "h"));
    got.sort();
    let mut want = vec!["-1", "0", "1", "0.5", "<n/a>", "-0.5"];
    want.sort();
    assert_eq!(got, want);
    // the axes are the categories of the heatmap
    let labels: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter(|n| {
            n.ancestors().any(|a| {
                a.attribute("class")
                    .is_some_and(|c| c.contains("charton-tick-label"))
            })
        })
        .map(|n| n.text().unwrap_or_default().to_string())
        .collect();
    for name in ["CL", "Vd", "AUC", "Cmax", "tmax"] {
        assert!(labels.iter().any(|l| l == name), "{name} in {labels:?}");
    }
}

#[test]
fn heatmap_colors_by_position() {
    let values = [[-1.0, 0.0, 1.0], [0.5, f64::NAN, -0.5]];
    let json = figure(
        "{}",
        "{}",
        "",
        r#"{"id": "h", "kind": "heatmap", "rows": ["r0", "r1"], "columns": ["c0", "c1", "c2"],
            "values": [[-1, 0, 1], [0.5, null, -0.5]],
            "scale": {"name": "rdbu", "low": -1, "high": 1, "center": 0}}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let group = element_group(&doc, "h");
    let p = panel_of(&doc, group);
    let s = ColorScale {
        center: Some(0.0),
        ..scale(ColorScaleName::Rdbu, -1.0, 1.0)
    };
    let cells = filled(group);
    assert_eq!(cells.len(), 6, "every cell, the one without a value too");
    for cell in cells {
        let ring = &subpaths(cell.attribute("d").unwrap())[0];
        assert!(ring.closed);
        let (x0, x1) = x_span(ring);
        let (y0, y1) = y_span(ring);
        // the columns at 0, 1, 2 of the axis -0.5..2.5, the rows at 0, 1 of -0.5..1.5 upwards
        let column = ((x0 + x1) / 2.0 - p.x) / p.width * 3.0 - 0.5;
        let row = (p.y + p.height - (y0 + y1) / 2.0) / p.height * 2.0 - 0.5;
        let (i, j) = (row.round() as usize, column.round() as usize);
        let want = &color_scale(&s, &[values[i][j]])[0];
        assert_eq!(cell.attribute("fill"), Some(want.as_str()), "cell {i} {j}");
    }
}

#[test]
fn box_from_stats() {
    let json = figure(
        r#"{"scale": "category", "categories": ["a"]}"#,
        r#"{"min": 0, "max": 10}"#,
        "",
        r#"{"id": "b", "kind": "box", "positions": [0],
            "stats": [{"low_whisker": 1, "q1": 3, "median": 4, "q3": 6, "high_whisker": 8,
                       "outliers": [9.5, 0.5]}]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let group = element_group(&doc, "b");
    let p = panel(&doc);
    let y = |v: f64| y_pixel(p, 0.0, 10.0, v);
    let center = x_pixel(p, -0.5, 0.5, 0.0);
    // the box: q1 to q3, centred on its position
    let boxes = filled(group);
    assert_eq!(boxes.len(), 1);
    let ring = &subpaths(boxes[0].attribute("d").unwrap())[0];
    assert!(ring.closed);
    let (top, bottom) = y_span(ring);
    assert_near(top, y(6.0), "q3");
    assert_near(bottom, y(3.0), "q1");
    let (left, right) = x_span(ring);
    assert_near((left + right) / 2.0, center, "box centre");
    // the whiskers reach the given numbers, the median is a line across the box
    let lines: Vec<Subpath> = open_lines(group)
        .iter()
        .flat_map(|l| subpaths(l.attribute("d").unwrap()))
        .collect();
    let vertical: Vec<&Subpath> = lines
        .iter()
        .filter(|s| near(s.points[0].0, s.points[1].0))
        .collect();
    let mut ends: Vec<f64> = vertical
        .iter()
        .flat_map(|s| [y_span(s).0, y_span(s).1])
        .collect();
    ends.sort_by(f64::total_cmp);
    assert_eq!(vertical.len(), 2, "two whiskers");
    assert_near(ends[0], y(8.0), "high whisker");
    assert_near(ends[1], y(6.0), "q3 end of the whisker");
    assert_near(ends[2], y(3.0), "q1 end of the whisker");
    assert_near(ends[3], y(1.0), "low whisker");
    let median = lines
        .iter()
        .find(|s| near(s.points[0].1, y(4.0)) && near(s.points[1].1, y(4.0)))
        .expect("a median line");
    let (ml, mr) = x_span(median);
    assert_near(ml, left, "median from the left of the box");
    assert_near(mr, right, "median to the right of the box");
    // one circle per outlier
    let circles = used_markers(&doc, group);
    assert_eq!(circles.len(), 2);
    assert!(circles.iter().all(|c| c.shape.has_tag_name("circle")));
    let mut cys: Vec<f64> = circles.iter().map(|c| c.y).collect();
    cys.sort_by(f64::total_cmp);
    assert_near(cys[0], y(9.5), "outlier 9.5");
    assert_near(cys[1], y(0.5), "outlier 0.5");
}

#[test]
fn box_below_a_log_axis_is_clipped_at_its_start() {
    // q1 of 0 has no place on a log axis: the body runs from the start of the axis to q3
    let json = figure(
        r#"{"scale": "category", "categories": ["a"]}"#,
        r#"{"scale": "log", "min": 0.1, "max": 100}"#,
        "",
        r#"{"id": "b", "kind": "box", "positions": [0],
            "stats": [{"low_whisker": -1, "q1": 0, "median": 2, "q3": 10, "high_whisker": 50}]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let group = element_group(&doc, "b");
    let p = panel(&doc);
    let y = |v: f64| p.y + p.height - (v.log10() + 1.0) / 3.0 * p.height;
    let boxes = filled(group);
    assert_eq!(boxes.len(), 1, "the body is drawn");
    let ring = &subpaths(boxes[0].attribute("d").unwrap())[0];
    let (top, bottom) = y_span(ring);
    assert_near(top, y(10.0), "q3");
    assert_near(bottom, p.y + p.height, "the start of the axis");
}

#[test]
fn histogram_bins() {
    let json = figure(
        r#"{"label": "value"}"#,
        r#"{"label": "count", "min": 0, "max": 6}"#,
        "",
        r#"{"id": "h", "kind": "histogram", "edges": [0, 1, 2, 3, 4], "counts": [1, 3, 2, 5]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    // the bars of neighbouring bins are one outline, without seams between them (since 0.33.0)
    let bars = filled(element_group(&doc, "h"));
    assert_eq!(bars.len(), 1, "one outline of the four bars");
    let ring = &subpaths(bars[0].attribute("d").unwrap())[0];
    assert!(ring.closed);
    assert_near(y_span(ring).1, p.y + p.height, "the base of the bars at 0");
    // along the tops: two corners per bin at the height of its count
    let y = |v: f64| y_pixel(p, 0.0, 6.0, v);
    for (k, c) in [1.0, 3.0, 2.0, 5.0].into_iter().enumerate() {
        assert_near(ring.points[2 * k].1, y(c), "the top of a bar");
        assert_near(ring.points[2 * k + 1].1, y(c), "the top of a bar");
    }
}

#[test]
fn bars_start_at_zero() {
    // the axis of the heights sticks to the base of the bars, as matplotlib's does
    let json = figure(
        "{}",
        r#"{"label": "count"}"#,
        "",
        r#"{"id": "h", "kind": "histogram", "edges": [0, 1, 2], "counts": [1, 3]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let bars = filled(element_group(&doc, "h"));
    assert_eq!(bars.len(), 1);
    for bar in bars {
        let ring = &subpaths(bar.attribute("d").unwrap())[0];
        assert_near(
            y_span(ring).1,
            p.y + p.height,
            "a bar on the bottom of the panel",
        );
    }
}

#[test]
fn violin_outline() {
    let json = figure(
        r#"{"scale": "category", "categories": ["a", "b"]}"#,
        "{}",
        "",
        r#"{"id": "v", "kind": "violin", "positions": [0, 1],
            "densities": [{"values": [0, 1, 2, 3], "density": [0.1, 0.6, 0.3, 0.05]},
                          {"values": [1, 2, 3], "density": [0.2, 0.2, 0.4]}]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let outlines = filled(element_group(&doc, "v"));
    assert_eq!(outlines.len(), 2, "one outline per position");
    for (k, o) in outlines.iter().enumerate() {
        let rings = subpaths(o.attribute("d").unwrap());
        assert_eq!(rings.len(), 1);
        let ring = &rings[0];
        assert!(ring.closed);
        let center = x_pixel(p, -0.5, 1.5, k as f64);
        let (l, r) = x_span(ring);
        assert_near((l + r) / 2.0, center, "violin centred on its position");
        // mirrored: every point has its partner across the position
        for (x, y) in &ring.points {
            let mirror = 2.0 * center - x;
            assert!(
                ring.points
                    .iter()
                    .any(|(a, b)| near(*a, mirror) && near(*b, *y)),
                "no mirror of ({x}, {y})"
            );
        }
    }
}

#[test]
fn polygon_paths() {
    let json = figure(
        "{}",
        "{}",
        "",
        r#"{"id": "g", "kind": "polygon",
            "paths": [[[0, 0], [1, 0], [1, 1]], [[2, 2], [3, 2], [3, 3], [2, 3]]]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let rings = filled(element_group(&doc, "g"));
    assert_eq!(rings.len(), 2, "one path per ring");
    let mut sizes: Vec<usize> = rings
        .iter()
        .map(|r| {
            let s = subpaths(r.attribute("d").unwrap());
            assert_eq!(s.len(), 1);
            assert!(s[0].closed);
            s[0].points.len()
        })
        .collect();
    sizes.sort();
    assert_eq!(sizes, [3, 4]);
}

#[test]
fn bar_errors() {
    let json = figure(
        r#"{"scale": "category", "categories": ["oral", "iv", "renal"]}"#,
        r#"{"min": 0, "max": 10}"#,
        "",
        r#"{"id": "b", "kind": "bar", "x": [0, 1, 2], "height": [3, 5, 7], "err": [1, 0.5, 2]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let group = element_group(&doc, "b");
    let p = panel(&doc);
    let y = |v: f64| y_pixel(p, 0.0, 10.0, v);
    assert_eq!(filled(group).len(), 3, "one bar per value");
    let errors: Vec<Subpath> = open_lines(group)
        .iter()
        .flat_map(|l| subpaths(l.attribute("d").unwrap()))
        .collect();
    assert_eq!(errors.len(), 3, "an error line per bar");
    for (k, (h, e)) in [(3.0, 1.0), (5.0, 0.5), (7.0, 2.0)].iter().enumerate() {
        let x = x_pixel(p, -0.5, 2.5, k as f64);
        let line = errors
            .iter()
            .find(|s| near(s.points[0].0, x))
            .unwrap_or_else(|| panic!("no error line of bar {k}"));
        let (top, bottom) = y_span(line);
        assert_near(top, y(h + e), "upper end of the error");
        assert_near(bottom, y(h - e), "lower end of the error");
    }
}

#[test]
fn colorbar_ticks() {
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 10}, "label": "dose"},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let bars: Vec<Node> = doc
        .descendants()
        .filter(|n| n.has_tag_name("g") && n.attribute("class") == Some("sbmlsim-colorbar"))
        .collect();
    assert_eq!(bars.len(), 1);
    let bar = bars[0];
    // the bar spans the panel, right of it
    let rect = bar
        .descendants()
        .find(|n| {
            n.has_tag_name("rect") && n.attribute("fill").is_some_and(|f| f.starts_with("url(#"))
        })
        .expect("the gradient of the bar");
    let a = |name: &str| rect.attribute(name).unwrap().parse::<f64>().unwrap();
    assert_near(a("y"), p.y, "top of the bar");
    assert_near(a("height"), p.height, "height of the bar");
    assert!(a("x") > p.x + p.width, "the bar right of the panel");
    assert!(a("x") + a("width") <= 500.0, "the bar inside the figure");
    // its colours are the ones of the scale from low (bottom) to high (top)
    let id = rect
        .attribute("fill")
        .unwrap()
        .trim_start_matches("url(#")
        .trim_end_matches(')');
    let gradient = doc
        .descendants()
        .find(|n| n.has_tag_name("linearGradient") && n.attribute("id") == Some(id))
        .expect("the gradient");
    let stops: Vec<Node> = gradient
        .children()
        .filter(|n| n.has_tag_name("stop"))
        .collect();
    assert_eq!(stops[0].attribute("stop-color"), Some("#440154"));
    assert_eq!(
        stops.last().unwrap().attribute("stop-color"),
        Some("#fde725")
    );
    // labelled ticks from low to high, the low one at the bottom, and the label
    let labels: Vec<(f64, f64)> = bar
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter_map(|n| {
            let v = n.text()?.parse::<f64>().ok()?;
            Some((v, n.attribute("y")?.parse::<f64>().ok()?))
        })
        .collect();
    assert!(labels.len() >= 3, "{labels:?}");
    assert_eq!(labels.first().map(|l| l.0), Some(0.0));
    assert_eq!(labels.last().map(|l| l.0), Some(10.0));
    for (v, y) in &labels {
        assert!((0.0..=10.0).contains(v));
        assert_near(*y, y_pixel(p, 0.0, 10.0, *v), "tick label at its value");
    }
    let ticks = bar.descendants().filter(|n| n.has_tag_name("line")).count();
    assert_eq!(ticks, labels.len(), "a tick per label");
    assert!(
        texts(bar).iter().any(|t| t == "dose"),
        "the label of the bar"
    );
}

/// The colour bars of a figure, in the order of the document.
fn colorbars<'a>(doc: &'a Document<'a>) -> Vec<Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| n.has_tag_name("g") && n.attribute("class") == Some("sbmlsim-colorbar"))
        .collect()
}

/// The rectangle of a colour bar filled with its gradient and the stop colours of that
/// gradient from the bottom up.
fn gradient_of<'a>(doc: &'a Document<'a>, bar: Node<'a, 'a>) -> (Node<'a, 'a>, Vec<String>) {
    let rect = bar
        .descendants()
        .find(|n| {
            n.has_tag_name("rect") && n.attribute("fill").is_some_and(|f| f.starts_with("url(#"))
        })
        .expect("the gradient of the bar");
    let id = rect
        .attribute("fill")
        .unwrap()
        .trim_start_matches("url(#")
        .trim_end_matches(')');
    let gradient = doc
        .descendants()
        .find(|n| n.has_tag_name("linearGradient") && n.attribute("id") == Some(id))
        .expect("the gradient");
    let stops = gradient
        .children()
        .filter(|n| n.has_tag_name("stop"))
        .map(|n| n.attribute("stop-color").unwrap().to_string())
        .collect();
    (rect, stops)
}

/// The tick labels of a colour bar with their y.
fn bar_labels(bar: Node) -> Vec<(String, f64)> {
    bar.descendants()
        .filter(|n| {
            n.has_tag_name("g") && n.attribute("class") == Some("sbmlsim-colorbar-tick-labels")
        })
        .flat_map(|g| g.descendants().filter(|n| n.has_tag_name("text")))
        .map(|n| {
            (
                n.text().unwrap_or_default().to_string(),
                n.attribute("y").unwrap().parse::<f64>().unwrap(),
            )
        })
        .collect()
}

#[test]
fn colorbar_range_fraction() {
    // the lines of a scan take viridis over 0..0.85: the bar shows exactly those colours over
    // exactly the data range, its ends labelled with the data ends
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 2, "high": 8, "start": 0, "end": 0.85},
                        "ticks": [2, 8]},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let bars = colorbars(&doc);
    assert_eq!(bars.len(), 1);
    let (_, stops) = gradient_of(&doc, bars[0]);
    let s = ColorScale {
        start: 0.0,
        end: 0.85,
        ..scale(ColorScaleName::Viridis, 2.0, 8.0)
    };
    let full = scale(ColorScaleName::Viridis, 0.0, 1.0);
    // the colours of the lines at the data ends
    let ends = color_scale(&s, &[2.0, 8.0]);
    assert_eq!(ends[0], color_scale(&full, &[0.0])[0]);
    assert_eq!(ends[1], color_scale(&full, &[0.85])[0]);
    assert_ne!(ends[1], "#fde725", "the top is not the end of the map");
    assert_eq!(stops.first(), Some(&ends[0]));
    assert_eq!(stops.last(), Some(&ends[1]));
    // a value beyond the data takes the colour of its end of the range, not of the map
    assert_eq!(color_scale(&s, &[100.0])[0], ends[1]);
    let p = panel(&doc);
    let labels = bar_labels(bars[0]);
    assert_eq!(
        labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["2", "8"]
    );
    assert_near(labels[0].1, p.y + p.height, "the low end at the bottom");
    assert_near(labels[1].1, p.y, "the high end at the top");
}

#[test]
fn colorbar_tick_labels() {
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "magma", "low": 0, "high": 10},
                        "ticks": [0, 2.5, 10], "tick_labels": ["none", "low", "<high>"]},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let bar = colorbars(&doc)[0];
    let labels = bar_labels(bar);
    assert_eq!(
        labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["none", "low", "<high>"]
    );
    for ((_, y), v) in labels.iter().zip([0.0, 2.5, 10.0]) {
        assert_near(*y, y_pixel(p, 0.0, 10.0, v), "a tick label at its value");
    }
    let ticks = bar.descendants().filter(|n| n.has_tag_name("line")).count();
    assert_eq!(ticks, 3, "a tick per label");
}

#[test]
fn categorical_colorbar() {
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 3, "end": 0.85},
                        "categorical": true, "ticks": [0, 1, 2, 3],
                        "tick_labels": ["a", "b", "c", "d"], "label": "dose"},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let bar = colorbars(&doc)[0];
    assert!(
        !bar.descendants()
            .any(|n| n.attribute("fill").is_some_and(|f| f.starts_with("url(#"))),
        "no gradient"
    );
    // one band per integer from the bottom up, in the colours of the lines
    let s = ColorScale {
        end: 0.85,
        ..scale(ColorScaleName::Viridis, 0.0, 3.0)
    };
    let colors = color_scale(&s, &[0.0, 1.0, 2.0, 3.0]);
    let mut bands: Vec<(f64, f64, String)> = bar
        .descendants()
        .filter(|n| n.has_tag_name("rect"))
        .filter_map(|n| {
            let fill = n.attribute("fill")?;
            fill.starts_with('#').then(|| {
                let a = |name: &str| n.attribute(name).unwrap().parse::<f64>().unwrap();
                (a("y"), a("height"), fill.to_string())
            })
        })
        .collect();
    bands.sort_by(|a, b| b.0.total_cmp(&a.0));
    assert_eq!(bands.len(), 4);
    let h = p.height / 4.0;
    for (k, (y, height, fill)) in bands.iter().enumerate() {
        assert_eq!(fill, &colors[k], "band {k}");
        assert_near(*height, h, "the height of a band");
        assert_near(
            *y,
            p.y + p.height - (k as f64 + 1.0) * h,
            "the place of a band",
        );
    }
    // the tick labels at the middle of their bands
    let labels = bar_labels(bar);
    assert_eq!(
        labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c", "d"]
    );
    for (k, (_, y)) in labels.iter().enumerate() {
        assert_near(
            *y,
            p.y + p.height - (k as f64 + 0.5) * h,
            "a label at its band",
        );
    }
    assert!(texts(bar).iter().any(|t| t == "dose"));
    // without ticks the integers are labelled
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 1, "high": 3}, "categorical": true},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let labels = bar_labels(colorbars(&doc)[0]);
    assert_eq!(
        labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
}

#[test]
fn two_colorbars_stack() {
    let json = figure(
        "{}",
        "{}",
        r#""colorbars": [
            {"scale": {"name": "viridis", "low": 0, "high": 10}, "label": "dose"},
            {"scale": {"name": "magma", "low": 1, "high": 100, "log": true}, "label": "weight"}
        ],"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let p = panel(&doc);
    let bars = colorbars(&doc);
    assert_eq!(bars.len(), 2);
    let (first, first_stops) = gradient_of(&doc, bars[0]);
    let (second, second_stops) = gradient_of(&doc, bars[1]);
    assert_ne!(
        first.attribute("fill"),
        second.attribute("fill"),
        "own gradients"
    );
    assert_eq!(first_stops.first().map(String::as_str), Some("#440154"));
    assert_eq!(second_stops.first().map(String::as_str), Some("#000004"));
    let a = |n: Node, name: &str| n.attribute(name).unwrap().parse::<f64>().unwrap();
    assert!(
        a(first, "x") > p.x + p.width,
        "the first bar right of the panel"
    );
    // the second bar right of everything of the first, its label included
    let right_of_first = bars[0]
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(|n| a(n, "x"))
        .fold(a(first, "x") + a(first, "width"), f64::max);
    assert!(
        a(second, "x") > right_of_first,
        "the second bar after the first"
    );
    assert!(
        a(second, "x") + a(second, "width") <= 500.0,
        "inside the figure"
    );
    for bar in [first, second] {
        assert_near(a(bar, "y"), p.y, "top of a bar");
        assert_near(a(bar, "height"), p.height, "height of a bar");
    }
    assert!(texts(bars[0]).iter().any(|t| t == "dose"));
    assert!(texts(bars[1]).iter().any(|t| t == "weight"));
    // one bar in `colorbars` draws as `colorbar` does
    let one = |field: &str| {
        render(&figure(
            "{}",
            "{}",
            &format!(
                r#""{field}": {}{}{},"#,
                if field == "colorbars" { "[" } else { "" },
                r#"{"scale": {"name": "viridis", "low": 0, "high": 10}, "label": "dose"}"#,
                if field == "colorbars" { "]" } else { "" }
            ),
            r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
        ))
    };
    assert_eq!(one("colorbars"), one("colorbar"));
}

#[test]
fn every_kind_draws() {
    let figure = parse(&spec_text("every_kind")).unwrap();
    let svg = render_svg(&figure);
    for bad in ["NaN", "inf", "var("] {
        assert!(!svg.contains(bad), "{bad} in the drawing");
    }
    let doc = Document::parse(&svg).unwrap();
    for plot in &figure.plots {
        for e in &plot.elements {
            let g = element_group(&doc, &e.id);
            assert!(
                g.descendants().any(|n| n.is_element() && n != g),
                "element {} draws nothing",
                e.id
            );
        }
    }
}

#[test]
fn golden_every_kind() {
    assert_golden("every_kind", &render(&spec_text("every_kind")));
}

#[test]
fn hollow_marker() {
    let json = figure(
        "{}",
        "{}",
        "",
        r##"{"id": "hollow", "kind": "curve", "label": "hollow", "x": [0, 1, 2], "y": [0, 1, 2],
            "style": {"color": "#aa0000", "line": {"type": "none"},
                      "marker": {"type": "circle", "fill": "none", "edge_width": 1.5}}},
           {"id": "edged", "kind": "curve", "x": [0, 1, 2], "y": [2, 1, 0],
            "style": {"color": "#aa0000", "line": {"type": "none"},
                      "marker": {"type": "square", "fill": "none", "edge": "#0000aa"}}}"##,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    // no fill, the edge in the colour of the series
    let circles = used_markers(&doc, element_group(&doc, "hollow"));
    assert_eq!(circles.len(), 3);
    for c in circles.iter().map(|c| c.shape) {
        assert!(c.has_tag_name("circle"));
        assert_eq!(c.attribute("fill"), Some("none"));
        assert_eq!(c.attribute("stroke"), Some("#aa0000"));
        assert_eq!(c.attribute("stroke-width"), Some("1.5"));
    }
    // an edge of its own
    let squares = used_markers(&doc, element_group(&doc, "edged"));
    assert_eq!(squares.len(), 3);
    assert!(
        squares
            .iter()
            .map(|s| s.shape)
            .all(|s| s.has_tag_name("rect")
                && s.attribute("fill") == Some("none")
                && s.attribute("stroke") == Some("#0000aa"))
    );
    // the swatch of the legend is hollow as well
    let swatch = doc
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-legend-swatch"))
        .unwrap();
    let glyph = swatch
        .descendants()
        .find(|n| n.has_tag_name("circle"))
        .unwrap();
    assert_eq!(glyph.attribute("fill"), Some("none"));
    assert_eq!(glyph.attribute("stroke"), Some("#aa0000"));
}

#[test]
fn bar_outline() {
    let json = figure(
        "{}",
        "{}",
        "",
        r##"{"id": "plain", "kind": "bar", "label": "plain", "x": [0, 1], "height": [1, 2],
            "style": {"color": "#aa0000"}},
           {"id": "outlined", "kind": "bar", "label": "outlined", "x": [2, 3], "height": [2, 1],
            "style": {"color": "#aa0000", "line": {"type": "dashed", "width": 1.5}}},
           {"id": "none", "kind": "bar", "x": [4], "height": [1],
            "style": {"line": {"type": "none", "width": 3}}}"##,
    );
    let svg = render(&json);
    let doc = Document::parse(&svg).unwrap();
    let rings = |id: &str| -> Vec<Node> {
        paths(element_group(&doc, id))
            .into_iter()
            .filter(|p| p.attribute("d").is_some_and(|d| d.contains('Z')))
            .collect()
    };
    // a bar without a line has no outline, as before 0.28.0
    for id in ["plain", "none"] {
        let r = rings(id);
        assert!(!r.is_empty());
        assert!(r.iter().all(|p| p.attribute("stroke").is_none()), "{id}");
    }
    // a line outlines the bars with its type and width, in the colour of the bars
    // (a filled ring and a stroked ring per bar, the fill under the outline)
    let outlined: Vec<Node> = rings("outlined")
        .into_iter()
        .filter(|p| p.attribute("stroke").is_some())
        .collect();
    assert_eq!(outlined.len(), 2);
    for p in &outlined {
        assert_eq!(p.attribute("fill"), Some("none"));
        assert_eq!(p.attribute("stroke"), Some("#aa0000"));
        assert_eq!(p.attribute("stroke-width"), Some("1.5"));
        assert!(p.attribute("stroke-dasharray").is_some());
    }
    // the swatch shows the outline
    let swatches: Vec<Node> = doc
        .descendants()
        .filter(|n| n.attribute("class") == Some("sbmlsim-legend-swatch"))
        .collect();
    assert_eq!(swatches.len(), 2);
    let edge = |s: &Node| {
        s.descendants()
            .find(|n| n.has_tag_name("rect"))
            .and_then(|r| r.attribute("stroke"))
            .map(str::to_string)
    };
    assert_eq!(edge(&swatches[0]), None);
    assert_eq!(edge(&swatches[1]).as_deref(), Some("#aa0000"));
}

#[test]
fn colorbar_label_of_lines_and_weights() {
    let json = figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 10},
                        "label": "dose"},"#,
        r#"{"id": "c", "kind": "curve", "x": [0, 1], "y": [0, 1]}"#,
    );
    let two = json.replace("\"label\": \"dose\"", "\"label\": \"dose\\nof HCTZ\"");
    let bold = two.replacen(
        "\"grid\"",
        r#""fonts": {"label_weight": "bold", "tick_weight": "bold", "tick_y": 9}, "grid""#,
        1,
    );
    let room = |svg: &str| {
        let doc = Document::parse(svg).unwrap();
        let p = panel(&doc);
        500.0 - (p.x + p.width)
    };
    let (s1, s2, s3) = (render(&json), render(&two), render(&bold));
    // a second line of the label: 1.2 times 13 pixels more room right of the panel
    assert!((room(&s2) - room(&s1) - 15.6).abs() < 0.02);
    let doc = Document::parse(&s3).unwrap();
    let bar = colorbars(&doc)[0];
    let label = bar
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-colorbar-label"))
        .and_then(|g| g.descendants().find(|n| n.has_tag_name("text")))
        .unwrap();
    assert_eq!(label.attribute("font-weight"), Some("bold"));
    let spans: Vec<Node> = label
        .children()
        .filter(|n| n.has_tag_name("tspan"))
        .collect();
    assert_eq!(spans.len(), 2);
    let (x, y) = (
        label.attribute("x").unwrap().parse::<f64>().unwrap(),
        label.attribute("y").unwrap().parse::<f64>().unwrap(),
    );
    // rotated to the right of the bar: the first line where the one line was, the second
    // further out (lower in the rotated text)
    let at = |n: &Node, a: &str| n.attribute(a).unwrap().parse::<f64>().unwrap();
    assert_near(at(&spans[0], "x"), x, "first line");
    assert_near(at(&spans[0], "y"), y, "first line");
    assert_near(at(&spans[1], "y"), y + 15.6, "second line");
    let ticks = bar
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-colorbar-tick-labels"))
        .unwrap();
    assert_eq!(ticks.attribute("font-weight"), Some("bold"));
    assert_eq!(ticks.attribute("font-size"), Some("9"));
}

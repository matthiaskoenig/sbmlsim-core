//! The rendering of bars, boxes, violins, histograms, heatmaps, polygons and the colour
//! scales with their colour bar.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{
    COLOR_SCALES, ColorScale, ColorScaleName, NO_VALUE_COLOR, color_scale, parse, render_svg,
};
use support::{assert_golden, spec_text};

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
        name,
        low,
        high,
        center: None,
        log: false,
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
    let circles: Vec<Node> = group
        .descendants()
        .filter(|n| n.has_tag_name("circle"))
        .collect();
    assert_eq!(circles.len(), 2);
    let mut cys: Vec<f64> = circles
        .iter()
        .map(|c| c.attribute("cy").unwrap().parse().unwrap())
        .collect();
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
    let bars = filled(element_group(&doc, "h"));
    assert_eq!(bars.len(), 4, "n bars for n + 1 edges");
    let mut tops: Vec<f64> = bars
        .iter()
        .map(|b| {
            let ring = &subpaths(b.attribute("d").unwrap())[0];
            assert!(ring.closed);
            assert_near(y_span(ring).1, p.y + p.height, "the base of a bar at 0");
            y_span(ring).0
        })
        .collect();
    tops.sort_by(f64::total_cmp);
    let y = |v: f64| y_pixel(p, 0.0, 6.0, v);
    let mut want = [y(1.0), y(3.0), y(2.0), y(5.0)];
    want.sort_by(f64::total_cmp);
    for (t, w) in tops.iter().zip(want) {
        assert_near(*t, w, "height of a bar");
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
    assert_eq!(bars.len(), 2);
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

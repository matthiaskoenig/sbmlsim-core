//! The fields of `figure/1` since 0.33.0 (#120): error bars of two sides, contours with labels
//! and filled bands, power scales, inset plots, arrows, texts with a rotation, a vertical anchor,
//! a box, subscripts, a location, marks in the fractions of the panel and unclipped, panel frames,
//! coloured tick labels, bars and histograms with a base, box styles, the markers octagon and
//! tri, thin rows, the weight of plot titles, the size of colour bar labels, further legends of
//! a plot and of the figure, anchored legends, tuples, monospace legends, the badness of
//! matplotlib's `best`, the ticks of matplotlib's log axes, colour bar extends, spans, strips
//! and bars in a panel, a top axis and row titles.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{Errors, Figure, geometry, parse, render_svg};
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

/// The error of a spec which does not parse: its path and message.
fn refused(json: &str) -> (String, String) {
    let e = parse(json).expect_err("the spec is refused");
    (e.path, e.message)
}

/// The group of an element.
fn element<'a>(doc: &'a Document<'a>, id: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| n.has_tag_name("g") && n.attribute("data-element") == Some(id))
        .unwrap_or_else(|| panic!("no element {id}"))
}

/// The text of a node and its descendants.
fn text(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

/// The nodes of a class.
fn of_class<'a>(doc: &'a Document<'a>, class: &str) -> Vec<Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| {
            n.attribute("class")
                .is_some_and(|c| c.split(' ').any(|k| k == class))
        })
        .collect()
}

#[test]
fn golden_v033() {
    assert_golden("v033", &render_svg(&spec(&spec_text("v033"))));
}

#[test]
fn golden_v033_legend() {
    assert_golden("v033_legend", &render_svg(&spec(&spec_text("v033_legend"))));
}

#[test]
fn the_fixtures_round_trip() {
    for name in ["v033", "v033_legend"] {
        // through JSON, as a gap (NaN) is no number equal to itself
        let text = serde_json::to_string(&spec(&spec_text(name))).unwrap();
        let again = serde_json::to_string(&spec(&text)).unwrap();
        assert_eq!(text, again, "{name}");
    }
}

#[test]
fn errors_of_two_sides() {
    let curve = r#"{"id": "c", "kind": "curve", "x": [0, 1, 2], "y": [1, 2, 3],
        "yerr": {"low": [null, 0.5, 0], "high": [0.5, null, 0.5]}, "error_cap": 6}"#;
    let f = spec(&figure("{}", r#"{"min": 0, "max": 4}"#, "", curve));
    match &f.plots[0].elements[0].kind {
        sbmlsim_plot::ElementKind::Curve { yerr: Some(e), .. } => {
            assert!(matches!(e, Errors::Sides { .. }));
            assert!(e.at(0).0.is_nan() && e.at(0).1 == 0.5);
        }
        k => panic!("{k:?}"),
    }
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    // a bar from the value to the end of its side: three bars, a cap at every side drawn
    // (two of the first two points, both of the third, its low side of 0 at the value)
    let d: String = element(&doc, "c")
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .filter_map(|n| n.attribute("d"))
        .collect();
    // the line, three bars and four caps
    assert_eq!(d.matches('M').count(), 1 + 3 + 4, "{d}");
    // refused: a negative side, a side of another length, an unknown key
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "c", "kind": "curve", "x": [0], "y": [1], "yerr": {"low": [-1], "high": [1]}}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].yerr.low[0]");
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "c", "kind": "curve", "x": [0], "y": [1], "yerr": {"low": [1, 2], "high": [1]}}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].yerr.low");
    let (_, message) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "c", "kind": "curve", "x": [0], "y": [1], "yerr": {"lo": [1]}}"#,
    ));
    assert!(message.contains("unknown field `lo`"), "{message}");
}

#[test]
fn contours_draw_lines_labels_and_bands() {
    let svg = render_svg(&spec(&spec_text("v033")));
    let doc = Document::parse(&svg).unwrap();
    let lines = of_class(&doc, "sbmlsim-contour-line");
    assert_eq!(lines.len(), 8, "a path per level");
    let labels: Vec<String> = of_class(&doc, "sbmlsim-contour-label")
        .into_iter()
        .map(text)
        .collect();
    assert!(labels.contains(&"0.8".to_string()), "{labels:?}");
    // a band per pair of levels the data reaches, without antialiasing
    let bands = of_class(&doc, "sbmlsim-contour-band");
    assert!(bands.len() >= 8, "{}", bands.len());
    assert!(
        bands
            .iter()
            .all(|b| b.attribute("shape-rendering") == Some("crispEdges"))
    );
    // refused: levels which do not increase, labels of bands, a grid of other rows
    let contour = |extra: &str| {
        figure(
            "{}",
            "{}",
            "",
            &format!(
                r#"{{"id": "k", "kind": "contour", "x": [0, 1], "y": [0, 1],
                    "values": [[0, 1], [1, 2]], {extra}}}"#
            ),
        )
    };
    assert_eq!(
        refused(&contour(r#""levels": [1, 0.5]"#)).0,
        "plots[0].elements[0].levels"
    );
    assert_eq!(
        refused(&contour(
            r#""levels": [0, 1], "filled": true, "labels": ["a", "b"]"#
        ))
        .0,
        "plots[0].elements[0].labels"
    );
    assert_eq!(
        refused(&contour(r#""levels": [0.5], "labels": ["a", "b"]"#)).0,
        "plots[0].elements[0].labels"
    );
}

#[test]
fn a_power_scale_colours_and_its_bar() {
    use sbmlsim_plot::{ColorScale, ColorScaleName, color_scale};
    let s = ColorScale {
        name: Some(ColorScaleName::Viridis),
        stops: None,
        low: 0.0,
        high: 1.0,
        center: None,
        log: false,
        gamma: Some(2.0),
        start: 0.0,
        end: 1.0,
    };
    let plain = ColorScale {
        gamma: None,
        ..s.clone()
    };
    // matplotlib's PowerNorm(2): 0.5 takes the colour of 0.25
    assert_eq!(color_scale(&s, &[0.5]), color_scale(&plain, &[0.25]));
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 1, "high": 10, "log": true, "gamma": 2}},"#,
        "",
    ));
    assert_eq!(path, "plots[0].colorbar.scale.gamma");
}

#[test]
fn an_inset_sits_in_its_host() {
    let f = spec(&spec_text("v033"));
    let g = geometry(&f);
    let rect = |id: &str| g.plots.iter().find(|p| p.id == id).unwrap().rect;
    let (host, inset) = (rect("host"), rect("inset"));
    assert!((inset.x - (host.x + 0.5 * host.width)).abs() < 1e-6);
    assert!((inset.width - 0.42 * host.width).abs() < 1e-6);
    assert!((inset.y + inset.height - (host.y + 0.5 * host.height)).abs() < 1e-6);
    // a white panel and no background of its canvas
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    assert_eq!(of_class(&doc, "sbmlsim-inset-panel").len(), 1);
    // refused: a host after it, another row
    let json = spec_text("v033");
    let moved = json.replacen(r#""plot": "host""#, r#""plot": "stacks""#, 1);
    assert!(refused(&moved).1.contains("no plot before this one"));
}

#[test]
fn texts_and_arrows_of_the_crate() {
    let svg = render_svg(&spec(&spec_text("v033")));
    let doc = Document::parse(&svg).unwrap();
    // a subscript is a lowered tspan of the size of a script
    let rotated = element(&doc, "t_rot");
    assert!(rotated.descendants().any(|n| {
        n.has_tag_name("g")
            && n.attribute("transform")
                .is_some_and(|t| t.contains("rotate(-90)"))
    }));
    let sub = rotated
        .descendants()
        .find(|n| n.has_tag_name("tspan"))
        .unwrap();
    assert_eq!(text(sub), "0-∞");
    // the box of a text and the frame of an anchored text
    assert_eq!(of_class(&doc, "sbmlsim-text-box").len(), 2);
    // an unclipped text is drawn after the panel, outside of its clip
    let out = of_class(&doc, "sbmlsim-unclipped")
        .into_iter()
        .find(|n| n.attribute("data-element") == Some("t_out"))
        .expect("the unclipped group of the text");
    assert!(out.ancestors().all(|a| a.attribute("clip-path").is_none()));
    // the arrows: a line with a filled head, one with an open head, one with a body
    assert_eq!(
        element(&doc, "a_text")
            .descendants()
            .filter(|n| n.attribute("class") == Some("sbmlsim-arrow-head"))
            .count(),
        1
    );
    assert!(text(element(&doc, "a_text")).contains("annotate"));
    // refused: a text with a location and a position
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "t", "kind": "text", "x": 0, "y": 0, "text": "a", "loc": "upper_left"}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].x");
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "t", "kind": "bar", "x": [0], "height": [1], "coordinates": "axes"}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].coordinates");
}

#[test]
fn frames_and_coloured_tick_labels() {
    let svg = render_svg(&spec(&spec_text("v033")));
    let doc = Document::parse(&svg).unwrap();
    let frame = of_class(&doc, "sbmlsim-frame");
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].attribute("stroke"), Some("#d62728"));
    assert_eq!(frame[0].attribute("stroke-width"), Some("3"));
    let colours: Vec<&str> = doc
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .filter(|n| ["duodenum", "jejunum", "ileum"].contains(&text(*n).as_str()))
        .filter_map(|n| n.attribute("fill"))
        .collect();
    assert_eq!(colours, ["#1f77b4", "#2ca02c", "#ff7f0e"]);
}

#[test]
fn bars_and_bins_stack_on_their_base() {
    let f = spec(&figure(
        r#"{"scale": "category", "categories": ["a"]}"#,
        r#"{"min": 0, "max": 4}"#,
        "",
        r#"{"id": "b", "kind": "bar", "x": [0], "height": [1], "base": [2]}"#,
    ));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let panel = geometry(&f).plots[0].rect;
    let d = element(&doc, "b")
        .descendants()
        .find(|n| n.has_tag_name("path"))
        .and_then(|n| n.attribute("d"))
        .unwrap()
        .to_string();
    let ys: Vec<f64> = d
        .split(['M', 'L', 'Z'])
        .filter(|p| !p.is_empty())
        .map(|p| p.split(' ').nth(1).unwrap().parse::<f64>().unwrap())
        .collect();
    let (top, bottom) = ys
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
    // from 2 to 3 of 0..4, in the canvas of the plot
    let h = panel.height;
    assert!(
        ((bottom - top) - h / 4.0).abs() < 0.02,
        "{top} {bottom} {h}"
    );
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "h", "kind": "histogram", "edges": [0, 1, 2], "counts": [1, 2], "base": [0]}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].base");
}

#[test]
fn legends_of_plots_and_of_the_figure() {
    let f = spec(&spec_text("v033_legend"));
    let g = geometry(&f);
    let owners: Vec<(&str, &str)> = g
        .legend_entries
        .iter()
        .map(|e| (e.owner.as_str(), e.legend.as_str()))
        .collect();
    // the groups of the legends of the figure leave the legend of the plot
    assert!(owners.contains(&("figure", "ind")) && owners.contains(&("figure", "s2")));
    assert!(!owners.contains(&("a", "ind")) && !owners.contains(&("a", "s2")));
    assert!(owners.contains(&("a", "s1")) && owners.contains(&("a", "sim")));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let titles: Vec<String> = of_class(&doc, "sbmlsim-legend-title")
        .into_iter()
        .map(text)
        .collect();
    for t in ["substance", "studies", "ID", "studies of the figure"] {
        assert!(titles.contains(&t.to_string()), "{titles:?}");
    }
    // the legend of the individuals is monospace and its swatch has both markers
    let ind = doc
        .descendants()
        .find(|n| {
            n.attribute("class") == Some("sbmlsim-legend-entry")
                && n.attribute("data-legend") == Some("ind")
        })
        .unwrap();
    assert!(ind.descendants().any(|n| {
        n.attribute("font-family")
            .is_some_and(|f| f.contains("monospace"))
    }));
    assert_eq!(
        ind.descendants()
            .filter(|n| n.has_tag_name("path") || n.has_tag_name("rect"))
            .count(),
        2
    );
    // the row titles
    let rows: Vec<String> = of_class(&doc, "sbmlsim-row-title")
        .into_iter()
        .map(text)
        .collect();
    assert_eq!(rows, ["healthy", "T2DM"]);
    // refused: a group in two legends of a plot
    let twice = spec_text("v033_legend").replace(
        r#""groups": [
            "s1"
          ]"#,
        r#""groups": [
            "s1",
            "s1"
          ]"#,
    );
    assert!(
        refused(&twice)
            .1
            .contains("in a legend of the plot already")
    );
}

#[test]
fn log_axes_of_matplotlib() {
    // a span of less than a decade labels the minor ticks of 2, 3, 4 and 6 (0.4 decades and
    // more), each with its power of ten, and draws the others unlabelled and shorter
    let f = spec(&figure(
        r#"{"scale": "log", "min": 0.15, "max": 0.9}"#,
        "{}",
        r#""legend": false,"#,
        "",
    ));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let x = doc
        .descendants()
        .find(|n| n.attribute("data-axis") == Some("x"))
        .unwrap();
    let labels: Vec<String> = x
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(text)
        .collect();
    assert_eq!(labels, ["2×10−1", "3×10−1", "4×10−1", "6×10−1"]);
    let marks: Vec<&str> = x
        .descendants()
        .filter(|n| {
            n.has_tag_name("path")
                && n.parent().is_some_and(|p| {
                    p.attribute("class")
                        .is_some_and(|c| c.contains("sbmlsim-ticks"))
                })
        })
        .filter_map(|n| n.attribute("stroke-width"))
        .collect();
    assert_eq!(marks, ["0.75"], "the minor ticks of one path");
    // two decades: the decades alone are labelled
    let f = spec(&figure(
        r#"{"scale": "log", "min": 0.05, "max": 20}"#,
        "{}",
        r#""legend": false,"#,
        "",
    ));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let x = doc
        .descendants()
        .find(|n| n.attribute("data-axis") == Some("x"))
        .unwrap();
    let labels: Vec<String> = x
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(text)
        .collect();
    assert_eq!(labels, ["10−1", "100", "101"]);
}

#[test]
fn colour_bars_of_0_32() {
    let svg = render_svg(&spec(&spec_text("v033_legend")));
    let doc = Document::parse(&svg).unwrap();
    // the bar in a panel, and the bar over two rows with a triangle at its high end
    assert_eq!(of_class(&doc, "sbmlsim-colorbar-panel").len(), 1);
    let svg = render_svg(&spec(&spec_text("v033")));
    let doc = Document::parse(&svg).unwrap();
    assert_eq!(of_class(&doc, "sbmlsim-colorbar-strip").len(), 1);
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "top", "rows": 2},"#,
        "",
    ));
    assert_eq!(path, "plots[0].colorbar.rows");
}

#[test]
fn a_top_axis_with_its_labels() {
    let svg = render_svg(&spec(&spec_text("v033")));
    let doc = Document::parse(&svg).unwrap();
    let top = doc
        .descendants()
        .find(|n| n.attribute("data-axis") == Some("x2"))
        .unwrap();
    let labels: Vec<String> = top
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(text)
        .collect();
    assert_eq!(labels, ["low", "mid", "high", "class"]);
}

#[test]
fn thin_rows_keep_their_panels() {
    // ten rows in 360 pixels, the ticks hidden but in the last row: no panel overlaps the next
    let mut plots = Vec::new();
    for r in 0..10 {
        let x = if r == 9 {
            r#"{"label": "time"}"#
        } else {
            r#"{"ticks_visible": false}"#
        };
        plots.push(format!(
            r#"{{"id": "p{r}", "row": {r}, "column": 0, "x": {x}, "y": {{"ticks_visible": false}},
                "elements": [{{"id": "c{r}", "kind": "curve", "x": [0, 1], "y": [0, 1]}}]}}"#
        ));
    }
    let json = format!(
        r#"{{"contract": "figure/1", "width": 500, "height": 360,
            "grid": {{"rows": 10, "columns": 1}}, "plots": [{}]}}"#,
        plots.join(",")
    );
    let f = spec(&json);
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    // the panels Charton draws are the panels of the layout, however thin
    let clips: Vec<f64> = doc
        .descendants()
        .filter(|n| n.has_tag_name("clipPath"))
        .filter_map(|c| c.first_element_child())
        .filter_map(|r| r.attribute("height").and_then(|h| h.parse().ok()))
        .collect();
    let g = geometry(&f);
    for (c, p) in clips.iter().zip(&g.plots) {
        assert!((c - p.rect.height).abs() < 0.02, "{c} {:?}", p.rect);
    }
    for w in g.plots.windows(2) {
        assert!(w[0].rect.y + w[0].rect.height < w[1].rect.y, "{:?}", w);
    }
}

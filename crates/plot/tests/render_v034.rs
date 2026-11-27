//! The fields of `figure/1` since 0.34.0 (#125): legends of the figure above the grid and
//! under or over a column, room for a legend anchored beyond its panel, fonts and frames of
//! every legend, the opacity of the median of a box, the opacity of an arrow with a body as a
//! whole, contours at the least value of the data, the edges of bars and bins and the swatch of
//! a bin with its edge, the weight of the labels and the size of the ticks of colour bars, the
//! drawing order `z` across both y axes, the side of the ticks of a horizontal colour bar and a
//! bar below a row, the weight of the tick labels and the size of the title of an axis, and the
//! image element.

mod support;

use roxmltree::{Document, Node};
use sbmlsim_plot::{Figure, geometry, parse, render_svg};
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

/// A number attribute of a node.
fn num(n: Node, name: &str) -> f64 {
    n.attribute(name)
        .unwrap_or_else(|| panic!("no {name}"))
        .parse()
        .unwrap()
}

/// The legend of an owner (`data-plot`, `data-figure`) whose entries include a group.
fn legend_of<'a>(doc: &'a Document<'a>, group: &str) -> Node<'a, 'a> {
    of_class(doc, "sbmlsim-legend")
        .into_iter()
        .find(|l| {
            l.descendants()
                .any(|n| n.attribute("data-legend") == Some(group))
        })
        .unwrap_or_else(|| panic!("no legend of {group}"))
}

/// A PNG of 1 by 1 pixel, white, as base64.
const WHITE: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC";

#[test]
fn golden_v034() {
    assert_golden("v034", &render_svg(&spec(&spec_text("v034"))));
}

#[test]
fn golden_v034_legend() {
    assert_golden("v034_legend", &render_svg(&spec(&spec_text("v034_legend"))));
}

#[test]
fn the_fixtures_round_trip() {
    for name in ["v034", "v034_legend"] {
        let text = serde_json::to_string(&spec(&spec_text(name))).unwrap();
        let again = serde_json::to_string(&spec(&text)).unwrap();
        assert_eq!(text, again, "{name}");
    }
}

#[test]
fn legends_of_the_figure_above_the_grid_and_under_columns() {
    let f = spec(&spec_text("v034_legend"));
    let g = geometry(&f);
    let panels: Vec<_> = g.plots.iter().map(|p| p.rect).collect();
    let top = panels.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    // the legend above the grid: between the title and the panels, centred on the figure,
    // without a frame, its labels of size 10; the figure grew by it
    let above = legend_of(&doc, "healthy");
    assert!(of_class(&doc, "sbmlsim-legend-frame").len() >= 2);
    assert!(
        !above
            .descendants()
            .any(|n| n.attribute("class") == Some("sbmlsim-legend-frame"))
    );
    let entries: Vec<_> = g
        .legend_entries
        .iter()
        .filter(|e| e.legend == "healthy" || e.legend == "cirrhosis")
        .collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].rect.y, entries[1].rect.y,
        "two columns in one row"
    );
    assert!(entries[0].rect.y > 32.0 && entries[0].rect.y + entries[0].rect.height < top);
    let label = of_class(&doc, "sbmlsim-legend-label")
        .into_iter()
        .find(|n| text(*n) == "healthy")
        .unwrap();
    assert_eq!(label.attribute("font-size"), Some("10"));
    assert!(g.height > 600.0);
    // the legends below the grid centred under the panels of their columns, side by side
    let middle = |r: &sbmlsim_plot::PixelRect| r.x + r.width / 2.0;
    let frames: Vec<(String, f64, f64)> = ["proximal", "distal"]
        .iter()
        .map(|grp| {
            let l = legend_of(&doc, grp);
            let r = l
                .children()
                .find(|n| n.attribute("class") == Some("sbmlsim-legend-frame"))
                .unwrap();
            (
                grp.to_string(),
                num(r, "x") + num(r, "width") / 2.0,
                num(r, "y"),
            )
        })
        .collect();
    assert!(
        (frames[0].1 - middle(&panels[0])).abs() < 0.01,
        "{frames:?}"
    );
    assert!(
        (frames[1].1 - middle(&panels[2])).abs() < 0.01,
        "{frames:?}"
    );
    assert_eq!(frames[0].2, frames[1].2);
    // the bold legend and the one of size 9
    let bold = of_class(&doc, "sbmlsim-legend-label")
        .into_iter()
        .find(|n| text(*n) == "proximal")
        .unwrap();
    assert_eq!(bold.attribute("font-weight"), Some("bold"));
    // refused: a column outside the grid, a column of a legend right of the grid
    let outside = spec_text("v034_legend").replace(r#""column": 2"#, r#""column": 3"#);
    assert_eq!(refused(&outside).0, "legends[2].column");
    let right = spec_text("v034_legend").replace(
        r#""position": "bottom",
      "column": 0"#,
        r#""position": "right",
      "column": 0"#,
    );
    assert_eq!(refused(&right).0, "legends[1].column");
}

#[test]
fn a_legend_anchored_beyond_its_panel_takes_room() {
    let f = spec(&spec_text("v034_legend"));
    let g = geometry(&f);
    let own = g
        .legend_entries
        .iter()
        .find(|e| e.legend == "own")
        .expect("the legend of the plot");
    // right of the panel of its plot, inside the figure, left of the next panel
    let (a, b) = (&g.plots[0].rect, &g.plots[1].rect);
    assert!(own.rect.x > a.x + a.width);
    assert!(own.rect.x + own.rect.width < b.x - 30.0, "{own:?} {b:?}");
    // without the room the next panel starts left of the legend's end
    let inside = spec_text("v034_legend").replace("[\n        1.02,", "[\n        0.98,");
    let g2 = geometry(&spec(&inside));
    assert!(g2.plots[1].rect.x < b.x - 30.0);
}

#[test]
fn fonts_and_frames_of_every_legend() {
    let svg = render_svg(&spec(&spec_text("v034")));
    let doc = Document::parse(&svg).unwrap();
    // the legend of the plot without a frame, of size 9, bold, monospace
    let plain = legend_of(&doc, "one");
    assert!(
        !plain
            .descendants()
            .any(|n| n.attribute("class") == Some("sbmlsim-legend-frame"))
    );
    let label = plain
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-legend-label"))
        .unwrap();
    assert_eq!(label.attribute("font-size"), Some("9"));
    assert_eq!(label.attribute("font-weight"), Some("bold"));
    assert!(
        label
            .attribute("font-family")
            .is_some_and(|f| f.contains("monospace"))
    );
    // a further legend of a plot: bold labels of 12, its title in the weight of the fonts
    let svg = render_svg(&spec(&spec_text("v034_legend")));
    let doc = Document::parse(&svg).unwrap();
    let data = legend_of(&doc, "obs");
    let title = data
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-legend-title"))
        .unwrap();
    assert_eq!(title.attribute("font-size"), Some("12"));
    assert_eq!(title.attribute("font-weight"), None);
    // refused: a size which is not positive
    let (path, _) = refused(&figure("{}", "{}", r#""legend_size": 0,"#, ""));
    assert_eq!(path, "plots[0].legend_size");
}

#[test]
fn medians_arrows_and_contours_of_matplotlib() {
    let svg = render_svg(&spec(&spec_text("v034")));
    let doc = Document::parse(&svg).unwrap();
    // the median of a translucent box opaque
    let opacities: Vec<Option<&str>> = element(&doc, "auc")
        .descendants()
        .filter(|n| n.has_tag_name("path") && n.attribute("stroke") == Some("#000000"))
        .map(|n| n.attribute("opacity"))
        .collect();
    assert_eq!(opacities, [None]);
    // an arrow with a body: one opacity of the whole, no opacity of its fill or edge
    let arrow = element(&doc, "wide")
        .descendants()
        .find(|n| n.attribute("class") == Some("sbmlsim-arrow"))
        .unwrap();
    assert_eq!(arrow.attribute("opacity"), Some("0.5"));
    assert_eq!(arrow.attribute("fill-opacity"), None);
    assert_eq!(arrow.attribute("stroke-opacity"), None);
    // the level of the least value of the data draws the edge of its region
    let levels: Vec<&str> = element(&doc, "zero")
        .descendants()
        .filter_map(|n| n.attribute("data-level"))
        .collect();
    assert_eq!(levels, ["0", "1", "2"]);
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "b", "kind": "box", "positions": [0], "median_opacity": 2,
            "stats": [{"low_whisker": 0, "q1": 1, "median": 2, "q3": 3, "high_whisker": 4}]}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].median_opacity");
}

#[test]
fn edges_of_bars_and_bins() {
    let svg = render_svg(&spec(&spec_text("v034")));
    let doc = Document::parse(&svg).unwrap();
    let strokes = |id: &str| -> Vec<(String, String)> {
        element(&doc, id)
            .descendants()
            .filter(|n| n.has_tag_name("path"))
            .filter_map(|n| {
                Some((
                    n.attribute("stroke")?.to_string(),
                    n.attribute("stroke-width")?.to_string(),
                ))
            })
            .collect()
    };
    assert!(strokes("stack_low").contains(&("#ffffff".into(), "1.5".into())));
    // a bin outlined in black at the width of the default edge
    assert!(strokes("survivors").contains(&("#000000".into(), "1".into())));
    // its swatch: a white box with a black edge
    let swatch = legend_of(&doc, "survivors")
        .descendants()
        .find(|n| {
            n.has_tag_name("rect")
                && n.ancestors()
                    .any(|a| a.attribute("data-legend") == Some("survivors"))
        })
        .unwrap();
    assert_eq!(swatch.attribute("fill"), Some("#ffffff"));
    assert_eq!(swatch.attribute("stroke"), Some("#000000"));
    for (field, value, path) in [
        ("edge_color", r#""white""#, "edge_color"),
        ("edge_width", "-1", "edge_width"),
    ] {
        let (p, _) = refused(&figure(
            "{}",
            "{}",
            "",
            &format!(
                r#"{{"id": "h", "kind": "histogram", "edges": [0, 1], "counts": [1],
                    "{field}": {value}}}"#
            ),
        ));
        assert_eq!(p, format!("plots[0].elements[0].{path}"));
    }
}

#[test]
fn the_drawing_order_across_both_y_axes() {
    let svg = render_svg(&spec(&spec_text("v034")));
    let doc = Document::parse(&svg).unwrap();
    // the bars of the right axis at z -1 under the boxes of the left one, in its plot group
    let order: Vec<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("data-element"))
        .filter(|id| *id == "activity" || *id == "auc")
        .collect();
    assert_eq!(order, ["activity", "auc"]);
    assert_eq!(
        element(&doc, "activity")
            .parent()
            .unwrap()
            .attribute("class"),
        element(&doc, "auc").parent().unwrap().attribute("class")
    );
    // an image goes under the other elements, one of a z above them over them
    let order: Vec<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("data-element"))
        .filter(|id| ["scan", "digitized", "checker"].contains(id))
        .collect();
    assert_eq!(order, ["scan", "digitized", "checker"]);
    // without a z the bytes of the charts: the right axis after the left one
    let plain = spec_text("v034").replace(r#""z": -1,"#, "");
    let doc_plain = render_svg(&spec(&plain));
    let at = |id: &str| doc_plain.find(&format!("data-element=\"{id}\"")).unwrap();
    assert!(at("auc") < at("activity"));
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        r#"{"id": "c", "kind": "curve", "x": [0], "y": [0], "z": "up"}"#,
    ));
    assert_eq!(path, "plots[0].elements[0].z");
}

#[test]
fn colour_bars_below_rows_and_their_fonts() {
    let f = spec(&spec_text("v034"));
    let g = geometry(&f);
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    // the bar below the last row: below its panels, its ticks of size 9 below it, its label
    // bold
    let bottom = of_class(&doc, "sbmlsim-colorbar-bottom");
    assert_eq!(bottom.len(), 1);
    let bar = bottom[0]
        .children()
        .find(|n| n.has_tag_name("rect"))
        .unwrap();
    let low = g
        .plots
        .iter()
        .filter(|p| p.id == "scan2d")
        .map(|p| p.rect.y + p.rect.height)
        .next()
        .unwrap();
    assert!(num(bar, "y") > low + 30.0, "below the axis of the row");
    let ticks = bottom[0]
        .children()
        .find(|n| n.attribute("class") == Some("sbmlsim-colorbar-tick-labels"))
        .unwrap();
    assert_eq!(ticks.attribute("font-size"), Some("9"));
    let first = ticks.children().find(|n| n.has_tag_name("text")).unwrap();
    assert!(num(first, "y") > num(bar, "y"));
    let label = bottom[0]
        .descendants()
        .find(|n| n.has_tag_name("text") && text(*n) == "AUC [mg h/l]")
        .unwrap();
    assert_eq!(label.attribute("font-weight"), Some("bold"));
    // the inset bar with its ticks above it
    let inset = of_class(&doc, "sbmlsim-colorbar-inset");
    let bar = inset[0]
        .children()
        .find(|n| n.has_tag_name("rect"))
        .unwrap();
    let ticks = inset[0]
        .children()
        .find(|n| n.attribute("class") == Some("sbmlsim-colorbar-tick-labels"))
        .unwrap();
    let first = ticks.children().find(|n| n.has_tag_name("text")).unwrap();
    assert!(num(first, "y") < num(bar, "y"));
    // refused: a side of the ticks of a vertical bar, two bars below a row
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        r#""colorbar": {"scale": {"name": "viridis", "low": 0, "high": 1}, "tick_side": "top"},"#,
        "",
    ));
    assert_eq!(path, "plots[0].colorbar.tick_side");
    let (path, message) = refused(&figure(
        "{}",
        "{}",
        r#""colorbars": [{"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "bottom"},
                         {"scale": {"name": "viridis", "low": 0, "high": 1}, "placement": "bottom"}],"#,
        "",
    ));
    assert_eq!(path, "plots[0].colorbars[1].placement");
    assert!(message.contains("below it already"), "{message}");
    let (path, _) =
        refused(&spec_text("v034").replace(r#""colorbar_tick": 9"#, r#""colorbar_tick": 0"#));
    assert_eq!(path, "fonts.colorbar_tick");
}

#[test]
fn fonts_of_an_axis() {
    let svg = render_svg(&spec(&spec_text("v034")));
    let doc = Document::parse(&svg).unwrap();
    let plot = doc
        .descendants()
        .find(|n| n.attribute("data-plot") == Some("axes"))
        .unwrap();
    let axis = |name: &str| {
        plot.descendants()
            .find(|n| n.attribute("data-axis") == Some(name))
            .unwrap()
    };
    // the tick labels of x bold, those of y not
    let weights = |a: Node| -> Vec<Option<String>> {
        a.descendants()
            .filter(|n| {
                n.has_tag_name("text")
                    && n.ancestors()
                        .any(|p| p.attribute("class") == Some("charton-tick-label"))
            })
            .map(|n| n.attribute("font-weight").map(str::to_string))
            .collect()
    };
    assert!(
        weights(axis("x"))
            .iter()
            .all(|w| w.as_deref() == Some("bold"))
    );
    assert!(weights(axis("y")).iter().all(Option::is_none));
    // the title of y of size 9
    let title = axis("y")
        .descendants()
        .find(|n| n.attribute("class") == Some("charton-axis-title"))
        .and_then(|t| t.children().find(|n| n.has_tag_name("text")))
        .unwrap();
    assert_eq!(title.attribute("font-size"), Some("9"));
    let (path, _) = refused(&figure(r#"{"label_size": -2}"#, "{}", "", ""));
    assert_eq!(path, "plots[0].x.label_size");
}

#[test]
fn an_image_over_its_extent() {
    let image = format!(
        r#"{{"id": "img", "kind": "image", "png": "{WHITE}", "extent": [0, 4, 2, 0],
            "origin": "lower", "interpolation": "nearest", "style": {{"opacity": 0.5}}}}"#
    );
    let f = spec(&figure("{}", "{}", r#""legend": false,"#, &image));
    let g = geometry(&f);
    // the axes stick to the edges of the image
    assert_eq!((g.plots[0].x.low, g.plots[0].x.high), (0.0, 4.0));
    assert_eq!((g.plots[0].y.low, g.plots[0].y.high), (0.0, 2.0));
    let svg = render_svg(&f);
    let doc = Document::parse(&svg).unwrap();
    let img = element(&doc, "img")
        .descendants()
        .find(|n| n.has_tag_name("image"))
        .unwrap();
    assert_eq!(
        img.attribute("href"),
        Some(format!("data:image/png;base64,{WHITE}").as_str())
    );
    assert_eq!(img.attribute("preserveAspectRatio"), Some("none"));
    assert_eq!(img.attribute("opacity"), Some("0.5"));
    assert_eq!(
        img.attribute("style"),
        Some("image-rendering:pixelated"),
        "nearest"
    );
    // the extent from top to bottom with the origin below: the first row at y0 = 2, the top,
    // so the image is not mirrored
    assert_eq!(img.attribute("transform"), None);
    let upper = spec(&figure(
        "{}",
        "{}",
        r#""legend": false,"#,
        &image.replace(r#""origin": "lower", "#, ""),
    ));
    let svg = render_svg(&upper);
    assert!(
        svg.contains("transform=\"matrix(1 0 0 -1 "),
        "mirrored along y"
    );
    // refused: no base64, no PNG, an extent without area
    for (png, needle) in [
        ("abc", "multiple of 4"),
        ("aGVsbG8gd29ybGQgaGVsbG8gd29ybGQh", "signature"),
    ] {
        let (path, message) = refused(&figure(
            "{}",
            "{}",
            "",
            &format!(r#"{{"id": "i", "kind": "image", "png": "{png}", "extent": [0, 1, 0, 1]}}"#),
        ));
        assert_eq!(path, "plots[0].elements[0].png");
        assert!(message.contains(needle), "{message}");
    }
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        &format!(r#"{{"id": "i", "kind": "image", "png": "{WHITE}", "extent": [1, 1, 0, 1]}}"#),
    ));
    assert_eq!(path, "plots[0].elements[0].extent");
    let (path, _) = refused(&figure(
        "{}",
        "{}",
        "",
        &format!(
            r#"{{"id": "i", "kind": "image", "png": "{WHITE}", "extent": [0, 1, 0, 1],
                "coordinates": "axes"}}"#
        ),
    ));
    assert_eq!(path, "plots[0].elements[0].coordinates");
}

use sbmlsim_plot::{ElementKind, FIGURE_CONTRACT, parse};

fn base(plot: &str) -> String {
    format!(
        r#"{{"contract":"figure/1","width":600,"height":400,"grid":{{"rows":1,"columns":1}},"plots":[{plot}]}}"#
    )
}
fn plot(extra: &str, elements: &str) -> String {
    format!(r#"{{"id":"p","row":0,"column":0,"x":{{}},"y":{{}},{extra}"elements":[{elements}]}}"#)
}

#[test]
fn parses_every_fixture() {
    let mut n = 0;
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/specs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let text = std::fs::read_to_string(&path).unwrap();
            let fig = parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(fig.contract, FIGURE_CONTRACT);
            let back = serde_json::to_string(&fig).unwrap();
            // NaN is no equal to itself, so the second write is compared as text
            let again = serde_json::to_string(&parse(&back).unwrap()).unwrap();
            assert_eq!(again, back, "{} round trip", path.display());
            n += 1;
        }
    }
    assert!(n >= 5, "found {n} fixtures");
}

#[test]
fn defaults_apply() {
    let f = parse(&base(&plot(
        "",
        r#"{"id":"a","kind":"curve","x":[1],"y":[2]}"#,
    )))
    .unwrap();
    assert_eq!(f.fonts.title, 16.0);
    assert_eq!(f.grid.wspace, 0.3);
    let p = &f.plots[0];
    assert!(p.legend && p.row_span == 1 && p.x.label_visible);
    let e = &p.elements[0];
    assert_eq!(e.style.line.width, 2.0);
    assert!(matches!(e.kind, ElementKind::Curve { .. }));
}

#[test]
fn refuses_other_contract() {
    let s = base("").replace("figure/1", "figure/2");
    assert_eq!(parse(&s).unwrap_err().path, "contract");
}

#[test]
fn refuses_unknown_field() {
    let e = parse(&base(&plot(
        "",
        r#"{"id":"a","kind":"curve","x":[1],"y":[2],"bogus":1}"#,
    )))
    .unwrap_err();
    assert_eq!(e.path, "plots[0].elements[0].bogus", "{e}");
    let e = parse(&base(&plot(r#""nope":1,"#, ""))).unwrap_err();
    assert_eq!(e.path, "plots[0].nope", "{e}");
}

#[test]
fn refuses_length_mismatch() {
    let els = r#"{"id":"a","kind":"curve","x":[1,2,3,4],"y":[1,2,3,4]},{"id":"b","kind":"curve","x":[1,2,3,4],"y":[1,2,3]}"#;
    let e = parse(&base(&plot("", els))).unwrap_err();
    assert_eq!(e.path, "plots[0].elements[1].y");
    assert_eq!(e.message, "3 values, x has 4");
    assert_eq!(e.to_string(), "plots[0].elements[1].y: 3 values, x has 4");
}

#[test]
fn refuses_duplicate_element_id() {
    let els = r#"{"id":"a","kind":"rule","orientation":"horizontal","value":1},{"id":"a","kind":"rule","orientation":"vertical","value":1}"#;
    let e = parse(&base(&plot("", els))).unwrap_err();
    assert_eq!(e.path, "plots[0].elements[1].id");
    // the same id in two plots
    let one = r#"{"id":"a","kind":"rule","orientation":"horizontal","value":1}"#;
    let p2 = plot("", one).replace(r#""id":"p""#, r#""id":"q""#);
    let s = base(&format!("{},{}", plot("", one), p2));
    let e = parse(&s).unwrap_err();
    assert_eq!(e.path, "plots[1].elements[0].id");
}

#[test]
fn null_is_a_gap() {
    let f = parse(&base(&plot(
        "",
        r#"{"id":"a","kind":"curve","x":[1,2,3],"y":[1,null,3],"xerr":null}"#,
    )))
    .unwrap();
    let ElementKind::Curve { y, xerr, .. } = &f.plots[0].elements[0].kind else {
        panic!()
    };
    assert!(y[1].is_nan() && y[0] == 1.0 && y[2] == 3.0);
    assert!(xerr.is_none());
    let back = serde_json::to_string(&f).unwrap();
    assert!(back.contains(r#""y":[1.0,null,3.0]"#), "{back}");
    assert!(!back.contains("xerr"), "{back}");
    assert!(
        !back.contains("null,") || back.contains("[1.0,null"),
        "{back}"
    );
}

#[test]
fn legend_group_defaults() {
    let f = parse(&base(&plot(
        "",
        r#"{"id":"a","kind":"rule","orientation":"horizontal","value":1,"label":"L"},
           {"id":"b","kind":"rule","orientation":"horizontal","value":1,"label":"L","legend":"g"},
           {"id":"c","kind":"rule","orientation":"horizontal","value":1}"#,
    )))
    .unwrap();
    let e = &f.plots[0].elements;
    assert_eq!(e[0].legend_group(), Some("a"));
    assert_eq!(e[1].legend_group(), Some("g"));
    assert_eq!(e[2].legend_group(), None);
}

#[test]
fn refuses_unknown_color_scale_name() {
    let el = r#"{"id":"h","kind":"heatmap","rows":["a"],"columns":["b"],"values":[[1]],"scale":{"name":"jet","low":0,"high":1}}"#;
    let e = parse(&base(&plot("", el))).unwrap_err();
    assert!(e.path.contains("scale") && e.message.contains("jet"), "{e}");
}

#[test]
fn refuses_inconsistent_data() {
    let bad = [
        (
            r#"{"id":"b","kind":"box","positions":[0],"stats":[{"low_whisker":3,"q1":1,"median":2,"q3":3,"high_whisker":4}]}"#,
            "plots[0].elements[0].stats[0]",
        ),
        (
            r#"{"id":"p","kind":"polygon","paths":[[[0,0],[1,1]]]}"#,
            "plots[0].elements[0].paths[0]",
        ),
        (
            r#"{"id":"p","kind":"polygon","paths":[]}"#,
            "plots[0].elements[0].paths",
        ),
        (
            r#"{"id":"h","kind":"histogram","edges":[],"counts":[]}"#,
            "plots[0].elements[0].edges",
        ),
        (
            r#"{"id":"h","kind":"heatmap","rows":["a"],"columns":["b"],"values":[[1]],"scale":{"name":"rdbu","low":0,"high":1,"center":2}}"#,
            "plots[0].elements[0].scale.center",
        ),
    ];
    for (el, path) in bad {
        assert_eq!(parse(&base(&plot("", el))).unwrap_err().path, path, "{el}");
    }
    let s = base("").replace(r#""width":600"#, r#""width":600,"fonts":{"tick":0}"#);
    assert_eq!(parse(&s).unwrap_err().path, "fonts.tick");
}

#[test]
fn refuses_span_outside_grid() {
    let e = parse(&base(&plot(r#""row_span":2,"#, ""))).unwrap_err();
    assert_eq!(e.path, "plots[0].row_span");
}

#[test]
fn refuses_categories_on_linear_axis() {
    let s = base(&plot("", "").replace(r#""x":{}"#, r#""x":{"categories":["a"]}"#));
    let e = parse(&s).unwrap_err();
    assert_eq!(e.path, "plots[0].x.categories");
}

#[test]
fn refuses_unknown_kind_and_bad_type() {
    assert!(parse(&base(&plot("", r#"{"id":"a","kind":"pie"}"#))).is_err());
    let e = parse(&base(&plot(
        "",
        r#"{"id":"a","kind":"curve","x":["s"],"y":[1]}"#,
    )))
    .unwrap_err();
    assert!(e.path.starts_with("plots[0].elements[0]"), "{e}");
}

#[test]
fn legend_order_parses_and_refuses_duplicates() {
    let el = r#"{"id":"a","kind":"curve","label":"a","x":[1],"y":[2]}"#;
    let f = parse(&base(&plot(r#""legend_order":["a","b"],"#, el))).unwrap();
    assert_eq!(
        f.plots[0].legend_order.as_deref(),
        Some(&["a".to_string(), "b".to_string()][..])
    );
    let e = parse(&base(&plot(r#""legend_order":["a","a"],"#, el))).unwrap_err();
    assert_eq!(e.path, "plots[0].legend_order[1]", "{e}");
}

#[test]
fn refuses_sizes_past_the_caps() {
    let s = base("").replace(r#""width":600"#, r#""width":16385"#);
    let e = parse(&s).unwrap_err();
    assert_eq!(e.path, "width", "{e}");
    assert!(e.message.contains("at most 16384"), "{e}");
    let s = base("").replace(r#""height":400"#, r#""height":1e9"#);
    assert_eq!(parse(&s).unwrap_err().path, "height");
    let s = base("").replace(r#""width":600"#, r#""width":16384"#);
    assert!(parse(&s).is_ok());
    let s = base("").replace(r#""rows":1"#, r#""rows":65"#);
    let e = parse(&s).unwrap_err();
    assert_eq!(e.path, "grid.rows", "{e}");
    assert!(e.message.contains("at most 64"), "{e}");
    let s = base("").replace(r#""columns":1"#, r#""columns":100000"#);
    assert_eq!(parse(&s).unwrap_err().path, "grid.columns");
    let s = base("").replace(r#""rows":1,"columns":1"#, r#""rows":64,"columns":64"#);
    assert!(parse(&s).is_ok());
}

#[test]
fn nan_and_infinity_are_no_json() {
    for token in ["NaN", "Infinity", "-Infinity"] {
        let s = base(&plot(
            "",
            &format!(r#"{{"id":"a","kind":"curve","x":[1,2],"y":[1,{token}]}}"#),
        ));
        let e = parse(&s).unwrap_err();
        assert_eq!(e.path, "plots[0].elements[0].y[1]", "{e}");
        assert!(e.message.contains(token), "{e}");
        assert!(e.message.contains("null"), "{e}");
    }
    // another syntax error keeps the message of serde
    let s = base(&plot(
        "",
        r#"{"id":"a","kind":"curve","x":[1,2],"y":[1,@]}"#,
    ));
    let e = parse(&s).unwrap_err();
    assert!(!e.message.contains("null"), "{e}");
}

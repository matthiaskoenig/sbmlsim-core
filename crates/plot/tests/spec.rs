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
            assert_eq!(parse(&back).unwrap(), fig, "{} round trip", path.display());
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

use sbmlsim_sbml::{ReadError, read};

fn data(name: &str) -> String {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the fixture '{path}' of the test is not read: {error} (the directory of the \
             workspace is {:?})",
            std::env::current_dir()
        )
    })
}

#[test]
fn reads_level_3_version_1() {
    let doc = read(&data("level3v1.xml")).expect("L3V1 is read");
    assert_eq!((doc.level, doc.version), (3, 1));
    assert_eq!(doc.model.id.as_deref(), Some("decay"));
    assert_eq!(doc.model.parameters.len(), 3);
    assert_eq!(doc.model.rules.len(), 2);
}

#[test]
fn reads_level_2_version_4() {
    let doc = read(&data("level2.xml")).expect("L2V4 is read");
    assert_eq!((doc.level, doc.version), (2, 4));
    assert_eq!(doc.model.id.as_deref(), Some("old"));
}

#[test]
fn reads_level_1_version_2() {
    let text = r#"<sbml xmlns="http://www.sbml.org/sbml/level1" level="1" version="2">
      <model name="m"/></sbml>"#;
    let doc = read(text).expect("L1V2 is read");
    assert_eq!((doc.level, doc.version), (1, 2));
    assert_eq!(doc.model.id.as_deref(), Some("m"));
}

#[test]
fn refuses_a_level_it_does_not_know() {
    let text = r#"<sbml xmlns="http://www.sbml.org/sbml/level1" level="4" version="1">
      <model name="m"/></sbml>"#;
    let err = read(text).expect_err("there is no level 4");
    assert_eq!(
        err,
        ReadError::UnsupportedLevel {
            level: 4,
            version: 1
        }
    );
}

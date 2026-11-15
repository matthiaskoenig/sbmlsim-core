use sbmlsim_sbml::{ReadError, read};

fn data(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
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
fn refuses_level_2() {
    let err = read(&data("level2.xml")).expect_err("L2 is refused");
    assert!(format!("{err}").contains("only level 3"));
    assert_eq!(
        err,
        ReadError::NotLevel3 {
            level: 2,
            version: 4
        }
    );
}

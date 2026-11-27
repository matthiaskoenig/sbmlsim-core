//! The notice of the direction numbers of Joe and Kuo, which the crate carries.
#![cfg(not(target_family = "wasm"))]

use std::path::PathBuf;

fn read(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn the_license_of_the_direction_numbers_is_in_the_notices() {
    let notices = read("THIRD_PARTY.md");
    let path = "crates/sensitivity/data/LICENSE-joe-kuo";
    let heading = format!("`{path}`\n\n```text\n");
    let at = notices
        .find(&heading)
        .unwrap_or_else(|| panic!("no section of {path}"));
    let text = &notices[at + heading.len()..];
    let text = &text[..text.find("\n```\n").unwrap()];
    assert_eq!(text, read(path).trim_end_matches('\n'));
}

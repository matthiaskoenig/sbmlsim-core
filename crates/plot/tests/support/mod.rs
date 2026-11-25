//! What the tests of the renderer share: the fixtures and the golden files.

#![allow(dead_code)]

use std::path::PathBuf;

/// The directory of the crate.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The text of the fixture `tests/specs/<name>.json`.
pub fn spec_text(name: &str) -> String {
    let path = crate_dir().join("tests/specs").join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Compares an SVG with the golden file `tests/golden/<name>.svg`.
///
/// With `SBMLSIM_PLOT_BLESS=1` the golden file is written instead, which is how a change of
/// the drawing is accepted after it was looked at.
pub fn assert_golden(name: &str, svg: &str) {
    let path = crate_dir().join("tests/golden").join(format!("{name}.svg"));
    if std::env::var("SBMLSIM_PLOT_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, svg).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        return;
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; write it with SBMLSIM_PLOT_BLESS=1 and look at it",
            path.display()
        )
    });
    if golden != svg {
        let at = golden
            .bytes()
            .zip(svg.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(golden.len().min(svg.len()));
        let from = at.saturating_sub(80);
        panic!(
            "{} differs at byte {at}:\n golden: {}\n drawn:  {}\nrewrite it with SBMLSIM_PLOT_BLESS=1 after looking at it",
            path.display(),
            golden.get(from..(at + 80).min(golden.len())).unwrap_or(""),
            svg.get(from..(at + 80).min(svg.len())).unwrap_or(""),
        );
    }
}

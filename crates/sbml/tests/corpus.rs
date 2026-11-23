//! Reading every case of the SBML Test Suite, in every level and version it carries.
//!
//! The suite is not part of the repository, so the test reads the directory of the semantic
//! cases from the environment and does nothing without it:
//!
//! ```bash
//! SBML_TEST_SUITE=~/.cache/sbml2cellml/sbml-test-suite/3.5.0/semantic cargo test -p sbmlsim-sbml --test corpus
//! ```
//!
//! A case file of a level the reader does not know yet is skipped, every other one must read:
//! the suite is the corpus of what the levels actually look like, far beyond the fixtures.

use sbmlsim_sbml::{ReadError, read};
use std::path::{Path, PathBuf};

/// The files of the cases, `<case>/<case>-sbml-l<level>v<version>.xml`.
fn case_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(cases) = std::fs::read_dir(root) else {
        return files;
    };
    for case in cases.flatten() {
        let Ok(entries) = std::fs::read_dir(case.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("-sbml-l") && n.ends_with(".xml"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn reads_every_case_of_the_test_suite() {
    let Some(root) = std::env::var_os("SBML_TEST_SUITE") else {
        eprintln!("SBML_TEST_SUITE is not set, the corpus is not read");
        return;
    };
    let files = case_files(Path::new(&root));
    assert!(!files.is_empty(), "no case in {root:?}");
    let mut read_files = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("the case is read from the disk");
        match read(&text) {
            Ok(_) => read_files += 1,
            // a level the reader does not know yet, and a package it refuses: neither is a
            // failure of the reader, the suite carries both
            Err(ReadError::UnsupportedLevel { .. } | ReadError::Package(_)) => skipped += 1,
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    eprintln!(
        "{read_files} of {} case files read, {skipped} skipped, {} failed",
        files.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} case files are not read:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

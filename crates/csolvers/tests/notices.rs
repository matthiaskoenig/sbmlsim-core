//! `THIRD_PARTY.md` is the notice of the libraries this crate builds: its license texts are the
//! files of the submodules, byte for byte, and its versions are the ones of their headers
//! (natively: the tests for wasm32-wasip1 run without access to the files).

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The value of `#define <name> <value>` in a header.
fn define(header: &str, name: &str) -> String {
    read(header)
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next().unwrap().to_string())
        })
        .unwrap_or_else(|| panic!("{name} in {header}"))
}

/// The version `MAIN.SUB.SUBSUB` of a library of SuiteSparse.
fn suitesparse_version(header: &str, prefix: &str) -> String {
    ["MAIN", "SUB", "SUBSUB"]
        .map(|part| define(header, &format!("{prefix}_{part}_VERSION")))
        .join(".")
}

#[test]
fn the_license_texts_are_the_files_of_the_submodules() {
    let notices = read("THIRD_PARTY.md");
    for path in [
        "third_party/sundials/LICENSE",
        "third_party/sundials/NOTICE",
        "third_party/SuiteSparse/KLU/Doc/License.txt",
        "third_party/SuiteSparse/BTF/Doc/License.txt",
        "third_party/SuiteSparse/AMD/Doc/License.txt",
        "third_party/SuiteSparse/COLAMD/Doc/License.txt",
        "third_party/SuiteSparse/KLU/Doc/lesser.txt",
    ] {
        let heading = format!("`{path}`\n\n```text\n");
        let at = notices
            .find(&heading)
            .unwrap_or_else(|| panic!("no section of {path}"));
        let text = &notices[at + heading.len()..];
        let text = &text[..text.find("\n```\n").unwrap()];
        assert_eq!(text, read(path).trim_end_matches('\n'), "{path}");
    }
}

#[test]
fn the_versions_are_the_ones_of_the_submodules() {
    let notices = read("THIRD_PARTY.md");
    // `set(PACKAGE_VERSION_MAJOR "7")` and so on in the CMakeLists of SUNDIALS
    let cmake = read("third_party/sundials/CMakeLists.txt");
    let part = |name: &str| {
        let key = format!("set(PACKAGE_VERSION_{name} \"");
        let line = cmake.lines().find(|l| l.starts_with(&key)).unwrap();
        line[key.len()..line.len() - 2].to_string()
    };
    let sundials = format!("{}.{}.{}", part("MAJOR"), part("MINOR"), part("PATCH"));
    let row = |library: &str, version: &str| {
        let row = format!("| {library} | {version} |");
        assert!(
            notices.lines().any(|line| line.starts_with(&row)),
            "{library} {version}"
        );
    };
    row(
        "SUNDIALS (CVODES, IDA and their vectors, matrices and linear solvers)",
        &sundials,
    );
    let ss = "third_party/SuiteSparse";
    row(
        "KLU (SuiteSparse)",
        &suitesparse_version(&format!("{ss}/KLU/Include/klu.h"), "KLU"),
    );
    row(
        "BTF (SuiteSparse)",
        &suitesparse_version(&format!("{ss}/BTF/Include/btf.h"), "BTF"),
    );
    row(
        "AMD (SuiteSparse)",
        &suitesparse_version(&format!("{ss}/AMD/Include/amd.h"), "AMD"),
    );
    row(
        "COLAMD (SuiteSparse)",
        &suitesparse_version(&format!("{ss}/COLAMD/Include/colamd.h"), "COLAMD"),
    );
    row(
        "SuiteSparse_config (SuiteSparse)",
        &suitesparse_version(
            &format!("{ss}/SuiteSparse_config/SuiteSparse_config.h"),
            "SUITESPARSE",
        ),
    );
}

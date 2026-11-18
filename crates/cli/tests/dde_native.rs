//! Native simulation of the delay models in the prepared semantic test suite.
#![cfg(feature = "native")]

use sbmlsim_cli::testsuite::{CaseSettings, Table, case_request, compare};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;
use std::{fs, path::Path};

#[test]
fn native_delay_cases_match_the_semantic_suite() {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if !cases.join("00001/case.json").exists() {
        assert!(
            std::env::var_os("SBMLSIM_CORE_REQUIRE_CASES").is_none(),
            "the prepared suite is required"
        );
        return;
    }
    let runtime = Runtime::embedded().unwrap();
    let mut count = 0;
    for entry in fs::read_dir(cases).unwrap() {
        let dir = entry.unwrap().path();
        let path = dir.join("model.xml");
        if !path.is_file() {
            continue;
        }
        let xml = fs::read_to_string(path).unwrap();
        if !xml.contains("http://www.sbml.org/sbml/symbols/delay") {
            continue;
        }
        let settings: CaseSettings =
            serde_json::from_slice(&fs::read(dir.join("case.json")).unwrap()).unwrap();
        let expected =
            Table::from_csv(&fs::read_to_string(dir.join("expected.csv")).unwrap()).unwrap();
        let compiled = compile(&xml, &CompileOptions::default()).unwrap();
        let mut native = runtime
            .instantiate_native(&compiled.wasm, &compiled.description)
            .unwrap();
        let result = native
            .simulate(&case_request(&settings))
            .unwrap_or_else(|err| panic!("{}: {err}", settings.cid));
        compare(
            &expected,
            &result,
            settings.absolute_tolerance,
            settings.relative_tolerance,
        )
        .unwrap_or_else(|err| panic!("{}: {}", settings.cid, err.message));
        count += 1;
    }
    assert_eq!(count, 53);
}

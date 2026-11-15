//! The runner of the SBML Test Suite: the comparison rule, the baseline, and the prepared cases.
//!
//! The tests over `testdata/cases` need `uv run scripts/prepare_testsuite.py` and skip with a
//! note when the directory is absent.

use std::path::{Path, PathBuf};
use std::process::Command;

use sbmlsim_abi::SimulationResult;
use sbmlsim_cli::testsuite::{
    Baseline, CaseResult, Expected, Status, Table, compare, differences, run_case,
};

fn result(columns: &[&str], rows: &[&[f64]]) -> SimulationResult {
    SimulationResult {
        columns: columns.iter().map(|c| c.to_string()).collect(),
        n_rows: rows.len(),
        values: rows.iter().flat_map(|r| r.iter().copied()).collect(),
    }
}

fn table(rows: &[&[f64]]) -> Table {
    Table {
        n_cols: rows[0].len(),
        values: rows.iter().flat_map(|r| r.iter().copied()).collect(),
    }
}

#[test]
fn comparison_rule() {
    let columns = ["time", "S1"];
    // |c - u| <= abs + rel * |c|, with c the expected value
    let expected = table(&[&[0.0, 100.0], &[1.0, 0.0]]);
    let inside = result(
        &columns,
        &[&[0.0, 100.0 + 1e-3 + 100.0 * 1e-5 - 1e-9], &[1.0, 1e-3]],
    );
    assert!(compare(&expected, &inside, 1e-3, 1e-5).is_ok());
    let outside = result(
        &columns,
        &[&[0.0, 100.0 + 1e-3 + 100.0 * 1e-5 + 1e-6], &[1.0, 0.0]],
    );
    let failed = compare(&expected, &outside, 1e-3, 1e-5).unwrap_err();
    assert_eq!(failed.status, Status::Tolerance);
    // the tolerance is relative to the expected value, not to the observed one
    let expected = table(&[&[0.0, 0.0]]);
    let observed = result(&columns, &[&[0.0, 1.0]]);
    assert!(compare(&expected, &observed, 1e-3, 1e9).is_err());
}

#[test]
fn non_finite_values_agree_with_themselves_only() {
    let columns = ["time", "S1"];
    let expected = table(&[
        &[0.0, f64::NAN],
        &[1.0, f64::INFINITY],
        &[2.0, f64::NEG_INFINITY],
    ]);
    let same = result(
        &columns,
        &[
            &[0.0, f64::NAN],
            &[1.0, f64::INFINITY],
            &[2.0, f64::NEG_INFINITY],
        ],
    );
    assert!(compare(&expected, &same, 1e-9, 1e-6).is_ok());
    let flipped = result(
        &columns,
        &[
            &[0.0, f64::NAN],
            &[1.0, f64::NEG_INFINITY],
            &[2.0, f64::NEG_INFINITY],
        ],
    );
    assert_eq!(
        compare(&expected, &flipped, 1e-9, 1e-6).unwrap_err().status,
        Status::Tolerance
    );
    let finite = result(
        &columns,
        &[
            &[0.0, 1.0],
            &[1.0, f64::INFINITY],
            &[2.0, f64::NEG_INFINITY],
        ],
    );
    assert!(compare(&expected, &finite, 1e-9, 1e-6).is_err());
}

#[test]
fn columns_are_matched_by_position() {
    // the names of the observed columns play no role, `time` is the first column and is not
    // compared (as in sbmlsim); a missing column and a missing row are `missing_variable`
    let expected = table(&[&[0.0, 1.0, 2.0], &[1.0, 1.0, 2.0]]);
    let renamed = result(&["time", "x", "x"], &[&[0.0, 1.0, 2.0], &[1.0, 1.0, 2.0]]);
    assert!(compare(&expected, &renamed, 1e-9, 1e-6).is_ok());
    let swapped = result(&["time", "x", "x"], &[&[0.0, 2.0, 1.0], &[1.0, 2.0, 1.0]]);
    assert!(compare(&expected, &swapped, 1e-9, 1e-6).is_err());
    let short = result(&["time", "x"], &[&[0.0, 1.0], &[1.0, 1.0]]);
    assert_eq!(
        compare(&expected, &short, 1e-9, 1e-6).unwrap_err().status,
        Status::MissingVariable
    );
    let fewer_rows = result(&["time", "x", "y"], &[&[0.0, 1.0, 2.0]]);
    assert_eq!(
        compare(&expected, &fewer_rows, 1e-9, 1e-6)
            .unwrap_err()
            .status,
        Status::MissingVariable
    );
}

fn outcome(cid: &str, status: Status) -> CaseResult {
    CaseResult {
        cid: cid.to_string(),
        status,
        message: "message".to_string(),
        max_difference: None,
    }
}

fn baseline(listed: &[(&str, &str)]) -> Baseline {
    Baseline {
        suite_commit: "commit".to_string(),
        n_cases: 3,
        n_passed: 3 - listed.len(),
        expected_failures: listed
            .iter()
            .map(|(cid, status)| {
                (
                    cid.to_string(),
                    Expected {
                        status: status.to_string(),
                        reason: "why".to_string(),
                    },
                )
            })
            .collect(),
    }
}

#[test]
fn baseline_both_directions() {
    let results = vec![
        outcome("00001", Status::Pass),
        outcome("00002", Status::Pass),
        outcome("00003", Status::NotCompiled),
    ];
    // as recorded: no difference
    let recorded = baseline(&[("00003", "not_compiled")]);
    assert!(differences(&results, &recorded, true).is_empty());
    // a listed case which passes
    let listed = baseline(&[("00002", "not_compiled"), ("00003", "not_compiled")]);
    let found = differences(&results, &listed, true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cid, "00002");
    assert_eq!(found[0].observed, "pass");
    // an unlisted failure
    let unlisted = baseline(&[]);
    let found = differences(&results, &unlisted, true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cid, "00003");
    assert_eq!(found[0].expected, "pass");
    // a listed case with another status
    let other = baseline(&[("00003", "tolerance")]);
    assert_eq!(differences(&results, &other, true).len(), 1);
    // a listed case which was not run is a difference of a full run only
    let absent = baseline(&[("00003", "not_compiled"), ("00009", "not_compiled")]);
    assert_eq!(differences(&results, &absent, true).len(), 1);
    assert!(differences(&results, &absent, false).is_empty());
}

fn cases_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if dir.join("00001/case.json").exists() {
        Some(dir)
    } else {
        eprintln!(
            "note: {} is absent, run `uv run scripts/prepare_testsuite.py`; skipped",
            dir.display()
        );
        None
    }
}

#[test]
fn prepared_cases_are_complete() {
    let Some(dir) = cases_dir() else { return };
    let baseline: Baseline = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/testsuite_baseline.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_dir() {
            continue;
        }
        n += 1;
        assert!(path.join("case.json").exists(), "{}", path.display());
        assert!(path.join("expected.csv").exists(), "{}", path.display());
    }
    assert_eq!(n, baseline.n_cases);
    for cid in baseline.expected_failures.keys() {
        assert!(
            dir.join(cid).is_dir(),
            "{cid} of the baseline is not prepared"
        );
    }
}

#[test]
fn command_writes_a_result_per_case() {
    let Some(dir) = cases_dir() else { return };
    let json = std::env::temp_dir().join(format!("sbmlsim-testsuite-{}.json", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .args(["testsuite"])
        .arg(&dir)
        .arg("--baseline")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/testsuite_baseline.json"))
        .args(["--cases", "00001-00005", "--jobs", "2", "--json"])
        .arg(&json)
        .output()
        .unwrap();
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    let _ = std::fs::remove_file(&json);
    let results = written["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        5,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for r in results {
        assert!(
            [
                "pass",
                "tolerance",
                "not_compiled",
                "simulation_error",
                "missing_variable",
                "repeat"
            ]
            .contains(&r["status"].as_str().unwrap())
        );
    }
    // a partial run reports no listed case as missing
    assert!(String::from_utf8_lossy(&output.stdout).contains("not_compiled"));
}

#[test]
#[ignore = "passes once the compiler generates code (Task 11)"]
fn runner_on_case_00001() {
    let Some(dir) = cases_dir() else { return };
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let result = run_case(&runtime, &dir.join("00001"));
    assert_eq!(result.status, Status::Pass, "{}", result.message);
}

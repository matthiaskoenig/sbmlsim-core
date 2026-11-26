//! The runner of the SBML Test Suite: the comparison rule, the baseline, and the prepared cases.
//!
//! The tests over `testdata/cases` need `uv run scripts/prepare_testsuite.py` and skip with a
//! note when the directory is absent, unless `SBMLSIM_CORE_REQUIRE_CASES` is set (the jobs of
//! the suite in CI), where they fail.

use std::path::{Path, PathBuf};
use std::process::Command;

use sbmlsim_abi::SimulationResult;
use sbmlsim_cli::testsuite::{
    Baseline, CaseResult, CaseSettings, Expected, Refusals, Status, Table, case_request,
    case_request_of, check_columns, check_commits, compare, differences, differences_with,
    repeat_check, repeat_difference, run_case,
};

fn result(columns: &[&str], rows: &[&[f64]]) -> SimulationResult {
    SimulationResult {
        sensitivities: None,
        local_errors: None,
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
        jacobian: None,
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
fn expected_csv_tokens_and_line_endings() {
    let text = "time,a,b\r\n0,NaN,NAN\r\n1,nan,INF\r\n2,Inf,-INF\r\n3,-Inf,1.5e-005\r\n";
    let table = Table::from_csv(text).unwrap();
    assert_eq!(table.n_cols, 3);
    assert_eq!(table.values.len(), 12);
    let v = &table.values;
    assert!(v[1].is_nan() && v[2].is_nan() && v[4].is_nan());
    assert_eq!(v[5], f64::INFINITY);
    assert_eq!(v[7], f64::INFINITY);
    assert_eq!(v[8], f64::NEG_INFINITY);
    assert_eq!(v[10], f64::NEG_INFINITY);
    assert_eq!(v[11], 1.5e-5);
    assert_eq!(v[9], 3.0);
    // a row with another number of columns and a word which is no number are errors
    assert!(Table::from_csv("time,a\n0,1,2\n").is_err());
    assert!(Table::from_csv("time,a\n0,x\n").is_err());
}

fn settings(selections: &[&str], commit: &str) -> CaseSettings {
    serde_json::from_value(serde_json::json!({
        "cid": "00001", "start": 0.0, "duration": 1.0, "steps": 1,
        "selections": selections, "absolute_tolerance": 1e-9, "relative_tolerance": 1e-6,
        "suite_commit": commit,
    }))
    .unwrap()
}

#[test]
fn column_count_mismatch_is_reported() {
    let table = Table::from_csv("time,a\n0,1\n").unwrap();
    assert!(check_columns(&table, &settings(&["time", "a"], "c")).is_none());
    let message = check_columns(&table, &settings(&["time", "a", "b"], "c")).unwrap();
    assert!(message.contains("2 columns"), "{message}");
}

#[test]
fn suite_commit_mismatch_fails_the_run() {
    let root = std::env::temp_dir().join(format!("sbmlsim-commit-{}", std::process::id()));
    let case = root.join("00001");
    std::fs::create_dir_all(&case).unwrap();
    let json = serde_json::to_vec(&serde_json::json!({
        "cid": "00001", "start": 0.0, "duration": 1.0, "steps": 1, "selections": ["time"],
        "absolute_tolerance": 1e-9, "relative_tolerance": 1e-6, "suite_commit": "old",
    }))
    .unwrap();
    std::fs::write(case.join("case.json"), json).unwrap();
    assert!(check_commits(std::slice::from_ref(&case), "old").is_ok());
    let error = check_commits(std::slice::from_ref(&case), "new")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("'old'") && error.contains("'new'"),
        "{error}"
    );
    // and the command refuses to run
    let baseline = root.join("baseline.json");
    std::fs::write(
        &baseline,
        r#"{"suite_commit":"new","n_cases":1,"n_passed":1,"expected_failures":{}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .arg("testsuite")
        .arg(&root)
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("suite commit"));
    let _ = std::fs::remove_dir_all(&root);
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

fn refusals(pinned: &[&str]) -> Refusals {
    Refusals {
        suite_commit: "c".to_string(),
        n_refused: pinned.len(),
        refused: pinned
            .iter()
            .map(|cid| (cid.to_string(), "message".to_string()))
            .collect(),
    }
}

#[test]
fn refusals_of_the_c_target_are_pinned() {
    let results = vec![
        outcome("00001", Status::Pass),
        outcome("00002", Status::Refused),
    ];
    // a pinned refusal is no difference, also where the baseline lists the case
    let pinned = refusals(&["00002"]);
    assert!(differences_with(&results, &baseline(&[]), Some(&pinned), true).is_empty());
    let listed = baseline(&[("00002", "tolerance")]);
    assert!(differences_with(&results, &listed, Some(&pinned), true).is_empty());
    // an unexpected refusal is a regression, without a pinned set and outside it
    for pinned in [None, Some(refusals(&[]))] {
        let found = differences_with(&results, &baseline(&[]), pinned.as_ref(), true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cid, "00002");
        assert_eq!(found[0].expected, "pass");
        assert_eq!(found[0].observed, "refused");
    }
    assert_eq!(differences(&results, &baseline(&[]), true).len(), 1);
    // a pinned case which is no longer refused
    let found = differences_with(
        &results,
        &baseline(&[]),
        Some(&refusals(&["00001", "00002"])),
        true,
    );
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].expected.as_str(), found[0].observed.as_str()),
        ("refused", "pass")
    );
    // a refusal of another feature than the pinned one
    let mut other = refusals(&["00002"]);
    other
        .refused
        .insert("00002".to_string(), "another".to_string());
    let found = differences_with(&results, &baseline(&[]), Some(&other), true);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].reason.contains("pinned with 'another'"),
        "{}",
        found[0].reason
    );
    // a pinned case which was not run is a difference of a full run only
    let absent = refusals(&["00002", "00009"]);
    let found = differences_with(&results, &baseline(&[]), Some(&absent), true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].observed, "not run");
    assert!(differences_with(&results, &baseline(&[]), Some(&absent), false).is_empty());
    assert_eq!(Status::Refused.as_str(), "refused");
}

fn cases_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if dir.join("00001/case.json").exists() {
        Some(dir)
    } else {
        assert!(
            std::env::var_os("SBMLSIM_CORE_REQUIRE_CASES").is_none(),
            "{} is absent and SBMLSIM_CORE_REQUIRE_CASES is set",
            dir.display()
        );
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
fn fast_reactions_are_refused_when_the_cases_are_prepared() {
    // L3V2 has no `fast` attribute: the preparation refuses a model with a fast reaction before
    // the conversion would drop it, so it is never simulated as a slow reaction
    let Some(dir) = cases_dir() else { return };
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_dir() {
            continue;
        }
        let case: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("case.json")).unwrap()).unwrap();
        let tagged = case["test_tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "FastReaction");
        let refused = case["not_converted"]
            .as_str()
            .is_some_and(|m| m.starts_with("fast reaction "));
        assert_eq!(
            tagged,
            refused,
            "{}: {}",
            path.display(),
            case["not_converted"]
        );
        if refused {
            n += 1;
            assert!(
                case["not_converted"].as_str().unwrap().ends_with(
                    ": refused (fast reactions are out of scope; L3V2 has no fast attribute)"
                ),
                "{}",
                path.display()
            );
            assert!(!path.join("model.xml").exists(), "{}", path.display());
            let result = run_case(&runtime, &path);
            assert_eq!(result.status, Status::NotCompiled, "{}", path.display());
            assert!(result.message.starts_with("not converted: fast reaction "));
        }
    }
    assert!(n > 0, "no prepared case has a fast reaction");
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
    // the 5 cases pass, as in the baseline
    assert!(results.iter().all(|r| r["status"] == "pass"), "{results:?}");
    assert!(written["differences"].as_array().unwrap().is_empty());
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pass             5"), "{stdout}");
    assert!(stdout.contains("no difference to the baseline"), "{stdout}");
}

#[test]
fn runner_on_case_00001() {
    let Some(dir) = cases_dir() else { return };
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    let result = run_case(&runtime, &dir.join("00001"));
    assert_eq!(result.status, Status::Pass, "{}", result.message);
}

/// The C target on the native engine: a case passes with its repeat checks, a delay is refused
/// with its feature, and the command compares with the baseline of `model.wasm`.
#[cfg(feature = "native")]
#[test]
fn runner_of_the_c_target() {
    let Some(dir) = cases_dir() else { return };
    let result = sbmlsim_cli::testsuite::run_case_c(&dir.join("00001"));
    assert_eq!(result.status, Status::Pass, "{}", result.message);
    assert_eq!(result.jacobian.as_deref(), Some("analytic"));
    let delay = sbmlsim_cli::testsuite::run_case_c(&dir.join("00937"));
    assert_eq!(delay.status, Status::Refused, "{}", delay.message);
    assert!(delay.message.contains("delay"), "{}", delay.message);

    let output = Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .args(["testsuite"])
        .arg(&dir)
        .arg("--baseline")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/testsuite_baseline.json"))
        .args([
            "--cases",
            "00001-00003,00937",
            "--model-code",
            "c",
            "--refused",
        ])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/testsuite_refused_c.json"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("pass             3"), "{stdout}");
    assert!(stdout.contains("refused          1"), "{stdout}");
    assert!(
        stdout.contains("00937: the C target does not support"),
        "{stdout}"
    );
    assert!(stdout.contains("no difference to the baseline"), "{stdout}");

    // a refusal which is not pinned fails the run
    let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
    let mut pinned: Refusals =
        serde_json::from_slice(&std::fs::read(testdata.join("testsuite_refused_c.json")).unwrap())
            .unwrap();
    assert!(pinned.refused.remove("00937").is_some());
    let refused = std::env::temp_dir().join(format!("sbmlsim-refused-{}.json", std::process::id()));
    std::fs::write(&refused, serde_json::to_vec(&pinned).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sbmlsim-core"))
        .args(["testsuite"])
        .arg(&dir)
        .arg("--baseline")
        .arg(testdata.join("testsuite_baseline.json"))
        .args(["--cases", "00001,00937", "--model-code", "c", "--refused"])
        .arg(&refused)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&refused);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("00937: 'pass' -> 'refused'"), "{stdout}");
}

#[test]
fn repeat_differences_are_bitwise() {
    let columns = ["time", "S1"];
    let first = result(&columns, &[&[0.0, 1.0], &[1.0, f64::NAN]]);
    assert_eq!(repeat_difference(&first, &first.clone(), "again"), None);
    let next = 1.0f64.next_up();
    let second = result(&columns, &[&[0.0, next], &[1.0, f64::NAN]]);
    let message = repeat_difference(&first, &second, "again").unwrap();
    assert!(
        message.starts_with("again differs from the first in 1 values"),
        "{message}"
    );
    assert!(message.contains("'S1' at the time 0"), "{message}");
    let shorter = result(&columns, &[&[0.0, 1.0]]);
    let message = repeat_difference(&first, &shorter, "again").unwrap();
    assert!(
        message.contains("1 rows of 2 columns and not 2 rows"),
        "{message}"
    );
}

#[test]
fn requests_of_the_repeat_checks() {
    let mut s = settings(&["time", "a"], "c");
    s.start = 2.0;
    s.duration = 10.0;
    s.steps = 51;
    let full = case_request(&s);
    assert_eq!(case_request_of(&s, 1.0, false), full);
    let half = case_request_of(&s, 0.5, false);
    assert_eq!((half.start, half.end), (2.0, 7.0));
    // 25.5 rounds half to even (Python's `round`), as 2.5 and 3.5
    assert_eq!(half.output, sbmlsim_abi::Output::Uniform { steps: 26 });
    for (steps, half) in [(5, 2), (7, 4), (50, 25), (101, 50)] {
        s.steps = steps;
        assert_eq!(
            case_request_of(&s, 0.5, false).output,
            sbmlsim_abi::Output::Uniform { steps: half },
            "{steps}"
        );
    }
    assert_eq!(half.steady_state, None);
    s.steps = 1;
    assert_eq!(
        case_request_of(&s, 0.5, false).output,
        sbmlsim_abi::Output::Uniform { steps: 1 }
    );
    let steady = case_request_of(&s, 1.0, true).steady_state.unwrap();
    assert_eq!(steady.max_time, 10.0);
    assert_eq!(
        (steady.absolute_tolerance, steady.relative_tolerance),
        (1e-8, 1e-6)
    );
}

#[test]
fn repeat_checks_compare_with_the_first_simulation() {
    let Some(dir) = cases_dir() else { return };
    let runtime = sbmlsim_host::Runtime::embedded().unwrap();
    // a case without a steady state (its steady state fails alike on both instances), a case
    // with delayed events which reaches a steady state, a case of random event execution
    for cid in ["00001", "00683", "00952"] {
        let case = dir.join(cid);
        let s: CaseSettings =
            serde_json::from_slice(&std::fs::read(case.join("case.json")).unwrap()).unwrap();
        let sbml = std::fs::read_to_string(case.join("model.xml")).unwrap();
        let expected =
            Table::from_csv(&std::fs::read_to_string(case.join("expected.csv")).unwrap()).unwrap();
        let compiled =
            sbmlsim_compiler::compile(&sbml, &sbmlsim_compiler::CompileOptions::default()).unwrap();
        let mut instance = runtime
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap();
        let first = instance.simulate(&case_request(&s)).unwrap();
        assert_eq!(
            repeat_check(&runtime, &compiled, &mut instance, &s, &expected, &first),
            None,
            "{cid}"
        );
        // a first simulation which differs in one bit is a repeat difference
        let mut other = first.clone();
        let k = other.columns.len();
        other.values[k + 1] = other.values[k + 1].next_up();
        let message = repeat_check(&runtime, &compiled, &mut instance, &s, &expected, &other)
            .unwrap_or_default();
        assert!(
            message.starts_with("the second simulation of the instance differs"),
            "{cid}: {message}"
        );
        assert_eq!(run_case(&runtime, &case).status, Status::Pass, "{cid}");
    }
}

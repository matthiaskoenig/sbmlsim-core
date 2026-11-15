//! A split module keeps the bits: every passing case of the SBML Test Suite compiled with limits
//! of the functions so low that its functions are split into helper functions gives the bits of
//! the module as one, with and without the optimizer.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use sbmlsim_abi::{JacobianChoice, JacobianKind};
use sbmlsim_cli::testsuite::{Baseline, CaseSettings, case_request, repeat_difference};
use sbmlsim_compiler::{CompileOptions, Limits, compile, compile_with_limits, is_above_the_limits};
use sbmlsim_host::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// The directories of the cases which pass, `None` without prepared cases.
fn passing_cases() -> Option<Vec<PathBuf>> {
    let dir = testdata().join("cases");
    if !dir.join("00001/case.json").exists() {
        eprintln!(
            "note: {} is absent, run `uv run scripts/prepare_testsuite.py`; skipped",
            dir.display()
        );
        return None;
    }
    let baseline: Baseline =
        serde_json::from_slice(&std::fs::read(testdata().join("testsuite_baseline.json")).unwrap())
            .unwrap();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("case.json").is_file())
        .filter(|p| {
            let cid = p.file_name().unwrap().to_string_lossy();
            !baseline.expected_failures.contains_key(cid.as_ref())
        })
        .collect();
    dirs.sort();
    assert_eq!(dirs.len(), baseline.n_passed, "the passing cases");
    Some(dirs)
}

/// The number of functions a module defines.
fn defined_functions(wasm: &[u8]) -> usize {
    wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter(|p| matches!(p, Ok(wasmparser::Payload::CodeSectionEntry(_))))
        .count()
}

/// The exports of a module which call helper functions (which are split).
fn split_exports(wasm: &[u8]) -> Vec<String> {
    let mut imports = 0;
    let mut exports: Vec<(u32, String)> = Vec::new();
    let mut bodies = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(import.unwrap().ty, wasmparser::TypeRef::Func(_)) {
                        imports += 1;
                    }
                }
            }
            wasmparser::Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    exports.push((export.index, export.name.to_string()));
                }
            }
            wasmparser::Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    exports
        .into_iter()
        .filter(|(index, _)| {
            bodies[(index - imports) as usize]
                .get_operators_reader()
                .unwrap()
                .into_iter()
                .any(|op| matches!(op.unwrap(), wasmparser::Operator::Call { function_index } if function_index >= imports))
        })
        .map(|(_, name)| name)
        .collect()
}

/// How a case compiled with `limits` differs from the case compiled with the default limits
/// (`Err` when a statement of the case is above `limits`; any other error of the compiler
/// fails the test), and the exports which are split.
fn compare(
    dir: &Path,
    optimize: bool,
    limits: Limits,
) -> Result<(Option<String>, Vec<String>), String> {
    let cid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let settings: CaseSettings =
        serde_json::from_slice(&std::fs::read(dir.join("case.json")).unwrap()).unwrap();
    let sbml = std::fs::read_to_string(dir.join("model.xml")).unwrap();
    let whole = compile(
        &sbml,
        &CompileOptions {
            optimize,
            jacobian: JacobianChoice::Auto,
        },
    )
    .unwrap();
    assert_eq!(defined_functions(&whole.wasm), 14, "{cid}");
    // the Jacobian of the module as one (a model_jac whose entry is above the low limits would
    // otherwise be the difference quotient)
    let jacobian = match whole.description.jacobian {
        JacobianKind::Analytic(_) => JacobianChoice::Analytic,
        JacobianKind::ColoredDifference { .. } => JacobianChoice::DifferenceQuotient,
    };
    let options = CompileOptions { optimize, jacobian };
    let split = match compile_with_limits(&sbml, &options, limits) {
        Ok(split) => split,
        Err(error) if is_above_the_limits(&error) => return Err(error.to_string()),
        Err(error) => panic!("{cid}: {error}"),
    };
    wasmparser::Validator::new()
        .validate_all(&split.wasm)
        .unwrap_or_else(|e| panic!("{cid}: {e}"));
    let is_split = split_exports(&split.wasm);
    assert_eq!(
        defined_functions(&split.wasm) > 14,
        !is_split.is_empty(),
        "{cid}"
    );
    let json = |d| serde_json::to_string(d).unwrap();
    if json(&whole.description) != json(&split.description) {
        return Ok((Some(format!("{cid}: the descriptions differ")), is_split));
    }
    let mut req = case_request(&settings);
    req.selections = std::iter::once("time".to_string())
        .chain(whole.description.selections.iter().map(|s| s.id.clone()))
        .collect();
    let run = |wasm: &[u8]| {
        runtime()
            .instantiate(wasm, &whole.description)
            .unwrap()
            .simulate(&req)
    };
    let difference = match (run(&whole.wasm), run(&split.wasm)) {
        (Ok(a), Ok(b)) => {
            repeat_difference(&a, &b, "the split model").map(|d| format!("{cid}: {d}"))
        }
        (Err(a), Err(b)) if a.to_string() == b.to_string() => None,
        (a, b) => Some(format!(
            "{cid}: {:?} != {:?}",
            a.err().map(|e| e.to_string()),
            b.err().map(|e| e.to_string())
        )),
    };
    Ok((difference, is_split))
}

#[test]
fn split_bits_equal() {
    let Some(dirs) = passing_cases() else { return };
    runtime();
    // the limits at which most cases are split (lower ones refuse more cases, whose statement or
    // chain of helper calls is above them)
    for limits in [
        Limits {
            bytes: 160,
            locals: 12,
        },
        Limits {
            bytes: 80,
            locals: 8,
        },
    ] {
        for optimize in [true, false] {
            let (split, refused) = (AtomicUsize::new(0), AtomicUsize::new(0));
            let exports = std::sync::Mutex::new(std::collections::BTreeMap::<String, usize>::new());
            let failures: Vec<String> = dirs
                .par_iter()
                .filter_map(|dir| match compare(dir, optimize, limits) {
                    Ok((difference, split_exports)) => {
                        if !split_exports.is_empty() {
                            split.fetch_add(1, Ordering::Relaxed);
                        }
                        let mut exports = exports.lock().unwrap();
                        for name in split_exports {
                            *exports.entry(name).or_default() += 1;
                        }
                        difference
                    }
                    Err(_) => {
                        refused.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                })
                .collect();
            let (split, refused) = (split.into_inner(), refused.into_inner());
            let exports = exports.into_inner().unwrap();
            eprintln!(
                "optimize {optimize}, {limits:?}: {} cases, {split} split, {refused} with a \
                 statement above the limits; split functions: {exports:?}",
                dirs.len()
            );
            assert!(
                failures.is_empty(),
                "{} of {} cases differ:\n{}",
                failures.len(),
                dirs.len(),
                failures[..failures.len().min(20)].join("\n")
            );
            // many cases are split, most functions of model/1 in some of them
            assert!(split > 500, "{split}");
            assert!(exports.len() >= 8, "{exports:?}");
        }
    }
}

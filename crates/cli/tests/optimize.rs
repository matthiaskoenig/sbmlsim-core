//! The optimizer keeps the bits: every passing case of the SBML Test Suite compiled with and
//! without it gives bitwise equal results, with the default Jacobian and with the colored
//! difference quotient.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use sbmlsim_abi::JacobianChoice;
use sbmlsim_cli::testsuite::{Baseline, CaseSettings, case_request, repeat_difference};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_host::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// The directories of the cases which pass (not expected to fail by the baseline), `None`
/// without prepared cases.
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

/// How the optimized model of a case differs from the model as written with the Jacobian
/// `choice` (`None` for none), and the sizes of both modules.
fn compare(dir: &Path, choice: JacobianChoice) -> (Option<String>, usize, usize) {
    let cid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let settings: CaseSettings =
        serde_json::from_slice(&std::fs::read(dir.join("case.json")).unwrap()).unwrap();
    let sbml = std::fs::read_to_string(dir.join("model.xml")).unwrap();
    let options = |optimize| CompileOptions {
        optimize,
        jacobian: choice,
    };
    let written = compile(&sbml, &options(false)).unwrap();
    let optimized = compile(&sbml, &options(true)).unwrap();
    let sizes = (written.wasm.len(), optimized.wasm.len());
    // as JSON, where a NaN equals a NaN
    let json = |d| serde_json::to_string(d).unwrap();
    if json(&written.description) != json(&optimized.description) {
        return (
            Some(format!("{cid}: the descriptions differ")),
            sizes.0,
            sizes.1,
        );
    }
    // every selection of the model, at the output times of the case
    let mut req = case_request(&settings);
    req.selections = std::iter::once("time".to_string())
        .chain(written.description.selections.iter().map(|s| s.id.clone()))
        .collect();
    let run = |wasm: &[u8]| {
        runtime()
            .instantiate(wasm, &written.description)
            .unwrap()
            .simulate(&req)
    };
    let difference = match (run(&written.wasm), run(&optimized.wasm)) {
        (Ok(a), Ok(b)) => repeat_difference(&a, &b, "the optimized model")
            .map(|d| format!("{cid} ({choice:?}): {d}")),
        (Err(a), Err(b)) if a.to_string() == b.to_string() => None,
        (a, b) => Some(format!(
            "{cid} ({choice:?}): {:?} != {:?}",
            a.err().map(|e| e.to_string()),
            b.err().map(|e| e.to_string())
        )),
    };
    (difference, sizes.0, sizes.1)
}

#[test]
fn optimized_bits_equal() {
    let Some(dirs) = passing_cases() else { return };
    runtime();
    for choice in [JacobianChoice::Auto, JacobianChoice::DifferenceQuotient] {
        let (written, optimized) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let smaller = AtomicUsize::new(0);
        let failures: Vec<String> = dirs
            .par_iter()
            .filter_map(|dir| {
                let (difference, a, b) = compare(dir, choice);
                written.fetch_add(a, Ordering::Relaxed);
                optimized.fetch_add(b, Ordering::Relaxed);
                if b < a {
                    smaller.fetch_add(1, Ordering::Relaxed);
                }
                difference
            })
            .collect();
        let (written, optimized) = (written.into_inner(), optimized.into_inner());
        eprintln!(
            "{choice:?}: {} cases, model.wasm {written} bytes as written, {optimized} optimized, \
             {} smaller",
            dirs.len(),
            smaller.into_inner()
        );
        assert!(
            failures.is_empty(),
            "{} of {} cases differ:\n{}",
            failures.len(),
            dirs.len(),
            failures[..failures.len().min(20)].join("\n")
        );
        // the optimizer does something
        assert!(optimized < written, "{optimized} >= {written}");
    }
}

/// An assignment rule `id = math` of the edge model.
fn rule(id: &str, math: &str) -> String {
    format!(
        r#"<assignmentRule variable="{id}"><math xmlns="http://www.w3.org/1998/Math/MathML">{math}</math></assignmentRule>"#
    )
}

/// `<apply><op/>args</apply>`.
fn ap(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

fn ci(id: &str) -> String {
    format!("<ci>{id}</ci>")
}

fn cn(x: &str) -> String {
    format!("<cn>{x}</cn>")
}

/// The rewrites which are not exact in binary64 are not made: a model whose rules compute them
/// at `-0`, the infinities, NaN and where a reassociation rounds otherwise gives the same bits
/// with and without the optimizer (the suite has no such values).
#[test]
fn edge_values_keep_their_bits() {
    let (z, inf, nan, one, big, x) = (ci("z"), ci("inf"), ci("nan"), ci("one"), ci("big"), ci("x"));
    let (zero, mzero, unit) = (cn("0"), cn("-0"), cn("1"));
    let rules: Vec<(&str, String)> = vec![
        // not exact: none of them is rewritten
        ("r1", ap("plus", &[&z, &zero])),
        ("r2", ap("plus", &[&zero, &z])),
        ("r3", ap("minus", &[&z, &mzero])),
        ("r4", ap("times", &[&zero, &inf])),
        ("r5", ap("times", &[&inf, &zero])),
        ("r6", ap("minus", &[&inf, &inf])),
        ("r7", ap("divide", &[&inf, &inf])),
        ("r8", ap("minus", &[&ap("minus", &[&z])])),
        // (1 + 1) + 1e16 is 1e16 + 2, 1 + (1 + 1e16) would be 1e16
        ("r9", ap("plus", &[&one, &unit, &cn("1e16")])),
        // (1e308 * 10) * 0.1 is inf, 1e308 * (10 * 0.1) would be 1e308
        ("r10", ap("times", &[&big, &cn("10"), &cn("0.1")])),
        (
            "r11",
            ap("plus", &[&ap("plus", &[&one, &cn("1e16")]), &cn("-1e16")]),
        ),
        // exact: rewritten
        ("r12", ap("times", &[&nan, &unit])),
        ("r13", ap("times", &[&unit, &z])),
        ("r14", ap("divide", &[&z, &unit])),
        ("r15", ap("minus", &[&z, &zero])),
        ("r16", ap("max", &[&zero, &z, &mzero])),
        ("r17", ap("min", &[&cn("2"), &cn("1"), &z, &nan])),
        // folded constants and common subexpressions over a state
        (
            "r18",
            ap(
                "plus",
                &[
                    &ap("times", &[&ap("exp", &[&unit]), &x]),
                    &ap("exp", &[&ap("times", &[&x, &x])]),
                    &ap("exp", &[&ap("times", &[&x, &x])]),
                ],
            ),
        ),
        (
            "r19",
            ap(
                "minus",
                &[&ap("times", &[&x, &mzero]), &ap("times", &[&x, &zero])],
            ),
        ),
    ];
    let mut parameters = String::new();
    for (id, value) in [
        ("z", "-0"),
        ("inf", "INF"),
        ("nan", "NaN"),
        ("one", "1"),
        ("big", "1e308"),
    ] {
        parameters.push_str(&format!(
            r#"<parameter id="{id}" value="{value}" constant="true"/>"#
        ));
    }
    parameters.push_str(r#"<parameter id="x" value="0.5" constant="false"/>"#);
    for (id, _) in &rules {
        parameters.push_str(&format!(r#"<parameter id="{id}" constant="false"/>"#));
    }
    let rate = r#"<rateRule variable="x"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><minus/><ci>x</ci></apply></math></rateRule>"#;
    let sbml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="edges">
<listOfParameters>{parameters}</listOfParameters>
<listOfRules>{rate}{}</listOfRules></model></sbml>"#,
        rules
            .iter()
            .map(|(id, math)| rule(id, math))
            .collect::<String>()
    );
    for choice in [JacobianChoice::Auto, JacobianChoice::DifferenceQuotient] {
        let options = |optimize| CompileOptions {
            optimize,
            jacobian: choice,
        };
        let written = compile(&sbml, &options(false)).unwrap();
        let optimized = compile(&sbml, &options(true)).unwrap();
        assert!(optimized.wasm.len() < written.wasm.len());
        let mut req = sbmlsim_abi::SimulationRequest {
            contract: sbmlsim_abi::SIMULATION_CONTRACT.to_string(),
            start: 0.0,
            end: 2.0,
            output: sbmlsim_abi::Output::Uniform { steps: 10 },
            preinit: Vec::new(),
            changes: Vec::new(),
            steady_state: None,
            selections: Vec::new(),
            settings: sbmlsim_abi::IntegratorSettings::default(),
            seed: 0,
        };
        req.selections = std::iter::once("time".to_string())
            .chain(written.description.selections.iter().map(|s| s.id.clone()))
            .collect();
        let run = |wasm: &[u8]| {
            runtime()
                .instantiate(wasm, &written.description)
                .unwrap()
                .simulate(&req)
                .unwrap()
        };
        let (a, b) = (run(&written.wasm), run(&optimized.wasm));
        // the values are those of the edges
        let column = |id: &str| a.columns.iter().position(|c| c == id).unwrap();
        let first = |id: &str| a.values[column(id)];
        assert_eq!(first("r1").to_bits(), 0.0f64.to_bits());
        assert_eq!(first("r8").to_bits(), 0.0f64.to_bits());
        assert_eq!(first("r9"), 1e16 + 2.0);
        assert_eq!(first("r10"), f64::INFINITY);
        assert_eq!(first("r13").to_bits(), (-0.0f64).to_bits());
        assert!(first("r4").is_nan() && first("r12").is_nan());
        if let Some(d) = repeat_difference(&a, &b, "the optimized model") {
            panic!("{choice:?}: {d}");
        }
    }
}

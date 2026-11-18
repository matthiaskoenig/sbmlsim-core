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

/// A description as JSON (where a NaN equals a NaN) but its number of constants, which only the
/// optimizer computes: the descriptions of a model with and without it are otherwise equal.
fn but_constants(desc: &sbmlsim_abi::ModelDescription) -> String {
    let desc = sbmlsim_abi::ModelDescription {
        n_constants: 0,
        ..desc.clone()
    };
    serde_json::to_string(&desc).unwrap()
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
    if but_constants(&written.description) != but_constants(&optimized.description) {
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
    let run = |compiled: &sbmlsim_compiler::CompiledModel| {
        runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&req)
    };
    let difference = match (run(&written), run(&optimized)) {
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
            sensitivities: vec![],
        };
        req.selections = std::iter::once("time".to_string())
            .chain(written.description.selections.iter().map(|s| s.id.clone()))
            .collect();
        let run = |compiled: &sbmlsim_compiler::CompiledModel| {
            runtime()
                .instantiate(&compiled.wasm, &compiled.description)
                .unwrap()
                .simulate(&req)
                .unwrap()
        };
        let (a, b) = (run(&written), run(&optimized));
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

/// `<piecewise>` of `(value, condition)` pieces and an optional `otherwise`.
fn pw(pieces: &[(&str, &str)], otherwise: Option<&str>) -> String {
    let mut s = String::from("<piecewise>");
    for (value, condition) in pieces {
        s.push_str(&format!("<piece>{value}{condition}</piece>"));
    }
    if let Some(o) = otherwise {
        s.push_str(&format!("<otherwise>{o}</otherwise>"));
    }
    s + "</piecewise>"
}

/// The rules of literals (and of a state) of the probe of the folding: the operators at their
/// edges, which the optimizer folds and the code as written computes.
fn probe_rules() -> Vec<String> {
    const NAN: &str = "<notanumber/>";
    const INF: &str = "<infinity/>";
    const T: &str = "<true/>";
    const F: &str = "<false/>";
    let (mz, z, one, x) = (cn("-0"), cn("0"), cn("1"), ci("x"));
    let deg = |n: &str, e: &str| format!("<apply><root/><degree>{n}</degree>{e}</apply>");
    let logb = |b: &str, e: &str| format!("<apply><log/><logbase>{b}</logbase>{e}</apply>");
    let c = |x: &str| cn(x);
    let minf = ap("minus", &[INF]);
    let exp_x = ap("exp", &[&x]);
    let sin_xx = ap("sin", &[&ap("times", &[&x, &x])]);
    vec![
        ap("power", &[&c("-8"), &c("0.3333333333333333")]),
        ap("power", &[&mz, &c("-1")]),
        ap("power", &[&minf, &c("0.5")]),
        ap("root", &[&mz]),
        deg(&c("3"), &c("-8")),
        deg(&mz, &c("2")),
        ap("root", &[&minf]),
        logb(&c("10"), &c("1000")),
        logb(&c("2"), &c("8")),
        ap("log", &[&c("1000")]),
        ap("ln", &[&mz]),
        logb(&one, &c("5")),
        ap("factorial", &[&c("170.5")]),
        ap("factorial", &[&c("171")]),
        ap("factorial", &[NAN]),
        ap("factorial", &[&c("-3")]),
        ap("rem", &[&c("-7"), &c("3")]),
        ap("rem", &[&c("7"), &mz]),
        ap("rem", &[INF, &c("2")]),
        ap("rem", &[&mz, &c("3")]),
        ap("quotient", &[&c("3e9"), &one]),
        ap("quotient", &[&c("-2147483648.5"), &one]),
        ap("quotient", &[&c("-2147483649"), &one]),
        ap("quotient", &[NAN, &one]),
        ap("quotient", &[&c("7"), &z]),
        ap("quotient", &[&mz, &c("3")]),
        ap("max", &[NAN, &one]),
        ap("max", &[&one, NAN]),
        ap("max", &[&z, &mz]),
        ap("min", &[&z, &mz]),
        ap("min", &[&mz, &z]),
        ap("max", &[]),
        ap("min", &[]),
        ap("max", &[NAN, &one, &x]),
        ap("max", &[&one, NAN, &x]),
        ap("min", &[&z, &mz, &x]),
        ap("max", &[&x, NAN, &one]),
        ap("times", &[&ap("lt", &[NAN, &one]), &x]),
        ap("eq", &[NAN, NAN]),
        ap("neq", &[NAN, NAN]),
        ap("geq", &[NAN, &one]),
        ap("eq", &[&one, &one, &c("2")]),
        ap("lt", &[&one]),
        pw(&[(&one, NAN)], Some(&c("2"))),
        pw(&[(&one, &mz)], Some(&c("2"))),
        pw(&[(&one, &c("3"))], Some(&c("2"))),
        pw(&[(&one, F)], None),
        pw(&[(&x, F), (&c("5"), T)], Some(&c("9"))),
        pw(
            &[(&x, &ap("lt", &[&x, &one])), (&c("5"), NAN)],
            Some(&ap("plus", &[&x, &one])),
        ),
        ap("minus", &[&mz]),
        ap("minus", &[&ap("minus", &[&mz])]),
        ap("minus", &[&z]),
        ap("minus", &[&x, &mz]),
        ap("minus", &[&x, &z]),
        ap("arccot", &[&mz]),
        ap("arcsech", &[&c("0.7")]),
        ap("arccsch", &[&mz]),
        ap("arccoth", &[&c("1")]),
        ap("arctanh", &[&c("1")]),
        ap("arccosh", &[&c("0.5")]),
        ap("sec", &[&c("1e300")]),
        ap("coth", &[&mz]),
        ap("csch", &[&mz]),
        ap("exp", &[&c("709.8")]),
        ap("tanh", &[&c("-0")]),
        ap("plus", &[&c("0.1"), &c("0.2"), &c("0.3"), &x]),
        ap("plus", &[&x, &c("0.1"), &c("0.2")]),
        ap("times", &[&c("0.5"), &c("2"), &x]),
        ap("times", &[&x, &one, NAN]),
        ap("times", &[T, &x]),
        ap("plus", &[]),
        ap("times", &[]),
        ap("and", &[]),
        ap("or", &[]),
        ap("xor", &[T, T, T]),
        ap("implies", &[F, NAN]),
        ap("not", &[NAN]),
        ap("divide", &[&x, &one]),
        ap("divide", &[&one, &mz]),
        ap("abs", &[&mz]),
        ap("floor", &[&c("-0.5")]),
        ap("ceiling", &[&c("-0.5")]),
        // common subexpressions with lazy branches
        ap(
            "plus",
            &[
                &pw(&[(&exp_x, &ap("gt", &[&x, &c("100")]))], Some(&z)),
                &exp_x,
            ],
        ),
        pw(&[(&sin_xx, &ap("gt", &[&x, &c("100")]))], Some(&sin_xx)),
    ]
}

/// The probe of the folding: the rules of [`probe_rules`], an initial assignment of literals,
/// a rate with a piecewise and an event whose trigger and assignment hold literals.
fn probe_sbml() -> String {
    let rules = probe_rules();
    let mut parameters: String = (0..rules.len())
        .map(|i| format!(r#"<parameter id="r{i}" constant="false"/>"#))
        .collect();
    parameters.push_str(r#"<parameter id="x" value="0.5" constant="false"/>"#);
    parameters.push_str(r#"<parameter id="k" constant="true"/>"#);
    let math = |e: &str| format!(r#"<math xmlns="http://www.w3.org/1998/Math/MathML">{e}</math>"#);
    let initial = math(&ap(
        "plus",
        &[
            &cn("0.1"),
            &cn("0.2"),
            &ap("max", &["<notanumber/>", &cn("3")]),
        ],
    ));
    let rate = math(&ap(
        "times",
        &[
            &ap("minus", &[&ci("k")]),
            &ci("x"),
            &pw(
                &[(&cn("1"), &ap("gt", &[&ci("x"), &cn("0.2")]))],
                Some(&cn("0.5")),
            ),
        ],
    ));
    let trigger = math(&ap("and", &[&ap("lt", &[&ci("x"), &cn("0.3")]), "<true/>"]));
    let assignment = math(&ap(
        "plus",
        &[&ci("x"), &ap("times", &[&cn("0.1"), &cn("2")])],
    ));
    let rules: String = rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                r#"<assignmentRule variable="r{i}">{}</assignmentRule>"#,
                math(r)
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="probe">
<listOfParameters>{parameters}</listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="k">{initial}</initialAssignment></listOfInitialAssignments>
<listOfRules><rateRule variable="x">{rate}</rateRule>{rules}</listOfRules>
<listOfEvents><event id="e" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true">{trigger}</trigger><listOfEventAssignments><eventAssignment variable="x">{assignment}</eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#
    )
}

/// Literal folding gives the doubles of the code as written, not only those of the native
/// evaluator: the probe of the folding (84 rules of literals at the edges of the operators, an
/// initial assignment and an event) simulated with and without the optimizer gives the same
/// bits, with every Jacobian of the compiler and of the request.
#[test]
fn literal_folding_gives_the_bits_of_the_code() {
    let sbml = probe_sbml();
    let n_rules = probe_rules().len();
    assert_eq!(n_rules, 84);
    for compiled_with in [JacobianChoice::Auto, JacobianChoice::DifferenceQuotient] {
        let options = |optimize| CompileOptions {
            optimize,
            jacobian: compiled_with,
        };
        let written = compile(&sbml, &options(false)).unwrap();
        let optimized = compile(&sbml, &options(true)).unwrap();
        assert_eq!(
            but_constants(&written.description),
            but_constants(&optimized.description)
        );
        assert_eq!(written.description.n_constants, 0);
        assert!(optimized.description.n_constants > 0);
        assert!(optimized.wasm.len() < written.wasm.len());
        for requested in [
            sbmlsim_abi::JacobianChoice::Auto,
            sbmlsim_abi::JacobianChoice::DifferenceQuotient,
        ] {
            let req = sbmlsim_abi::SimulationRequest {
                sensitivities: vec![],
                contract: sbmlsim_abi::SIMULATION_CONTRACT.to_string(),
                start: 0.0,
                end: 3.0,
                output: sbmlsim_abi::Output::Uniform { steps: 30 },
                preinit: Vec::new(),
                changes: Vec::new(),
                steady_state: None,
                selections: std::iter::once("time".to_string())
                    .chain((0..n_rules).map(|i| format!("r{i}")))
                    .chain(["x".to_string(), "k".to_string()])
                    .collect(),
                settings: sbmlsim_abi::IntegratorSettings {
                    relative_tolerance: 1e-10,
                    absolute_tolerance: sbmlsim_abi::Tolerance::Scalar(1e-12),
                    maximum_num_steps: 20000,
                    stiff: false,
                    jacobian: requested,
                    ..sbmlsim_abi::IntegratorSettings::default()
                },
                seed: 0,
            };
            let run = |compiled: &sbmlsim_compiler::CompiledModel| {
                runtime()
                    .instantiate(&compiled.wasm, &compiled.description)
                    .unwrap()
                    .simulate(&req)
                    .unwrap()
            };
            let (a, b) = (run(&written), run(&optimized));
            assert_eq!(a.n_rows, 31);
            // the event fired: x falls below 0.3 and is raised by 0.2
            let x = a.columns.iter().position(|c| c == "x").unwrap();
            let k = a.columns.len();
            assert!((0..a.n_rows).any(|r| a.values[r * k + x] > a.values[(r.max(1) - 1) * k + x]));
            if let Some(d) = repeat_difference(&a, &b, "the optimized model") {
                panic!("{compiled_with:?} {requested:?}: {d}");
            }
        }
    }
}

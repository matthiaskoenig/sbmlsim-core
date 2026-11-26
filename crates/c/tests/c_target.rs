//! The C target against the WebAssembly model: the same description, the same right hand side
//! and Jacobian up to the math library, the same simulations within the tolerances of the
//! integration, and the cache of the compiled libraries.
//!
//! The tests over `testdata/cases` need `uv run scripts/prepare_testsuite.py` and skip with a
//! note when the directory is absent, unless `SBMLSIM_CORE_REQUIRE_CASES` is set (the jobs of
//! the suite in CI), where they fail.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use sbmlsim_abi::{
    IntegratorSettings, ModelDescription, Output, SIMULATION_CONTRACT, SimulationRequest,
    SimulationResult, Tolerance,
};
use sbmlsim_c::{BuildOptions, CError, CModel, CSource, Library, generate};
use sbmlsim_compiler::{CompileOptions, CompiledModel, compile};
use sbmlsim_engine::{Engine, ModelFns};
use sbmlsim_host::Runtime;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

fn spt() -> String {
    fs::read_to_string(testdata().join("models/spt_liver_cell.xml")).unwrap()
}

/// SplitMix64, a seeded generator of uniform numbers in `[0, 1)`.
struct Random(u64);

impl Random {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.uniform()
    }
}

/// The C source, its library and the model of it.
fn native(sbml: &str) -> (CSource, CModel) {
    let source = generate(sbml).unwrap();
    let library = Library::build(&source.code).unwrap();
    let model = CModel::new(library, &source.description).unwrap();
    (source, model)
}

/// The WebAssembly model of the options which give the description of the C target (no
/// `model_jac_p`).
fn wasm(sbml: &str) -> CompiledModel {
    let options = CompileOptions {
        parameter_jacobian: false,
        ..CompileOptions::default()
    };
    compile(sbml, &options).unwrap()
}

fn request(
    start: f64,
    end: f64,
    steps: u32,
    selections: &[String],
    preinit: Vec<(String, f64)>,
    tolerance: (f64, f64),
) -> SimulationRequest {
    SimulationRequest {
        sensitivities: vec![],
        local_errors: false,
        contract: SIMULATION_CONTRACT.to_string(),
        start,
        end,
        output: Output::Uniform { steps },
        preinit,
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
        selections: selections.to_vec(),
        settings: IntegratorSettings {
            relative_tolerance: tolerance.0,
            absolute_tolerance: Tolerance::Scalar(tolerance.1),
            ..IntegratorSettings::default()
        },
        seed: 1,
    }
}

#[test]
fn spt_matches_wasm_model() {
    let sbml = spt();
    let (source, model) = native(&sbml);
    let compiled = wasm(&sbml);
    // the description is the one of the WebAssembly target, built by the same builder
    assert_eq!(source.description, compiled.description);
    let description = &source.description;
    let mut engine = Engine::new(Box::new(model), description.clone()).unwrap();
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let states: Vec<String> = description.states.iter().map(|s| s.id.clone()).collect();
    assert_eq!(states.len(), 7);

    let mut random = Random(7);
    for set in 0..50 {
        let preinit = vec![
            ("pattern".to_string(), 7.0),
            ("protein_ext".to_string(), random.uniform()),
            ("position".to_string(), random.uniform()),
            ("[S_ext]".to_string(), random.between(0.0, 5.0)),
        ];
        let req = request(0.0, 1440.0, 99, &states, preinit.clone(), (1e-10, 1e-12));
        let c = engine.simulate(&req).unwrap();
        let w = instance.simulate(&req).unwrap();
        assert_eq!(c.columns, w.columns);
        assert_eq!(c.n_rows, 100);
        for (k, (a, b)) in c.values.iter().zip(&w.values).enumerate() {
            assert!(
                (a - b).abs() <= 1e-6 * b.abs() + 1e-9,
                "set {set} {preinit:?}: {} at row {}: C {a}, wasm {b}",
                c.columns[k % c.columns.len()],
                k / c.columns.len()
            );
        }
    }
}

/// A model of three states whose rates apply every operator of the math to them, each in a
/// domain where it is defined at states in `(0, 1)`.
fn operators() -> String {
    let ci = |id: &str| format!("<ci>{id}</ci>");
    let cn = |x: f64| format!("<cn>{x}</cn>");
    let ap = |op: &str, args: &[String]| format!("<apply><{op}/>{}</apply>", args.concat());
    let ap1 = |op: &str, arg: &str| format!("<apply><{op}/>{arg}</apply>");
    let (x, y, z) = (ci("x"), ci("y"), ci("z"));
    let one_plus = |a: &String| ap("plus", &[cn(1.0), a.clone()]);
    let terms_x = [
        ap1("exp", &x),
        ap1("ln", &y),
        ap1("log", &x),
        format!("<apply><log/><logbase>{}</logbase>{}</apply>", cn(2.0), z),
        ap("power", &[x.clone(), y.clone()]),
        ap1("root", &z),
        format!("<apply><root/><degree>{}</degree>{}</apply>", cn(3.0), y),
        ap("abs", &[ap("minus", &[x.clone(), y.clone()])]),
        ap1("sin", &x),
        ap1("cos", &y),
        ap1("tan", &z),
        ap1("sec", &x),
        ap1("csc", &y),
        ap1("cot", &z),
    ];
    let terms_y = [
        ap1("sinh", &x),
        ap1("cosh", &y),
        ap1("tanh", &z),
        ap1("sech", &x),
        ap1("csch", &y),
        ap1("coth", &z),
        ap1("arcsin", &x),
        ap1("arccos", &y),
        ap1("arctan", &z),
        ap("arcsec", &[one_plus(&x)]),
        ap("arccsc", &[one_plus(&y)]),
        ap1("arccot", &z),
    ];
    let terms_z = [
        ap1("arcsinh", &x),
        ap("arccosh", &[one_plus(&y)]),
        ap("arctanh", &[ap("times", &[cn(0.5), z.clone()])]),
        ap1("arcsech", &x),
        ap1("arccsch", &y),
        ap("arccoth", &[one_plus(&z)]),
        ap("max", &[x.clone(), y.clone(), z.clone()]),
        ap("min", &[x.clone(), y.clone()]),
        ap("rem", &[ap("times", &[cn(7.0), x.clone()]), y.clone()]),
        ap("times", &[ap("quotient", &[cn(7.0), y.clone()]), x.clone()]),
        ap(
            "times",
            &[
                ap("floor", &[ap("times", &[cn(3.0), x.clone()])]),
                y.clone(),
            ],
        ),
        ap(
            "times",
            &[
                ap("ceiling", &[ap("times", &[cn(3.0), y.clone()])]),
                z.clone(),
            ],
        ),
        ap("divide", &[x.clone(), y.clone()]),
        format!(
            "<piecewise><piece>{}{}</piece><piece>{}{}</piece><otherwise>{}</otherwise></piecewise>",
            ap("times", &[x.clone(), y.clone()]),
            ap(
                "and",
                &[
                    ap("lt", &[x.clone(), y.clone()]),
                    ap("gt", &[z.clone(), cn(0.5)])
                ]
            ),
            ap("times", &[x.clone(), z.clone()]),
            ap(
                "or",
                &[
                    ap("eq", &[x.clone(), z.clone()]),
                    ap(
                        "xor",
                        &[
                            ap("geq", &[y.clone(), z.clone()]),
                            ap("leq", &[x.clone(), cn(0.25)])
                        ]
                    ),
                    ap(
                        "implies",
                        &[
                            ap("neq", &[x.clone(), y.clone()]),
                            ap("not", &[ap("lt", &[x.clone(), y.clone(), z.clone()])])
                        ]
                    ),
                ]
            ),
            ap1("minus", &y),
        ),
    ];
    let rate = |id: &str, terms: &[String]| {
        format!(
            r#"<rateRule variable="{id}"><math {MATH}>{}</math></rateRule>"#,
            ap("plus", terms)
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
  <model id="operators">
    <listOfParameters>
      <parameter id="x" value="0.3" constant="false"/>
      <parameter id="y" value="0.6" constant="false"/>
      <parameter id="z" value="0.8" constant="false"/>
    </listOfParameters>
    <listOfRules>{}{}{}</listOfRules>
  </model>
</sbml>"#,
        rate("x", &terms_x),
        rate("y", &terms_y),
        rate("z", &terms_z)
    )
}

/// Whether two values agree within `relative` of the larger one, NaN with NaN.
fn agrees(a: f64, b: f64, relative: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan()) || (a - b).abs() <= relative * a.abs().max(b.abs())
}

/// The right hand side and the Jacobian of the C model and of the WebAssembly model at 100
/// random states and times: the same expressions in the same order, the math functions of
/// `<math.h>` against those of mathlib.
fn compare_functions(sbml: &str, state: impl Fn(&mut Random, f64) -> f64) {
    let (source, model) = native(sbml);
    let compiled = wasm(sbml);
    assert_eq!(source.description, compiled.description);
    let description: &ModelDescription = &source.description;
    let mut functions = runtime()
        .model_functions(&compiled.wasm, &compiled.description)
        .unwrap();
    let parameters: Vec<f64> = description.parameters.iter().map(|e| e.initial).collect();
    let n_constants = description.n_constants as usize;
    let nnz = description.jacobian.pattern().row_idx.len();
    let mut random = Random(13);
    for _ in 0..100 {
        let t = random.between(0.0, 100.0);
        let y: Vec<f64> = description
            .states
            .iter()
            .map(|e| state(&mut random, e.initial))
            .collect();
        let rhs = functions.rhs(t, &y, &parameters).unwrap();
        let jac = functions.jac(t, &y, &parameters).unwrap();

        let mut cy = y.clone();
        let mut cp = parameters.clone();
        cp.resize(parameters.len() + n_constants, 0.0);
        model.assign(t, &mut cy, &mut cp);
        let mut c_rhs = vec![0.0; y.len()];
        model.rhs(t, &cy, &cp, &mut c_rhs);
        let mut c_jac = vec![0.0; nnz];
        model.jac(t, &cy, &cp, &mut c_jac);

        for (k, (a, b)) in c_rhs.iter().zip(&rhs).enumerate() {
            assert!(
                agrees(*a, *b, 1e-13),
                "rhs {k} at t {t}, y {y:?}: {a} != {b}"
            );
        }
        for (k, (a, b)) in c_jac.iter().zip(&jac).enumerate() {
            assert!(
                agrees(*a, *b, 1e-13),
                "jac {k} at t {t}, y {y:?}: {a} != {b}"
            );
        }
    }
}

#[test]
fn rhs_and_jacobian_match_wasm() {
    compare_functions(&spt(), |random, initial| {
        initial.abs().max(1e-3) * random.between(0.0, 2.0)
    });
    compare_functions(&operators(), |random, _| random.between(0.05, 0.95));
}

/// A model of expressions nested deeper than [`sbmlsim_c::MAX_NESTING`]: a chain of 97
/// operations on a `piecewise` (in the rate, which adds a level, the `piecewise` is at twice
/// the nesting, where it goes into a local as a lazy chain of `if`s), a `piecewise` of 60
/// pieces and a `max` of 60 arguments (of the time, so its derivative is no piecewise of 60
/// levels).
fn deep() -> String {
    let mut chain = "<piecewise><piece><ci>x</ci><apply><lt/><ci>x</ci><cn>0.5</cn></apply></piece><otherwise><apply><times/><cn>2</cn><ci>x</ci></apply></otherwise></piecewise>".to_string();
    for k in 0..97 {
        let op = if k % 2 == 0 { "times" } else { "plus" };
        chain = format!("<apply><{op}/>{chain}<cn>1.001</cn></apply>");
    }
    let pieces: String = (0..60)
        .map(|k| {
            format!(
                "<piece><cn>{k}</cn><apply><lt/><ci>x</ci><cn>{}</cn></apply></piece>",
                f64::from(k) / 60.0
            )
        })
        .collect();
    let arguments: String = (0..60)
        .map(|k| {
            format!(
                r#"<apply><times/><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol><cn>{}</cn></apply>"#,
                f64::from(k % 7)
            )
        })
        .collect();
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="deep"><listOfParameters><parameter id="x" value="0.5" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}><apply><plus/>{chain}<piecewise>{pieces}<otherwise><cn>-1</cn></otherwise></piecewise><apply><max/>{arguments}</apply></apply></math></rateRule></listOfRules></model></sbml>"#
    )
}

#[test]
fn deep_expressions_stay_within_the_nesting_of_compilers() {
    let source = generate(&deep()).unwrap();
    let mut depth = 0usize;
    let mut deepest = 0;
    for c in source.code.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    // clang refuses 256 nested brackets
    assert!(deepest < 200, "{deepest}");
    assert!(source.code.contains("do {"), "{}", source.code);
    compare_functions(&deep(), |random, _| random.between(0.0, 1.0));
}

/// The directory of the prepared cases, `None` (with a note) where it is absent.
fn cases() -> Option<PathBuf> {
    let dir = testdata().join("cases");
    if dir.join("00001/case.json").is_file() {
        return Some(dir);
    }
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

/// Simulates a case of the suite on the native engine with the C model, at the tolerance of
/// the runner (1e-12), and compares every value but the time with its expected results by the
/// rule of the suite.
fn check_case(dir: &Path) {
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("case.json")).unwrap()).unwrap();
    let number = |key: &str| settings[key].as_f64().unwrap();
    let selections: Vec<String> = settings["selections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let sbml = fs::read_to_string(dir.join("model.xml")).unwrap();
    let (source, model) = native(&sbml);
    let mut engine = Engine::new(Box::new(model), source.description).unwrap();
    let start = number("start");
    let req = request(
        start,
        start + number("duration"),
        settings["steps"].as_u64().unwrap() as u32,
        &selections,
        vec![],
        (1e-12, 1e-12),
    );
    let observed: SimulationResult = engine.simulate(&req).unwrap();

    let expected = fs::read_to_string(dir.join("expected.csv")).unwrap();
    let rows: Vec<Vec<f64>> = expected
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').map(|x| x.trim().parse().unwrap()).collect())
        .collect();
    let (abs_tol, rel_tol) = (number("absolute_tolerance"), number("relative_tolerance"));
    let n = selections.len();
    assert_eq!(observed.n_rows, rows.len(), "{}", dir.display());
    for (r, row) in rows.iter().enumerate() {
        for (c, expected) in row.iter().enumerate().skip(1) {
            let value = observed.values[r * n + c];
            assert!(
                (expected - value).abs() <= abs_tol + rel_tol * expected.abs(),
                "{}: {} at row {r}: {value}, expected {expected}",
                dir.display(),
                selections[c]
            );
        }
    }
}

#[test]
fn event_model_matches_wasm() {
    let Some(cases) = cases() else { return };
    // events: a delay (00071), priorities (00934), an event which assigns a compartment of a
    // species of a concentration (01506); an algebraic rule (00039)
    for case in ["00071", "00934", "01506", "00039"] {
        check_case(&cases.join(case));
    }
}

/// A small model: `y' = -k y` with an assignment rule.
fn decay() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="decay_cache"><listOfParameters><parameter id="k" value="0.5" constant="true"/><parameter id="y" value="1" constant="false"/><parameter id="h" constant="false"/></listOfParameters><listOfRules><assignmentRule variable="h"><math {MATH}><apply><divide/><cn>0.7</cn><ci>k</ci></apply></math></assignmentRule><rateRule variable="y"><math {MATH}><apply><times/><apply><minus/><ci>k</ci></apply><ci>y</ci></apply></math></rateRule></listOfRules></model></sbml>"#
    )
}

#[test]
fn cache_reuses_library() {
    let cache = tempfile::tempdir().unwrap();
    let options = BuildOptions {
        cache_dir: cache.path().to_path_buf(),
        ..BuildOptions::from_env().unwrap()
    };
    let source = generate(&decay()).unwrap();
    let first = Library::build_with(&source.code, &options).unwrap();
    let path = first.path().to_path_buf();
    assert!(path.starts_with(cache.path()), "{}", path.display());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    // unloaded, so the second build finds the library in the cache directory
    drop(first);

    let begin = Instant::now();
    let second = Library::build_with(&source.code, &options).unwrap();
    let elapsed = begin.elapsed();
    assert!(elapsed < Duration::from_millis(50), "{elapsed:?}");
    assert_eq!(second.path(), path);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let libraries = fs::read_dir(cache.path())
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name.to_string_lossy()
                .ends_with(std::env::consts::DLL_SUFFIX)
        })
        .count();
    assert_eq!(libraries, 1);

    // a library which is loaded is the one of the process
    let third = Library::build_with(&source.code, &options).unwrap();
    assert!(Arc::ptr_eq(&second, &third));
    let model = CModel::new(third, &source.description).unwrap();
    let mut engine = Engine::new(Box::new(model), source.description.clone()).unwrap();
    let ids = vec!["y".to_string(), "h".to_string()];
    let result = engine
        .simulate(&request(0.0, 2.0, 2, &ids, vec![], (1e-10, 1e-12)))
        .unwrap();
    // the columns are the time, y and h; the row of t = 2
    assert_eq!(result.columns, ["time", "y", "h"]);
    assert!(
        (result.values[7] - (-1.0f64).exp()).abs() < 1e-8,
        "{result:?}"
    );
    assert_eq!(result.values[8], 0.7 / 0.5);
}

#[test]
fn missing_compiler_is_an_error() {
    let cache = tempfile::tempdir().unwrap();
    let options = BuildOptions {
        compiler: "/nonexistent/cc".to_string(),
        cache_dir: cache.path().to_path_buf(),
    };
    let source = generate(&decay()).unwrap();
    match Library::build_with(&source.code, &options) {
        Err(error @ CError::NoCompiler { .. }) => {
            assert_eq!(
                error,
                CError::NoCompiler {
                    command: "/nonexistent/cc".to_string()
                }
            );
            assert!(error.to_string().contains("/nonexistent/cc"), "{error}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn refusals_name_their_feature() {
    let model = |math: &str| {
        format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m"><listOfParameters><parameter id="x" value="1" constant="false"/></listOfParameters><listOfRules><rateRule variable="x"><math {MATH}>{math}</math></rateRule></listOfRules></model></sbml>"#
        )
    };
    let delay = model(
        r#"<apply><minus/><apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>x</ci><cn>1</cn></apply></apply>"#,
    );
    match generate(&delay) {
        Err(CError::Unsupported { feature }) => assert!(feature.contains("delay"), "{feature}"),
        other => panic!("{other:?}"),
    }
    let factorial = model("<apply><factorial/><ci>x</ci></apply>");
    match generate(&factorial) {
        Err(CError::Unsupported { feature }) => {
            assert!(feature.contains("analytic Jacobian"), "{feature}")
        }
        other => panic!("{other:?}"),
    }
    // a refusal of the reader or of the model representation is one of the model
    assert!(matches!(generate("<sbml"), Err(CError::Model(_))));
}

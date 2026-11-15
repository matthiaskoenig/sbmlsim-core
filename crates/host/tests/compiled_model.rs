//! Models of `sbmlsim_compiler::compile` in the chain mathlib -> model -> engine.

use std::path::Path;

use sbmlsim_abi::{IntegratorSettings, Output, SIMULATION_CONTRACT, SimulationRequest};
use sbmlsim_compiler::{CompileOptions, compile};
use sbmlsim_engine::model::Decay;
use sbmlsim_host::Runtime;

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::embedded().unwrap())
}

fn request(end: f64, steps: u32, selections: &[&str]) -> SimulationRequest {
    SimulationRequest {
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output: Output::Uniform { steps },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        selections: selections.iter().map(|s| s.to_string()).collect(),
        settings: IntegratorSettings {
            relative_tolerance: 1e-10,
            stiff: false,
            ..IntegratorSettings::default()
        },
        seed: 0,
    }
}

/// The parameters `x` (value) and `r<i>` (an initial assignment of each formula over `x`, `y`
/// and `z`) of a model without states.
fn formulas(x: f64, formulas: &[String]) -> String {
    let parameters: String = formulas
        .iter()
        .enumerate()
        .map(|(i, _)| format!(r#"<parameter id="r{i}" constant="true"/>"#))
        .collect();
    let assignments: String = formulas
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                r#"<initialAssignment symbol="r{i}"><math {MATH}>{f}</math></initialAssignment>"#
            )
        })
        .collect();
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="f"><listOfParameters><parameter id="x" value="{x:e}" constant="true"/><parameter id="y" value="1" constant="true"/><parameter id="z" value="0" constant="true"/>{parameters}</listOfParameters><listOfInitialAssignments>{assignments}</listOfInitialAssignments></model></sbml>"#
    )
}

/// `<apply><op/>args</apply>`.
fn ap(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

const X: &str = "<ci>x</ci>";
const Y: &str = "<ci>y</ci>";
const Z: &str = "<ci>z</ci>";

/// Every operator of the model representation, as unary functions of `x` and over `y = 1`,
/// `z = 0` and NaN.
fn every_operator() -> Vec<String> {
    let nan = "<notanumber/>";
    let mut f: Vec<String> = [
        "abs",
        "exp",
        "ln",
        "floor",
        "ceiling",
        "factorial",
        "sin",
        "cos",
        "tan",
        "sec",
        "csc",
        "cot",
        "sinh",
        "cosh",
        "tanh",
        "sech",
        "csch",
        "coth",
        "arcsin",
        "arccos",
        "arctan",
        "arcsec",
        "arccsc",
        "arccot",
        "arcsinh",
        "arccosh",
        "arctanh",
        "arcsech",
        "arccsch",
        "arccoth",
        "root",
        "log",
        "minus",
        "not",
    ]
    .iter()
    .map(|op| ap(op, &[X]))
    .collect();
    f.extend([
        ap("plus", &[]),
        ap("plus", &[X, Y, X]),
        ap("times", &[]),
        ap("times", &[X, X, Y]),
        ap("minus", &[X, Y]),
        ap("divide", &[X, "<cn>49</cn>"]),
        ap("power", &[X, "<cn>0.5</cn>"]),
        format!("<apply><root/><degree><cn>3</cn></degree>{X}</apply>"),
        format!("<apply><log/><logbase><cn>2</cn></logbase>{X}</apply>"),
        ap("max", &[X, nan, Y]),
        ap("min", &[nan, X]),
        ap("max", &[Z, "<cn>-0</cn>"]),
        ap("max", &[]),
        ap("min", &[]),
        ap("rem", &[X, Y]),
        ap("quotient", &[X, Z]),
        ap("quotient", &["<cn>-7</cn>", "<cn>2</cn>"]),
        ap("eq", &[nan, nan]),
        ap("eq", &[Y, Y, X]),
        ap("neq", &[X, nan]),
        ap("lt", &[Z, Y, X]),
        ap("gt", &[X, Y]),
        ap("geq", &[nan, Y]),
        ap("leq", &[X, Y]),
        ap("and", &[]),
        ap("or", &[X, Z]),
        ap("xor", &[X, Y, Y]),
        ap("implies", &[Z, X]),
        ap("not", &[nan]),
        ap("arccot", &[Z]),
        format!("<piecewise><piece>{X}<apply><gt/>{X}{Y}</apply></piece></piecewise>"),
        format!("<piecewise><piece>{X}{nan}</piece><otherwise>{Y}</otherwise></piecewise>"),
        format!(
            "<piecewise><piece>{X}{Z}</piece><piece>{Y}{X}</piece><otherwise>{Z}</otherwise></piecewise>"
        ),
    ]);
    f
}

#[test]
fn the_initial_values_of_the_description_are_the_bits_of_model_init() {
    for x in [0.37, -0.81, 2.6, 0.0, -0.0, 1e300, f64::INFINITY] {
        let fs = every_operator();
        let sbml = formulas(x, &fs);
        let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
        let ids: Vec<String> = (0..fs.len()).map(|i| format!("r{i}")).collect();
        let selections: Vec<&str> = ids.iter().map(String::as_str).collect();
        let result = runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&request(1.0, 1, &selections))
            .unwrap();
        for (i, id) in ids.iter().enumerate() {
            let simulated = result.values[1 + i];
            let p = &compiled.description.parameters;
            let initial = p.iter().find(|e| &e.id == id).unwrap().initial;
            assert!(
                simulated.to_bits() == initial.to_bits()
                    || (simulated.is_nan() && initial.is_nan()),
                "x = {x}, {}: model_init {simulated:e}, description {initial:e}",
                fs[i]
            );
        }
    }
}

#[test]
fn the_sbml_of_decay_wat_computes_its_bits() {
    let sbml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay.xml"))
            .unwrap();
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let req = request(10.0, 10, &["time", "y", "t_half"]);
    let wasm = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&req)
        .unwrap();
    // the native decay model computes -k * y and ln(2) / k, as the SBML does
    let native = Decay { k: 0.5 };
    let expected = sbmlsim_engine::simulate(&native, &native.description(), &req).unwrap();
    assert_eq!(wasm.columns, expected.columns);
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&wasm.values), bits(&expected.values));
}

#[test]
fn a_later_time_column_is_the_entity_time() {
    let sbml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model><listOfParameters><parameter id="time" value="10" constant="true"/></listOfParameters></model></sbml>"#;
    let compiled = compile(sbml, &CompileOptions::default()).unwrap();
    let result = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&request(1.0, 1, &["time", "time"]))
        .unwrap();
    assert_eq!(result.values, [0.0, 10.0, 1.0, 10.0]);
}

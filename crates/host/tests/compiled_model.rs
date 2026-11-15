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

/// A model of events: the compartment `C` (1), the species `S` (a concentration, 1) and `R` (a
/// concentration of the rate rule `R' = 0`, 1) in it, the parameters `n` and `x` and the
/// assignment rule `a = 2 time`.
///
/// - `E1`: `time >= 1` (initial value false): `S = 3`, `C = 2`, `n = n + 1`;
/// - `E2`: `a > 3` (at 1.5), the delay 0.25, values at the trigger: `x = a`.
fn event_model() -> String {
    let time = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="ev">
<listOfCompartments><compartment id="C" size="1" spatialDimensions="3" constant="false"/></listOfCompartments>
<listOfSpecies>
<species id="S" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="R" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
</listOfSpecies>
<listOfParameters><parameter id="n" value="0" constant="false"/><parameter id="x" value="0" constant="false"/><parameter id="a" constant="false"/></listOfParameters>
<listOfRules>
<rateRule variable="R"><math {MATH}><cn>0</cn></math></rateRule>
<assignmentRule variable="a"><math {MATH}><apply><times/><cn>2</cn>{time}</apply></math></assignmentRule>
</listOfRules>
<listOfEvents>
<event id="E1" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}><apply><geq/>{time}<cn>1</cn></apply></math></trigger>
<listOfEventAssignments>
<eventAssignment variable="S"><math {MATH}><cn>3</cn></math></eventAssignment>
<eventAssignment variable="C"><math {MATH}><cn>2</cn></math></eventAssignment>
<eventAssignment variable="n"><math {MATH}><apply><plus/><ci>n</ci><cn>1</cn></apply></math></eventAssignment>
</listOfEventAssignments></event>
<event id="E2" useValuesFromTriggerTime="true"><trigger initialValue="true" persistent="true"><math {MATH}><apply><gt/><ci>a</ci><cn>3</cn></apply></math></trigger>
<delay><math {MATH}><cn>0.25</cn></math></delay>
<listOfEventAssignments><eventAssignment variable="x"><math {MATH}><ci>a</ci></math></eventAssignment></listOfEventAssignments></event>
</listOfEvents></model></sbml>"#
    )
}

#[test]
fn compiled_events_execute() {
    let compiled = compile(&event_model(), &CompileOptions::default()).unwrap();
    assert_eq!(compiled.description.n_roots, 2);
    let req = request(2.0, 8, &["time", "S", "[S]", "R", "[R]", "C", "n", "x"]);
    let result = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&req)
        .unwrap();
    let k = result.columns.len();
    let row = |i: usize| &result.values[i * k..(i + 1) * k];
    // before the event at 1
    assert_eq!(row(3), [0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
    // at 1 (time >= 1 holds) the output sees the event: the amount of S is 3 times the
    // compartment before the event, R keeps its amount in the compartment of 2
    assert_eq!(row(4), [1.0, 3.0, 1.5, 1.0, 0.5, 2.0, 1.0, 0.0]);
    // E2 is triggered at 1.5 (a = 3, the strict comparison right after) and executes at 1.75
    // with the value at its trigger
    assert_eq!(row(6)[7], 0.0);
    let x = row(7)[7];
    assert!((x - 3.0).abs() < 1e-9, "{x}");
    assert_eq!(row(8)[6], 1.0, "E1 executes once");
    // another instance gives the same result
    let again = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&req)
        .unwrap();
    assert_eq!(again, result);
}

#[test]
fn a_description_of_another_model_is_refused() {
    let compiled = compile(&event_model(), &CompileOptions::default()).unwrap();
    let mut desc = compiled.description.clone();
    desc.n_roots += 1;
    let error = runtime()
        .instantiate(&compiled.wasm, &desc)
        .err()
        .expect("refused")
        .to_string();
    assert!(
        error.contains("2 root functions") && error.contains("3 root functions"),
        "{error}"
    );
}

/// A model of the compartment `C` (of the assignment rule `rule`, or of the size 1 without),
/// the species `S` (a concentration of the rate rule `S' = 0`) and `A` (a concentration of no
/// rule) in it, both 1, the constant compartment `D` with the boundary species `X` (an amount,
/// 0), the parameter `k` (1) and the event `time > 0.5` of the given assignments.
fn compartment_model(rule: Option<&str>, assignments: &[(&str, &str)]) -> String {
    let time = r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;
    let (size, rule) = match rule {
        Some(math) => (
            String::new(),
            format!(r#"<assignmentRule variable="C"><math {MATH}>{math}</math></assignmentRule>"#),
        ),
        None => (r#"size="1""#.to_string(), String::new()),
    };
    let assignments: String = assignments
        .iter()
        .map(|(v, x)| {
            format!(r#"<eventAssignment variable="{v}"><math {MATH}><cn>{x}</cn></math></eventAssignment>"#)
        })
        .collect();
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="cm">
<listOfCompartments><compartment id="C" {size} spatialDimensions="3" constant="false"/><compartment id="D" size="1" spatialDimensions="3" constant="true"/></listOfCompartments>
<listOfSpecies>
<species id="S" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="A" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="X" compartment="D" initialAmount="0" hasOnlySubstanceUnits="true" boundaryCondition="true" constant="false"/>
</listOfSpecies>
<listOfParameters><parameter id="k" value="1" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="S"><math {MATH}><cn>0</cn></math></rateRule>{rule}</listOfRules>
<listOfEvents><event id="e" useValuesFromTriggerTime="true"><trigger initialValue="true" persistent="true"><math {MATH}><apply><gt/>{time}<cn>0.5</cn></apply></math></trigger>
<listOfEventAssignments>{assignments}</listOfEventAssignments></event></listOfEvents></model></sbml>"#
    )
}

/// The rule of the compartment, the assignments of the event and the values at 1.
type Shape<'a> = (Option<&'a str>, &'a [(&'a str, &'a str)], [f64; 5]);

#[test]
fn species_keep_their_amounts_when_an_event_changes_their_compartment() {
    // [S, [S], A, [A], C] at 1, the values of libroadrunner 2.11.1 (Task 13 report, fix round 1)
    let k = Some("<ci>k</ci>");
    let x = Some("<apply><plus/><cn>1</cn><ci>X</ci></apply>");
    let cases: [Shape; 8] = [
        // the compartment of an assignment rule reading a parameter the event sets
        (k, &[("k", "2")], [1.0, 0.5, 1.0, 0.5, 2.0]),
        // ... reading a species the event sets
        (x, &[("X", "1")], [1.0, 0.5, 1.0, 0.5, 2.0]),
        // a concentration assigned after the parameter sees the compartment after it, one
        // assigned before it the compartment before it
        (k, &[("k", "2"), ("S", "3")], [6.0, 3.0, 1.0, 0.5, 2.0]),
        (k, &[("S", "3"), ("k", "2")], [3.0, 1.5, 1.0, 0.5, 2.0]),
        (k, &[("k", "2"), ("A", "3")], [1.0, 0.5, 6.0, 3.0, 2.0]),
        (k, &[("A", "3"), ("k", "2")], [1.0, 0.5, 3.0, 1.5, 2.0]),
        // a compartment the event assigns is assigned last: the amounts are the values times
        // the compartment before the event, in either order
        (
            None,
            &[("C", "2"), ("S", "3"), ("A", "3")],
            [3.0, 1.5, 3.0, 1.5, 2.0],
        ),
        (
            None,
            &[("S", "3"), ("A", "3"), ("C", "2")],
            [3.0, 1.5, 3.0, 1.5, 2.0],
        ),
    ];
    for (rule, assignments, expected) in cases {
        let compiled = compile(
            &compartment_model(rule, assignments),
            &CompileOptions::default(),
        )
        .unwrap();
        let result = runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&request(1.0, 2, &["time", "S", "[S]", "A", "[A]", "C"]))
            .unwrap();
        let last = &result.values[result.values.len() - 6..];
        assert_eq!(last[1..], expected, "{rule:?} {assignments:?}");
        // before the event every value is 1
        assert_eq!(result.values[7..12], [1.0; 5], "{rule:?} {assignments:?}");
    }
}

#[test]
fn a_description_of_other_assignments_is_refused() {
    // the same total of assignments, split otherwise over the events
    let compiled = compile(&event_model(), &CompileOptions::default()).unwrap();
    let mut desc = compiled.description.clone();
    assert_eq!(
        (desc.events[0].n_assignments, desc.events[1].n_assignments),
        (3, 1)
    );
    desc.events[0].n_assignments = 1;
    desc.events[1].n_assignments = 3;
    let error = runtime()
        .instantiate(&compiled.wasm, &desc)
        .err()
        .expect("refused")
        .to_string();
    assert!(
        error.contains("[3, 1]") && error.contains("[1, 3]"),
        "{error}"
    );
}

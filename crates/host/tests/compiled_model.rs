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
        sensitivities: vec![],
        contract: SIMULATION_CONTRACT.to_string(),
        start: 0.0,
        end,
        output: Output::Uniform { steps },
        preinit: vec![],
        changes: vec![],
        steady_state: None,
        steady_state_after: None,
        before_events: false,
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
fn the_uncertainty_of_a_fitted_parameter_changes_no_value() {
    // decay.xml with the required distrib package and the uncertainty of the fit which gave
    // `k`, as the models of pkdb_models write it: the elements annotate the parameter and the
    // compiled model is the one of the document without them, bit for bit
    let sbml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay.xml"))
            .unwrap();
    let distrib = sbml
        .replace(
            r#"level="3" version="2""#,
            r#"xmlns:distrib="http://www.sbml.org/sbml/level3/version1/distrib/version1" level="3" version="2" distrib:required="true""#,
        )
        .replace(
            r#"<parameter id="k" value="0.5" constant="true"/>"#,
            r#"<parameter metaid="meta_k" id="k" value="0.5" constant="true">
        <distrib:listOfUncertainties>
          <distrib:uncertainty distrib:name="fit PK_20261028_003811__eb362">
            <distrib:uncertSpan distrib:name="95 % confidence interval (profile likelihood)" distrib:units="dimensionless" distrib:type="confidenceInterval" distrib:valueLower="0.449782163912786" distrib:valueUpper="0.557640733722810"/>
            <distrib:uncertParameter distrib:name="standard error (Fisher information)" distrib:value="0.029663600785681" distrib:units="dimensionless" distrib:type="standardError"/>
            <distrib:uncertParameter distrib:name="distribution of the draws" distrib:type="distribution">
              <math xmlns="http://www.w3.org/1998/Math/MathML">
                <apply>
                  <csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/distrib/lognormal"> lognormal </csymbol>
                  <cn> -0.693147180559945 </cn>
                  <cn> 0.059327201571362 </cn>
                  <cn> 0.1 </cn>
                  <cn> 10 </cn>
                </apply>
              </math>
            </distrib:uncertParameter>
          </distrib:uncertainty>
        </distrib:listOfUncertainties>
      </parameter>"#,
        );
    assert!(distrib.contains("distrib:uncertainty"), "{distrib}");
    let req = request(10.0, 10, &["time", "y", "t_half"]);
    let values = |sbml: &str| {
        let compiled = compile(sbml, &CompileOptions::default()).unwrap();
        assert_eq!(
            compiled
                .description
                .parameters
                .iter()
                .find(|p| p.id == "k")
                .map(|p| p.initial),
            Some(0.5)
        );
        runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&req)
            .unwrap()
    };
    let with = values(&distrib);
    let without = values(&sbml);
    assert_eq!(with.columns, without.columns);
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&with.values), bits(&without.values));
    // the ODE is simulated: y(t) = exp(-k t)
    let k = 0.5;
    let y: Vec<f64> = with.values.chunks(3).map(|row| row[1]).collect();
    for (i, y) in y.iter().enumerate() {
        let expected = (-k * i as f64).exp();
        assert!((y - expected).abs() < 1e-8, "y({i}) = {y}, not {expected}");
    }
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
    // [S, [S], A, [A], C] at 1, the values of libroadrunner 2.11.1
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

/// The compartment `C` (size 1, not constant) with the species `S` (a concentration of no rule,
/// stored as its amount), `R` (a concentration of the rate rule `R' = 0`, stored as its
/// concentration) and `H` (an amount, `hasOnlySubstanceUnits`), every one 1, the reaction
/// `S -> ` of the rate `k S` and the parameter `k` (0).
fn species_model() -> String {
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="sp">
<listOfCompartments><compartment id="C" size="1" spatialDimensions="3" constant="false"/></listOfCompartments>
<listOfSpecies>
<species id="S" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="R" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
<species id="H" compartment="C" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/>
</listOfSpecies>
<listOfParameters><parameter id="k" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="R"><math {MATH}><cn>0</cn></math></rateRule></listOfRules>
<listOfReactions><reaction id="J" reversible="false"><listOfReactants><speciesReference species="S" stoichiometry="1" constant="true"/></listOfReactants>
<kineticLaw><math {MATH}><apply><times/><ci>k</ci><ci>S</ci></apply></math></kineticLaw></reaction></listOfReactions>
</model></sbml>"#
    )
}

/// The values of a change and the values after it.
type ChangeCase<'a> = (&'a [(&'a str, f64)], [f64; 7]);

#[test]
fn compiled_changes_keep_concentrations() {
    // the values of libroadrunner 2.11.1 through sbmlsim: a
    // compartment set by a change keeps the concentrations of its species of a concentration,
    // the species of an amount keeps its amount; `R` is the amount, `[R]` the concentration
    let compiled = compile(&species_model(), &CompileOptions::default()).unwrap();
    let selections = ["time", "S", "[S]", "R", "[R]", "H", "[H]", "C"];
    let cases: [ChangeCase; 4] = [
        (&[("C", 2.0)], [2.0, 1.0, 2.0, 1.0, 1.0, 0.5, 2.0]),
        (
            &[("C", 2.0), ("[S]", 3.0)],
            [6.0, 3.0, 2.0, 1.0, 1.0, 0.5, 2.0],
        ),
        (
            &[("C", 2.0), ("S", 3.0)],
            [3.0, 1.5, 2.0, 1.0, 1.0, 0.5, 2.0],
        ),
        (
            &[("C", 2.0), ("R", 3.0)],
            [2.0, 1.0, 3.0, 1.5, 1.0, 0.5, 2.0],
        ),
    ];
    for (values, expected) in cases {
        let mut req = request(10.0, 2, &selections);
        req.changes = vec![sbmlsim_abi::Change {
            time: 5.0,
            values: values.iter().map(|(id, v)| (id.to_string(), *v)).collect(),
            increments: Vec::new(),
        }];
        let result = runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&req)
            .unwrap();
        assert_eq!(result.n_rows, 4, "{values:?}");
        let k = result.columns.len();
        let row = |i: usize| &result.values[i * k + 1..(i + 1) * k];
        assert_eq!(row(1), [1.0; 7], "{values:?}");
        assert_eq!(row(2), expected, "{values:?}");
        assert_eq!(row(3), expected, "{values:?}");
    }
}

#[test]
fn requests_on_one_instance_are_independent() {
    // requests A, B, A, B on one instance of the wasm engine: every result is the one of a
    // fresh instance, bitwise; B has events, a pre-initialization, changes and a steady state
    let compiled = compile(&event_model(), &CompileOptions::default()).unwrap();
    let a = request(2.0, 8, &["time", "S", "[S]", "R", "[R]", "C", "n", "x"]);
    let mut b = request(3.0, 6, &[]);
    b.output = Output::IntegratorSteps;
    b.preinit = vec![("n".to_string(), 5.0)];
    b.changes = vec![sbmlsim_abi::Change {
        time: 0.5,
        values: vec![("C".to_string(), 4.0), ("x".to_string(), 1.0)],
        increments: Vec::new(),
    }];
    b.steady_state = Some(sbmlsim_abi::SteadyState {
        max_time: 100.0,
        ..sbmlsim_abi::SteadyState::default()
    });
    b.seed = 3;
    b.settings.stiff = true;
    let fresh = |req: &SimulationRequest| {
        runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(req)
            .unwrap()
    };
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let a1 = instance.simulate(&a).unwrap();
    let b1 = instance.simulate(&b).unwrap();
    let a2 = instance.simulate(&a).unwrap();
    let b2 = instance.simulate(&b).unwrap();
    let bits = |r: &sbmlsim_abi::SimulationResult| {
        r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(bits(&a1), bits(&a2));
    assert_eq!(bits(&a1), bits(&fresh(&a)));
    assert_eq!(bits(&b1), bits(&b2));
    assert_eq!(bits(&b1), bits(&fresh(&b)));
    // B ran its steady state (E1 executed there: n = 5 + 1), its change and its events
    let n = b1.columns.iter().position(|c| c == "n").unwrap();
    assert_eq!(b1.values[n], 6.0);
}

#[test]
fn compiled_preinit_takes_the_selections_of_sbmlsim() {
    // the values of libroadrunner 2.11.1 through sbmlsim, the compartment of the size 2: `R` the amount, `[R]` the concentration of the
    // species of the rate rule, `S` and `[S]` of the species without a rule
    let sbml = species_model().replace(r#"id="C" size="1""#, r#"id="C" size="2""#);
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let cases: [(&str, [f64; 4]); 4] = [
        ("R", [2.0, 1.0, 3.0, 1.5]),
        ("[R]", [2.0, 1.0, 6.0, 3.0]),
        ("S", [3.0, 1.5, 2.0, 1.0]),
        ("[S]", [6.0, 3.0, 2.0, 1.0]),
    ];
    for (id, expected) in cases {
        let mut req = request(1.0, 1, &["time", "S", "[S]", "R", "[R]"]);
        req.preinit = vec![(id.to_string(), 3.0)];
        let result = runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&req)
            .unwrap();
        assert_eq!(result.values[1..5], expected, "{id}");
    }
}

const TIME: &str =
    r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;

#[test]
fn a_change_converts_with_the_size_of_a_compartment_of_a_rule() {
    // the compartment C := 1 + time, S of the initial concentration 1, the change [S] = 3 at 5;
    // sbmlsim (libroadrunner 2.11.1): at 5 C = 6, S = 1
    // and [S] = 1/6 before, S = 18 and [S] = 3 after, at 10 C = 11, S = 18, [S] = 18/11
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="cr">
<listOfCompartments><compartment id="C" spatialDimensions="3" constant="false"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="C" initialConcentration="1" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfRules><assignmentRule variable="C"><math {MATH}><apply><plus/><cn>1</cn>{TIME}</apply></math></assignmentRule></listOfRules>
</model></sbml>"#
    );
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    for with_steps in [false, true] {
        let mut req = request(10.0, 2, &["time", "C", "S", "[S]"]);
        if with_steps {
            req.output = Output::IntegratorSteps;
        }
        req.changes = vec![sbmlsim_abi::Change {
            time: 5.0,
            values: vec![("[S]".to_string(), 3.0)],
            increments: Vec::new(),
        }];
        let result = runtime()
            .instantiate(&compiled.wasm, &compiled.description)
            .unwrap()
            .simulate(&req)
            .unwrap();
        let k = result.columns.len();
        let rows: Vec<&[f64]> = (0..result.n_rows)
            .map(|i| &result.values[i * k..(i + 1) * k])
            .filter(|r| r[0] == 5.0)
            .collect();
        assert_eq!(rows.len(), 2, "{with_steps}");
        assert_eq!(rows[0][1..3], [6.0, 1.0], "{with_steps}");
        assert!((rows[0][3] - 1.0 / 6.0).abs() < 1e-15, "{with_steps}");
        assert_eq!(rows[1][1..], [6.0, 18.0, 3.0], "{with_steps}");
        let last = &result.values[result.values.len() - k..];
        assert_eq!(last[..3], [10.0, 11.0, 18.0], "{with_steps}");
        assert!((last[3] - 18.0 / 11.0).abs() < 1e-15, "{with_steps}");
    }
}

#[test]
fn initial_assignments_see_the_time_zero() {
    // x := time + 1, y := 2 x (y of the rate rule 0): sbmlsim initializes at the time 0, also
    // for a simulation from 5 or -3 and with a steady state (probed with libroadrunner 2.11.1
    // through sbmlsim): x = 1, y = 2; with the pre-initialization x = 4: x = 4, y = 8
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="ia">
<listOfParameters><parameter id="x" constant="false"/><parameter id="y" value="0" constant="false"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="x"><math {MATH}><apply><plus/>{TIME}<cn>1</cn></apply></math></initialAssignment>
<initialAssignment symbol="y"><math {MATH}><apply><times/><cn>2</cn><ci>x</ci></apply></math></initialAssignment></listOfInitialAssignments>
<listOfRules><rateRule variable="y"><math {MATH}><cn>0</cn></math></rateRule></listOfRules>
</model></sbml>"#
    );
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    for start in [5.0, -3.0] {
        for steady in [false, true] {
            for (preinit, expected) in [(None, [1.0, 2.0]), (Some(4.0), [4.0, 8.0])] {
                let mut req = request(start + 1.0, 1, &["time", "x", "y"]);
                req.start = start;
                req.preinit = preinit.map(|x| ("x".to_string(), x)).into_iter().collect();
                req.steady_state = steady.then(sbmlsim_abi::SteadyState::default);
                let result = runtime()
                    .instantiate(&compiled.wasm, &compiled.description)
                    .unwrap()
                    .simulate(&req)
                    .unwrap();
                assert_eq!(result.values[..3], [start, expected[0], expected[1]]);
                assert_eq!(result.values[4..], expected, "{start} {steady} {preinit:?}");
            }
        }
    }
}

#[test]
fn a_failed_request_leaves_the_instance_as_a_fresh_one() {
    // z' = 1 never reaches a steady state; one step is too few; then a good request with an
    // event, a change and the pre-initialization equals the one of a fresh instance, bitwise
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="ramp">
<listOfParameters><parameter id="z" value="0" constant="false"/><parameter id="n" value="0" constant="false"/></listOfParameters>
<listOfRules><rateRule variable="z"><math {MATH}><cn>1</cn></math></rateRule></listOfRules>
<listOfEvents><event id="e" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}><apply><gt/><ci>z</ci><cn>0.5</cn></apply></math></trigger>
<listOfEventAssignments><eventAssignment variable="n"><math {MATH}><apply><plus/><ci>n</ci><cn>1</cn></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#
    );
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let mut never = request(1.0, 2, &[]);
    never.steady_state = Some(sbmlsim_abi::SteadyState {
        max_time: 20.0,
        ..sbmlsim_abi::SteadyState::default()
    });
    let mut one_step = request(1.0, 2, &[]);
    one_step.settings.maximum_num_steps = 1;
    one_step.settings.relative_tolerance = 1e-12;
    one_step.end = 1e6;
    let mut good = request(2.0, 8, &[]);
    good.preinit = vec![("n".to_string(), 3.0)];
    good.changes = vec![sbmlsim_abi::Change {
        time: 1.0,
        values: vec![("z".to_string(), 0.0)],
        increments: Vec::new(),
    }];
    let mut instance = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap();
    let error = instance.simulate(&never).unwrap_err().to_string();
    assert!(error.contains("steady state"), "{error}");
    assert!(instance.simulate(&one_step).is_err());
    let after_failures = instance.simulate(&good).unwrap();
    let fresh = runtime()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&good)
        .unwrap();
    let bits = |r: &sbmlsim_abi::SimulationResult| {
        r.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(bits(&after_failures), bits(&fresh));
    // the event executed at z = 0.5 and again after the change
    let n = after_failures
        .columns
        .iter()
        .position(|c| c == "n")
        .unwrap();
    let k = after_failures.columns.len();
    assert_eq!(
        after_failures.values[after_failures.values.len() - k + n],
        5.0
    );
}

/// `x' = -pow(k, 1.5) * BSA * x` with the rule `BSA = sqrt(BW * H / 3600)` of parameters, and
/// the event `x < 0.5`: `k = 2 k`: the constants of the optimizer (`pow(k, 1.5) * BSA`) follow
/// every change of their parameters, by a change of the request and by an event, so the
/// optimized model gives the bits of the model as written.
#[test]
fn the_constants_follow_the_changes_and_the_events() {
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="c">
<listOfParameters><parameter id="x" value="1" constant="false"/><parameter id="k" value="0.3" constant="false"/><parameter id="BW" value="70" constant="true"/><parameter id="H" value="175" constant="true"/><parameter id="BSA" constant="false"/></listOfParameters>
<listOfRules>
<assignmentRule variable="BSA"><math {MATH}><apply><root/><apply><divide/><apply><times/><ci>BW</ci><ci>H</ci></apply><cn>3600</cn></apply></apply></math></assignmentRule>
<rateRule variable="x"><math {MATH}><apply><times/><apply><minus/><apply><power/><ci>k</ci><cn>1.5</cn></apply></apply><ci>BSA</ci><ci>x</ci></apply></math></rateRule>
</listOfRules>
<listOfEvents><event id="e" useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math {MATH}><apply><lt/><ci>x</ci><cn>0.5</cn></apply></math></trigger>
<listOfEventAssignments><eventAssignment variable="k"><math {MATH}><apply><times/><cn>2</cn><ci>k</ci></apply></math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#
    );
    let compiled = |optimize| {
        let options = CompileOptions {
            optimize,
            ..CompileOptions::default()
        };
        compile(&sbml, &options).unwrap()
    };
    let (written, optimized) = (compiled(false), compiled(true));
    assert_eq!(written.description.n_constants, 0);
    assert!(optimized.description.n_constants > 0);
    let mut req = request(10.0, 40, &["time", "x", "k", "BSA"]);
    req.changes = vec![
        sbmlsim_abi::Change {
            time: 1.0,
            values: vec![("k".to_string(), 0.1)],
            increments: Vec::new(),
        },
        sbmlsim_abi::Change {
            time: 6.0,
            values: vec![("BW".to_string(), 50.0)],
            increments: Vec::new(),
        },
    ];
    let run = |c: &sbmlsim_compiler::CompiledModel| {
        runtime()
            .instantiate(&c.wasm, &c.description)
            .unwrap()
            .simulate(&req)
            .unwrap()
    };
    let (a, b) = (run(&written), run(&optimized));
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a.values), bits(&b.values));
    // the event fired (k doubled) and the change of BW changed BSA
    let k_last = b.values[b.values.len() - 2];
    let bsa_last = b.values[b.values.len() - 1];
    assert!(k_last > 0.1 && (bsa_last - (50.0f64 * 175.0 / 3600.0).sqrt()).abs() < 1e-15);
}

#[test]
fn a_loaded_model_makes_instances_which_simulate_the_same_bits() {
    let sbml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay.xml"))
            .unwrap();
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let req = request(10.0, 10, &["time", "y", "t_half"]);
    let loaded = runtime()
        .load(&compiled.wasm, &compiled.description)
        .unwrap();
    let a = loaded.instantiate().unwrap().simulate(&req).unwrap();
    let b = loaded.instantiate().unwrap().simulate(&req).unwrap();
    // an instance of a fresh runtime, which compiles model.wasm again
    let fresh = Runtime::embedded()
        .unwrap()
        .instantiate(&compiled.wasm, &compiled.description)
        .unwrap()
        .simulate(&req)
        .unwrap();
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(a.columns, b.columns);
    assert_eq!(bits(&a.values), bits(&b.values));
    assert_eq!(a.columns, fresh.columns);
    assert_eq!(bits(&a.values), bits(&fresh.values));
}

#[test]
fn a_loaded_model_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<sbmlsim_host::LoadedModel>();
}

/// The values of the selections of a description in a loaded model, by id.
fn initial_by_id(sbml: &str) -> std::collections::HashMap<String, f64> {
    let compiled = compile(sbml, &CompileOptions::default()).unwrap();
    let loaded = runtime()
        .load(&compiled.wasm, &compiled.description)
        .unwrap();
    let values = loaded.initial_selections().unwrap();
    let selections = &compiled.description.selections;
    assert_eq!(values.len(), selections.len());
    selections
        .iter()
        .map(|s| s.id.clone())
        .zip(values)
        .collect()
}

#[test]
fn the_initial_selections_are_the_model_as_loaded() {
    let sbml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay.xml"))
            .unwrap();
    let values = initial_by_id(&sbml);
    // y starts at 1, k is 0.5 and t_half follows its assignment rule
    assert_eq!(values["y"], 1.0);
    assert_eq!(values["k"], 0.5);
    assert_eq!(values["t_half"], 2.0f64.ln() / 0.5);
}

#[test]
fn the_initial_selections_evaluate_the_initial_assignments() {
    let sbml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="ia">
<listOfParameters><parameter id="q" value="3" constant="true"/><parameter id="p" constant="true"/></listOfParameters>
<listOfInitialAssignments><initialAssignment symbol="p"><math {MATH}><apply><times/><cn>2</cn><ci>q</ci></apply></math></initialAssignment></listOfInitialAssignments>
</model></sbml>"#
    );
    let values = initial_by_id(&sbml);
    assert_eq!(values["q"], 3.0);
    assert_eq!(values["p"], 6.0);
}

#[test]
fn the_initial_selections_refuse_a_description_of_other_sizes() {
    let sbml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decay.xml"))
            .unwrap();
    let compiled = compile(&sbml, &CompileOptions::default()).unwrap();
    let mut desc = compiled.description.clone();
    desc.n_roots += 1;
    let loaded = runtime().load(&compiled.wasm, &desc).unwrap();
    let error = loaded.initial_selections().unwrap_err().to_string();
    // the model has no root function, the description one
    assert!(
        error.contains("has the sizes") && error.contains("the description"),
        "{error}"
    );
}

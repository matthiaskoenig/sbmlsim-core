//! The module and the description of `sbmlsim_codegen::emit`.

use sbmlsim_abi::{self as abi, EntityKind, JacobianChoice, JacobianKind, MODEL_CONTRACT};
use sbmlsim_codegen::{DIFFERENCE_REQUESTED, MAX_LOCALS, emit, emit_optimized, emit_with};
use sbmlsim_ir::{Assignment, CompileError, Entity, Expr, Model, Op, Symbol, build};
use wasmparser::{
    CompositeInnerType, ExternalKind, FuncType, Operator, Parser, Payload, TypeRef, ValType,
};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// The model of an SBML Level 3 Version 2 document around a model body; `$` in `body` is the
/// MathML namespace declaration.
fn model(body: &str) -> Model {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">{}</model></sbml>"#,
        body.replace('$', MATH)
    );
    build(&sbmlsim_sbml::read(&xml).unwrap()).unwrap()
}

/// `S -> P` at the rate `q / 49 * S` in `C`, with the assignment rule `a = 2 * q`, the initial
/// assignment `b = exp(q) + max(q, 1)`, the rate rule `r' = piecewise(1, a > 3, sin(q))` and a
/// species reference `sr`.
fn small() -> Model {
    model(
        r#"<listOfCompartments><compartment id="C" size="2" constant="true"/></listOfCompartments>
<listOfSpecies>
  <species id="S" compartment="C" initialConcentration="3" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>
  <species id="P" compartment="C" initialAmount="0" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/>
</listOfSpecies>
<listOfParameters>
  <parameter id="q" value="0.5" units="second" constant="true"/>
  <parameter id="a" constant="false"/>
  <parameter id="b" constant="true"/>
  <parameter id="r" value="0" constant="false"/>
</listOfParameters>
<listOfInitialAssignments>
  <initialAssignment symbol="b"><math $><apply><plus/><apply><exp/><ci>q</ci></apply><apply><max/><ci>q</ci><cn>1</cn></apply></apply></math></initialAssignment>
</listOfInitialAssignments>
<listOfRules>
  <assignmentRule variable="a"><math $><apply><times/><cn>2</cn><ci>q</ci></apply></math></assignmentRule>
  <rateRule variable="r"><math $><piecewise><piece><cn>1</cn><apply><gt/><ci>a</ci><cn>3</cn></apply></piece><otherwise><apply><sin/><ci>q</ci></apply></otherwise></piecewise></math></rateRule>
</listOfRules>
<listOfReactions>
  <reaction id="J" reversible="false">
    <listOfReactants><speciesReference id="sr" species="S" stoichiometry="1" constant="true"/></listOfReactants>
    <listOfProducts><speciesReference species="P" stoichiometry="1" constant="true"/></listOfProducts>
    <kineticLaw><math $><apply><times/><apply><divide/><ci>q</ci><cn>49</cn></apply><ci>S</ci></apply></math></kineticLaw>
  </reaction>
</listOfReactions>"#,
    )
}

#[test]
fn an_algebraic_species_reference_contributes_residual_sparsity_and_discontinuities() {
    let m = model(
        r#"<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
    <listOfSpecies><species id="S" compartment="C" hasOnlySubstanceUnits="true" initialAmount="0" boundaryCondition="false" constant="false"/></listOfSpecies>
    <listOfRules><algebraicRule><math $><apply><minus/><ci>nu</ci>
    <piecewise><piece><ci>S</ci><apply><gt/><ci>S</ci><cn>1</cn></apply></piece><otherwise><cn>1</cn></otherwise></piecewise>
    </apply></math></algebraicRule></listOfRules>
    <listOfReactions><reaction id="r" reversible="false"><listOfProducts><speciesReference id="nu" species="S" constant="false"/></listOfProducts>
    <kineticLaw><math $><cn>1</cn></math></kineticLaw></reaction></listOfReactions>"#,
    );
    for emit_model in [emit_with, emit_optimized] {
        for choice in [JacobianChoice::Analytic, JacobianChoice::DifferenceQuotient] {
            let (wasm, d) = emit_model(&m, choice).unwrap();
            wasmparser::validate(&wasm).unwrap();
            assert_eq!(d.n_algebraic, 1);
            assert_eq!(d.states[1].id, "nu");
            assert_eq!(d.n_discontinuity_roots, 1);
            assert_eq!(d.jacobian.pattern().rows(0), [1]);
            assert_eq!(d.jacobian.pattern().rows(1), [0, 1]);
        }
    }
}

/// The function types, the imports, the exports and the code of the functions of a module.
struct Parsed {
    types: Vec<FuncType>,
    imports: Vec<(String, String, TypeRef)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    bodies: Vec<Vec<Operator<'static>>>,
    memories: u32,
    globals: u32,
    data: u32,
    start: bool,
}

fn parse(wasm: &[u8]) -> Parsed {
    let wasm: &'static [u8] = Vec::leak(wasm.to_vec());
    let mut p = Parsed {
        types: vec![],
        imports: vec![],
        functions: vec![],
        exports: vec![],
        bodies: vec![],
        memories: 0,
        globals: 0,
        data: 0,
        start: false,
    };
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            Payload::TypeSection(s) => {
                for t in s.into_iter_err_on_gc_types() {
                    p.types.push(t.unwrap());
                }
            }
            Payload::ImportSection(s) => {
                for i in s.into_imports() {
                    let i = i.unwrap();
                    p.imports
                        .push((i.module.to_string(), i.name.to_string(), i.ty));
                }
            }
            Payload::FunctionSection(s) => {
                p.functions = s.into_iter().map(|t| t.unwrap()).collect()
            }
            Payload::ExportSection(s) => {
                for e in s {
                    let e = e.unwrap();
                    p.exports.push((e.name.to_string(), e.kind, e.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                let ops = body
                    .get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .map(|o| o.unwrap())
                    .collect();
                p.bodies.push(ops);
            }
            Payload::MemorySection(s) => p.memories += s.count(),
            Payload::GlobalSection(s) => p.globals += s.count(),
            Payload::DataSection(s) => p.data += s.count(),
            Payload::StartSection { .. } => p.start = true,
            _ => {}
        }
    }
    let _ = CompositeInnerType::Func;
    p
}

impl Parsed {
    /// The number of imported functions.
    fn n_imported(&self) -> u32 {
        self.imports
            .iter()
            .filter(|(_, _, t)| matches!(t, TypeRef::Func(_)))
            .count() as u32
    }

    /// The type of the exported function `name`.
    fn export_type(&self, name: &str) -> &FuncType {
        let (_, kind, index) = self
            .exports
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("no export {name}"));
        assert_eq!(*kind, ExternalKind::Func);
        let defined = (index - self.n_imported()) as usize;
        &self.types[self.functions[defined] as usize]
    }

    /// The operators of the exported function `name`.
    fn body(&self, name: &str) -> &[Operator<'static>] {
        let (_, _, index) = self.exports.iter().find(|(n, _, _)| n == name).unwrap();
        &self.bodies[(index - self.n_imported()) as usize]
    }
}

fn wasm_of(model: &Model) -> Vec<u8> {
    emit(model).unwrap().0
}

#[test]
fn emitted_module_validates() {
    let wasm = wasm_of(&small());
    wasmparser::validate(&wasm).unwrap();
    // a model without states, parameters or rules too
    wasmparser::validate(&wasm_of(&model(""))).unwrap();
}

#[test]
fn imports_only_memory_and_math() {
    const MATHLIB: [&str; 21] = [
        "math_exp",
        "math_log",
        "math_log10",
        "math_pow",
        "math_sqrt",
        "math_sin",
        "math_cos",
        "math_tan",
        "math_asin",
        "math_acos",
        "math_atan",
        "math_sinh",
        "math_cosh",
        "math_tanh",
        "math_asinh",
        "math_acosh",
        "math_atanh",
        "math_floor",
        "math_ceil",
        "math_factorial",
        "math_fmod",
    ];
    let p = parse(&wasm_of(&small()));
    let (memory, functions): (Vec<_>, Vec<_>) = p
        .imports
        .iter()
        .partition(|(_, _, t)| matches!(t, TypeRef::Memory(_)));
    assert_eq!(memory.len(), 1);
    assert_eq!(
        (memory[0].0.as_str(), memory[0].1.as_str()),
        ("env", "memory")
    );
    let mut names = vec![];
    for (module, name, ty) in functions {
        assert_eq!(module, "math");
        assert!(MATHLIB.contains(&name.as_str()), "{name}");
        let TypeRef::Func(t) = ty else {
            panic!("{name} is no function")
        };
        let t = &p.types[*t as usize];
        let arity = if name == "math_pow" || name == "math_fmod" {
            2
        } else {
            1
        };
        assert_eq!(t.params(), vec![ValType::F64; arity].as_slice(), "{name}");
        assert_eq!(t.results(), [ValType::F64], "{name}");
        names.push(name.as_str());
    }
    // only what the model calls: exp, sin (rhs and selections) and nothing else
    names.sort();
    assert_eq!(names, ["math_exp", "math_sin"]);
    assert_eq!((p.memories, p.globals, p.data, p.start), (0, 0, 0, false));
}

#[test]
fn exports_model_2_functions() {
    use ValType::{F64, I32};
    let p = parse(&wasm_of(&small()));
    let expected: [(&str, &[ValType], &[ValType]); 14] = [
        (abi::DEFAULTS, &[I32, I32], &[]),
        (abi::INIT, &[F64, I32, I32, I32], &[]),
        (abi::RHS, &[F64, I32, I32, I32], &[]),
        (abi::JAC, &[F64, I32, I32, I32], &[]),
        (abi::ROOTS, &[F64, I32, I32, I32], &[]),
        (abi::TRIGGER, &[I32, F64, I32, I32], &[I32]),
        (abi::PRIORITY, &[I32, F64, I32, I32], &[F64]),
        (abi::DELAY, &[I32, F64, I32, I32], &[F64]),
        (abi::EVENT_VALUES, &[I32, F64, I32, I32, I32], &[]),
        (abi::EVENT_APPLY, &[I32, F64, I32, I32, I32], &[]),
        (abi::SELECTIONS, &[F64, I32, I32, I32, I32], &[]),
        (abi::ASSIGN, &[F64, I32, I32], &[]),
        (abi::SIZES, &[I32], &[]),
        (abi::EVENT_SIZES, &[I32], &[]),
    ];
    assert_eq!(p.exports.len(), expected.len());
    for (name, params, results) in expected {
        let t = p.export_type(name);
        assert_eq!((t.params(), t.results()), (params, results), "{name}");
    }
}

#[test]
fn division_kept() {
    let p = parse(&wasm_of(&small()));
    let rhs = p.body(abi::RHS);
    let divisions = rhs
        .windows(2)
        .filter(|w| {
            matches!(w[0], Operator::F64Const { value } if f64::from(value) == 49.0)
                && matches!(w[1], Operator::F64Div)
        })
        .count();
    // the rate of S and the rate of P each read the kinetic law
    assert_eq!(divisions, 2, "{rhs:?}");
    assert!(
        !rhs.iter()
            .any(|o| matches!(o, Operator::F64Const { value } if f64::from(*value) == 0.5 / 49.0)),
        "a folded constant"
    );
}

#[test]
fn the_description_is_of_model_1() {
    let m = small();
    let (_, d) = emit(&m).unwrap();
    assert_eq!(d.contract, MODEL_CONTRACT);
    assert_eq!(d.id, "m");
    let ids = |v: &[abi::Entity]| v.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&d.states), ["r", "S", "P"]);
    assert_eq!(ids(&d.parameters), ["C", "q", "a", "b", "sr"]);
    // the values as loaded: the initial evaluations at the time 0
    let initial = |v: &[abi::Entity], id: &str| v.iter().find(|e| e.id == id).unwrap().initial;
    assert_eq!(initial(&d.states, "S"), 3.0 * 2.0);
    assert_eq!(initial(&d.parameters, "a"), 1.0);
    let b = sbmlsim_mathlib::f::math_exp(0.5) + 1.0;
    assert_eq!(initial(&d.parameters, "b").to_bits(), b.to_bits());
    assert!(!d.parameters[2].settable);
    assert_eq!(
        d.states[1].kind,
        EntityKind::Species {
            compartment: "C".to_string(),
            has_only_substance_units: false,
            stores_concentration: false,
        }
    );
    // the selections of the model representation, at the indices 0..k
    let selections: Vec<&str> = d.selections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        selections,
        ["C", "S", "P", "[S]", "[P]", "q", "a", "b", "r", "sr", "J"]
    );
    assert!(
        d.selections
            .iter()
            .enumerate()
            .all(|(i, s)| s.index as usize == i)
    );
    assert_eq!(d.units.get("q").map(String::as_str), Some("second"));
    // the analytic Jacobian on the sparse pattern: only the rates of S and P read S
    let JacobianKind::Analytic(pattern) = &d.jacobian else {
        panic!("{:?}", d.jacobian)
    };
    assert_eq!(pattern.col_ptr, [0, 0, 2, 2]);
    assert_eq!(pattern.row_idx, [1, 2]);
    assert_eq!(abi::ModelSizes::of(&d).jacobian_entries, 2);
    assert!(d.events.is_empty());
}

#[test]
fn the_jacobian_follows_the_choice() {
    let m = small();
    // the difference quotient over the same pattern, with its coloring and the reason
    let (wasm, d) = emit_with(&m, JacobianChoice::DifferenceQuotient).unwrap();
    let JacobianKind::ColoredDifference {
        pattern,
        colors,
        reason,
    } = &d.jacobian
    else {
        panic!("{:?}", d.jacobian)
    };
    assert_eq!(pattern.row_idx, [1, 2]);
    assert_eq!(colors, &[0, 0, 0]);
    assert_eq!(reason, DIFFERENCE_REQUESTED);
    // its model_jac traps
    assert!(matches!(
        parse(&wasm).body("model_jac"),
        [Operator::Unreachable, Operator::End]
    ));
    assert!(matches!(
        emit_with(&m, JacobianChoice::Analytic).unwrap().1.jacobian,
        JacobianKind::Analytic(_)
    ));
    // a model without an analytic Jacobian: the difference quotient with the reason, refused
    // where the analytic one is asked for
    let mut f = small();
    f.rates[1] = Expr::apply(Op::Factorial, vec![Expr::Symbol(Symbol::State(1))]);
    let (_, d) = emit(&f).unwrap();
    let JacobianKind::ColoredDifference { reason, .. } = &d.jacobian else {
        panic!("{:?}", d.jacobian)
    };
    assert_eq!(reason, "factorial has no derivative (the rate of S)");
    let error = emit_with(&f, JacobianChoice::Analytic).unwrap_err();
    assert_eq!(
        error.to_string(),
        "an analytic Jacobian: factorial has no derivative (the rate of S) is not supported (m)"
    );
}

#[test]
fn the_events_are_described() {
    let m = model(
        r#"<listOfParameters><parameter id="x" value="0" constant="false"/></listOfParameters>
<listOfEvents><event id="E" useValuesFromTriggerTime="true">
  <trigger initialValue="false" persistent="true"><math $><apply><and/><apply><gt/><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol><cn>1</cn></apply><apply><lt/><ci>x</ci><cn>2</cn><cn>3</cn></apply></apply></math></trigger>
  <delay><math $><cn>0.5</cn></math></delay>
  <listOfEventAssignments><eventAssignment variable="x"><math $><cn>1</cn></math></eventAssignment></listOfEventAssignments>
</event>
<event useValuesFromTriggerTime="false">
  <trigger initialValue="true" persistent="false"><math $><ci>x</ci></math></trigger>
  <priority><math $><cn>2</cn></math></priority>
</event></listOfEvents>"#,
    );
    let (wasm, d) = emit(&m).unwrap();
    wasmparser::Validator::new().validate_all(&wasm).unwrap();
    // the root functions: time - 1, then 2 - x and 3 - 2 of the chain x < 2 < 3, then x
    assert_eq!(d.n_roots, 4);
    assert_eq!(
        d.events,
        [
            abi::EventInfo {
                id: "E".to_string(),
                has_delay: true,
                has_priority: false,
                persistent: true,
                initial_value: false,
                use_values_from_trigger_time: true,
                n_assignments: 1,
            },
            abi::EventInfo {
                id: String::new(),
                has_delay: false,
                has_priority: true,
                persistent: false,
                initial_value: true,
                use_values_from_trigger_time: false,
                n_assignments: 0,
            }
        ]
    );
    // model_sizes and model_event_sizes store the sizes of the description
    let p = parse(&wasm);
    let stored = |export: &str| -> Vec<i32> {
        p.body(export)
            .iter()
            .filter_map(|op| match op {
                Operator::I32Const { value } => Some(*value),
                _ => None,
            })
            .collect()
    };
    let sizes = abi::ModelSizes::of(&d);
    assert_eq!(stored(abi::SIZES), sizes.words().map(|x| x as i32));
    assert_eq!(stored(abi::EVENT_SIZES), [1, 0]);
    assert_eq!(sizes.assignments, [1, 0]);
}

#[test]
fn a_statement_above_the_limits_is_an_error() {
    // a selection which reads more rules than a function may have locals: one statement whose
    // values are alive at once, which no split makes smaller (rules of the time, which are no
    // constants of the optimizer)
    let n = MAX_LOCALS;
    let parameters = (0..=n)
        .map(|i| Entity {
            id: format!("p{i}"),
            kind: sbmlsim_ir::EntityKind::Parameter,
            default: 1.0,
            settable: i == 0,
            unit: None,
        })
        .collect();
    let assignment_rules = (1..=n)
        .map(|i| Assignment {
            target: Symbol::Param(i as u32),
            expr: Expr::apply(
                Op::Times,
                vec![Expr::Symbol(Symbol::Time), Expr::Number(i as f64)],
            ),
        })
        .collect();
    let sum = Expr::apply(
        Op::Plus,
        (1..=n)
            .map(|i| Expr::Symbol(Symbol::Param(i as u32)))
            .collect(),
    );
    let m = Model {
        n_algebraic: 0,
        id: None,
        states: vec![],
        parameters,
        assignment_rules,
        initial: vec![],
        rates: vec![],
        events: vec![],
        selections: vec![sbmlsim_ir::SelectionDef {
            id: "sum".to_string(),
            kind: sbmlsim_ir::SelectionKind::Parameter,
            expr: sum,
            unit: None,
        }],
    };
    for result in [emit(&m), emit_optimized(&m, abi::JacobianChoice::Auto)] {
        match result.unwrap_err() {
            CompileError::Unsupported { feature, element } => {
                assert!(feature.contains("limits"), "{feature}");
                assert!(feature.contains("one statement"), "{feature}");
                assert_eq!(element, abi::SELECTIONS);
            }
            other => panic!("{other:?}"),
        }
    }
}

/// A chain of more assignment rules than a function may have locals is within the limits: the
/// local of a rule is given back after the last statement which reads it.
#[test]
fn locals_are_reused_after_their_last_use() {
    let n = MAX_LOCALS + 1;
    let parameters = (0..=n)
        .map(|i| Entity {
            id: format!("p{i}"),
            kind: sbmlsim_ir::EntityKind::Parameter,
            default: 1.0,
            settable: i == 0,
            unit: None,
        })
        .collect();
    let assignment_rules = (1..=n)
        .map(|i| Assignment {
            target: Symbol::Param(i as u32),
            expr: Expr::apply(Op::Plus, vec![Expr::Symbol(Symbol::Param(i as u32 - 1))]),
        })
        .collect();
    let m = Model {
        n_algebraic: 0,
        id: None,
        states: vec![],
        parameters,
        assignment_rules,
        initial: vec![],
        rates: vec![],
        events: vec![],
        selections: vec![sbmlsim_ir::SelectionDef {
            id: "last".to_string(),
            kind: sbmlsim_ir::SelectionKind::Parameter,
            expr: Expr::Symbol(Symbol::Param(n as u32)),
            unit: None,
        }],
    };
    let (wasm, _) = emit(&m).unwrap();
    wasmparser::Validator::new().validate_all(&wasm).unwrap();
    // model_selections: the parameters and one local alive at a time (a rule and the next)
    let locals = function_locals(&wasm, 10);
    assert!(locals <= 2, "{locals}");
}

/// The number of locals (without the parameters) of the defined function at `index`.
fn function_locals(wasm: &[u8], index: usize) -> u32 {
    let bodies: Vec<wasmparser::FunctionBody> = wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| match p.unwrap() {
            wasmparser::Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .collect();
    bodies[index]
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .map(|l| l.unwrap().0)
        .sum()
}

#[test]
fn requested_difference_quotients_need_only_the_structural_pattern() {
    // A long product has a quadratic expanded analytic derivative, despite its small
    // structural pattern. Difference quotients must compile only the written product.
    let product = format!("<apply><times/>{}</apply>", "<ci>x</ci>".repeat(4096));
    let m = model(&format!(
        r#"<listOfParameters><parameter id="x" constant="false" value="1"/></listOfParameters><listOfRules><rateRule variable="x"><math $>{product}</math></rateRule></listOfRules>"#
    ));
    let (wasm, description) = emit_with(&m, JacobianChoice::DifferenceQuotient).unwrap();
    let JacobianKind::ColoredDifference {
        pattern,
        colors,
        reason,
    } = description.jacobian
    else {
        panic!("expected difference quotients")
    };
    assert_eq!(pattern, sbmlsim_ir::derive::pattern(&m));
    assert_eq!(colors, vec![0]);
    assert_eq!(reason, DIFFERENCE_REQUESTED);
    assert!(wasm.len() < 100_000);
    let (_, description) = emit_with(&m, JacobianChoice::Auto).unwrap();
    assert!(
        matches!(description.jacobian, JacobianKind::ColoredDifference { ref reason, .. } if reason.contains("construction exceeds"))
    );
    let error = emit_with(&m, JacobianChoice::Analytic).unwrap_err();
    assert!(
        matches!(error, CompileError::Unsupported { ref feature, .. } if feature.contains("construction exceeds")),
        "{error:?}"
    );
}

#[test]
fn delay_uses_initial_assignments_at_the_requested_time() {
    let delay =
        r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol>"#;
    let time = r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/time">time</csymbol>"#;
    let m = model(&format!(
        r#"<listOfParameters>
        <parameter id="x" constant="false"/><parameter id="past" constant="false"/>
      </listOfParameters><listOfInitialAssignments>
        <initialAssignment symbol="x"><math $><apply><plus/><cn>3</cn>{time}</apply></math></initialAssignment>
      </listOfInitialAssignments><listOfRules>
        <rateRule variable="x"><math $><cn>1</cn></math></rateRule>
        <assignmentRule variable="past"><math $><apply>{delay}
          <apply>{delay}<ci>x</ci><cn>2</cn></apply><cn>0.5</cn></apply></math></assignmentRule>
      </listOfRules>"#
    ));
    let (bytes, desc) = emit(&m).unwrap();
    wasmparser::Validator::new().validate_all(&bytes).unwrap();
    let section = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| match payload.unwrap() {
            Payload::CustomSection(s) if s.name() == abi::history::SECTION => Some(s.data()),
            _ => None,
        })
        .expect("delay history contract");
    assert_eq!(section, [1, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(desc.n_delays, 2);
    assert_eq!(desc.n_constants, 1);
    assert!(matches!(
        desc.jacobian,
        JacobianKind::ColoredDifference { .. }
    ));
    assert_eq!(
        desc.parameters
            .iter()
            .find(|p| p.id == "past")
            .unwrap()
            .initial,
        0.5
    );
}

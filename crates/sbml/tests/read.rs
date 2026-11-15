use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{ReadError, Rule, read};

fn data(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The math of one initial assignment of a model made around `math`.
fn math(body: &str) -> Node {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model>
<listOfInitialAssignments><initialAssignment symbol="a"><math xmlns="http://www.w3.org/1998/Math/MathML">{body}</math></initialAssignment></listOfInitialAssignments>
</model></sbml>"#
    );
    read(&xml).unwrap().model.initial_assignments[0]
        .math
        .clone()
}

fn bad_math(body: &str) -> ReadError {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model>
<listOfInitialAssignments><initialAssignment symbol="a"><math xmlns="http://www.w3.org/1998/Math/MathML">{body}</math></initialAssignment></listOfInitialAssignments>
</model></sbml>"#
    );
    read(&xml).unwrap_err()
}

#[test]
fn reads_minimal_model() {
    let doc = read(&data("minimal.xml")).unwrap();
    assert_eq!((doc.level, doc.version), (3, 2));
    let m = &doc.model;
    assert_eq!(m.id.as_deref(), Some("decay"));
    assert_eq!(m.substance_units.as_deref(), Some("mole"));
    assert_eq!(m.time_units.as_deref(), Some("second"));
    assert_eq!(m.unit_definitions[0].id, "mmol_per_min");
    assert_eq!(m.unit_definitions[0].expression, "1e-3*mole*60*second**-1");
    assert_eq!(m.compartments[0].id, "c");
    assert_eq!(m.compartments[0].size, Some(2.0));
    assert!(m.compartments[0].constant);
    assert_eq!(m.species.len(), 2);
    assert_eq!(m.species[0].initial_amount, Some(10.0));
    assert!(m.species[0].has_only_substance_units);
    assert_eq!(m.species[1].initial_concentration, Some(0.5));
    assert!(m.species[1].boundary_condition);
    assert_eq!(m.species[1].conversion_factor.as_deref(), Some("cf"));
    assert_eq!(m.parameters[0].value, Some(0.1));
    let r = &m.reactions[0];
    assert_eq!(r.id, "r1");
    assert!(!r.reversible && !r.fast);
    assert_eq!(r.reactants[0].species, "S1");
    assert_eq!(r.reactants[0].stoichiometry, Some(2.0));
    assert!(r.reactants[0].constant);
    assert_eq!(r.products[0].id.as_deref(), Some("sr"));
    assert_eq!(r.products[0].stoichiometry, None);
    assert_eq!(r.modifiers[0].species, "S2");
    let k = r.kinetic_law.as_ref().unwrap();
    assert_eq!(
        k.math,
        Some(Node::Apply {
            op: Op::Times,
            args: vec![Node::Symbol("k".into()), Node::Symbol("S1".into())]
        })
    );
    assert_eq!(k.local_parameters[0].id, "lp");
    assert_eq!(k.local_parameters[0].value, Some(3.0));
}

#[test]
fn number_is_double_of_text() {
    assert_eq!(math("<cn> 1e-3 </cn>"), Node::Number(1e-3));
    assert_eq!(math(r#"<cn type="real">0.1</cn>"#), Node::Number(0.1));
    assert_eq!(math(r#"<cn type="integer"> 7 </cn>"#), Node::Number(7.0));
    assert_eq!(
        math(r#"<cn type="e-notation"> 2 <sep/> -3 </cn>"#),
        Node::Number(2e-3)
    );
    assert_eq!(
        math(r#"<cn type="e-notation">1.1<sep/>-5</cn>"#),
        Node::Number(1.1e-5)
    );
    assert_eq!(
        math(r#"<cn type="rational"> 1 <sep/> 3 </cn>"#),
        Node::Number(1.0 / 3.0)
    );
    assert_eq!(
        math(
            r#"<cn sbml:units="mole" xmlns:sbml="http://www.sbml.org/sbml/level3/version2/core">49</cn>"#
        ),
        Node::Number(49.0)
    );
    assert!(matches!(bad_math("<cn>abc</cn>"), ReadError::Xml(_)));
}

#[test]
fn refuses_level_2() {
    assert_eq!(
        read(&data("level2.xml")).unwrap_err(),
        ReadError::NotLevel3Version2 {
            level: 2,
            version: 4
        }
    );
}

#[test]
fn refuses_comp_model() {
    assert_eq!(
        read(&data("comp.xml")).unwrap_err(),
        ReadError::Package("comp".into())
    );
}

#[test]
fn ignores_optional_package() {
    let doc = read(&data("ignored_package.xml")).unwrap();
    assert_eq!(doc.model.parameters[0].id, "p");
}

#[test]
fn csymbols() {
    let doc = read(&data("csymbols.xml")).unwrap();
    let m = &doc.model;
    assert_eq!(
        m.initial_assignments[0].math,
        Node::Apply {
            op: Op::Plus,
            args: vec![Node::CsymbolTime, Node::CsymbolAvogadro]
        }
    );
    let Rule::Assignment { variable, math: mm } = &m.rules[0] else {
        panic!()
    };
    assert_eq!(variable, "b");
    assert_eq!(
        *mm,
        Node::Apply {
            op: Op::RateOf,
            args: vec![Node::Symbol("c".into())]
        }
    );
    let Rule::Rate { variable, math: mm } = &m.rules[1] else {
        panic!()
    };
    assert_eq!(variable, "c");
    assert_eq!(
        *mm,
        Node::Apply {
            op: Op::Delay,
            args: vec![Node::Symbol("b".into()), Node::Number(2.0)]
        }
    );
    assert!(matches!(&m.rules[2], Rule::Algebraic { .. }));
    // a csymbol outside of an operator position
    assert_eq!(
        math(r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">d</csymbol>"#),
        Node::CsymbolDelay
    );
    assert_eq!(
        math(r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/rateOf">r</csymbol>"#),
        Node::CsymbolRateOf
    );
}

#[test]
fn event_fields() {
    let doc = read(&data("events.xml")).unwrap();
    let e = &doc.model.events;
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].id.as_deref(), Some("e1"));
    assert!(!e[0].use_values_from_trigger_time);
    assert!(!e[0].trigger.initial_value);
    assert!(e[0].trigger.persistent);
    assert_eq!(
        e[0].trigger.math,
        Node::Apply {
            op: Op::Gt,
            args: vec![Node::Symbol("x".into()), Node::Number(1.0)]
        }
    );
    assert_eq!(e[0].priority, Some(Node::Number(5.0)));
    assert_eq!(e[0].delay, Some(Node::Number(0.5)));
    assert_eq!(e[0].assignments.len(), 2);
    assert_eq!(e[0].assignments[1].variable, "y");
    assert_eq!(e[0].assignments[1].math, Node::Symbol("x".into()));
    // an event without an id keeps its position
    assert_eq!(e[1].id, None);
    assert!(e[1].use_values_from_trigger_time);
    assert!(e[1].trigger.initial_value && !e[1].trigger.persistent);
    assert_eq!(e[1].trigger.math, Node::True);
    assert_eq!(e[1].delay, None);
    assert_eq!(e[1].priority, None);
    assert!(e[1].assignments.is_empty());
}

#[test]
fn mathml_constants_and_defaults() {
    assert_eq!(math("<pi/>"), Node::Pi);
    assert_eq!(math("<exponentiale/>"), Node::ExponentialE);
    assert_eq!(math("<infinity/>"), Node::Infinity);
    assert_eq!(math("<notanumber/>"), Node::NotANumber);
    assert_eq!(math("<false/>"), Node::False);
    // log without logbase has one argument (base 10), with it [base, x]
    assert_eq!(
        math("<apply><log/><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Log,
            args: vec![Node::Symbol("x".into())]
        }
    );
    assert_eq!(
        math("<apply><log/><logbase><cn>2</cn></logbase><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Log,
            args: vec![Node::Number(2.0), Node::Symbol("x".into())]
        }
    );
    assert_eq!(
        math("<apply><root/><degree><cn type=\"integer\">3</cn></degree><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Root,
            args: vec![Node::Number(3.0), Node::Symbol("x".into())]
        }
    );
    assert_eq!(
        math("<apply><root/><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Root,
            args: vec![Node::Symbol("x".into())]
        }
    );
    // unary minus, n-ary plus in written order
    assert_eq!(
        math("<apply><minus/><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Minus,
            args: vec![Node::Symbol("x".into())]
        }
    );
    let Node::Apply { op, args } =
        math("<apply><plus/><ci>c</ci><ci>a</ci><ci>b</ci><cn>1</cn></apply>")
    else {
        panic!()
    };
    assert_eq!(op, Op::Plus);
    assert_eq!(args.len(), 4);
    assert_eq!(args[0], Node::Symbol("c".into()));
    assert_eq!(args[1], Node::Symbol("a".into()));
}

#[test]
fn piecewise_lambda_semantics_and_functions() {
    assert_eq!(
        math(
            "<piecewise><piece><cn>1</cn><apply><lt/><ci>x</ci><cn>2</cn></apply></piece><otherwise><cn>3</cn></otherwise></piecewise>"
        ),
        Node::Piecewise {
            pieces: vec![(
                Node::Number(1.0),
                Node::Apply {
                    op: Op::Lt,
                    args: vec![Node::Symbol("x".into()), Node::Number(2.0)]
                }
            )],
            otherwise: Some(Box::new(Node::Number(3.0)))
        }
    );
    assert_eq!(
        math("<piecewise><piece><cn>1</cn><true/></piece></piecewise>"),
        Node::Piecewise {
            pieces: vec![(Node::Number(1.0), Node::True)],
            otherwise: None
        }
    );
    assert_eq!(
        math(
            "<semantics><ci>x</ci><annotation-xml encoding=\"x\"><a/></annotation-xml></semantics>"
        ),
        Node::Symbol("x".into())
    );
    assert_eq!(
        math("<apply><ci>f</ci><cn>1</cn></apply>"),
        Node::Apply {
            op: Op::Function("f".into()),
            args: vec![Node::Number(1.0)]
        }
    );
    let xml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model>
<listOfFunctionDefinitions><functionDefinition id="f"><math xmlns="http://www.w3.org/1998/Math/MathML">
<lambda><bvar><ci>a</ci></bvar><bvar><ci>b</ci></bvar><apply><times/><ci>a</ci><ci>b</ci></apply></lambda>
</math></functionDefinition></listOfFunctionDefinitions></model></sbml>"#;
    let f = &read(xml).unwrap().model.function_definitions[0];
    assert_eq!(f.id, "f");
    assert_eq!(
        f.math,
        Some(Node::Lambda {
            params: vec!["a".into(), "b".into()],
            body: Box::new(Node::Apply {
                op: Op::Times,
                args: vec![Node::Symbol("a".into()), Node::Symbol("b".into())]
            })
        })
    );
}

#[test]
fn all_operators_are_read() {
    let ops = [
        ("plus", Op::Plus),
        ("times", Op::Times),
        ("power", Op::Power),
        ("abs", Op::Abs),
        ("exp", Op::Exp),
        ("ln", Op::Ln),
        ("floor", Op::Floor),
        ("ceiling", Op::Ceiling),
        ("factorial", Op::Factorial),
        ("sin", Op::Sin),
        ("cos", Op::Cos),
        ("tan", Op::Tan),
        ("sec", Op::Sec),
        ("csc", Op::Csc),
        ("cot", Op::Cot),
        ("sinh", Op::Sinh),
        ("cosh", Op::Cosh),
        ("tanh", Op::Tanh),
        ("sech", Op::Sech),
        ("csch", Op::Csch),
        ("coth", Op::Coth),
        ("arcsin", Op::Arcsin),
        ("arccos", Op::Arccos),
        ("arctan", Op::Arctan),
        ("arcsec", Op::Arcsec),
        ("arccsc", Op::Arccsc),
        ("arccot", Op::Arccot),
        ("arcsinh", Op::Arcsinh),
        ("arccosh", Op::Arccosh),
        ("arctanh", Op::Arctanh),
        ("arcsech", Op::Arcsech),
        ("arccsch", Op::Arccsch),
        ("arccoth", Op::Arccoth),
        ("eq", Op::Eq),
        ("neq", Op::Neq),
        ("gt", Op::Gt),
        ("lt", Op::Lt),
        ("geq", Op::Geq),
        ("leq", Op::Leq),
        ("and", Op::And),
        ("or", Op::Or),
        ("xor", Op::Xor),
        ("not", Op::Not),
        ("implies", Op::Implies),
        ("max", Op::Max),
        ("min", Op::Min),
        ("rem", Op::Rem),
        ("quotient", Op::Quotient),
        ("divide", Op::Divide),
    ];
    for (name, expected) in ops {
        let args = match name {
            "divide" | "power" | "rem" | "quotient" | "implies" => "<ci>x</ci><ci>y</ci>",
            "abs" | "exp" | "ln" | "floor" | "ceiling" | "factorial" | "not" => "<ci>x</ci>",
            n if n.contains("sin")
                || n.contains("cos")
                || n.contains("tan")
                || n.contains("sec")
                || n.contains("csc")
                || n.contains("cot") =>
            {
                "<ci>x</ci>"
            }
            _ => "<ci>x</ci><ci>y</ci>",
        };
        let Node::Apply { op, .. } = math(&format!("<apply><{name}/>{args}</apply>")) else {
            panic!("{name}")
        };
        assert_eq!(op, expected, "{name}");
    }
}

#[test]
fn unknown_elements_are_named() {
    let ReadError::Xml(m) = bad_math("<apply><frobnicate/><ci>x</ci></apply>") else {
        panic!()
    };
    assert!(m.contains("frobnicate"), "{m}");
    let ReadError::Xml(m) = bad_math("<vector/>") else {
        panic!()
    };
    assert!(m.contains("vector"), "{m}");
    assert!(matches!(
        bad_math(r#"<csymbol definitionURL="urn:other">x</csymbol>"#),
        ReadError::Xml(_)
    ));
    assert!(matches!(read("<notxml"), Err(ReadError::Xml(_))));
}

/// Reads every normalized case of Task 10 when `testdata/cases` exists.
#[test]
fn reads_all_cases() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if !root.is_dir() {
        println!("testdata/cases absent, run the preparation of Task 10");
        return;
    }
    let mut n = 0;
    let mut failures = Vec::new();
    for dir in std::fs::read_dir(&root).unwrap() {
        let dir = dir.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(&dir).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_some_and(|e| e == "xml") {
                n += 1;
                let text = std::fs::read_to_string(&file).unwrap();
                if let Err(e) = read(&text) {
                    failures.push(format!("{}: {e}", file.display()));
                }
            }
        }
    }
    println!("read {n} cases");
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

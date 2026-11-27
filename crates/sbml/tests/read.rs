use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{ReadError, Rule, Unit, read};

fn data(name: &str) -> String {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the fixture '{path}' of the test is not read: {error} (the directory of the \
             workspace is {:?})",
            std::env::current_dir()
        )
    })
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
        .expect("the initial assignment has math")
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
    assert_eq!(
        m.unit_definitions[0].expression,
        "0.001*mole*(60*second)**-1"
    );
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
    assert_eq!(math("<cn> 1e-3 </cn>"), Node::number(1e-3));
    assert_eq!(math(r#"<cn type="real">0.1</cn>"#), Node::number(0.1));
    assert_eq!(math(r#"<cn type="integer"> 7 </cn>"#), Node::number(7.0));
    assert_eq!(
        math(r#"<cn type="e-notation"> 2 <sep/> -3 </cn>"#),
        Node::number(2e-3)
    );
    assert_eq!(
        math(r#"<cn type="e-notation">1.1<sep/>-5</cn>"#),
        Node::number(1.1e-5)
    );
    assert_eq!(
        math(r#"<cn type="rational"> 1 <sep/> 3 </cn>"#),
        Node::number(1.0 / 3.0)
    );
    // a number of level 3 keeps the units it carries
    assert_eq!(
        math(
            r#"<cn sbml:units="mole" xmlns:sbml="http://www.sbml.org/sbml/level3/version2/core">49</cn>"#
        ),
        Node::Number {
            value: 49.0,
            units: Some("mole".to_string())
        }
    );
    // the attribute is read by its name, a document without the prefix means the same
    assert_eq!(
        math(r#"<cn units="mole">49</cn>"#),
        Node::Number {
            value: 49.0,
            units: Some("mole".to_string())
        }
    );
    assert_eq!(
        math(r#"<cn type="e-notation" units="per_min">2<sep/>-3</cn>"#),
        Node::Number {
            value: 2e-3,
            units: Some("per_min".to_string())
        }
    );
    assert!(matches!(bad_math("<cn>abc</cn>"), ReadError::Xml(_)));
}

fn unit_expr(units: &str) -> String {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model>
<listOfUnitDefinitions><unitDefinition id="u"><listOfUnits>{units}</listOfUnits></unitDefinition></listOfUnitDefinitions></model></sbml>"#
    );
    read(&xml).unwrap().model.unit_definitions[0]
        .expression
        .clone()
}

#[test]
fn unit_expressions() {
    let u = |k: &str, e: &str, s: &str, m: &str| {
        format!(r#"<unit kind="{k}" exponent="{e}" scale="{s}" multiplier="{m}"/>"#)
    };
    assert_eq!(unit_expr(&u("second", "-1", "0", "60")), "(60*second)**-1");
    assert_eq!(
        unit_expr(&(u("mole", "1", "-3", "1") + &u("second", "-1", "0", "60"))),
        "0.001*mole*(60*second)**-1"
    );
    assert_eq!(unit_expr(&u("litre", "1", "-3", "1")), "0.001*litre");
    assert_eq!(unit_expr(&u("gram", "1", "-3", "1")), "0.001*gram");
    assert_eq!(unit_expr(&u("mole", "1", "-6", "1")), "0.000001*mole");
    // mg/dL: scale and exponent -1
    assert_eq!(
        unit_expr(&(u("gram", "1", "-3", "1") + &u("litre", "-1", "-1", "1"))),
        "0.001*gram*(0.1*litre)**-1"
    );
    assert_eq!(unit_expr(&u("metre", "2", "0", "1")), "metre**2");
    assert_eq!(unit_expr(&u("mole", "1", "0", "1")), "mole");
    assert_eq!(unit_expr(""), "dimensionless");
    // a multiplier in e-notation
    assert_eq!(unit_expr(&u("litre", "1", "-3", "1e2")), "0.1*litre");
    assert_eq!(unit_expr(&u("litre", "1", "0", "1.5E1")), "15*litre");
    assert_eq!(unit_expr(&u("litre", "-1", "-3", "1e2")), "(0.1*litre)**-1");
}

#[test]
fn a_unit_definition_keeps_its_units() {
    let xml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
<model><listOfUnitDefinitions>
  <unitDefinition id="mmol_per_min"><listOfUnits>
    <unit kind="mole" scale="-3"/>
    <unit kind="second" exponent="-1" multiplier="60"/>
  </listOfUnits></unitDefinition>
  <unitDefinition id="nothing"><listOfUnits/></unitDefinition>
</listOfUnitDefinitions></model></sbml>"#;
    let definitions = read(xml).unwrap().model.unit_definitions;
    // the expression a unit registry parses, and the units the document writes
    assert_eq!(definitions[0].expression, "0.001*mole*(60*second)**-1");
    assert_eq!(
        definitions[0].units,
        [
            Unit {
                kind: "mole".to_string(),
                exponent: 1.0,
                scale: -3,
                multiplier: 1.0,
            },
            Unit {
                kind: "second".to_string(),
                exponent: -1.0,
                scale: 0,
                multiplier: 60.0,
            },
        ]
    );
    assert_eq!(definitions[1].expression, "dimensionless");
    assert!(
        definitions[1].units.is_empty(),
        "a definition without a unit"
    );
}

#[test]
fn elements_keep_their_ids() {
    let xml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:m="http://www.w3.org/1998/Math/MathML" level="3" version="2"><model>
<listOfInitialAssignments><initialAssignment id="ia" symbol="a"><m:math><m:cn>1</m:cn></m:math></initialAssignment></listOfInitialAssignments>
<listOfRules><assignmentRule id="ar" variable="a"><m:math><m:cn>1</m:cn></m:math></assignmentRule><rateRule id="rr" variable="b"><m:math><m:cn>1</m:cn></m:math></rateRule><algebraicRule id="al"><m:math><m:cn>1</m:cn></m:math></algebraicRule></listOfRules>
<listOfReactions><reaction id="r" reversible="false" fast="false"><kineticLaw id="kl"><m:math><m:cn>1</m:cn></m:math></kineticLaw></reaction></listOfReactions>
<listOfEvents><event><trigger id="tr" initialValue="true" persistent="true"><m:math><m:true/></m:math></trigger><priority id="pr"><m:math><m:cn>1</m:cn></m:math></priority><delay id="de"><m:math><m:cn>1</m:cn></m:math></delay>
<listOfEventAssignments><eventAssignment id="ea" variable="a"><m:math><m:cn>1</m:cn></m:math></eventAssignment></listOfEventAssignments></event></listOfEvents>
</model></sbml>"#;
    let m = read(xml).unwrap().model;
    assert_eq!(m.initial_assignments[0].id.as_deref(), Some("ia"));
    assert!(matches!(&m.rules[0], Rule::Assignment { id, .. } if id.as_deref() == Some("ar")));
    assert!(matches!(&m.rules[1], Rule::Rate { id, .. } if id.as_deref() == Some("rr")));
    assert!(matches!(&m.rules[2], Rule::Algebraic { id, .. } if id.as_deref() == Some("al")));
    assert_eq!(
        m.reactions[0].kinetic_law.as_ref().unwrap().id.as_deref(),
        Some("kl")
    );
    let e = &m.events[0];
    assert_eq!(e.trigger.as_ref().unwrap().id.as_deref(), Some("tr"));
    assert_eq!(e.priority_id.as_deref(), Some("pr"));
    assert_eq!(e.delay_id.as_deref(), Some("de"));
    assert_eq!(e.assignments[0].id.as_deref(), Some("ea"));
}

#[test]
fn reads_level_2() {
    let doc = read(&data("level2.xml")).expect("L2V4 is read");
    assert_eq!((doc.level, doc.version), (2, 4));
    assert_eq!(doc.model.id.as_deref(), Some("old"));
}

#[test]
fn reads_a_comp_model() {
    // a composed model is flattened (`tests/comp.rs`); the document of the fixture has no
    // submodel, so it is its model
    let doc = read(&data("comp.xml")).unwrap();
    assert_eq!(doc.model.id.as_deref(), Some("m"));
}

#[test]
fn ignores_optional_package() {
    let doc = read(&data("ignored_package.xml")).unwrap();
    assert_eq!(doc.model.parameters[0].id, "p");
}

/// The math of the rate rule `y' = -k y` of `distrib.xml`.
fn decay_rate_rule() -> Node {
    Node::Apply {
        op: Op::Times,
        args: vec![
            Node::Apply {
                op: Op::Minus,
                args: vec![Node::Symbol("k".into())],
            },
            Node::Symbol("y".into()),
        ],
    }
}

#[test]
fn reads_a_required_distrib_document() {
    // the uncertainty of a fitted parameter annotates the parameter and defines no mathematics,
    // so the document is read with its distrib elements ignored, the `<math>` of the
    // distribution of the draws among them (no child of the parameter, never read)
    let doc = read(&data("distrib.xml")).unwrap();
    let m = &doc.model;
    assert_eq!(m.id.as_deref(), Some("decay"));
    assert_eq!(
        m.parameters
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["y", "k", "t_half"]
    );
    assert_eq!(m.parameters[1].value, Some(0.5));
    assert!(m.parameters[1].constant);
    assert_eq!(m.rules.len(), 2);
    assert_eq!(
        m.rules[0],
        Rule::Rate {
            id: None,
            variable: "y".into(),
            math: Some(decay_rate_rule()),
        }
    );
}

#[test]
fn reads_distrib_when_it_is_not_required() {
    // the same document is read as a document of an optional package was always read
    let xml =
        data("distrib.xml").replace(r#"distrib:required="true""#, r#"distrib:required="false""#);
    let doc = read(&xml).unwrap();
    assert_eq!(doc.model.rules[0].math(), Some(&decay_rate_rule()));
    assert_eq!(doc.model.parameters[1].value, Some(0.5));
}

#[test]
fn refuses_a_required_package_of_the_prefix_of_a_read_one() {
    // a package is its namespace URI, the prefix of a document is any text: a required package
    // of another namespace is refused although it is declared under the prefix `distrib`
    let xml = r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="urn:x-other-package" level="3" version="2" distrib:required="true"><model id="m"/></sbml>"#;
    assert_eq!(read(xml).unwrap_err(), ReadError::Package("distrib".into()));
}

/// The namespace of the distrib package.
const DISTRIB_NS: &str = "http://www.sbml.org/sbml/level3/version1/distrib/version1";

/// A draw of the drafts of distrib, which is an element and no csymbol.
const DRAW: &str = r#"<distrib:drawFromDistribution>
          <distrib:listOfDistribInputs>
            <distrib:distribInput distrib:id="mu" distrib:index="0"/>
          </distrib:listOfDistribInputs>
          <distrib:uncertML>
            <NormalDistribution xmlns="http://www.uncertml.org/3.0" definition="http://www.uncertml.org/distributions/normal">
              <mean><var varId="mu"/></mean>
              <stddev><cn>1</cn></stddev>
            </NormalDistribution>
          </distrib:uncertML>
        </distrib:drawFromDistribution>"#;

/// The message of a document the reader refuses as the document it is.
fn refusal(xml: &str) -> String {
    match read(xml).unwrap_err() {
        ReadError::Xml(m) => m,
        other => panic!("refused with a message of its own: {other}"),
    }
}

#[test]
fn refuses_a_draw_on_a_function_definition() {
    // a draw of distrib on a function definition which carries a lambda as well: the reader
    // ignores every element which is not core, so the definition would be read as its lambda
    // alone and the model simulated as if the draw were not there. an element of a read package
    // which is no uncertainty is refused, and the message names the element and the namespace
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="{DISTRIB_NS}" level="3" version="2" distrib:required="true"><model id="m">
<listOfFunctionDefinitions><functionDefinition id="draw">
        {DRAW}
        <math xmlns="http://www.w3.org/1998/Math/MathML"><lambda><bvar><ci>x</ci></bvar><ci>x</ci></lambda></math>
      </functionDefinition></listOfFunctionDefinitions>
<listOfParameters><parameter id="mu" value="1" constant="true"/><parameter id="b" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="b"><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><ci>draw</ci><ci>mu</ci></apply></math></assignmentRule></listOfRules>
</model></sbml>"#
    );
    let message = refusal(&xml);
    assert!(message.contains("drawFromDistribution"), "{message}");
    assert!(message.contains(DISTRIB_NS), "{message}");
}

#[test]
fn refuses_a_draw_on_a_parameter() {
    // the same element on another core element, in a document whose math is core throughout
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="{DISTRIB_NS}" level="3" version="2" distrib:required="true"><model id="m">
<listOfParameters><parameter id="p" value="1" constant="true">
        {DRAW}
      </parameter></listOfParameters>
</model></sbml>"#
    );
    let message = refusal(&xml);
    assert!(message.contains("drawFromDistribution"), "{message}");
    assert!(message.contains(DISTRIB_NS), "{message}");
}

#[test]
fn refuses_a_draw_of_an_optional_distrib() {
    // `required="false"` is the claim that the package changes no mathematics: a draw in such a
    // document is refused as well, because it changes the mathematics whatever the document
    // claims, and the elements of the package are read for their annotations either way
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="{DISTRIB_NS}" level="3" version="2" distrib:required="false"><model id="m">
<listOfParameters><parameter id="p" value="1" constant="true">
        {DRAW}
      </parameter></listOfParameters>
</model></sbml>"#
    );
    let message = refusal(&xml);
    assert!(message.contains("drawFromDistribution"), "{message}");
    assert!(message.contains(DISTRIB_NS), "{message}");
}

#[test]
fn refuses_a_draw_in_an_event() {
    // a draw which no reader of an element reaches: the walk over the document is the only
    // thing which sees it, here on the event assignment of an event
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="{DISTRIB_NS}" level="3" version="2" distrib:required="true"><model id="m">
<listOfParameters><parameter id="b" constant="false" value="0"/></listOfParameters>
<listOfEvents><event id="e">
        <trigger><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><gt/><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">time</csymbol><cn>1</cn></apply></math></trigger>
        <listOfEventAssignments><eventAssignment variable="b">
          {DRAW}
          <math xmlns="http://www.w3.org/1998/Math/MathML"><cn>1</cn></math>
        </eventAssignment></listOfEventAssignments>
      </event></listOfEvents>
</model></sbml>"#
    );
    let message = refusal(&xml);
    assert!(message.contains("drawFromDistribution"), "{message}");
    assert!(message.contains(DISTRIB_NS), "{message}");
}

#[test]
fn refuses_a_distrib_draw_in_a_rule() {
    // an uncertainty annotates, a draw computes: a csymbol of a distribution of distrib in the
    // math of the model is refused and the message names it, although distrib is read
    let rule = |math: &str| {
        format!(
            r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" xmlns:distrib="http://www.sbml.org/sbml/level3/version1/distrib/version1" level="3" version="2" distrib:required="true"><model>
<listOfParameters><parameter id="b" constant="false"/></listOfParameters>
<listOfRules><assignmentRule variable="b"><math xmlns="http://www.w3.org/1998/Math/MathML">{math}</math></assignmentRule></listOfRules>
</model></sbml>"#
        )
    };
    const NORMAL: &str = "http://www.sbml.org/sbml/symbols/distrib/normal";
    let message = |math: &str| match read(&rule(math)).unwrap_err() {
        ReadError::Xml(m) => m,
        other => panic!("a draw is refused as a math which is not understood: {other}"),
    };
    let applied = message(&format!(
        r#"<apply><csymbol encoding="text" definitionURL="{NORMAL}">normal</csymbol><cn>0</cn><cn>1</cn></apply>"#
    ));
    assert!(
        applied.contains(&format!("csymbol of the definitionURL {NORMAL}")),
        "{applied}"
    );
    // the csymbol on its own, outside of an operator position
    let bare = message(&format!(
        r#"<csymbol encoding="text" definitionURL="{NORMAL}">normal</csymbol>"#
    ));
    assert!(
        bare.contains(&format!("csymbol of the definitionURL {NORMAL}")),
        "{bare}"
    );
}

#[test]
fn csymbols() {
    let doc = read(&data("csymbols.xml")).unwrap();
    let m = &doc.model;
    assert_eq!(
        m.initial_assignments[0].math,
        Some(Node::Apply {
            op: Op::Plus,
            args: vec![Node::CsymbolTime, Node::CsymbolAvogadro]
        })
    );
    let Rule::Assignment {
        variable, math: mm, ..
    } = &m.rules[0]
    else {
        panic!()
    };
    assert_eq!(variable, "b");
    assert_eq!(
        *mm,
        Some(Node::Apply {
            op: Op::RateOf,
            args: vec![Node::Symbol("c".into())]
        })
    );
    let Rule::Rate {
        variable, math: mm, ..
    } = &m.rules[1]
    else {
        panic!()
    };
    assert_eq!(variable, "c");
    assert_eq!(
        *mm,
        Some(Node::Apply {
            op: Op::Delay,
            args: vec![Node::Symbol("b".into()), Node::number(2.0)]
        })
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
    let t0 = e[0].trigger.as_ref().unwrap();
    assert!(!t0.initial_value);
    assert!(t0.persistent);
    assert_eq!(
        t0.math,
        Some(Node::Apply {
            op: Op::Gt,
            args: vec![Node::Symbol("x".into()), Node::number(1.0)]
        })
    );
    assert_eq!(e[0].priority, Some(Node::number(5.0)));
    assert_eq!(e[0].delay, Some(Node::number(0.5)));
    assert_eq!(e[0].assignments.len(), 2);
    assert_eq!(e[0].assignments[1].variable, "y");
    assert_eq!(e[0].assignments[1].math, Some(Node::Symbol("x".into())));
    // an event without an id keeps its position
    assert_eq!(e[1].id, None);
    assert!(e[1].use_values_from_trigger_time);
    let t1 = e[1].trigger.as_ref().unwrap();
    assert!(t1.initial_value && !t1.persistent);
    assert_eq!(t1.math, Some(Node::True));
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
            args: vec![Node::number(2.0), Node::Symbol("x".into())]
        }
    );
    assert_eq!(
        math("<apply><root/><degree><cn type=\"integer\">3</cn></degree><ci>x</ci></apply>"),
        Node::Apply {
            op: Op::Root,
            args: vec![Node::number(3.0), Node::Symbol("x".into())]
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
                Node::number(1.0),
                Node::Apply {
                    op: Op::Lt,
                    args: vec![Node::Symbol("x".into()), Node::number(2.0)]
                }
            )],
            otherwise: Some(Box::new(Node::number(3.0)))
        }
    );
    assert_eq!(
        math("<piecewise><piece><cn>1</cn><true/></piece></piecewise>"),
        Node::Piecewise {
            pieces: vec![(Node::number(1.0), Node::True)],
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
            args: vec![Node::number(1.0)]
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
    // a csymbol is named by the text of its definitionURL (not `Some("...")`)
    let ReadError::Xml(m) = bad_math(r#"<csymbol definitionURL="urn:other">x</csymbol>"#) else {
        panic!()
    };
    assert!(m.contains("csymbol of the definitionURL urn:other"), "{m}");
    assert!(!m.contains("Some("), "{m}");
    let ReadError::Xml(m) = bad_math(
        r#"<apply><csymbol definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol><ci>x</ci></apply>"#,
    ) else {
        panic!()
    };
    assert!(
        m.contains(
            "csymbol of the definitionURL http://www.sbml.org/sbml/symbols/time cannot be applied"
        ),
        "{m}"
    );
    let ReadError::Xml(m) = bad_math("<apply><csymbol>f</csymbol><ci>x</ci></apply>") else {
        panic!()
    };
    assert!(
        m.contains("csymbol without a definitionURL cannot be applied"),
        "{m}"
    );
    assert!(matches!(read("<notxml"), Err(ReadError::Xml(_))));
}

/// Reads every prepared case of the SBML Test Suite when `testdata/cases` exists.
#[test]
fn reads_all_cases() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases");
    if !root.is_dir() {
        assert!(
            std::env::var_os("SBMLSIM_CORE_REQUIRE_CASES").is_none(),
            "testdata/cases is absent and SBMLSIM_CORE_REQUIRE_CASES is set"
        );
        println!("testdata/cases absent, run `uv run scripts/prepare_testsuite.py`");
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

/// A model of `body` in SBML Level 3 Version 2.
fn model(body: &str) -> sbmlsim_sbml::Model {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model>{body}</model></sbml>"#
    );
    read(&xml).unwrap().model
}

const M: &str = r#"<math xmlns="http://www.w3.org/1998/Math/MathML">"#;

#[test]
fn case_01234_an_initial_assignment_without_math_is_read() {
    let m = model(
        r#"<listOfInitialAssignments><initialAssignment symbol="p"/></listOfInitialAssignments>"#,
    );
    assert_eq!(m.initial_assignments[0].symbol, "p");
    assert_eq!(m.initial_assignments[0].math, None);
}

#[test]
fn case_01235_rules_without_math_are_read() {
    let m = model(
        r#"<listOfRules><assignmentRule variable="a"/><rateRule variable="b"/><algebraicRule/></listOfRules>"#,
    );
    assert!(
        matches!(&m.rules[0], Rule::Assignment { variable, math: None, .. } if variable == "a")
    );
    assert!(matches!(&m.rules[1], Rule::Rate { variable, math: None, .. } if variable == "b"));
    assert!(matches!(&m.rules[2], Rule::Algebraic { math: None, .. }));
    assert!(m.rules.iter().all(|r| r.math().is_none()));
}

#[test]
fn case_01238_a_math_without_content_is_empty() {
    // a trigger without math, an assignment whose math has no content
    let m = model(&format!(
        r#"<listOfEvents><event id="E0"><trigger initialValue="true" persistent="true"/><listOfEventAssignments><eventAssignment variable="p">{M}
</math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#
    ));
    let e = &m.events[0];
    assert_eq!(e.trigger.as_ref().unwrap().math, None);
    assert_eq!(e.assignments[0].math, Some(Node::Empty));
}

#[test]
fn case_01239_an_event_without_a_trigger_is_read() {
    let m = model(&format!(
        r#"<listOfEvents><event id="E0"><listOfEventAssignments><eventAssignment variable="p">{M}<cn>1</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#
    ));
    assert_eq!(m.events[0].trigger, None);
    assert_eq!(m.events[0].assignments[0].math, Some(Node::number(1.0)));
}

#[test]
fn case_01241_a_delay_or_priority_without_math_is_absent() {
    let m = model(&format!(
        r#"<listOfEvents><event id="E0"><trigger initialValue="true" persistent="true">{M}<true/></math></trigger><delay id="d"/><priority/><listOfEventAssignments><eventAssignment variable="d"/></listOfEventAssignments></event></listOfEvents>"#
    ));
    let e = &m.events[0];
    assert_eq!(e.delay, None);
    assert_eq!(e.delay_id.as_deref(), Some("d"));
    assert_eq!(e.priority, None);
    // case 01243: an event assignment without math
    assert_eq!(e.assignments[0].math, None);
}

#[test]
fn an_apply_whose_only_child_is_an_apply_is_that_child() {
    // `Beer_MolBioSystems2014` of the PEtab benchmark collection: libSBML reads the outer
    // `<apply>` as the inner one, nested to any depth
    let inner = "<apply><times/><ci>a</ci><ci>b</ci></apply>";
    let expected = math(inner);
    assert_eq!(math(&format!("<apply>{inner}</apply>")), expected);
    assert_eq!(
        math(&format!("<apply><apply>{inner}</apply></apply>")),
        expected
    );
    assert_eq!(
        math(&format!(
            "<apply><divide/><apply>{inner}</apply><ci>c</ci></apply>"
        )),
        Node::Apply {
            op: Op::Divide,
            args: vec![expected, Node::Symbol("c".into())],
        }
    );
    // an `<apply>` applied to arguments is a function of higher order, which SBML has not
    let ReadError::Xml(m) = bad_math(&format!("<apply>{inner}<ci>c</ci></apply>")) else {
        panic!()
    };
    assert!(m.contains("no function of higher order"), "{m}");
}

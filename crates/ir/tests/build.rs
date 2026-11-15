use sbmlsim_ir::{
    AVOGADRO, CompileError, EntityKind, Expr, Model, Op, SelectionKind, SpeciesValue, Symbol, build,
};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const TIME: &str =
    r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;

/// A document of SBML Level 3 Version 2 around the content of a model; `$` in `body` is the
/// MathML namespace declaration.
fn doc(model_attributes: &str, body: &str) -> sbmlsim_sbml::Document {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m" {model_attributes}>{}</model></sbml>"#,
        body.replace('$', MATH)
    );
    sbmlsim_sbml::read(&xml).unwrap()
}

fn model(body: &str) -> Model {
    build(&doc("", body)).unwrap()
}

fn error(body: &str) -> CompileError {
    build(&doc("", body)).unwrap_err()
}

fn rate_of(x: &str) -> String {
    format!(
        r#"<apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/rateOf">rateOf</csymbol><ci>{x}</ci></apply>"#
    )
}

fn compartment(id: &str, size: &str, constant: bool) -> String {
    format!(r#"<compartment id="{id}" size="{size}" constant="{constant}"/>"#)
}

fn species(id: &str, attributes: &str) -> String {
    format!(
        r#"<species id="{id}" compartment="C" boundaryCondition="false" constant="false" {attributes}/>"#
    )
}

fn amount(id: &str, value: &str) -> String {
    species(
        id,
        &format!(r#"initialAmount="{value}" hasOnlySubstanceUnits="true""#),
    )
}

fn parameter(id: &str, value: &str) -> String {
    format!(r#"<parameter id="{id}" value="{value}" constant="true"/>"#)
}

fn variable(id: &str) -> String {
    format!(r#"<parameter id="{id}" constant="false"/>"#)
}

fn assignment_rule(variable: &str, math: &str) -> String {
    format!(r#"<assignmentRule variable="{variable}"><math $>{math}</math></assignmentRule>"#)
}

fn ci(id: &str) -> String {
    format!("<ci>{id}</ci>")
}

fn apply(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

fn sym(m: &Model, id: &str) -> Expr {
    Expr::Symbol(m.symbol(id).unwrap_or_else(|| panic!("no symbol {id}")))
}

fn ap(op: Op, args: Vec<Expr>) -> Expr {
    Expr::Apply { op, args }
}

fn num(x: f64) -> Expr {
    Expr::Number(x)
}

/// The expression of the assignment rule of `id`.
fn rule<'m>(m: &'m Model, id: &str) -> &'m Expr {
    let target = m.symbol(id).unwrap();
    &m.assignment_rules
        .iter()
        .find(|a| a.target == target)
        .unwrap()
        .expr
}

/// The expression of the initial evaluation of `id`.
fn initial<'m>(m: &'m Model, id: &str) -> &'m Expr {
    let target = m.symbol(id).unwrap();
    &m.initial.iter().find(|a| a.target == target).unwrap().expr
}

fn reaction(id: &str, reactants: &str, products: &str, law: &str) -> String {
    format!(
        r#"<reaction id="{id}" reversible="false"><listOfReactants>{reactants}</listOfReactants><listOfProducts>{products}</listOfProducts>{law}</reaction>"#
    )
}

fn reference(species: &str, attributes: &str) -> String {
    format!(r#"<speciesReference species="{species}" constant="true" {attributes}/>"#)
}

fn law(math: &str) -> String {
    format!("<kineticLaw><math $>{math}</math></kineticLaw>")
}

/// `S1 -> 2 S2` with the rate `k * S1`, both species amounts, `rest` after the reactions.
fn decay(extra_parameters: &str, rest: &str) -> String {
    format!(
        "<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}{}</listOfSpecies><listOfParameters>{}{extra_parameters}</listOfParameters><listOfReactions>{}</listOfReactions>{rest}",
        compartment("C", "1", true),
        amount("S1", "10"),
        amount("S2", "0"),
        parameter("k", "0.1"),
        reaction(
            "J",
            &reference("S1", r#"stoichiometry="1""#),
            &reference("S2", r#"stoichiometry="2""#),
            &law(&apply("times", &[&ci("k"), &ci("S1")]))
        )
    )
}

#[test]
fn amount_rates_from_stoichiometry() {
    let m = model(&decay("", ""));
    let v = ap(Op::Times, vec![sym(&m, "k"), sym(&m, "S1")]);
    assert_eq!(
        m.rate("S1").unwrap(),
        &ap(Op::Minus, vec![ap(Op::Times, vec![num(1.0), v.clone()])])
    );
    assert_eq!(m.rate("S2").unwrap(), &ap(Op::Times, vec![num(2.0), v]));
    assert_eq!(m.states.len(), 2);
    assert_eq!(m.rates.len(), 2);
    assert!(matches!(m.symbol("S1"), Some(Symbol::State(_))));
    assert!(matches!(m.symbol("k"), Some(Symbol::Param(_))));
}

#[test]
fn concentration_species_selection() {
    let m = model(&format!(
        "<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies>",
        compartment("C", "2", true),
        species(
            "S1",
            r#"initialConcentration="0.5" hasOnlySubstanceUnits="false""#
        )
    ));
    let s = m.selection("[S1]").unwrap();
    assert_eq!(s.kind, SelectionKind::Concentration);
    assert_eq!(s.expr, ap(Op::Divide, vec![sym(&m, "S1"), sym(&m, "C")]));
    let a = m.selection("S1").unwrap();
    assert_eq!(a.kind, SelectionKind::Amount);
    assert_eq!(a.expr, sym(&m, "S1"));
    // the amount is the concentration times the size of the compartment at init
    assert_eq!(
        initial(&m, "S1"),
        &ap(Op::Times, vec![num(0.5), sym(&m, "C")])
    );
    assert!(m.entity(m.symbol("S1").unwrap()).default.is_nan());
}

#[test]
fn assignment_rules_in_dependency_order() {
    let m = model(&format!(
        "<listOfParameters>{}{}{}</listOfParameters><listOfRules>{}{}{}</listOfRules>",
        variable("a"),
        variable("b"),
        variable("c"),
        assignment_rule("a", &apply("plus", &[&ci("b"), "<cn>1</cn>"])),
        assignment_rule("b", &apply("times", &[&ci("c"), "<cn>2</cn>"])),
        assignment_rule("c", "<cn>3</cn>"),
    ));
    let order: Vec<Symbol> = m.assignment_rules.iter().map(|a| a.target).collect();
    let expected: Vec<Symbol> = ["c", "b", "a"]
        .iter()
        .map(|id| m.symbol(id).unwrap())
        .collect();
    assert_eq!(order, expected);
    let order: Vec<Symbol> = m.initial.iter().map(|a| a.target).collect();
    assert_eq!(order, expected);
    assert_eq!(rule(&m, "a"), &ap(Op::Plus, vec![sym(&m, "b"), num(1.0)]));
    // the targets of assignment rules are parameters which are not settable
    assert!(!m.entity(m.symbol("a").unwrap()).settable);
}

#[test]
fn independent_rules_keep_document_order() {
    let m = model(&format!(
        "<listOfParameters>{}{}{}</listOfParameters><listOfRules>{}{}{}</listOfRules>",
        variable("z"),
        variable("y"),
        variable("x"),
        assignment_rule("z", "<cn>1</cn>"),
        assignment_rule("x", &ci("y")),
        assignment_rule("y", "<cn>2</cn>"),
    ));
    let order: Vec<&str> = m
        .assignment_rules
        .iter()
        .map(|a| m.entity(a.target).id.as_str())
        .collect();
    assert_eq!(order, ["z", "y", "x"]);
}

#[test]
fn rate_of_is_substituted() {
    let m = model(&decay(
        &format!("{}{}", variable("r1"), variable("r2")),
        &format!(
            "<listOfRules>{}{}</listOfRules>",
            assignment_rule("r1", &rate_of("S1")),
            assignment_rule("r2", &rate_of("k")),
        ),
    ));
    assert_eq!(rule(&m, "r1"), m.rate("S1").unwrap());
    assert_eq!(rule(&m, "r2"), &num(0.0));
}

#[test]
fn rate_of_a_concentration_species() {
    // constant compartment: the amount rate over the size
    let body = |constant: bool, rate_rule: &str| {
        format!(
            "<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfParameters>{}{}</listOfParameters><listOfRules>{}{rate_rule}</listOfRules><listOfReactions>{}</listOfReactions>",
            compartment("C", "3", constant),
            species(
                "F",
                r#"initialConcentration="1" hasOnlySubstanceUnits="false""#
            ),
            parameter("k", "0.5"),
            variable("r"),
            assignment_rule("r", &rate_of("F")),
            reaction(
                "J",
                &reference("F", r#"stoichiometry="1""#),
                "",
                &law(&apply("times", &[&ci("k"), &ci("F")]))
            )
        )
    };
    let m = model(&body(true, ""));
    let amount_rate = m.rate("F").unwrap().clone();
    assert_eq!(
        rule(&m, "r"),
        &ap(Op::Divide, vec![amount_rate, sym(&m, "C")])
    );
    // a compartment of a rate rule: n'/V - (n/V) * V' / V, as libroadrunner computes it
    let m = model(&body(
        false,
        r#"<rateRule variable="C"><math $><cn>1.3</cn></math></rateRule>"#,
    ));
    let (n, v) = (sym(&m, "F"), sym(&m, "C"));
    let expected = ap(
        Op::Minus,
        vec![
            ap(Op::Divide, vec![m.rate("F").unwrap().clone(), v.clone()]),
            ap(
                Op::Divide,
                vec![
                    ap(
                        Op::Times,
                        vec![ap(Op::Divide, vec![n, v.clone()]), num(1.3)],
                    ),
                    v,
                ],
            ),
        ],
    );
    assert_eq!(rule(&m, "r"), &expected);
    assert_eq!(m.rate("C").unwrap(), &num(1.3));
    // a compartment of an assignment rule: libroadrunner refuses it
    let e = build(&doc(
        "",
        &body(
            false,
            &assignment_rule("C", &apply("plus", &[TIME, "<cn>3</cn>"])),
        ),
    ))
    .unwrap_err();
    assert!(
        matches!(&e, CompileError::Unsupported { feature, .. } if feature.contains("rateOf")),
        "{e:?}"
    );
}

#[test]
fn rate_of_an_assignment_rule_target_is_refused() {
    let e = error(&format!(
        "<listOfParameters>{}{}</listOfParameters><listOfRules>{}{}</listOfRules>",
        variable("a"),
        variable("r"),
        assignment_rule("a", TIME),
        assignment_rule("r", &rate_of("a")),
    ));
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "rateOf of an assignment rule target".into(),
            element: "a".into()
        }
    );
}

#[test]
fn refuses_algebraic_rule() {
    let e = error(&format!(
        r#"<listOfParameters>{}</listOfParameters><listOfRules><algebraicRule><math $><ci>x</ci></math></algebraicRule></listOfRules>"#,
        variable("x")
    ));
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "algebraic rule".into(),
            element: "algebraicRule #0".into()
        }
    );
}

#[test]
fn refuses_delay() {
    let delay = r#"<apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>k</ci><cn>1</cn></apply>"#;
    let e = error(&format!(
        "<listOfParameters>{}{}</listOfParameters><listOfRules>{}</listOfRules>",
        parameter("k", "1"),
        variable("x"),
        assignment_rule("x", delay)
    ));
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "delay csymbol".into(),
            element: "assignmentRule #0 (x)".into()
        }
    );
}

#[test]
fn refuses_fast_reaction() {
    let body = decay("", "").replace(r#"reversible="false""#, r#"reversible="false" fast="true""#);
    assert_eq!(
        error(&body),
        CompileError::Unsupported {
            feature: "fast reaction".into(),
            element: "J".into()
        }
    );
}

#[test]
fn ids_named_like_functions_are_symbols() {
    let m = model(&format!(
        "<listOfParameters>{}{}{}{}{}{}</listOfParameters><listOfRules>{}</listOfRules>",
        parameter("exp", "1"),
        parameter("pi", "2"),
        parameter("time", "3"),
        parameter("Time", "4"),
        parameter("TIME", "5"),
        variable("y"),
        assignment_rule(
            "y",
            &apply(
                "plus",
                &[
                    &ci("exp"),
                    &ci("pi"),
                    &ci("time"),
                    &ci("Time"),
                    &ci("TIME"),
                    TIME
                ]
            )
        )
    ));
    assert_eq!(
        rule(&m, "y"),
        &ap(
            Op::Plus,
            vec![
                sym(&m, "exp"),
                sym(&m, "pi"),
                sym(&m, "time"),
                sym(&m, "Time"),
                sym(&m, "TIME"),
                Expr::Symbol(Symbol::Time),
            ]
        )
    );
    for id in ["exp", "pi", "time", "Time", "TIME"] {
        assert_eq!(m.selection(id).unwrap().kind, SelectionKind::Parameter);
    }
}

#[test]
fn local_parameter_shadows_global() {
    let body = decay("", "").replace(
        "</math></kineticLaw>",
        r#"</math><listOfLocalParameters><localParameter id="k" value="2"/></listOfLocalParameters></kineticLaw>"#,
    );
    let m = model(&body);
    let v = ap(Op::Times, vec![num(2.0), sym(&m, "S1")]);
    assert_eq!(
        m.rate("S2").unwrap(),
        &ap(Op::Times, vec![num(2.0), v.clone()])
    );
    assert_eq!(m.selection("J").unwrap().expr, v);
    assert_eq!(m.selection("J").unwrap().kind, SelectionKind::ReactionRate);
}

#[test]
fn conversion_factors_of_species_and_model() {
    let body =
        decay(&parameter("cf", "3"), "").replace(r#"id="S2""#, r#"id="S2" conversionFactor="cf""#);
    let m = model(&body);
    let v = ap(Op::Times, vec![sym(&m, "k"), sym(&m, "S1")]);
    assert_eq!(
        m.rate("S2").unwrap(),
        &ap(
            Op::Times,
            vec![sym(&m, "cf"), ap(Op::Times, vec![num(2.0), v.clone()])]
        )
    );
    // the conversion factor of the model applies to a species without its own
    let body = decay(
        &format!("{}{}", parameter("cf", "3"), parameter("mcf", "5")),
        "",
    )
    .replace(r#"id="S2""#, r#"id="S2" conversionFactor="cf""#);
    let m = build(&doc(r#"conversionFactor="mcf""#, &body)).unwrap();
    assert_eq!(
        m.rate("S1").unwrap(),
        &ap(
            Op::Times,
            vec![
                sym(&m, "mcf"),
                ap(Op::Minus, vec![ap(Op::Times, vec![num(1.0), v.clone()])])
            ]
        )
    );
    assert_eq!(
        m.rate("S2").unwrap(),
        &ap(
            Op::Times,
            vec![sym(&m, "cf"), ap(Op::Times, vec![num(2.0), v])]
        )
    );
}

#[test]
fn boundary_species_are_not_changed_by_reactions() {
    let body = decay("", "").replace(
        r#"id="S2" compartment="C" boundaryCondition="false""#,
        r#"id="S2" compartment="C" boundaryCondition="true""#,
    );
    let m = model(&body);
    assert!(matches!(m.symbol("S2"), Some(Symbol::Param(_))));
    assert!(m.rate("S2").is_none());
    assert_eq!(m.states.len(), 1);
    assert!(matches!(
        m.entity(m.symbol("S2").unwrap()).kind,
        EntityKind::Species {
            boundary_condition: true,
            ..
        }
    ));
    // a constant species is a parameter as well
    let body = decay("", "").replace(
        r#"id="S1" compartment="C" boundaryCondition="false" constant="false""#,
        r#"id="S1" compartment="C" boundaryCondition="false" constant="true""#,
    );
    let body = body.replace(&reference("S1", r#"stoichiometry="1""#), "");
    let m = model(&body);
    assert!(matches!(m.symbol("S1"), Some(Symbol::Param(_))));
}

#[test]
fn species_reference_read_in_math_and_assigned() {
    let body = decay(
        &variable("x"),
        &format!(
            "<listOfInitialAssignments><initialAssignment symbol=\"sr\"><math $><cn>3</cn></math></initialAssignment></listOfInitialAssignments><listOfRules>{}</listOfRules>",
            assignment_rule("x", &ci("sr"))
        ),
    )
    .replace(
        &reference("S2", r#"stoichiometry="2""#),
        &reference("S2", r#"id="sr""#),
    );
    let m = model(&body);
    let sr = m.symbol("sr").unwrap();
    assert!(matches!(sr, Symbol::Param(_)));
    assert_eq!(m.entity(sr).kind, EntityKind::SpeciesReference);
    assert!(m.entity(sr).default.is_nan());
    assert_eq!(rule(&m, "x"), &Expr::Symbol(sr));
    assert_eq!(initial(&m, "sr"), &num(3.0));
    // sr is evaluated before x
    let order: Vec<Symbol> = m.initial.iter().map(|a| a.target).collect();
    assert_eq!(order, [sr, m.symbol("x").unwrap()]);
    let v = ap(Op::Times, vec![sym(&m, "k"), sym(&m, "S1")]);
    assert_eq!(
        m.rate("S2").unwrap(),
        &ap(Op::Times, vec![Expr::Symbol(sr), v])
    );
    assert_eq!(
        m.selection("sr").unwrap().kind,
        SelectionKind::SpeciesReference
    );
}

#[test]
fn initial_concentration_in_an_assigned_compartment() {
    let m = model(&format!(
        r#"<listOfCompartments><compartment id="C" constant="true"/></listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfParameters>{}</listOfParameters><listOfInitialAssignments><initialAssignment symbol="C"><math $>{}</math></initialAssignment></listOfInitialAssignments>"#,
        species(
            "S",
            r#"initialConcentration="0.5" hasOnlySubstanceUnits="false""#
        ),
        parameter("p", "2"),
        apply("times", &["<cn>2</cn>", &ci("p")])
    ));
    let order: Vec<&str> = m
        .initial
        .iter()
        .map(|a| m.entity(a.target).id.as_str())
        .collect();
    assert_eq!(order, ["C", "S"]);
    assert_eq!(
        initial(&m, "S"),
        &ap(Op::Times, vec![num(0.5), sym(&m, "C")])
    );
    assert_eq!(
        initial(&m, "C"),
        &ap(Op::Times, vec![num(2.0), sym(&m, "p")])
    );
}

#[test]
fn a_cycle_is_named() {
    let e = error(&format!(
        "<listOfParameters>{}{}{}</listOfParameters><listOfRules>{}{}{}</listOfRules>",
        variable("u"),
        variable("a"),
        variable("b"),
        assignment_rule("u", "<cn>1</cn>"),
        assignment_rule("a", &apply("plus", &[&ci("b"), &ci("u")])),
        assignment_rule("b", &ci("a")),
    ));
    let CompileError::Invalid(message) = e else {
        panic!("{e:?}")
    };
    assert!(message.contains("cycle"), "{message}");
    assert!(message.contains("a, b"), "{message}");
    assert!(
        !message.contains("u,") && !message.contains(", u"),
        "{message}"
    );
    // a cycle of initial assignments
    let e = error(&format!(
        r#"<listOfParameters>{}{}</listOfParameters><listOfInitialAssignments><initialAssignment symbol="a"><math $><ci>b</ci></math></initialAssignment><initialAssignment symbol="b"><math $><ci>a</ci></math></initialAssignment></listOfInitialAssignments>"#,
        parameter("a", "1"),
        parameter("b", "1"),
    ));
    let CompileError::Invalid(message) = e else {
        panic!("{e:?}")
    };
    assert!(
        message.contains("cycle") && message.contains("a, b"),
        "{message}"
    );
}

#[test]
fn avogadro_is_the_value_of_libroadrunner() {
    assert_eq!(AVOGADRO, 6.02214179e23);
    let m = model(&format!(
        r#"<listOfParameters>{}</listOfParameters><listOfRules>{}</listOfRules>"#,
        variable("x"),
        assignment_rule(
            "x",
            r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/avogadro">avogadro</csymbol>"#
        )
    ));
    assert_eq!(rule(&m, "x"), &num(6.02214179e23));
}

#[test]
fn missing_values_are_nan() {
    let m = model(&format!(
        r#"<listOfCompartments><compartment id="C" constant="true"/></listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfParameters><parameter id="p" constant="true"/></listOfParameters>"#,
        species("S", r#"hasOnlySubstanceUnits="false""#)
    ));
    for id in ["C", "S", "p"] {
        assert!(m.entity(m.symbol(id).unwrap()).default.is_nan(), "{id}");
    }
    assert!(m.initial.is_empty());
}

#[test]
fn function_definitions_are_inlined() {
    let m = model(&format!(
        r#"<listOfFunctionDefinitions><functionDefinition id="f"><math $><lambda><bvar><ci>x</ci></bvar><bvar><ci>y</ci></bvar>{}</lambda></math></functionDefinition></listOfFunctionDefinitions><listOfParameters>{}{}</listOfParameters><listOfRules>{}</listOfRules>"#,
        apply("divide", &[&ci("x"), &ci("y")]),
        parameter("k", "1"),
        variable("z"),
        assignment_rule(
            "z",
            &format!(
                "<apply><ci>f</ci>{}<cn>49</cn></apply>",
                apply("plus", &[&ci("k"), TIME])
            )
        )
    ));
    assert_eq!(
        rule(&m, "z"),
        &ap(
            Op::Divide,
            vec![
                ap(Op::Plus, vec![sym(&m, "k"), Expr::Symbol(Symbol::Time)]),
                num(49.0)
            ]
        )
    );
}

#[test]
fn rate_rule_species_stores_its_value() {
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfRules><rateRule variable="R"><math $><cn>0.25</cn></math></rateRule></listOfRules>"#,
        compartment("C", "3", true),
        species("R", r#"initialAmount="6" hasOnlySubstanceUnits="false""#)
    ));
    let r = m.symbol("R").unwrap();
    assert!(matches!(r, Symbol::State(_)));
    assert!(matches!(
        m.entity(r).kind,
        EntityKind::Species {
            value: SpeciesValue::Concentration,
            ..
        }
    ));
    assert_eq!(m.rate("R").unwrap(), &num(0.25));
    assert_eq!(
        initial(&m, "R"),
        &ap(Op::Divide, vec![num(6.0), sym(&m, "C")])
    );
    assert_eq!(m.selection("[R]").unwrap().expr, Expr::Symbol(r));
    assert_eq!(
        m.selection("R").unwrap().expr,
        ap(Op::Times, vec![Expr::Symbol(r), sym(&m, "C")])
    );
}

#[test]
fn reaction_without_kinetic_law_has_rate_zero() {
    let body = decay("", "").replace(&law(&apply("times", &[&ci("k"), &ci("S1")])), "");
    let m = model(&body);
    assert_eq!(m.selection("J").unwrap().expr, num(0.0));
}

#[test]
fn units_of_selections() {
    let body = format!(
        r#"<listOfUnitDefinitions><unitDefinition id="mM"><listOfUnits><unit kind="mole" exponent="1" scale="-3" multiplier="1"/><unit kind="litre" exponent="-1" scale="0" multiplier="1"/></listOfUnits></unitDefinition></listOfUnitDefinitions>{}"#,
        decay(
            r#"<parameter id="c" value="1" units="mM" constant="true"/>"#,
            ""
        )
        .replace(r#"id="C""#, r#"id="C" spatialDimensions="3""#)
    );
    let m = build(&doc(
        r#"substanceUnits="mole" volumeUnits="litre" extentUnits="mole" timeUnits="second""#,
        &body,
    ))
    .unwrap();
    let unit = |id: &str| m.selection(id).unwrap().unit.clone();
    assert_eq!(unit("c").as_deref(), Some("0.001*mole*litre**-1"));
    assert_eq!(unit("S1").as_deref(), Some("mole"));
    assert_eq!(unit("[S1]").as_deref(), Some("mole/litre"));
    assert_eq!(unit("C").as_deref(), Some("litre"));
    assert_eq!(unit("J").as_deref(), Some("mole/second"));
    assert_eq!(unit("k"), None);
}

#[test]
fn a_cycle_through_rate_of_is_named() {
    let e = error(&format!(
        r#"<listOfParameters>{}</listOfParameters><listOfRules><rateRule variable="x"><math $>{}</math></rateRule></listOfRules>"#,
        r#"<parameter id="x" value="1" constant="false"/>"#,
        rate_of("x")
    ));
    assert_eq!(
        e,
        CompileError::Invalid("a cycle of rates through rateOf: x".into())
    );
}

#[test]
fn rate_of_a_boundary_species_in_a_variable_compartment_is_refused() {
    // libroadrunner gives 0 although the concentration changes with the compartment
    let e = error(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfParameters>{}</listOfParameters><listOfRules><rateRule variable="C"><math $><cn>1</cn></math></rateRule>{}</listOfRules>"#,
        compartment("C", "3", false),
        species(
            "B",
            r#"initialConcentration="1" hasOnlySubstanceUnits="false""#
        )
        .replace(
            r#"boundaryCondition="false""#,
            r#"boundaryCondition="true""#
        ),
        variable("r"),
        assignment_rule("r", &rate_of("B")),
    ));
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "rateOf of a concentration species which reactions do not change in a variable compartment".into(),
            element: "B".into()
        }
    );
}

#[test]
fn event_assignments_name_their_targets() {
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfParameters>{}</listOfParameters><listOfEvents><event useValuesFromTriggerTime="false"><trigger initialValue="false" persistent="true"><math $>{}</math></trigger><priority><math $><cn>2</cn></math></priority><listOfEventAssignments><eventAssignment variable="S"><math $><cn>4</cn></math></eventAssignment><eventAssignment variable="p"><math $><ci>S</ci></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        compartment("C", "2", true),
        species(
            "S",
            r#"initialConcentration="1" hasOnlySubstanceUnits="false""#
        ),
        r#"<parameter id="p" value="0" constant="false"/>"#,
        apply("gt", &[TIME, "<cn>1</cn>"]),
    ));
    let e = &m.events[0];
    assert_eq!(e.id, None);
    assert!(!e.initial_value && e.persistent && !e.use_values_from_trigger_time);
    assert_eq!(
        e.trigger,
        ap(Op::Gt, vec![Expr::Symbol(Symbol::Time), num(1.0)])
    );
    assert_eq!(e.priority, Some(num(2.0)));
    assert_eq!(e.delay, None);
    let (s, c) = (m.symbol("S").unwrap(), m.symbol("C").unwrap());
    assert_eq!(e.assignments[0].target, s);
    assert_eq!(e.assignments[0].value, num(4.0));
    assert_eq!(e.assignments[0].compartment, Some(c));
    assert_eq!(e.assignments[1].target, m.symbol("p").unwrap());
    assert_eq!(
        e.assignments[1].value,
        ap(Op::Divide, vec![Expr::Symbol(s), Expr::Symbol(c)])
    );
    assert_eq!(e.assignments[1].compartment, None);
}

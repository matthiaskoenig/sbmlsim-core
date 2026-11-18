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
fn coupled_algebraic_rules_match_distinct_unset_parameters() {
    // x is eligible in both rows. Matching the second row must reassign the first to y,
    // even though the second row could instead use the initialized parameter z.
    let m = model(
        r#"<listOfParameters>
      <parameter id="x" constant="false"/><parameter id="y" constant="false"/>
      <parameter id="z" value="1" constant="false"/><parameter id="t" value="0" constant="false"/>
    </listOfParameters><listOfRules>
      <algebraicRule><math $><apply><plus/><ci>x</ci><ci>y</ci></apply></math></algebraicRule>
      <algebraicRule><math $><apply><minus/><ci>x</ci><ci>z</ci></apply></math></algebraicRule>
      <rateRule variable="t"><math $><ci>x</ci></math></rateRule>
    </listOfRules>"#,
    );
    assert_eq!(m.n_algebraic, 2);
    assert_eq!(
        m.states.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["t", "y", "x"]
    );
    assert!(
        m.states[1..]
            .iter()
            .all(|s| !s.settable && s.default.is_finite())
    );
    assert_eq!(m.rate("t"), Some(&sym(&m, "x")));
    assert!(m.rate("x").is_none());
    assert_eq!(m.rates[1], ap(Op::Plus, vec![sym(&m, "x"), sym(&m, "y")]));
}

#[test]
fn algebraic_matching_expands_assignment_and_function_dependencies() {
    let m = model(
        r#"<listOfFunctionDefinitions><functionDefinition id="f"><math $>
      <lambda><bvar><ci>a</ci></bvar><bvar><ci>b</ci></bvar><ci>b</ci></lambda>
    </math></functionDefinition></listOfFunctionDefinitions><listOfParameters>
      <parameter id="x" constant="false" value="2"/><parameter id="y" constant="false" value="3"/>
      <parameter id="z" constant="false"/>
    </listOfParameters><listOfRules>
      <assignmentRule variable="z"><math $><apply><ci>f</ci><ci>x</ci><ci>y</ci></apply></math></assignmentRule>
      <algebraicRule><math $><ci>z</ci></math></algebraicRule>
    </listOfRules>"#,
    );
    assert_eq!(m.states[0].id, "y");
    assert_eq!(m.rates, [sym(&m, "z")]);
}

#[test]
fn an_algebraic_species_stores_the_quantity_seen_by_math() {
    let m = model(
        r#"<listOfCompartments><compartment id="C" size="3" constant="false"/></listOfCompartments>
    <listOfSpecies><species id="S" compartment="C" initialAmount="6" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
    <listOfRules><algebraicRule><math $><ci>S</ci></math></algebraicRule></listOfRules>"#,
    );
    assert_eq!(m.n_algebraic, 1);
    assert_eq!(m.states[0].id, "S");
    assert_eq!(m.selection("[S]").unwrap().expr, sym(&m, "S"));
    assert_eq!(
        m.selection("S").unwrap().expr,
        ap(Op::Times, vec![sym(&m, "S"), sym(&m, "C")])
    );
    assert_eq!(
        m.initial[0].expr,
        ap(Op::Divide, vec![num(6.0), sym(&m, "C")])
    );
}

#[test]
fn an_algebraic_rule_cannot_redetermine_a_reacting_species_or_a_rateof_target() {
    let e = error(&format!(
        r#"<listOfCompartments>{}</listOfCompartments>
    <listOfSpecies>{}</listOfSpecies><listOfParameters>
    <parameter id="p" value="2" constant="false"/><parameter id="q" constant="false"/>
    </listOfParameters><listOfRules>
    <assignmentRule variable="q"><math $>{}</math></assignmentRule>
    <algebraicRule><math $><apply><plus/><ci>p</ci><ci>S</ci></apply></math></algebraicRule></listOfRules>
    <listOfReactions><reaction id="r" reversible="false"><listOfProducts><speciesReference species="S" constant="true"/></listOfProducts></reaction></listOfReactions>"#,
        compartment("C", "1", true),
        amount("S", "1"),
        rate_of("p")
    ));
    assert_eq!(
        e.to_string(),
        "overdetermined algebraic system: no independent variable for algebraicRule #1"
    );
}

#[test]
fn preserves_delay_operands() {
    let delay = r#"<apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol><ci>k</ci><cn>1</cn></apply>"#;
    let m = model(&format!(
        "<listOfParameters>{}{}</listOfParameters><listOfRules>{}</listOfRules>",
        parameter("k", "1"),
        variable("x"),
        assignment_rule("x", delay)
    ));
    assert!(
        matches!(&m.assignment_rules[0].expr, Expr::Apply { op: Op::Delay, args } if args.len()==2)
    );
}

#[test]
fn rejects_a_delay_symbol_without_operands() {
    let body = format!(
        "<listOfParameters>{}</listOfParameters><listOfRules>{}</listOfRules>",
        variable("x"),
        assignment_rule(
            "x",
            r#"<csymbol definitionURL="http://www.sbml.org/sbml/symbols/delay">delay</csymbol>"#
        )
    );
    assert!(
        matches!(error(&body),CompileError::Invalid(reason) if reason.contains("delay is not applied"))
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
    // libroadrunner: a missing stoichiometry is 1, the initial assignment sets it
    assert_eq!(m.entity(sr).default, 1.0);
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

/// A parameter `p` and `extra` (rules, initial assignments, events) after it.
fn parameter_without_value(constant: bool, extra: &str) -> Result<Model, CompileError> {
    build(&doc(
        "",
        &format!(
            r#"<listOfParameters><parameter id="p" constant="{constant}"/></listOfParameters>{extra}"#
        ),
    ))
}

#[test]
fn a_missing_compartment_size_is_one() {
    let m = model(&format!(
        r#"<listOfCompartments><compartment id="C" constant="true"/><compartment id="R" constant="false"/></listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfRules><rateRule variable="R"><math $><cn>1</cn></math></rateRule></listOfRules>"#,
        species(
            "S",
            r#"initialConcentration="2" hasOnlySubstanceUnits="false""#
        )
    ));
    assert_eq!(m.entity(m.symbol("C").unwrap()).default, 1.0);
    assert_eq!(m.entity(m.symbol("R").unwrap()).default, 1.0);
    assert_eq!(
        initial(&m, "S"),
        &ap(Op::Times, vec![num(2.0), sym(&m, "C")])
    );
}

#[test]
fn a_missing_species_value_is_zero() {
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}{}{}</listOfSpecies><listOfRules><rateRule variable="R"><math $><cn>1</cn></math></rateRule></listOfRules>"#,
        compartment("C", "2", true),
        species("S", r#"hasOnlySubstanceUnits="false""#),
        species("H", r#"hasOnlySubstanceUnits="true""#).replace(
            r#"boundaryCondition="false" constant="false""#,
            r#"boundaryCondition="true" constant="true""#
        ),
        species("R", r#"hasOnlySubstanceUnits="false""#),
    ));
    for id in ["S", "H", "R"] {
        assert_eq!(m.entity(m.symbol(id).unwrap()).default, 0.0, "{id}");
    }
    // R stores its concentration, 0 as well
    assert!(matches!(
        m.entity(m.symbol("R").unwrap()).kind,
        EntityKind::Species {
            value: SpeciesValue::Concentration,
            ..
        }
    ));
    assert!(m.initial.is_empty());
}

#[test]
fn a_missing_stoichiometry_is_one() {
    let body = decay("", "")
        .replace(
            &reference("S1", r#"stoichiometry="1""#),
            &reference("S1", ""),
        )
        .replace(
            &reference("S2", r#"stoichiometry="2""#),
            &reference("S2", r#"id="sr""#),
        );
    let m = model(&body);
    let v = ap(Op::Times, vec![sym(&m, "k"), sym(&m, "S1")]);
    assert_eq!(
        m.rate("S1").unwrap(),
        &ap(Op::Minus, vec![ap(Op::Times, vec![num(1.0), v.clone()])])
    );
    assert_eq!(m.entity(m.symbol("sr").unwrap()).default, 1.0);
    assert_eq!(
        m.rate("S2").unwrap(),
        &ap(Op::Times, vec![sym(&m, "sr"), v])
    );
}

#[test]
fn a_parameter_without_a_value_is_refused_unless_assigned() {
    let refused = CompileError::Unsupported {
        feature: "parameter without a value".into(),
        element: "p".into(),
    };
    // libroadrunner: "Global parameter 'p' is missing a value", also with a rate rule or an event
    assert_eq!(parameter_without_value(true, "").unwrap_err(), refused);
    assert_eq!(parameter_without_value(false, "").unwrap_err(), refused);
    let rate_rule =
        r#"<listOfRules><rateRule variable="p"><math $><cn>1</cn></math></rateRule></listOfRules>"#;
    assert_eq!(
        parameter_without_value(false, rate_rule).unwrap_err(),
        refused
    );
    let event = format!(
        r#"<listOfEvents><event><trigger initialValue="false" persistent="true"><math $>{}</math></trigger><listOfEventAssignments><eventAssignment variable="p"><math $><cn>4</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        apply("gt", &[TIME, "<cn>0.5</cn>"])
    );
    assert_eq!(parameter_without_value(false, &event).unwrap_err(), refused);
    // an initial assignment or an assignment rule gives it its value
    let ia = r#"<listOfInitialAssignments><initialAssignment symbol="p"><math $><cn>2</cn></math></initialAssignment></listOfInitialAssignments>"#;
    let m = parameter_without_value(true, ia).unwrap();
    assert_eq!(initial(&m, "p"), &num(2.0));
    let m = parameter_without_value(
        false,
        &format!(
            "<listOfRules>{}</listOfRules>",
            assignment_rule("p", "<cn>3</cn>")
        ),
    )
    .unwrap();
    assert_eq!(rule(&m, "p"), &num(3.0));
}

#[test]
fn a_local_parameter_without_a_value_is_nan() {
    let body = decay("", "").replace(
        "</math></kineticLaw>",
        r#"</math><listOfLocalParameters><localParameter id="k"/></listOfLocalParameters></kineticLaw>"#,
    );
    let m = model(&body);
    let Expr::Apply { args, .. } = &m.selection("J").unwrap().expr else {
        panic!()
    };
    assert!(matches!(args[0], Expr::Number(x) if x.is_nan()));
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

/// The value parameter `p` = 3, not constant, and `rest` after the parameters.
fn p3(rest: &str) -> Model {
    model(&format!(
        r#"<listOfParameters><parameter id="p" value="3" constant="false"/></listOfParameters>{rest}"#
    ))
}

/// `p` is a parameter of the default 3 which nothing assigns.
fn untouched(m: &Model) {
    let p = m.symbol("p").unwrap();
    assert!(matches!(p, Symbol::Param(_)));
    assert_eq!(m.entity(p).default, 3.0);
    assert!(m.entity(p).settable);
    assert!(m.states.is_empty() && m.rates.is_empty());
    assert!(m.assignment_rules.is_empty() && m.initial.is_empty());
}

#[test]
fn case_01234_an_initial_assignment_without_math_is_ignored() {
    // libroadrunner: p = 3, and no initial assignment of p
    untouched(&p3(
        r#"<listOfInitialAssignments><initialAssignment symbol="p"/></listOfInitialAssignments>"#,
    ));
    // it gives a parameter without a value no value
    let e = error(
        r#"<listOfParameters><parameter id="p" constant="true"/></listOfParameters><listOfInitialAssignments><initialAssignment symbol="p"/></listOfInitialAssignments>"#,
    );
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "parameter without a value".into(),
            element: "p".into()
        }
    );
}

#[test]
fn case_01554_an_initial_assignment_without_math_keeps_the_initial_concentration() {
    // libroadrunner: S = 6, [S] = 3 for initialConcentration 3 in C = 2
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfInitialAssignments><initialAssignment symbol="S"/></listOfInitialAssignments>"#,
        compartment("C", "2", true),
        species(
            "S",
            r#"initialConcentration="3" hasOnlySubstanceUnits="false""#
        )
    ));
    assert_eq!(
        initial(&m, "S"),
        &ap(Op::Times, vec![num(3.0), sym(&m, "C")])
    );
}

#[test]
fn case_01235_an_assignment_rule_without_math_is_ignored() {
    untouched(&p3(
        r#"<listOfRules><assignmentRule variable="p"/></listOfRules>"#,
    ));
    // libroadrunner does not look at its variable either, nor count it as the second rule
    untouched(&p3(
        r#"<listOfRules><assignmentRule variable="zz"/><assignmentRule variable="p"/><assignmentRule variable="p"/></listOfRules>"#,
    ));
    let m = p3(
        r#"<listOfInitialAssignments><initialAssignment symbol="p"><math $><cn>7</cn></math></initialAssignment></listOfInitialAssignments><listOfRules><assignmentRule variable="p"/></listOfRules>"#,
    );
    assert_eq!(initial(&m, "p"), &num(7.0));
    assert!(m.entity(m.symbol("p").unwrap()).settable);
}

#[test]
fn case_01236_a_rate_rule_without_math_is_ignored() {
    untouched(&p3(
        r#"<listOfRules><rateRule variable="p"/></listOfRules>"#,
    ));
    // with an assignment rule with math of the same variable, the assignment rule holds
    let m = p3(&format!(
        r#"<listOfRules><rateRule variable="p"/>{}</listOfRules>"#,
        assignment_rule("p", "<cn>5</cn>")
    ));
    assert_eq!(rule(&m, "p"), &num(5.0));
}

#[test]
fn case_01461_rate_of_a_variable_of_a_rate_rule_without_math_is_zero() {
    let m = model(&format!(
        r#"<listOfParameters>{}<parameter id="p" value="2" constant="false"/></listOfParameters><listOfRules>{}<rateRule variable="p"/></listOfRules>"#,
        variable("x"),
        assignment_rule("x", &rate_of("p"))
    ));
    assert_eq!(rule(&m, "x"), &num(0.0));
}

#[test]
fn case_01556_a_species_of_a_rate_rule_without_math_is_changed_by_its_reactions() {
    // libroadrunner: S(1) = 2 for S(0) = 3 and a rate of the reaction 1
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfRules><rateRule variable="S"/></listOfRules><listOfReactions>{}</listOfReactions>"#,
        compartment("C", "2", true),
        amount("S", "3"),
        reaction(
            "J",
            &reference("S", r#"stoichiometry="1""#),
            "",
            &law("<cn>1</cn>")
        )
    ));
    assert!(matches!(m.symbol("S"), Some(Symbol::State(_))));
    assert_eq!(
        m.rate("S").unwrap(),
        &ap(Op::Minus, vec![ap(Op::Times, vec![num(1.0), num(1.0)])])
    );
}

#[test]
fn case_01464_a_species_reference_of_a_rate_rule_without_math_is_constant() {
    let m = model(&format!(
        r#"<listOfCompartments>{}</listOfCompartments><listOfSpecies>{}</listOfSpecies><listOfRules><rateRule variable="sr"/></listOfRules><listOfReactions>{}</listOfReactions>"#,
        compartment("C", "1", true),
        amount("S", "0"),
        reaction(
            "J",
            "",
            r#"<speciesReference id="sr" species="S" stoichiometry="2" constant="false"/>"#,
            &law("<cn>1</cn>")
        )
    ));
    let sr = m.symbol("sr").unwrap();
    assert!(matches!(sr, Symbol::Param(_)));
    assert_eq!(m.entity(sr).default, 2.0);
    assert_eq!(m.states.len(), 1);
}

#[test]
fn case_01244_an_algebraic_rule_without_math_is_ignored() {
    untouched(&p3(r#"<listOfRules><algebraicRule/></listOfRules>"#));
}

/// An event `E` at `time > 0.5` with `parts` before its assignments.
fn event_of(trigger: &str, parts: &str, assignments: &str) -> String {
    format!(
        r#"<listOfEvents><event id="E" useValuesFromTriggerTime="true">{trigger}{parts}<listOfEventAssignments>{assignments}</listOfEventAssignments></event></listOfEvents>"#
    )
}

fn trigger() -> String {
    format!(
        r#"<trigger initialValue="true" persistent="true"><math $>{}</math></trigger>"#,
        apply("gt", &[TIME, "<cn>0.5</cn>"])
    )
}

const EMPTY_ASSIGNMENT: &str = r#"<eventAssignment variable="p"><math $>
</math></eventAssignment>"#;

#[test]
fn case_01238_an_event_whose_trigger_has_no_math_is_ignored() {
    // with everything in it, also an assignment whose math has no content
    let m = p3(&event_of(
        r#"<trigger initialValue="true" persistent="true"/>"#,
        "",
        EMPTY_ASSIGNMENT,
    ));
    assert!(m.events.is_empty());
}

#[test]
fn case_01239_an_event_without_a_trigger_is_ignored() {
    let m = p3(&event_of("", "", EMPTY_ASSIGNMENT));
    assert!(m.events.is_empty());
}

#[test]
fn case_01238_a_math_without_content_is_refused() {
    // libroadrunner: "Unknown ASTNode type of 9999"
    let body = format!(
        r#"<listOfParameters><parameter id="p" value="3" constant="false"/></listOfParameters>{}"#,
        event_of(&trigger(), "", EMPTY_ASSIGNMENT)
    );
    assert_eq!(
        error(&body),
        CompileError::Unsupported {
            feature: "math without content".into(),
            element: "eventAssignment #0 (p) of E".into()
        }
    );
    let e = error(
        r#"<listOfParameters><parameter id="p" value="3" constant="false"/></listOfParameters><listOfRules><rateRule variable="p"><math $></math></rateRule></listOfRules>"#,
    );
    assert_eq!(
        e,
        CompileError::Unsupported {
            feature: "math without content".into(),
            element: "rateRule #0 (p)".into()
        }
    );
}

#[test]
fn case_01243_an_event_assignment_without_math_is_ignored() {
    let m = model(&format!(
        r#"<listOfParameters><parameter id="p" value="3" constant="false"/><parameter id="d" value="5" constant="false"/></listOfParameters>{}"#,
        event_of(
            &trigger(),
            "",
            r#"<eventAssignment variable="d"/><eventAssignment variable="zz"/><eventAssignment variable="p"><math $><cn>5</cn></math></eventAssignment>"#
        )
    ));
    let e = &m.events[0];
    assert_eq!(e.assignments.len(), 1);
    assert_eq!(e.assignments[0].target, m.symbol("p").unwrap());
    assert_eq!(e.assignments[0].value, num(5.0));
}

#[test]
fn case_01241_a_delay_without_math_is_none() {
    let m = p3(&event_of(
        &trigger(),
        r#"<delay/>"#,
        r#"<eventAssignment variable="p"><math $><cn>5</cn></math></eventAssignment>"#,
    ));
    assert_eq!(m.events[0].delay, None);
}

#[test]
fn case_01242_a_priority_without_math_is_none() {
    let m = p3(&event_of(
        &trigger(),
        r#"<priority/>"#,
        r#"<eventAssignment variable="p"><math $><cn>5</cn></math></eventAssignment>"#,
    ));
    assert_eq!(m.events[0].priority, None);
}

#[test]
fn case_01271_a_function_definition_without_math_is_refused_where_it_is_called() {
    let defs = |math: &str| {
        format!(
            r#"<listOfFunctionDefinitions><functionDefinition id="foo">{math}</functionDefinition></listOfFunctionDefinitions>"#
        )
    };
    // not called: libroadrunner loads the model
    untouched(&p3(&defs("")));
    untouched(&p3(&defs("<math $></math>")));
    // called: libroadrunner crashes (no math) or refuses ("must be a lambda")
    for math in ["", "<math $></math>"] {
        let e = error(&format!(
            r#"{}<listOfParameters><parameter id="p" value="3" constant="false"/></listOfParameters><listOfRules>{}</listOfRules>"#,
            defs(math),
            assignment_rule("p", "<apply><ci>foo</ci><cn>1</cn></apply>")
        ));
        assert_eq!(
            e,
            CompileError::Unsupported {
                feature: "function definition without math".into(),
                element: "foo".into()
            }
        );
    }
}

#[test]
fn two_rules_of_one_variable_are_named_in_document_order() {
    // the first rule in document order whose variable has a rule before it: the rule of b
    let rr = |v: &str| format!(r#"<rateRule variable="{v}"><math $><cn>1</cn></math></rateRule>"#);
    let e = error(&format!(
        r#"<listOfParameters>{}{}</listOfParameters><listOfRules>{}{}{}{}</listOfRules>"#,
        variable("a").replace("/>", r#" value="1"/>"#),
        variable("b").replace("/>", r#" value="1"/>"#),
        rr("a"),
        rr("b"),
        assignment_rule("b", "<cn>2</cn>"),
        assignment_rule("a", "<cn>2</cn>"),
    ));
    assert_eq!(
        e,
        CompileError::Invalid("b is the variable of two rules".into())
    );
}

/// The function definitions `defs` (`id`, bound variables, body) as SBML.
fn functions(defs: &[(&str, &[&str], &str)]) -> String {
    let defs: String = defs
        .iter()
        .map(|(id, params, body)| {
            let bvars: String = params
                .iter()
                .map(|p| format!("<bvar><ci>{p}</ci></bvar>"))
                .collect();
            format!(
                r#"<functionDefinition id="{id}"><math $><lambda>{bvars}{body}</lambda></math></functionDefinition>"#
            )
        })
        .collect();
    format!("<listOfFunctionDefinitions>{defs}</listOfFunctionDefinitions>")
}

fn call(f: &str, args: &[&str]) -> String {
    format!("<apply><ci>{f}</ci>{}</apply>", args.concat())
}

#[test]
fn a_bound_variable_named_like_a_model_id_is_the_argument() {
    // f(k) = k * 2, called with k + 1 where k is the parameter of the model
    let m = model(&format!(
        r#"{}<listOfParameters>{}{}</listOfParameters><listOfRules>{}</listOfRules>"#,
        functions(&[("f", &["k"], &apply("times", &[&ci("k"), "<cn>2</cn>"]))]),
        parameter("k", "5"),
        variable("z"),
        assignment_rule(
            "z",
            &call("f", &[&apply("plus", &[&ci("k"), "<cn>1</cn>"])])
        )
    ));
    assert_eq!(
        rule(&m, "z"),
        &ap(
            Op::Times,
            vec![ap(Op::Plus, vec![sym(&m, "k"), num(1.0)]), num(2.0)]
        )
    );
}

#[test]
fn a_function_calling_another_function_is_inlined() {
    // g(x) = f(x * x) + f(1), f(y) = x + y: the body of f sees the parameter x of the model,
    // not the bound variable x of g
    let m = model(&format!(
        r#"{}<listOfParameters>{}{}</listOfParameters><listOfRules>{}</listOfRules>"#,
        functions(&[
            ("f", &["y"], &apply("plus", &[&ci("x"), &ci("y")])),
            (
                "g",
                &["x"],
                &apply(
                    "plus",
                    &[
                        &call("f", &[&apply("times", &[&ci("x"), &ci("x")])]),
                        &call("f", &["<cn>1</cn>"])
                    ]
                )
            ),
        ]),
        parameter("x", "7"),
        variable("z"),
        assignment_rule("z", &call("g", &["<cn>3</cn>"]))
    ));
    let x = sym(&m, "x");
    assert_eq!(
        rule(&m, "z"),
        &ap(
            Op::Plus,
            vec![
                ap(
                    Op::Plus,
                    vec![x.clone(), ap(Op::Times, vec![num(3.0), num(3.0)])]
                ),
                ap(Op::Plus, vec![x, num(1.0)])
            ]
        )
    );
}

#[test]
fn a_local_parameter_as_an_argument_and_the_global_in_the_body() {
    // the kinetic law f(k) with the local parameter k = 0.5, f(a) = a * k reads the global k
    let body = format!(
        "{}{}",
        functions(&[("f", &["a"], &apply("times", &[&ci("a"), &ci("k")]))]),
        decay("", "")
    )
    .replace(
        &law(&apply("times", &[&ci("k"), &ci("S1")])),
        &format!(
            r#"<kineticLaw><math $>{}</math><listOfLocalParameters><localParameter id="k" value="0.5"/></listOfLocalParameters></kineticLaw>"#,
            call("f", &[&ci("k")])
        ),
    );
    let m = model(&body);
    assert_eq!(
        m.selection("J").unwrap().expr,
        ap(Op::Times, vec![num(0.5), sym(&m, "k")])
    );
}

#[test]
fn the_id_of_an_ignored_event_is_not_registered() {
    // libroadrunner 2.11 loads an event without a trigger whose id repeats another id (it
    // crashes when the ignored event comes after a kept one of its id; this one is kept)
    let ignored = r#"<event id="p" useValuesFromTriggerTime="true"><listOfEventAssignments><eventAssignment variable="p"><math $><cn>5</cn></math></eventAssignment></listOfEventAssignments></event>"#;
    let kept = format!(
        r#"<event id="E" useValuesFromTriggerTime="true">{}<listOfEventAssignments><eventAssignment variable="p"><math $><cn>5</cn></math></eventAssignment></listOfEventAssignments></event>"#,
        trigger()
    );
    let m = p3(&format!(
        "<listOfEvents>{ignored}{}{kept}</listOfEvents>",
        ignored.replace(r#"id="p""#, r#"id="E""#)
    ));
    assert_eq!(m.events.len(), 1);
    assert_eq!(m.events[0].id.as_deref(), Some("E"));
    // two kept events of one id are invalid
    let e = error(&format!(
        r#"<listOfParameters><parameter id="p" value="3" constant="false"/></listOfParameters><listOfEvents>{kept}{kept}</listOfEvents>"#
    ));
    assert_eq!(
        e,
        CompileError::Invalid("the id E is used more than once".into())
    );
}

#[test]
fn an_event_whose_assignments_have_no_math_is_kept() {
    // libroadrunner lists it in getEventIds (cases 01237, 01600-01603, 01605)
    let m = p3(&event_of(
        &trigger(),
        "",
        r#"<eventAssignment variable="p"/>"#,
    ));
    assert_eq!(m.events.len(), 1);
    assert!(m.events[0].assignments.is_empty());
    assert_eq!(m.events[0].root_functions().len(), 1);
}

#[test]
fn root_functions_of_triggers() {
    let p = || sym(&p3(""), "p");
    let root = |trigger: &str| {
        let m = p3(&event_of(
            &format!(
                r#"<trigger initialValue="true" persistent="true"><math $>{trigger}</math></trigger>"#
            ),
            "",
            "",
        ));
        m.events[0].root_functions()
    };
    let t = || Expr::Symbol(Symbol::Time);
    let minus = |a: Expr, b: Expr| ap(Op::Minus, vec![a, b]);
    // positive where the comparison holds
    assert_eq!(
        root(&apply("gt", &[TIME, "<cn>1</cn>"])),
        [minus(t(), num(1.0))]
    );
    assert_eq!(
        root(&apply("geq", &[TIME, "<cn>1</cn>"])),
        [minus(t(), num(1.0))]
    );
    assert_eq!(
        root(&apply("lt", &[TIME, "<cn>1</cn>"])),
        [minus(num(1.0), t())]
    );
    assert_eq!(
        root(&apply("leq", &[TIME, "<cn>1</cn>"])),
        [minus(num(1.0), t())]
    );
    // a chain gives one per adjacent pair, and boolean operators collect their arguments
    assert_eq!(
        root(&apply(
            "and",
            &[
                &apply("lt", &["<cn>0</cn>", &ci("p"), "<cn>2</cn>"]),
                &apply("not", &[&apply("eq", &[TIME, "<cn>3</cn>"])])
            ]
        )),
        [
            minus(p(), num(0.0)),
            minus(num(2.0), p()),
            minus(t(), num(3.0))
        ]
    );
    // a number read as a boolean is its own root function, a boolean constant has none
    assert_eq!(root(&ci("p")), [p()]);
    assert_eq!(root("<true/>"), []);
    // a comparison inside a number of a comparison gives its own
    assert_eq!(
        root(&apply(
            "gt",
            &[
                &format!(
                    r#"<piecewise><piece><cn>1</cn>{}</piece><otherwise><cn>0</cn></otherwise></piecewise>"#,
                    apply("gt", &[TIME, "<cn>2</cn>"])
                ),
                "<cn>0.5</cn>"
            ]
        )),
        [
            minus(
                Expr::Piecewise {
                    pieces: vec![(num(1.0), ap(Op::Gt, vec![t(), num(2.0)]))],
                    otherwise: Some(Box::new(num(0.0)))
                },
                num(0.5)
            ),
            minus(t(), num(2.0))
        ]
    );
}

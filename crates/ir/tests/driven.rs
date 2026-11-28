//! Driven species: the transform of the model representation and its refusals.

use sbmlsim_abi::DrivenSpecies;
use sbmlsim_ir::{
    Assignment, CompileError, EntityKind, Expr, Model, Op, SelectionKind, Symbol, build, driven,
};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;

/// A document of SBML Level 3 Version 2 around the content of a model; `$` in `body` is the
/// MathML namespace declaration.
fn doc(body: &str) -> sbmlsim_sbml::Document {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">{}</model></sbml>"#,
        body.replace('$', MATH)
    );
    sbmlsim_sbml::read(&xml).unwrap()
}

fn model(body: &str) -> Model {
    build(&doc(body)).unwrap()
}

fn ci(id: &str) -> String {
    format!("<ci>{id}</ci>")
}

fn apply(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

fn csymbol(name: &str, args: &[&str]) -> String {
    format!(
        r#"<apply><csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/{name}">{name}</csymbol>{}</apply>"#,
        args.concat()
    )
}

fn sym(m: &Model, id: &str) -> Expr {
    Expr::Symbol(m.symbol(id).unwrap_or_else(|| panic!("no symbol {id}")))
}

fn ap(op: Op, args: Vec<Expr>) -> Expr {
    Expr::Apply { op, args }
}

fn param(m: &Model, id: &str) -> u32 {
    match m.symbol(id) {
        Some(Symbol::Param(i)) => i,
        other => panic!("{id} is no parameter: {other:?}"),
    }
}

fn state(m: &Model, id: &str) -> u32 {
    match m.symbol(id) {
        Some(Symbol::State(i)) => i,
        other => panic!("{id} is no state: {other:?}"),
    }
}

/// `X__value + X__slope * (time - driven__t_ref)`.
fn path(m: &Model, x: &str) -> Expr {
    ap(
        Op::Plus,
        vec![
            sym(m, &format!("{x}__value")),
            ap(
                Op::Times,
                vec![
                    sym(m, &format!("{x}__slope")),
                    ap(
                        Op::Minus,
                        vec![Expr::Symbol(Symbol::Time), sym(m, "driven__t_ref")],
                    ),
                ],
            ),
        ],
    )
}

fn ids(entities: &[sbmlsim_ir::Entity]) -> Vec<&str> {
    entities.iter().map(|e| e.id.as_str()).collect()
}

fn entity<'m>(m: &'m Model, id: &str) -> &'m sbmlsim_ir::Entity {
    m.entity(m.symbol(id).unwrap())
}

/// `X -> C` with the rate `k * X`, `X` in `blood` (`blood` the compartment element), `C` in
/// `cell`; `x` the attributes of `X`, `extra` more model content (parameters, rules, events)
/// after the reactions, `parameters` more parameters.
fn uptake(blood: &str, x: &str, parameters: &str, extra: &str) -> String {
    format!(
        r#"<listOfCompartments>{blood}<compartment id="cell" size="1" constant="true"/></listOfCompartments><listOfSpecies><species id="X" compartment="blood" boundaryCondition="false" constant="false" {x}/><species id="C" compartment="cell" initialConcentration="0" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies><listOfParameters><parameter id="k" value="0.5" constant="true"/>{parameters}</listOfParameters><listOfReactions><reaction id="R" reversible="false"><listOfReactants><speciesReference species="X" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="C" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math $>{}</math></kineticLaw></reaction></listOfReactions>{extra}"#,
        apply("times", &[&ci("k"), &ci("X")])
    )
}

const BLOOD: &str = r#"<compartment id="blood" size="2" constant="true"/>"#;
const CONCENTRATION: &str = r#"initialConcentration="3" hasOnlySubstanceUnits="false""#;

fn drive(m: Model, ids: &[&str]) -> Result<Model, CompileError> {
    driven::apply(m, &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

#[test]
fn driven_species_leaves_the_states() {
    let x = format!(r#"{CONCENTRATION} conversionFactor="cf""#);
    let before = model(&uptake(
        BLOOD,
        &x,
        r#"<parameter id="cf" value="2" constant="true"/>"#,
        "",
    ));
    let m = drive(before.clone(), &["X"]).unwrap();

    assert_eq!(ids(&m.states), ["C", "X__exchange"]);
    assert_eq!(m.rates.len(), 2);
    assert_eq!(
        ids(&m.parameters),
        [
            "blood",
            "cell",
            "k",
            "cf",
            "X__value",
            "X__slope",
            "driven__t_ref"
        ]
    );
    // the concentration the math reads is the path, the amount of `X` its product with `blood`
    let v = ap(Op::Times, vec![sym(&m, "k"), path(&m, "X")]);
    let rate_c = ap(Op::Times, vec![Expr::Number(1.0), v]);
    assert_eq!(m.rate("C").unwrap(), &rate_c);
    // the amount rate `X` would have had: minus the rate of C times the conversion factor
    assert_eq!(
        m.rate("X__exchange").unwrap(),
        &ap(Op::Times, vec![sym(&m, "cf"), ap(Op::Minus, vec![rate_c])])
    );
    assert_eq!(
        m.driven,
        [DrivenSpecies {
            id: "X".into(),
            value: param(&m, "X__value"),
            slope: param(&m, "X__slope"),
            exchange: state(&m, "X__exchange"),
            amount: false,
        }]
    );
    assert_eq!(m.t_ref, Some(param(&m, "driven__t_ref")));
    // the initial concentration is the value of the path, the slope and t_ref 0
    assert_eq!(entity(&m, "X__value").default, 3.0);
    assert_eq!(entity(&m, "X__slope").default, 0.0);
    assert_eq!(entity(&m, "driven__t_ref").default, 0.0);
    assert_eq!(entity(&m, "X__exchange").default, 0.0);
    assert_eq!(entity(&m, "X__exchange").kind, EntityKind::Parameter);
    assert!(
        m.initial.iter().all(
            |a| !a.expr.symbols().contains(&m.symbol("X__value").unwrap())
                && a.target != m.symbol("X__value").unwrap()
        ),
        "{:?}",
        m.initial
    );
    // the selections read the path
    assert_eq!(
        m.selection("X").unwrap().expr,
        ap(Op::Times, vec![path(&m, "X"), sym(&m, "blood")])
    );
    assert_eq!(m.selection("[X]").unwrap().expr, path(&m, "X"));
    assert_eq!(
        m.selection("R").unwrap().expr,
        ap(Op::Times, vec![sym(&m, "k"), path(&m, "X")])
    );
    // the new entities can be selected, the states of C keep its amount
    for id in ["X__value", "X__slope", "driven__t_ref", "X__exchange"] {
        let s = m
            .selection(id)
            .unwrap_or_else(|| panic!("no selection {id}"));
        assert_eq!(s.kind, SelectionKind::Parameter);
        assert_eq!(s.expr, sym(&m, id));
    }
    assert_eq!(
        m.selection("C").unwrap().expr,
        Expr::Symbol(Symbol::State(0))
    );
}

#[test]
fn driven_species_in_parameter_compartment() {
    // the size of `blood` is the parameter `Vb` (an initial assignment), `X` given as an amount
    let blood = r#"<compartment id="blood" constant="true"/>"#;
    let extra = format!(
        r#"<listOfInitialAssignments><initialAssignment symbol="blood"><math $>{}</math></initialAssignment></listOfInitialAssignments>"#,
        ci("Vb")
    );
    let m = drive(
        model(&uptake(
            blood,
            r#"initialAmount="6" hasOnlySubstanceUnits="false""#,
            r#"<parameter id="Vb" value="1.5" constant="true"/>"#,
            &extra,
        )),
        &["X"],
    )
    .unwrap();

    let blood = m.symbol("blood").unwrap();
    let value = m.symbol("X__value").unwrap();
    // the path is a concentration: the amount reads `(X__value + X__slope (t - t_ref)) * blood`
    // with `blood = Vb`
    assert_eq!(
        m.selection("X").unwrap().expr,
        ap(Op::Times, vec![path(&m, "X"), Expr::Symbol(blood)])
    );
    assert_eq!(m.selection("[X]").unwrap().expr, path(&m, "X"));
    assert_eq!(
        m.rate("C").unwrap(),
        &ap(
            Op::Times,
            vec![
                Expr::Number(1.0),
                ap(Op::Times, vec![sym(&m, "k"), path(&m, "X")])
            ]
        )
    );
    // the initial amount is a concentration once the size of `blood` is known
    assert!(entity(&m, "X__value").default.is_nan());
    let at = |target| m.initial.iter().position(|a| a.target == target).unwrap();
    assert!(at(blood) < at(value), "{:?}", m.initial);
    assert_eq!(
        m.initial[at(value)],
        Assignment {
            target: value,
            expr: ap(Op::Divide, vec![Expr::Number(6.0), Expr::Symbol(blood)]),
        }
    );
    assert_eq!(m.initial[at(blood)].expr, sym(&m, "Vb"));
    assert!(!m.driven[0].amount);
}

#[test]
fn driven_amount_species() {
    let m = drive(
        model(&uptake(
            BLOOD,
            r#"initialAmount="4" hasOnlySubstanceUnits="true""#,
            "",
            "",
        )),
        &["X"],
    )
    .unwrap();
    assert!(m.driven[0].amount);
    assert_eq!(entity(&m, "X__value").default, 4.0);
    // no compartment factor: the math reads the amount, the path
    assert_eq!(
        m.rate("C").unwrap(),
        &ap(
            Op::Times,
            vec![
                Expr::Number(1.0),
                ap(Op::Times, vec![sym(&m, "k"), path(&m, "X")])
            ]
        )
    );
    assert_eq!(m.selection("X").unwrap().expr, path(&m, "X"));
    assert_eq!(
        m.selection("[X]").unwrap().expr,
        ap(Op::Divide, vec![path(&m, "X"), sym(&m, "blood")])
    );
}

#[test]
fn driven_species_read_by_rules_and_events() {
    // an assignment rule, an initial assignment, an event and a species after `X` read `X`
    let extra = format!(
        r#"<listOfInitialAssignments><initialAssignment symbol="q"><math $>{}</math></initialAssignment></listOfInitialAssignments><listOfRules><assignmentRule variable="a"><math $>{}</math></assignmentRule></listOfRules><listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math $>{}</math></trigger><listOfEventAssignments><eventAssignment variable="C"><math $>{}</math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        apply("times", &[&ci("X"), "<cn>2</cn>"]),
        ci("X"),
        apply("gt", &[&ci("X"), "<cn>1</cn>"]),
        ci("X"),
    );
    let m = drive(
        model(&uptake(
            BLOOD,
            r#"initialAmount="4" hasOnlySubstanceUnits="true""#,
            r#"<parameter id="a" constant="false"/><parameter id="q" constant="false"/>"#,
            &extra,
        )),
        &["X"],
    )
    .unwrap();
    let p = path(&m, "X");
    let a = m.symbol("a").unwrap();
    let q = m.symbol("q").unwrap();
    let rule = m.assignment_rules.iter().find(|r| r.target == a).unwrap();
    assert_eq!(rule.expr, p);
    let init = m.initial.iter().find(|r| r.target == q).unwrap();
    assert_eq!(init.expr, ap(Op::Times, vec![p.clone(), Expr::Number(2.0)]));
    let e = &m.events[0];
    assert_eq!(e.trigger, ap(Op::Gt, vec![p.clone(), Expr::Number(1.0)]));
    assert_eq!(e.assignments[0].target, m.symbol("C").unwrap());
    assert_eq!(e.assignments[0].value, p);
    assert_eq!(
        e.assignments[0].compartment,
        Some(m.symbol("cell").unwrap())
    );
}

#[test]
fn driven_species_before_algebraic_states() {
    // `z` is the algebraic unknown of `z - C = 0`, behind the accumulator after the pass
    let extra = format!(
        r#"<listOfRules><algebraicRule><math $>{}</math></algebraicRule></listOfRules>"#,
        apply("minus", &[&ci("z"), &ci("C")])
    );
    let before = model(&uptake(
        BLOOD,
        CONCENTRATION,
        r#"<parameter id="z" value="0" constant="false"/>"#,
        &extra,
    ));
    assert_eq!(ids(&before.states), ["X", "C", "z"]);
    let m = drive(before, &["X"]).unwrap();
    assert_eq!(ids(&m.states), ["C", "X__exchange", "z"]);
    assert_eq!(m.n_algebraic, 1);
    assert_eq!(m.driven[0].exchange, 1);
    assert_eq!(
        m.rates[2],
        ap(
            Op::Minus,
            vec![
                sym(&m, "z"),
                ap(Op::Divide, vec![sym(&m, "C"), sym(&m, "cell")])
            ]
        )
    );
}

#[test]
fn jacobian_of_driven_species() {
    // the rate `k * X * C`: the rows of C and of the accumulator read C, nothing reads the
    // accumulator, and no column belongs to X
    let law = apply("times", &[&ci("k"), &ci("X")]);
    let body = uptake(BLOOD, CONCENTRATION, "", "")
        .replace(&law, &apply("times", &[&ci("k"), &ci("X"), &ci("C")]));
    let m = drive(model(&body), &["X"]).unwrap();
    let jacobian = sbmlsim_ir::derive::jacobian(&m);
    let pattern = jacobian.pattern();
    assert_eq!(pattern.n, 2);
    assert_eq!(pattern.col_ptr, [0, 2, 2]);
    assert_eq!(pattern.row_idx, [0, 1]);
    assert!(
        matches!(jacobian, sbmlsim_ir::derive::JacobianIr::Analytic { .. }),
        "{jacobian:?}"
    );
}

#[test]
fn without_driven_species_unchanged() {
    let m = model(&uptake(BLOOD, CONCENTRATION, "", ""));
    let same = driven::apply(m.clone(), &[]).unwrap();
    // equal as written: the defaults given as concentrations are NaN, which `==` refuses
    assert_eq!(format!("{same:?}"), format!("{m:?}"));
    assert!(same.driven.is_empty());
    assert_eq!(same.t_ref, None);
}

/// The refusal of driving `id`: the feature names the reason, the element the species.
fn refusal(body: &str, id: &str, reason: &str) {
    match drive(model(body), &[id]) {
        Err(CompileError::Unsupported { feature, element }) => {
            assert_eq!(element, id, "{feature}");
            assert!(feature.contains(reason), "{feature} for {reason}");
            let message = CompileError::Unsupported { feature, element }.to_string();
            assert!(message.contains(id), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn refuses_unknown_id() {
    refusal(&uptake(BLOOD, CONCENTRATION, "", ""), "Y", "unknown id");
}

#[test]
fn refuses_non_species() {
    refusal(&uptake(BLOOD, CONCENTRATION, "", ""), "k", "no species");
    refusal(&uptake(BLOOD, CONCENTRATION, "", ""), "blood", "no species");
}

#[test]
fn refuses_boundary_species() {
    let body = uptake(BLOOD, CONCENTRATION, "", "").replace(
        r#"id="X" compartment="blood" boundaryCondition="false""#,
        r#"id="X" compartment="blood" boundaryCondition="true""#,
    );
    refusal(&body, "X", "boundary");
}

#[test]
fn refuses_constant_species() {
    // a constant species which no reaction changes
    let body = format!(
        r#"<listOfCompartments>{BLOOD}</listOfCompartments><listOfSpecies><species id="X" compartment="blood" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="true"/></listOfSpecies>"#
    );
    refusal(&body, "X", "constant");
}

/// A species `X` in `blood` which no reaction changes, `rest` after the species.
fn lone(x: &str, parameters: &str, rest: &str) -> String {
    format!(
        r#"<listOfCompartments>{BLOOD}</listOfCompartments><listOfSpecies><species id="X" compartment="blood" boundaryCondition="false" constant="false" {x}/></listOfSpecies><listOfParameters><parameter id="k" value="0.5" constant="true"/>{parameters}</listOfParameters>{rest}"#
    )
}

const AMOUNT: &str = r#"initialAmount="1" hasOnlySubstanceUnits="true""#;

#[test]
fn refuses_rate_rule_target() {
    let rule = format!(
        r#"<listOfRules><rateRule variable="X"><math $>{}</math></rateRule></listOfRules>"#,
        ci("k")
    );
    // stored as an amount (hasOnlySubstanceUnits) and as a concentration
    refusal(&lone(AMOUNT, "", &rule), "X", "rate rule");
    refusal(&lone(CONCENTRATION, "", &rule), "X", "rate rule");
}

#[test]
fn refuses_assignment_rule_target() {
    let rule = format!(
        r#"<listOfRules><assignmentRule variable="X"><math $>{}</math></assignmentRule></listOfRules>"#,
        ci("k")
    );
    refusal(&lone(AMOUNT, "", &rule), "X", "assignment rule");
}

#[test]
fn refuses_algebraic_rule_target() {
    let rule = format!(
        r#"<listOfRules><algebraicRule><math $>{}</math></algebraicRule></listOfRules>"#,
        apply("minus", &[&ci("X"), &ci("k")])
    );
    let m = model(&lone(AMOUNT, "", &rule));
    assert_eq!(m.n_algebraic, 1, "X is algebraic");
    refusal(&lone(AMOUNT, "", &rule), "X", "algebraic rule");
}

fn event(variable: &str) -> String {
    format!(
        r#"<listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math $>{}</math></trigger><listOfEventAssignments><eventAssignment variable="{variable}"><math $><cn>2</cn></math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        apply(
            "gt",
            &[
                r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#,
                "<cn>1</cn>"
            ]
        )
    )
}

#[test]
fn refuses_event_assignment_target() {
    refusal(&uptake(BLOOD, CONCENTRATION, "", &event("X")), "X", "event");
}

#[test]
fn refuses_species_in_variable_compartment() {
    let variable = r#"<compartment id="blood" size="2" constant="false"/>"#;
    let rate = format!(
        r#"<listOfRules><rateRule variable="blood"><math $>{}</math></rateRule></listOfRules>"#,
        ci("k")
    );
    let assigned = format!(
        r#"<listOfRules><assignmentRule variable="blood"><math $>{}</math></assignmentRule></listOfRules>"#,
        ci("k")
    );
    for rest in [rate, assigned, event("blood")] {
        refusal(
            &uptake(variable, CONCENTRATION, "", &rest),
            "X",
            "compartment",
        );
    }
}

#[test]
fn refuses_species_read_by_rate_of() {
    let rule = format!(
        r#"<listOfRules><assignmentRule variable="a"><math $>{}</math></assignmentRule></listOfRules>"#,
        csymbol("rateOf", &[&ci("X")])
    );
    refusal(
        &uptake(
            BLOOD,
            CONCENTRATION,
            r#"<parameter id="a" constant="false"/>"#,
            &rule,
        ),
        "X",
        "rateOf",
    );
}

#[test]
fn refuses_species_read_by_delay() {
    let rule = format!(
        r#"<listOfRules><assignmentRule variable="a"><math $>{}</math></assignmentRule></listOfRules>"#,
        csymbol("delay", &[&ci("X"), "<cn>1</cn>"])
    );
    refusal(
        &uptake(
            BLOOD,
            CONCENTRATION,
            r#"<parameter id="a" constant="false"/>"#,
            &rule,
        ),
        "X",
        "delay",
    );
}

#[test]
fn refuses_species_driven_twice() {
    let m = model(&uptake(BLOOD, CONCENTRATION, "", ""));
    match drive(m, &["X", "X"]) {
        Err(CompileError::Unsupported { feature, element }) => {
            assert_eq!(element, "X");
            assert!(feature.contains("twice"), "{feature}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn refuses_taken_ids() {
    refusal(
        &uptake(
            BLOOD,
            CONCENTRATION,
            r#"<parameter id="X__slope" value="1" constant="true"/>"#,
            "",
        ),
        "X",
        "X__slope",
    );
    refusal(
        &uptake(
            BLOOD,
            CONCENTRATION,
            r#"<parameter id="driven__t_ref" value="1" constant="true"/>"#,
            "",
        ),
        "X",
        "driven__t_ref",
    );
    // a reaction id
    refusal(
        &uptake(BLOOD, CONCENTRATION, "", "").replace(r#"id="R""#, r#"id="X__exchange""#),
        "X",
        "X__exchange",
    );
}

/// `A -> C -> B` with the rates `k1 * A` and `k2 * C`, `A` and `B` in `blood`, `C` in `cell`,
/// in this order of the species, so `A` is before the kept state `C` and `B` after it; `model`
/// the attributes of the model element.
fn chain(model: &str) -> String {
    let species = |id: &str, compartment: &str, value: &str| {
        format!(
            r#"<species id="{id}" compartment="{compartment}" initialConcentration="{value}" hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/>"#
        )
    };
    let reaction = |id: &str, from: &str, to: &str, k: &str| {
        format!(
            r#"<reaction id="{id}" reversible="false"><listOfReactants><speciesReference species="{from}" stoichiometry="1" constant="true"/></listOfReactants><listOfProducts><speciesReference species="{to}" stoichiometry="1" constant="true"/></listOfProducts><kineticLaw><math $>{}</math></kineticLaw></reaction>"#,
            apply("times", &[&ci(k), &ci(from)])
        )
    };
    format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m" {model}><listOfCompartments>{BLOOD}<compartment id="cell" size="1" constant="true"/></listOfCompartments><listOfSpecies>{}{}{}</listOfSpecies><listOfParameters><parameter id="k1" value="0.5" constant="true"/><parameter id="k2" value="0.25" constant="true"/><parameter id="cf" value="3" constant="true"/></listOfParameters><listOfReactions>{}{}</listOfReactions></model></sbml>"#,
        species("A", "blood", "3"),
        species("C", "cell", "0"),
        species("B", "blood", "1"),
        reaction("R1", "A", "C", "k1"),
        reaction("R2", "C", "B", "k2"),
    )
    .replace('$', MATH)
}

fn read(xml: &str) -> Model {
    build(&sbmlsim_sbml::read(xml).unwrap()).unwrap()
}

#[test]
fn two_driven_species_around_a_kept_state() {
    let before = read(&chain(""));
    assert_eq!(ids(&before.states), ["A", "C", "B"]);
    // given in the other order than the species: the new entities follow the given order
    let m = drive(before, &["B", "A"]).unwrap();

    assert_eq!(ids(&m.states), ["C", "B__exchange", "A__exchange"]);
    assert_eq!(
        ids(&m.parameters),
        [
            "blood",
            "cell",
            "k1",
            "k2",
            "cf",
            "B__value",
            "B__slope",
            "A__value",
            "A__slope",
            "driven__t_ref"
        ]
    );
    assert_eq!(
        m.driven,
        [
            DrivenSpecies {
                id: "B".into(),
                value: 5,
                slope: 6,
                exchange: 1,
                amount: false,
            },
            DrivenSpecies {
                id: "A".into(),
                value: 7,
                slope: 8,
                exchange: 2,
                amount: false,
            }
        ]
    );
    assert_eq!(m.t_ref, Some(9));
    assert_eq!(entity(&m, "A__value").default, 3.0);
    assert_eq!(entity(&m, "B__value").default, 1.0);
    // R1 reads the path of A, R2 the concentration of the kept C
    let r1 = ap(Op::Times, vec![sym(&m, "k1"), path(&m, "A")]);
    let r2 = ap(
        Op::Times,
        vec![
            sym(&m, "k2"),
            ap(Op::Divide, vec![sym(&m, "C"), sym(&m, "cell")]),
        ],
    );
    assert_eq!(m.selection("R1").unwrap().expr, r1);
    assert_eq!(m.selection("R2").unwrap().expr, r2);
    // the exchange rates: A loses R1, B gains R2
    assert_eq!(
        m.rate("A__exchange").unwrap(),
        &ap(Op::Minus, vec![ap(Op::Times, vec![Expr::Number(1.0), r1])])
    );
    assert_eq!(
        m.rate("B__exchange").unwrap(),
        &ap(Op::Times, vec![Expr::Number(1.0), r2])
    );
    for id in ["A__exchange", "B__exchange"] {
        assert_eq!(m.selection(id).unwrap().expr, sym(&m, id));
    }
}

#[test]
fn driven_species_with_the_conversion_factor_of_the_model() {
    let m = drive(read(&chain(r#"conversionFactor="cf""#)), &["A"]).unwrap();
    assert_eq!(ids(&m.states), ["C", "B", "A__exchange"]);
    let r1 = ap(Op::Times, vec![sym(&m, "k1"), path(&m, "A")]);
    // the amount rate of A times the conversion factor of the model
    assert_eq!(
        m.rate("A__exchange").unwrap(),
        &ap(
            Op::Times,
            vec![
                sym(&m, "cf"),
                ap(Op::Minus, vec![ap(Op::Times, vec![Expr::Number(1.0), r1])])
            ]
        )
    );
}

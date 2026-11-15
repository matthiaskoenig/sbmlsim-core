//! The root functions of the conditions of `piecewise` (`Model::discontinuity_roots`).

use sbmlsim_ir::{Expr, Model, Op, Symbol, build};

const MATH: &str = r#"xmlns="http://www.w3.org/1998/Math/MathML""#;
const TIME: &str =
    r#"<csymbol encoding="text" definitionURL="http://www.sbml.org/sbml/symbols/time">t</csymbol>"#;

/// A model of the state `x` (a parameter of a rate rule), the amount species `S` in `C`, the
/// constant `k` and the variable `r`; `$` in `rest` is the MathML namespace declaration.
fn model(rest: &str) -> Model {
    let xml = format!(
        r#"<sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2"><model id="m">
<listOfCompartments><compartment id="C" size="1" constant="true"/></listOfCompartments>
<listOfSpecies><species id="S" compartment="C" initialAmount="1" hasOnlySubstanceUnits="true" boundaryCondition="false" constant="false"/></listOfSpecies>
<listOfParameters><parameter id="x" value="0" constant="false"/><parameter id="k" value="3" constant="true"/><parameter id="r" value="0" constant="false"/></listOfParameters>
{}</model></sbml>"#,
        rest.replace('$', MATH)
    );
    build(&sbmlsim_sbml::read(&xml).unwrap()).unwrap()
}

/// The rate rule of `x` and the assignment rule of `r` (a constant without `r`).
fn rules(x: &str, r: Option<&str>) -> String {
    let r = r.map_or(String::new(), |m| {
        format!(r#"<assignmentRule variable="r"><math $>{m}</math></assignmentRule>"#)
    });
    format!(r#"<listOfRules><rateRule variable="x"><math $>{x}</math></rateRule>{r}</listOfRules>"#)
}

fn ap(op: &str, args: &[&str]) -> String {
    format!("<apply><{op}/>{}</apply>", args.concat())
}

fn cn(x: &str) -> String {
    format!("<cn>{x}</cn>")
}

/// `piecewise(1, c, 0)`.
fn step(c: &str) -> String {
    format!("<piecewise><piece><cn>1</cn>{c}</piece><otherwise><cn>0</cn></otherwise></piecewise>")
}

fn sym(m: &Model, id: &str) -> Expr {
    Expr::Symbol(m.symbol(id).unwrap())
}

fn e(op: Op, args: Vec<Expr>) -> Expr {
    Expr::apply(op, args)
}

fn n(x: f64) -> Expr {
    Expr::Number(x)
}

fn time() -> Expr {
    Expr::Symbol(Symbol::Time)
}

#[test]
fn a_condition_of_the_time_is_its_difference() {
    let m = model(&rules(&step(&ap("gt", &[TIME, &cn("1.5")])), None));
    assert_eq!(
        m.discontinuity_roots(),
        [e(Op::Minus, vec![time(), n(1.5)])]
    );
}

#[test]
fn a_condition_of_a_species_and_of_a_rule_of_a_state() {
    // S < 2 in the rate rule, x >= k through the rule r = x
    let m = model(&rules(
        &ap(
            "plus",
            &[&step(&ap("lt", &["<ci>S</ci>", &cn("2")])), "<ci>r</ci>"],
        ),
        Some(&step(&ap("geq", &["<ci>x</ci>", "<ci>k</ci>"]))),
    ));
    assert_eq!(
        m.discontinuity_roots(),
        [
            // the rule first, then the rate
            e(Op::Minus, vec![sym(&m, "x"), sym(&m, "k")]),
            e(Op::Minus, vec![n(2.0), sym(&m, "S")]),
        ]
    );
}

#[test]
fn a_constant_condition_gives_no_root() {
    // a parameter, a literal, and a parameter of a rule of constants
    let m = model(&rules(
        &ap(
            "plus",
            &[
                &step(&ap("gt", &["<ci>k</ci>", &cn("2")])),
                &step(&ap("gt", &[&cn("1"), &cn("2")])),
                &step(&ap("gt", &["<ci>r</ci>", &cn("2")])),
            ],
        ),
        Some(&ap("times", &[&cn("2"), "<ci>k</ci>"])),
    ));
    assert_eq!(m.discontinuity_roots(), []);
    // a rule of the time is no constant
    let m = model(&rules(
        &step(&ap("gt", &["<ci>r</ci>", &cn("2")])),
        Some(&ap("times", &[&cn("2"), TIME])),
    ));
    assert_eq!(
        m.discontinuity_roots(),
        [e(Op::Minus, vec![sym(&m, "r"), n(2.0)])]
    );
}

#[test]
fn a_boolean_combination_is_one_root() {
    let c = ap(
        "or",
        &[
            &ap(
                "and",
                &[
                    &ap("gt", &[TIME, &cn("1")]),
                    &ap("lt", &["<ci>x</ci>", &cn("2")]),
                ],
            ),
            &ap("not", &[&ap("leq", &["<ci>S</ci>", &cn("3")])]),
            &ap("neq", &["<ci>x</ci>", &cn("4")]),
        ],
    );
    let m = model(&rules(&step(&c), None));
    let (x, s) = (sym(&m, "x"), sym(&m, "S"));
    assert_eq!(
        m.discontinuity_roots(),
        // the max of three is two (folded)
        [e(
            Op::Max,
            vec![
                e(
                    Op::Max,
                    vec![
                        e(
                            Op::Min,
                            vec![
                                e(Op::Minus, vec![time(), n(1.0)]),
                                e(Op::Minus, vec![n(2.0), x.clone()]),
                            ]
                        ),
                        e(Op::Minus, vec![e(Op::Minus, vec![n(3.0), s])]),
                    ]
                ),
                e(Op::Abs, vec![e(Op::Minus, vec![x, n(4.0)])]),
            ]
        )]
    );
}

#[test]
fn every_branch_but_the_otherwise_is_a_root() {
    // piecewise(1, time < 1, 2, time < 2, 3): the second branch is time < 2 && !(time < 1)
    let pw = format!(
        "<piecewise><piece><cn>1</cn>{}</piece><piece><cn>2</cn>{}</piece><otherwise><cn>3</cn></otherwise></piecewise>",
        ap("lt", &[TIME, &cn("1")]),
        ap("lt", &[TIME, &cn("2")])
    );
    let m = model(&rules(&pw, None));
    let first = e(Op::Minus, vec![n(1.0), time()]);
    assert_eq!(
        m.discontinuity_roots(),
        [
            first.clone(),
            e(
                Op::Min,
                vec![
                    e(Op::Minus, vec![n(2.0), time()]),
                    e(Op::Minus, vec![first])
                ]
            ),
        ]
    );
}

#[test]
fn the_math_of_rules_reactions_and_triggers_once_each() {
    let c = ap("gt", &[TIME, &cn("1")]);
    let d = ap("gt", &[TIME, &cn("2")]);
    let f = ap("gt", &[TIME, &cn("3")]);
    let g = ap("gt", &[TIME, &cn("4")]);
    // the same piecewise in the rate rule and the kinetic law is one root; the initial
    // assignment and the event assignment give none, the trigger one
    let rest = format!(
        r#"{}<listOfInitialAssignments><initialAssignment symbol="k"><math $>{}</math></initialAssignment></listOfInitialAssignments>
<listOfReactions><reaction id="J" reversible="false"><listOfReactants><speciesReference species="S" stoichiometry="1" constant="true"/></listOfReactants><kineticLaw><math $>{}</math></kineticLaw></reaction></listOfReactions>
<listOfEvents><event useValuesFromTriggerTime="true"><trigger initialValue="false" persistent="true"><math $>{}</math></trigger><listOfEventAssignments><eventAssignment variable="x"><math $>{}</math></eventAssignment></listOfEventAssignments></event></listOfEvents>"#,
        rules(&step(&c), None),
        step(&d),
        step(&c),
        ap("gt", &[&step(&f), &cn("0.5")]),
        step(&g)
    );
    let m = model(&rest);
    assert_eq!(
        m.discontinuity_roots(),
        [
            e(Op::Minus, vec![time(), n(1.0)]),
            e(Op::Minus, vec![time(), n(3.0)]),
        ]
    );
}

#[test]
fn a_piecewise_of_booleans_and_a_number_read_as_a_boolean() {
    // piecewise(1, piecewise(true, time < 1, x > 2), 0) and piecewise(1, x, 0)
    let inner = format!(
        "<piecewise><piece><true/>{}</piece><otherwise>{}</otherwise></piecewise>",
        ap("lt", &[TIME, &cn("1")]),
        ap("gt", &["<ci>x</ci>", &cn("2")])
    );
    let m = model(&rules(
        &ap("plus", &[&step(&inner), &step("<ci>x</ci>")]),
        None,
    ));
    let x = sym(&m, "x");
    let c = e(Op::Minus, vec![n(1.0), time()]);
    assert_eq!(
        m.discontinuity_roots(),
        [
            e(
                Op::Max,
                vec![
                    e(Op::Min, vec![n(1.0), c.clone()]),
                    e(
                        Op::Min,
                        vec![
                            e(Op::Minus, vec![x.clone(), n(2.0)]),
                            e(Op::Minus, vec![c.clone()])
                        ]
                    ),
                ]
            ),
            // the inner piecewise
            c,
            e(Op::Abs, vec![x]),
        ]
    );
}

//! The infix formulas of Level 1, read as the tree a `<math>` of a later level gives.
//!
//! Every reading below is the one of libsbml's parser of a Level 1 formula, which is the
//! definition of the syntax in practice.

use sbmlsim_sbml::formula::parse;
use sbmlsim_sbml::math::{Node, Op};

/// `Apply` of `op` over `args`.
fn apply(op: Op, args: Vec<Node>) -> Node {
    Node::Apply { op, args }
}

fn symbol(name: &str) -> Node {
    Node::Symbol(name.to_string())
}

#[test]
fn reads_a_number_a_symbol_and_a_sum() {
    assert_eq!(parse("2").unwrap(), Node::Number(2.0));
    assert_eq!(parse("2.5").unwrap(), Node::Number(2.5));
    assert_eq!(parse("2e-3").unwrap(), Node::Number(0.002));
    assert_eq!(parse(" 1.5E+2 ").unwrap(), Node::Number(150.0));
    assert_eq!(parse("x_1").unwrap(), symbol("x_1"));
    assert_eq!(
        parse("a + b").unwrap(),
        apply(Op::Plus, vec![symbol("a"), symbol("b")])
    );
}

#[test]
fn the_operators_of_a_sum_and_a_product_associate_to_the_left() {
    assert_eq!(
        parse("x/y/z").unwrap(),
        apply(
            Op::Divide,
            vec![
                apply(Op::Divide, vec![symbol("x"), symbol("y")]),
                symbol("z"),
            ],
        )
    );
    assert_eq!(
        parse("a - b + c").unwrap(),
        apply(
            Op::Plus,
            vec![
                apply(Op::Minus, vec![symbol("a"), symbol("b")]),
                symbol("c"),
            ],
        )
    );
}

#[test]
fn a_product_binds_tighter_than_a_sum_and_a_power_tighter_than_a_product() {
    assert_eq!(
        parse("2*3^2").unwrap(),
        apply(
            Op::Times,
            vec![
                Node::Number(2.0),
                apply(Op::Power, vec![Node::Number(3.0), Node::Number(2.0)]),
            ],
        )
    );
    assert_eq!(
        parse("1 + 2 * 3").unwrap(),
        apply(
            Op::Plus,
            vec![
                Node::Number(1.0),
                apply(Op::Times, vec![Node::Number(2.0), Node::Number(3.0)]),
            ],
        )
    );
}

#[test]
fn a_power_associates_to_the_left_as_libsbml_reads_it() {
    assert_eq!(
        parse("2^3^2").unwrap(),
        apply(
            Op::Power,
            vec![
                apply(Op::Power, vec![Node::Number(2.0), Node::Number(3.0)]),
                Node::Number(2.0),
            ],
        )
    );
}

#[test]
fn a_sign_binds_tighter_than_a_power() {
    // `-x^2` is `(-x)^2`
    assert_eq!(
        parse("-x^2").unwrap(),
        apply(
            Op::Power,
            vec![apply(Op::Minus, vec![symbol("x")]), Node::Number(2.0),],
        )
    );
    // the sign of a number is the sign of the number itself
    assert_eq!(
        parse("-2^2").unwrap(),
        apply(Op::Power, vec![Node::Number(-2.0), Node::Number(2.0)])
    );
    assert_eq!(
        parse("a - -b").unwrap(),
        apply(
            Op::Minus,
            vec![symbol("a"), apply(Op::Minus, vec![symbol("b")])],
        )
    );
}

#[test]
fn the_logarithms_carry_their_base() {
    assert_eq!(parse("log(x)").unwrap(), apply(Op::Ln, vec![symbol("x")]));
    assert_eq!(
        parse("log(2, 8)").unwrap(),
        apply(Op::Log, vec![Node::Number(2.0), Node::Number(8.0)])
    );
    assert_eq!(
        parse("log10(x)").unwrap(),
        apply(Op::Log, vec![Node::Number(10.0), symbol("x")])
    );
    assert_eq!(parse("ln(x)").unwrap(), apply(Op::Ln, vec![symbol("x")]));
}

#[test]
fn the_roots_carry_their_degree() {
    assert_eq!(
        parse("sqrt(x)").unwrap(),
        apply(Op::Root, vec![Node::Number(2.0), symbol("x")])
    );
    assert_eq!(
        parse("root(3, x)").unwrap(),
        apply(Op::Root, vec![Node::Number(3.0), symbol("x")])
    );
}

#[test]
fn the_functions_of_the_grammar() {
    assert_eq!(
        parse("pow(2, 3)").unwrap(),
        apply(Op::Power, vec![Node::Number(2.0), Node::Number(3.0)])
    );
    assert_eq!(
        parse("ceil(1.5)").unwrap(),
        apply(Op::Ceiling, vec![Node::Number(1.5)])
    );
    assert_eq!(parse("abs(x)").unwrap(), apply(Op::Abs, vec![symbol("x")]));
    assert_eq!(
        parse("asin(x)").unwrap(),
        apply(Op::Arcsin, vec![symbol("x")])
    );
    assert_eq!(
        parse("and(a, b)").unwrap(),
        apply(Op::And, vec![symbol("a"), symbol("b")])
    );
    assert_eq!(
        parse("leq(a, b, c)").unwrap(),
        apply(Op::Leq, vec![symbol("a"), symbol("b"), symbol("c")])
    );
    assert_eq!(
        parse("delay(x, 1)").unwrap(),
        apply(Op::Delay, vec![symbol("x"), Node::Number(1.0)])
    );
}

#[test]
fn a_call_of_a_name_which_is_no_function_of_the_grammar_is_a_function_of_the_model() {
    assert_eq!(
        parse("f(x, y)").unwrap(),
        apply(
            Op::Function("f".to_string()),
            vec![symbol("x"), symbol("y")],
        )
    );
}

#[test]
fn the_constants_of_the_grammar() {
    assert_eq!(parse("true").unwrap(), Node::True);
    assert_eq!(parse("false").unwrap(), Node::False);
    assert_eq!(parse("pi").unwrap(), Node::Pi);
    assert_eq!(parse("exponentiale").unwrap(), Node::ExponentialE);
    assert_eq!(parse("INF").unwrap(), Node::Infinity);
    assert_eq!(parse("NaN").unwrap(), Node::NotANumber);
    assert_eq!(
        parse("-INF").unwrap(),
        apply(Op::Minus, vec![Node::Infinity])
    );
    // Level 1 reserves neither `time` nor `avogadro`, they are symbols of the model
    assert_eq!(parse("time").unwrap(), symbol("time"));
    assert_eq!(parse("avogadro").unwrap(), symbol("avogadro"));
}

#[test]
fn a_bracket_groups_what_it_holds() {
    assert_eq!(
        parse("(a + b) * c").unwrap(),
        apply(
            Op::Times,
            vec![apply(Op::Plus, vec![symbol("a"), symbol("b")]), symbol("c"),],
        )
    );
}

#[test]
fn a_formula_which_is_no_formula_is_refused() {
    for text in ["", "a +", "(a", "a)", "f(a", "a $ b", "a b"] {
        assert!(parse(text).is_err(), "{text:?} is no formula");
    }
}

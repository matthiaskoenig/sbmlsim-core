//! The Level 3 formula of a syntax tree.
//!
//! This is a port of `SBML_formulaToL3String` of libsbml
//! (`src/sbml/math/L3FormulaFormatter.cpp`) with the default
//! `L3ParserSettings`, which is what the python implementation calls
//! (`backend/sbml4humans/mathml.py`): units are written (`L3P_PARSE_UNITS` is
//! true) and a nested unary minus is not collapsed (`L3P_EXPAND_UNARY_MINUS` is
//! false).
//!
//! The shape of the port follows the shape of libsbml so that the two can be
//! read against each other: [`visit`] is `L3FormulaFormatter_visit`,
//! [`is_grouped`] is `L3FormulaFormatter_isGrouped`, and so on down to
//! [`significant_15`], which is the `"%.15g"` of `LIBSBML_FLOAT_FORMAT`.

use super::MathError;
use crate::{Ast, AstKind, AstNumber, MAX_AST_DEPTH};

/// The Level 3 formula of `ast`, as libsbml writes it.
///
/// The operators of a formula print with a space on each side, apart from `^`,
/// which prints without one; the arguments of a function are separated by `, `;
/// a child is parenthesised by the precedence of libsbml, which wraps every
/// ambiguous operand of a logical or relational operator whatever its
/// precedence; and a number prints its `sbml:units` behind its value.
///
/// # The stack
///
/// The walk keeps its own stack on the heap, so a tree costs it no call frame
/// per level: what is left to write is a list of pieces, and a node is one
/// piece which is replaced by the pieces of what it writes when its turn
/// comes. Nothing of the formula is decided by what another piece wrote, so
/// the text is the one the recursive walk this is the port of writes.
///
/// # Errors
///
/// [`MathError::TooDeep`] for a tree nested deeper than [`MAX_AST_DEPTH`] and
/// [`MathError::UnknownKind`] for a tree which holds a node of a kind the
/// libsbml shim does not map, whose formula would not be the one of the
/// document.
pub fn formula(ast: &Ast) -> Result<String, MathError> {
    let mut out = String::new();
    let mut rest = vec![Piece::Visit {
        parent: None,
        node: ast,
        depth: 1,
    }];
    while let Some(piece) = rest.pop() {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Token(node) => format(node, &mut out),
            Piece::Visit {
                parent,
                node,
                depth,
            } => visit(parent, node, depth, &mut out, &mut rest)?,
        }
    }
    Ok(out)
}

/* ------------------------------------------------------------------ the walk */

/// One piece of what is left of a formula.
///
/// The pieces stand on the stack of [`formula`] with the next one last, so a
/// writer puts the pieces of what it writes there in reverse.
enum Piece<'a> {
    /// A node, the child of a parent, which stands this deep in the tree.
    Visit {
        /// The parent of the node, [`None`] for the root of the math.
        parent: Option<&'a Ast>,
        /// The node itself.
        node: &'a Ast,
        /// How many levels below the root it stands.
        depth: usize,
    },
    /// Literal text.
    Text(&'static str),
    /// The token of a node, which stands before or between its children.
    Token(&'a Ast),
}

/// Writes `node`, the child of `parent`, at `depth` levels below the root.
///
/// What stands after a child of the node goes on `rest`, which is what makes
/// the walk cost no call frame per level.
fn visit<'a>(
    parent: Option<&'a Ast>,
    node: &'a Ast,
    depth: usize,
    out: &mut String,
    rest: &mut Vec<Piece<'a>>,
) -> Result<(), MathError> {
    if depth > MAX_AST_DEPTH {
        return Err(MathError::TooDeep);
    }
    if node.kind == AstKind::Other {
        return Err(MathError::UnknownKind(node.kind));
    }
    if is_log10(node) {
        visit_one_of_two(node, "log10(", depth, out, rest)
    } else if is_sqrt(node) {
        visit_one_of_two(node, "sqrt(", depth, out, rest)
    } else if is_translated_modulo(node) {
        visit_modulo(parent, node, depth, out, rest)
    } else if is_function(node) {
        visit_function(node, depth, out, rest);
        Ok(())
    } else if is_unary_minus(node) {
        visit_unary(parent, node, '-', depth, out, rest)
    } else if is_unary_not(node) {
        visit_unary(parent, node, '!', depth, out, rest)
    } else {
        visit_other(parent, node, depth, out, rest);
        Ok(())
    }
}

/// Writes `node` as a call of its name, its children separated by `, `.
fn visit_function<'a>(node: &'a Ast, depth: usize, out: &mut String, rest: &mut Vec<Piece<'a>>) {
    format(node, out);
    out.push('(');
    rest.push(Piece::Text(")"));
    for (n, child) in node.children.iter().enumerate().rev() {
        rest.push(Piece::Visit {
            parent: Some(node),
            node: child,
            depth: depth + 1,
        });
        if n > 0 {
            rest.push(Piece::Text(", "));
        }
    }
}

/// Writes `node` as `prefix` and its second child, the form of `log10` and of `sqrt`.
///
/// libsbml writes a logarithm to the base ten and a root of degree two by the
/// name of their one argument form, dropping the base or the degree, which
/// their syntax tree holds as the first child.
fn visit_one_of_two<'a>(
    node: &'a Ast,
    prefix: &str,
    depth: usize,
    out: &mut String,
    rest: &mut Vec<Piece<'a>>,
) -> Result<(), MathError> {
    let argument = node.children.get(1).ok_or(MathError::MissingChild)?;
    out.push_str(prefix);
    rest.push(Piece::Text(")"));
    rest.push(Piece::Visit {
        parent: Some(node),
        node: argument,
        depth: depth + 1,
    });
    Ok(())
}

/// Writes `node`, a unary minus or a unary not, as `sign` and its one child.
///
/// A unary operator is not of the highest precedence, since a power binds
/// tighter, so its own parentheses are the ones of [`is_grouped`].
fn visit_unary<'a>(
    parent: Option<&Ast>,
    node: &'a Ast,
    sign: char,
    depth: usize,
    out: &mut String,
    rest: &mut Vec<Piece<'a>>,
) -> Result<(), MathError> {
    let operand = node.children.first().ok_or(MathError::MissingChild)?;
    let group = is_grouped(parent, node);
    if group {
        out.push('(');
        rest.push(Piece::Text(")"));
    }
    out.push(sign);
    rest.push(Piece::Visit {
        parent: Some(node),
        node: operand,
        depth: depth + 1,
    });
    Ok(())
}

/// Writes `node`, a piecewise which spells out a modulo, as `x % y`.
fn visit_modulo<'a>(
    parent: Option<&Ast>,
    node: &'a Ast,
    depth: usize,
    out: &mut String,
    rest: &mut Vec<Piece<'a>>,
) -> Result<(), MathError> {
    // the shape is piecewise(x - y * ceil(x / y), xor(x < 0, y < 0), x - y *
    // floor(x / y)), which `is_translated_modulo` established, so x is the first
    // child of the first child and y the first child of its second child
    let difference = node.children.first().ok_or(MathError::MissingChild)?;
    let x = difference.children.first().ok_or(MathError::MissingChild)?;
    let product = difference.children.get(1).ok_or(MathError::MissingChild)?;
    let y = product.children.first().ok_or(MathError::MissingChild)?;
    let group = is_grouped(parent, node);
    if group {
        out.push('(');
        rest.push(Piece::Text(")"));
    }
    rest.push(Piece::Visit {
        parent: Some(node),
        node: y,
        depth: depth + 1,
    });
    rest.push(Piece::Text(" % "));
    rest.push(Piece::Visit {
        parent: Some(node),
        node: x,
        depth: depth + 1,
    });
    Ok(())
}

/// Writes `node` as an infix expression, its token between its children.
///
/// A node of no child is its token alone, a number, a symbol or a constant. A
/// node of one child which is neither a unary operator nor a function cannot
/// come from a document; libsbml writes it as a call of its token, which this
/// does too.
fn visit_other<'a>(
    parent: Option<&Ast>,
    node: &'a Ast,
    depth: usize,
    out: &mut String,
    rest: &mut Vec<Piece<'a>>,
) {
    let group = is_grouped(parent, node);
    if group {
        out.push('(');
        rest.push(Piece::Text(")"));
    }
    match node.children.split_first() {
        None => format(node, out),
        Some((only, [])) => {
            format(node, out);
            out.push('(');
            rest.push(Piece::Text(")"));
            rest.push(Piece::Visit {
                parent: Some(node),
                node: only,
                depth: depth + 1,
            });
        }
        Some((first, children)) => {
            for child in children.iter().rev() {
                rest.push(Piece::Visit {
                    parent: Some(node),
                    node: child,
                    depth: depth + 1,
                });
                rest.push(Piece::Token(node));
            }
            rest.push(Piece::Visit {
                parent: Some(node),
                node: first,
                depth: depth + 1,
            });
        }
    }
}

/* ------------------------------------------------------------- the grouping */

/// Whether `child`, a child of `parent`, is written within parentheses.
///
/// A child is wrapped when the precedence of its parent binds tighter than its
/// own, and when the two bind equally and the child is not the left operand of
/// an associative parent. Every ambiguous operand of a logical or relational
/// operator is wrapped whatever its precedence, since few readers know the
/// precedence of those, and only the right operand of one is spared when it is
/// a unary minus or a unary not, which no parenthesis disambiguates.
fn is_grouped(parent: Option<&Ast>, child: &Ast) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    let mut child = child;
    if is_unary_minus(parent) || is_unary_not(parent) {
        // the only child of a unary operator is the child this was called for
        if let Some(operand) = parent.children.first() {
            child = operand;
        }
        if is_unary_minus(parent) && is_unary_not(child) {
            // -(!a), and !(-a) below, are always wrapped
            return true;
        }
        if is_unary_not(parent) && is_unary_minus(child) {
            return true;
        }
    }
    if is_logical(parent) || is_relational(parent) || is_translated_modulo(parent) {
        if has_unambiguous_grammar(None, child) {
            return false;
        }
        let is_right = right_child(parent).is_some_and(|right| std::ptr::eq(right, child));
        return !(is_right && (is_unary_minus(child) || is_unary_not(child)));
    }
    if has_unambiguous_grammar(Some(parent), child) {
        return false;
    }
    if (is_logical(child) || is_relational(child) || is_translated_modulo(child))
        && !is_unary_minus(child)
        && !is_unary_not(child)
    {
        return true;
    }
    let parent_precedence = l3_precedence(parent);
    let child_precedence = l3_precedence(child);
    let is_left = parent
        .children
        .first()
        .is_some_and(|left| std::ptr::eq(left, child));
    if parent_precedence < child_precedence {
        false
    } else if parent_precedence == child_precedence {
        // the left operand of a parent of the same kind needs no parenthesis,
        // and neither does the left operand of a difference or a quotient,
        // which associate to the left. libsbml also spares a logical or
        // relational parent of the kind of its child here, which the branch
        // above has answered for already.
        !is_left
            || !(parent.kind == child.kind
                || parent.kind == AstKind::Divide
                || parent.kind == AstKind::Minus)
    } else {
        // a unary operand of a power is ambiguous on its left only: (-x)^y, but
        // x^-y
        !(parent_precedence == 7 && child_precedence == 6) || is_left
    }
}

/// Whether the grammar of `node` wraps `child` of its own, so that no parenthesis is needed.
///
/// A function delimits its arguments with commas, and a node of the highest
/// precedence, which is what every function and every other self delimiting
/// node has, is unambiguous wherever it stands. `node` is [`None`] where
/// libsbml asks of a child alone, which is unambiguous only by its own
/// precedence.
fn has_unambiguous_grammar(node: Option<&Ast>, child: &Ast) -> bool {
    node.is_some_and(is_function) || l3_precedence(child) == 8
}

/// The precedence of `node` as the Level 3 parser of libsbml documents it.
///
/// A node whose number of arguments is wrong is written as a call and so has
/// the highest precedence, 8, which is also the precedence of every function
/// and of every leaf. A node of a package has -1, the precedence the base AST
/// plugin of libsbml answers and neither the l3v2extendedmath plugin nor the
/// distrib one overrides; that -1 never shows, since a node of a package is
/// always written as a call, which asks for no parenthesis of its own and
/// wraps its arguments with its own, but it is what libsbml answers.
fn l3_precedence(node: &Ast) -> i32 {
    let children = node.children.len();
    if !has_correct_number_arguments(node) {
        return 8;
    }
    if is_translated_modulo(node) {
        return 5;
    }
    match node.kind {
        AstKind::Power | AstKind::FunctionPower => 7,
        AstKind::LogicalNot => 6,
        AstKind::Divide | AstKind::Times if children < 2 => 8,
        AstKind::Divide | AstKind::Times => 5,
        AstKind::Minus if children == 1 => 6,
        AstKind::Minus | AstKind::Plus if children < 2 => 8,
        AstKind::Minus | AstKind::Plus => 4,
        AstKind::LogicalAnd | AstKind::LogicalOr if children >= 2 => 2,
        _ if is_relational(node) && children >= 2 => 3,
        kind if is_of_a_package(kind) => -1,
        // every function, every leaf, a lambda, an exclusive or, and a sum, a
        // product, a conjunction, a disjunction or a relation of one operand,
        // which is written as a call of its name
        _ => 8,
    }
}

/* -------------------------------------------------------------- the tokens */

/// Writes the token of `node`: its name, its value or its operator.
fn format(node: &Ast, out: &mut String) {
    if is_function(node) {
        format_function(node, out);
    } else if is_operator(node) || node.kind == AstKind::FunctionPower {
        format_operator(node, out);
    } else if is_logical(node) || is_relational(node) {
        out.push_str(logical_relational_token(node.kind));
    } else if node.kind == AstKind::Rational {
        format_rational(node, out);
    } else if is_a_number(node) {
        format_real(node, out);
    } else if node.kind == AstKind::NameAvogadro {
        out.push_str("avogadro");
    } else if node.kind == AstKind::NameTime {
        out.push_str("time");
    } else if node.kind == AstKind::Name {
        // the one node of a formula whose name libsbml reads of the document
        out.push_str(node.name.as_deref().unwrap_or_default());
    } else {
        out.push_str(libsbml_name(node.kind));
    }
}

/// Writes the name `node` is called by.
fn format_function(node: &Ast, out: &mut String) {
    let name = match node.kind {
        // the names the Level 3 formatter writes of its own, which differ from
        // the ones libsbml gives the nodes
        AstKind::Plus => "plus",
        AstKind::Times => "times",
        AstKind::Minus => "minus",
        AstKind::Divide => "divide",
        AstKind::Power => "pow",
        AstKind::FunctionLn => "ln",
        AstKind::FunctionDelay => "delay",
        // the four names and the one power the Level 1 formatter it falls back
        // to writes of its own
        AstKind::FunctionArccos => "acos",
        AstKind::FunctionArcsin => "asin",
        AstKind::FunctionArctan => "atan",
        AstKind::FunctionCeiling => "ceil",
        AstKind::FunctionPower => "pow",
        // a call of a function of the model and a csymbol of a definition URL
        // libsbml does not know are named by the document
        AstKind::Function | AstKind::CsymbolFunction => node.name.as_deref().unwrap_or_default(),
        kind => libsbml_name(kind),
    };
    out.push_str(name);
}

/// Writes the operator of `node` with a space on each side, a power without one.
fn format_operator(node: &Ast, out: &mut String) {
    match node.kind {
        AstKind::Power | AstKind::FunctionPower => out.push('^'),
        AstKind::Plus => out.push_str(" + "),
        AstKind::Minus => out.push_str(" - "),
        AstKind::Times => out.push_str(" * "),
        AstKind::Divide => out.push_str(" / "),
        // no other kind is an operator of libsbml
        _ => {}
    }
}

/// Writes `node`, a rational, as `(numerator/denominator)` and its units.
fn format_rational(node: &Ast, out: &mut String) {
    let (numerator, denominator) = match node.number {
        Some(AstNumber::Rational {
            numerator,
            denominator,
        }) => (numerator, denominator),
        // libsbml holds the numerator of a node which was never given one as 0
        // and its denominator as 1
        _ => (0, 1),
    };
    out.push('(');
    out.push_str(&decimal(numerator));
    out.push('/');
    out.push_str(&decimal(denominator));
    out.push(')');
    write_units(node, out);
}

/// Writes `node`, an integer or a real, as its value and its units.
fn format_real(node: &Ast, out: &mut String) {
    let value = value_of(node);
    let (mantissa, exponent) = match node.number {
        Some(AstNumber::RealE { mantissa, exponent }) => (mantissa, exponent),
        _ => (value, 0),
    };
    if value.is_nan() {
        out.push_str("NaN");
    } else if value.is_infinite() {
        out.push_str(if value < 0.0 { "-INF" } else { "INF" });
    } else if is_negative_zero(value) {
        out.push_str("-0");
    } else if node.kind == AstKind::RealE {
        write_exponent_form(mantissa, exponent, value, out);
    } else {
        out.push_str(&significant_15(value));
    }
    write_units(node, out);
}

/// Writes the mantissa and the exponent `node` was written with.
///
/// libsbml keeps the form of the document only while the mantissa reads as
/// itself: one of 14 digits or more, or one below a ten thousandth, is written
/// as the value of the node instead, which its own 15 significant digits would
/// otherwise lose or lengthen.
fn write_exponent_form(mantissa: f64, exponent: i64, value: f64, out: &mut String) {
    if mantissa >= 1e14 || mantissa <= -1e14 || (mantissa < 1e-4 && mantissa > -1e-4) {
        out.push_str(&significant_15(value));
        return;
    }
    out.push_str(&significant_15(mantissa));
    out.push('e');
    out.push_str(&decimal(exponent));
}

/// Writes the `sbml:units` of `node` behind its value, a space between.
fn write_units(node: &Ast, out: &mut String) {
    if let Some(units) = node.units.as_deref() {
        out.push(' ');
        out.push_str(units);
    }
}

/// The value of the number of `node`, 0 where it holds none.
///
/// A number in exponent form is its mantissa times the power of ten of its
/// exponent and a rational is its quotient, which is how libsbml answers the
/// real of either.
fn value_of(node: &Ast) -> f64 {
    match node.number {
        Some(AstNumber::Integer(integer)) => integer as f64,
        Some(AstNumber::Real(real)) => real,
        Some(AstNumber::RealE { mantissa, exponent }) => {
            crate::scientific_value(mantissa, exponent)
        }
        Some(AstNumber::Rational {
            numerator,
            denominator,
        }) => numerator as f64 / denominator as f64,
        None => 0.0,
    }
}

/// The token of a logical or relational operator which is written infix.
///
/// Only a conjunction, a disjunction and the six relations of two operands or
/// more are written infix; a negation is written by [`visit_unary`], an
/// exclusive or and an implication are written as calls, and libsbml writes
/// `!!` for a node which reaches this and is none of them.
fn logical_relational_token(kind: AstKind) -> &'static str {
    match kind {
        AstKind::LogicalAnd => " && ",
        AstKind::LogicalOr => " || ",
        AstKind::RelationalEq => " == ",
        AstKind::RelationalGeq => " >= ",
        AstKind::RelationalGt => " > ",
        AstKind::RelationalLeq => " <= ",
        AstKind::RelationalLt => " < ",
        AstKind::RelationalNeq => " != ",
        _ => " !! ",
    }
}

/* ----------------------------------------------------------- the predicates */

/// Whether `node` is written as a call rather than as an infix expression.
fn is_function(node: &Ast) -> bool {
    let children = node.children.len();
    match node.kind {
        AstKind::Plus
        | AstKind::Times
        | AstKind::LogicalAnd
        | AstKind::LogicalOr
        | AstKind::RelationalEq
        | AstKind::RelationalGeq
        | AstKind::RelationalGt
        | AstKind::RelationalLeq
        | AstKind::RelationalLt => children < 2,
        AstKind::Minus => children != 1 && children != 2,
        AstKind::Divide | AstKind::RelationalNeq | AstKind::Power | AstKind::FunctionPower => {
            children != 2
        }
        AstKind::LogicalNot => children != 1,
        AstKind::Integer
        | AstKind::Real
        | AstKind::RealE
        | AstKind::Rational
        | AstKind::Name
        | AstKind::NameAvogadro
        | AstKind::NameTime
        | AstKind::ConstantE
        | AstKind::ConstantFalse
        | AstKind::ConstantPi
        | AstKind::ConstantTrue => false,
        // every function of the core, every function of a package, a lambda, an
        // exclusive or, and the node of a math libsbml could not read, which it
        // writes as a call of no name
        _ => true,
    }
}

/// Whether `node` is one of the five operators of libsbml.
fn is_operator(node: &Ast) -> bool {
    matches!(
        node.kind,
        AstKind::Plus | AstKind::Minus | AstKind::Times | AstKind::Divide | AstKind::Power
    )
}

/// Whether `node` is a logical operator, the implication of a package among them.
fn is_logical(node: &Ast) -> bool {
    matches!(
        node.kind,
        AstKind::LogicalAnd
            | AstKind::LogicalNot
            | AstKind::LogicalOr
            | AstKind::LogicalXor
            | AstKind::LogicalImplies
    )
}

/// Whether `node` is one of the six relational operators.
fn is_relational(node: &Ast) -> bool {
    matches!(
        node.kind,
        AstKind::RelationalEq
            | AstKind::RelationalGeq
            | AstKind::RelationalGt
            | AstKind::RelationalLeq
            | AstKind::RelationalLt
            | AstKind::RelationalNeq
    )
}

/// Whether `node` holds a number libsbml answers a real for.
fn is_a_number(node: &Ast) -> bool {
    matches!(
        node.kind,
        AstKind::Integer | AstKind::Real | AstKind::RealE | AstKind::Rational
    )
}

/// Whether `node` is a negation of one operand.
fn is_unary_not(node: &Ast) -> bool {
    node.kind == AstKind::LogicalNot && node.children.len() == 1
}

/// Whether `node` is a negative sign rather than a difference.
fn is_unary_minus(node: &Ast) -> bool {
    node.kind == AstKind::Minus && node.children.len() == 1
}

/// Whether `node` is a logarithm to the base ten, which prints as `log10(x)`.
fn is_log10(node: &Ast) -> bool {
    node.kind == AstKind::FunctionLog && node.children.len() == 2 && is_integer(node, 10)
}

/// Whether `node` is a root of degree two, which prints as `sqrt(x)`.
fn is_sqrt(node: &Ast) -> bool {
    node.kind == AstKind::FunctionRoot && node.children.len() == 2 && is_integer(node, 2)
}

/// Whether the first child of `node` is the integer `value`.
fn is_integer(node: &Ast, value: i64) -> bool {
    node.children.first().is_some_and(|first| {
        first.kind == AstKind::Integer && first.number == Some(AstNumber::Integer(value))
    })
}

/// Whether `node` is a piecewise which spells out the modulo of two operands.
///
/// A modulo of the math of Level 3 Version 2 has no node of its own in the
/// syntax tree of libsbml: it is written as `piecewise(x - y * ceil(x / y),
/// xor(x < 0, y < 0), x - y * floor(x / y))`, which libsbml reads back and
/// writes as `x % y`.
///
/// Where libsbml compares the two operands by the formulas they write, this
/// compares them as trees, since writing a subtree once per comparison would
/// cost a formula whose work doubles with every nested modulo, which a crafted
/// document could drive to a denial of service.
///
/// The two agree wherever libsbml's own writer produced the pattern, since
/// every one of the four copies of `x` and the three of `y` is then a copy of
/// one subtree. They do not agree on every document: a tree comparison is the
/// stricter of the two, so a pattern whose copies are an `Integer(2)` in one
/// place and a `Real(2.0)` in another, or a csymbol whose text differs between
/// two copies, writes a `piecewise` here where libsbml writes `%`. A pattern
/// whose operand holds a NaN does too, for a reason which is this comparison
/// and not the document: the equality of [`Ast`] compares its `f64` as IEEE
/// 754 asks, and a NaN is equal to nothing, itself included.
fn is_translated_modulo(node: &Ast) -> bool {
    // the children of `node` when it is of `kind` and carries `count` of them
    fn children_of(node: Option<&Ast>, kind: AstKind, count: usize) -> Option<&[Ast]> {
        let node = node?;
        (node.kind == kind && node.children.len() == count).then_some(node.children.as_slice())
    }
    // whether `node` is `operand < 0`
    fn is_below_zero(node: Option<&Ast>, operand: &Ast) -> bool {
        children_of(node, AstKind::RelationalLt, 2).is_some_and(|below| {
            below.first() == Some(operand)
                && below.get(1).is_some_and(|zero| {
                    zero.kind == AstKind::Integer && zero.number == Some(AstNumber::Integer(0))
                })
        })
    }

    let Some(cases) = children_of(Some(node), AstKind::FunctionPiecewise, 3) else {
        return false;
    };
    // x - y * ceil(x / y)
    let Some(difference) = children_of(cases.first(), AstKind::Minus, 2) else {
        return false;
    };
    let Some(x) = difference.first() else {
        return false;
    };
    let Some(product) = children_of(difference.get(1), AstKind::Times, 2) else {
        return false;
    };
    let Some(y) = product.first() else {
        return false;
    };
    let rounds_up = children_of(product.get(1), AstKind::FunctionCeiling, 1)
        .and_then(|ceiling| children_of(ceiling.first(), AstKind::Divide, 2))
        .is_some_and(|quotient| quotient.first() == Some(x) && quotient.get(1) == Some(y));
    if !rounds_up {
        return false;
    }
    // xor(x < 0, y < 0)
    let signs_differ = children_of(cases.get(1), AstKind::LogicalXor, 2)
        .is_some_and(|signs| is_below_zero(signs.first(), x) && is_below_zero(signs.get(1), y));
    if !signs_differ {
        return false;
    }
    // x - y * floor(x / y)
    children_of(cases.get(2), AstKind::Minus, 2).is_some_and(|difference| {
        difference.first() == Some(x)
            && children_of(difference.get(1), AstKind::Times, 2).is_some_and(|product| {
                product.first() == Some(y)
                    && children_of(product.get(1), AstKind::FunctionFloor, 1)
                        .and_then(|floor| children_of(floor.first(), AstKind::Divide, 2))
                        .is_some_and(|quotient| {
                            quotient.first() == Some(x) && quotient.get(1) == Some(y)
                        })
            })
    })
}

/// Whether `node` carries the number of children its kind takes.
///
/// A node which does not is written as a call of its name, whatever its kind,
/// since no infix form of it would read back.
fn has_correct_number_arguments(node: &Ast) -> bool {
    let children = node.children.len();
    match node.kind {
        AstKind::Integer
        | AstKind::Real
        | AstKind::RealE
        | AstKind::Rational
        | AstKind::Name
        | AstKind::NameAvogadro
        | AstKind::NameTime
        | AstKind::ConstantE
        | AstKind::ConstantFalse
        | AstKind::ConstantPi
        | AstKind::ConstantTrue => children == 0,
        AstKind::FunctionAbs
        | AstKind::FunctionArccos
        | AstKind::FunctionArccosh
        | AstKind::FunctionArccot
        | AstKind::FunctionArccoth
        | AstKind::FunctionArccsc
        | AstKind::FunctionArccsch
        | AstKind::FunctionArcsec
        | AstKind::FunctionArcsech
        | AstKind::FunctionArcsin
        | AstKind::FunctionArcsinh
        | AstKind::FunctionArctan
        | AstKind::FunctionArctanh
        | AstKind::FunctionCeiling
        | AstKind::FunctionCos
        | AstKind::FunctionCosh
        | AstKind::FunctionCot
        | AstKind::FunctionCoth
        | AstKind::FunctionCsc
        | AstKind::FunctionCsch
        | AstKind::FunctionExp
        | AstKind::FunctionFactorial
        | AstKind::FunctionFloor
        | AstKind::FunctionLn
        | AstKind::FunctionSec
        | AstKind::FunctionSech
        | AstKind::FunctionSin
        | AstKind::FunctionSinh
        | AstKind::FunctionTan
        | AstKind::FunctionTanh
        | AstKind::LogicalNot
        | AstKind::FunctionRateOf
        | AstKind::DistribBernoulli => children == 1,
        AstKind::Divide
        | AstKind::Power
        | AstKind::RelationalNeq
        | AstKind::FunctionDelay
        | AstKind::FunctionPower
        | AstKind::FunctionLog
        | AstKind::FunctionQuotient
        | AstKind::FunctionRem
        | AstKind::LogicalImplies
        | AstKind::DistribUniform => children == 2,
        AstKind::RelationalEq
        | AstKind::RelationalGeq
        | AstKind::RelationalGt
        | AstKind::RelationalLeq
        | AstKind::RelationalLt => children >= 2,
        AstKind::FunctionRoot | AstKind::Minus => children == 1 || children == 2,
        AstKind::FunctionPiecewise | AstKind::Lambda => children >= 1,
        AstKind::DistribNormal
        | AstKind::DistribBinomial
        | AstKind::DistribGamma
        | AstKind::DistribLognormal => children == 2 || children == 4,
        // the first count of the distrib plugin is the one of the annotation the
        // package was an annotation of
        AstKind::DistribCauchy | AstKind::DistribLaplace => {
            children == 2 || children == 1 || children == 4
        }
        AstKind::DistribChisquare
        | AstKind::DistribExponential
        | AstKind::DistribPoisson
        | AstKind::DistribRayleigh => children == 1 || children == 3,
        // a sum, a product, a conjunction, a disjunction, an exclusive or, a
        // call of a function of the model, a maximum, a minimum and the
        // unmapped kind take any number of children
        _ => true,
    }
}

/// Whether `kind` is a node of a package of libsbml rather than of its core.
///
/// The AST plugins of l3v2extendedmath and of distrib answer every one of these
/// as a function and neither overrides the infix precedence of the base plugin,
/// which is -1, nor its infix syntax, so each is written as a call.
fn is_of_a_package(kind: AstKind) -> bool {
    matches!(
        kind,
        AstKind::FunctionMax
            | AstKind::FunctionMin
            | AstKind::FunctionQuotient
            | AstKind::FunctionRateOf
            | AstKind::FunctionRem
            | AstKind::LogicalImplies
            | AstKind::DistribBernoulli
            | AstKind::DistribBinomial
            | AstKind::DistribCauchy
            | AstKind::DistribChisquare
            | AstKind::DistribExponential
            | AstKind::DistribGamma
            | AstKind::DistribLaplace
            | AstKind::DistribLognormal
            | AstKind::DistribNormal
            | AstKind::DistribPoisson
            | AstKind::DistribRayleigh
            | AstKind::DistribUniform
    )
}

/// The last child of `node`, which libsbml answers for a node of two children or more.
///
/// A unary minus and a unary not answer their one child, and a piecewise which
/// spells out a modulo answers its second operand, which is the first child of
/// the product of the first child of its first child.
fn right_child(node: &Ast) -> Option<&Ast> {
    if is_translated_modulo(node) {
        return node
            .children
            .first()
            .and_then(|difference| difference.children.get(1))
            .and_then(|product| product.children.first());
    }
    if is_unary_minus(node) || is_unary_not(node) {
        return node.children.first();
    }
    if node.children.len() > 1 {
        return node.children.last();
    }
    None
}

/// The name libsbml gives a node of `kind` which carries none of a document.
///
/// This is `ASTNode::getName` of libsbml for every kind whose name a writer
/// reads: the empty string stands for the null libsbml answers for an operator
/// and for a number, which it appends nothing of.
fn libsbml_name(kind: AstKind) -> &'static str {
    match kind {
        // libsbml names a node it could not read nothing, so it writes its
        // children as a parenthesised list of no name at all
        AstKind::Other
        | AstKind::Unknown
        | AstKind::Integer
        | AstKind::Real
        | AstKind::RealE
        | AstKind::Rational
        | AstKind::Plus
        | AstKind::Minus
        | AstKind::Times
        | AstKind::Divide
        | AstKind::Power
        | AstKind::Function
        | AstKind::CsymbolFunction
        | AstKind::Name => "",
        AstKind::NameTime => "time",
        AstKind::NameAvogadro => "avogadro",
        AstKind::ConstantE => "exponentiale",
        AstKind::ConstantFalse => "false",
        AstKind::ConstantPi => "pi",
        AstKind::ConstantTrue => "true",
        AstKind::FunctionAbs => "abs",
        AstKind::FunctionArccos => "arccos",
        AstKind::FunctionArccosh => "arccosh",
        AstKind::FunctionArccot => "arccot",
        AstKind::FunctionArccoth => "arccoth",
        AstKind::FunctionArccsc => "arccsc",
        AstKind::FunctionArccsch => "arccsch",
        AstKind::FunctionArcsec => "arcsec",
        AstKind::FunctionArcsech => "arcsech",
        AstKind::FunctionArcsin => "arcsin",
        AstKind::FunctionArcsinh => "arcsinh",
        AstKind::FunctionArctan => "arctan",
        AstKind::FunctionArctanh => "arctanh",
        AstKind::FunctionCeiling => "ceiling",
        AstKind::FunctionCos => "cos",
        AstKind::FunctionCosh => "cosh",
        AstKind::FunctionCot => "cot",
        AstKind::FunctionCoth => "coth",
        AstKind::FunctionCsc => "csc",
        AstKind::FunctionCsch => "csch",
        AstKind::FunctionDelay => "delay",
        AstKind::FunctionExp => "exp",
        AstKind::FunctionFactorial => "factorial",
        AstKind::FunctionFloor => "floor",
        AstKind::FunctionLn => "ln",
        AstKind::FunctionLog => "log",
        AstKind::FunctionPiecewise => "piecewise",
        AstKind::FunctionPower => "power",
        AstKind::FunctionRoot => "root",
        AstKind::FunctionSec => "sec",
        AstKind::FunctionSech => "sech",
        AstKind::FunctionSin => "sin",
        AstKind::FunctionSinh => "sinh",
        AstKind::FunctionTan => "tan",
        AstKind::FunctionTanh => "tanh",
        AstKind::Lambda => "lambda",
        AstKind::RelationalEq => "eq",
        AstKind::RelationalGeq => "geq",
        AstKind::RelationalGt => "gt",
        AstKind::RelationalLeq => "leq",
        AstKind::RelationalLt => "lt",
        AstKind::RelationalNeq => "neq",
        AstKind::LogicalAnd => "and",
        AstKind::LogicalNot => "not",
        AstKind::LogicalOr => "or",
        AstKind::LogicalXor => "xor",
        AstKind::LogicalImplies => "implies",
        AstKind::FunctionMax => "max",
        AstKind::FunctionMin => "min",
        AstKind::FunctionQuotient => "quotient",
        AstKind::FunctionRateOf => "rateOf",
        AstKind::FunctionRem => "rem",
        AstKind::DistribBernoulli => "bernoulli",
        AstKind::DistribBinomial => "binomial",
        AstKind::DistribCauchy => "cauchy",
        AstKind::DistribChisquare => "chisquare",
        AstKind::DistribExponential => "exponential",
        AstKind::DistribGamma => "gamma",
        AstKind::DistribLaplace => "laplace",
        AstKind::DistribLognormal => "lognormal",
        AstKind::DistribNormal => "normal",
        AstKind::DistribPoisson => "poisson",
        AstKind::DistribRayleigh => "rayleigh",
        AstKind::DistribUniform => "uniform",
    }
}

/* -------------------------------------------------------------- the numbers */

/// Whether `value` is the negative zero libsbml writes as `-0`.
///
/// libsbml reads the high byte of the double and answers that the value is a
/// negative zero when it is `0x80`, which is the sign bit and the seven high
/// bits of the exponent, so every negative value below about 7e-304 answers it
/// too. That is what the python implementation writes, so it is what this
/// writes.
fn is_negative_zero(value: f64) -> bool {
    value.to_bits() >> 56 == 0x80
}

/// `value` as the `"%d"` of C writes a long on the platform of the gate.
///
/// libsbml passes a long to a `"%d"`, which reads the low 32 bits of it, so an
/// exponent or a numerator beyond an `i32` is written as those bits alone.
fn decimal(value: i64) -> String {
    (value as i32).to_string()
}

/// `value` as the `"%.15g"` of C writes it, which is the float format of libsbml.
///
/// `%g` writes a value whose decimal exponent is below -4 or at the precision
/// in exponent notation, with an exponent of at least two digits, and writes
/// every other value in fixed notation; it drops the trailing zeros of the
/// fraction and the point of a fraction of no digit.
///
/// The latex writer uses it as well: libsbml writes the `cn` of a number of its
/// MathML with the same format, and that MathML is what the latex is made of.
pub fn significant_15(value: f64) -> String {
    /// The significant digits of the float format of libsbml.
    const PRECISION: i64 = 15;

    // the lower exponent format of Rust writes the mantissa of the value
    // rounded to the precision, one `e` and the decimal exponent, without the
    // padding and the sign of the `%e` of C
    let scientific = format!("{:.*e}", (PRECISION - 1) as usize, value);
    let mut parts = scientific.splitn(2, 'e');
    let mantissa = parts.next().unwrap_or_default();
    let exponent: i64 = parts.next().unwrap_or_default().parse().unwrap_or(0);
    if !(-4..PRECISION).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!(
            "{}e{sign}{:02}",
            without_trailing_zeros(mantissa),
            exponent.unsigned_abs()
        )
    } else {
        let decimals = usize::try_from(PRECISION - 1 - exponent).unwrap_or(0);
        without_trailing_zeros(&format!("{value:.decimals$}")).to_owned()
    }
}

/// `formatted` without the trailing zeros of its fraction and without a bare point.
///
/// A value of no fraction holds no point and keeps its own trailing zeros.
fn without_trailing_zeros(formatted: &str) -> &str {
    if formatted.contains('.') {
        formatted.trim_end_matches('0').trim_end_matches('.')
    } else {
        formatted
    }
}

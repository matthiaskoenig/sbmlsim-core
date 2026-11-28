//! The unit libsbml 5.21.2 derives for a math, `UnitFormulaFormatter` with the units of the
//! package `l3v2extendedmath`, for a model of SBML Level 3: the unit definition of the math and
//! the flags of undeclared and inconsistent units, which decide what a sum and a power give.
//!
//! The port keeps what decides the units, including where libsbml is surprising: a sum or a
//! maximum of units which are not equivalent has no units, a number without units is
//! undeclared, a sum and a product of more than two terms are binary ones nested to the left, a
//! root takes the integral part of an exponent, the multiplier of a cancelled kind goes to the
//! first unit left, and a function call substitutes its arguments one after the other. It
//! leaves out what decides nothing here, the flag `mCanIgnoreUndeclaredUnits`, and the cache of
//! libsbml keyed by the addresses of nodes, which also hits a node freed after a call of a
//! function definition and gives a later node of the same address its units (case 00856 of the
//! SBML Test Suite, a call of a function definition in the arguments of a call of it).

use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_sbml::{Compartment, LocalParameter, Model, Species};

use super::algebra::{
    DIMENSIONLESS, Definition, Unit, are_equivalent, is_level3_kind, is_variant_of_dimensionless,
    kind_of, simplify,
};

/// The units of a math in a model.
pub(crate) struct Formatter<'m> {
    model: &'m Model,
    /// The unit definitions of the model, their units in the order of their kinds, as sbmlsim
    /// leaves them (`Units.udef_to_str` reorders them before any unit is derived).
    definitions: &'m [(String, Definition)],
    /// The local parameters of the kinetic law whose math is derived.
    local: &'m [LocalParameter],
    undeclared: bool,
    inconsistent: bool,
}

impl<'m> Formatter<'m> {
    pub fn new(
        model: &'m Model,
        definitions: &'m [(String, Definition)],
        local: &'m [LocalParameter],
    ) -> Formatter<'m> {
        Formatter {
            model,
            definitions,
            local,
            undeclared: false,
            inconsistent: false,
        }
    }

    /// The unit definition of a math, `getUnitDefinition` after `resetFlags`.
    pub fn of(&mut self, math: &Node) -> Definition {
        self.reset();
        self.get(math)
    }

    fn reset(&mut self) {
        self.undeclared = false;
        self.inconsistent = false;
    }

    fn definition(&self, id: &str) -> Option<&'m Definition> {
        self.definitions
            .iter()
            .find(|(uid, _)| uid == id)
            .map(|(_, ud)| ud)
    }

    /// `getUnitDefinition`: the dispatch, then a simplification of more than one unit, and an
    /// empty result undeclared unless it is inconsistent.
    fn get(&mut self, node: &Node) -> Definition {
        let mut ud = self.dispatch(node);
        self.finish(&mut ud);
        ud
    }

    /// The end of `getUnitDefinition` for the units of a node.
    fn finish(&mut self, ud: &mut Definition) {
        if ud.len() > 1 {
            simplify(ud);
        }
        if !self.inconsistent && ud.is_empty() {
            self.undeclared = true;
        }
    }

    fn dispatch(&mut self, node: &Node) -> Definition {
        match node {
            Node::Apply { op, args } => self.apply(op, args),
            Node::Number { units, .. } => self.number(units.as_deref()),
            Node::Infinity | Node::NotANumber => self.number(None),
            Node::Symbol(name) => self.name(name),
            Node::CsymbolTime => self.time(),
            Node::Piecewise { pieces, otherwise } => {
                let mut children: Vec<&Node> = Vec::new();
                for (value, condition) in pieces {
                    children.push(value);
                    children.push(condition);
                }
                if let Some(o) = otherwise {
                    children.push(o);
                }
                self.piecewise(&children)
            }
            Node::Lambda { .. } | Node::Pi => vec![Unit::dimensionless()],
            Node::True | Node::False => self.dimensionless_return(false, &[]),
            Node::ExponentialE => {
                self.undeclared = true;
                Vec::new()
            }
            // avogadro, the csymbols delay and rateOf alone and an empty math: no units
            Node::CsymbolAvogadro | Node::CsymbolDelay | Node::CsymbolRateOf | Node::Empty => {
                Vec::new()
            }
        }
    }

    fn apply(&mut self, op: &Op, args: &[Node]) -> Definition {
        // libsbml reads a sum and a product of more than two terms as binary ones nested to
        // the left, `(a * b) * c`, each of which ends as a node does (`finish`); the flags
        // before the first term are the ones before every nested node
        if matches!(op, Op::Plus | Op::Times) && args.len() > 2 {
            let original = self.undeclared;
            let mut ud = self.get(&args[0]);
            for (k, arg) in args.iter().enumerate().skip(1) {
                if *op == Op::Plus {
                    ud = self.arg_units_after(original, ud, &[arg]);
                } else {
                    ud.extend(self.get(arg));
                }
                if k < args.len() - 1 {
                    self.finish(&mut ud);
                }
            }
            return ud;
        }
        // libsbml reads a root without a degree with the degree 2 and a logarithm without a
        // base with the base 10, both integers
        let (two, ten) = (Node::number(2.0), Node::number(10.0));
        let mut children: Vec<&Node> = args.iter().collect();
        match (op, args.len()) {
            (Op::Root, 1) => children.insert(0, &two),
            (Op::Log, 1) => children.insert(0, &ten),
            _ => {}
        }
        match op {
            Op::Plus | Op::Minus | Op::Abs | Op::Ceiling | Op::Floor | Op::Max | Op::Min => {
                self.arg_units_return(&children)
            }
            Op::Times => self.times(&children),
            Op::Divide | Op::Quotient => self.divide(&children),
            Op::Power => self.power(&children),
            Op::Root => self.root(&children),
            Op::Delay | Op::Rem => match children.first() {
                Some(child) => self.get(child),
                None => Vec::new(),
            },
            Op::RateOf => {
                let mut ud = match children.first() {
                    Some(child) => self.get(child),
                    None => Vec::new(),
                };
                for mut unit in self.time() {
                    unit.set_exponent_checking(-unit.exponent);
                    ud.push(unit);
                }
                ud
            }
            Op::Function(name) => self.function(name, args),
            Op::Log => self.dimensionless_return(true, &children),
            Op::Exp
            | Op::Ln
            | Op::Factorial
            | Op::Sin
            | Op::Cos
            | Op::Tan
            | Op::Sec
            | Op::Csc
            | Op::Cot
            | Op::Sinh
            | Op::Cosh
            | Op::Tanh
            | Op::Sech
            | Op::Csch
            | Op::Coth
            | Op::Arcsin
            | Op::Arccos
            | Op::Arctan
            | Op::Arcsec
            | Op::Arccsc
            | Op::Arccot
            | Op::Arcsinh
            | Op::Arccosh
            | Op::Arctanh
            | Op::Arcsech
            | Op::Arccsch
            | Op::Arccoth
            | Op::Eq
            | Op::Neq
            | Op::Gt
            | Op::Lt
            | Op::Geq
            | Op::Leq
            | Op::And
            | Op::Or
            | Op::Xor
            | Op::Not
            | Op::Implies => self.dimensionless_return(false, &children),
        }
    }

    /// `getUnitDefinitionFromTimes`: the units of every factor.
    fn times(&mut self, children: &[&Node]) -> Definition {
        let Some((first, rest)) = children.split_first() else {
            return vec![Unit::dimensionless()];
        };
        let mut ud = self.get(first);
        for child in rest {
            ud.extend(self.get(child));
        }
        ud
    }

    /// `getUnitDefinitionFromDivide`: the units of the first argument over the ones of the
    /// last.
    fn divide(&mut self, children: &[&Node]) -> Definition {
        let Some(first) = children.first() else {
            return Vec::new();
        };
        let mut ud = self.get(first);
        if children.len() == 1 {
            return ud;
        }
        for mut unit in self.get(children[children.len() - 1]) {
            unit.set_exponent_checking(-unit.exponent);
            ud.push(unit);
        }
        ud
    }

    /// `getUnitDefinitionFromPower`: the units of the base to the value of an exponent which
    /// is a number or dimensionless, else the base undeclared or, for an exponent with units,
    /// inconsistent.
    fn power(&mut self, children: &[&Node]) -> Definition {
        if children.is_empty() || children.len() > 2 {
            return Vec::new();
        }
        let mut base = self.get(children[0]);
        if children.len() == 1 {
            self.undeclared = true;
            return base;
        }
        let base_undeclared = self.undeclared;
        let exponent_node = children[1];
        let mut exponent = self.get(exponent_node);
        simplify(&mut exponent);
        let mut inconsistent = false;
        if is_number(exponent_node) || is_variant_of_dimensionless(&exponent) {
            let value = self.evaluate(exponent_node);
            for unit in &mut base {
                unit.set_exponent_checking(value * unit.exponent);
            }
            self.undeclared = base_undeclared;
        } else if !exponent.is_empty() {
            inconsistent = true;
        } else {
            self.undeclared = true;
        }
        if inconsistent {
            base.clear();
            self.inconsistent = true;
        }
        base
    }

    /// `getUnitDefinitionFromRoot`: the units of the radicand with the integral part of their
    /// exponents over the degree.
    fn root(&mut self, children: &[&Node]) -> Definition {
        let Some(radicand) = children.last() else {
            return Vec::new();
        };
        let radicand = self.get(radicand);
        let mut ud = Vec::new();
        if children.len() == 1 {
            return ud;
        }
        let degree = children[0];
        let mut inconsistent = false;
        // libsbml derives the degree once and takes the units of its cache afterwards, which
        // leave the flags as they are
        let mut cached: Option<Definition> = None;
        for mut unit in radicand {
            if unit.kind != DIMENSIONLESS {
                if let Some(value) = number_value(degree) {
                    unit.set_exponent_checking(unit.int_exponent() as f64 / value);
                } else {
                    let mut degree_ud = match &cached {
                        Some(ud) => ud.clone(),
                        None => {
                            let ud = self.get(degree);
                            cached = Some(ud.clone());
                            ud
                        }
                    };
                    if degree_ud.is_empty() {
                        self.undeclared = true;
                    } else {
                        simplify(&mut degree_ud);
                        if is_variant_of_dimensionless(&degree_ud) {
                            let value = self.evaluate(degree);
                            if value.is_nan() {
                                inconsistent = true;
                            } else {
                                unit.set_exponent_checking(unit.int_exponent() as f64 / value);
                            }
                        } else {
                            inconsistent = true;
                        }
                    }
                }
            }
            if inconsistent {
                self.inconsistent = true;
            } else {
                ud.push(unit);
            }
        }
        ud
    }

    /// `getUnitDefinitionFromPiecewise`: the units of the first piece; the other pieces are
    /// derived for their flags while nothing is undeclared.
    fn piecewise(&mut self, children: &[&Node]) -> Definition {
        let ud = match children.first() {
            Some(first) => self.get(first),
            None => Vec::new(),
        };
        let mut n = 0;
        while !self.undeclared && n < children.len() {
            n += 2;
            if let Some(child) = children.get(n) {
                self.get(child);
            }
        }
        ud
    }

    /// `getUnitDefinitionFromDimensionlessReturnFunction`: dimensionless, undeclared when every
    /// argument is (the base of a logarithm, the first argument, not counted).
    fn dimensionless_return(&mut self, log: bool, children: &[&Node]) -> Definition {
        let original = self.undeclared;
        let mut undeclared = 0;
        for (i, child) in children.iter().enumerate() {
            self.get(child);
            if self.undeclared && !(log && i == 0) {
                undeclared += 1;
            }
        }
        self.undeclared = if undeclared == 0 { original } else { true };
        vec![Unit::dimensionless()]
    }

    /// `getUnitDefinitionFromArgUnitsReturnFunction`: the units of the first argument whose
    /// units are declared; none when another argument has units which are not equivalent.
    fn arg_units_return(&mut self, children: &[&Node]) -> Definition {
        let Some((first, rest)) = children.split_first() else {
            return Vec::new();
        };
        let original = self.undeclared;
        let ud = self.get(first);
        self.arg_units_after(original, ud, rest)
    }

    /// [`Formatter::arg_units_return`] after its first argument, whose units are `ud`, with the
    /// flag of undeclared units before it.
    fn arg_units_after(&mut self, original: bool, ud: Definition, rest: &[&Node]) -> Definition {
        let n = rest.len() + 1;
        let mut ud = ud;
        let mut current_undeclared = original;
        let mut i = 0;
        while self.undeclared && i < n - 1 {
            current_undeclared = true;
            i += 1;
            self.reset();
            ud = self.get(rest[i - 1]);
        }
        let mut conflicting = false;
        if !(self.undeclared && i == n - 1) {
            for child in &rest[i..] {
                self.reset();
                let other = self.get(child);
                if !other.is_empty() && !are_equivalent(&ud, &other) {
                    conflicting = true;
                }
                if self.undeclared {
                    current_undeclared = true;
                }
            }
        }
        if n > 1 {
            self.undeclared = current_undeclared;
        }
        if conflicting {
            self.inconsistent = true;
            ud.clear();
        }
        ud
    }

    /// `getUnitDefinitionFromFunction`: the units of the body of the function definition with
    /// the arguments substituted one after the other (`ASTNode::replaceArgument`).
    fn function(&mut self, name: &str, args: &[Node]) -> Definition {
        let lambda = self
            .model
            .function_definitions
            .iter()
            .find(|f| f.id == name)
            .and_then(|f| f.math.as_ref());
        let Some(Node::Lambda { params, body }) = lambda else {
            return Vec::new();
        };
        let mut math = (**body).clone();
        for (param, arg) in params.iter().zip(args) {
            replace(&mut math, param, arg);
        }
        self.get(&math)
    }

    /// `getUnitDefinitionFromOther` of a number: its units of Level 3, else undeclared.
    fn number(&mut self, units: Option<&str>) -> Definition {
        let Some(units) = units else {
            self.undeclared = true;
            return Vec::new();
        };
        if is_level3_kind(units) {
            self.undeclared = false;
            return vec![Unit::of(kind_of(units))];
        }
        match self.definition(units) {
            Some(ud) => {
                self.undeclared = false;
                ud.iter()
                    .map(|u| Unit {
                        checking: false,
                        ..u.clone()
                    })
                    .collect()
            }
            None => Vec::new(),
        }
    }

    /// `getUnitDefinitionFromOther` of a name: a local parameter, a compartment, a species, a
    /// parameter, a species reference or a reaction, in this order; none else.
    fn name(&mut self, name: &str) -> Definition {
        let m = self.model;
        if let Some(p) = self.local.iter().find(|p| p.id == name) {
            return self.parameter(p.units.as_deref());
        }
        if let Some(c) = m.compartments.iter().find(|c| c.id == name) {
            return self.compartment(c);
        }
        if let Some(s) = m.species.iter().find(|s| s.id == name) {
            return self.species(s);
        }
        if let Some(p) = m.parameters.iter().find(|p| p.id == name) {
            return self.parameter(p.units.as_deref());
        }
        let reference = m.reactions.iter().any(|r| {
            r.reactants
                .iter()
                .chain(&r.products)
                .any(|sr| sr.id.as_deref() == Some(name))
        });
        if reference {
            return vec![Unit::dimensionless()];
        }
        if m.reactions.iter().any(|r| r.id == name) {
            return self.reaction();
        }
        Vec::new()
    }

    /// The units of a unit reference: a unit kind, else the units of a unit definition copied
    /// by the unit checking, else `None`.
    fn reference(&self, units: &str) -> Option<Definition> {
        if is_level3_kind(units) {
            return Some(vec![Unit::of(kind_of(units))]);
        }
        self.definition(units).map(|ud| {
            ud.iter()
                .map(|u| {
                    let mut u = u.clone();
                    u.set_exponent_checking(u.exponent);
                    u
                })
                .collect()
        })
    }

    /// The units of a unit reference as the time and the extent are read: a unit kind, else
    /// the units of a unit definition with their exponents as integers (times `sign`), else
    /// undeclared.
    fn integral_reference(&mut self, units: Option<&str>, sign: i64) -> Definition {
        let units = units.unwrap_or("");
        if is_level3_kind(units) {
            let mut unit = Unit::of(kind_of(units));
            unit.exponent = sign as f64;
            return vec![unit];
        }
        match self.definition(units) {
            Some(ud) => ud
                .iter()
                .map(|u| Unit {
                    exponent: (u.int_exponent() * sign) as f64,
                    checking: false,
                    ..u.clone()
                })
                .collect(),
            None => {
                self.undeclared = true;
                Vec::new()
            }
        }
    }

    /// `getTimeUnitDefinition`.
    fn time(&mut self) -> Definition {
        self.integral_reference(self.model.time_units.as_deref(), 1)
    }

    /// The units of a reaction, its extent per time.
    fn reaction(&mut self) -> Definition {
        let mut ud = self.integral_reference(self.model.extent_units.as_deref(), 1);
        ud.extend(self.integral_reference(self.model.time_units.as_deref(), -1));
        ud
    }

    /// `getUnitDefinitionFromParameter`: its units, undeclared without.
    fn parameter(&mut self, units: Option<&str>) -> Definition {
        match units {
            None | Some("") => {
                self.undeclared = true;
                Vec::new()
            }
            Some(units) => self.reference(units).unwrap_or_default(),
        }
    }

    /// `getUnitDefinitionFromCompartment`: its units, else the units of the model for its
    /// spatial dimensions.
    fn compartment(&self, c: &Compartment) -> Definition {
        let m = self.model;
        let units = match c.units.as_deref() {
            Some(units) if !units.is_empty() => Some(units),
            // `(int)` of the spatial dimensions
            _ => match c.spatial_dimensions.map(|d| d.trunc()) {
                Some(1.0) => m.length_units.as_deref(),
                Some(2.0) => m.area_units.as_deref(),
                Some(3.0) => m.volume_units.as_deref(),
                _ => None,
            },
        };
        match units {
            Some(units) if !units.is_empty() => self.reference(units).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// `getUnitDefinitionFromSpecies`: its substance (else the one of the model), over the
    /// size of its compartment unless it has only substance units or is in a compartment of
    /// zero dimensions; none when either is missing.
    fn species(&self, s: &Species) -> Definition {
        let m = self.model;
        let units = s
            .substance_units
            .as_deref()
            .filter(|u| !u.is_empty())
            .or(m.substance_units.as_deref())
            .unwrap_or("");
        if units.is_empty() {
            return Vec::new();
        }
        let Some(mut ud) = self.reference(units) else {
            return Vec::new();
        };
        if s.has_only_substance_units {
            return ud;
        }
        let compartment = m.compartments.iter().find(|c| c.id == s.compartment);
        let Some(c) = compartment else {
            return ud;
        };
        // `getSpatialDimensions` is an unsigned integer
        if c.spatial_dimensions.map(f64::trunc) == Some(0.0) {
            return ud;
        }
        let size = self.compartment(c);
        if size.is_empty() {
            return Vec::new();
        }
        for mut unit in size {
            unit.set_exponent_checking(-unit.exponent);
            ud.push(unit);
        }
        ud
    }

    /// `SBMLTransforms::evaluateASTNode` of an exponent or a degree: the value of a math of
    /// numbers and the values of the parameters and compartments of the model, `NaN` for what
    /// it does not evaluate.
    fn evaluate(&self, node: &Node) -> f64 {
        let m = self.model;
        let args = |args: &[Node]| -> Vec<f64> { args.iter().map(|a| self.evaluate(a)).collect() };
        match node {
            Node::Number { value, .. } => *value,
            Node::Infinity => f64::INFINITY,
            Node::NotANumber => f64::NAN,
            Node::Pi => std::f64::consts::PI,
            Node::ExponentialE => std::f64::consts::E,
            Node::True => 1.0,
            Node::False => 0.0,
            Node::Symbol(name) => m
                .parameters
                .iter()
                .find(|p| &p.id == name)
                .and_then(|p| p.value)
                .or_else(|| {
                    m.compartments
                        .iter()
                        .find(|c| &c.id == name)
                        .and_then(|c| c.size)
                })
                .unwrap_or(f64::NAN),
            Node::Apply { op, args: a } => {
                let v = args(a);
                match (op, v.as_slice()) {
                    (Op::Plus, _) => v.iter().sum(),
                    (Op::Times, _) => v.iter().product(),
                    (Op::Minus, [x]) => -x,
                    (Op::Minus, [x, y]) => x - y,
                    (Op::Divide, [x, y]) => x / y,
                    (Op::Power, [x, y]) => super::algebra::pow(*x, *y),
                    (Op::Root, [x]) => x.sqrt(),
                    (Op::Root, [n, x]) => super::algebra::pow(*x, 1.0 / n),
                    (Op::Abs, [x]) => x.abs(),
                    (Op::Exp, [x]) => libm::exp(*x),
                    (Op::Ln, [x]) => libm::log(*x),
                    (Op::Floor, [x]) => x.floor(),
                    (Op::Ceiling, [x]) => x.ceil(),
                    _ => f64::NAN,
                }
            }
            _ => f64::NAN,
        }
    }
}

/// Whether a math is a number of libsbml, `isInteger` or `isReal`.
fn is_number(node: &Node) -> bool {
    number_value(node).is_some()
}

/// The value of a number of libsbml.
fn number_value(node: &Node) -> Option<f64> {
    match node {
        Node::Number { value, .. } => Some(*value),
        Node::Infinity => Some(f64::INFINITY),
        Node::NotANumber => Some(f64::NAN),
        _ => None,
    }
}

/// Every name `param` of a math replaced by `arg`.
fn replace(node: &mut Node, param: &str, arg: &Node) {
    match node {
        Node::Symbol(name) if name == param => *node = arg.clone(),
        Node::Apply { args, .. } => {
            for a in args {
                replace(a, param, arg);
            }
        }
        Node::Piecewise { pieces, otherwise } => {
            for (value, condition) in pieces {
                replace(value, param, arg);
                replace(condition, param, arg);
            }
            if let Some(o) = otherwise {
                replace(o, param, arg);
            }
        }
        Node::Lambda { body, .. } => replace(body, param, arg),
        _ => {}
    }
}

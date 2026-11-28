//! The unit of a formula, derived as sbmlsim derives it (`_derive_unit` of
//! `simulator/observables.py`): the formula applied to quantities of one in the units of its
//! symbols with the algebra of pint, whose result names the units of the formula.
//!
//! The algebra is pint's: a product and a quotient combine the units without converting one
//! into another (`mmol/l / (mol/m^3)` keeps four units), a sum and the maximum or minimum of
//! two values need one dimension and have the unit of the first, a power raises the units to
//! the number of the exponent, the functions of numbers (`exp`, `log`, the trigonometric ones)
//! take a dimensionless value and give a number, a comparison of one dimension and a logical
//! operation give a number. `piecewise` has a unit only where all its values are numbers
//! (`sign`); a unit which pint cannot derive (a piecewise of quantities, a sum of two
//! dimensions) is `None`, for which an observable needs a declared unit.

use sbmlsim_sbml::math::{Node, Op};
use sbmlsim_units::{DIMENSIONS, Named, Registry};

use crate::formula::unescaped;
use crate::reduce::{Function, Reduced};

/// A value of the algebra: a number of python, or a quantity of pint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Quantity {
    /// The magnitude.
    pub magnitude: f64,
    /// The units of a quantity, `None` for a number.
    pub units: Option<Named>,
}

impl Quantity {
    fn number(magnitude: f64) -> Quantity {
        Quantity {
            magnitude,
            units: None,
        }
    }

    /// The unit as pint writes it, `dimensionless` for a number.
    pub fn text(&self) -> String {
        match &self.units {
            None => "dimensionless".to_string(),
            Some(units) => units.to_string(),
        }
    }
}

/// The units of the symbols and the registry which reads them.
pub(crate) struct Units<'a> {
    pub registry: &'a Registry,
    /// The unit expression of a symbol, `""` without one (dimensionless).
    pub unit_of: &'a dyn Fn(&str) -> String,
}

impl Units<'_> {
    /// The quantity of one in the unit of a symbol, `Quantity(1.0, unit or "dimensionless")`.
    fn symbol(&self, name: &str) -> Option<Quantity> {
        let unit = (self.unit_of)(name);
        let named = self.registry.named(&unit).ok()?;
        Some(Quantity {
            magnitude: named.magnitude,
            units: Some(Named::number(1.0).times(&Named {
                magnitude: 1.0,
                units: named.units,
            })),
        })
    }

    /// The exponents of the base dimensions and the factor to the root units of units.
    fn root(&self, units: &Named) -> Option<([f64; DIMENSIONS.len()], f64)> {
        let mut dims = [0.0; DIMENSIONS.len()];
        let mut factor = 1.0;
        for (name, e) in &units.units {
            let unit = self.registry.parse(name).ok()?;
            for (d, x) in dims.iter_mut().zip(unit.dims) {
                *d += f64::from(x) * e;
            }
            factor *= unit.factor.powf(*e);
        }
        Some((dims, factor))
    }

    /// The dimensions of a value, none for a number.
    fn dims(&self, q: &Quantity) -> Option<[f64; DIMENSIONS.len()]> {
        match &q.units {
            None => Some([0.0; DIMENSIONS.len()]),
            Some(units) => self.root(units).map(|(d, _)| d),
        }
    }

    /// The magnitude of a dimensionless value as a number, `None` for a value of a dimension.
    fn plain(&self, q: &Quantity) -> Option<f64> {
        match &q.units {
            None => Some(q.magnitude),
            Some(units) => {
                let (dims, factor) = self.root(units)?;
                dims.iter()
                    .all(|d| *d == 0.0)
                    .then_some(q.magnitude * factor)
            }
        }
    }

    /// A sum, a difference, a maximum or a minimum: one dimension, the units of the first.
    fn same(&self, a: &Quantity, b: &Quantity, f: impl Fn(f64, f64) -> f64) -> Option<Quantity> {
        let (da, db) = (self.dims(a)?, self.dims(b)?);
        if da != db {
            return None;
        }
        match (&a.units, &b.units) {
            (None, None) => Some(Quantity::number(f(a.magnitude, b.magnitude))),
            (Some(_), None) | (None, Some(_)) => {
                let units = a.units.clone().or_else(|| b.units.clone());
                let (x, y) = (self.plain(a)?, self.plain(b)?);
                Some(Quantity {
                    magnitude: f(x, y),
                    units,
                })
            }
            (Some(ua), Some(ub)) => {
                let (_, fa) = self.root(ua)?;
                let (_, fb) = self.root(ub)?;
                Some(Quantity {
                    magnitude: f(a.magnitude, b.magnitude * fb / fa),
                    units: Some(ua.clone()),
                })
            }
        }
    }

    /// The quantity of a node, `None` where pint has no unit for it.
    pub fn of(&self, node: &Node, scope: &dyn Fn(&str) -> Option<Quantity>) -> Option<Quantity> {
        let args = |args: &[Node]| -> Option<Vec<Quantity>> {
            args.iter().map(|a| self.of(a, scope)).collect()
        };
        match node {
            Node::Number { value, .. } => Some(Quantity::number(*value)),
            Node::True => Some(Quantity::number(1.0)),
            Node::False => Some(Quantity::number(0.0)),
            Node::Pi => Some(Quantity::number(std::f64::consts::PI)),
            Node::ExponentialE => Some(Quantity::number(std::f64::consts::E)),
            Node::Infinity => Some(Quantity::number(f64::INFINITY)),
            Node::NotANumber => Some(Quantity::number(f64::NAN)),
            Node::Symbol(name) => {
                let name = unescaped(name);
                scope(&name).or_else(|| self.symbol(&name))
            }
            Node::CsymbolTime => scope("time").or_else(|| self.symbol("time")),
            Node::Piecewise { pieces, otherwise } => {
                let values = pieces
                    .iter()
                    .map(|(v, _)| v)
                    .chain(otherwise.as_deref())
                    .map(|v| self.of(v, scope))
                    .collect::<Option<Vec<_>>>()?;
                values
                    .iter()
                    .all(|q| q.units.is_none())
                    .then(|| Quantity::number(values.first().map_or(f64::NAN, |q| q.magnitude)))
            }
            Node::Apply { op, args: nodes } => {
                let xs = args(nodes)?;
                self.apply(op, xs)
            }
            _ => None,
        }
    }

    fn apply(&self, op: &Op, xs: Vec<Quantity>) -> Option<Quantity> {
        let first = xs.first()?.clone();
        let fold = |f: &dyn Fn(&Quantity, &Quantity) -> Option<Quantity>| {
            xs[1..].iter().try_fold(first.clone(), |a, b| f(&a, b))
        };
        let number = |f: fn(f64) -> f64| -> Option<Quantity> {
            let x = self.plain(&first)?;
            Some(Quantity::number(f(x)))
        };
        match op {
            Op::Plus => fold(&|a, b| self.same(a, b, |x, y| x + y)),
            Op::Minus if xs.len() == 1 => Some(Quantity {
                magnitude: -first.magnitude,
                units: first.units,
            }),
            Op::Minus => fold(&|a, b| self.same(a, b, |x, y| x - y)),
            Op::Max => fold(&|a, b| self.same(a, b, f64::max)),
            Op::Min => fold(&|a, b| self.same(a, b, f64::min)),
            Op::Times => fold(&|a, b| Some(product(a, b, false))),
            Op::Divide => fold(&|a, b| Some(product(a, b, true))),
            Op::Power => {
                let exponent = self.plain(xs.get(1)?)?;
                Some(power(&first, exponent))
            }
            Op::Root if xs.len() == 1 => Some(power(&first, 0.5)),
            Op::Abs => Some(Quantity {
                magnitude: first.magnitude.abs(),
                units: first.units,
            }),
            Op::Floor => Some(Quantity {
                magnitude: first.magnitude.floor(),
                units: first.units,
            }),
            Op::Ceiling => Some(Quantity {
                magnitude: first.magnitude.ceil(),
                units: first.units,
            }),
            Op::Exp => number(f64::exp),
            Op::Ln => number(f64::ln),
            Op::Log => {
                let base = self.plain(&first)?;
                let x = self.plain(xs.get(1)?)?;
                Some(Quantity::number(x.ln() / base.ln()))
            }
            Op::Sin
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
            | Op::Arccoth => xs
                .iter()
                .all(|q| self.plain(q).is_some())
                .then(|| Quantity::number(1.0)),
            Op::Eq | Op::Neq | Op::Gt | Op::Lt | Op::Geq | Op::Leq => {
                let b = xs.get(1)?;
                let one = self.same(&first, b, |x, y| x - y)?;
                let _ = one;
                Some(Quantity::number(1.0))
            }
            Op::And | Op::Or | Op::Xor | Op::Not | Op::Implies => Some(Quantity::number(1.0)),
            _ => None,
        }
    }
}

/// A product or a quotient of pint.
fn product(a: &Quantity, b: &Quantity, divide: bool) -> Quantity {
    let magnitude = if divide {
        a.magnitude / b.magnitude
    } else {
        a.magnitude * b.magnitude
    };
    let units = match (&a.units, &b.units) {
        (None, None) => None,
        (ua, ub) => {
            let ua = ua.clone().unwrap_or_default();
            let ub = ub.clone().unwrap_or_default();
            Some(if divide {
                ua.divide(&ub)
            } else {
                ua.times(&ub)
            })
        }
    };
    Quantity { magnitude, units }
}

/// A power of pint.
fn power(base: &Quantity, exponent: f64) -> Quantity {
    Quantity {
        magnitude: base.magnitude.powf(exponent),
        units: base.units.as_ref().map(|u| u.power(exponent)),
    }
}

/// The quantity of a formula with its reductions, which keep the unit of their argument: the
/// reductions of a quantity of one are the identity, as in sbmlsim without times. `None` where
/// pint derives no unit.
pub(crate) fn quantity_of(reduced: &Reduced, units: &Units<'_>) -> Option<Quantity> {
    let mut placeholders: Vec<(String, Quantity)> = Vec::new();
    for reduction in &reduced.reductions {
        let scope = |name: &str| {
            placeholders
                .iter()
                .find(|(s, _)| s == name)
                .map(|(_, q)| q.clone())
        };
        let x = units.of(reduction.arguments[0].node(), &scope)?;
        placeholders.push((reduction.symbol.clone(), x));
    }
    let scope = |name: &str| {
        placeholders
            .iter()
            .find(|(s, _)| s == name)
            .map(|(_, q)| q.clone())
    };
    units.of(reduced.outer.node(), &scope)
}

/// The quantities of the times of the `at` reductions of a formula, in their order; empty
/// where pint cannot derive them (`_time_units`).
pub(crate) fn at_times(reduced: &Reduced, units: &Units<'_>) -> Vec<Quantity> {
    let mut placeholders: Vec<(String, Quantity)> = Vec::new();
    let mut times = Vec::new();
    for reduction in &reduced.reductions {
        let scope = |name: &str| {
            placeholders
                .iter()
                .find(|(s, _)| s == name)
                .map(|(_, q)| q.clone())
        };
        let Some(x) = units.of(reduction.arguments[0].node(), &scope) else {
            return Vec::new();
        };
        if reduction.function == Function::At {
            match units.of(reduction.arguments[1].node(), &scope) {
                Some(when) => times.push(when),
                None => return Vec::new(),
            }
        }
        placeholders.push((reduction.symbol.clone(), x));
    }
    times
}

/// The value of a formula at symbols of one, as sbmlsim evaluates it to compare with the
/// magnitude of [`quantity_of`] (`_mixes_scales`).
pub(crate) fn value_at_ones(reduced: &Reduced) -> f64 {
    let mut placeholders: Vec<(String, f64)> = Vec::new();
    let read = |part: &crate::formula::CompiledFormula, placeholders: &[(String, f64)]| {
        let values: Vec<f64> = part
            .symbols()
            .iter()
            .map(|s| {
                placeholders
                    .iter()
                    .find(|(p, _)| p == s)
                    .map_or(1.0, |(_, v)| *v)
            })
            .collect();
        part.evaluate(&values, 1.0)
    };
    for reduction in &reduced.reductions {
        let x = read(&reduction.arguments[0], &placeholders);
        placeholders.push((reduction.symbol.clone(), x));
    }
    read(&reduced.outer, &placeholders)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(formula: &str) -> Option<String> {
        let registry = Registry::new();
        let unit_of = |s: &str| {
            match s {
                "x" => "mmol/l",
                "y" => "mol/m^3",
                "time" => "min",
                "m2" => "m^2",
                "p" => "percent",
                _ => "",
            }
            .to_string()
        };
        let units = Units {
            registry: &registry,
            unit_of: &unit_of,
        };
        quantity_of(&Reduced::new(formula).unwrap(), &units).map(|q| q.text())
    }

    #[test]
    fn units_as_pint_derives_them() {
        for (formula, text) in [
            ("x > y", Some("dimensionless")),
            ("x > 1", None),
            ("piecewise(x, d > 1, y)", None),
            ("exp(d)", Some("dimensionless")),
            ("exp(x)", None),
            ("sqrt(x)", Some("millimole ** 0.5 / liter ** 0.5")),
            ("sqrt(m2)", Some("meter")),
            ("x^2", Some("millimole ** 2 / liter ** 2")),
            ("x^n", Some("millimole / liter")),
            ("abs(x)", Some("millimole / liter")),
            ("max(x,y)", Some("millimole / liter")),
            ("log(x/y)", Some("dimensionless")),
            ("x + y", Some("millimole / liter")),
            ("y + x", Some("mole / meter ** 3")),
            ("x + 1", None),
            ("d + 1", Some("dimensionless")),
            ("x*time", Some("millimole * minute / liter")),
            ("x/x", Some("dimensionless")),
            ("sign(x)", Some("dimensionless")),
            ("x && d", Some("dimensionless")),
            ("p*x", Some("millimole * percent / liter")),
            ("x^-1", Some("liter / millimole")),
            ("2^x", None),
            ("p+d", Some("percent")),
            ("d+p", Some("dimensionless")),
            (
                "max(x) / mean(y)",
                Some("meter ** 3 * millimole / liter / mole"),
            ),
            ("at(x, 5)", Some("millimole / liter")),
        ] {
            assert_eq!(unit(formula).as_deref(), text, "{formula}");
        }
    }
}

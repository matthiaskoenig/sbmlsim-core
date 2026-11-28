//! The registry: the units by their names, the reading of a name (a unit, a prefix and a unit,
//! a plural), the factor of units to their root units as pint computes it, and conversions.

use std::collections::BTreeMap;

use crate::parse::{self, Units, Value, add};
use crate::table::{Builtin, Dim, PREFIXES, UNITS};
use crate::{DIMENSIONS, Dimensions, Named, Unit, UnitError};

/// A unit a name reads: a unit of the registry, under a prefix or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    prefix: Option<usize>,
    unit: usize,
}

/// What a unit of the registry is.
#[derive(Debug, Clone)]
enum Kind {
    /// A base unit of a dimension, of none for a dimensionless one.
    Base(Option<Dim>),
    /// A scale over a reference.
    Derived { scale: f64, reference: Units<Key> },
    /// A unit with an offset, which converts no value by a factor.
    Offset,
}

/// A unit of the registry: its name, the expression it was defined by and what it is.
#[derive(Debug, Clone)]
struct Definition {
    name: String,
    expression: String,
    kind: Kind,
}

/// A registry of units: the builtin table, the units a caller defines and the units of a
/// model.
///
/// ```
/// use sbmlsim_units::Registry;
///
/// let mut registry = Registry::new();
/// assert_eq!(registry.factor("mg/min", "g/hr").unwrap(), 0.06);
/// registry.define("IU_insulin", "0.0347 * mg").unwrap();
/// let unit = registry.parse("IU_insulin/ml").unwrap();
/// assert_eq!(unit.dimensionality(), "[mass] / [length] ** 3");
/// ```
#[derive(Debug, Clone)]
pub struct Registry {
    units: Vec<Definition>,
    /// The names and aliases of the units, by the index of their unit.
    names: BTreeMap<String, usize>,
    /// The names and symbols of the prefixes in the order pint tries them, the empty prefix
    /// first, with the index of their prefix in [`PREFIXES`].
    prefix_names: Vec<(&'static str, Option<usize>)>,
    /// The unit ids of a model, by their expression.
    unit_ids: BTreeMap<String, String>,
    /// The units of the entities of a model, by the id of the entity.
    entity_units: BTreeMap<String, String>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

/// Whether a text is a name a unit can be defined under.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '°')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '°')
        && name != "dimensionless"
        && name != "per"
}

/// A factor which is finite and not zero, else the error of the expression it is of: a
/// power out of the range of a double, which pint refuses as an `OverflowError`.
fn finite(factor: f64, expression: &str) -> Result<f64, UnitError> {
    if factor.is_finite() && factor != 0.0 {
        Ok(factor)
    } else {
        Err(UnitError::Syntax {
            expression: expression.to_owned(),
            reason: format!("its factor is {factor}, out of the range of a double"),
        })
    }
}

impl Registry {
    /// The registry of the builtin table: the base and derived units of SI and the units of
    /// pint which sbmlsim uses, by their names, symbols and aliases, the prefixes of SI from
    /// `q` to `Q` (`µ`, `μ` and `u` for micro), plurals of the names, `M`/`molar`, and the units
    /// sbmlsim adds, `none` and `item` as counts and `percent` as `0.01 * count`.
    ///
    /// # Panics
    ///
    /// Panics for a builtin table which does not read, which the tests of the crate exclude.
    #[must_use]
    pub fn new() -> Registry {
        let mut prefix_names = vec![("", None)];
        for (k, (name, _, aliases)) in PREFIXES.iter().enumerate() {
            prefix_names.push((name, Some(k)));
            for alias in *aliases {
                prefix_names.push((alias, Some(k)));
            }
        }
        let mut registry = Registry {
            units: Vec::new(),
            names: BTreeMap::new(),
            prefix_names,
            unit_ids: BTreeMap::new(),
            entity_units: BTreeMap::new(),
        };
        for (name, builtin, aliases) in UNITS {
            let (expression, kind) = match builtin {
                Builtin::Base(dim) => (
                    dim.map_or_else(|| "[]".to_owned(), |d| format!("[{}]", DIMENSIONS[d])),
                    Kind::Base(*dim),
                ),
                Builtin::Derived(expression) => {
                    let value = registry
                        .value(expression, false)
                        .unwrap_or_else(|err| panic!("the builtin unit {name}: {err}"));
                    (
                        (*expression).to_owned(),
                        Kind::Derived {
                            scale: value.magnitude,
                            reference: value.units,
                        },
                    )
                }
                Builtin::Offset(expression, offset) => {
                    (format!("{expression}; offset: {offset}"), Kind::Offset)
                }
            };
            let index = registry.units.len();
            registry.units.push(Definition {
                name: (*name).to_owned(),
                expression,
                kind,
            });
            registry.names.insert((*name).to_owned(), index);
            for alias in *aliases {
                registry.names.insert((*alias).to_owned(), index);
            }
        }
        registry
    }

    /// A copy of the registry with the units of a model: `units` are the unit expressions of
    /// its entities by their ids (the `units` of its `model/2` description), which
    /// [`Registry::unit_of`] gives, and `unit_ids` its unit definitions, an id with its
    /// expression, which name a unit in every expression the copy parses.
    ///
    /// A unit id is replaced by its expression as a whole, `mg_per_min/kg` with `mg_per_min =
    /// mg/min` reads `(mg/min)/kg`, and only once, so the names of its expression are units of
    /// the registry; a unit id wins over a unit of the registry of the same name, since the
    /// model defines what its id means, and an empty expression is dimensionless. This is
    /// `UnitsInformation.quantity` of sbmlsim.
    #[must_use]
    pub fn with_model_units(
        &self,
        units: &BTreeMap<String, String>,
        unit_ids: &BTreeMap<String, String>,
    ) -> Registry {
        let mut registry = self.clone();
        registry.entity_units.clone_from(units);
        registry.unit_ids.clone_from(unit_ids);
        registry
    }

    /// The unit expression of an entity of the model of [`Registry::with_model_units`].
    #[must_use]
    pub fn unit_of(&self, entity: &str) -> Option<&str> {
        self.entity_units.get(entity).map(String::as_str)
    }

    /// Defines a unit by an expression of the units of the registry, as `define` of pint does
    /// (`IU_insulin = 0.0347 * mg`).
    ///
    /// A name which is a unit already, under a prefix or a plural as well, is refused unless
    /// the expression gives it the same factor and dimensions, in which case nothing changes.
    ///
    /// # Errors
    ///
    /// A name which is no identifier, an expression which does not parse, and a name which is
    /// a unit of another factor or other dimensions.
    pub fn define(&mut self, name: &str, expression: &str) -> Result<(), UnitError> {
        if !is_name(name) {
            return Err(UnitError::Name {
                name: name.to_owned(),
            });
        }
        let value = self.value(expression, false)?;
        let unit = self.unit(&value, expression)?;
        if let Some(key) = self.key_of(name) {
            let existing = self.unit(
                &Value {
                    magnitude: 1.0,
                    units: vec![(key, 1)],
                },
                name,
            )?;
            if existing == unit {
                return Ok(());
            }
            return Err(UnitError::Redefinition {
                name: name.to_owned(),
                existing: self.describe(key),
                expression: expression.to_owned(),
            });
        }
        let index = self.units.len();
        self.units.push(Definition {
            name: name.to_owned(),
            expression: expression.to_owned(),
            kind: Kind::Derived {
                scale: value.magnitude,
                reference: value.units,
            },
        });
        self.names.insert(name.to_owned(), index);
        Ok(())
    }

    /// The factor of a unit expression to the root units and its dimensions.
    ///
    /// The factor is the magnitude of `ureg.Quantity(1, expression).to_root_units()` of pint
    /// (of `ureg(expression)` for an expression with a number): the root units are the base
    /// units of pint, `meter`, `gram`, `second`, `mole`, `ampere`, `kelvin` and `candela`.
    ///
    /// # Errors
    ///
    /// An expression which does not parse, names a unit the registry does not have or a unit
    /// with an offset, has a power which is no integer or a factor which is zero or not finite.
    pub fn parse(&self, expression: &str) -> Result<Unit, UnitError> {
        let value = self.value(expression, true)?;
        self.unit(&value, expression)
    }

    /// The factor which converts a value in the unit `from` into the unit `to`.
    ///
    /// It is the factor of `Quantity(1, from).to(to)` of pint: the factor of the root units of
    /// `from / to`, so it is exactly 1 for two expressions of the same units and exactly 0.06
    /// for `mg/min` to `g/hr`, where the quotient of the two factors to the root units would
    /// round twice.
    ///
    /// # Errors
    ///
    /// The errors of [`Registry::parse`] for either expression, and two expressions of other
    /// dimensions, whose error names both.
    pub fn factor(&self, from: &str, to: &str) -> Result<f64, UnitError> {
        let a = self.value(from, true)?;
        let b = self.value(to, true)?;
        let (_, from_dims) = self.root(&a.units, from)?;
        let (_, to_dims) = self.root(&b.units, to)?;
        if from_dims != to_dims {
            return Err(UnitError::Dimension {
                from: from.to_owned(),
                to: to.to_owned(),
                from_dimensions: crate::dimensionality(&from_dims),
                to_dimensions: crate::dimensionality(&to_dims),
            });
        }
        let mut units = a.units;
        for (key, exponent) in b.units {
            add(&mut units, key, -exponent);
        }
        let (factor, _) = self.root(&units, from)?;
        finite(a.magnitude * factor / b.magnitude, from)
    }

    /// Converts values in the unit `from` into the unit `to` in place, each multiplied by
    /// [`Registry::factor`].
    ///
    /// # Errors
    ///
    /// The errors of [`Registry::factor`]; the values are unchanged then.
    pub fn convert(&self, values: &mut [f64], from: &str, to: &str) -> Result<(), UnitError> {
        let factor = self.factor(from, to)?;
        for value in values {
            *value *= factor;
        }
        Ok(())
    }

    /// A unit expression as pint names it: the magnitude of its numbers and its units by the
    /// names of pint, a prefix and a unit written out (`mmol/l` is `millimole / liter`), which
    /// [`Named`] formats as `str(ureg.Unit(expression))` of pint.
    ///
    /// # Errors
    ///
    /// An expression which does not parse or names a unit the registry does not have.
    pub fn named(&self, expression: &str) -> Result<Named, UnitError> {
        let value = self.value(expression, true)?;
        let mut units = BTreeMap::new();
        for (key, exponent) in value.units {
            let definition = &self.units[key.unit];
            let name = match key.prefix {
                Some(p) => format!("{}{}", PREFIXES[p].0, definition.name),
                None => definition.name.clone(),
            };
            *units.entry(name).or_insert(0.0) += f64::from(exponent);
        }
        units.retain(|_, e| *e != 0.0);
        Ok(Named {
            magnitude: value.magnitude,
            units,
        })
    }

    /// The unit of a value: its magnitude times the factor of its units to the root units.
    fn unit(&self, value: &Value<Key>, expression: &str) -> Result<Unit, UnitError> {
        let (factor, dims) = self.root(&value.units, expression)?;
        Ok(Unit {
            factor: finite(value.magnitude * factor, expression)?,
            dims,
        })
    }

    /// The value of an expression, its unit ids read when `ids` is set.
    fn value(&self, expression: &str, ids: bool) -> Result<Value<Key>, UnitError> {
        let value = match parse::parse(expression)? {
            None => Value::number(1.0),
            Some(expr) => expr.evaluate(expression, &mut |name| {
                self.name_value(name, expression, ids)
            })?,
        };
        if !value.magnitude.is_finite() || value.magnitude == 0.0 {
            return Err(UnitError::Syntax {
                expression: expression.to_owned(),
                reason: format!(
                    "its number is {}, not a finite number other than zero",
                    value.magnitude
                ),
            });
        }
        Ok(value)
    }

    /// The value of a name of an expression: a unit id, `dimensionless` or a unit.
    fn name_value(&self, name: &str, expression: &str, ids: bool) -> Result<Value<Key>, UnitError> {
        if ids && let Some(definition) = self.unit_ids.get(name) {
            return self.value(definition, false);
        }
        if name == "dimensionless" {
            return Ok(Value::number(1.0));
        }
        match self.key_of(name) {
            Some(key) => Ok(Value {
                magnitude: 1.0,
                units: vec![(key, 1)],
            }),
            None => Err(UnitError::Undefined {
                expression: expression.to_owned(),
                name: name.to_owned(),
            }),
        }
    }

    /// The unit a name reads, as `parse_unit_name` of pint reads it: a name or alias of a unit,
    /// else the first prefix and unit (with the plural `s` after the singular ones) which make
    /// it up, the prefixes in the order of pint.
    fn key_of(&self, name: &str) -> Option<Key> {
        if let Some(&unit) = self.names.get(name) {
            return Some(Key { prefix: None, unit });
        }
        for suffix in ["", "s"] {
            let Some(stem) = name.strip_suffix(suffix) else {
                continue;
            };
            for (prefix_name, prefix) in &self.prefix_names {
                let Some(rest) = stem.strip_prefix(prefix_name) else {
                    continue;
                };
                // pint reads no plural of a name of one character (`ms` is no `m`)
                if rest.is_empty() || (!suffix.is_empty() && rest.chars().count() == 1) {
                    continue;
                }
                if let Some(&unit) = self.names.get(rest) {
                    return Some(Key {
                        prefix: *prefix,
                        unit,
                    });
                }
            }
        }
        None
    }

    /// The text of a unit a key reads, for an error: the expression of its definition, or the
    /// prefix and the name of its unit.
    fn describe(&self, key: Key) -> String {
        let definition = &self.units[key.unit];
        match key.prefix {
            Some(p) => format!("{}{}", PREFIXES[p].0, definition.name),
            None => definition.expression.clone(),
        }
    }

    /// The factor of units to the root units and their dimensions, as `_get_root_units` of
    /// pint computes it.
    ///
    /// Every scale met on the way down to the base units is counted with its exponent, the
    /// scales of a positive exponent apart from those of a negative one, equal scales
    /// together; a scale on both sides cancels, and the factor is the product of the powers of
    /// the scales of a positive exponent and then of a negative one, in the order of their
    /// first appearance. `liter` over `milliliter` is so exactly 1000, as in pint.
    fn root(&self, units: &Units<Key>, expression: &str) -> Result<(f64, Dimensions), UnitError> {
        let mut fraction = Fraction::default();
        self.descend(units, 1, &mut fraction, expression)?;
        Ok((fraction.factor(), fraction.dims))
    }

    fn descend(
        &self,
        units: &Units<Key>,
        exponent: i32,
        fraction: &mut Fraction,
        expression: &str,
    ) -> Result<(), UnitError> {
        for &(key, e) in units {
            let e2 = exponent
                .checked_mul(e)
                .ok_or_else(|| parse::too_large(expression))?;
            let definition = &self.units[key.unit];
            if let Some(p) = key.prefix {
                if matches!(definition.kind, Kind::Offset) {
                    return Err(self.offset(key, expression));
                }
                fraction.count(PREFIXES[p].1, e2);
                let reference = vec![(
                    Key {
                        prefix: None,
                        unit: key.unit,
                    },
                    1,
                )];
                self.descend(&reference, e2, fraction, expression)?;
                continue;
            }
            match &definition.kind {
                Kind::Base(dim) => {
                    if let Some(d) = dim {
                        fraction.dims[*d] += e2;
                    }
                }
                Kind::Derived { scale, reference } => {
                    fraction.count(*scale, e2);
                    self.descend(reference, e2, fraction, expression)?;
                }
                Kind::Offset => return Err(self.offset(key, expression)),
            }
        }
        Ok(())
    }

    fn offset(&self, key: Key, expression: &str) -> UnitError {
        let definition = &self.units[key.unit];
        let unit = match key.prefix {
            Some(p) => format!("{}{}", PREFIXES[p].0, definition.name),
            None => definition.name.clone(),
        };
        UnitError::Offset {
            expression: expression.to_owned(),
            unit,
        }
    }
}

/// The scales of the units on the way to the root units: those of a positive exponent and
/// those of a negative one with the sum of their exponents, and the exponents of the
/// dimensions.
#[derive(Default)]
struct Fraction {
    numerator: Vec<(f64, i32)>,
    denominator: Vec<(f64, i32)>,
    dims: Dimensions,
}

impl Fraction {
    /// Counts a scale with its exponent, on the side of its sign.
    fn count(&mut self, scale: f64, exponent: i32) {
        let (side, exponent) = if exponent < 0 {
            (&mut self.denominator, -exponent)
        } else {
            (&mut self.numerator, exponent)
        };
        match side.iter_mut().find(|(s, _)| *s == scale) {
            Some(entry) => entry.1 += exponent,
            None => side.push((scale, exponent)),
        }
    }

    /// The factor: the scales on both sides cancelled, the powers of the numerator and then of
    /// the denominator multiplied in their order, with `pow` (the one of `libm`, so a factor
    /// has the same bits natively and in wasm).
    fn factor(&mut self) -> f64 {
        // a scale on both sides cancels against the smaller exponent of the two: what pint's
        // loop leaves, with the order of the scales kept
        for entry in &mut self.numerator {
            if let Some(at) = self.denominator.iter().position(|(s, _)| *s == entry.0) {
                let d = self.denominator[at].1;
                if entry.1 >= d {
                    entry.1 -= d;
                    self.denominator.remove(at);
                }
            }
        }
        for entry in &mut self.denominator {
            if let Some(at) = self.numerator.iter().position(|(s, _)| *s == entry.0) {
                let n = self.numerator[at].1;
                if entry.1 >= n {
                    entry.1 -= n;
                    self.numerator.remove(at);
                }
            }
        }
        let mut factor = 1.0;
        for &(scale, exponent) in &self.numerator {
            if exponent != 0 {
                factor *= libm::pow(scale, f64::from(exponent));
            }
        }
        for &(scale, exponent) in &self.denominator {
            if exponent != 0 {
                factor *= libm::pow(scale, -f64::from(exponent));
            }
        }
        factor
    }
}

#[cfg(test)]
mod tests {
    use super::Registry;

    #[test]
    fn names_as_pint_reads_them() {
        let r = Registry::new();
        // an exact name before a prefix: `min` is no milli-inch, `cd` no centi-day
        assert_eq!(r.parse("min").unwrap().factor, 60.0);
        assert_eq!(r.parse("cd").unwrap().dims[6], 1);
        // a prefix, a plural, both
        assert_eq!(r.parse("ms").unwrap().factor, 1e-3);
        assert_eq!(r.parse("grams").unwrap().factor, 1.0);
        assert_eq!(r.parse("milligrams").unwrap().factor, 1e-3);
        assert_eq!(r.parse("mmHg").unwrap().factor, 133_322.387_415);
        assert!(r.parse("ft").is_err());
    }

    #[test]
    fn exact_factors_of_pint() {
        let r = Registry::new();
        assert_eq!(r.factor("l", "ml").unwrap(), 1000.0);
        assert_eq!(r.factor("mg/min", "g/hr").unwrap(), 0.06);
        assert_eq!(r.factor("mM", "mM").unwrap(), 1.0);
        assert_eq!(r.factor("", "dimensionless").unwrap(), 1.0);
    }
}

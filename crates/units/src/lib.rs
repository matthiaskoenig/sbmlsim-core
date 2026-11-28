//! The units of sbmlsim-core: unit expressions parsed into a factor and the exponents of the
//! base dimensions, and conversions between them.
//!
//! sbmlsim computes with the units of pint (`sbmlsim.units.ureg`); this crate is the same
//! arithmetic in Rust, so the scan planner, python and JavaScript convert times and values
//! with one authority and with the numbers of pint. An expression is what pint reads for the
//! units sbmlsim uses ([`parse`](Registry::parse)): products, quotients, integer powers,
//! parentheses and numbers (`ml/min/(1.73*m^2)`, `micromol/(0.5*hr)`), the prefixes of SI in
//! their short and long forms (`µ`, `μ` and `u` for micro), the short and long names of pint
//! and their plurals (`mg`, `milligram`, `milligrams`, `hr`, `hour`, `h`, `l`, `liter`,
//! `litre`, `M` for molar), and the units sbmlsim adds, `none` and `item` as counts and
//! `percent` as `0.01 * count`.
//!
//! A [`Unit`] is the factor of an expression to the root units of pint (`meter`, `gram`,
//! `second`, `mole`, `ampere`, `kelvin`, `candela`) and the exponents of their dimensions. The
//! factor is computed as pint computes it, scale by scale down the definitions of the units
//! (`hour = 60 * minute`, `liter = decimeter ** 3`, `molar = mole / liter`) with the scales on
//! both sides of a quotient cancelled, so it agrees with pint to the last bit or within a few
//! units in the last place (the corpus of sbmlsim, `testdata/units_corpus.json`, holds it to
//! 1e-15 relative). A conversion ([`factor`](Registry::factor)) is the factor of the quotient
//! of the two expressions, as pint converts, which is exactly 1000 for `l` to `ml`. Two
//! expressions of other dimensions are an error naming both, and a unit with an offset
//! (`degC`) is known and refused, since no factor converts it.
//!
//! The powers are integers: pint reads `m**0.5`, which no unit of sbmlsim has, and this crate
//! refuses it.
//!
//! ```
//! use std::collections::BTreeMap;
//! use sbmlsim_units::Registry;
//!
//! let registry = Registry::new();
//! let unit = registry.parse("mg/l").unwrap();
//! assert_eq!(unit.dimensionality(), "[mass] / [length] ** 3");
//! let mut values = [1.0, 2.0];
//! registry.convert(&mut values, "hr", "min").unwrap();
//! assert_eq!(values, [60.0, 120.0]);
//!
//! // the unit definitions of a model are units in its expressions
//! let ids = BTreeMap::from([("mg_per_min".to_owned(), "mg/min".to_owned())]);
//! let model = registry.with_model_units(&BTreeMap::new(), &ids);
//! assert_eq!(model.factor("mg_per_min/kg", "mg/min/kg").unwrap(), 1.0);
//! ```

mod parse;
mod registry;
mod table;

pub use registry::Registry;

/// The base dimensions, the order of the exponents of a [`Unit`]: the root dimensions of pint
/// without their brackets (`[length]` is `length`).
pub const DIMENSIONS: [&str; 7] = [
    "length",
    "mass",
    "time",
    "substance",
    "current",
    "temperature",
    "luminosity",
];

/// The exponents of the base dimensions, in the order of [`DIMENSIONS`].
pub type Dimensions = [i32; DIMENSIONS.len()];

/// A unit: its factor to the root units of pint and the exponents of its dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    /// The factor to the root units, the magnitude of `Quantity(1, unit).to_root_units()` of
    /// pint.
    pub factor: f64,
    /// The exponents of the base dimensions, in the order of [`DIMENSIONS`].
    pub dims: Dimensions,
}

impl Unit {
    /// The dimensions with an exponent other than zero, by their names, in the order of
    /// [`DIMENSIONS`].
    #[must_use]
    pub fn dimensions(&self) -> Vec<(&'static str, i32)> {
        DIMENSIONS
            .iter()
            .zip(self.dims)
            .filter(|(_, e)| *e != 0)
            .map(|(name, e)| (*name, e))
            .collect()
    }

    /// The dimensions as pint writes them, `[mass] / [length] ** 3`, `dimensionless` for none.
    #[must_use]
    pub fn dimensionality(&self) -> String {
        dimensionality(&self.dims)
    }
}

/// The dimensions as pint writes them: the ones of a positive exponent joined by `*`, then
/// the ones of a negative exponent after a `/`.
pub(crate) fn dimensionality(dims: &Dimensions) -> String {
    let term = |name: &str, e: i32| {
        if e == 1 {
            format!("[{name}]")
        } else {
            format!("[{name}] ** {e}")
        }
    };
    let up: Vec<String> = DIMENSIONS
        .iter()
        .zip(dims)
        .filter(|(_, e)| **e > 0)
        .map(|(name, e)| term(name, *e))
        .collect();
    let down: Vec<String> = DIMENSIONS
        .iter()
        .zip(dims)
        .filter(|(_, e)| **e < 0)
        .map(|(name, e)| term(name, -*e))
        .collect();
    match (up.is_empty(), down.is_empty()) {
        (true, true) => "dimensionless".to_owned(),
        (false, true) => up.join(" * "),
        (true, false) => format!("1 / {}", down.join(" / ")),
        (false, false) => format!("{} / {}", up.join(" * "), down.join(" / ")),
    }
}

/// A unit as pint names it, [`Registry::named`]: the magnitude of the numbers of its
/// expression and the exponent of every unit by its name in pint, which the algebra of pint's
/// quantities combines (a product adds the exponents, a power multiplies them) without
/// converting one unit into another, so `mmol/l / (mol/m^3)` stays four units.
///
/// It formats as pint's default format of a unit: the names of a positive exponent sorted and
/// joined by ` * `, then the ones of a negative exponent after ` / ` each, an exponent other
/// than one as ` ** e`, `1 / ...` without a numerator and `dimensionless` without units.
///
/// ```
/// use sbmlsim_units::Registry;
///
/// let r = Registry::new();
/// let ratio = r.named("mmol/l").unwrap().divide(&r.named("mol/m^3").unwrap());
/// assert_eq!(ratio.to_string(), "meter ** 3 * millimole / liter / mole");
/// assert_eq!(r.named("1/min").unwrap().to_string(), "1 / minute");
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Named {
    /// The magnitude of the numbers of the expression, `1` for an expression of units only.
    pub magnitude: f64,
    /// The exponent of every unit by its name; no exponent is zero.
    pub units: std::collections::BTreeMap<String, f64>,
}

impl Named {
    /// A number without units.
    #[must_use]
    pub fn number(magnitude: f64) -> Named {
        Named {
            magnitude,
            units: std::collections::BTreeMap::new(),
        }
    }

    /// The units with the exponents of other units added, `sign` times.
    fn combined(&self, other: &Named, sign: f64, magnitude: f64) -> Named {
        let mut units = self.units.clone();
        for (name, exponent) in &other.units {
            *units.entry(name.clone()).or_insert(0.0) += sign * exponent;
        }
        units.retain(|_, e| *e != 0.0);
        Named { magnitude, units }
    }

    /// The product, as pint multiplies two quantities.
    #[must_use]
    pub fn times(&self, other: &Named) -> Named {
        self.combined(other, 1.0, self.magnitude * other.magnitude)
    }

    /// The quotient, as pint divides two quantities.
    #[must_use]
    pub fn divide(&self, other: &Named) -> Named {
        self.combined(other, -1.0, self.magnitude / other.magnitude)
    }

    /// The power, as pint raises a quantity to a number.
    #[must_use]
    pub fn power(&self, exponent: f64) -> Named {
        let mut units = self.units.clone();
        for e in units.values_mut() {
            *e *= exponent;
        }
        units.retain(|_, e| *e != 0.0);
        Named {
            magnitude: self.magnitude.powf(exponent),
            units,
        }
    }

    /// Whether it has no units.
    #[must_use]
    pub fn is_unitless(&self) -> bool {
        self.units.is_empty()
    }
}

/// An exponent as python writes the number: an integral one without its fraction.
fn exponent_text(exponent: f64) -> String {
    if exponent.fract() == 0.0 && exponent.abs() < 1e15 {
        format!("{}", exponent as i64)
    } else {
        format!("{exponent}")
    }
}

impl std::fmt::Display for Named {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let term = |name: &str, e: f64| {
            if e == 1.0 {
                name.to_owned()
            } else {
                format!("{name} ** {}", exponent_text(e))
            }
        };
        let up: Vec<String> = self
            .units
            .iter()
            .filter(|(_, e)| **e > 0.0)
            .map(|(n, e)| term(n, *e))
            .collect();
        let down: Vec<String> = self
            .units
            .iter()
            .filter(|(_, e)| **e < 0.0)
            .map(|(n, e)| term(n, -*e))
            .collect();
        match (up.is_empty(), down.is_empty()) {
            (true, true) => f.write_str("dimensionless"),
            (false, true) => f.write_str(&up.join(" * ")),
            (true, false) => write!(f, "1 / {}", down.join(" / ")),
            (false, false) => write!(f, "{} / {}", up.join(" * "), down.join(" / ")),
        }
    }
}

/// An expression which is no unit, a unit which converts no value by a factor, or two units
/// which do not convert into each other. Every error names the expression it is about.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UnitError {
    /// An expression which does not parse, or whose number is zero or not finite.
    #[error("the unit '{expression}' is no unit expression: {reason}")]
    Syntax {
        /// The expression.
        expression: String,
        /// What is wrong with it.
        reason: String,
    },
    /// A name of an expression which is no unit of the registry.
    #[error("the unit '{name}' of '{expression}' is not defined")]
    Undefined {
        /// The expression.
        expression: String,
        /// The name.
        name: String,
    },
    /// A unit with an offset, which converts no value by a factor.
    #[error(
        "the unit '{unit}' of '{expression}' has an offset, so no factor converts it; \
         use kelvin"
    )]
    Offset {
        /// The expression.
        expression: String,
        /// The unit with the offset.
        unit: String,
    },
    /// A power of units which is no integer.
    #[error(
        "the unit '{expression}' has the power {exponent}, which is no integer: the powers of \
         units are integers"
    )]
    FractionalPower {
        /// The expression.
        expression: String,
        /// The power.
        exponent: f64,
    },
    /// Two units of other dimensions.
    #[error("cannot convert '{from}' ({from_dimensions}) to '{to}' ({to_dimensions})")]
    Dimension {
        /// The unit of the values.
        from: String,
        /// The unit to convert them into.
        to: String,
        /// The dimensions of `from`, as pint writes them.
        from_dimensions: String,
        /// The dimensions of `to`.
        to_dimensions: String,
    },
    /// A unit defined again with another factor or other dimensions.
    #[error("the unit '{name}' is defined as '{existing}' and cannot be defined as '{expression}'")]
    Redefinition {
        /// The name.
        name: String,
        /// The definition it has.
        existing: String,
        /// The expression it was to be defined as.
        expression: String,
    },
    /// A name a unit cannot be defined under.
    #[error("'{name}' is no name of a unit: a letter or '_' and then letters, digits or '_'")]
    Name {
        /// The name.
        name: String,
    },
}

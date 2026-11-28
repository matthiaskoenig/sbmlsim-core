//! The unit definitions of libsbml 5.21.2 and the operations on them which derive the unit of
//! a math (`Unit` and `UnitDefinition` of `src/sbml`): a unit is `(multiplier * 10^scale *
//! kind)^exponent`, a unit definition a list of units in an order which matters, since
//! [`simplify`] gives a remaining multiplier to the first unit.
//!
//! The powers are the ones of `libm`, the same bits natively and in wasm; libsbml rounds every
//! multiplier it merges or rescales to 15 digits ([`truncate`]), which hides the last bit of a
//! power.

/// The unit kinds of SBML in the order of `UnitKind_t` of libsbml, which orders the units of a
/// definition ([`reorder`]) and numbers the kinds of a unit signature.
pub(crate) const KINDS: [&str; 36] = [
    "ampere",
    "avogadro",
    "becquerel",
    "candela",
    "celsius",
    "coulomb",
    "dimensionless",
    "farad",
    "gram",
    "gray",
    "henry",
    "hertz",
    "item",
    "joule",
    "katal",
    "kelvin",
    "kilogram",
    "liter",
    "litre",
    "lumen",
    "lux",
    "meter",
    "metre",
    "mole",
    "newton",
    "ohm",
    "pascal",
    "radian",
    "second",
    "siemens",
    "sievert",
    "steradian",
    "tesla",
    "volt",
    "watt",
    "weber",
];

/// The kind which is no unit kind, `UNIT_KIND_INVALID`.
pub(crate) const INVALID: usize = KINDS.len();

/// The kind `dimensionless`.
pub(crate) const DIMENSIONLESS: usize = 6;

/// The kind of a name, [`INVALID`] for a name which is no unit kind.
pub(crate) fn kind_of(name: &str) -> usize {
    KINDS.iter().position(|k| *k == name).unwrap_or(INVALID)
}

/// Whether a name is a unit kind of SBML Level 3, `UnitKind_isValidUnitKindString`: every kind
/// but `celsius`, `liter` and `meter`.
pub(crate) fn is_level3_kind(name: &str) -> bool {
    !matches!(name, "celsius" | "liter" | "meter") && kind_of(name) != INVALID
}

/// A unit of libsbml.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unit {
    /// The kind, an index of [`KINDS`] or [`INVALID`].
    pub kind: usize,
    /// The exponent, `mExponentDouble`.
    pub exponent: f64,
    /// The scale.
    pub scale: i32,
    /// The multiplier.
    pub multiplier: f64,
    /// Whether the exponent was set by the unit checking of libsbml
    /// (`mInternalUnitCheckingFlag`), which decides whether [`simplify`] reads it as a double.
    pub checking: bool,
}

impl Unit {
    /// A unit of a kind with the defaults, `initDefaults`.
    pub fn of(kind: usize) -> Unit {
        Unit {
            kind,
            exponent: 1.0,
            scale: 0,
            multiplier: 1.0,
            checking: false,
        }
    }

    /// The unit `dimensionless`.
    pub fn dimensionless() -> Unit {
        Unit::of(DIMENSIONLESS)
    }

    /// Sets the exponent as the unit checking does, `setExponentUnitChecking`.
    pub fn set_exponent_checking(&mut self, exponent: f64) {
        self.exponent = exponent;
        self.checking = true;
    }

    /// The exponent as an integer, `getExponent` of Level 3: the exponent when it is integral,
    /// else 0.
    pub fn int_exponent(&self) -> i64 {
        if self.exponent.ceil() == self.exponent.floor() {
            // `static_cast<int>`
            self.exponent as i64
        } else {
            0
        }
    }
}

/// A unit definition of libsbml, its units in their order.
pub(crate) type Definition = Vec<Unit>;

/// `pow` of C.
pub(crate) fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

/// `util_isEqual`: closer than the square root of the machine epsilon.
pub(crate) fn is_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < f64::EPSILON.sqrt()
}

/// A double rounded to 15 significant digits, `truncateDoublePrecision` (an `ostringstream` of
/// precision 15 read back with `strtod`).
pub(crate) fn truncate(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// `Unit::removeScale`: the scale moved into the multiplier.
pub(crate) fn remove_scale(unit: &mut Unit) {
    let multiplier = unit.multiplier * pow(10.0, f64::from(unit.scale));
    unit.multiplier = truncate(multiplier);
    unit.scale = 0;
}

/// `Unit::merge` of two units of one kind into the first.
fn merge(first: &mut Unit, second: &Unit) {
    let mut second = second.clone();
    remove_scale(first);
    remove_scale(&mut second);
    let exponent = first.exponent + second.exponent;
    let factor = |u: &Unit| {
        if is_equal(u.exponent, 0.0) && !is_equal(u.multiplier, 1.0) {
            u.multiplier
        } else {
            pow(u.multiplier, u.exponent)
        }
    };
    let product = factor(first) * factor(&second);
    let multiplier = if exponent == 0.0 {
        product
    } else {
        pow(product, 1.0 / exponent)
    };
    first.scale = 0;
    first.exponent = exponent;
    first.multiplier = truncate(multiplier);
}

/// `UnitDefinition::simplify`: a dimensionless unit dropped beside others (its multiplier
/// kept), the units of a kind merged, the units of exponent 0 dropped, and the multiplier of
/// what was dropped given to the first unit, or a dimensionless unit of it when none is left.
pub(crate) fn simplify(ud: &mut Definition) {
    let dimensionless_present = ud.iter().any(|u| u.kind == DIMENSIONLESS);
    let mut cancelled = false;
    let mut saved = 1.0;
    if ud.len() > 1 {
        if dimensionless_present {
            for n in (0..ud.len()).rev() {
                remove_scale(&mut ud[n]);
                if ud[n].kind == DIMENSIONLESS {
                    let factor = pow(ud[n].multiplier, ud[n].int_exponent() as f64);
                    if !is_equal(factor, 1.0) {
                        cancelled = true;
                        saved *= factor;
                    }
                    ud.remove(n);
                }
            }
        }
        let mut n = 0;
        while n < ud.len() {
            let mut i = n + 1;
            while i < ud.len() {
                if ud[i].kind == ud[n].kind {
                    let other = ud.remove(i);
                    merge(&mut ud[n], &other);
                } else {
                    i += 1;
                }
            }
            n += 1;
        }
    }
    let mut multiplier = saved;
    for n in (0..ud.len()).rev() {
        let zero = if ud[n].checking {
            ud[n].exponent == 0.0
        } else {
            ud[n].int_exponent() == 0
        };
        if zero {
            multiplier *= ud[n].multiplier;
            ud.remove(n);
            cancelled = true;
        }
    }
    if cancelled || (dimensionless_present && ud.is_empty()) {
        if ud.is_empty() {
            let mut unit = Unit::dimensionless();
            unit.multiplier = multiplier;
            ud.push(unit);
        } else if !is_equal(multiplier, 1.0) {
            let first = &mut ud[0];
            first.multiplier *= pow(multiplier, 1.0 / first.exponent);
        }
    }
}

/// `UnitDefinition::reorder`: the units in the order of their kinds, units of one kind in their
/// order.
pub(crate) fn reorder(ud: &mut Definition) {
    ud.sort_by_key(|u| u.kind);
}

/// `UnitDefinition::isVariantOfDimensionless`: a definition which simplifies to one
/// dimensionless unit; an empty one is not.
pub(crate) fn is_variant_of_dimensionless(ud: &Definition) -> bool {
    if ud.is_empty() {
        return false;
    }
    let mut ud = ud.clone();
    simplify(&mut ud);
    ud.len() == 1 && ud[0].kind == DIMENSIONLESS
}

/// The SI units of a unit kind, `Unit::convertToSI` without the multipliers, which no
/// comparison of [`are_equivalent`] reads: the SI kinds with the factors of the exponent.
fn si_units(kind: usize) -> &'static [(&'static str, f64)] {
    match KINDS.get(kind).copied().unwrap_or("") {
        "ampere" => &[("ampere", 1.0)],
        "avogadro" | "dimensionless" | "radian" | "steradian" => &[("dimensionless", 1.0)],
        "becquerel" | "hertz" => &[("second", -1.0)],
        "candela" | "lumen" => &[("candela", 1.0)],
        "celsius" | "kelvin" => &[("kelvin", 1.0)],
        "coulomb" => &[("ampere", 1.0), ("second", 1.0)],
        "farad" => &[
            ("ampere", 2.0),
            ("kilogram", -1.0),
            ("metre", -2.0),
            ("second", 4.0),
        ],
        "gram" | "kilogram" => &[("kilogram", 1.0)],
        "gray" | "sievert" => &[("metre", 2.0), ("second", -2.0)],
        "henry" => &[
            ("ampere", -2.0),
            ("kilogram", 1.0),
            ("metre", 2.0),
            ("second", -2.0),
        ],
        "item" => &[("item", 1.0)],
        "joule" => &[("kilogram", 1.0), ("metre", 2.0), ("second", -2.0)],
        "katal" => &[("mole", 1.0), ("second", -1.0)],
        "liter" | "litre" => &[("metre", 3.0)],
        "lux" => &[("candela", 1.0), ("metre", -2.0)],
        "meter" | "metre" => &[("metre", 1.0)],
        "mole" => &[("mole", 1.0)],
        "newton" => &[("kilogram", 1.0), ("metre", 1.0), ("second", -2.0)],
        "ohm" => &[
            ("ampere", -2.0),
            ("kilogram", 1.0),
            ("metre", 2.0),
            ("second", -3.0),
        ],
        "pascal" => &[("kilogram", 1.0), ("metre", -1.0), ("second", -2.0)],
        "second" => &[("second", 1.0)],
        "siemens" => &[
            ("ampere", 2.0),
            ("kilogram", -1.0),
            ("metre", -2.0),
            ("second", 3.0),
        ],
        "tesla" => &[("kilogram", 1.0), ("second", -2.0), ("ampere", -1.0)],
        "volt" => &[
            ("kilogram", 1.0),
            ("metre", 2.0),
            ("second", -3.0),
            ("ampere", -1.0),
        ],
        "watt" => &[("kilogram", 1.0), ("metre", 2.0), ("second", -3.0)],
        "weber" => &[
            ("kilogram", 1.0),
            ("metre", 2.0),
            ("second", -2.0),
            ("ampere", -1.0),
        ],
        _ => &[],
    }
}

/// `UnitDefinition::convertToSI`: every unit as its SI units, simplified.
fn convert_to_si(ud: &Definition) -> Definition {
    let mut out = Vec::new();
    for unit in ud {
        // the first SI unit takes the exponent of the unit (as an integer outside the unit
        // checking), the others the exponent as a double, and are of the unit checking
        let first = if unit.checking {
            unit.exponent
        } else {
            unit.int_exponent() as f64
        };
        let parts = si_units(unit.kind);
        if parts.is_empty() {
            let mut u = unit.clone();
            u.exponent = first;
            out.push(u);
        }
        for (k, (name, factor)) in parts.iter().enumerate() {
            let single = parts.len() == 1 && *factor == 1.0;
            let exponent = if k == 0 { first } else { unit.exponent };
            out.push(Unit {
                kind: kind_of(name),
                exponent: factor * exponent,
                scale: 0,
                multiplier: 1.0,
                checking: if single { unit.checking } else { true },
            });
        }
    }
    simplify(&mut out);
    out
}

/// `Unit::areEquivalent`: the same kind and, for a kind other than dimensionless, the same
/// exponent.
fn units_equivalent(a: &Unit, b: &Unit) -> bool {
    if a.kind != b.kind {
        return false;
    }
    if a.kind == DIMENSIONLESS {
        return true;
    }
    if a.checking || b.checking {
        is_equal(a.exponent, b.exponent)
    } else {
        a.int_exponent() == b.int_exponent()
    }
}

/// `UnitDefinition::areEquivalent`: the same SI units with the same exponents, whatever the
/// multipliers.
pub(crate) fn are_equivalent(a: &Definition, b: &Definition) -> bool {
    let mut a = convert_to_si(a);
    let mut b = convert_to_si(b);
    if a.len() != b.len() {
        return false;
    }
    reorder(&mut a);
    reorder(&mut b);
    a.iter().zip(&b).all(|(x, y)| units_equivalent(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(kind: &str, exponent: f64, scale: i32, multiplier: f64) -> Unit {
        Unit {
            kind: kind_of(kind),
            exponent,
            scale,
            multiplier,
            checking: true,
        }
    }

    #[test]
    fn the_kinds_are_in_the_order_of_libsbml() {
        assert_eq!(kind_of("dimensionless"), DIMENSIONLESS);
        assert_eq!(kind_of("weber"), 35);
        assert_eq!(kind_of("foo"), INVALID);
        assert!(is_level3_kind("litre") && !is_level3_kind("liter"));
    }

    #[test]
    fn a_multiplier_is_rounded_to_15_digits() {
        assert_eq!(truncate(1.0 / 60.0), 0.016_666_666_666_666_7);
        assert_eq!(truncate(0.001), 0.001);
    }

    #[test]
    fn a_cancelled_kind_leaves_its_multiplier() {
        // second * (60 second)^-1
        let mut ud = vec![unit("second", 1.0, 0, 1.0), unit("second", -1.0, 0, 60.0)];
        simplify(&mut ud);
        assert_eq!(ud.len(), 1);
        assert_eq!(ud[0].kind, DIMENSIONLESS);
        assert_eq!(ud[0].multiplier, 0.016_666_666_666_666_7);
        // the multiplier of a cancelled kind goes to the first unit left
        let mut ud = vec![
            unit("mole", 1.0, -3, 1.0),
            unit("second", 1.0, 0, 1.0),
            unit("second", -1.0, 0, 60.0),
        ];
        simplify(&mut ud);
        assert_eq!(ud.len(), 1);
        assert_eq!(
            (ud[0].scale, ud[0].multiplier),
            (-3, 0.016_666_666_666_666_7)
        );
    }

    #[test]
    fn units_of_the_same_dimensions_are_equivalent() {
        let a = vec![unit("mole", 1.0, -3, 1.0), unit("litre", -1.0, 0, 1.0)];
        let b = vec![unit("mole", 1.0, 0, 1.0), unit("metre", -3.0, 0, 1.0)];
        let c = vec![unit("second", -1.0, 0, 60.0)];
        assert!(are_equivalent(&a, &b));
        assert!(!are_equivalent(&a, &c));
        assert!(are_equivalent(&c, &vec![unit("hertz", 1.0, 0, 1.0)]));
    }
}

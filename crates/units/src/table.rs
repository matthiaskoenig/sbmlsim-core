//! The builtin table: the prefixes and the units of pint which sbmlsim uses, with the units
//! sbmlsim adds, defined as `default_en.txt` and `constants_en.txt` of pint define them.
//!
//! A unit is defined by an expression of the units before it, whose number is the scale of the
//! unit and whose units are its reference, as pint reads a definition: `hour = 60 * minute` is
//! the scale 60 over `minute`, not 3600 seconds, so the factor of a unit to its root units is
//! the product pint computes. The order of the table is the order of the definitions, every
//! unit after the units its expression names.

/// A base dimension, an index of [`crate::DIMENSIONS`].
pub(crate) type Dim = usize;

/// The definition of a builtin unit.
pub(crate) enum Builtin {
    /// A base unit of a dimension, `None` for a dimensionless one (`count`, `radian`).
    Base(Option<Dim>),
    /// A unit of a scale over a reference, the expression of both.
    Derived(&'static str),
    /// A unit with an offset (`degC`): its scale and reference, and its offset, which no factor
    /// converts.
    Offset(&'static str, f64),
}

/// The prefixes of pint, in its order (which decides between two readings of a name, as in
/// pint): the name, its scale and its symbols and aliases.
pub(crate) const PREFIXES: &[(&str, f64, &[&str])] = &[
    ("quecto", 1e-30, &["q"]),
    ("ronto", 1e-27, &["r"]),
    ("yocto", 1e-24, &["y"]),
    ("zepto", 1e-21, &["z"]),
    ("atto", 1e-18, &["a"]),
    ("femto", 1e-15, &["f"]),
    ("pico", 1e-12, &["p"]),
    ("nano", 1e-9, &["n"]),
    ("micro", 1e-6, &["μ", "µ", "u", "mu", "mc"]),
    ("milli", 1e-3, &["m"]),
    ("centi", 1e-2, &["c"]),
    ("deci", 1e-1, &["d"]),
    ("deca", 1e+1, &["da", "deka"]),
    ("hecto", 1e2, &["h"]),
    ("kilo", 1e3, &["k"]),
    ("mega", 1e6, &["M"]),
    ("giga", 1e9, &["G"]),
    ("tera", 1e12, &["T"]),
    ("peta", 1e15, &["P"]),
    ("exa", 1e18, &["E"]),
    ("zetta", 1e21, &["Z"]),
    ("yotta", 1e24, &["Y"]),
    ("ronna", 1e27, &["R"]),
    ("quetta", 1e30, &["Q"]),
];

/// The units: the name, the definition and the symbols and aliases.
///
/// The names of pint which are a single letter and which this table does not have (`a` for a
/// year, `t` for a tonne) are left out on purpose: a name of pint which the table has not is an
/// error and never read as another unit with a prefix.
pub(crate) const UNITS: &[(&str, Builtin, &[&str])] = &[
    // the base units of pint, its root units
    ("meter", Builtin::Base(Some(0)), &["m", "metre"]),
    ("second", Builtin::Base(Some(2)), &["s", "sec"]),
    ("ampere", Builtin::Base(Some(4)), &["A", "amp"]),
    ("candela", Builtin::Base(Some(6)), &["cd", "candle"]),
    ("gram", Builtin::Base(Some(1)), &["g"]),
    ("mole", Builtin::Base(Some(3)), &["mol"]),
    (
        "kelvin",
        Builtin::Base(Some(5)),
        &["K", "degK", "°K", "degree_Kelvin", "degreeK"],
    ),
    ("radian", Builtin::Base(None), &["rad"]),
    ("count", Builtin::Base(None), &[]),
    // dimensionless
    ("steradian", Builtin::Derived("radian ** 2"), &["sr"]),
    ("ppm", Builtin::Derived("1e-6"), &[]),
    // length
    (
        "angstrom",
        Builtin::Derived("1e-10 * meter"),
        &["Å", "ångström"],
    ),
    ("micron", Builtin::Derived("micrometer"), &["μ", "µ"]),
    // mass
    (
        "atomic_mass_constant",
        Builtin::Derived("1.66053906892e-27 * kilogram"),
        &["m_u"],
    ),
    ("dalton", Builtin::Derived("atomic_mass_constant"), &["Da"]),
    // time
    ("minute", Builtin::Derived("60 * second"), &["min"]),
    ("hour", Builtin::Derived("60 * minute"), &["h", "hr"]),
    ("day", Builtin::Derived("24 * hour"), &["d"]),
    ("week", Builtin::Derived("7 * day"), &[]),
    (
        "year",
        Builtin::Derived("365.25 * day"),
        &["yr", "julian_year"],
    ),
    // temperature
    (
        "degree_Celsius",
        Builtin::Offset("kelvin", 273.15),
        &["°C", "celsius", "degC", "degreeC"],
    ),
    (
        "degree_Rankine",
        Builtin::Derived("5 / 9 * kelvin"),
        &["°R", "rankine", "degR", "degreeR"],
    ),
    (
        "degree_Fahrenheit",
        Builtin::Offset("5 / 9 * kelvin", 233.15 + 200.0 / 9.0),
        &["°F", "fahrenheit", "degF", "degreeF"],
    ),
    // volume
    (
        "liter",
        Builtin::Derived("decimeter ** 3"),
        &["l", "L", "ℓ", "litre"],
    ),
    (
        "cubic_centimeter",
        Builtin::Derived("centimeter ** 3"),
        &["cc"],
    ),
    // frequency and radiation
    ("hertz", Builtin::Derived("1 / second"), &["Hz"]),
    (
        "counts_per_second",
        Builtin::Derived("count / second"),
        &["cps"],
    ),
    // force, energy, power, pressure
    (
        "newton",
        Builtin::Derived("kilogram * meter / second ** 2"),
        &["N"],
    ),
    ("joule", Builtin::Derived("newton * meter"), &["J"]),
    (
        "calorie",
        Builtin::Derived("4.184 * joule"),
        &["cal", "thermochemical_calorie", "cal_th"],
    ),
    ("watt", Builtin::Derived("joule / second"), &["W"]),
    ("pascal", Builtin::Derived("newton / meter ** 2"), &["Pa"]),
    ("bar", Builtin::Derived("1e5 * pascal"), &[]),
    (
        "atmosphere",
        Builtin::Derived("101325 * pascal"),
        &["atm", "standard_atmosphere"],
    ),
    ("torr", Builtin::Derived("atm / 760"), &[]),
    (
        "standard_gravity",
        Builtin::Derived("9.80665 * meter / second ** 2"),
        &["g_0", "g0", "g_n", "gravity"],
    ),
    (
        "conventional_mercury_density",
        Builtin::Derived("13595.1 * kilogram / meter ** 3"),
        &["ρHg"],
    ),
    (
        "meter_Hg",
        Builtin::Derived("meter * conventional_mercury_density * g_0"),
        &["mHg", "m_Hg"],
    ),
    // concentration and catalytic activity
    ("molar", Builtin::Derived("mole / liter"), &["M"]),
    ("katal", Builtin::Derived("mole / second"), &["kat"]),
    // radiation
    ("becquerel", Builtin::Derived("counts_per_second"), &["Bq"]),
    ("gray", Builtin::Derived("joule / kilogram"), &["Gy"]),
    ("sievert", Builtin::Derived("joule / kilogram"), &["Sv"]),
    // photometry
    ("lumen", Builtin::Derived("candela * steradian"), &["lm"]),
    ("lux", Builtin::Derived("lumen / meter ** 2"), &["lx"]),
    // electromagnetism
    ("coulomb", Builtin::Derived("ampere * second"), &["C"]),
    ("volt", Builtin::Derived("joule / coulomb"), &["V"]),
    ("ohm", Builtin::Derived("volt / ampere"), &["Ω"]),
    ("siemens", Builtin::Derived("ampere / volt"), &["S", "mho"]),
    ("farad", Builtin::Derived("coulomb / volt"), &["F"]),
    ("weber", Builtin::Derived("volt * second"), &["Wb"]),
    ("henry", Builtin::Derived("weber / ampere"), &["H"]),
    ("tesla", Builtin::Derived("weber / meter ** 2"), &["T"]),
    // the units sbmlsim adds (`sbmlsim.units`): counts, and percent as a count
    ("none", Builtin::Derived("count"), &[]),
    ("item", Builtin::Derived("count"), &[]),
    ("percent", Builtin::Derived("0.01 * count"), &[]),
];

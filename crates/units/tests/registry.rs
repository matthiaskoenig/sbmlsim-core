//! The errors and the definitions of the registry.

use std::collections::BTreeMap;

use sbmlsim_units::{Registry, UnitError};

#[test]
fn refuses_offset_units() {
    let registry = Registry::new();
    for (from, to) in [
        ("degC", "K"),
        ("K", "degC"),
        ("degF", "degC"),
        ("mdegC", "K"),
    ] {
        let err = registry.factor(from, to).unwrap_err();
        assert!(
            matches!(err, UnitError::Offset { .. }),
            "{from} {to}: {err}"
        );
    }
    let err = registry.parse("degC/min").unwrap_err();
    assert_eq!(
        err,
        UnitError::Offset {
            expression: "degC/min".to_owned(),
            unit: "degree_Celsius".to_owned(),
        }
    );
    assert!(err.to_string().contains("degree_Celsius"), "{err}");
    // a temperature without an offset converts
    assert_eq!(registry.factor("K", "mK").unwrap(), 1000.0);
}

#[test]
fn dimension_mismatch_names_both() {
    let registry = Registry::new();
    let err = registry.factor("mg", "ml").unwrap_err();
    assert_eq!(
        err,
        UnitError::Dimension {
            from: "mg".to_owned(),
            to: "ml".to_owned(),
            from_dimensions: "[mass]".to_owned(),
            to_dimensions: "[length] ** 3".to_owned(),
        }
    );
    assert_eq!(
        err.to_string(),
        "cannot convert 'mg' ([mass]) to 'ml' ([length] ** 3)"
    );
    let mut values = [1.0, 2.0];
    assert!(registry.convert(&mut values, "mg/l", "mM").is_err());
    assert_eq!(values, [1.0, 2.0]);
}

#[test]
fn define_refuses_other_value() {
    let mut registry = Registry::new();
    registry.define("IU_insulin", "0.0347 * mg").unwrap();
    // the same value again changes nothing
    registry.define("IU_insulin", "0.0347 * mg").unwrap();
    let err = registry.define("IU_insulin", "0.045 * mg").unwrap_err();
    assert_eq!(
        err,
        UnitError::Redefinition {
            name: "IU_insulin".to_owned(),
            existing: "0.0347 * mg".to_owned(),
            expression: "0.045 * mg".to_owned(),
        }
    );
    // a builtin, and a name read with a prefix, are units already
    assert!(matches!(
        registry.define("hour", "3000 * s"),
        Err(UnitError::Redefinition { .. })
    ));
    registry.define("mg", "0.001 * gram").unwrap();
    let err = registry.define("mg", "gram").unwrap_err();
    assert!(err.to_string().contains("milligram"), "{err}");
    assert!(matches!(
        registry.define("2x", "mg"),
        Err(UnitError::Name { .. })
    ));
    assert_eq!(registry.factor("IU_insulin", "ug").unwrap(), 34.7);
    assert_eq!(
        registry.parse("IU_insulin/ml").unwrap().dims,
        registry.parse("mg/ml").unwrap().dims
    );
}

#[test]
fn model_unit_ids() {
    let registry = Registry::new();
    let unit_ids = BTreeMap::from([
        ("mg_per_min".to_owned(), "mg/min".to_owned()),
        ("per_hr".to_owned(), "1/hr".to_owned()),
        ("dimless".to_owned(), String::new()),
        // a unit id wins over a unit of the registry of the same name
        ("g".to_owned(), "kg".to_owned()),
    ]);
    let units = BTreeMap::from([("CL".to_owned(), "mg_per_min/kg".to_owned())]);
    let model = registry.with_model_units(&units, &unit_ids);
    assert_eq!(model.unit_of("CL"), Some("mg_per_min/kg"));
    assert_eq!(model.unit_of("x"), None);
    assert_eq!(
        model.parse("mg_per_min/kg").unwrap(),
        registry.parse("mg/min/kg").unwrap()
    );
    // the expression as a whole: a power applies to all of it
    assert_eq!(
        model.parse("mg_per_min^2").unwrap(),
        registry.parse("(mg/min)^2").unwrap()
    );
    assert_eq!(
        model.factor("per_hr", "1/min").unwrap(),
        registry.factor("1/hr", "1/min").unwrap()
    );
    assert_eq!(model.parse("dimless").unwrap().factor, 1.0);
    assert_eq!(model.factor("g", "kg").unwrap(), 1.0);
    // the base registry knows no unit id
    assert!(matches!(
        registry.parse("mg_per_min/kg"),
        Err(UnitError::Undefined { .. })
    ));
}

#[test]
fn fractional_powers_are_refused() {
    let registry = Registry::new();
    assert!(matches!(
        registry.parse("m**0.5"),
        Err(UnitError::FractionalPower { .. })
    ));
    // a number may have any power, a unit an integer one
    assert_eq!(registry.parse("4**0.5 * m").unwrap().factor, 2.0);
    assert_eq!(registry.parse("m**2.0").unwrap().dims[0], 2);
}

#[test]
fn syntax_and_undefined_name_the_expression() {
    let registry = Registry::new();
    let err = registry.parse("mg/(min").unwrap_err();
    assert!(matches!(err, UnitError::Syntax { .. }), "{err}");
    assert!(err.to_string().contains("mg/(min"), "{err}");
    assert_eq!(
        registry.parse("mg/foo").unwrap_err(),
        UnitError::Undefined {
            expression: "mg/foo".to_owned(),
            name: "foo".to_owned(),
        }
    );
    assert!(registry.parse("0*m").is_err());
    assert!(registry.parse("m/0").is_err());
    // a factor or a power out of range is an error, never an infinity or a wrapped exponent
    assert!(registry.parse("Ym**30").is_err());
    assert!(registry.factor("ym", "Ym**30").is_err());
    assert!(registry.parse("((m**30000)**30000)**30000").is_err());
}

#[test]
fn units_named_as_pint_formats_them() {
    let r = sbmlsim_units::Registry::new();
    let cases = [
        ("mmol/l", "millimole / liter"),
        ("mol/m^3", "mole / meter ** 3"),
        ("1/(min*m)", "1 / meter / minute"),
        ("mol/(s*Hz*A)", "mole / ampere / hertz / second"),
        ("m^2*s/kg", "meter ** 2 * second / kilogram"),
        ("mmol/l*l/mmol", "dimensionless"),
        ("percent", "percent"),
        ("item/fl", "item / femtoliter"),
        ("M", "molar"),
        ("uM/min", "micromolar / minute"),
        ("hr", "hour"),
        ("", "dimensionless"),
        ("ng/ml", "nanogram / milliliter"),
    ];
    for (expression, text) in cases {
        assert_eq!(
            r.named(expression).unwrap().to_string(),
            text,
            "{expression}"
        );
    }
    let product = r
        .named("mmol/litre")
        .unwrap()
        .times(&r.named("g/mol").unwrap());
    assert_eq!(product.to_string(), "gram * millimole / liter / mole");
    assert_eq!(
        r.named("min").unwrap().power(0.5).to_string(),
        "minute ** 0.5"
    );
    assert_eq!(r.named("m^2").unwrap().power(0.5).to_string(), "meter");
    assert_eq!(r.named("0.001*mole").unwrap().magnitude, 0.001);
}

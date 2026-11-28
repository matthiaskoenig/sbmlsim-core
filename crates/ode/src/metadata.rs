//! Authored identifiers, names and unit text retained beside mathematical lowering.
use sbmlsim_document::{Element, Model, Unit};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct Symbol {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub kind: &'static str,
    pub source: Vec<String>,
}
pub(crate) fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}
fn general(x: f64) -> String {
    let text = sbmlsim_document::formatting::python_float(x);
    text.strip_suffix(".0").unwrap_or(&text).to_owned()
}
/// The value to 15 significant digits, which drops the noise of the division by the
/// prefix (6e-05 s is 60 μs, not 60.00000000000001 μs) and keeps every digit of a
/// multiplier, which a double holds to 15 to 17 digits.
fn significant(x: f64) -> f64 {
    if x.is_finite() && x != 0.0 {
        format!("{x:.14e}").parse().unwrap_or(x)
    } else {
        x
    }
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8 + 1e-5 * b.abs()
}
fn term(mut factor: f64, kind: &str) -> String {
    for (k, n, text) in [
        ("second", 60.0, "min"),
        ("second", 3600.0, "hr"),
        ("second", 86400.0, "day"),
        ("metre", 0.01, "cm"),
    ] {
        if k == kind && close(factor, n) {
            return text.into();
        }
    }
    let symbol = match kind {
        "ampere" => "A",
        "avogadro" => "avogadro",
        "becquerel" => "Bq",
        "candela" => "cd",
        "coulomb" => "C",
        "dimensionless" => "",
        "farad" => "F",
        "gram" | "kilogram" => "g",
        "gray" => "Gy",
        "henry" => "H",
        "hertz" => "Hz",
        "item" => "item",
        "joule" => "J",
        "katal" => "kat",
        "kelvin" => "K",
        "liter" | "litre" => "l",
        "lumen" => "lm",
        "lux" => "lx",
        "meter" | "metre" => "m",
        "mole" => "mol",
        "newton" => "N",
        "ohm" => "Ω",
        "pascal" => "Pa",
        "radian" => "rad",
        "second" => "s",
        "siemens" => "S",
        "sievert" => "Sv",
        "steradian" => "sr",
        "tesla" => "T",
        "volt" => "V",
        "watt" => "W",
        "weber" => "Wb",
        "Celsius" => "°C",
        _ => kind,
    };
    if kind == "kilogram" {
        factor *= 1000.0;
    }
    let mut magnitude = significant(factor);
    let mut prefix = "";
    if !matches!(kind, "dimensionless" | "item" | "avogadro") && factor > 0.0 && factor.is_finite()
    {
        let power = (3.0 * (factor.log10() / 3.0).floor()).clamp(-24.0, 24.0) as i32;
        magnitude = significant(factor / 10.0f64.powi(power));
        prefix = match power {
            -24 => "y",
            -21 => "z",
            -18 => "a",
            -15 => "f",
            -12 => "p",
            -9 => "n",
            -6 => "μ",
            -3 => "m",
            3 => "k",
            6 => "M",
            9 => "G",
            12 => "T",
            15 => "P",
            18 => "E",
            21 => "Z",
            24 => "Y",
            _ => "",
        };
    }
    if close(magnitude, 1.0) {
        format!("{prefix}{symbol}")
    } else if symbol.is_empty() {
        general(magnitude)
    } else {
        format!("{} {prefix}{symbol}", general(magnitude))
    }
}
fn group(text: String) -> String {
    if text.contains(' ') && !(text.starts_with('(') && text.ends_with(')')) {
        format!("({text})")
    } else {
        text
    }
}
fn definition(units: &[Unit]) -> String {
    let (mut numerator, mut denominator) = (vec![], vec![]);
    for u in units {
        let exponent = u.exponent.unwrap_or(1.0);
        let mut text = term(
            u.multiplier.unwrap_or(1.0) * 10.0f64.powi(u.scale.unwrap_or(0)),
            u.kind.as_deref().unwrap_or("invalid"),
        );
        if text.is_empty() || exponent == 0.0 {
            continue;
        }
        if exponent.abs() != 1.0 {
            text = format!("{}^{}", group(text), general(exponent.abs()));
        }
        if exponent > 0.0 {
            numerator.push(text)
        } else {
            denominator.push(text)
        }
    }
    if numerator.len() > 1 || !denominator.is_empty() {
        numerator = numerator.into_iter().map(group).collect();
        denominator = denominator.into_iter().map(group).collect();
    }
    let (n, d) = (numerator.join("*"), denominator.join("/"));
    match (n.is_empty(), d.is_empty()) {
        (true, true) => "-".into(),
        (true, false) => format!("1/{d}"),
        (false, true) => n,
        _ => format!("{n}/{d}"),
    }
}
pub(crate) fn unit(id: Option<&str>, model: &Model) -> Option<String> {
    let id = id.filter(|id| !id.is_empty())?;
    if let Some(definition_) = model
        .unit_definitions
        .iter()
        .find(|u| u.element.id.as_deref() == Some(id))
    {
        return Some(definition(&definition_.units));
    }
    Some(
        match id {
            "metre" | "meter" => "m",
            "liter" | "litre" => "l",
            "dimensionless" => "-",
            "second" => "s",
            "ampere" | "avogadro" | "becquerel" | "candela" | "coulomb" | "farad" | "gram"
            | "gray" | "henry" | "hertz" | "item" | "joule" | "katal" | "kelvin" | "kilogram"
            | "lumen" | "lux" | "mole" | "newton" | "ohm" | "pascal" | "radian" | "siemens"
            | "sievert" | "steradian" | "tesla" | "volt" | "watt" | "weber" | "Celsius" => id,
            _ => "-",
        }
        .to_owned(),
    )
}
pub(crate) fn compartment(id: &str, model: &Model) -> Option<String> {
    let c = model
        .compartments
        .iter()
        .find(|c| c.element.id.as_deref() == Some(id))?;
    let fallback = match c.spatial_dimensions.unwrap_or(3.0) {
        3.0 => model.units.volume.as_deref(),
        2.0 => model.units.area.as_deref(),
        1.0 => model.units.length.as_deref(),
        _ => None,
    };
    unit(c.units.as_deref().or(fallback), model)
}
pub(crate) fn quotient(a: Option<String>, b: Option<String>) -> Option<String> {
    let (a, b) = (a?, b?);
    let single = b
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '^' | '-' | '.'));
    Some(format!("{a}/{}", if single { b } else { format!("({b})") }))
}
pub(crate) fn symbol(element: &Element, unit: Option<String>, kind: &'static str) -> Symbol {
    Symbol {
        name: element.name.as_deref().map(single_line),
        unit,
        kind,
        source: element.id.iter().cloned().collect(),
    }
}
pub(crate) fn symbols(model: &Model) -> HashMap<String, Symbol> {
    let mut out = HashMap::new();
    let mut add = |e: &Element, u: Option<String>, kind| {
        if let Some(id) = &e.id {
            out.insert(id.clone(), symbol(e, u, kind));
        }
    };
    for c in &model.compartments {
        add(
            &c.element,
            compartment(c.element.id.as_deref().unwrap_or(""), model),
            "compartment",
        );
    }
    for s in &model.species {
        let substance = unit(
            s.substance_units
                .as_deref()
                .or(model.units.substance.as_deref()),
            model,
        );
        let u = if s.has_only_substance_units.unwrap_or(false) {
            substance
        } else {
            quotient(substance, compartment(&s.compartment, model))
        };
        add(&s.element, u, "species");
    }
    for p in &model.parameters {
        add(&p.element, unit(p.units.as_deref(), model), "parameter");
    }
    for r in &model.reactions {
        add(
            &r.element,
            quotient(
                unit(model.units.extent.as_deref(), model),
                unit(model.units.time.as_deref(), model),
            ),
            "reaction",
        );
        for s in r.reactants.iter().chain(&r.products) {
            add(&s.element, None, "species_reference");
        }
    }
    for f in &model.function_definitions {
        add(&f.element, None, "function");
    }
    for e in &model.events {
        add(&e.element, None, "event");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::term;

    #[test]
    fn a_unit_factor_prints_without_the_noise_of_its_prefix() {
        assert_eq!(term(6e-05, "second"), "60 μs");
        assert_eq!(term(0.6, "dimensionless"), "0.6");
        assert_eq!(term(6.0 * 10.0f64.powi(-1), "dimensionless"), "0.6");
        assert_eq!(term(133.32239, "newton"), "133.32239 N");
        assert_eq!(term(1.0 / 3.0, "litre"), "333.333333333333 ml");
        assert_eq!(term(1e-3, "litre"), "ml");
    }
}

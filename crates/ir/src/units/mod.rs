//! The units of the entities and selections of a model as sbmlsim names them, text a unit
//! registry such as pint parses; nothing is computed with them.
//!
//! It is the port of `UnitsInformation.from_sbml_doc` of `sbmlsim.units` (sbmlsim 0.54) on
//! the SBML of Level 3 sbmlsim hands to the engine, which names a unit id by `model_uid_dict`
//! and a unit definition by `Units.udef_to_str`, and of the unit libsbml 5.21.2 derives for a
//! kinetic law ([`formula`]), which sbmlsim names by the first unit definition of the model of
//! the same unit (`unit_signature`) or else writes itself. The map has the key `time`, the
//! time unit of the model (`second` without one), every compartment, species (its amount and
//! its concentration `[S]`) and parameter (empty without units), every reaction with a kinetic
//! law and every kinetic law with an id; an entity of the id `time` replaces the time unit, as
//! in sbmlsim. `testdata/model_units` pins it against sbmlsim.
//!
//! A unit definition whose expression is no unit of pint (a factor which is no prefix, e.g.
//! `(133.322*newton)/m^2`) is named by its id, or `<id>_<n>` where pint has the id as another
//! unit, as `UnitsInformation._define` names it in a new process, and the description carries such a
//! name with its expression (`unit_definitions` of `model/2`) so a registry reads it; sbmlsim
//! defines it in the registry of its process, so a model it reads after another one with a different unit of
//! that id names it otherwise, which a description cannot know.
//!
//! sbmlsim converts a document of an older level to Level 3 with libsbml before it reads the
//! units; the reader reads such a document into Level 3 itself and states the built in units
//! of the model, of a species (`substance`) and of a compartment (by its dimensions) as that
//! conversion does, but names some units close to but not as it does (the time `s` where
//! sbmlsim has `second`), so the units of an older level are pinned by the fixtures of the
//! scan (`testdata/scan`, the repressilator of Level 2) only.

mod algebra;
mod formula;
mod text;

use std::collections::BTreeMap;

use sbmlsim_sbml::Model;
use sbmlsim_units::Registry;

use algebra::{DIMENSIONLESS, Definition, Unit, kind_of, pow, reorder};
use formula::Formatter;
pub(crate) use text::quotient;
use text::{isclose, udef_to_str};

/// The predefined units of SBML Level 2, which `model_uid_dict` names for every level.
const PREDEFINED: [(&str, &str); 5] = [
    ("substance", "mole"),
    ("volume", "litre"),
    ("area", "meter^2"),
    ("length", "meter"),
    ("time", "second"),
];

/// The id of a unit, empty without one.
fn id(uid: Option<&String>) -> &str {
    uid.map_or("", String::as_str)
}

/// The units sbmlsim gives the entities of a model, see the module, and the unit definitions
/// named by their id because pint has no unit of them (name to expression).
pub(crate) fn units_of(model: &Model) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let definitions = definitions(model);
    let (names, defined) = unit_names(&definitions);
    let expression = |uid: &str| -> String {
        if let Some(name) = names.get(uid) {
            return name.clone();
        }
        if uid.is_empty() {
            return "dimensionless".into();
        }
        PREDEFINED
            .iter()
            .find(|(id, _)| *id == uid)
            .map_or_else(|| uid.to_string(), |(_, e)| (*e).to_string())
    };
    let of = |uid: &str| {
        if uid.is_empty() {
            String::new()
        } else {
            expression(uid)
        }
    };
    let mut units = BTreeMap::new();
    let time = match model.time_units.as_deref() {
        Some(uid) if !uid.is_empty() => expression(uid),
        _ => "second".to_string(),
    };
    units.insert("time".to_string(), time);
    // the entities in the order of `getAllElementIdList`; a later one of an id replaces an
    // earlier one
    for c in &model.compartments {
        units.insert(c.id.clone(), of(id(c.units.as_ref())));
    }
    for s in &model.species {
        let substance = id(s.substance_units.as_ref());
        let volume = model
            .compartments
            .iter()
            .find(|c| c.id == s.compartment)
            .map_or("", |c| id(c.units.as_ref()));
        let concentration = if substance.is_empty() || volume.is_empty() {
            String::new()
        } else {
            quotient(&expression(substance), &expression(volume))
        };
        units.insert(s.id.clone(), of(substance));
        units.insert(format!("[{}]", s.id), concentration);
    }
    for p in &model.parameters {
        units.insert(p.id.clone(), of(id(p.units.as_ref())));
    }
    let signatures: Vec<(&str, Signature)> = definitions
        .iter()
        .map(|(uid, ud)| (uid.as_str(), signature(ud)))
        .collect();
    for r in &model.reactions {
        let Some(law) = &r.kinetic_law else { continue };
        let Some(math) = &law.math else { continue };
        let ud = Formatter::new(model, &definitions, &law.local_parameters).of(math);
        let unit = match matching(&ud, &signatures) {
            Some(uid) => expression(uid),
            None => udef_to_str(&ud),
        };
        if let Some(id) = &law.id {
            units.insert(id.clone(), unit.clone());
        }
        units.insert(r.id.clone(), unit);
    }
    (units, defined)
}

/// The unit definitions of the model with their units in the order of their kinds, in which
/// sbmlsim leaves them.
fn definitions(model: &Model) -> Vec<(String, Definition)> {
    model
        .unit_definitions
        .iter()
        .map(|d| {
            let mut ud: Definition = d
                .units
                .iter()
                .map(|u| Unit {
                    kind: kind_of(&u.kind),
                    exponent: u.exponent,
                    scale: u.scale,
                    multiplier: u.multiplier,
                    checking: false,
                })
                .collect();
            reorder(&mut ud);
            (d.id.clone(), ud)
        })
        .collect()
}

/// The names of the unit definitions, `model_uid_dict`: the expression of a definition, or the
/// name `UnitsInformation._define` gives a definition which is no unit of pint; and the names
/// it defines with their expressions, which a registry needs to read them.
fn unit_names(
    definitions: &[(String, Definition)],
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut registry = Registry::new();
    let mut names = BTreeMap::new();
    let mut defined_names = BTreeMap::new();
    for (uid, ud) in definitions {
        let expression = udef_to_str(ud);
        let name = if text::is_pint_unit(&expression) {
            expression
        } else {
            let (name, new) = defined(&mut registry, uid, &expression);
            if new {
                defined_names.insert(name.clone(), expression);
            }
            name
        };
        names.insert(uid.clone(), name);
    }
    (names, defined_names)
}

/// `UnitsInformation._define`: the id if the registry does not have it (which defines it) or
/// has it as the same unit, else `<id>_<n>` with the first `n` from 2 which is free or the same
/// unit; and whether the name is new to the registry, i.e. defined by this call.
fn defined(registry: &mut Registry, uid: &str, expression: &str) -> (String, bool) {
    let Ok(unit) = registry.parse(expression) else {
        return (uid.to_string(), false);
    };
    let base = |u: &sbmlsim_units::Unit| {
        // the magnitude in the base units of pint, the kilogram for the root unit gram
        let mass = sbmlsim_units::DIMENSIONS
            .iter()
            .position(|d| *d == "mass")
            .map_or(0, |k| u.dims[k]);
        u.factor * pow(1000.0, -f64::from(mass))
    };
    let mut name = uid.to_string();
    let mut n = 1;
    loop {
        match registry.parse(&name) {
            Err(_) => {
                // `define` refuses no free name
                let new = registry.define(&name, expression).is_ok();
                return (name, new);
            }
            Ok(existing) => {
                if isclose(base(&existing), base(&unit), 1e-5, 1e-8) && existing.dims == unit.dims {
                    return (name, false);
                }
            }
        }
        n += 1;
        name = format!("{uid}_{n}");
    }
}

/// `unit_signature` of sbmlsim: the kinds with their summed exponents (dimensionless left out,
/// a kind whose exponents cancel left out) and the factor of the multipliers and scales.
type Signature = (Vec<(usize, f64)>, f64);

fn signature(ud: &Definition) -> Signature {
    let mut exponents: Vec<(usize, f64)> = Vec::new();
    let mut factor = 1.0;
    for u in ud {
        factor *= pow(u.multiplier * pow(10.0, f64::from(u.scale)), u.exponent);
        if u.kind == DIMENSIONLESS {
            continue;
        }
        match exponents.iter_mut().find(|(k, _)| *k == u.kind) {
            Some((_, e)) => *e += u.exponent,
            None => exponents.push((u.kind, u.exponent)),
        }
    }
    exponents.retain(|(_, e)| *e != 0.0);
    exponents.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    (exponents, factor)
}

/// `_matching_unit_definition`: the id of the first unit definition of the same signature, the
/// factors within a relative 1e-12.
fn matching<'a>(ud: &Definition, signatures: &[(&'a str, Signature)]) -> Option<&'a str> {
    let (kinds, factor) = signature(ud);
    signatures
        .iter()
        .find(|(_, (other_kinds, other_factor))| {
            *other_kinds == kinds && isclose(*other_factor, factor, 1e-12, 0.0)
        })
        .map(|(uid, _)| *uid)
}

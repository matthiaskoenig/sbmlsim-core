//! The data model of an SBML core document and its reader, every level and version.
//!
//! A document of Level 1 or Level 2 is read into the same model as one of Level 3, so a
//! consumer sees one shape whatever the document says: the built in units `substance`,
//! `volume`, `area`, `length` and `time` of the older levels become unit definitions and the
//! default units of the model, the `<stoichiometryMath>` of a species reference becomes its
//! stoichiometry or an assignment rule of its id, and an attribute a level does not have is
//! read as that level means it (a reaction of Level 1 and 2 is reversible, its stoichiometry
//! is 1, a compartment and a parameter are constant, the assignments of an event take the
//! values of the trigger time).

use crate::formula;
use crate::math::{self, Node};
use crate::{Document, ReadError};
use roxmltree::Node as X;

/// The namespace of SBML Level 3 Version 2 core.
pub const SBML_NS_L3V2: &str = "http://www.sbml.org/sbml/level3/version2/core";

/// The namespace of SBML Level 3 Version 1 core.
pub const SBML_NS_L3V1: &str = "http://www.sbml.org/sbml/level3/version1/core";

/// The namespace of every level and version of SBML core, with the level and the version it
/// names. A namespace which names no version (Level 1, and Level 2 Version 1, whose namespace
/// carries the level alone) has `None` and takes the version from the attribute of `<sbml>`.
const NAMESPACES: [(&str, u32, Option<u32>); 8] = [
    (SBML_NS_L3V2, 3, Some(2)),
    (SBML_NS_L3V1, 3, Some(1)),
    ("http://www.sbml.org/sbml/level2/version5", 2, Some(5)),
    ("http://www.sbml.org/sbml/level2/version4", 2, Some(4)),
    ("http://www.sbml.org/sbml/level2/version3", 2, Some(3)),
    ("http://www.sbml.org/sbml/level2/version2", 2, Some(2)),
    // Level 2 Version 1 and Level 1 (Version 1 and Version 2) name no version in the namespace
    ("http://www.sbml.org/sbml/level2", 2, Some(1)),
    ("http://www.sbml.org/sbml/level1", 1, None),
];

/// Whether `ns` is the namespace of a level and version of SBML core.
pub fn is_sbml_ns(ns: &str) -> bool {
    NAMESPACES.iter().any(|(uri, _, _)| *uri == ns)
}

/// The level and version of a document, which the readers of its elements follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// The level.
    pub level: u32,
    /// The version of the level.
    pub version: u32,
}

impl Level {
    /// Whether the document is of Level 3, which states every attribute the older levels
    /// default and carries the units of the model itself.
    const fn is_level_3(self) -> bool {
        self.level == 3
    }

    /// Whether the document is of Level 1, which names an element by its `name` and holds its
    /// mathematics as an infix formula instead of MathML.
    const fn is_level_1(self) -> bool {
        self.level == 1
    }
}

/// One unit of a unit definition, as the document writes it: the kind of the unit and the
/// three factors of `multiplier * 10^scale * kind^exponent`.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// The kind, a base unit of SBML or, for new base units of a model which the levels
    /// before 3 allow, the id of the definition itself.
    pub kind: String,
    /// The exponent, 1 without one.
    pub exponent: f64,
    /// The scale, the power of ten, 0 without one.
    pub scale: i32,
    /// The multiplier, 1 without one.
    pub multiplier: f64,
}

/// A unit definition as a unit expression, e.g. `0.001*mole*(60*second)**-1`, and as the units
/// the document writes it with, which a conversion of the units needs.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitDefinition {
    /// The id.
    pub id: String,
    /// The expression a unit registry parses.
    pub expression: String,
    /// The units of the definition, in the order of the document; empty for a definition
    /// without a unit, whose expression is `dimensionless`.
    pub units: Vec<Unit>,
    /// Whether the reader defines the units itself, which is true for the units every level
    /// before Level 3 has built in and which the model does not state (`substance`, `volume`,
    /// `area`, `length` and `time`); a conversion of the model writes what the model states,
    /// so it tells the two apart.
    pub built_in: bool,
}

/// A function definition, `math` is its lambda.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDefinition {
    /// The id.
    pub id: String,
    /// The lambda, absent when the definition has no math.
    pub math: Option<Node>,
}

/// A compartment.
#[derive(Debug, Clone, PartialEq)]
pub struct Compartment {
    /// The id.
    pub id: String,
    /// The spatial dimensions.
    pub spatial_dimensions: Option<f64>,
    /// The size.
    pub size: Option<f64>,
    /// The units.
    pub units: Option<String>,
    /// Whether the size is constant.
    pub constant: bool,
}

/// A species.
#[derive(Debug, Clone, PartialEq)]
pub struct Species {
    /// The id.
    pub id: String,
    /// The compartment.
    pub compartment: String,
    /// The initial amount.
    pub initial_amount: Option<f64>,
    /// The initial concentration.
    pub initial_concentration: Option<f64>,
    /// The substance units.
    pub substance_units: Option<String>,
    /// Whether the species is an amount.
    pub has_only_substance_units: bool,
    /// Whether the species is a boundary condition.
    pub boundary_condition: bool,
    /// Whether the species is constant.
    pub constant: bool,
    /// The id of the conversion factor.
    pub conversion_factor: Option<String>,
}

/// A parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// The id.
    pub id: String,
    /// The value.
    pub value: Option<f64>,
    /// The units.
    pub units: Option<String>,
    /// Whether the value is constant.
    pub constant: bool,
}

/// A parameter local to a kinetic law.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalParameter {
    /// The id.
    pub id: String,
    /// The value.
    pub value: Option<f64>,
    /// The units.
    pub units: Option<String>,
}

/// An initial assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct InitialAssignment {
    /// The id.
    pub id: Option<String>,
    /// The assigned symbol.
    pub symbol: String,
    /// The math, absent when the element has no `<math>`.
    pub math: Option<Node>,
}

/// A rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// `variable = math`.
    Assignment {
        /// The id.
        id: Option<String>,
        /// The variable.
        variable: String,
        /// The math, absent when the element has no `<math>`.
        math: Option<Node>,
    },
    /// `d variable / dt = math`.
    Rate {
        /// The id.
        id: Option<String>,
        /// The variable.
        variable: String,
        /// The math, absent when the element has no `<math>`.
        math: Option<Node>,
    },
    /// `0 = math`.
    Algebraic {
        /// The id.
        id: Option<String>,
        /// The math, absent when the element has no `<math>`.
        math: Option<Node>,
    },
}

impl Rule {
    /// The math of the rule, absent when the element has no `<math>`.
    pub fn math(&self) -> Option<&Node> {
        match self {
            Rule::Assignment { math, .. }
            | Rule::Rate { math, .. }
            | Rule::Algebraic { math, .. } => math.as_ref(),
        }
    }
}

/// A species reference of a reactant or a product.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesReference {
    /// The species.
    pub species: String,
    /// The id of the reference.
    pub id: Option<String>,
    /// The stoichiometry.
    pub stoichiometry: Option<f64>,
    /// Whether the stoichiometry is constant.
    pub constant: bool,
}

/// A modifier of a reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct ModifierSpeciesReference {
    /// The species.
    pub species: String,
    /// The id of the reference.
    pub id: Option<String>,
}

/// A kinetic law.
#[derive(Debug, Clone, PartialEq)]
pub struct KineticLaw {
    /// The id.
    pub id: Option<String>,
    /// The math.
    pub math: Option<Node>,
    /// The local parameters.
    pub local_parameters: Vec<LocalParameter>,
}

/// A reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct Reaction {
    /// The id.
    pub id: String,
    /// Whether the reaction is reversible.
    pub reversible: bool,
    /// Whether the reaction is fast.
    pub fast: bool,
    /// The compartment.
    pub compartment: Option<String>,
    /// The reactants.
    pub reactants: Vec<SpeciesReference>,
    /// The products.
    pub products: Vec<SpeciesReference>,
    /// The modifiers.
    pub modifiers: Vec<ModifierSpeciesReference>,
    /// The kinetic law.
    pub kinetic_law: Option<KineticLaw>,
}

/// The trigger of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    /// The id.
    pub id: Option<String>,
    /// The value of the trigger before the time 0.
    pub initial_value: bool,
    /// Whether the trigger is persistent.
    pub persistent: bool,
    /// The math, absent when the element has no `<math>`.
    pub math: Option<Node>,
}

/// An assignment of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventAssignment {
    /// The id.
    pub id: Option<String>,
    /// The variable.
    pub variable: String,
    /// The math, absent when the element has no `<math>`.
    pub math: Option<Node>,
}

/// An event.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// The id, absent for an event named by its position.
    pub id: Option<String>,
    /// Whether the assignments use the values at the time of the trigger (default true).
    pub use_values_from_trigger_time: bool,
    /// The trigger, absent when the event has no `<trigger>`.
    pub trigger: Option<Trigger>,
    /// The id of the delay.
    pub delay_id: Option<String>,
    /// The delay, absent without a `<delay>` or when it has no `<math>`.
    pub delay: Option<Node>,
    /// The id of the priority.
    pub priority_id: Option<String>,
    /// The priority, absent without a `<priority>` or when it has no `<math>`.
    pub priority: Option<Node>,
    /// The assignments.
    pub assignments: Vec<EventAssignment>,
}

/// A model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// The id.
    pub id: Option<String>,
    /// The default substance units.
    pub substance_units: Option<String>,
    /// The default time units.
    pub time_units: Option<String>,
    /// The default extent units.
    pub extent_units: Option<String>,
    /// The default volume units.
    pub volume_units: Option<String>,
    /// The default area units.
    pub area_units: Option<String>,
    /// The default length units.
    pub length_units: Option<String>,
    /// The id of the conversion factor.
    pub conversion_factor: Option<String>,
    /// The unit definitions.
    pub unit_definitions: Vec<UnitDefinition>,
    /// The function definitions.
    pub function_definitions: Vec<FunctionDefinition>,
    /// The compartments.
    pub compartments: Vec<Compartment>,
    /// The species.
    pub species: Vec<Species>,
    /// The parameters.
    pub parameters: Vec<Parameter>,
    /// The initial assignments.
    pub initial_assignments: Vec<InitialAssignment>,
    /// The rules.
    pub rules: Vec<Rule>,
    /// The reactions.
    pub reactions: Vec<Reaction>,
    /// The events.
    pub events: Vec<Event>,
}

fn err(message: String) -> ReadError {
    ReadError::Xml(message)
}

fn is_core(n: &X, name: &str) -> bool {
    n.is_element()
        && n.tag_name().name() == name
        && n.tag_name().namespace().is_some_and(is_sbml_ns)
}

fn child<'a, 'i>(n: X<'a, 'i>, name: &str) -> Option<X<'a, 'i>> {
    n.children().find(|c| is_core(c, name))
}

/// The items of `<listOf...>` named `list`, empty when the list is absent.
fn items<'a, 'i>(n: X<'a, 'i>, list: &str, item: &str) -> Vec<X<'a, 'i>> {
    child(n, list)
        .map(|l| l.children().filter(|c| is_core(c, item)).collect())
        .unwrap_or_default()
}

impl Level {
    /// The element of a species and the attribute which names one, `<specie>` and `specie` in
    /// Level 1 Version 1, which Version 2 renamed.
    const fn species_names(self) -> (&'static str, &'static str) {
        if self.level == 1 && self.version == 1 {
            ("specie", "specie")
        } else {
            ("species", "species")
        }
    }

    /// The element of a species reference, `<specieReference>` in Level 1 Version 1.
    const fn species_reference_name(self) -> &'static str {
        if self.level == 1 && self.version == 1 {
            "specieReference"
        } else {
            "speciesReference"
        }
    }
}

/// The identifier of an element: its `name` in Level 1, its `id` in every later level.
fn sid(n: X, lv: Level) -> Result<String, ReadError> {
    req(n, if lv.is_level_1() { "name" } else { "id" })
}

/// The identifier of an element which may have none.
fn sid_opt(n: X, lv: Level) -> Option<String> {
    opt(n, if lv.is_level_1() { "name" } else { "id" })
}

/// The mathematics of an element: the `formula` attribute in Level 1, the `<math>` child in
/// every later level, absent when the element carries neither.
fn math_or_formula(n: X, lv: Level) -> Result<Option<Node>, ReadError> {
    if !lv.is_level_1() {
        return math_of(n);
    }
    let Some(text) = n.attribute("formula") else {
        return Ok(None);
    };
    formula::parse(text)
        .map(Some)
        .map_err(|message| err(format!("<{}> formula: {message}", n.tag_name().name())))
}

fn req(n: X, name: &str) -> Result<String, ReadError> {
    n.attribute(name).map(str::to_string).ok_or_else(|| {
        err(format!(
            "<{}> without the attribute {name}",
            n.tag_name().name()
        ))
    })
}

fn opt(n: X, name: &str) -> Option<String> {
    n.attribute(name).map(str::to_string)
}

fn flag(n: X, name: &str, default: bool) -> Result<bool, ReadError> {
    match n.attribute(name) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(v) => Err(err(format!(
            "<{}> {name}={v:?} is no boolean",
            n.tag_name().name()
        ))),
    }
}

fn double(n: X, name: &str) -> Result<Option<f64>, ReadError> {
    n.attribute(name)
        .map(|v| {
            v.trim().parse::<f64>().map_err(|_| {
                err(format!(
                    "<{}> {name}={v:?} is no number",
                    n.tag_name().name()
                ))
            })
        })
        .transpose()
}

fn math_of(n: X) -> Result<Option<Node>, ReadError> {
    child_math(n).map(math::read_math).transpose()
}

fn child_math<'a, 'i>(n: X<'a, 'i>) -> Option<X<'a, 'i>> {
    n.children().find(|c| {
        c.is_element()
            && c.tag_name().name() == "math"
            && c.tag_name().namespace() == Some(math::MATHML_NS)
    })
}

/// A unit as `kind`, `factor*kind`, `kind**e` or `(factor*kind)**e`, the factor the double of
/// the text `{multiplier}e{scale}` (the multiplier read as a double first).
fn unit_term(u: X) -> Result<Unit, ReadError> {
    let kind = req(u, "kind")?;
    let text = |name: &str, default: &str| {
        u.attribute(name)
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| default.to_string())
    };
    let (multiplier, scale) = (text("multiplier", "1"), text("scale", "0"));
    let no_number = |what: &str, v: &str| err(format!("<unit> {what} {v:?} is no number"));
    let multiplier = multiplier
        .parse::<f64>()
        .map_err(|_| no_number("multiplier", &multiplier))?;
    let scale = scale
        .parse::<i32>()
        .map_err(|_| no_number("scale", &scale))?;
    Ok(Unit {
        kind,
        exponent: double(u, "exponent")?.unwrap_or(1.0),
        scale,
        multiplier,
    })
}

/// The expression of one unit, `multiplier * 10^scale` as one factor in front of the kind.
fn unit_text(unit: &Unit) -> String {
    let Unit {
        kind,
        exponent,
        scale,
        multiplier,
    } = unit;
    // the display of a finite double has no exponent, so the text is a valid number
    let factor = if multiplier.is_finite() {
        format!("{multiplier}e{scale}")
            .parse::<f64>()
            .unwrap_or(*multiplier)
    } else {
        *multiplier
    };
    match (factor == 1.0, *exponent == 1.0) {
        (true, true) => kind.clone(),
        (false, true) => format!("{factor}*{kind}"),
        (true, false) => format!("{kind}**{exponent}"),
        (false, false) => format!("({factor}*{kind})**{exponent}"),
    }
}

fn unit_definition(n: X, lv: Level) -> Result<UnitDefinition, ReadError> {
    let units = items(n, "listOfUnits", "unit")
        .into_iter()
        .map(unit_term)
        .collect::<Result<Vec<_>, _>>()?;
    let expression = if units.is_empty() {
        "dimensionless".to_string()
    } else {
        units.iter().map(unit_text).collect::<Vec<_>>().join("*")
    };
    Ok(UnitDefinition {
        id: sid(n, lv)?,
        expression,
        units,
        built_in: false,
    })
}

/// A species reference, with the rule of a `<stoichiometryMath>` which is no number appended
/// to `rules`: the stoichiometry of Level 1 and Level 2 changes in time through that element,
/// where Level 3 has an assignment rule of the id of the reference, so the reader writes one
/// and gives the reference the id the rule assigns (`name`, generated when the reference has
/// no id of its own).
fn species_reference(
    n: X,
    lv: Level,
    name: &str,
    rules: &mut Vec<Rule>,
) -> Result<SpeciesReference, ReadError> {
    let stoichiometry = match (double(n, "stoichiometry")?, double(n, "denominator")?) {
        // the denominator of Level 1 and Level 2 divides the stoichiometry
        (Some(s), Some(d)) => Some(s / d),
        (Some(s), None) => Some(s),
        // Level 3 leaves the stoichiometry of a reference a rule determines unset
        (None, _) if lv.is_level_3() => None,
        (None, Some(d)) => Some(1.0 / d),
        (None, None) => Some(1.0),
    };
    let id = opt(n, "id");
    let species = req(n, lv.species_names().1)?;
    let Some(math) = child(n, "stoichiometryMath")
        .map(math_of)
        .transpose()?
        .flatten()
    else {
        return Ok(SpeciesReference {
            species,
            id,
            stoichiometry,
            // a stoichiometry of Level 1 and Level 2 is constant without the element
            constant: flag(n, "constant", !lv.is_level_3())?,
        });
    };
    if let Node::Number { value, .. } = math {
        // a number is the stoichiometry itself, no rule is needed for it
        return Ok(SpeciesReference {
            species,
            id,
            stoichiometry: Some(value),
            constant: true,
        });
    }
    let id = id.unwrap_or_else(|| name.to_string());
    rules.push(Rule::Assignment {
        id: None,
        variable: id.clone(),
        math: Some(math),
    });
    Ok(SpeciesReference {
        species,
        id: Some(id),
        stoichiometry: None,
        constant: false,
    })
}

/// A reaction, with the rules of the stoichiometries which change in time appended to `rules`.
fn reaction(n: X, lv: Level, rules: &mut Vec<Rule>) -> Result<Reaction, ReadError> {
    let id = sid(n, lv)?;
    let reference = lv.species_reference_name();
    let mut refs =
        |list: &str, item: &str, role: &str| -> Result<Vec<SpeciesReference>, ReadError> {
            items(n, list, item)
                .into_iter()
                .enumerate()
                .map(|(index, r)| {
                    let name = format!("__stoichiometry_{id}_{role}_{index}");
                    species_reference(r, lv, &name, rules)
                })
                .collect()
        };
    let kinetic_law = child(n, "kineticLaw")
        .map(|k| -> Result<KineticLaw, ReadError> {
            // the local parameters of Level 1 and Level 2 are a list of parameters
            let (list, item) = if lv.is_level_3() {
                ("listOfLocalParameters", "localParameter")
            } else {
                ("listOfParameters", "parameter")
            };
            let local_parameters = items(k, list, item)
                .into_iter()
                .map(|p| {
                    Ok(LocalParameter {
                        id: sid(p, lv)?,
                        value: double(p, "value")?,
                        units: opt(p, "units"),
                    })
                })
                .collect::<Result<_, ReadError>>()?;
            Ok(KineticLaw {
                id: opt(k, "id"),
                math: math_or_formula(k, lv)?,
                local_parameters,
            })
        })
        .transpose()?;
    let reactants = refs("listOfReactants", reference, "reactant")?;
    let products = refs("listOfProducts", reference, "product")?;
    Ok(Reaction {
        // a reaction of Level 1 and Level 2 is reversible unless it says otherwise
        reversible: flag(n, "reversible", !lv.is_level_3())?,
        fast: flag(n, "fast", false)?,
        compartment: opt(n, "compartment"),
        reactants,
        products,
        modifiers: items(n, "listOfModifiers", "modifierSpeciesReference")
            .into_iter()
            .map(|m| {
                Ok(ModifierSpeciesReference {
                    species: req(m, lv.species_names().1)?,
                    id: opt(m, "id"),
                })
            })
            .collect::<Result<_, ReadError>>()?,
        kinetic_law,
        id,
    })
}

/// The math of the child `name` (`priority` and `delay` hold a `<math>`), absent without the
/// child or when it has no `<math>`.
fn wrapped_math(n: X, name: &str) -> Result<Option<Node>, ReadError> {
    Ok(child(n, name).map(math_of).transpose()?.flatten())
}

fn event(n: X) -> Result<Event, ReadError> {
    let trigger = child(n, "trigger")
        .map(|t| -> Result<Trigger, ReadError> {
            Ok(Trigger {
                id: opt(t, "id"),
                initial_value: flag(t, "initialValue", true)?,
                persistent: flag(t, "persistent", true)?,
                math: math_of(t)?,
            })
        })
        .transpose()?;
    Ok(Event {
        id: opt(n, "id"),
        use_values_from_trigger_time: flag(n, "useValuesFromTriggerTime", true)?,
        trigger,
        delay_id: child(n, "delay").and_then(|d| opt(d, "id")),
        delay: wrapped_math(n, "delay")?,
        priority_id: child(n, "priority").and_then(|d| opt(d, "id")),
        priority: wrapped_math(n, "priority")?,
        assignments: items(n, "listOfEventAssignments", "eventAssignment")
            .into_iter()
            .map(|a| {
                Ok(EventAssignment {
                    id: opt(a, "id"),
                    variable: req(a, "variable")?,
                    math: math_of(a)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
    })
}

/// The elements a level names a rule with, and the attribute which names what the rule
/// determines: Level 1 has one element per kind of variable and no `<assignmentRule>`, and
/// tells an assignment from a rate by the attribute `type`.
fn rule_elements(lv: Level) -> &'static [(&'static str, &'static str)] {
    if lv.is_level_1() {
        if lv.version == 1 {
            &[
                ("parameterRule", "name"),
                ("specieConcentrationRule", "specie"),
                ("compartmentVolumeRule", "compartment"),
                ("algebraicRule", ""),
            ]
        } else {
            &[
                ("parameterRule", "name"),
                ("speciesConcentrationRule", "species"),
                ("compartmentVolumeRule", "compartment"),
                ("algebraicRule", ""),
            ]
        }
    } else {
        &[
            ("assignmentRule", "variable"),
            ("rateRule", "variable"),
            ("algebraicRule", ""),
        ]
    }
}

fn rule(n: X, lv: Level) -> Result<Rule, ReadError> {
    let name = n.tag_name().name();
    let math = math_or_formula(n, lv)?;
    let id = opt(n, "id");
    let Some((_, attribute)) = rule_elements(lv).iter().find(|(e, _)| *e == name) else {
        return Ok(Rule::Algebraic { id, math });
    };
    if attribute.is_empty() {
        return Ok(Rule::Algebraic { id, math });
    }
    let variable = req(n, attribute)?;
    // Level 1 states the kind of a rule in its `type`, every later level in its element
    let rate = if lv.is_level_1() {
        n.attribute("type") == Some("rate")
    } else {
        name == "rateRule"
    };
    Ok(if rate {
        Rule::Rate { id, variable, math }
    } else {
        Rule::Assignment { id, variable, math }
    })
}

/// The units every level before Level 3 defines itself, with the expression of a unit
/// registry: a model which does not redefine them takes them as they are, and every one of them
/// is a default of the model, which Level 3 states on `<model>` instead.
const BUILT_IN_UNITS: [(&str, &str, f64); 5] = [
    ("substance", "mole", 1.0),
    ("volume", "litre", 1.0),
    ("area", "metre", 2.0),
    ("length", "metre", 1.0),
    ("time", "second", 1.0),
];

fn read_model(m: X, lv: Level) -> Result<Model, ReadError> {
    let list = |l: &str, i: &str| items(m, l, i);
    let mut unit_definitions = list("listOfUnitDefinitions", "unitDefinition")
        .into_iter()
        .map(|u| unit_definition(u, lv))
        .collect::<Result<Vec<_>, _>>()?;
    if !lv.is_level_3() {
        // the built in units of the level, unless the model redefines them
        for (id, kind, exponent) in BUILT_IN_UNITS {
            if !unit_definitions.iter().any(|u| u.id == id) {
                let unit = Unit {
                    kind: kind.to_string(),
                    exponent,
                    scale: 0,
                    multiplier: 1.0,
                };
                unit_definitions.push(UnitDefinition {
                    id: id.to_string(),
                    expression: unit_text(&unit),
                    units: vec![unit],
                    built_in: true,
                });
            }
        }
    }
    let mut rules = child(m, "listOfRules")
        .map(|l| {
            l.children()
                .filter(|c| rule_elements(lv).iter().any(|(n, _)| is_core(c, n)))
                .map(|r| rule(r, lv))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let reactions = list("listOfReactions", "reaction")
        .into_iter()
        .map(|r| reaction(r, lv, &mut rules))
        .collect::<Result<Vec<_>, _>>()?;
    let events = list("listOfEvents", "event")
        .into_iter()
        .map(event)
        .collect::<Result<Vec<_>, _>>()?;
    let mut compartments = Vec::new();
    let mut defaulted = Vec::new();
    for c in list("listOfCompartments", "compartment") {
        let id = sid(c, lv)?;
        if c.attribute("constant").is_none() {
            defaulted.push(id.clone());
        }
        compartments.push(Compartment {
            id,
            spatial_dimensions: double(c, "spatialDimensions")?,
            // the size of a compartment of Level 1 is its volume
            size: double(c, if lv.is_level_1() { "volume" } else { "size" })?,
            units: opt(c, "units"),
            // a compartment of Level 1 and Level 2 is constant by default
            constant: flag(c, "constant", !lv.is_level_3())?,
        });
    }
    let mut parameters = Vec::new();
    for p in list("listOfParameters", "parameter") {
        let id = sid(p, lv)?;
        if p.attribute("constant").is_none() {
            defaulted.push(id.clone());
        }
        parameters.push(Parameter {
            id,
            value: double(p, "value")?,
            units: opt(p, "units"),
            // a parameter of Level 1 and Level 2 is constant by default
            constant: flag(p, "constant", !lv.is_level_3())?,
        });
    }
    // Level 1 has no `constant` at all and Level 2 lets it default to true, so a compartment or
    // a parameter which a rule or an event determines is no constant whatever the default says
    let mut determined: Vec<&str> = rules
        .iter()
        .filter_map(|r| match r {
            Rule::Assignment { variable, .. } | Rule::Rate { variable, .. } => {
                Some(variable.as_str())
            }
            Rule::Algebraic { .. } => None,
        })
        .chain(
            events
                .iter()
                .flat_map(|e| e.assignments.iter().map(|a| a.variable.as_str())),
        )
        .collect();
    // Level 1 has no `constant` at all, so a quantity which an algebraic rule reads is one of
    // the unknowns of the rule and no constant, as libsbml converts such a model as well
    if lv.is_level_1() {
        for rule in &rules {
            if let Rule::Algebraic {
                math: Some(math), ..
            } = rule
            {
                determined.extend(math.symbols());
            }
        }
    }
    for id in &defaulted {
        if determined.contains(&id.as_str()) {
            if let Some(c) = compartments.iter_mut().find(|c| &c.id == id) {
                c.constant = false;
            }
            if let Some(p) = parameters.iter_mut().find(|p| &p.id == id) {
                p.constant = false;
            }
        }
    }
    // the units of the model are the built in ones before Level 3, which states them itself;
    // the extent of a reaction of those levels is a substance
    let default_units = |name: &str, built_in: &str| match opt(m, name) {
        Some(units) => Some(units),
        None if lv.is_level_3() => None,
        None => Some(built_in.to_string()),
    };
    Ok(Model {
        id: sid_opt(m, lv),
        substance_units: default_units("substanceUnits", "substance"),
        time_units: default_units("timeUnits", "time"),
        extent_units: default_units("extentUnits", "substance"),
        volume_units: default_units("volumeUnits", "volume"),
        area_units: default_units("areaUnits", "area"),
        length_units: default_units("lengthUnits", "length"),
        conversion_factor: opt(m, "conversionFactor"),
        unit_definitions,
        function_definitions: list("listOfFunctionDefinitions", "functionDefinition")
            .into_iter()
            .map(|f| {
                Ok(FunctionDefinition {
                    id: sid(f, lv)?,
                    math: math_of(f)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
        compartments,
        species: list("listOfSpecies", lv.species_names().0)
            .into_iter()
            .map(|s| {
                Ok(Species {
                    id: sid(s, lv)?,
                    compartment: req(s, "compartment")?,
                    initial_amount: double(s, "initialAmount")?,
                    initial_concentration: double(s, "initialConcentration")?,
                    // the substance units of a species of Level 1 are its `units`
                    substance_units: opt(
                        s,
                        if lv.is_level_1() {
                            "units"
                        } else {
                            "substanceUnits"
                        },
                    ),
                    has_only_substance_units: flag(s, "hasOnlySubstanceUnits", false)?,
                    boundary_condition: flag(s, "boundaryCondition", false)?,
                    constant: flag(s, "constant", false)?,
                    conversion_factor: opt(s, "conversionFactor"),
                })
            })
            .collect::<Result<_, ReadError>>()?,
        parameters,
        initial_assignments: list("listOfInitialAssignments", "initialAssignment")
            .into_iter()
            .map(|a| {
                Ok(InitialAssignment {
                    id: opt(a, "id"),
                    symbol: req(a, "symbol")?,
                    math: math_of(a)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
        rules,
        reactions,
        events,
    })
}

/// The packages which are read although they are required, each as the namespace URI it is
/// identified by, without the version of the package, and the elements of it which annotate the
/// model: those define none of its mathematics, so a model without them is the model the
/// document means. A package is its namespace URI, never the prefix a document gives it.
///
/// `distrib` carries the uncertainty of a parameter (`<distrib:uncertainty>` of a parameter,
/// which the models of pkdb_models write for every fitted parameter: the confidence interval
/// of the profile likelihood, the standard error of the Fisher information and the
/// distribution of the draws). The elements listed are the whole of the package as libsbml
/// 5.21.2 implements it, its reference implementation (`externalParameter` is no element of it
/// but a `type` of an `<distrib:uncertParameter>`); every other element of the namespace is
/// refused by [`check_read_package_elements`], because the reader would ignore it as it ignores
/// everything which is not core. What reaches the mathematics is a draw, which the released
/// package writes as a `<csymbol>` of a distribution
/// (`http://www.sbml.org/sbml/symbols/distrib/normal`), refused by [`math`](crate::math) as an
/// unknown csymbol wherever a math of the model stands, and the drafts of the package as a
/// `<distrib:drawFromDistribution>` element, refused as an element of the namespace which is no
/// uncertainty. The `<math>` of an uncertainty is no child of the parameter and is never read.
const READ_REQUIRED_PACKAGES: [(&str, &[&str]); 1] = [(
    "http://www.sbml.org/sbml/level3/version1/distrib/",
    &[
        "listOfUncertainties",
        "uncertainty",
        "listOfUncertParameters",
        "uncertParameter",
        "uncertSpan",
    ],
)];

/// The elements of a read package which annotate, or `None` for a namespace of no read package.
fn read_package_elements(ns: &str) -> Option<&'static [&'static str]> {
    READ_REQUIRED_PACKAGES
        .iter()
        .find(|(uri, _)| ns.starts_with(uri))
        .map(|(_, elements)| *elements)
}

/// Refuses an element of a read package which is none of the elements of it which annotate.
///
/// A read package is read for its annotations, which the reader ignores as it ignores every
/// element that is not core. An element of such a package which computes must not be ignored:
/// a draw of the drafts of distrib (`<distrib:drawFromDistribution>` of a function definition,
/// of an initial assignment or of a rule, with the distribution in UncertML beside it) or an
/// element of a later version of the package would be dropped without a trace and the model
/// simulated as if it were not there, which is worse than a model refused. The elements of the
/// other packages are not looked at: a package which is not read is refused by its `required`
/// when it changes the model, and ignored when it declares that it does not.
fn check_read_package_elements(root: X) -> Result<(), ReadError> {
    for n in root.descendants() {
        let tag = n.tag_name();
        if n.is_element()
            && let Some(ns) = tag.namespace()
            && let Some(annotations) = read_package_elements(ns)
            && !annotations.contains(&tag.name())
        {
            return Err(err(format!(
                "<{}> of the namespace {ns} is not supported: of that package only the elements of an uncertainty are read, which annotate the model and compute nothing",
                tag.name()
            )));
        }
    }
    Ok(())
}

/// The package of a namespace URI such as `.../level3/version1/comp/version1`.
fn package_name(uri: &str, prefix: Option<&str>) -> String {
    let parts: Vec<&str> = uri.trim_end_matches('/').split('/').collect();
    match parts.len().checked_sub(2).map(|i| parts[i]) {
        Some(p) if !p.is_empty() && !p.starts_with("level") => p.to_string(),
        _ => prefix.unwrap_or(uri).to_string(),
    }
}

pub(crate) fn read_document(
    sbml: &str,
    sources: Option<crate::comp::Sources>,
) -> Result<Document, ReadError> {
    let doc = roxmltree::Document::parse(sbml).map_err(|e| err(format!("invalid XML: {e}")))?;
    if doc.root_element().tag_name().name() == "sbml" && crate::comp::uses_comp(&doc) {
        // a composed model is read as the model of SBML core it flattens into
        let flat = crate::comp::flatten(&doc, sources)?;
        let doc = roxmltree::Document::parse(&flat)
            .map_err(|e| err(format!("the flattened model is no XML: {e}")))?;
        return read_parsed(&doc);
    }
    read_parsed(&doc)
}

fn read_parsed(doc: &roxmltree::Document) -> Result<Document, ReadError> {
    let root = doc.root_element();
    if root.tag_name().name() != "sbml" {
        return Err(err(format!(
            "root element <{}> is not <sbml>",
            root.tag_name().name()
        )));
    }
    let number = |name: &str| -> Result<u32, ReadError> {
        req(root, name)?
            .trim()
            .parse()
            .map_err(|_| err(format!("<sbml> {name} is no integer")))
    };
    let (level, version) = (number("level")?, number("version")?);
    if !matches!((level, version), (1, 1 | 2) | (2, 1..=5) | (3, 1 | 2)) {
        return Err(ReadError::UnsupportedLevel { level, version });
    }
    let namespace = root
        .tag_name()
        .namespace()
        .ok_or_else(|| err("<sbml> without a namespace".into()))?;
    let (_, ns_level, ns_version) = NAMESPACES
        .iter()
        .find(|(uri, _, _)| *uri == namespace)
        .ok_or_else(|| err(format!("{namespace} is no namespace of SBML core")))?;
    if *ns_level != level || ns_version.is_some_and(|v| v != version) {
        return Err(err(format!(
            "the namespace {namespace} is no namespace of SBML level {level} version {version}"
        )));
    }
    let lv = Level { level, version };
    for a in root.attributes() {
        if a.name() == "required"
            && let Some(ns) = a.namespace()
            && a.value() == "true"
            && read_package_elements(ns).is_none()
        {
            return Err(ReadError::Package(package_name(ns, root.lookup_prefix(ns))));
        }
    }
    check_read_package_elements(root)?;
    let model = child(root, "model").ok_or_else(|| err("<sbml> without a <model>".into()))?;
    Ok(Document {
        level,
        version,
        model: read_model(model, lv)?,
    })
}

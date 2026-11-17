//! The data model of an SBML Level 3 Version 1 or 2 core document and its reader.

use crate::math::{self, Node};
use crate::{Document, ReadError};
use roxmltree::Node as X;

/// The namespace of SBML Level 3 Version 2 core.
pub const SBML_NS_L3V2: &str = "http://www.sbml.org/sbml/level3/version2/core";

/// The namespace of SBML Level 3 Version 1 core.
pub const SBML_NS_L3V1: &str = "http://www.sbml.org/sbml/level3/version1/core";

/// Whether `ns` is the namespace of SBML Level 3 core, version 1 or 2.
pub fn is_sbml_ns(ns: &str) -> bool {
    ns == SBML_NS_L3V2 || ns == SBML_NS_L3V1
}

/// A unit definition as a unit expression, e.g. `0.001*mole*(60*second)**-1`.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitDefinition {
    /// The id.
    pub id: String,
    /// The expression a unit registry parses.
    pub expression: String,
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
fn unit_term(u: X) -> Result<String, ReadError> {
    let kind = req(u, "kind")?;
    let text = |name: &str, default: &str| {
        u.attribute(name)
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| default.to_string())
    };
    let (multiplier, scale) = (text("multiplier", "1"), text("scale", "0"));
    let no_number = |what: &str, v: &str| err(format!("<unit> {what} {v:?} is no number"));
    let m = multiplier
        .parse::<f64>()
        .map_err(|_| no_number("multiplier", &multiplier))?;
    // the display of a finite double has no exponent, so the text is a valid number
    let factor = if m.is_finite() {
        format!("{m}e{scale}")
            .parse::<f64>()
            .map_err(|_| no_number("scale", &scale))?
    } else {
        m
    };
    let exponent = double(u, "exponent")?.unwrap_or(1.0);
    Ok(match (factor == 1.0, exponent == 1.0) {
        (true, true) => kind,
        (false, true) => format!("{factor}*{kind}"),
        (true, false) => format!("{kind}**{exponent}"),
        (false, false) => format!("({factor}*{kind})**{exponent}"),
    })
}

fn unit_definition(n: X) -> Result<UnitDefinition, ReadError> {
    let terms = items(n, "listOfUnits", "unit")
        .into_iter()
        .map(unit_term)
        .collect::<Result<Vec<_>, _>>()?;
    let expression = if terms.is_empty() {
        "dimensionless".to_string()
    } else {
        terms.join("*")
    };
    Ok(UnitDefinition {
        id: req(n, "id")?,
        expression,
    })
}

fn species_reference(n: X) -> Result<SpeciesReference, ReadError> {
    Ok(SpeciesReference {
        species: req(n, "species")?,
        id: opt(n, "id"),
        stoichiometry: double(n, "stoichiometry")?,
        constant: flag(n, "constant", false)?,
    })
}

fn reaction(n: X) -> Result<Reaction, ReadError> {
    let refs = |list, item| -> Result<Vec<SpeciesReference>, ReadError> {
        items(n, list, item)
            .into_iter()
            .map(species_reference)
            .collect()
    };
    let kinetic_law = child(n, "kineticLaw")
        .map(|k| -> Result<KineticLaw, ReadError> {
            let local_parameters = items(k, "listOfLocalParameters", "localParameter")
                .into_iter()
                .map(|p| {
                    Ok(LocalParameter {
                        id: req(p, "id")?,
                        value: double(p, "value")?,
                        units: opt(p, "units"),
                    })
                })
                .collect::<Result<_, ReadError>>()?;
            Ok(KineticLaw {
                id: opt(k, "id"),
                math: math_of(k)?,
                local_parameters,
            })
        })
        .transpose()?;
    Ok(Reaction {
        id: req(n, "id")?,
        reversible: flag(n, "reversible", false)?,
        fast: flag(n, "fast", false)?,
        compartment: opt(n, "compartment"),
        reactants: refs("listOfReactants", "speciesReference")?,
        products: refs("listOfProducts", "speciesReference")?,
        modifiers: items(n, "listOfModifiers", "modifierSpeciesReference")
            .into_iter()
            .map(|m| {
                Ok(ModifierSpeciesReference {
                    species: req(m, "species")?,
                    id: opt(m, "id"),
                })
            })
            .collect::<Result<_, ReadError>>()?,
        kinetic_law,
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

fn rule(n: X) -> Result<Rule, ReadError> {
    Ok(match n.tag_name().name() {
        "assignmentRule" => Rule::Assignment {
            id: opt(n, "id"),
            variable: req(n, "variable")?,
            math: math_of(n)?,
        },
        "rateRule" => Rule::Rate {
            id: opt(n, "id"),
            variable: req(n, "variable")?,
            math: math_of(n)?,
        },
        _ => Rule::Algebraic {
            id: opt(n, "id"),
            math: math_of(n)?,
        },
    })
}

fn read_model(m: X) -> Result<Model, ReadError> {
    let list = |l: &str, i: &str| items(m, l, i);
    Ok(Model {
        id: opt(m, "id"),
        substance_units: opt(m, "substanceUnits"),
        time_units: opt(m, "timeUnits"),
        extent_units: opt(m, "extentUnits"),
        volume_units: opt(m, "volumeUnits"),
        area_units: opt(m, "areaUnits"),
        length_units: opt(m, "lengthUnits"),
        conversion_factor: opt(m, "conversionFactor"),
        unit_definitions: list("listOfUnitDefinitions", "unitDefinition")
            .into_iter()
            .map(unit_definition)
            .collect::<Result<_, _>>()?,
        function_definitions: list("listOfFunctionDefinitions", "functionDefinition")
            .into_iter()
            .map(|f| {
                Ok(FunctionDefinition {
                    id: req(f, "id")?,
                    math: math_of(f)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
        compartments: list("listOfCompartments", "compartment")
            .into_iter()
            .map(|c| {
                Ok(Compartment {
                    id: req(c, "id")?,
                    spatial_dimensions: double(c, "spatialDimensions")?,
                    size: double(c, "size")?,
                    units: opt(c, "units"),
                    constant: flag(c, "constant", false)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
        species: list("listOfSpecies", "species")
            .into_iter()
            .map(|s| {
                Ok(Species {
                    id: req(s, "id")?,
                    compartment: req(s, "compartment")?,
                    initial_amount: double(s, "initialAmount")?,
                    initial_concentration: double(s, "initialConcentration")?,
                    substance_units: opt(s, "substanceUnits"),
                    has_only_substance_units: flag(s, "hasOnlySubstanceUnits", false)?,
                    boundary_condition: flag(s, "boundaryCondition", false)?,
                    constant: flag(s, "constant", false)?,
                    conversion_factor: opt(s, "conversionFactor"),
                })
            })
            .collect::<Result<_, ReadError>>()?,
        parameters: list("listOfParameters", "parameter")
            .into_iter()
            .map(|p| {
                Ok(Parameter {
                    id: req(p, "id")?,
                    value: double(p, "value")?,
                    units: opt(p, "units"),
                    constant: flag(p, "constant", false)?,
                })
            })
            .collect::<Result<_, ReadError>>()?,
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
        rules: child(m, "listOfRules")
            .map(|l| {
                l.children()
                    .filter(|c| {
                        ["assignmentRule", "rateRule", "algebraicRule"]
                            .iter()
                            .any(|n| is_core(c, n))
                    })
                    .map(rule)
                    .collect::<Result<_, _>>()
            })
            .transpose()?
            .unwrap_or_default(),
        reactions: list("listOfReactions", "reaction")
            .into_iter()
            .map(reaction)
            .collect::<Result<_, _>>()?,
        events: list("listOfEvents", "event")
            .into_iter()
            .map(event)
            .collect::<Result<_, _>>()?,
    })
}

/// The namespace URIs, without the version of the package, of the packages which are read
/// although they are required: their elements annotate the model and define none of its
/// mathematics, so a model without them is the model the document means. A package is its
/// namespace URI, never the prefix a document gives it.
///
/// `distrib` carries the uncertainty of a parameter (`<distrib:uncertainty>` of a parameter,
/// which the models of pkdb_models write for every fitted parameter: the confidence interval
/// of the profile likelihood, the standard error of the Fisher information and the
/// distribution of the draws). The one distrib construct which reaches the mathematics is a
/// draw, a `<csymbol>` of a distribution (`http://www.sbml.org/sbml/symbols/distrib/normal`),
/// which [`math`](crate::math) refuses as an unknown csymbol wherever a math of the model
/// stands; the `<math>` of an uncertainty is no child of the parameter and is never read.
const READ_REQUIRED_PACKAGES: [&str; 1] = ["http://www.sbml.org/sbml/level3/version1/distrib/"];

/// The package of a namespace URI such as `.../level3/version1/comp/version1`.
fn package_name(uri: &str, prefix: Option<&str>) -> String {
    let parts: Vec<&str> = uri.trim_end_matches('/').split('/').collect();
    match parts.len().checked_sub(2).map(|i| parts[i]) {
        Some(p) if !p.is_empty() && !p.starts_with("level") => p.to_string(),
        _ => prefix.unwrap_or(uri).to_string(),
    }
}

pub(crate) fn read_document(sbml: &str) -> Result<Document, ReadError> {
    let doc = roxmltree::Document::parse(sbml).map_err(|e| err(format!("invalid XML: {e}")))?;
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
    if level != 3 || !matches!(version, 1 | 2) {
        return Err(ReadError::NotLevel3 { level, version });
    }
    for a in root.attributes() {
        if a.name() == "required"
            && let Some(ns) = a.namespace()
            && a.value() == "true"
            && !READ_REQUIRED_PACKAGES.iter().any(|uri| ns.starts_with(uri))
        {
            return Err(ReadError::Package(package_name(ns, root.lookup_prefix(ns))));
        }
    }
    let model = child(root, "model").ok_or_else(|| err("<sbml> without a <model>".into()))?;
    Ok(Document {
        level,
        version,
        model: read_model(model)?,
    })
}

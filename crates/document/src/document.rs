//! The document, its model and the elements of the model.

use crate::element::Element;
use crate::math::Ast;

/// A function definition of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FunctionDefinition {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The syntax tree of the `math` of the definition, a lambda.
    pub math: Option<Ast>,
}

/// One factor of a unit definition: a base unit with its exponent, scale and multiplier.
///
/// A unit carries no identifier and nothing in SBML refers to it, so it is part
/// of its definition and no element of its own.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unit {
    /// The `kind` attribute, the name of the base unit.
    pub kind: Option<String>,
    /// The `exponent` attribute, a double since Level 3 allows a fractional one.
    pub exponent: Option<f64>,
    /// The `scale` attribute.
    pub scale: Option<i32>,
    /// The `multiplier` attribute.
    pub multiplier: Option<f64>,
}

/// A unit definition of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitDefinition {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The units of the definition, in the order of the document.
    pub units: Vec<Unit>,
}

/// A compartment of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compartment {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `spatialDimensions` attribute.
    pub spatial_dimensions: Option<f64>,
    /// The `size` attribute.
    pub size: Option<f64>,
    /// The `constant` attribute.
    pub constant: Option<bool>,
    /// The `units` attribute.
    pub units: Option<String>,
}

/// A species of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Species {
    /// The parameter named as this species' conversion factor.
    pub conversion_factor: Option<String>,
    /// The fields every SBML object shares.
    pub element: Element,
    /// The id of the compartment of the species.
    pub compartment: String,
    /// The `initialAmount` attribute.
    pub initial_amount: Option<f64>,
    /// The `initialConcentration` attribute.
    pub initial_concentration: Option<f64>,
    /// The `substanceUnits` attribute.
    pub substance_units: Option<String>,
    /// Legacy Level 2 concentration size unit override.
    pub spatial_size_units: Option<String>,
    /// The `hasOnlySubstanceUnits` attribute.
    pub has_only_substance_units: Option<bool>,
    /// The `boundaryCondition` attribute.
    pub boundary_condition: Option<bool>,
    /// The `constant` attribute.
    pub constant: Option<bool>,
}

/// A parameter of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parameter {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `value` attribute.
    pub value: Option<f64>,
    /// The `constant` attribute.
    pub constant: Option<bool>,
    /// The `units` attribute.
    pub units: Option<String>,
}

/// An initial assignment of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InitialAssignment {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `symbol` attribute, the element the assignment sets.
    ///
    /// libsbml aliases the id of an initial assignment to this attribute, so
    /// [`Element::id`] holds the `id` attribute of the document alone.
    pub symbol: String,
    /// The syntax tree of the `math` of the assignment.
    pub math: Option<Ast>,
}

/// Which of the three rule classes of SBML a rule is.
///
/// It is the discriminator the report writes as the type of the element, and
/// an algebraic rule is the one of the three which sets no variable.
///
/// The default is the algebraic rule, so that the default [`Rule`], which
/// carries no variable, is a rule of the one class which needs none: a default
/// of either other class would be a rule the `variable` of which is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleKind {
    /// An assignment rule, which states the value of its variable.
    Assignment,
    /// A rate rule, which states the rate of change of its variable.
    Rate,
    /// An algebraic rule, which states an expression of value zero.
    #[default]
    Algebraic,
}

/// A rule of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rule {
    /// The fields every SBML object shares.
    pub element: Element,
    /// Which of the three rule classes the rule is.
    pub kind: RuleKind,
    /// The `variable` attribute, `None` for an algebraic rule, which sets none.
    ///
    /// libsbml aliases the id of a rule to this attribute, so [`Element::id`]
    /// holds the `id` attribute of the document alone.
    pub variable: Option<String>,
    /// The syntax tree of the `math` of the rule.
    pub math: Option<Ast>,
}

/// A constraint of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Constraint {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The syntax tree of the `math` of the constraint.
    pub math: Option<Ast>,
    /// The `message` of the constraint as XML.
    pub message: Option<String>,
}

/// A reactant or a product of a reaction.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpeciesReference {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The id of the species the reference names.
    pub species: String,
    /// The `stoichiometry` as `getStoichiometry()` of libsbml answers it.
    ///
    /// It is the raw answer of libsbml, which is the value of the attribute
    /// where the document sets one and the default of its level where it does
    /// not: measured against libsbml 5.21.2, a reference of Level 1 or Level 2
    /// which sets nothing answers `1.0` and one of Level 3 answers `NaN`. The
    /// value the report shows and the coefficient of the equation follow from
    /// this number and the two flags below, and no reader may hand on a
    /// stoichiometry of its own.
    pub stoichiometry: f64,
    /// Whether the document sets the `stoichiometry` attribute.
    pub stoichiometry_set: bool,
    /// Whether the reference carries a `stoichiometryMath`, which Level 3 dropped.
    ///
    /// A reference of Level 1 or Level 2 with a `stoichiometryMath` has no
    /// stoichiometry of its own, however the default of its level reads.
    pub stoichiometry_math: bool,
    /// Level 1/2 mathematical stoichiometry, promoted to a rule for equation analysis.
    pub stoichiometry_math_ast: Option<Ast>,
    /// The `constant` attribute.
    pub constant: Option<bool>,
}

/// A modifier of a reaction, which names a species and nothing else.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModifierSpeciesReference {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The id of the species the reference names.
    pub species: String,
}

/// A parameter of a kinetic law, local to its reaction.
///
/// A Level 3 law writes them as `localParameter` and a Level 1 or Level 2 law
/// as `parameter`; both carry the id, the value and the units the report shows,
/// so the model has one type for either.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalParameter {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `value` attribute.
    pub value: Option<f64>,
    /// The `units` attribute.
    pub units: Option<String>,
}

/// The kinetic law of a reaction.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KineticLaw {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The syntax tree of the `math` of the law, its rate.
    pub math: Option<Ast>,
    /// The local parameters of the law, in the order of the document.
    pub local_parameters: Vec<LocalParameter>,
}

/// A reaction of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reaction {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `reversible` attribute, `None` where the document does not set it.
    ///
    /// `getReversible()` of libsbml answers `true` for a reaction which does
    /// **not** set the attribute, and does so at every level, where it is the
    /// default of Level 1 and of Level 2 and no value at all in Level 3. So the
    /// filler asks `isSetReversible()` and this field is what the document
    /// states, and the default of the level is written where the report is
    /// built and never read from libsbml.
    pub reversible: Option<bool>,
    /// The `fast` attribute, which Level 3 Version 2 dropped.
    pub fast: Option<bool>,
    /// The `compartment` attribute, which Level 3 added.
    pub compartment: Option<String>,
    /// The reactants, in the order of the document.
    pub reactants: Vec<SpeciesReference>,
    /// The products, in the order of the document.
    pub products: Vec<SpeciesReference>,
    /// The modifiers, in the order of the document.
    pub modifiers: Vec<ModifierSpeciesReference>,
    /// The kinetic law, when the reaction has one.
    pub kinetic_law: Option<KineticLaw>,
}

/// An element whose one field is a math, the priority and the delay of an event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MathElement {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The syntax tree of the `math` of the element.
    pub math: Option<Ast>,
}

/// The trigger of an event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trigger {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The syntax tree of the `math` of the trigger, its condition.
    pub math: Option<Ast>,
    /// The `initialValue` attribute, which Level 3 added.
    pub initial_value: Option<bool>,
    /// The `persistent` attribute, which Level 3 added.
    pub persistent: Option<bool>,
}

/// One assignment an event makes when it fires.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventAssignment {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `variable` attribute, the element the assignment sets.
    ///
    /// libsbml aliases the id of an event assignment to this attribute, so
    /// [`Element::id`] holds the `id` attribute of the document alone.
    pub variable: String,
    /// The syntax tree of the `math` of the assignment.
    pub math: Option<Ast>,
}

/// An event of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Event {
    /// The fields every SBML object shares.
    pub element: Element,
    /// The `useValuesFromTriggerTime` attribute.
    pub use_values_from_trigger_time: Option<bool>,
    /// The trigger, when the event has one.
    pub trigger: Option<Trigger>,
    /// The priority, when the event has one, which Level 3 added.
    pub priority: Option<MathElement>,
    /// The delay, when the event has one.
    pub delay: Option<MathElement>,
    /// The assignments, in the order of the document.
    pub event_assignments: Vec<EventAssignment>,
}

/// The six model-level default unit attributes of Level 3.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelUnits {
    /// Default substance units.
    pub substance: Option<String>,
    /// Default time units.
    pub time: Option<String>,
    /// Default volume units.
    pub volume: Option<String>,
    /// Default area units.
    pub area: Option<String>,
    /// Default length units.
    pub length: Option<String>,
    /// Default extent units.
    pub extent: Option<String>,
}

/// The model of a document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// The model's default unit identifiers.
    pub units: ModelUnits,
    /// The parameter named as the model conversion factor.
    pub conversion_factor: Option<String>,
    /// The fields every SBML object shares.
    pub element: Element,
    /// The function definitions, in the order of the document.
    pub function_definitions: Vec<FunctionDefinition>,
    /// The unit definitions, in the order of the document.
    pub unit_definitions: Vec<UnitDefinition>,
    /// The compartments, in the order of the document.
    pub compartments: Vec<Compartment>,
    /// The species, in the order of the document.
    pub species: Vec<Species>,
    /// The parameters, in the order of the document.
    pub parameters: Vec<Parameter>,
    /// The initial assignments, in the order of the document.
    pub initial_assignments: Vec<InitialAssignment>,
    /// The rules, in the order of the document.
    pub rules: Vec<Rule>,
    /// The constraints, in the order of the document.
    pub constraints: Vec<Constraint>,
    /// The reactions, in the order of the document.
    pub reactions: Vec<Reaction>,
    /// The events, in the order of the document.
    pub events: Vec<Event>,
}

/// An SBML package a document uses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Package {
    /// Canonical package name, independent of an authored namespace alias.
    pub name: String,
    /// The prefix of the package namespace, such as `comp`.
    pub prefix: String,
    /// The version of the package.
    pub version: u32,
    /// Whether the package is required by the document.
    pub required: bool,
}

/// An SBML document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    /// Canonical XML digest used only when an anonymous main model collides with a definition.
    pub main_model_fallback: Option<String>,
    /// Internal comp model definitions, in document order.
    pub model_definitions: Vec<Model>,
    /// The SBML level.
    pub level: u32,
    /// The SBML version within the level.
    pub version: u32,
    /// The packages the document uses, in the order of the document.
    pub packages: Vec<Package>,
    /// The fields every SBML object shares.
    pub element: Element,
    /// The model, when the document has one.
    pub model: Option<Model>,
}

impl Document {
    /// Models in report order: the main model, then internal definitions.
    pub fn models(&self) -> impl Iterator<Item = &Model> {
        self.model.iter().chain(&self.model_definitions)
    }

    /// Visit every core SBase without cloning the owned document.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        let mut elements = Vec::new();

        elements.push(&self.element);
        for model in self.models() {
            elements.push(&model.element);
            macro_rules! each {
                ($list:expr) => {
                    for child in $list {
                        elements.push(&child.element);
                    }
                };
            }
            each!(&model.function_definitions);
            each!(&model.unit_definitions);
            each!(&model.compartments);
            each!(&model.species);
            each!(&model.parameters);
            each!(&model.initial_assignments);
            each!(&model.rules);
            each!(&model.constraints);
            for reaction in &model.reactions {
                elements.push(&reaction.element);
                each!(&reaction.reactants);
                each!(&reaction.products);
                each!(&reaction.modifiers);
                if let Some(law) = &reaction.kinetic_law {
                    elements.push(&law.element);
                    each!(&law.local_parameters);
                }
            }
            for event in &model.events {
                elements.push(&event.element);
                if let Some(trigger) = &event.trigger {
                    elements.push(&trigger.element);
                }
                if let Some(priority) = &event.priority {
                    elements.push(&priority.element);
                }
                if let Some(delay) = &event.delay {
                    elements.push(&delay.element);
                }
                each!(&event.event_assignments);
            }
        }

        elements.into_iter()
    }

    /// Visit every core SBase once, excluding package objects and list metadata.
    pub fn visit_elements_mut(&mut self, mut visit: impl FnMut(&mut Element)) {
        visit(&mut self.element);
        for model in self.model.iter_mut().chain(&mut self.model_definitions) {
            visit(&mut model.element);
            macro_rules! each {
                ($list:expr) => {
                    for child in $list {
                        visit(&mut child.element);
                    }
                };
            }
            each!(&mut model.function_definitions);
            each!(&mut model.unit_definitions);
            each!(&mut model.compartments);
            each!(&mut model.species);
            each!(&mut model.parameters);
            each!(&mut model.initial_assignments);
            each!(&mut model.rules);
            each!(&mut model.constraints);
            for reaction in &mut model.reactions {
                visit(&mut reaction.element);
                each!(&mut reaction.reactants);
                each!(&mut reaction.products);
                each!(&mut reaction.modifiers);
                if let Some(law) = &mut reaction.kinetic_law {
                    visit(&mut law.element);
                    each!(&mut law.local_parameters);
                }
            }
            for event in &mut model.events {
                visit(&mut event.element);
                if let Some(trigger) = &mut event.trigger {
                    visit(&mut trigger.element);
                }
                if let Some(priority) = &mut event.priority {
                    visit(&mut priority.element);
                }
                if let Some(delay) = &mut event.delay {
                    visit(&mut delay.element);
                }
                each!(&mut event.event_assignments);
            }
        }
    }
}

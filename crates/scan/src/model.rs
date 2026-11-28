//! What the planner reads of a model: its description `model/2` alone, so a host plans a scan
//! from the compiled model without the SBML.

use std::collections::{BTreeMap, HashMap, HashSet};

use sbmlsim_abi::{EntityKind, ModelDescription};
use sbmlsim_units::Registry;

use crate::error::ScanError;

/// The kind of a target of a change, sbmlsim's `TargetKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// A parameter.
    Parameter,
    /// A compartment.
    Compartment,
    /// The amount of a species, `S`.
    SpeciesAmount,
    /// The concentration of a species, `[S]`.
    SpeciesConcentration,
}

impl TargetKind {
    /// The kind in the words of sbmlsim.
    pub fn text(self) -> &'static str {
        match self {
            TargetKind::Parameter => "parameter",
            TargetKind::Compartment => "compartment",
            TargetKind::SpeciesAmount => "species amount",
            TargetKind::SpeciesConcentration => "species concentration",
        }
    }
}

/// An entity of the description.
#[derive(Debug, Clone)]
struct EntityInfo {
    kind: EntityKind,
    settable: bool,
    initial: f64,
    state: bool,
}

/// A model as the planner reads it.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// The id of the model, which messages name it by.
    pub label: String,
    entities: HashMap<String, EntityInfo>,
    selections: HashSet<String>,
    units: BTreeMap<String, String>,
    time_unit: Option<String>,
    initial_assignments: HashSet<String>,
    time_dependent: bool,
    has_events: bool,
    states: HashSet<String>,
    resume_targets: Option<Vec<String>>,
    registry: Registry,
}

impl ModelInfo {
    /// The model of a description.
    ///
    /// # Errors
    ///
    /// A description without the time unit of the model, which a compiler before the planner
    /// wrote without the units sbmlsim names, the initial assignments and the dependence on the
    /// time, so the planner would plan it wrong.
    pub fn new(description: &ModelDescription) -> Result<ModelInfo, ScanError> {
        if description.time_unit.is_none() {
            return Err(ScanError::request(format!(
                "The description of the model '{}' was written by {}, which writes no time unit, \
                 initial assignments or time dependence of the model, which a scan plans with: \
                 compile the model again with the current sbmlsim-core.",
                description.id, description.compiler
            )));
        }
        let mut entities = HashMap::new();
        for (state, list) in [
            (true, &description.states),
            (false, &description.parameters),
        ] {
            for e in list {
                entities.insert(
                    e.id.clone(),
                    EntityInfo {
                        kind: e.kind.clone(),
                        settable: e.settable,
                        initial: e.initial,
                        state,
                    },
                );
            }
        }
        let selections: HashSet<String> = description
            .selections
            .iter()
            .map(|s| s.id.clone())
            .collect();
        Ok(ModelInfo {
            label: description.id.clone(),
            resume_targets: resume_targets(description, &selections),
            states: description.states.iter().map(|e| e.id.clone()).collect(),
            entities,
            selections,
            units: description.units.clone(),
            time_unit: description.time_unit.clone(),
            initial_assignments: description.initial_assignments.iter().cloned().collect(),
            time_dependent: description.time_dependent,
            has_events: !description.events.is_empty(),
            registry: Registry::new()
                .with_model_units(&description.units, &description.unit_definitions),
        })
    }

    /// The registry of the units, with the units of the entities of the model.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The entity of a target, the species of a concentration `[S]`.
    pub fn entity(target: &str) -> &str {
        match target.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            Some(species) => species,
            None => target,
        }
    }

    /// The kind of a target, sbmlsim's `ModelSymbols.kind`.
    ///
    /// # Errors
    ///
    /// A target which is no entity of the model, a concentration of no species, or an entity a
    /// rule determines, which a change cannot set.
    pub fn kind(&self, target: &str) -> Result<TargetKind, ScanError> {
        let entity = ModelInfo::entity(target);
        let Some(info) = self.entities.get(entity) else {
            return Err(ScanError::request(format!(
                "'{target}' is not a parameter, a compartment or a species of the model."
            )));
        };
        if !info.settable {
            return Err(ScanError::request(format!(
                "'{target}' is the variable of an assignment rule, its value is the rule and \
                 a change cannot set it."
            )));
        }
        let species = matches!(info.kind, EntityKind::Species { .. });
        if entity != target {
            if !species {
                return Err(ScanError::request(format!(
                    "'{target}' is a concentration, but '{entity}' is no species of the model."
                )));
            }
            return Ok(TargetKind::SpeciesConcentration);
        }
        match info.kind {
            EntityKind::Species { .. } => Ok(TargetKind::SpeciesAmount),
            EntityKind::Compartment => Ok(TargetKind::Compartment),
            EntityKind::Parameter => Ok(TargetKind::Parameter),
            EntityKind::SpeciesReference => Err(ScanError::request(format!(
                "'{target}' is not a parameter, a compartment or a species of the model."
            ))),
        }
    }

    /// Whether a symbol is a parameter of the model (a constant one, the variable of a rule or
    /// of a rate rule alike), sbmlsim's `symbol in ModelSymbols.parameters`.
    pub fn is_parameter(&self, symbol: &str) -> bool {
        self.entities
            .get(symbol)
            .is_some_and(|e| e.kind == EntityKind::Parameter)
    }

    /// Whether the entity of a symbol is a parameter, a compartment or a species.
    pub fn is_entity(&self, symbol: &str) -> bool {
        self.entities
            .get(ModelInfo::entity(symbol))
            .is_some_and(|e| e.kind != EntityKind::SpeciesReference)
    }

    /// Whether a symbol is a constant parameter: a parameter of no initial assignment and of no
    /// rule, whose value as loaded nothing a pre-initialization sets can move.
    pub fn constant_parameter(&self, symbol: &str) -> bool {
        self.entities.get(symbol).is_some_and(|e| {
            e.kind == EntityKind::Parameter
                && e.settable
                && !e.state
                && !self.initial_assignments.contains(symbol)
        })
    }

    /// The value of a parameter, a compartment or the amount of a species of the model as
    /// loaded, the `initial` of its entity; `None` for anything else (a concentration, a
    /// reaction, a species whose entity stores its concentration).
    pub fn loaded_value(&self, symbol: &str) -> Option<f64> {
        let e = self.entities.get(symbol)?;
        match e.kind {
            EntityKind::Species {
                stores_concentration: true,
                ..
            }
            | EntityKind::SpeciesReference => None,
            _ => Some(e.initial),
        }
    }

    /// Whether the engine has an output of a selection, `time` among them.
    pub fn has_selection(&self, selection: &str) -> bool {
        selection == "time" || self.selections.contains(selection)
    }

    /// The unit expression of a target in the model, `None` without one.
    pub fn unit(&self, target: &str) -> Option<&str> {
        self.units.get(target).map(String::as_str)
    }

    /// The unit of the time of the model as sbmlsim names it, `""` without one.
    pub fn time_unit_text(&self) -> &str {
        self.time_unit.as_deref().unwrap_or("")
    }

    /// The unit of the time of the model, `dimensionless` without one.
    pub fn time_unit(&self) -> &str {
        match self.time_unit.as_deref() {
            Some(unit) if !unit.is_empty() => unit,
            _ => "dimensionless",
        }
    }

    /// Whether the model depends on the absolute time of its clock.
    pub fn time_dependent(&self) -> bool {
        self.time_dependent
    }

    /// Whether the model has events.
    pub fn has_events(&self) -> bool {
        self.has_events
    }

    /// Whether an id is a state of the model, one the integrator integrates.
    pub fn is_state(&self, id: &str) -> bool {
        self.states.contains(id)
    }

    /// The targets the state of the model is handed to the engine with, sorted, `None` for a
    /// model whose state they do not cover, see the function `resume_targets` of the module.
    pub fn resume_targets(&self) -> Option<&[String]> {
        self.resume_targets.as_deref()
    }
}

/// The targets a segment of a fold resumes from, sbmlsim's `CoreSBMLModel.resume_targets`:
/// every parameter, compartment and species (its amount) which is not the variable of an
/// assignment rule and which the engine selects, sorted, so a change of them at the start of a
/// segment sets the whole state of the model. `None` if a state of the model is none of them,
/// e.g. a species reference which is not constant: such a state would keep its value as
/// loaded, so the segments of that model start at the start of the simulation.
///
/// The variable of an assignment rule is an entity which is not settable and is no algebraic
/// unknown (the last `n_algebraic` states), which are not settable either.
fn resume_targets(
    description: &ModelDescription,
    selections: &HashSet<String>,
) -> Option<Vec<String>> {
    let n_states = description.states.len();
    let first_algebraic = n_states.saturating_sub(description.n_algebraic as usize);
    let algebraic: HashSet<&str> = description.states[first_algebraic..]
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    let mut targets: Vec<String> = description
        .states
        .iter()
        .chain(&description.parameters)
        .filter(|e| e.kind != EntityKind::SpeciesReference)
        .filter(|e| e.settable || algebraic.contains(e.id.as_str()))
        .filter(|e| selections.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    // the order of python's `sorted`, the code points, which is the order of the bytes of UTF-8
    targets.sort_unstable();
    targets.dedup();
    let covered = description
        .states
        .iter()
        .all(|e| targets.binary_search(&e.id).is_ok());
    covered.then_some(targets)
}

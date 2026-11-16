//! The changes of a simulation: values set at absolute times, as sbmlsim's `Simulation` and
//! `simulator/changes.py` `apply_changes` (PEtab v2).
//!
//! A change sets the values of entities at one time, simultaneously: every value is a number of
//! the request, so nothing is evaluated in the state before the change, and the result does not
//! depend on the order of the pairs. The id of a value is a selection of sbmlsim:
//!
//! - `S` of a species is its amount, `[S]` its concentration, whatever the species stores (a
//!   species of a concentration which is the target of a rate rule stores its concentration);
//! - any other id is the value of its entity (a parameter, a compartment, a species reference).
//!
//! A compartment which a change sets keeps the concentration of every species of a
//! concentration in it (not `hasOnlySubstanceUnits`), except a species of an assignment rule,
//! which follows its rule, and a species the change sets itself: its amount is the
//! concentration before the change times the new size, `(amount / size_before) * size_after`,
//! as sbmlsim reads the concentration and sets it again. A species which stores its
//! concentration keeps it as it is. The compartments are set first, then every other value, so
//! a concentration `[S]` set with its compartment is a concentration in the new compartment
//! (sbmlsim's order); the size of a compartment of an assignment rule is the value of its rule
//! at the time of the change (the rules are evaluated before the values are set). Then the
//! assignment rules are evaluated again (`model_assign`).
//!
//! The pre-initialization takes the same ids (`resolve_values`, sbmlsim's selections), set
//! before `init` (see `initial_values` of the crate).
//!
//! An event with a delay which a change makes true is skipped, as sbmlsim's executors skip it
//! (`events`): it is not pending, assigns nothing and its trigger is armed.
//!
//! Known differences to sbmlsim: a change which sets a state exactly to the
//! threshold of a strict comparison triggers its event right after the change (as sbmlsim's
//! executors of COPASI and AMICI, not its roadrunner, whose CVODE ignores a root which is 0 at
//! the start of a segment); the variable of an assignment rule without math is settable (the
//! rule is ignored, as libroadrunner does), sbmlsim refuses to change it; an entity named
//! `time` is set as an entity, sbmlsim sets the clock of roadrunner.
//!
//! A parameter which an assignment rule of a compartment reads and a concentration `[S]` in
//! that compartment set by one change: the concentration is converted with the size of the
//! rule before the change (the rules are evaluated first, then the values set); sbmlsim sets
//! the values one after the other in the order of the change, so its result depends on that
//! order (a known difference).
//!
//! Note that an event keeps the amounts when it changes a compartment (as libroadrunner does,
//! see `model_event_apply` of `sbmlsim_abi`): a change and an event differ there, as in sbmlsim.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sbmlsim_abi::{Change, EntityKind, ModelDescription};

use crate::model::ModelFns;
use crate::{EngineError, request};

/// What a value of a change or of the pre-initialization sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    /// The value of the entity at this position (the parameters, then the states).
    Entity(usize),
    /// The amount of the species at this position.
    Amount(Species),
    /// The concentration of the species at this position.
    Concentration(Species),
}

/// A species and what it stores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Species {
    /// The position of the species (the parameters, then the states).
    index: usize,
    /// The position of its compartment.
    compartment: usize,
    /// Whether it stores its concentration.
    stores_concentration: bool,
}

/// A change resolved to the positions of its targets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedChange {
    /// The time.
    pub time: f64,
    /// The targets and their values, in the order of the request.
    values: Vec<(Target, f64)>,
    /// The targets and their increments, added to their values before the change.
    increments: Vec<(Target, f64)>,
}

/// The species of a concentration in a compartment which a change of the compartment
/// rescales: the position of the species and of its compartment.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rescaled {
    species: usize,
    compartment: usize,
}

/// The changes of a simulation, in the order of their times.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Changes {
    changes: Vec<ResolvedChange>,
    /// The species a change of their compartment rescales (amounts of a concentration, not
    /// `hasOnlySubstanceUnits`, settable).
    rescaled: Arc<[Rescaled]>,
    /// The number of parameters (the states follow them in the positions).
    n_parameters: usize,
}

/// The position of the entity `id` (the parameters, then the states) and the entity.
#[derive(Clone, Copy)]
struct EntityTarget {
    index: usize,
    settable: bool,
    species: Option<Species>,
}

/// Immutable entity lookup and compartment relationships, built once per loaded engine.
pub(crate) struct Metadata {
    id: String,
    entities: BTreeMap<String, EntityTarget>,
    rescaled: Arc<[Rescaled]>,
    n_parameters: usize,
}

/// The species of a species entity, `None` for another entity.
impl Metadata {
    pub fn new(desc: &ModelDescription) -> Self {
        let mut positions = BTreeMap::new();
        for (i, entity) in desc.parameters.iter().chain(&desc.states).enumerate() {
            positions.entry(entity.id.as_str()).or_insert(i);
        }
        let mut entities = BTreeMap::new();
        let mut rescaled = Vec::new();
        for (index, entity) in desc.parameters.iter().chain(&desc.states).enumerate() {
            let species = match &entity.kind {
                EntityKind::Species {
                    compartment,
                    stores_concentration,
                    ..
                } => positions
                    .get(compartment.as_str())
                    .map(|&compartment| Species {
                        index,
                        compartment,
                        stores_concentration: *stores_concentration,
                    }),
                _ => None,
            };
            if entity.settable
                && matches!(
                    entity.kind,
                    EntityKind::Species {
                        has_only_substance_units: false,
                        stores_concentration: false,
                        ..
                    }
                )
                && let Some(s) = species
            {
                rescaled.push(Rescaled {
                    species: s.index,
                    compartment: s.compartment,
                });
            }
            entities.entry(entity.id.clone()).or_insert(EntityTarget {
                index,
                settable: entity.settable,
                species,
            });
        }
        Self {
            id: desc.id.clone(),
            entities,
            rescaled: rescaled.into(),
            n_parameters: desc.parameters.len(),
        }
    }
}

impl Target {
    /// The position of the entity it sets (the parameters, then the states).
    pub fn index(&self) -> usize {
        match self {
            Target::Entity(i) => *i,
            Target::Amount(s) | Target::Concentration(s) => s.index,
        }
    }

    /// The position of the compartment whose size converts the value into the quantity the
    /// entity stores: an amount of a species which stores its concentration, a concentration
    /// of a species which stores its amount; `None` without a conversion.
    pub fn compartment(&self) -> Option<usize> {
        match self {
            Target::Amount(s) if s.stores_concentration => Some(s.compartment),
            Target::Concentration(s) if !s.stores_concentration => Some(s.compartment),
            _ => None,
        }
    }

    /// The value of the target for the value `stored` the entity stores in a compartment of
    /// the size `size`, the inverse of [`Target::stored`]: `stored * size` for an amount of a
    /// species which stores its concentration, `stored / size` for a concentration of one which
    /// stores its amount (sbmlsim's `getValue`).
    pub fn read(&self, stored: f64, size: f64) -> f64 {
        match self {
            Target::Amount(s) if s.stores_concentration => stored * size,
            Target::Concentration(s) if !s.stores_concentration => stored / size,
            _ => stored,
        }
    }

    /// The value the entity stores for the value `value` of the target in a compartment of
    /// the size `size` (used only by a target with a [`Target::compartment`]): `value / size`
    /// for an amount, `value * size` for a concentration, the conversion of sbmlsim's
    /// `setValue`.
    ///
    /// Which size that is, is the caller's: the pre-initialization takes the one the initial
    /// assignments settle on (`initial_values` of the crate) and deliberately not, as
    /// `setValue` does, the one the model carries at the moment of the assignment, which
    /// would make the result depend on the order of the values.
    pub fn stored(&self, value: f64, size: f64) -> f64 {
        match self {
            Target::Amount(s) if s.stores_concentration => value / size,
            Target::Concentration(s) if !s.stores_concentration => value * size,
            _ => value,
        }
    }
}

/// Resolves the values of a change or of the pre-initialization, both in the selections of
/// sbmlsim (`S` the amount of a species, `[S]` its concentration); `place` names them in a
/// message (`at t = 5`, `in the pre-initialization`). An unknown id, an entity which is not
/// settable, a target set twice and a value which is not finite are [`EngineError::Request`]
/// naming the id.
pub(crate) fn resolve_values(
    metadata: &Metadata,
    values: &[(String, f64)],
    place: &str,
) -> Result<Vec<(Target, f64)>, EngineError> {
    let mut resolved: Vec<(Target, f64)> = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for (id, value) in values {
        let concentration = id.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
        let name = concentration.unwrap_or(id);
        let entity = metadata
            .entities
            .get(name)
            .ok_or_else(|| request(format!("model {} has no entity {id}", metadata.id)))?;
        let (index, species) = (entity.index, entity.species);
        let target = match (concentration, species) {
            (Some(_), Some(species)) => Target::Concentration(species),
            (Some(_), None) => {
                return Err(request(format!(
                    "{id} of model {} is no concentration: {name} is no species",
                    metadata.id
                )));
            }
            (None, Some(species)) => Target::Amount(species),
            (None, None) => Target::Entity(index),
        };
        if !entity.settable {
            return Err(request(format!(
                "{id} of model {} is not settable",
                metadata.id
            )));
        }
        if !value.is_finite() {
            return Err(request(format!(
                "the value {value} of {id} {place} is not finite"
            )));
        }
        if !seen.insert(index) {
            return Err(request(format!("{name} is set more than once {place}")));
        }
        resolved.push((target, *value));
    }
    Ok(resolved)
}

impl Changes {
    /// Resolves and checks the changes of a request: finite times in `[start, end]` in
    /// non-decreasing order, the values as [`resolve_values`] checks them. The changes of one
    /// time are one change, applied at once.
    #[cfg(test)]
    pub fn new(
        desc: &ModelDescription,
        changes: &[Change],
        start: f64,
        end: f64,
    ) -> Result<Changes, EngineError> {
        Self::with_metadata(&Metadata::new(desc), changes, start, end)
    }

    pub fn with_metadata(
        metadata: &Metadata,
        changes: &[Change],
        start: f64,
        end: f64,
    ) -> Result<Changes, EngineError> {
        type Pairs = Vec<(String, f64)>;
        let mut merged: Vec<(f64, Pairs, Pairs)> = Vec::with_capacity(changes.len());
        let mut previous = start;
        for change in changes {
            let t = change.time;
            if !t.is_finite() || t < start || t > end || t < previous {
                return Err(request(format!(
                    "change at t = {t} is not within the start {start} and the end {end} and \
                     in order of the time"
                )));
            }
            match merged.last_mut() {
                Some((time, values, increments)) if *time == t => {
                    values.extend(change.values.iter().cloned());
                    increments.extend(change.increments.iter().cloned());
                }
                _ => merged.push((t, change.values.clone(), change.increments.clone())),
            }
            previous = t;
        }
        let resolved = merged
            .iter()
            .map(|(time, values, increments)| {
                let place = format!("at t = {time}");
                // one id is set or incremented once: resolved together, then split
                let all: Vec<(String, f64)> = values.iter().chain(increments).cloned().collect();
                let mut resolved = resolve_values(metadata, &all, &place)?;
                let increments = resolved.split_off(values.len());
                Ok(ResolvedChange {
                    time: *time,
                    values: resolved,
                    increments,
                })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        Ok(Changes {
            changes: resolved,
            rescaled: metadata.rescaled.clone(),
            n_parameters: metadata.n_parameters,
        })
    }

    /// The time of change `k`, `None` past the last one.
    pub fn time(&self, k: usize) -> Option<f64> {
        self.changes.get(k).map(|c| c.time)
    }

    /// Applies change `k` at its time to the state `y` (the `m` values of the model) and the
    /// parameters `p`, then evaluates the assignment rules.
    pub fn apply(
        &self,
        k: usize,
        model: &dyn ModelFns,
        y: &mut [f64],
        p: &mut [f64],
    ) -> Result<(), EngineError> {
        let change = &self.changes[k];
        // the values of the assignment rules at the time of the change, so the size of a
        // compartment of a rule which converts a value is the current one
        model.assign(change.time, y, p);
        model.check_error()?;
        let np = self.n_parameters;
        let get = |y: &[f64], p: &[f64], i: usize| if i < np { p[i] } else { y[i - np] };
        let set = |y: &mut [f64], p: &mut [f64], i: usize, v: f64| {
            if i < np {
                p[i] = v;
            } else {
                y[i - np] = v;
            }
        };
        // an increment is a value: the value of its target before the change plus it, read
        // in the state after the assignment rules, as the other values are set into it
        let incremented: Vec<(Target, f64)> = change
            .increments
            .iter()
            .map(|&(target, increment)| {
                let size = target.compartment().map_or(f64::NAN, |c| get(y, p, c));
                (
                    target,
                    target.read(get(y, p, target.index()), size) + increment,
                )
            })
            .collect();
        let values: Vec<(Target, f64)> = change.values.iter().copied().chain(incremented).collect();
        let targeted = |i: usize| {
            values.iter().any(|(target, _)| match target {
                Target::Entity(e) => *e == i,
                Target::Amount(s) | Target::Concentration(s) => s.index == i,
            })
        };
        // the compartments first, each keeping the concentrations of its species
        for &(target, value) in &values {
            let Target::Entity(c) = target else { continue };
            let before = get(y, p, c);
            set(y, p, c, value);
            for r in self.rescaled.iter().filter(|r| r.compartment == c) {
                if targeted(r.species) {
                    continue;
                }
                let concentration = get(y, p, r.species) / before;
                set(y, p, r.species, concentration * value);
            }
        }
        // then the species, in the compartments after the change
        for &(target, value) in &values {
            if matches!(target, Target::Entity(_)) {
                continue;
            }
            let size = target.compartment().map_or(f64::NAN, |c| get(y, p, c));
            set(y, p, target.index(), target.stored(value, size));
        }
        // the assignment rules, at the time of the change
        model.assign(change.time, y, p);
        model.check_error()?;
        Ok(())
    }
}

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
    rescaled: Vec<Rescaled>,
    /// The number of parameters (the states follow them in the positions).
    n_parameters: usize,
}

/// The position of the entity `id` (the parameters, then the states) and the entity.
fn position<'d>(desc: &'d ModelDescription, id: &str) -> Option<(usize, &'d sbmlsim_abi::Entity)> {
    desc.parameters
        .iter()
        .chain(&desc.states)
        .enumerate()
        .find(|(_, e)| e.id == id)
}

/// The species of a species entity, `None` for another entity.
fn species_of(
    desc: &ModelDescription,
    index: usize,
    entity: &sbmlsim_abi::Entity,
) -> Option<Species> {
    match &entity.kind {
        EntityKind::Species {
            compartment,
            stores_concentration,
            ..
        } => position(desc, compartment).map(|(c, _)| Species {
            index,
            compartment: c,
            stores_concentration: *stores_concentration,
        }),
        _ => None,
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
    desc: &ModelDescription,
    values: &[(String, f64)],
    place: &str,
) -> Result<Vec<(Target, f64)>, EngineError> {
    let mut resolved: Vec<(Target, f64)> = Vec::with_capacity(values.len());
    let mut seen: Vec<usize> = Vec::with_capacity(values.len());
    for (id, value) in values {
        let concentration = id.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
        let name = concentration.unwrap_or(id);
        let (index, entity) = position(desc, name)
            .ok_or_else(|| request(format!("model {} has no entity {id}", desc.id)))?;
        let species = species_of(desc, index, entity);
        let target = match (concentration, species) {
            (Some(_), Some(species)) => Target::Concentration(species),
            (Some(_), None) => {
                return Err(request(format!(
                    "{id} of model {} is no concentration: {name} is no species",
                    desc.id
                )));
            }
            (None, Some(species)) => Target::Amount(species),
            (None, None) => Target::Entity(index),
        };
        if !entity.settable {
            return Err(request(format!(
                "{id} of model {} is not settable",
                desc.id
            )));
        }
        if !value.is_finite() {
            return Err(request(format!(
                "the value {value} of {id} {place} is not finite"
            )));
        }
        if seen.contains(&index) {
            return Err(request(format!("{name} is set more than once {place}")));
        }
        seen.push(index);
        resolved.push((target, *value));
    }
    Ok(resolved)
}

impl Changes {
    /// Resolves and checks the changes of a request: finite times in `[start, end]` in
    /// non-decreasing order, the values as [`resolve_values`] checks them. The changes of one
    /// time are one change, applied at once.
    pub fn new(
        desc: &ModelDescription,
        changes: &[Change],
        start: f64,
        end: f64,
    ) -> Result<Changes, EngineError> {
        let mut merged: Vec<(f64, Vec<(String, f64)>)> = Vec::with_capacity(changes.len());
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
                Some((time, values)) if *time == t => values.extend(change.values.iter().cloned()),
                _ => merged.push((t, change.values.clone())),
            }
            previous = t;
        }
        let resolved = merged
            .iter()
            .map(|(time, values)| {
                Ok(ResolvedChange {
                    time: *time,
                    values: resolve_values(desc, values, &format!("at t = {time}"))?,
                })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let rescaled = desc
            .parameters
            .iter()
            .chain(&desc.states)
            .enumerate()
            .filter_map(|(index, entity)| match &entity.kind {
                EntityKind::Species {
                    has_only_substance_units: false,
                    stores_concentration: false,
                    ..
                } if entity.settable => species_of(desc, index, entity),
                _ => None,
            })
            .map(|s| Rescaled {
                species: s.index,
                compartment: s.compartment,
            })
            .collect();
        Ok(Changes {
            changes: resolved,
            rescaled,
            n_parameters: desc.parameters.len(),
        })
    }

    /// The time of change `k`, `None` past the last one.
    pub fn time(&self, k: usize) -> Option<f64> {
        self.changes.get(k).map(|c| c.time)
    }

    /// Applies change `k` at its time to the state `y` (the `m` values of the model) and the
    /// parameters `p`, then evaluates the assignment rules.
    pub fn apply(&self, k: usize, model: &dyn ModelFns, y: &mut [f64], p: &mut [f64]) {
        let change = &self.changes[k];
        // the values of the assignment rules at the time of the change, so the size of a
        // compartment of a rule which converts a value is the current one
        model.assign(change.time, y, p);
        let np = self.n_parameters;
        let get = |y: &[f64], p: &[f64], i: usize| if i < np { p[i] } else { y[i - np] };
        let set = |y: &mut [f64], p: &mut [f64], i: usize, v: f64| {
            if i < np {
                p[i] = v;
            } else {
                y[i - np] = v;
            }
        };
        let targeted = |i: usize| {
            change.values.iter().any(|(target, _)| match target {
                Target::Entity(e) => *e == i,
                Target::Amount(s) | Target::Concentration(s) => s.index == i,
            })
        };
        // the compartments first, each keeping the concentrations of its species
        for &(target, value) in &change.values {
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
        for &(target, value) in &change.values {
            if matches!(target, Target::Entity(_)) {
                continue;
            }
            let size = target.compartment().map_or(f64::NAN, |c| get(y, p, c));
            set(y, p, target.index(), target.stored(value, size));
        }
        // the assignment rules, at the time of the change
        model.assign(change.time, y, p);
    }
}

//! The construction of the [`Model`] from the data model of the reader.

mod entities;
mod outputs;
mod rates;

use outputs::ordered;

use std::collections::HashMap;

use sbmlsim_sbml::math::Node;
use sbmlsim_sbml::{Event as SbmlEvent, Model as Sbml, Rule, SpeciesReference};

use crate::expr::{self, Context, Scope};
use crate::order::order;
use crate::units::Units;
use crate::{
    Assignment, CompileError, Entity, EntityKind, Event, EventAssignment, Expr, Model, Op,
    SelectionDef, SelectionKind, SpeciesValue, Symbol, index,
};

/// What an SBML id names.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    Compartment(usize),
    Species(usize),
    Parameter(usize),
    /// A species reference with an id, by its position in `Builder::refs`.
    SpeciesReference(usize),
    Reaction(usize),
    Function(usize),
    Event,
}

/// A species reference of a reaction.
struct Ref<'m> {
    reaction: usize,
    reference: &'m SpeciesReference,
    product: bool,
}

/// Where the rate of a state comes from.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// The rate rule of this position in the list of rules.
    RateRule(usize),
    /// The reactions, for the species of this position.
    Reactions(usize),
}

/// What the math needs to know of a species.
#[derive(Debug, Clone, Copy)]
struct SpeciesInfo {
    symbol: Symbol,
    compartment: Symbol,
    compartment_index: usize,
    has_only_substance_units: bool,
    value: SpeciesValue,
}

struct Builder<'m> {
    m: &'m Sbml,
    items: HashMap<&'m str, Item>,
    refs: Vec<Ref<'m>>,
    symbols: HashMap<&'m str, Symbol>,
    /// The position in the list of rules of the assignment rule of an id.
    assignment_rule: HashMap<&'m str, usize>,
    /// The position in the list of rules of the rate rule of an id.
    rate_rule: HashMap<&'m str, usize>,
    species: Vec<SpeciesInfo>,
    /// The ids of the states, in the order of `y`.
    state_ids: Vec<&'m str>,
    sources: Vec<Source>,
    reaction_rates: HashMap<usize, Expr>,
    state_rates: HashMap<usize, Expr>,
    /// The rates being built (reaction and state ids), to name a cycle through `rateOf`.
    busy: Vec<String>,
}

fn invalid(message: String) -> CompileError {
    CompileError::Invalid(message)
}

fn unsupported(feature: &str, element: &str) -> CompileError {
    CompileError::Unsupported {
        feature: feature.to_string(),
        element: element.to_string(),
    }
}

/// The name of an element in an error: its id, else its kind and position.
fn name(id: Option<&String>, kind: &str, position: usize, target: Option<&str>) -> String {
    match (id, target) {
        (Some(id), _) => id.clone(),
        (None, Some(t)) => format!("{kind} #{position} ({t})"),
        (None, None) => format!("{kind} #{position}"),
    }
}

fn rule_name(position: usize, rule: &Rule) -> String {
    match rule {
        Rule::Assignment { id, variable, .. } => {
            name(id.as_ref(), "assignmentRule", position, Some(variable))
        }
        Rule::Rate { id, variable, .. } => name(id.as_ref(), "rateRule", position, Some(variable)),
        Rule::Algebraic { id, .. } => name(id.as_ref(), "algebraicRule", position, None),
    }
}

fn event_name(position: usize, event: &SbmlEvent) -> String {
    name(event.id.as_ref(), "event", position, None)
}

/// A sum of terms as written, `0` without terms.
fn sum(mut terms: Vec<Expr>) -> Expr {
    match terms.len() {
        0 => Expr::Number(0.0),
        1 => terms.pop().expect("one term"),
        _ => Expr::apply(Op::Plus, terms),
    }
}

pub(crate) fn build(m: &Sbml) -> Result<Model, CompileError> {
    refuse_unsupported(m)?;
    let mut b = Builder::new(m)?;
    b.classify()?;
    let (states, parameters) = b.entities();

    let rates = (0..b.state_ids.len())
        .map(|k| b.state_rate(k))
        .collect::<Result<Vec<_>, _>>()?;

    // the assignment rules, built once for both lists
    let mut rules = Vec::new();
    for (position, rule) in m.rules.iter().enumerate() {
        if let Rule::Assignment { variable, math, .. } = rule {
            let element = rule_name(position, rule);
            let e = expr::build(math, &mut b, &Scope::global(&element))?;
            rules.push((b.symbols[variable.as_str()], e));
        }
    }
    let id_of = |s: Symbol| match s {
        Symbol::State(i) => states[i as usize].id.clone(),
        Symbol::Param(i) => parameters[i as usize].id.clone(),
        Symbol::Time => "time".to_string(),
    };
    let assignment_rules = ordered(rules.clone(), id_of, "assignment rules")?;
    let mut entries = b.initial_values()?;
    entries.extend(b.initial_assignments()?);
    entries.extend(rules);
    let initial = ordered(entries, id_of, "initial assignments and assignment rules")?;

    let events = m
        .events
        .iter()
        .enumerate()
        .map(|(k, e)| b.event(k, e))
        .collect::<Result<_, _>>()?;
    let selections = b.selections()?;
    Ok(Model {
        id: m.id.clone(),
        states,
        parameters,
        assignment_rules,
        initial,
        rates,
        events,
        selections,
    })
}

/// The features refused before anything is built: algebraic rules and fast reactions.
fn refuse_unsupported(m: &Sbml) -> Result<(), CompileError> {
    for (position, rule) in m.rules.iter().enumerate() {
        if let Rule::Algebraic { .. } = rule {
            return Err(unsupported("algebraic rule", &rule_name(position, rule)));
        }
    }
    if let Some(r) = m.reactions.iter().find(|r| r.fast) {
        return Err(unsupported("fast reaction", &r.id));
    }
    Ok(())
}

//! The model representation of sbmlsim-core and its construction from an SBML document.
//!
//! [`build`] turns the [`Document`] of the reader into a [`Model`]: the
//! vectors of the states `y` and of the parameters `p`, the assignment rules in their order of
//! dependency, the initial evaluations in their order of dependency, one rate or residual per state, the
//! events and the selections, every math an [`Expr`] over resolved [`Symbol`]s. Code generation
//! (`sbmlsim-codegen`) emits the functions of `model/2` from it, as written or as the programs of the
//! optimizer ([`optimize`]), with the Jacobian of [`derive`](mod@derive) and the meanings of
//! [`eval`]; nothing here does file system, environment or OS access, so the crate builds for
//! `wasm32-unknown-unknown`.
//!
//! # States and parameters
//!
//! `y` holds what is integrated, in the order of libroadrunner's state vector: first the target
//! of every rate rule in the order of the rules (a compartment, a parameter, a species or a
//! species reference), then every species which reactions change (neither constant nor a
//! boundary condition nor the target of a rule) in the order of the species. Algebraic
//! unknowns follow in the order of their matched algebraic rules. `p` holds
//! everything else in the order compartments, species, parameters, species references with an
//! id, each in document order: the constants, the boundary and constant species, the entities
//! only events change and the targets of assignment rules (which, like algebraic unknowns, are not settable,
//! [`Entity::settable`]).
//!
//! # Species
//!
//! A species is stored as its amount ([`SpeciesValue::Amount`]), so a change of its
//! compartment keeps its amount and changes its concentration, and its rate is an amount rate:
//! the sum over the reactions of stoichiometry times rate (a reactant `-(s * v)`, a product
//! `s * v`, in the order of the reactions and of their reactants and products), times the
//! conversion factor of the species, else of the model. Where the math reads a species of a
//! concentration (`hasOnlySubstanceUnits` false) it reads `amount / compartment`. The exception
//! is a species of a concentration which is the target of a rule: it stores its concentration
//! ([`SpeciesValue::Concentration`]), the value the rule gives, as libroadrunner does (its
//! amount is `concentration * compartment`, and a rate rule keeps the concentration when the
//! compartment changes). The selection `S` is the amount, `[S]` the concentration.
//!
//! # Values
//!
//! An entity's [`Entity::default`] is the value its SBML attribute gives in the quantity it
//! stores. A missing value is the one libroadrunner 2.11 sets: a compartment without a size 1,
//! a species without an initial amount or concentration 0, a species reference without a
//! stoichiometry 1 (also inlined where it has no id), a local parameter without a value NaN; a
//! parameter without a value is refused (`Unsupported`, `parameter without a value`) unless an
//! initial assignment or an assignment rule gives it one, also when a rate rule or an event
//! changes it, as libroadrunner refuses to load it. An algebraic parameter without a value
//! starts with a finite guess for the consistency solve. The default is NaN for a species given in
//! the other quantity and for a parameter without a value which is assigned.
//! [`Model::initial`] completes them: the amount of a species given as a concentration
//! (`initialConcentration * compartment`, the compartment as evaluated at init), the
//! concentration of a species of a rule given as an amount, the initial assignments (of a
//! species of a concentration stored as an amount: `value * compartment`) and the initial
//! values of the assignment rules, all in one order of dependency.
//!
//! # Missing math
//!
//! As libroadrunner 2.11 does: a rule (also an algebraic rule) or an initial assignment without
//! math is ignored, as if it were not there (it does not give a parameter a value, and its
//! variable is not looked at); an event without a trigger, or whose trigger has no math, is
//! ignored with everything in it; an event assignment without math is ignored; a delay or a
//! priority without math is none. Refused (`Unsupported`): a kinetic law without math
//! (`kinetic law without math`), a call of a function definition without math or with an empty
//! math (`function definition without math`; libroadrunner crashes or refuses), and a math
//! without content, an empty `<math>`, wherever it is compiled (`math without content`;
//! libroadrunner: "Unknown ASTNode type of 9999").

mod builder;
pub mod derive;
mod discontinuity;
pub mod eval;
mod expr;
pub mod optimize;
mod order;
mod units;

pub use expr::{AVOGADRO, Expr, Op, Symbol};

use sbmlsim_sbml::Document;
use thiserror::Error;

/// A failure to build the model representation.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CompileError {
    /// A feature the engine does not simulate as sbmlsim does.
    #[error("{feature} is not supported ({element})")]
    Unsupported {
        /// The feature, e.g. `fast reaction`.
        feature: String,
        /// The element: its id, or its kind and position (`algebraicRule #0`).
        element: String,
    },
    /// A model which is not valid SBML, e.g. an unknown identifier or a cycle of assignment
    /// rules.
    #[error("{0}")]
    Invalid(String),
}

/// What a species stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeciesValue {
    /// Its amount.
    Amount,
    /// Its concentration (a species of a concentration which is the target of a rule).
    Concentration,
}

/// The kind of an [`Entity`].
#[derive(Debug, Clone, PartialEq)]
pub enum EntityKind {
    /// A species.
    Species {
        /// The compartment.
        compartment: Symbol,
        /// The `hasOnlySubstanceUnits` attribute: the math reads the amount.
        has_only_substance_units: bool,
        /// The `boundaryCondition` attribute: reactions do not change it.
        boundary_condition: bool,
        /// The `constant` attribute.
        constant: bool,
        /// What the entity stores.
        value: SpeciesValue,
    },
    /// A compartment, its size.
    Compartment,
    /// A parameter.
    Parameter,
    /// A species reference with an id, its stoichiometry.
    SpeciesReference,
}

/// A state or a parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// The SBML id.
    pub id: String,
    /// What it is.
    pub kind: EntityKind,
    /// The value of its SBML attribute in the quantity it stores, a missing one as libroadrunner
    /// sets it (see the crate documentation), NaN until [`Model::initial`] when it is given in
    /// the other quantity or assigned.
    pub default: f64,
    /// Whether a pre-initialization or a change may set it. Assignment rules and algebraic
    /// constraints determine their own dependent values.
    pub settable: bool,
    /// The unit expression of its value, when SBML gives one.
    pub unit: Option<String>,
}

/// A state, an entry of `y`.
pub type State = Entity;
/// A parameter, an entry of `p`.
pub type Param = Entity;

/// `target = expr`.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// The symbol assigned, a state or a parameter.
    pub target: Symbol,
    /// The value in the quantity the target stores.
    pub expr: Expr,
}

/// An assignment of an event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventAssignment {
    /// The symbol assigned.
    pub target: Symbol,
    /// The value of the math, in the quantity the math sees.
    pub value: Expr,
    /// For a species of a concentration (`hasOnlySubstanceUnits` false), its compartment: the
    /// amount of the species after the event is `value * compartment`, the compartment before
    /// the assignments of the event (as libroadrunner 2.11 and the expected results of the SBML
    /// Test Suite, case 01506, also when the event assigns the compartment).
    pub compartment: Option<Symbol>,
}

/// An event; its semantics are the engine's.
///
/// The engine locates the times at which a trigger may change by the zeros of its
/// [`Event::root_functions`] and evaluates the trigger itself there.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// The id, absent for an event named by its position.
    pub id: Option<String>,
    /// The trigger.
    pub trigger: Expr,
    /// The value of the trigger before the time 0.
    pub initial_value: bool,
    /// Whether the trigger is persistent.
    pub persistent: bool,
    /// Whether the assignments use the values at the time of the trigger.
    pub use_values_from_trigger_time: bool,
    /// The delay.
    pub delay: Option<Expr>,
    /// The priority.
    pub priority: Option<Expr>,
    /// The assignments, in document order.
    pub assignments: Vec<EventAssignment>,
}

impl Event {
    /// The root functions of the trigger: one per comparison (for a chain, one per adjacent
    /// pair), positive where the comparison holds (`a - b` of `a > b` and `a >= b`, `b - a` of
    /// `a < b` and `a <= b`, `a - b` of `a == b` and `a != b`), and one per expression read as a
    /// boolean which is no boolean operator (a number, which is true where it is not 0), in
    /// the written order. The trigger can change its value only where one of them changes its
    /// sign, or at an event.
    pub fn root_functions(&self) -> Vec<Expr> {
        let mut out = Vec::new();
        roots_of(&self.trigger, true, &mut out);
        out
    }
}

/// Collects the root functions of `e` into `out`; `boolean` is whether `e` is read as a boolean.
fn roots_of(e: &Expr, boolean: bool, out: &mut Vec<Expr>) {
    match e {
        Expr::Number(_) | Expr::Bool(_) => {}
        Expr::Symbol(_) => {
            if boolean {
                out.push(e.clone());
            }
        }
        Expr::Apply { op, args } => match op {
            Op::Delay => {
                let start = out.len();
                roots_of(&args[0], boolean, out);
                for root in &mut out[start..] {
                    let historical = std::mem::replace(root, Expr::Number(0.));
                    *root = Expr::apply(Op::Delay, vec![historical, args[1].clone()]);
                }
                roots_of(&args[1], false, out);
            }
            Op::Eq | Op::Neq | Op::Gt | Op::Geq | Op::Lt | Op::Leq => {
                for pair in args.windows(2) {
                    let (a, b) = (pair[0].clone(), pair[1].clone());
                    out.push(match op {
                        Op::Lt | Op::Leq => Expr::apply(Op::Minus, vec![b, a]),
                        _ => Expr::apply(Op::Minus, vec![a, b]),
                    });
                }
                for a in args {
                    roots_of(a, false, out);
                }
            }
            Op::And | Op::Or | Op::Xor | Op::Not | Op::Implies => {
                for a in args {
                    roots_of(a, true, out);
                }
            }
            _ => {
                if boolean {
                    out.push(e.clone());
                }
                for a in args {
                    roots_of(a, false, out);
                }
            }
        },
        Expr::Piecewise { pieces, otherwise } => {
            if boolean && !is_boolean_valued(e) {
                out.push(e.clone());
            }
            for (value, condition) in pieces {
                roots_of(value, boolean, out);
                roots_of(condition, true, out);
            }
            if let Some(o) = otherwise {
                roots_of(o, boolean, out);
            }
        }
    }
}

/// Whether every value of a piecewise is a boolean (a relation, a boolean operator or a
/// boolean constant), so it is no number read as a boolean.
pub(crate) fn is_boolean_valued(e: &Expr) -> bool {
    match e {
        Expr::Bool(_) => true,
        Expr::Apply { op, .. } => matches!(
            op,
            Op::Eq
                | Op::Neq
                | Op::Gt
                | Op::Geq
                | Op::Lt
                | Op::Leq
                | Op::And
                | Op::Or
                | Op::Xor
                | Op::Not
                | Op::Implies
        ),
        Expr::Piecewise { pieces, otherwise } => {
            pieces.iter().all(|(v, _)| is_boolean_valued(v))
                && otherwise.as_deref().is_none_or(is_boolean_valued)
        }
        _ => false,
    }
}

/// What a selection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// The amount of a species, `S`.
    Amount,
    /// The concentration of a species, `[S]`.
    Concentration,
    /// The size of a compartment.
    Compartment,
    /// A parameter.
    Parameter,
    /// The stoichiometry of a species reference.
    SpeciesReference,
    /// The rate of a reaction.
    ReactionRate,
}

/// A value of an output row.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionDef {
    /// The id, an SBML id or `[S]` for the concentration of the species `S`.
    pub id: String,
    /// What it is.
    pub kind: SelectionKind,
    /// The value.
    pub expr: Expr,
    /// The unit expression, when SBML gives one.
    pub unit: Option<String>,
}

/// The model representation.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The id of the model.
    pub id: Option<String>,
    /// The states, the vector `y`.
    pub states: Vec<State>,
    /// The number of trailing states determined by algebraic rules, in rule order.
    pub n_algebraic: u32,
    /// The parameters, the vector `p`.
    pub parameters: Vec<Param>,
    /// The assignment rules, in their order of dependency (ties in document order).
    pub assignment_rules: Vec<Assignment>,
    /// The initial evaluations (see the crate documentation), in their order of dependency
    /// (ties: the species given in the other quantity, the initial assignments, the assignment
    /// rules, each in document order).
    pub initial: Vec<Assignment>,
    /// The differential rates followed by the algebraic residuals, in the order of `states`.
    pub rates: Vec<Expr>,
    /// The events, in document order.
    pub events: Vec<Event>,
    /// The selections: every compartment, every species as `S` and `[S]`, every parameter,
    /// every species reference with an id and every reaction, in this order.
    pub selections: Vec<SelectionDef>,
}

impl Model {
    /// The symbol of an entity id.
    pub fn symbol(&self, id: &str) -> Option<Symbol> {
        let find = |v: &[Entity]| v.iter().position(|e| e.id == id).map(index);
        find(&self.states)
            .map(Symbol::State)
            .or_else(|| find(&self.parameters).map(Symbol::Param))
    }

    /// The entity of a symbol of a state or a parameter.
    ///
    /// # Panics
    ///
    /// For [`Symbol::Time`], a [`Symbol::Derivative`], a [`Symbol::Common`], a
    /// [`Symbol::Constant`] or an index out of range.
    pub fn entity(&self, symbol: Symbol) -> &Entity {
        match symbol {
            Symbol::State(i) => &self.states[i as usize],
            Symbol::Param(i) => &self.parameters[i as usize],
            Symbol::Time | Symbol::Derivative(_) | Symbol::Common(_) | Symbol::Constant(_) => {
                panic!("{symbol:?} is no entity")
            }
        }
    }

    /// The rate of the state `id`.
    pub fn rate(&self, id: &str) -> Option<&Expr> {
        match self.symbol(id)? {
            Symbol::State(i) if (i as usize) < self.states.len() - self.n_algebraic as usize => {
                Some(&self.rates[i as usize])
            }
            _ => None,
        }
    }

    /// The selection `id`.
    pub fn selection(&self, id: &str) -> Option<&SelectionDef> {
        self.selections.iter().find(|s| s.id == id)
    }
}

/// An index of `p` or `y`.
pub(crate) fn index(i: usize) -> u32 {
    u32::try_from(i).expect("fewer than 2^32 entities")
}

/// Builds the model representation of a document.
pub fn build(doc: &Document) -> Result<Model, CompileError> {
    builder::build(&doc.model)
}

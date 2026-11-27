//! The owned data model of an SBML document.
//!
//! Plain structs which a reader fills and the report builder reads. Every list
//! keeps the order of the document, so the position of an element is its index
//! in its list.

mod document;
mod element;
mod math;

pub use document::{
    Compartment, Constraint, Document, Event, EventAssignment, FunctionDefinition,
    InitialAssignment, KineticLaw, LocalParameter, MathElement, Model, ModelUnits,
    ModifierSpeciesReference, Package, Parameter, Reaction, Rule, RuleKind, Species,
    SpeciesReference, Trigger, Unit, UnitDefinition,
};
pub use element::{Attribute, Creator, CvTerm, Element, Extension, History, ListMetadata};
pub use math::{Ast, AstKind, AstNumber, MAX_AST_DEPTH, scientific_value};

pub mod formatting;
pub mod locations;
pub mod packages;

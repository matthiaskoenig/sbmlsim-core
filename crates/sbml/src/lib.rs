//! Reader of SBML Level 3 Version 2 core and its MathML.
//!
//! [`read`] takes the text of a document and gives a [`Document`], a plain data model of the
//! elements the compiler needs, without any file system access. Every list keeps the order of
//! the document, so the position of an element is its index in its list (an event without an id
//! is named by its position). A number keeps the double of its text.
//!
//! Packages: a package whose `required` attribute on `<sbml>` is `"true"` is refused with
//! [`ReadError::Package`] (the package name is read from its namespace URI, e.g. `comp`; comp
//! must be flattened before); a package with `required="false"` (layout, render, fbc
//! annotations, ...) is ignored.
//!
//! Units pass through as text and nothing is computed with them. A unit definition becomes a
//! unit expression a unit registry such as pint parses, the units joined by `*`, each as its
//! kind with `**exponent` (when not 1), preceded by the factor `multiplier * 10^scale` when not
//! 1, e.g. `mmol/min` as `1e-3*mole*60*second**-1` (the multiplier belongs to its own unit).

mod model;

pub mod math;

pub use model::*;

use thiserror::Error;

/// A failure to read a document.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ReadError {
    /// The document is not SBML Level 3 Version 2.
    #[error("SBML level {level} version {version} is not supported, only level 3 version 2")]
    NotLevel3Version2 {
        /// The level of the document.
        level: u32,
        /// The version of the document.
        version: u32,
    },
    /// A required package other than core.
    #[error("the required package {0} is not supported")]
    Package(String),
    /// Malformed XML, a missing or invalid attribute or element, or an unknown MathML element.
    #[error("{0}")]
    Xml(String),
}

/// An SBML document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The SBML level, 3.
    pub level: u32,
    /// The SBML version, 2.
    pub version: u32,
    /// The model.
    pub model: Model,
}

/// Reads the text of an SBML Level 3 Version 2 document.
pub fn read(sbml: &str) -> Result<Document, ReadError> {
    model::read_document(sbml)
}

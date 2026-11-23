//! Reader of SBML core, every level and version, and its MathML.
//!
//! ```
//! let document = sbmlsim_sbml::read(
//!     r#"<?xml version="1.0" encoding="UTF-8"?>
//!        <sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
//!          <model id="decay">
//!            <listOfParameters>
//!              <parameter id="y" value="10" constant="false"/>
//!              <parameter id="k" value="0.1" constant="true"/>
//!            </listOfParameters>
//!            <listOfRules>
//!              <rateRule variable="y">
//!                <math xmlns="http://www.w3.org/1998/Math/MathML">
//!                  <apply><minus/><apply><times/><ci>k</ci><ci>y</ci></apply></apply>
//!                </math>
//!              </rateRule>
//!            </listOfRules>
//!          </model>
//!        </sbml>"#,
//! )?;
//! assert_eq!((document.level, document.version), (3, 2));
//! assert_eq!(document.model.parameters.len(), 2);
//! assert_eq!(document.model.rules.len(), 1);
//! # Ok::<(), sbmlsim_sbml::ReadError>(())
//! ```
//!
//! [`read`] takes the text of a document and gives a [`Document`], a plain data model of the
//! elements the compiler needs, without any file system access. Every list keeps the order of
//! the document, so the position of an element is its index in its list (an event without an id
//! is named by its position). A number keeps the double of its text.
//!
//! Missing math: an element without a `<math>` child (a rule, an initial assignment, a trigger, an event assignment, a kinetic law, a function definition) has its math absent, a `<delay>` or `<priority>` without one is read as absent, an event without a `<trigger>` has its trigger absent, and a `<math>` without an element in it is [`math::Node::Empty`]. What they mean is the compiler's decision.
//!
//! Packages: a package whose `required` attribute on `<sbml>` is `"true"` is refused with
//! [`ReadError::Package`] (the package name is read from its namespace URI, e.g. `comp`; comp
//! must be flattened before, a model read without its submodels is another model); a package
//! with `required="false"` (layout, render, fbc annotations, ...) is ignored. A required
//! package which annotates and computes nothing is read as well and its elements are ignored,
//! so far `distrib`: a `<distrib:uncertainty>` of a parameter is the uncertainty of the fit
//! which gave its value, and the mathematics of the model is the same without it; such a
//! package is matched by its namespace URI, never by its prefix, and only the elements of an
//! uncertainty are read of it, every other element of its namespace being refused instead of
//! ignored. What a package computes is refused whatever `required` says, in either shape a
//! draw of `distrib` has: a `<csymbol>` of a distribution in a math of the model is an unknown
//! csymbol, and a `<distrib:drawFromDistribution>` element, the draw of the drafts of the
//! package, is an element of the namespace which is no uncertainty, so neither is dropped on
//! the way to a deterministic simulation.
//!
//! Units pass through as text and nothing is computed with them. A unit definition becomes a unit expression a unit registry such as pint parses, the units joined by `*`; a unit is `(multiplier * 10^scale * kind)^exponent`, written as `kind`, `factor*kind`, `kind**e` or `(factor*kind)**e` where the factor is the double of the text `{multiplier}e{scale}`, e.g. `mmol/min` as `0.001*mole*(60*second)**-1`.

mod model;

pub mod formula;
pub mod math;

pub use model::*;

use thiserror::Error;

/// A failure to read a document.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ReadError {
    /// The document is no level and version of SBML core the reader knows.
    #[error("SBML level {level} version {version} is not supported")]
    UnsupportedLevel {
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
    /// The SBML level, 1, 2 or 3.
    pub level: u32,
    /// The version of the level.
    pub version: u32,
    /// The model.
    pub model: Model,
}

/// Reads the text of an SBML core document of any level and version.
///
/// A document of Level 1 or Level 2 is read into the model of Level 3: the built in units of
/// the older levels become unit definitions and the defaults of the model, and what a later
/// level states differently is read as the older level means it.
pub fn read(sbml: &str) -> Result<Document, ReadError> {
    if deeper_than(sbml, MAX_XML_DEPTH).is_some() {
        return Err(ReadError::Xml(format!(
            "the document is nested deeper than {MAX_XML_DEPTH} elements"
        )));
    }
    model::read_document(sbml)
}

/// The deepest nesting of elements a document may have: the structure of SBML around a math
/// (`sbml`, `model`, a list, an element, a part of it such as `trigger`, `math`), annotations
/// and notes, and [`math::MAX_MATH_DEPTH`] below the math. The XML parser recurses once per
/// element, so a deeper document is refused before it is parsed.
pub const MAX_XML_DEPTH: usize = math::MAX_MATH_DEPTH + 64;

/// The depth of the first element of `xml` deeper than `limit`, scanning its tags without
/// recursion; `None` when there is none (or when the text is no XML the scan understands, which
/// the parser then refuses).
fn deeper_than(xml: &str, limit: usize) -> Option<usize> {
    let bytes = xml.as_bytes();
    // the position after the first occurrence of `end` at or after `from`
    let after = |from: usize, end: &[u8]| {
        bytes[from..]
            .windows(end.len())
            .position(|w| w == end)
            .map(|i| from + i + end.len())
    };
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(lt) = bytes[i..].iter().position(|&b| b == b'<') {
        let at = i + lt;
        let rest = &bytes[at..];
        i = if rest.starts_with(b"<!--") {
            after(at, b"-->")?
        } else if rest.starts_with(b"<![CDATA[") {
            after(at, b"]]>")?
        } else if rest.starts_with(b"<?") {
            after(at, b"?>")?
        } else if rest.starts_with(b"<!") {
            // a document type declaration, with an internal subset in brackets
            let mut brackets = 0usize;
            let mut j = at + 2;
            loop {
                match bytes.get(j)? {
                    b'[' => brackets += 1,
                    b']' => brackets = brackets.saturating_sub(1),
                    b'>' if brackets == 0 => break j + 1,
                    _ => {}
                }
                j += 1;
            }
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            after(at, b">")?
        } else {
            // a start tag: up to its `>` outside of the quotes of its attributes
            let mut quote = None;
            let mut j = at + 1;
            let end = loop {
                let b = *bytes.get(j)?;
                match quote {
                    Some(q) if b == q => quote = None,
                    Some(_) => {}
                    None if b == b'"' || b == b'\'' => quote = Some(b),
                    None if b == b'>' => break j,
                    None => {}
                }
                j += 1;
            };
            if bytes[end - 1] != b'/' {
                depth += 1;
                if depth > limit {
                    return Some(depth);
                }
            }
            end + 1
        };
    }
    None
}

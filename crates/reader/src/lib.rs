//! A pure Rust SBML reader which preserves authored report data.
//!
//! The reader has no file system or network access. Namespace URIs identify
//! SBML and MathML, while the spelling of user prefixes remains part of XML.

mod document;
mod encoding;
mod formula;
mod math;
mod metadata;
mod packages;
mod xml;
pub use document::read;

pub use formula::read_formula;
pub use math::read_math;

/// An input which cannot be represented by the report's owned SBML model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReadError(pub String);

pub(crate) const MATHML: &str = "http://www.w3.org/1998/Math/MathML";

pub(crate) fn elements<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    node.children().filter(roxmltree::Node::is_element)
}

pub(crate) fn text(node: roxmltree::Node) -> String {
    node.children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Supplement the temporary native oracle's core model with reader-owned packages.
/// Native attributes and metadata remain independent comparison inputs.
///
/// # Errors
/// Returns a reader error for malformed XML or unsupported package math.
pub fn supplement(source: &[u8], target: &mut sbmlsim_document::Document) -> Result<(), ReadError> {
    let mut parsed = read(source)?;
    let mut extras = Vec::new();
    parsed.visit_elements_mut(|element| {
        extras.push((
            std::mem::take(&mut element.package_attributes),
            std::mem::take(&mut element.extensions),
            std::mem::take(&mut element.lists),
        ));
    });
    target.model_definitions = parsed.model_definitions;
    target.main_model_fallback = parsed.main_model_fallback;
    let count = extras.len();
    let mut extras = extras.into_iter();
    let mut visited = 0;
    target.visit_elements_mut(|element| {
        visited += 1;
        if let Some((attributes, extensions, lists)) = extras.next() {
            element.package_attributes = attributes;
            element.extensions = extensions;
            element.lists = lists;
        }
    });
    if visited != count {
        return Err(ReadError(format!(
            "The native oracle and Rust reader disagree on the core skeleton ({visited} versus {count} SBase objects)."
        )));
    }
    Ok(())
}

//! Namespace-aware package objects and metadata-bearing ListOf containers.

use crate::{MATHML, ReadError, elements, math, metadata, xml};
use roxmltree::Node;
use sbmlsim_document::{Attribute, Document, Element, Extension, ListMetadata};
use std::collections::HashMap;

pub(crate) fn name(uri: Option<&str>) -> Option<&'static str> {
    let uri = uri?;
    if uri == "http://sbml.org/fbc/keyvaluepair" {
        return Some("fbc");
    }
    if !uri.starts_with("http://www.sbml.org/sbml/level3/") {
        return None;
    }
    if uri.contains("/comp/version") {
        Some("comp")
    } else if uri.contains("/fbc/version") {
        Some("fbc")
    } else if uri.contains("/qual/version") {
        Some("qual")
    } else if uri.contains("/distrib/version") {
        Some("distrib")
    } else {
        None
    }
}
fn attrs(node: Node) -> Vec<Attribute> {
    node.attributes()
        .map(|a| Attribute {
            namespace: a.namespace().map(str::to_owned),
            name: a.name().to_owned(),
            value: a.value().to_owned(),
        })
        .collect()
}
fn states(element: &Element) -> bool {
    element.id.is_some()
        || element.meta_id.is_some()
        || element.name.is_some()
        || element.sbo.is_some()
        || element.notes.is_some()
        || element.annotation.is_some()
}
fn extension(
    node: Node,
    level: u32,
    depth: usize,
    positions: &metadata::Positions,
) -> Result<Extension, ReadError> {
    if depth > 128 {
        return Err(ReadError(
            "SBML package nesting exceeds 128 objects.".into(),
        ));
    }
    let package = name(node.tag_name().namespace()).unwrap_or_default();
    let mut element = metadata::element(
        node,
        level,
        !node.tag_name().name().starts_with("listOf")
            && !matches!(
                node.tag_name().name(),
                "geneProductAssociation" | "geneProductRef" | "and" | "or"
            ),
        positions,
    );
    fill(node, level, &mut element, depth + 1, positions)?;
    // Descendant package objects belong to this tree, not to the generic SBase extension.
    element.extensions.retain(|e| e.package != package);
    let mut children = Vec::new();
    for child in elements(node).filter(|n| name(n.tag_name().namespace()) == Some(package)) {
        children.push(extension(child, level, depth + 1, positions)?);
    }
    let math = elements(node)
        .find(|n| n.tag_name().namespace() == Some(MATHML) && n.tag_name().name() == "math")
        .map(math::from_node)
        .transpose()?
        .flatten();
    Ok(Extension {
        element,
        package: package.to_owned(),
        name: node.tag_name().name().to_owned(),
        attributes: attrs(node),
        math,
        children,
    })
}
fn fill(
    node: Node,
    level: u32,
    element: &mut Element,
    depth: usize,
    positions: &metadata::Positions,
) -> Result<(), ReadError> {
    element.package_attributes = attrs(node)
        .into_iter()
        .filter(|a| name(a.namespace.as_deref()).is_some())
        .collect();
    for child in elements(node) {
        let child_name = child.tag_name().name();
        if let Some(package) = name(child.tag_name().namespace())
            && !matches!(child_name, "listOfModelDefinitions" | "modelDefinition")
            && (name(node.tag_name().namespace()) != Some(package)
                || node.tag_name().name() == "modelDefinition")
        {
            element
                .extensions
                .push(extension(child, level, depth + 1, positions)?);
        }
        if child_name.starts_with("listOf")
            && (child.tag_name().namespace()
                == node.document().root_element().tag_name().namespace()
                || name(child.tag_name().namespace()).is_some())
        {
            let mut base = metadata::element(child, level, false, positions);
            if states(&base) {
                fill(child, level, &mut base, depth + 1, positions)?;
                base.xml = Some(xml::list_fragment(child, level));
                element.lists.push(ListMetadata {
                    element: base,
                    name: child_name.to_owned(),
                    size: elements(child)
                        .filter(|n| {
                            n.tag_name().namespace() == child.tag_name().namespace()
                                && !matches!(
                                    n.tag_name().name(),
                                    "defaultTerm" | "notes" | "annotation"
                                )
                        })
                        .count(),
                });
            }
        }
    }
    // fbc key-value annotations are a package API on their immediate SBase owner.
    if node.document().root_element().namespaces().any(|ns| {
        name(Some(ns.uri())) == Some("fbc") && ns.uri() != "http://sbml.org/fbc/keyvaluepair"
    }) && let Some(annotation) = elements(node).find(|n| n.tag_name().name() == "annotation")
    {
        for list in elements(annotation).filter(|n| {
            name(n.tag_name().namespace()) == Some("fbc")
                && n.tag_name().name() == "listOfKeyValuePairs"
        }) {
            element
                .extensions
                .push(extension(list, level, depth + 1, positions)?);
        }
    }
    Ok(())
}
pub(crate) fn apply(
    xml: &roxmltree::Document,
    level: u32,
    document: &mut Document,
    locations: &metadata::Positions,
) -> Result<(), ReadError> {
    let positions: HashMap<_, _> = xml
        .descendants()
        .filter(Node::is_element)
        .map(|n| {
            let pos = locations.at(n.range().start);
            (pos, n)
        })
        .collect();
    let mut error = None;
    document.visit_elements_mut(|element| {
        if error.is_none()
            && let Some(node) = positions.get(&(element.line, element.column))
        {
            error = fill(*node, level, element, 0, locations).err();
        }
    });
    error.map_or(Ok(()), Err)
}

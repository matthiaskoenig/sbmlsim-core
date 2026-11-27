//! Namespace-independent access to preserved SBML package objects.
use crate::{Element, Extension};
pub fn attr<'a>(object: &'a Extension, key: &str) -> Option<&'a str> {
    object
        .attributes
        .iter()
        .find(|a| a.name == key)
        .map(|a| a.value.as_str())
}
pub fn children<'a>(element: &'a Element, package: &str, list: &str) -> Vec<&'a Extension> {
    element
        .extensions
        .iter()
        .filter(|e| e.package == package && e.name == list)
        .flat_map(|e| &e.children)
        .collect()
}

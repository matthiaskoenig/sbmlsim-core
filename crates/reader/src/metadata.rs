//! Common SBase fields and controlled RDF annotations.

use crate::{elements, text, xml};
use roxmltree::Node;
use sbmlsim_document::{Creator, CvTerm, Element, History};

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    elements(node).find(|n| n.tag_name().name() == name)
}
fn descendant<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn value(node: Node, name: &str) -> Option<String> {
    descendant(node, name).map(text).filter(|v| !v.is_empty())
}
fn qualifier(node: Node) -> Option<String> {
    let prefix = xml::qname(node);
    let (prefix, name) = prefix.split_once(':')?;
    let suffix = match name {
        "is" => "IS",
        "hasPart" => "HAS_PART",
        "isPartOf" => "IS_PART_OF",
        "isVersionOf" => "IS_VERSION_OF",
        "hasVersion" => "HAS_VERSION",
        "isHomologTo" => "IS_HOMOLOG_TO",
        "isDescribedBy" => "IS_DESCRIBED_BY",
        "isEncodedBy" => "IS_ENCODED_BY",
        "encodes" => "ENCODES",
        "occursIn" => "OCCURS_IN",
        "hasProperty" => "HAS_PROPERTY",
        "isPropertyOf" => "IS_PROPERTY_OF",
        "hasTaxon" => "HAS_TAXON",
        "isDerivedFrom" => "IS_DERIVED_FROM",
        "isInstanceOf" => "IS_INSTANCE_OF",
        "hasInstance" => "HAS_INSTANCE",
        _ => return None,
    };
    let group = match prefix {
        "bqbiol" => "BQB",
        "bqmodel" => "BQM",
        _ => return None,
    };
    Some(format!("{group}_{suffix}"))
}
fn term(node: Node) -> Option<CvTerm> {
    let qualifier = qualifier(node)?;
    let bag = child(node, "Bag")?;
    let resources: Vec<String> = elements(bag)
        .filter(|n| n.tag_name().namespace() == Some(RDF) && n.tag_name().name() == "li")
        .filter_map(|n| n.attribute((RDF, "resource")).map(str::to_owned))
        .collect();
    let nested: Vec<CvTerm> = elements(bag).filter_map(term).collect();
    if resources.is_empty() && nested.is_empty() {
        return None;
    }
    Some(CvTerm {
        qualifier,
        resources,
        nested,
    })
}

pub(crate) struct Positions<'a> {
    source: &'a str,
    starts: Vec<usize>,
}
impl<'a> Positions<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        );
        Self { source, starts }
    }
    pub(crate) fn at(&self, offset: usize) -> (u32, u32) {
        let line = self
            .starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        (
            (line + 1) as u32,
            (self.source[self.starts[line]..offset].chars().count() + 1) as u32,
        )
    }
}

pub(crate) fn element(node: Node, level: u32, with_xml: bool, positions: &Positions) -> Element {
    let position = positions.at(node.range().start);
    let id = node
        .attribute("id")
        .or_else(|| (level == 1).then(|| node.attribute("name")).flatten())
        .map(str::to_owned);
    let name = node
        .attribute("name")
        .or_else(|| {
            node.tag_name()
                .namespace()
                .filter(|ns| {
                    crate::packages::name(Some(ns)).is_some()
                        && node.tag_name().name() != "modelDefinition"
                        && !node.tag_name().name().starts_with("listOf")
                })
                .and_then(|ns| node.attribute((ns, "name")))
        })
        .map(str::to_owned);
    let meta_id = node.attribute("metaid").map(str::to_owned);
    let annotation = child(node, "annotation");
    let description = annotation
        .and_then(|a| {
            elements(a)
                .find(|n| n.tag_name().namespace() == Some(RDF) && n.tag_name().name() == "RDF")
        })
        .and_then(|rdf| child(rdf, "Description"));
    let description = description.filter(|d| {
        meta_id.as_deref().is_some_and(|id| {
            d.attribute((RDF, "about"))
                .is_some_and(|about| !about.is_empty() && about.contains(id))
        })
    });
    let cvterms: Vec<CvTerm> = description
        .map(|d| elements(d).filter_map(term).collect())
        .unwrap_or_default();
    let history = description.and_then(|d| {
        let creators: Vec<_> = elements(d)
            .filter(|n| n.tag_name().name() == "creator")
            .flat_map(|n| elements(n).flat_map(elements))
            .map(|c| Creator {
                given_name: value(c, "Given").or_else(|| value(c, "given-name")),
                family_name: value(c, "Family").or_else(|| value(c, "family-name")),
                organization: value(c, "Orgname").or_else(|| value(c, "organization-name")),
                email: value(c, "EMAIL").or_else(|| value(c, "hasEmail")),
            })
            .collect();
        let created_date = child(d, "created").and_then(|n| value(n, "W3CDTF"));
        let modified_dates: Vec<_> = elements(d)
            .filter(|n| n.tag_name().name() == "modified")
            .filter_map(|n| value(n, "W3CDTF"))
            .collect();
        if creators.is_empty() || created_date.is_none() || modified_dates.is_empty() {
            None
        } else {
            Some(History {
                creators,
                created_date,
                modified_dates,
            })
        }
    });
    let xml = with_xml
        .then(|| xml::fragment_with_rdf(node, level, cvterms.iter().any(|t| !t.nested.is_empty())));
    Element {
        id,
        meta_id,
        name,
        sbo: node.attribute("sboTerm").map(str::to_owned),
        notes: child(node, "notes").map(|n| xml::fragment(n, level)),
        annotation: annotation
            .filter(|n| {
                elements(*n)
                    .any(|c| c.tag_name().namespace() != Some("http://sbml.org/fbc/keyvaluepair"))
                    || !node.document().root_element().namespaces().any(|ns| {
                        crate::packages::name(Some(ns.uri())) == Some("fbc")
                            && ns.uri() != "http://sbml.org/fbc/keyvaluepair"
                    })
            })
            .map(|n| xml::annotation_fragment(n, level)),
        cvterms,
        history,
        xml,
        line: position.0,
        column: position.1,
        ..Element::default()
    }
}

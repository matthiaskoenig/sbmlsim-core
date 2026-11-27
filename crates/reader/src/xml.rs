//! Canonical XML fragments for the report, without recursive serialization.

use crate::{MATHML, elements, text};
use roxmltree::{Node, NodeId};
use std::collections::HashSet;

pub(crate) fn qname(node: Node) -> String {
    let input = node.document().input_text();
    let raw = &input[node.range()];
    raw.strip_prefix('<')
        .unwrap_or(raw)
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_owned()
}
fn escape(value: &str, attribute: bool) -> String {
    let value = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('\"', "&quot;");
    if attribute {
        value.replace('"', "&quot;")
    } else {
        value
    }
}

fn package_order(name: &str) -> &'static [&'static str] {
    match name {
        "port" => &["portRef", "idRef", "unitRef", "metaIdRef", "id", "name"],
        "deletion" => &["portRef", "idRef", "unitRef", "metaIdRef", "id", "name"],
        "sBaseRef" => &["portRef", "idRef", "unitRef", "metaIdRef"],
        "replacedBy" => &["portRef", "idRef", "unitRef", "metaIdRef", "submodelRef"],
        "replacedElement" => &[
            "portRef",
            "idRef",
            "unitRef",
            "metaIdRef",
            "submodelRef",
            "deletion",
            "conversionFactor",
        ],
        "submodel" => &[
            "id",
            "name",
            "modelRef",
            "timeConversionFactor",
            "extentConversionFactor",
        ],
        "externalModelDefinition" => &["id", "name", "source", "modelRef", "md5"],
        "qualitativeSpecies" => &[
            "id",
            "compartment",
            "constant",
            "name",
            "initialLevel",
            "maxLevel",
        ],
        "input" => &[
            "id",
            "qualitativeSpecies",
            "transitionEffect",
            "name",
            "sign",
            "thresholdLevel",
        ],
        "output" => &[
            "id",
            "qualitativeSpecies",
            "transitionEffect",
            "name",
            "outputLevel",
        ],
        "fluxObjective" => &[
            "id",
            "name",
            "reaction",
            "coefficient",
            "variableType",
            "reaction2",
        ],
        "userDefinedConstraintComponent" => &[
            "id",
            "name",
            "coefficient",
            "variable",
            "variable2",
            "variableType",
        ],
        "uncertParameter" | "uncertSpan" => &[
            "id",
            "name",
            "value",
            "var",
            "units",
            "type",
            "definitionURL",
            "valueLower",
            "valueUpper",
            "varLower",
            "varUpper",
        ],
        _ => &[],
    }
}

fn native_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == f64::INFINITY {
        return "INF".into();
    }
    if value == f64::NEG_INFINITY {
        return "-INF".into();
    }
    let scientific = format!("{value:.14e}");
    let (mantissa, exp) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent = exp.parse::<i32>().unwrap_or(0);
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    if !(-4..15).contains(&exponent) {
        return format!("{mantissa}e{exponent:+03}");
    }
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    let point = exponent + 1;
    if point <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!(
            "{sign}{digits}{}",
            "0".repeat(point as usize - digits.len())
        )
    } else {
        let (a, b) = digits.split_at(point as usize);
        format!("{sign}{a}.{b}")
    }
}

fn order(name: &str) -> &'static [&'static str] {
    match name {
        "unit" => &["kind", "exponent", "scale", "multiplier", "offset"],
        "compartment" => &[
            "spatialDimensions",
            "size",
            "volume",
            "units",
            "outside",
            "constant",
        ],
        "species" => &[
            "compartment",
            "initialAmount",
            "initialConcentration",
            "substanceUnits",
            "spatialSizeUnits",
            "units",
            "hasOnlySubstanceUnits",
            "boundaryCondition",
            "charge",
            "constant",
            "conversionFactor",
        ],
        "parameter" | "localParameter" => &["value", "units", "constant"],
        "reaction" => &["reversible", "fast", "compartment"],
        "speciesReference" => &["species", "stoichiometry", "denominator", "constant"],
        "modifierSpeciesReference" => &["species"],
        "initialAssignment" => &["symbol"],
        "assignmentRule" | "rateRule" => &["variable", "formula"],
        "kineticLaw" => &["formula", "timeUnits", "substanceUnits"],
        "event" => &["useValuesFromTriggerTime", "timeUnits"],
        "trigger" => &["initialValue", "persistent"],
        "cn" => &["units", "type"],
        "csymbol" => &["encoding", "definitionURL"],
        _ => &[],
    }
}

fn attributes(node: Node, level: u32) -> Vec<(String, String)> {
    let name = node.tag_name().name();
    let mut attributes: Vec<_> = node
        .attributes()
        .map(|a| {
            let key = node.document().input_text()[a.range_qname()].to_owned();
            (key, a.name().to_owned(), a.value().to_owned())
        })
        .collect();
    if node.tag_name().namespace() == Some(MATHML) {
        attributes.retain(|(_, name, value)| {
            !(node.tag_name().name() == "cn" && name == "type" && value == "real")
        });
    }
    let sbml_object = node.tag_name().namespace()
        == node.document().root_element().tag_name().namespace()
        || crate::packages::name(node.tag_name().namespace()).is_some();
    if !sbml_object && node.tag_name().namespace() != Some(MATHML) {
        return attributes
            .into_iter()
            .map(|(key, _, value)| (key, value))
            .collect();
    }
    for (key, local, value) in &mut attributes {
        let numeric = match name {
            "unit" => matches!(
                local.as_str(),
                "exponent" | "scale" | "multiplier" | "offset"
            ),
            "compartment" => matches!(local.as_str(), "spatialDimensions" | "size" | "volume"),
            "species" => matches!(
                local.as_str(),
                "initialAmount" | "initialConcentration" | "charge"
            ),
            "parameter" | "localParameter" | "fluxBound" => local == "value",
            "speciesReference" | "specieReference" => {
                matches!(local.as_str(), "stoichiometry" | "denominator")
            }
            "fluxObjective" => local == "coefficient",
            "uncertParameter" | "uncertSpan" => {
                matches!(local.as_str(), "value" | "valueLower" | "valueUpper")
            }
            _ => false,
        };
        if numeric && let Ok(number) = value.parse::<f64>() {
            *value = native_number(number);
        }
        if matches!(
            local.as_str(),
            "constant"
                | "reversible"
                | "fast"
                | "boundaryCondition"
                | "hasOnlySubstanceUnits"
                | "initialValue"
                | "persistent"
                | "useValuesFromTriggerTime"
        ) {
            if value == "0" {
                *value = "false".into();
            } else if value == "1" {
                *value = "true".into();
            }
        }
        if name == "cn" && local == "units" {
            *key = "sbml:units".into();
        }
    }
    if level < 3 && node.tag_name().namespace() != Some(MATHML) {
        attributes.retain(|(_, attr, value)| {
            !matches!(
                (name, attr.as_str(), value.as_str()),
                ("compartment", "spatialDimensions", "3")
                    | ("compartment", "constant", "true")
                    | (
                        "species",
                        "hasOnlySubstanceUnits" | "boundaryCondition" | "constant",
                        "false"
                    )
                    | ("parameter", "constant", "true")
                    | ("reaction", "reversible", "true")
                    | ("event", "useValuesFromTriggerTime", "true")
            )
        });
    }
    attributes.sort_by_key(|(key, name, _)| {
        let package_attribute = key.contains(':')
            && node
                .attributes()
                .any(|a| a.name() == name && crate::packages::name(a.namespace()).is_some());
        let package_object = crate::packages::name(node.tag_name().namespace()).is_some();
        if name == "metaid" {
            return 0;
        }
        if name == "sboTerm" {
            return 1;
        }
        if package_object && !package_order(node.tag_name().name()).is_empty() {
            return package_order(node.tag_name().name())
                .iter()
                .position(|n| *n == name)
                .map_or(1000, |i| i + 2);
        }
        if package_attribute && !package_object {
            return 100
                + [
                    "charge",
                    "chemicalFormula",
                    "lowerFluxBound",
                    "upperFluxBound",
                ]
                .iter()
                .position(|n| *n == name)
                .unwrap_or(20);
        }
        match name.as_str() {
            "id" => 2,
            "name" => 3,
            name => order(node.tag_name().name())
                .iter()
                .position(|key| *key == name)
                .map_or(1000, |i| i + 4),
        }
    });
    attributes
        .into_iter()
        .map(|(key, _, value)| (key, value))
        .collect()
}

/// Serialize one fragment in the same indentation and text style as native SBML.
pub(crate) fn fragment(root: Node, level: u32) -> String {
    if matches!(root.tag_name().name(), "notes" | "message") {
        return node_fragment(root);
    }
    fragment_with_rdf(root, level, false)
}

pub(crate) fn fragment_with_rdf(root: Node, level: u32, normalize_rdf: bool) -> String {
    render_fragment(root, level, normalize_rdf, false, false)
}

pub(crate) fn list_fragment(root: Node, level: u32) -> String {
    render_fragment(root, level, false, true, false)
}

pub(crate) fn annotation_fragment(root: Node, level: u32) -> String {
    let fbc = root
        .document()
        .root_element()
        .namespaces()
        .any(|ns| crate::packages::name(Some(ns.uri())) == Some("fbc"));
    render_fragment(root, level, false, false, fbc)
}

fn render_fragment(
    root: Node,
    level: u32,
    normalize_rdf: bool,
    list_only: bool,
    annotation_only: bool,
) -> String {
    let preserve_whitespace = false;
    enum Work<'a, 'input> {
        Open(Node<'a, 'input>, usize, Option<NodeId>),
        Close(String, usize),
        Raw(String),
        TrimNewline,
    }
    let mut unit_maths = HashSet::new();
    let mut scan = vec![(root, None)];
    while let Some((node, current)) = scan.pop() {
        let current =
            if node.tag_name().namespace() == Some(MATHML) && node.tag_name().name() == "math" {
                Some(node.id())
            } else {
                current
            };
        if node.tag_name().name() == "cn"
            && node.attributes().any(|a| a.name() == "units")
            && let Some(current) = current
        {
            unit_maths.insert(current);
        }
        scan.extend(elements(node).map(|n| (n, current)));
    }
    let mut output = String::new();
    let mut stack = vec![Work::Open(root, 0, None)];
    while let Some(work) = stack.pop() {
        match work {
            Work::Raw(text) => output.push_str(&text),
            Work::TrimNewline => {
                if output.ends_with('\n') {
                    output.pop();
                }
            }
            Work::Close(name, depth) => {
                output.push_str(&"  ".repeat(depth));
                output.push_str(&format!("</{name}>\n"));
            }
            Work::Open(node, depth, current_math) => {
                let current_math = if node.tag_name().namespace() == Some(MATHML)
                    && node.tag_name().name() == "math"
                {
                    Some(node.id())
                } else {
                    current_math
                };
                if depth > 0
                    && matches!(node.tag_name().name(), "notes" | "message")
                    && node.tag_name().namespace()
                        == node.document().root_element().tag_name().namespace()
                {
                    let fragment = node_fragment_at(node, depth);
                    output.push_str(fragment.strip_prefix('\n').unwrap_or(&fragment));
                    output.push('\n');
                    continue;
                }
                let name = qname(node);
                let indent = "  ".repeat(depth);
                output.push_str(&indent);
                output.push('<');
                output.push_str(&name);
                let rewrite_rdf = normalize_rdf
                    && node.tag_name().name() == "RDF"
                    && node.parent_element().and_then(|p| p.parent_element()) == Some(root);
                if rewrite_rdf {
                    for (prefix, uri) in [
                        ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
                        ("dcterms", "http://purl.org/dc/terms/"),
                        ("vCard", "http://www.w3.org/2001/vcard-rdf/3.0#"),
                        ("vCard4", "http://www.w3.org/2006/vcard/ns#"),
                        ("bqbiol", "http://biomodels.net/biology-qualifiers/"),
                        ("bqmodel", "http://biomodels.net/model-qualifiers/"),
                    ] {
                        output.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
                    }
                }
                let unit_namespace = current_math.filter(|id| unit_maths.contains(id)).map(|_| {
                    node.document()
                        .root_element()
                        .tag_name()
                        .namespace()
                        .unwrap_or("http://www.sbml.org/sbml/level3/version2/core")
                });
                if node.tag_name().name() == "math"
                    && let Some(uri) = unit_namespace
                {
                    output.push_str(&format!(" xmlns=\"{MATHML}\" xmlns:sbml=\"{uri}\""));
                }
                for namespace in node.namespaces().filter(|_| !rewrite_rdf) {
                    if unit_namespace
                        .is_some_and(|uri| namespace.uri() == uri || namespace.uri() == MATHML)
                    {
                        continue;
                    }
                    let parent = node.parent_element();
                    let inherited = parent.and_then(|p| p.lookup_namespace_uri(namespace.name()));
                    let explicit = (node.tag_name().name() == "math" && namespace.name().is_none())
                        || (node.tag_name().name() == "RDF"
                            && node.tag_name().namespace()
                                == Some("http://www.w3.org/1999/02/22-rdf-syntax-ns#")
                            && explicit_namespace(node, namespace.name()));
                    if inherited != Some(namespace.uri()) || explicit {
                        // A detached SBML object keeps the inherited core namespace implicit.
                        if depth == 0
                            && node.tag_name().namespace() != Some(MATHML)
                            && namespace.uri().contains("sbml.org/sbml/")
                            && namespace.name().is_none()
                        {
                            continue;
                        }
                        let prefix = namespace
                            .name()
                            .map(|p| format!(":{p}"))
                            .unwrap_or_default();
                        output.push_str(&format!(
                            " xmlns{prefix}=\"{}\"",
                            escape(namespace.uri(), true)
                        ));
                    }
                }
                for (name, value) in attributes(node, level) {
                    output.push_str(&format!(" {name}=\"{}\"", escape(&value, true)));
                }
                let mut children: Vec<_> = elements(node)
                    .filter(|c| {
                        !list_only
                            || node != root
                            || matches!(c.tag_name().name(), "notes" | "annotation")
                            || c.tag_name().namespace() != root.tag_name().namespace()
                    })
                    .filter(|child| {
                        !annotation_only
                            || node != root
                            || child.tag_name().namespace()
                                != Some("http://sbml.org/fbc/keyvaluepair")
                    })
                    .collect();
                if !matches!(node.tag_name().name(), "annotation" | "notes")
                    && (node.tag_name().namespace()
                        == node.document().root_element().tag_name().namespace()
                        || crate::packages::name(node.tag_name().namespace()).is_some())
                {
                    children.sort_by_key(|c| match c.tag_name().name() {
                        "notes" => 0,
                        "annotation" => 1,
                        "defaultTerm" => 2,
                        _ if crate::packages::name(c.tag_name().namespace()).is_some()
                            && c.tag_name().namespace() != node.tag_name().namespace() =>
                        {
                            4
                        }
                        _ => 3,
                    });
                }
                if node.tag_name().name() == "event" {
                    children.sort_by_key(|c| match c.tag_name().name() {
                        "notes" => 0,
                        "annotation" => 1,
                        "trigger" => 2,
                        "delay" => 3,
                        "priority" => 4,
                        "listOfEventAssignments" => 5,
                        _ => 6,
                    });
                }
                if matches!(node.tag_name().name(), "ci" | "csymbol")
                    && node.tag_name().namespace() == Some(MATHML)
                {
                    output.push_str(&format!("> {} </{name}>\n", escape(&text(node), false)));
                    continue;
                }
                if node.tag_name().name() == "cn" && node.tag_name().namespace() == Some(MATHML) {
                    output.push_str("> ");
                    let mut content = String::new();
                    for child in node.children() {
                        if child.is_element() && child.tag_name().name() == "sep" {
                            content.push_str(" <sep/> ");
                        } else if child.is_text() {
                            content.push_str(child.text().unwrap_or("").trim());
                        }
                    }
                    output.push_str(&escape(&content, false).replace("&lt;sep/&gt;", "<sep/>"));
                    output.push_str(&format!(" </{name}>\n"));
                    continue;
                }
                if children.is_empty() {
                    let text = node
                        .children()
                        .filter(Node::is_text)
                        .filter_map(|n| n.text())
                        .collect::<String>();
                    let text = if text.trim().is_empty() {
                        String::new()
                    } else {
                        text
                    };
                    if text.is_empty() {
                        output.push_str("/>\n");
                    } else {
                        output.push_str(&format!(">{}</{name}>\n", escape(&text, false)));
                    }
                } else {
                    let mixed = node.children().any(|n| {
                        n.is_text()
                            && n.text()
                                .is_some_and(|t| preserve_whitespace || !t.trim().is_empty())
                    });
                    if mixed {
                        output.push('>');
                        stack.push(Work::Raw(format!("</{name}>\n")));
                        for child in node.children().collect::<Vec<_>>().into_iter().rev() {
                            if child.is_text() {
                                stack.push(Work::Raw(escape(child.text().unwrap_or(""), false)));
                            } else if child.is_element() {
                                stack.push(Work::TrimNewline);
                                stack.push(Work::Open(child, 0, current_math));
                            }
                        }
                    } else {
                        output.push_str(">\n");
                        stack.push(Work::Close(name, depth));
                        stack.extend(
                            children
                                .into_iter()
                                .rev()
                                .map(|child| Work::Open(child, depth + 1, current_math)),
                        );
                    }
                }
            }
        }
    }
    output.trim_end_matches('\n').to_owned()
}

/// XMLNode-style mixed content uses an output state rather than structural depth.
/// Text does not increase indentation, and closing a text-bearing element does
/// not decrease it. This preserves the native report's observable XHTML layout.
fn explicit_namespace(node: Node, prefix: Option<&str>) -> bool {
    let source = &node.document().input_text()[node.range()];
    let bytes = source.as_bytes();
    let mut i = 1;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'/' | b'>') {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || matches!(bytes[i], b'/' | b'>') {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = &source[start..i];
        if prefix.map_or(name == "xmlns", |p| name.strip_prefix("xmlns:") == Some(p)) {
            return true;
        }
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        i += 1;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        i += 1;
    }
    false
}
fn node_fragment(root: Node) -> String {
    node_fragment_at(root, 0)
}
fn node_fragment_at(root: Node, initial_indent: usize) -> String {
    enum Event<'a, 'input> {
        Start(Node<'a, 'input>),
        End(Node<'a, 'input>, bool),
        Text(&'input str),
    }
    let mut work = vec![Event::Start(root)];
    let mut out = String::new();
    let mut indent = initial_indent;
    let mut pending = false;
    let mut in_text = false;
    let mut skip = false;
    while let Some(event) = work.pop() {
        match event {
            Event::Text(text) => {
                if pending {
                    out.push('>');
                    pending = false;
                }
                out.push_str(&escape(text, false));
                in_text = true;
                skip = true;
            }
            Event::Start(node) => {
                if pending {
                    out.push('>');
                    indent += 1;
                }
                pending = true;
                if in_text && skip {
                    skip = false;
                } else if indent > 0 {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                }
                out.push('<');
                out.push_str(&qname(node));
                for ns in node.namespaces() {
                    if node
                        .parent_element()
                        .and_then(|p| p.lookup_namespace_uri(ns.name()))
                        != Some(ns.uri())
                        || explicit_namespace(node, ns.name())
                    {
                        let prefix = ns.name().map(|n| format!(":{n}")).unwrap_or_default();
                        out.push_str(&format!(" xmlns{prefix}=\"{}\"", escape(ns.uri(), true)));
                    }
                }
                for attr in node.attributes() {
                    let name = &node.document().input_text()[attr.range_qname()];
                    out.push_str(&format!(" {name}=\"{}\"", escape(attr.value(), true)));
                }
                let children: Vec<_> = node
                    .children()
                    .filter(|n| {
                        n.is_element()
                            || (n.is_text() && n.text().is_some_and(|s| !s.trim().is_empty()))
                    })
                    .collect();
                let has_text = children.iter().any(|n| n.is_text());
                work.push(Event::End(node, has_text));
                for child in children.into_iter().rev() {
                    if child.is_text() {
                        work.push(Event::Text(child.text().unwrap_or("")));
                    } else {
                        work.push(Event::Start(child));
                    }
                }
            }
            Event::End(node, has_text) => {
                if pending {
                    out.push_str("/>");
                    pending = false;
                } else {
                    if in_text || has_text {
                        in_text = false;
                        skip = false;
                    } else {
                        indent = indent.saturating_sub(1);
                        out.push('\n');
                        out.push_str(&"  ".repeat(indent));
                    }
                    out.push_str(&format!("</{}>", qname(node)));
                }
            }
        }
    }
    out
}

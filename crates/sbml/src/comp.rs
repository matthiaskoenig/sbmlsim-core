//! The flattening of the package of hierarchical model composition, `comp`.
//!
//! A document of `comp` composes its model of submodels, each an instance of a model definition
//! of the document (`<comp:modelDefinition>`) or of a model of another document
//! (`<comp:externalModelDefinition>`), whose elements are deleted (`<comp:deletion>`), replaced
//! by an element of the model which contains them (`<comp:replacedElement>`) or replace one of
//! it (`<comp:replacedBy>`), and whose time and extent are converted
//! (`comp:timeConversionFactor`, `comp:extentConversionFactor`). The flattened model is the one
//! model of SBML core which means the same, the one the specification of the package describes
//! and libSBML's `CompFlatteningConverter` writes:
//!
//! - Every submodel is flattened first, then the ids of its elements (the ids of the elements of
//!   SBML, the ids of the unit definitions and the meta ids) get the id of the submodel and two
//!   underscores as prefix (`sub__x`, `outer__inner__x` for a submodel of a submodel), the ones
//!   libSBML gives them. A local parameter keeps its id, which is local to its kinetic law.
//! - An element a deletion names is removed with everything in it.
//! - An element a `<comp:replacedElement>` of an element of the containing model names is
//!   removed, and every reference to it names the replacing element instead. With a
//!   `comp:conversionFactor` the replaced element is the replacing one divided by the factor:
//!   a math which reads it reads `replacing / factor`, and an assignment to it (a rule, an
//!   initial assignment, an event assignment) assigns its math times the factor.
//! - An element of the containing model with a `<comp:replacedBy>` is removed, and the element of
//!   the submodel which replaces it takes its id and meta id.
//! - With a `comp:timeConversionFactor` the time of the submodel is the one of the containing
//!   model divided by the factor: the symbol `time` reads `time / factor`, a rate rule and the
//!   rate of a reaction are divided by it, the delay of an event and of the function `delay` are
//!   multiplied by it and `rateOf` is multiplied by it. With a `comp:extentConversionFactor` the
//!   rate of every reaction of the submodel is multiplied by the factor. A math of the submodel
//!   which reads the rate of a reaction of the submodel reads it in the units of the submodel,
//!   converted back. The conversions follow the replacements: an element of the containing
//!   model which replaces one of the submodel is in the units of the containing model, so a
//!   reaction of it which replaces one of the submodel is read as it is, or converted by the
//!   `comp:conversionFactor` of its replacement (the cases 01178 to 01183 of the SBML Test
//!   Suite).
//! - The model conversion factor of a submodel becomes the conversion factor of its species
//!   which have none of their own.
//!
//! A reference names an element of a submodel by its id, its meta id, the id of a unit
//! definition or a port of the submodel, and an element of a submodel of the submodel by a
//! `<comp:sBaseRef>` in it; a reference to an element which was replaced is a reference to its
//! replacement. A document without the package is never touched: [`uses_comp`] is checked first.
//!
//! The flattening works on the XML: the document is read into a tree of elements, flattened and
//! written again as SBML core, which the reader then reads as any document. An external model
//! definition is the document its `comp:source` names, which [`crate::read_with`] reads with a
//! function of the caller; [`crate::read`] has none and refuses it.

use std::collections::{HashMap, HashSet};

use crate::ReadError;
use crate::math::{DELAY_URL, MATHML_NS, RATEOF_URL, TIME_URL};
use crate::model::is_sbml_ns;

/// The beginning of the namespace URI of every version of the package.
pub(crate) const COMP_NS: &str = "http://www.sbml.org/sbml/level3/version1/comp/";

/// The namespace of the attributes `xml:*`, which is declared by XML itself.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The deepest nesting of submodels, which also stops a model which contains itself.
const MAX_DEPTH: usize = 64;

/// The lists of a model of SBML core, in the order of the specification.
const LISTS: [&str; 10] = [
    "listOfFunctionDefinitions",
    "listOfUnitDefinitions",
    "listOfCompartments",
    "listOfSpecies",
    "listOfParameters",
    "listOfInitialAssignments",
    "listOfRules",
    "listOfConstraints",
    "listOfReactions",
    "listOfEvents",
];

/// The function which gives the text of the document of an external model definition, by its
/// source relative to the document which is read.
pub(crate) type Sources<'a> = &'a dyn Fn(&str) -> Result<String, String>;

fn err(message: impl Into<String>) -> ReadError {
    ReadError::Xml(format!("comp: {}", message.into()))
}

fn is_comp(ns: Option<&str>) -> bool {
    ns.is_some_and(|ns| ns.starts_with(COMP_NS))
}

/// Whether a document uses the package: it requires it or has an element or an attribute of its
/// namespace.
pub(crate) fn uses_comp(doc: &roxmltree::Document) -> bool {
    doc.root_element().descendants().any(|n| {
        n.is_element()
            && (is_comp(n.tag_name().namespace()) || n.attributes().any(|a| is_comp(a.namespace())))
    })
}

#[derive(Debug, Clone)]
struct Attr {
    ns: Option<String>,
    name: String,
    value: String,
}

/// An element of the tree, with the handle which names it while the model is flattened.
#[derive(Debug, Clone)]
struct El {
    ns: Option<String>,
    name: String,
    attrs: Vec<Attr>,
    children: Vec<Item>,
    handle: usize,
}

#[derive(Debug, Clone)]
enum Item {
    El(El),
    Text(String),
}

impl El {
    fn from_xml(n: roxmltree::Node) -> El {
        El {
            ns: n.tag_name().namespace().map(str::to_string),
            name: n.tag_name().name().to_string(),
            attrs: n
                .attributes()
                .map(|a| Attr {
                    ns: a.namespace().map(str::to_string),
                    name: a.name().to_string(),
                    value: a.value().to_string(),
                })
                .collect(),
            children: n
                .children()
                .filter_map(|c| {
                    if c.is_element() {
                        Some(Item::El(El::from_xml(c)))
                    } else if c.is_text() {
                        c.text().map(|t| Item::Text(t.to_string()))
                    } else {
                        None
                    }
                })
                .collect(),
            handle: 0,
        }
    }

    fn new(ns: &str, name: &str) -> El {
        El {
            ns: Some(ns.to_string()),
            name: name.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
            handle: 0,
        }
    }

    fn is_core(&self, name: &str) -> bool {
        self.name == name && self.ns.as_deref().is_some_and(is_sbml_ns)
    }

    fn is_comp(&self, name: &str) -> bool {
        self.name == name && is_comp(self.ns.as_deref())
    }

    fn is_math(&self) -> bool {
        self.name == "math" && self.ns.as_deref() == Some(MATHML_NS)
    }

    /// An attribute of no namespace, the attributes of SBML core.
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    fn set_attr(&mut self, name: &str, value: &str) {
        match self
            .attrs
            .iter_mut()
            .find(|a| a.ns.is_none() && a.name == name)
        {
            Some(a) => a.value = value.to_string(),
            None => self.attrs.push(Attr {
                ns: None,
                name: name.to_string(),
                value: value.to_string(),
            }),
        }
    }

    fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|a| a.ns.is_some() || a.name != name);
    }

    /// An attribute of the package, whose namespace is the one of `comp`.
    fn comp_attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| is_comp(a.ns.as_deref()) && a.name == name)
            .map(|a| a.value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &El> {
        self.children.iter().filter_map(|c| match c {
            Item::El(e) => Some(e),
            Item::Text(_) => None,
        })
    }

    fn elements_mut(&mut self) -> impl Iterator<Item = &mut El> {
        self.children.iter_mut().filter_map(|c| match c {
            Item::El(e) => Some(e),
            Item::Text(_) => None,
        })
    }

    fn child_comp(&self, name: &str) -> Option<&El> {
        self.elements().find(|e| e.is_comp(name))
    }

    /// The text of the element, trimmed, the name of a `<ci>`.
    fn text(&self) -> String {
        self.children
            .iter()
            .filter_map(|c| match c {
                Item::Text(t) => Some(t.as_str()),
                Item::El(_) => None,
            })
            .collect::<String>()
            .trim()
            .to_string()
    }

    fn set_text(&mut self, text: &str) {
        self.children = vec![Item::Text(text.to_string())];
    }

    /// Gives every element a handle of its own.
    fn number(&mut self, next: &mut usize) {
        *next += 1;
        self.handle = *next;
        for e in self.elements_mut() {
            e.number(next);
        }
    }

    fn find(&self, handle: usize) -> Option<&El> {
        if self.handle == handle {
            return Some(self);
        }
        self.elements().find_map(|e| e.find(handle))
    }

    fn find_mut(&mut self, handle: usize) -> Option<&mut El> {
        if self.handle == handle {
            return Some(self);
        }
        self.elements_mut().find_map(|e| e.find_mut(handle))
    }

    /// Removes the element of a handle below this one, `false` when there is none.
    fn remove(&mut self, handle: usize) -> bool {
        let before = self.children.len();
        self.children
            .retain(|c| !matches!(c, Item::El(e) if e.handle == handle));
        before != self.children.len() || self.elements_mut().any(|e| e.remove(handle))
    }

    /// Removes the elements and attributes of the package.
    fn strip_comp(&mut self) {
        self.attrs.retain(|a| !is_comp(a.ns.as_deref()));
        self.children
            .retain(|c| !matches!(c, Item::El(e) if is_comp(e.ns.as_deref())));
        for e in self.elements_mut() {
            e.strip_comp();
        }
    }

    /// Moves the elements and attributes of every namespace of SBML core into `core`, the one
    /// of the document which is read, so that the elements of a document of another version
    /// (an external model definition) are read as elements of SBML core.
    fn move_to_core(&mut self, core: &str) {
        if self.ns.as_deref().is_some_and(is_sbml_ns) {
            self.ns = Some(core.to_string());
        }
        for a in &mut self.attrs {
            if a.ns.as_deref().is_some_and(is_sbml_ns) {
                a.ns = Some(core.to_string());
            }
        }
        for e in self.elements_mut() {
            e.move_to_core(core);
        }
    }

    fn write(&self, parent_ns: Option<&str>, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        if self.ns.as_deref() != parent_ns {
            out.push_str(" xmlns=\"");
            escape(self.ns.as_deref().unwrap_or(""), out);
            out.push('"');
        }
        let mut prefixes: Vec<&str> = Vec::new();
        for a in &self.attrs {
            out.push(' ');
            match a.ns.as_deref() {
                None => {}
                Some(XML_NS) => out.push_str("xml:"),
                Some(ns) => {
                    let k = prefixes.iter().position(|p| *p == ns).unwrap_or_else(|| {
                        prefixes.push(ns);
                        prefixes.len() - 1
                    });
                    out.push_str(&format!("a{k}:"));
                }
            }
            out.push_str(&a.name);
            out.push_str("=\"");
            escape(&a.value, out);
            out.push('"');
        }
        for (k, ns) in prefixes.iter().enumerate() {
            out.push_str(&format!(" xmlns:a{k}=\""));
            escape(ns, out);
            out.push('"');
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            match c {
                Item::El(e) => e.write(self.ns.as_deref(), out),
                Item::Text(t) => escape(t, out),
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
}

// the math the flattening writes

fn ci(name: &str) -> El {
    let mut e = El::new(MATHML_NS, "ci");
    e.set_text(name);
    e
}

fn apply(op: &str, args: Vec<El>) -> El {
    let mut e = El::new(MATHML_NS, "apply");
    e.children.push(Item::El(El::new(MATHML_NS, op)));
    e.children.extend(args.into_iter().map(Item::El));
    e
}

fn csymbol_url(e: &El) -> Option<&str> {
    (e.name == "csymbol" && e.ns.as_deref() == Some(MATHML_NS))
        .then(|| e.attr("definitionURL"))
        .flatten()
}

/// Replaces the expression of a `<math>` by a function of it.
fn wrap(math: &mut El, f: impl FnOnce(El) -> El) {
    if let Some(Item::El(expression)) = math.children.iter_mut().find(|c| matches!(c, Item::El(_)))
    {
        let inner = std::mem::replace(expression, El::new(MATHML_NS, "none"));
        *expression = f(inner);
    }
}

/// A reference to an element of a model: an `<comp:sBaseRef>` and the attributes it shares with
/// a port, a deletion, a replaced element and a replacement.
#[derive(Debug, Clone)]
struct Ref {
    kind: RefKind,
    value: String,
    /// The reference into the submodel the reference names.
    child: Option<Box<Ref>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RefKind {
    Id,
    MetaId,
    Unit,
    Port,
}

impl Ref {
    fn of(e: &El) -> Result<Ref, ReadError> {
        let found: Vec<(RefKind, &str)> = [
            (RefKind::Id, "idRef"),
            (RefKind::MetaId, "metaIdRef"),
            (RefKind::Unit, "unitRef"),
            (RefKind::Port, "portRef"),
        ]
        .into_iter()
        .filter_map(|(kind, name)| e.comp_attr(name).map(|value| (kind, value)))
        .collect();
        let [(kind, value)] = found.as_slice() else {
            return Err(err(format!(
                "a <{}> names its element by one of idRef, metaIdRef, unitRef and portRef, it \
                 has {}",
                e.name,
                found.len()
            )));
        };
        let child = match e.child_comp("sBaseRef") {
            Some(child) => Some(Box::new(Ref::of(child)?)),
            None => None,
        };
        Ok(Ref {
            kind: *kind,
            value: (*value).to_string(),
            child,
        })
    }
}

/// What the references of the containing model can name in a flattened model: its elements and
/// the ones of its submodels, by the identifiers of the model definition.
#[derive(Debug, Default)]
struct Scope {
    ids: HashMap<String, usize>,
    metaids: HashMap<String, usize>,
    units: HashMap<String, usize>,
    ports: HashMap<String, Ref>,
    submodels: HashMap<String, Scope>,
    /// The submodels by their meta ids.
    submodel_metaids: HashMap<String, String>,
    /// The ids of the elements the deletions of a submodel removed, by the submodel and the
    /// deletion.
    deleted: HashMap<(String, String), Option<String>>,
}

impl Scope {
    /// The scope of the elements of a model, before any of its submodels is flattened.
    fn of(model: &El) -> Scope {
        let mut scope = Scope::default();
        for e in model.elements() {
            scope.collect(e, false);
        }
        scope
    }

    fn collect(&mut self, e: &El, local: bool) {
        if e.is_math() || matches!(e.name.as_str(), "annotation" | "notes") {
            return;
        }
        if is_comp(e.ns.as_deref()) {
            match e.name.as_str() {
                "submodel" => {
                    if let (Some(id), Some(metaid)) = (e.comp_attr("id"), e.attr("metaid")) {
                        self.submodel_metaids
                            .insert(metaid.to_string(), id.to_string());
                    }
                }
                "port" => {
                    if let (Some(id), Ok(reference)) = (e.comp_attr("id"), Ref::of(e)) {
                        self.ports.insert(id.to_string(), reference);
                    }
                }
                "listOfSubmodels" | "listOfPorts" => {}
                _ => return,
            }
            for c in e.elements() {
                self.collect(c, local);
            }
            return;
        }
        if let Some(metaid) = e.attr("metaid") {
            self.metaids.insert(metaid.to_string(), e.handle);
        }
        if let Some(id) = e.attr("id") {
            if e.is_core("unitDefinition") {
                self.units.insert(id.to_string(), e.handle);
            } else if !local && !e.is_core("localParameter") {
                self.ids.insert(id.to_string(), e.handle);
            }
        }
        let local = local || e.is_core("listOfLocalParameters");
        for c in e.elements() {
            self.collect(c, local);
        }
    }
}

/// The renaming of identifiers: of the elements of SBML (`SId`) and of the unit definitions.
#[derive(Default)]
struct Renaming {
    sids: HashMap<String, String>,
    units: HashMap<String, String>,
}

/// The attributes of SBML core which name an element of the model, by the elements which have
/// them.
fn sid_ref(element: &str, attribute: &str) -> bool {
    matches!(
        (element, attribute),
        ("species" | "reaction", "compartment")
            | ("species" | "model", "conversionFactor")
            | ("speciesReference" | "modifierSpeciesReference", "species")
            | ("initialAssignment", "symbol")
            | (
                "assignmentRule" | "rateRule" | "eventAssignment",
                "variable"
            )
            | ("compartment", "outside")
    )
}

/// The attributes of SBML core which name a unit.
fn unit_ref(element: &str, attribute: &str) -> bool {
    matches!(
        (element, attribute),
        ("compartment" | "parameter" | "localParameter", "units")
            | ("species" | "model", "substanceUnits")
            | (
                "model",
                "timeUnits" | "extentUnits" | "volumeUnits" | "areaUnits" | "lengthUnits"
            )
    )
}

impl Renaming {
    fn sid<'a>(&'a self, id: &'a str) -> &'a str {
        self.sids.get(id).map_or(id, String::as_str)
    }

    fn unit<'a>(&'a self, id: &'a str) -> &'a str {
        self.units.get(id).map_or(id, String::as_str)
    }

    /// Renames the definitions and the references of a tree, the math included.
    fn apply(&self, e: &mut El) {
        if matches!(e.name.as_str(), "annotation" | "notes") {
            return;
        }
        if e.is_math() {
            self.math(e, &HashSet::new());
            return;
        }
        let element = e.name.clone();
        let core = e.ns.as_deref().is_some_and(is_sbml_ns);
        for a in &mut e.attrs {
            if a.ns.is_some() || !core {
                continue;
            }
            let renamed = if a.name == "id" {
                match element.as_str() {
                    "unitDefinition" => self.unit(&a.value),
                    "localParameter" => continue,
                    _ => self.sid(&a.value),
                }
            } else if sid_ref(&element, &a.name) {
                self.sid(&a.value)
            } else if unit_ref(&element, &a.name) {
                self.unit(&a.value)
            } else {
                continue;
            };
            a.value = renamed.to_string();
        }
        if e.is_core("kineticLaw") {
            // the math of a kinetic law reads its local parameters before the model
            let locals: HashSet<String> = e
                .elements()
                .filter(|c| c.is_core("listOfLocalParameters") || c.is_core("listOfParameters"))
                .flat_map(|list| list.elements().filter_map(|p| p.attr("id")))
                .map(str::to_string)
                .collect();
            for c in e.elements_mut() {
                if c.is_math() {
                    self.math(c, &locals);
                } else {
                    self.apply(c);
                }
            }
            return;
        }
        for c in e.elements_mut() {
            self.apply(c);
        }
    }

    fn math(&self, e: &mut El, shadowed: &HashSet<String>) {
        if e.name == "ci" {
            let name = e.text();
            if !shadowed.contains(&name)
                && let Some(renamed) = self.sids.get(&name)
            {
                e.set_text(renamed);
            }
            return;
        }
        if e.name == "cn" {
            for a in &mut e.attrs {
                if a.name == "units" && a.ns.as_deref().is_some_and(is_sbml_ns) {
                    a.value = self.unit(&a.value).to_string();
                }
            }
            return;
        }
        if e.name == "lambda" {
            let mut shadowed = shadowed.clone();
            for bvar in e.elements().filter(|c| c.name == "bvar") {
                shadowed.extend(bvar.elements().filter(|c| c.name == "ci").map(El::text));
            }
            for c in e.elements_mut() {
                self.math(c, &shadowed);
            }
            return;
        }
        for c in e.elements_mut() {
            self.math(c, shadowed);
        }
    }
}

/// Replaces every `<ci>` of a name outside the local scopes (the local parameters of a kinetic
/// law and the arguments of a function) by a math of it.
fn replace_ci(e: &mut El, name: &str, by: &dyn Fn(El) -> El) {
    fn math(e: &mut El, name: &str, by: &dyn Fn(El) -> El) {
        if e.name == "lambda"
            && e.elements()
                .filter(|c| c.name == "bvar")
                .flat_map(El::elements)
                .any(|c| c.text() == name)
        {
            return;
        }
        for c in &mut e.children {
            if let Item::El(child) = c {
                if child.name == "ci" && child.text() == name {
                    let original = std::mem::replace(child, El::new(MATHML_NS, "none"));
                    *child = by(original);
                } else {
                    math(child, name, by);
                }
            }
        }
    }
    if matches!(e.name.as_str(), "annotation" | "notes") {
        return;
    }
    if e.is_math() {
        math(e, name, by);
        return;
    }
    if e.is_core("kineticLaw")
        && e.elements()
            .filter(|c| c.is_core("listOfLocalParameters") || c.is_core("listOfParameters"))
            .flat_map(El::elements)
            .any(|p| p.attr("id") == Some(name))
    {
        return;
    }
    for c in e.elements_mut() {
        replace_ci(c, name, by);
    }
}

/// Multiplies the math of every assignment to `variable` (rules, initial assignments and event
/// assignments) by `factor`.
fn scale_assignments(e: &mut El, variable: &str, factor: &str) {
    if matches!(e.name.as_str(), "annotation" | "notes") || e.is_math() {
        return;
    }
    let target = match e.name.as_str() {
        "assignmentRule" | "rateRule" | "eventAssignment" => e.attr("variable"),
        "initialAssignment" => e.attr("symbol"),
        _ => None,
    };
    if target == Some(variable) {
        for math in e.elements_mut().filter(|c| c.is_math()) {
            wrap(math, |x| apply("times", vec![x, ci(factor)]));
        }
        return;
    }
    for c in e.elements_mut() {
        scale_assignments(c, variable, factor);
    }
}

/// The conversion of the time and the extent of a submodel, see the module.
struct Conversion<'a> {
    time: Option<&'a str>,
    extent: Option<&'a str>,
    /// The ids of the reactions of the submodel, whose rates a math of the submodel reads in
    /// its units.
    reactions: HashSet<String>,
}

impl Conversion<'_> {
    fn apply(&self, e: &mut El) {
        if matches!(e.name.as_str(), "annotation" | "notes") {
            return;
        }
        if e.is_math() {
            for c in e.elements_mut() {
                self.math(c);
            }
            return;
        }
        let name = e.name.clone();
        for c in e.elements_mut() {
            self.apply(c);
        }
        let (time, extent) = (self.time, self.extent);
        for math in e.elements_mut().filter(|c| c.is_math()) {
            match name.as_str() {
                "rateRule" => {
                    if let Some(time) = time {
                        wrap(math, |x| apply("divide", vec![x, ci(time)]));
                    }
                }
                "kineticLaw" => {
                    if let Some(extent) = extent {
                        wrap(math, |x| apply("times", vec![x, ci(extent)]));
                    }
                    if let Some(time) = time {
                        wrap(math, |x| apply("divide", vec![x, ci(time)]));
                    }
                }
                "delay" => {
                    if let Some(time) = time {
                        wrap(math, |x| apply("times", vec![x, ci(time)]));
                    }
                }
                _ => {}
            }
        }
    }

    /// Converts a math, inside out: the time, the delays, `rateOf` and the rates of the
    /// reactions it reads.
    fn math(&self, e: &mut El) {
        for c in e.elements_mut() {
            self.math(c);
        }
        let time = self.time;
        if e.name == "ci" && self.reactions.contains(&e.text()) {
            // the rate of a reaction of the submodel in the units of the submodel
            let mut rate = std::mem::replace(e, El::new(MATHML_NS, "none"));
            if let Some(time) = time {
                rate = apply("times", vec![rate, ci(time)]);
            }
            if let Some(extent) = self.extent {
                rate = apply("divide", vec![rate, ci(extent)]);
            }
            *e = rate;
            return;
        }
        if csymbol_url(e) == Some(TIME_URL) {
            if let Some(time) = time {
                let symbol = std::mem::replace(e, El::new(MATHML_NS, "none"));
                *e = apply("divide", vec![symbol, ci(time)]);
            }
            return;
        }
        if e.name != "apply" {
            return;
        }
        let Some(time) = time else { return };
        let head = e.elements().next().and_then(csymbol_url);
        if head == Some(DELAY_URL) {
            // the delay, the second argument
            if let Some(Item::El(delay)) = e
                .children
                .iter_mut()
                .filter(|c| matches!(c, Item::El(_)))
                .nth(2)
            {
                let inner = std::mem::replace(delay, El::new(MATHML_NS, "none"));
                *delay = apply("times", vec![inner, ci(time)]);
            }
        } else if head == Some(RATEOF_URL) {
            let rate = std::mem::replace(e, El::new(MATHML_NS, "none"));
            *e = apply("times", vec![rate, ci(time)]);
        }
    }
}

/// A document of the composition: the main one or the one of an external model definition.
struct Doc {
    root: El,
    /// The directory of the document relative to the main one, empty or ending with `/`.
    base: String,
}

impl Doc {
    fn definitions(&self, list: &str) -> impl Iterator<Item = &El> {
        self.root
            .elements()
            .filter(move |e| e.is_comp(list))
            .flat_map(El::elements)
    }

    fn model(&self) -> Option<&El> {
        self.root.elements().find(|e| e.is_core("model"))
    }
}

/// Joins a relative source to the directory of the document which names it.
fn join(base: &str, source: &str) -> String {
    if source.contains("://") || source.starts_with('/') {
        return source.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in source.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn directory(path: &str) -> String {
    match path.rfind('/') {
        Some(k) => path[..=k].to_string(),
        None => String::new(),
    }
}

struct Flattener<'a> {
    sources: Option<Sources<'a>>,
    /// The namespace of SBML core of the document which is read.
    core: String,
    docs: Vec<Doc>,
    /// The documents by their path, read once.
    paths: HashMap<String, usize>,
    next: usize,
    /// The element which replaced an element, by their handles.
    replaced: HashMap<usize, usize>,
    /// The models being flattened, by their document and id, which stops a cycle.
    stack: Vec<(usize, String)>,
}

impl Flattener<'_> {
    /// The model definition `id` of a document, with the document it is in.
    fn definition(&mut self, doc: usize, id: &str) -> Result<(usize, El), ReadError> {
        if let Some(model) = self.docs[doc]
            .definitions("listOfModelDefinitions")
            .find(|e| e.attr("id") == Some(id))
        {
            return Ok((doc, model.clone()));
        }
        let Some(external) = self.docs[doc]
            .definitions("listOfExternalModelDefinitions")
            .find(|e| e.comp_attr("id") == Some(id))
        else {
            return Err(err(format!(
                "the model '{id}' of a submodel is no model definition of the document"
            )));
        };
        let source = external.comp_attr("source").ok_or_else(|| {
            err(format!(
                "the external model definition '{id}' has no comp:source"
            ))
        })?;
        let model_ref = external.comp_attr("modelRef").map(str::to_string);
        let path = join(&self.docs[doc].base, source);
        let other = self.document(&path, id)?;
        match model_ref {
            None => self.docs[other]
                .model()
                .cloned()
                .map(|model| (other, model))
                .ok_or_else(|| err(format!("the document '{path}' has no model"))),
            Some(model_ref) => {
                if let Some(model) = self.docs[other]
                    .model()
                    .filter(|m| m.attr("id") == Some(model_ref.as_str()))
                {
                    return Ok((other, model.clone()));
                }
                if self.stack.len() > MAX_DEPTH {
                    return Err(err("the external model definitions refer to each other"));
                }
                self.stack.push((other, format!("external {model_ref}")));
                let found = self.definition(other, &model_ref);
                self.stack.pop();
                found
            }
        }
    }

    /// The index of the document of a path, read once.
    fn document(&mut self, path: &str, id: &str) -> Result<usize, ReadError> {
        if let Some(&k) = self.paths.get(path) {
            return Ok(k);
        }
        let Some(sources) = self.sources else {
            return Err(err(format!(
                "the external model definition '{id}' is the document '{path}', which a read of \
                 one document cannot read; `read_with` reads it"
            )));
        };
        let text = sources(path).map_err(|message| {
            err(format!(
                "the document '{path}' of the external model definition '{id}': {message}"
            ))
        })?;
        let parsed = roxmltree::Document::parse(&text)
            .map_err(|e| err(format!("the document '{path}' is no XML: {e}")))?;
        let root = El::from_xml(parsed.root_element());
        if root.name != "sbml" {
            return Err(err(format!("the document '{path}' is no SBML")));
        }
        self.docs.push(Doc {
            root,
            base: directory(path),
        });
        self.paths.insert(path.to_string(), self.docs.len() - 1);
        Ok(self.docs.len() - 1)
    }

    /// The element a reference names in a scope, the replacement of a replaced one.
    fn resolve(&self, scope: &Scope, reference: &Ref) -> Result<usize, ReadError> {
        let handle = match (reference.kind, &reference.child) {
            (RefKind::Port, child) => {
                let port = scope.ports.get(&reference.value).ok_or_else(|| {
                    err(format!(
                        "the port '{}' is no port of the model",
                        reference.value
                    ))
                })?;
                if child.is_some() {
                    return Err(err(format!(
                        "the reference to the port '{}' has an sBaseRef",
                        reference.value
                    )));
                }
                return self.resolve(scope, port);
            }
            (RefKind::Id, Some(child)) => {
                let submodel = scope.submodels.get(&reference.value).ok_or_else(|| {
                    err(format!(
                        "'{}' with an sBaseRef is no submodel of the model",
                        reference.value
                    ))
                })?;
                return self.resolve(submodel, child);
            }
            (RefKind::MetaId, Some(child)) => {
                let submodel = scope
                    .submodel_metaids
                    .get(&reference.value)
                    .and_then(|id| scope.submodels.get(id))
                    .ok_or_else(|| {
                        err(format!(
                            "the meta id '{}' with an sBaseRef is no submodel of the model",
                            reference.value
                        ))
                    })?;
                return self.resolve(submodel, child);
            }
            (RefKind::Unit, Some(_)) => {
                return Err(err(format!(
                    "the reference to the unit '{}' has an sBaseRef",
                    reference.value
                )));
            }
            (RefKind::Id, None) => scope.ids.get(&reference.value).copied().ok_or_else(|| {
                if scope.submodels.contains_key(&reference.value) {
                    err(format!(
                        "the submodel '{}' as a whole is deleted or replaced, which is not \
                         supported",
                        reference.value
                    ))
                } else {
                    err(format!("'{}' is no element of the model", reference.value))
                }
            })?,
            (RefKind::MetaId, None) => {
                scope
                    .metaids
                    .get(&reference.value)
                    .copied()
                    .ok_or_else(|| {
                        err(format!(
                            "the meta id '{}' is no element of the model",
                            reference.value
                        ))
                    })?
            }
            (RefKind::Unit, None) => {
                scope.units.get(&reference.value).copied().ok_or_else(|| {
                    err(format!(
                        "'{}' is no unit definition of the model",
                        reference.value
                    ))
                })?
            }
        };
        let mut handle = handle;
        let mut steps = 0;
        while let Some(&next) = self.replaced.get(&handle) {
            handle = next;
            steps += 1;
            if steps > self.replaced.len() {
                return Err(err("the replacements of an element form a cycle"));
            }
        }
        Ok(handle)
    }

    /// Flattens a model of a document: its submodels, their deletions, conversions and
    /// replacements, see the module.
    fn flatten(&mut self, doc: usize, mut model: El) -> Result<(El, Scope), ReadError> {
        let id = model.attr("id").unwrap_or_default().to_string();
        if self.stack.iter().any(|(d, m)| *d == doc && *m == id) {
            return Err(err(format!("the model '{id}' contains itself")));
        }
        if self.stack.len() > MAX_DEPTH {
            return Err(err(format!(
                "the submodels are nested deeper than {MAX_DEPTH}"
            )));
        }
        self.stack.push((doc, id));
        let flattened = self
            .flatten_model(doc, &mut model)
            .map(|scope| (model, scope));
        self.stack.pop();
        flattened
    }

    fn flatten_model(&mut self, doc: usize, model: &mut El) -> Result<Scope, ReadError> {
        model.number(&mut self.next);
        let mut scope = Scope::of(model);

        let submodels: Vec<El> = model
            .child_comp("listOfSubmodels")
            .map(|list| {
                list.elements()
                    .filter(|e| e.is_comp("submodel"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut instances = Vec::new();
        let mut conversions: Vec<(Vec<usize>, Option<String>, Option<String>)> = Vec::new();
        for submodel in &submodels {
            let sid = submodel
                .comp_attr("id")
                .ok_or_else(|| err("a submodel has no comp:id"))?
                .to_string();
            let model_ref = submodel
                .comp_attr("modelRef")
                .ok_or_else(|| err(format!("the submodel '{sid}' has no comp:modelRef")))?;
            let (definition_doc, definition) = self.definition(doc, model_ref)?;
            let (mut instance, sub_scope) = self.flatten(definition_doc, definition)?;

            // the ids of the instance get the prefix of the submodel, the ones of the elements
            // the deletions remove as well, so a reference to a deleted element names the
            // element which replaces the deletion; the deletions come first, so that a math
            // which read a deleted local parameter reads the parameter of the model it shadowed
            let prefix = format!("{sid}__");
            let mut renaming = Renaming::default();
            let defined = Scope::of(&instance);
            for id in defined.ids.into_keys() {
                renaming.sids.insert(id.clone(), format!("{prefix}{id}"));
            }
            for id in defined.units.into_keys() {
                renaming.units.insert(id.clone(), format!("{prefix}{id}"));
            }
            if let Some(list) = submodel.child_comp("listOfDeletions") {
                for deletion in list.elements().filter(|e| e.is_comp("deletion")) {
                    let handle = self.resolve(&sub_scope, &Ref::of(deletion)?)?;
                    // an element inside an element which was deleted before is gone already
                    let Some(element) = instance.find(handle) else {
                        continue;
                    };
                    let deleted = element.attr("id").and_then(|id| {
                        if element.is_core("unitDefinition") {
                            renaming.units.get(id).cloned()
                        } else {
                            renaming.sids.get(id).cloned()
                        }
                    });
                    instance.remove(handle);
                    if let Some(deletion_id) = deletion.comp_attr("id") {
                        scope
                            .deleted
                            .insert((sid.clone(), deletion_id.to_string()), deleted);
                    }
                }
            }
            renaming.apply(&mut instance);
            prefix_metaids(&mut instance, &prefix);

            // the conversions of time and extent, after the replacements: an element of the
            // model which replaces one of the submodel is in the units of the model
            let time = submodel.comp_attr("timeConversionFactor");
            let extent = submodel.comp_attr("extentConversionFactor");
            if time.is_some() || extent.is_some() {
                let items = instance
                    .elements()
                    .filter(|list| LISTS.contains(&list.name.as_str()))
                    .flat_map(El::elements)
                    .map(|e| e.handle)
                    .collect();
                conversions.push((items, time.map(str::to_string), extent.map(str::to_string)));
            }

            // the conversion factor of the model of the submodel is the one of its species
            if let Some(factor) = instance.attr("conversionFactor").map(str::to_string) {
                for list in instance
                    .elements_mut()
                    .filter(|e| e.is_core("listOfSpecies"))
                {
                    for species in list.elements_mut().filter(|e| e.is_core("species")) {
                        if species.attr("conversionFactor").is_none() {
                            species.set_attr("conversionFactor", &factor);
                        }
                    }
                }
            }
            if let Some(metaid) = submodel.attr("metaid") {
                scope
                    .submodel_metaids
                    .insert(metaid.to_string(), sid.clone());
            }
            scope.submodels.insert(sid, sub_scope);
            instances.push(instance);
        }
        for instance in instances {
            merge(model, instance, &self.core);
        }

        // the replacements of the elements of the submodels by the elements of the model
        let mut replacing: Vec<(usize, El)> = Vec::new();
        let mut replaced_by: Vec<(usize, El)> = Vec::new();
        collect_replacements(model, &mut replacing, &mut replaced_by);
        for (parent, replaced) in replacing {
            let submodel = replaced
                .comp_attr("submodelRef")
                .ok_or_else(|| err("a replaced element has no comp:submodelRef"))?
                .to_string();
            let sub_scope = scope
                .submodels
                .get(&submodel)
                .ok_or_else(|| err(format!("'{submodel}' of a replaced element is no submodel")))?;
            let factor = replaced.comp_attr("conversionFactor").map(str::to_string);
            // the element of the model which replaces, gone when it was replaced itself
            let Some(new) = model.find(parent) else {
                continue;
            };
            let (new_id, new_unit) = (
                new.attr("id").map(str::to_string),
                new.is_core("unitDefinition"),
            );
            if let Some(deletion) = replaced.comp_attr("deletion") {
                let deleted = scope
                    .deleted
                    .get(&(submodel.clone(), deletion.to_string()))
                    .ok_or_else(|| {
                        err(format!(
                            "'{deletion}' is no deletion of the submodel '{submodel}'"
                        ))
                    })?
                    .clone();
                if let (Some(old), Some(new)) = (deleted, new_id) {
                    replace_references(model, &old, &new, factor.as_deref(), new_unit);
                }
                continue;
            }
            let handle = self.resolve(sub_scope, &Ref::of(&replaced)?)?;
            if handle == parent {
                continue;
            }
            let old = model
                .find(handle)
                .ok_or_else(|| err("a replaced element is no element of the flattened model"))?;
            let (old_id, unit) = (
                old.attr("id").map(str::to_string),
                old.is_core("unitDefinition"),
            );
            model.remove(handle);
            if let (Some(old), Some(new)) = (old_id, &new_id) {
                replace_references(model, &old, new, factor.as_deref(), unit);
            }
            self.replaced.insert(handle, parent);
        }
        for (parent, replacement) in replaced_by {
            let submodel = replacement
                .comp_attr("submodelRef")
                .ok_or_else(|| err("a replacedBy has no comp:submodelRef"))?
                .to_string();
            let sub_scope = scope
                .submodels
                .get(&submodel)
                .ok_or_else(|| err(format!("'{submodel}' of a replacedBy is no submodel")))?;
            let handle = self.resolve(sub_scope, &Ref::of(&replacement)?)?;
            let Some(old) = model.find(parent) else {
                continue;
            };
            let (id, metaid) = (
                old.attr("id").map(str::to_string),
                old.attr("metaid").map(str::to_string),
            );
            model.remove(parent);
            let element = model
                .find_mut(handle)
                .ok_or_else(|| err("a replacement is no element of the flattened model"))?;
            let unit = element.is_core("unitDefinition");
            let previous = element.attr("id").map(str::to_string);
            if let Some(id) = &id {
                element.set_attr("id", id);
            }
            match &metaid {
                Some(metaid) => element.set_attr("metaid", metaid),
                None => element.remove_attr("metaid"),
            }
            if let (Some(previous), Some(id)) = (previous, &id) {
                replace_references(model, &previous, id, None, unit);
            }
            self.replaced.insert(parent, handle);
        }
        for (items, time, extent) in conversions {
            // the reactions of the submodel which were not replaced
            let reactions = items
                .iter()
                .filter_map(|&h| model.find(h))
                .filter(|e| e.is_core("reaction"))
                .filter_map(|e| e.attr("id"))
                .map(str::to_string)
                .collect();
            let conversion = Conversion {
                time: time.as_deref(),
                extent: extent.as_deref(),
                reactions,
            };
            for handle in items {
                if let Some(element) = model.find_mut(handle) {
                    conversion.apply(element);
                }
            }
        }
        model.strip_comp();
        Ok(scope)
    }
}

fn prefix_metaids(e: &mut El, prefix: &str) {
    if e.is_math() || matches!(e.name.as_str(), "annotation" | "notes") {
        return;
    }
    for a in &mut e.attrs {
        if a.ns.is_none() && a.name == "metaid" {
            a.value = format!("{prefix}{}", a.value);
        }
    }
    for c in e.elements_mut() {
        prefix_metaids(c, prefix);
    }
}

/// Makes the references to `old` references to `new`; with a conversion factor a math reads
/// `new / factor` and an assignment assigns its math times the factor.
fn replace_references(model: &mut El, old: &str, new: &str, factor: Option<&str>, unit: bool) {
    let mut renaming = Renaming::default();
    if unit {
        renaming.units.insert(old.to_string(), new.to_string());
        renaming.apply(model);
        return;
    }
    if let Some(factor) = factor {
        replace_ci(model, old, &|x| apply("divide", vec![x, ci(factor)]));
        scale_assignments(model, old, factor);
    }
    renaming.sids.insert(old.to_string(), new.to_string());
    renaming.apply(model);
}

/// The replaced elements and the replacements of the elements of a model, by the handles of
/// the elements which have them.
fn collect_replacements(
    e: &El,
    replacing: &mut Vec<(usize, El)>,
    replaced_by: &mut Vec<(usize, El)>,
) {
    if e.is_math() || matches!(e.name.as_str(), "annotation" | "notes") {
        return;
    }
    for c in e.elements() {
        if c.is_comp("listOfReplacedElements") {
            for r in c.elements().filter(|r| r.is_comp("replacedElement")) {
                replacing.push((e.handle, r.clone()));
            }
        } else if c.is_comp("replacedBy") {
            replaced_by.push((e.handle, c.clone()));
        } else if !is_comp(c.ns.as_deref()) {
            collect_replacements(c, replacing, replaced_by);
        }
    }
}

/// Appends the elements of the lists of a flattened submodel to the lists of the model.
fn merge(model: &mut El, instance: El, core: &str) {
    for list in instance.children.into_iter().filter_map(|c| match c {
        Item::El(e)
            if LISTS.contains(&e.name.as_str()) && e.ns.as_deref().is_some_and(is_sbml_ns) =>
        {
            Some(e)
        }
        _ => None,
    }) {
        let mut items: Vec<Item> = list
            .children
            .into_iter()
            .filter(|c| matches!(c, Item::El(_)))
            .collect();
        for item in &mut items {
            if let Item::El(e) = item {
                e.move_to_core(core);
            }
        }
        let position = |name: &str| LISTS.iter().position(|l| *l == name).unwrap_or(LISTS.len());
        let target = match model
            .children
            .iter()
            .position(|c| matches!(c, Item::El(e) if e.is_core(&list.name)))
        {
            Some(k) => k,
            None => {
                let rank = position(&list.name);
                let at = model
                    .children
                    .iter()
                    .position(|c| match c {
                        Item::El(e) if e.ns.as_deref().is_some_and(is_sbml_ns) => {
                            LISTS.contains(&e.name.as_str()) && position(&e.name) > rank
                        }
                        Item::El(e) => is_comp(e.ns.as_deref()),
                        Item::Text(_) => false,
                    })
                    .unwrap_or(model.children.len());
                model
                    .children
                    .insert(at, Item::El(El::new(core, &list.name)));
                at
            }
        };
        if let Item::El(target) = &mut model.children[target] {
            target.children.extend(items);
        }
    }
}

/// Flattens a document which uses the package into the text of a document of SBML core.
pub(crate) fn flatten(
    doc: &roxmltree::Document,
    sources: Option<Sources>,
) -> Result<String, ReadError> {
    let mut root = El::from_xml(doc.root_element());
    let core = root.ns.clone().unwrap_or_default();
    let model = root
        .elements()
        .find(|e| e.is_core("model"))
        .cloned()
        .ok_or_else(|| err("<sbml> without a <model>"))?;
    let mut flattener = Flattener {
        sources,
        core: core.clone(),
        docs: vec![Doc {
            root: root.clone(),
            base: String::new(),
        }],
        paths: HashMap::new(),
        next: 0,
        replaced: HashMap::new(),
        stack: Vec::new(),
    };
    let (mut flat, _) = flattener.flatten(0, model)?;
    flat.move_to_core(&core);
    root.attrs.retain(|a| !is_comp(a.ns.as_deref()));
    root.children
        .retain(|c| !matches!(c, Item::El(e) if is_comp(e.ns.as_deref())));
    for c in &mut root.children {
        if let Item::El(e) = c
            && e.is_core("model")
        {
            *e = flat;
            break;
        }
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    root.write(None, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::join;

    #[test]
    fn a_source_is_joined_to_the_directory_of_its_document() {
        assert_eq!(join("", "a.xml"), "a.xml");
        assert_eq!(join("sub/", "a.xml"), "sub/a.xml");
        assert_eq!(join("sub/", "../a.xml"), "a.xml");
        assert_eq!(join("sub/", "./b/a.xml"), "sub/b/a.xml");
        assert_eq!(
            join("sub/", "http://example.org/a.xml"),
            "http://example.org/a.xml"
        );
    }
}

//! The fields every SBML object shares.

/// The fields every SBML object shares.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Package-qualified attributes, retained independently of the core namespace.
    pub package_attributes: Vec<Attribute>,
    /// The package extension trees directly owned by this object.
    pub extensions: Vec<Extension>,
    /// List containers which state metadata of their own.
    pub lists: Vec<ListMetadata>,
    /// The `id` attribute.
    pub id: Option<String>,
    /// The `metaid` attribute.
    pub meta_id: Option<String>,
    /// The `name` attribute.
    pub name: Option<String>,
    /// The `sboTerm` attribute, as the id of the term.
    pub sbo: Option<String>,
    /// The text of the `notes` element.
    pub notes: Option<String>,
    /// The text of the `annotation` element.
    pub annotation: Option<String>,
    /// The controlled annotations parsed from the RDF of this element.
    pub cvterms: Vec<CvTerm>,
    /// The creators and dates parsed from the RDF of this element.
    pub history: Option<History>,
    /// The element serialized back to XML.
    pub xml: Option<String>,
    /// The line where the element starts in the source.
    pub line: u32,
    /// The column where the element starts in the source.
    pub column: u32,
}

/// One controlled annotation, including annotations of that relation itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CvTerm {
    /// The wire name of the model or biological qualifier.
    pub qualifier: String,
    /// Resource URIs in their authored order.
    pub resources: Vec<String>,
    /// Qualifiers of this relation, in their authored order.
    pub nested: Vec<CvTerm>,
}

/// A creator named in the history of an element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Creator {
    /// The given name, absent when it is not stated.
    pub given_name: Option<String>,
    /// The family name, absent when it is not stated.
    pub family_name: Option<String>,
    /// The organization, absent when it is not stated.
    pub organization: Option<String>,
    /// The email address, absent when it is not stated.
    pub email: Option<String>,
}

/// The history of an element, with dates normalized by the reader as SBML defines them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    /// The creators, in their authored order.
    pub creators: Vec<Creator>,
    /// The creation date, absent when none is stated.
    pub created_date: Option<String>,
    /// The modification dates, in their authored order.
    pub modified_dates: Vec<String>,
}

/// An authored attribute of a package extension.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attribute {
    /// Namespace URI; absent for unqualified attributes.
    pub namespace: Option<String>,
    /// Local attribute name.
    pub name: String,
    /// Authored value.
    pub value: String,
}

/// An owned package object or package list, independent of any XML implementation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extension {
    /// Common SBML metadata and extensions of this object.
    pub element: Element,
    /// Package name, identified by its namespace URI rather than its prefix.
    pub package: String,
    /// Local XML element name.
    pub name: String,
    /// Authored attributes, including attributes without a namespace.
    pub attributes: Vec<Attribute>,
    /// Content MathML owned by this object.
    pub math: Option<crate::Ast>,
    /// Child objects, preserving list and document order.
    pub children: Vec<Extension>,
}

/// The metadata of a ListOf, without duplicating its child objects.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListMetadata {
    /// Common metadata and package extensions on the list itself.
    pub element: Element,
    /// Local XML element name.
    pub name: String,
    /// Number of list members, excluding the qual default term.
    pub size: usize,
}

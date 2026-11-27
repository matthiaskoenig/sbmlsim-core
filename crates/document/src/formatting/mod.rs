//! Stable textual formatting shared by reports and source generators.
use crate::{AstKind, MAX_AST_DEPTH};
mod formula;
mod number;
pub use formula::{formula, significant_15};
pub use number::python_float;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MathError {
    /// The syntax tree is nested deeper than [`MAX_AST_DEPTH`].
    #[error("the math is nested deeper than {MAX_AST_DEPTH} levels")]
    TooDeep,
    /// The tree holds a node of a kind no writer knows, which only
    /// [`AstKind::Other`] is: the kind the libsbml shim answers for a type it
    /// does not map, a node of a package this crate does not write.
    #[error("the math holds a node of the kind {0:?}, which no writer writes")]
    UnknownKind(AstKind),
    /// A node lacks a child a writer of its kind reads.
    ///
    /// Every writer asks for a child of a node whose shape it has established,
    /// so a tree the libsbml shim filled never answers this; it is here so that
    /// no writer indexes into the children of a node and panics instead.
    #[error("a node of the math lacks a child its writer reads")]
    MissingChild,
}

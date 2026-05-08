//! Type expressions appearing in source (annotations, generics, etc.).
//!
//! Distinct from runtime types — these are the *syntactic* forms the
//! parser reads. The type checker resolves them against actual type
//! definitions.

use crate::span::Spanned;

/// A type expression as written in source code.
///
/// V0.1 supports: named types, single-level generics (parsed but not yet
/// resolved), tuple types, nullable wrapper `T?`, and function types for
/// closures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeExpr {
    /// A named type: `Integer`, `String`, `Trilean`, `MyAlias`.
    Named(String),

    /// A generic instantiation: `Option<Integer>`, `List<String>`.
    ///
    /// V0.1 parses this but the type checker rejects unknown generics
    /// since user-defined generics arrive in v0.2.
    Generic {
        /// Type constructor name (e.g. `Option`).
        name: String,
        /// Type arguments inside `<...>`.
        arguments: Vec<Spanned<Self>>,
    },

    /// Tuple type: `(Integer, Trilean)`, `(String, String, Integer)`.
    Tuple(Vec<Spanned<Self>>),

    /// Nullable type: `Integer?`. Wraps any inner type.
    Nullable(Box<Spanned<Self>>),

    /// Function type used in closure annotations: `(Integer) -> String`.
    Function {
        /// Parameter types (positional).
        parameters: Vec<Spanned<Self>>,
        /// Return type.
        return_type: Box<Spanned<Self>>,
    },
}

//! Type expressions appearing in source (annotations, generics, etc.).
//!
//! Distinct from runtime types — these are the *syntactic* forms the
//! parser reads. The type checker resolves them against actual type
//! definitions.

use crate::arena::TypeId;

/// A type expression as written in source code.
///
/// V0.1 supports: named types, single-level generics (parsed but not yet
/// resolved), tuple types, nullable wrapper `T?`, and function types for
/// closures. Recursive children are stored as `TypeId` handles.
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
        arguments: Vec<TypeId>,
    },

    /// Tuple type: `(Integer, Trilean)`, `(String, String, Integer)`.
    Tuple(Vec<TypeId>),

    /// Nullable type: `Integer?`. Wraps any inner type.
    Nullable(TypeId),

    /// Function type used in closure annotations: `(Integer) -> String`.
    Function {
        /// Parameter types (positional).
        parameters: Vec<TypeId>,
        /// Return type.
        return_type: TypeId,
    },

    /// Outcome type (v0.7.4.3-error per [ADR-0020]):
    ///
    /// - `T~E` binary outcome (`allow_null_state = false`): success T or
    ///   failure E. `Trit::Zero` state invalid (typecheck E1025).
    /// - `T?~E` ternary outcome (`allow_null_state = true`): success T,
    ///   null (`Trit::Zero`), or failure E. Parses from `?~` compound token.
    ///
    /// [ADR-0020]: ../../../../docs/decisions/0020-outcome-error-handling.md
    Outcome {
        /// Success-arm payload type.
        value_type: TypeId,
        /// Failure-arm payload type.
        error_type: TypeId,
        /// True when the type allows a null state (`Trit::Zero` arm
        /// carrying no payload). Parsed from `T?~E` compound; false
        /// for binary `T~E`.
        allow_null_state: bool,
    },

    /// Refined Trilean type `Trilean!` per [ADR-0021] §2.7. Parsed only
    /// after a bare `Trilean` identifier — `Integer!` and friends are
    /// rejected at parse time because there is no refinement concept
    /// for other types in v0.7. Resolves to `Type::Trilean { refined: true }`.
    ///
    /// [ADR-0021]: ../../../../docs/decisions/0021-trilean-refinement.md
    RefinedTrilean,
}

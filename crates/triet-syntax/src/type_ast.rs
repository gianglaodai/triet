//! Type expressions appearing in source (annotations, generics, etc.).
//!
//! Distinct from runtime types — these are the *syntactic* forms the
//! parser reads. The type checker resolves them against actual type
//! definitions.

use crate::arena::TypeId;

/// Reference ownership form per ADR-0022 §2.
///
/// Each of the 5 reference forms maps to one arm of the trit-based
/// ownership scheme. The `mutable` field distinguishes frozen vs mutable
/// for `&+` (strong) and `&0` (neutral borrow) forms. `&-` (weak) is
/// always immutable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReferenceForm {
    /// `&+ T` — strong owner, frozen (immutable).
    StrongFrozen,
    /// `&+ mutable T` — strong owner, mutable.
    StrongMutable,
    /// `&0 T` — neutral borrow, read-only.
    BorrowReadOnly,
    /// `&0 mutable T` — neutral borrow, exclusive mutable.
    BorrowExclusiveMutable,
    /// `&- T` — weak observer, always immutable.
    WeakObserver,
}

impl ReferenceForm {
    /// Returns true when the reference permits mutation.
    pub const fn is_mutable(self) -> bool {
        matches!(self, Self::StrongMutable | Self::BorrowExclusiveMutable)
    }

    /// Returns true when this is an owning reference (`&+` family).
    pub const fn is_owning(self) -> bool {
        matches!(self, Self::StrongFrozen | Self::StrongMutable)
    }

    /// Returns true when this is a scope borrow (`&0` family).
    pub const fn is_borrow(self) -> bool {
        matches!(self, Self::BorrowReadOnly | Self::BorrowExclusiveMutable)
    }

    /// Returns the trit polarity: +1 for strong, 0 for neutral, -1 for weak.
    pub const fn polarity_trit(self) -> i8 {
        match self {
            Self::StrongFrozen | Self::StrongMutable => 1,
            Self::BorrowReadOnly | Self::BorrowExclusiveMutable => 0,
            Self::WeakObserver => -1,
        }
    }
}

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

    /// Reference type per ADR-0022 §2. Wraps an inner type with one of
    /// the 5 reference forms: `&+ T`, `&+ mutable T`, `&0 T`,
    /// `&0 mutable T`, or `&- T`. V0.8 parses and stores the form;
    /// enforcement is deferred to v0.9+ per ADR-0025 §12.
    Reference {
        /// Which reference form applies.
        form: ReferenceForm,
        /// The inner type (the payload behind the reference).
        inner: TypeId,
    },
}

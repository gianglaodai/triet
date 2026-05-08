//! Type-checker error types.

use thiserror::Error;
use triet_syntax::Span;

use crate::types::Type;

/// An error raised while type-checking a `Program`.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TypeError {
    /// A type expression names a type the checker doesn't recognize.
    #[error("unknown type `{name}` at byte {span:?}")]
    UnknownType {
        /// The unrecognized name.
        name: String,
        /// Source location.
        span: Span,
    },

    /// An identifier is referenced but not bound in scope.
    #[error("undefined name `{name}` at byte {span:?}")]
    UndefinedName {
        /// The unbound identifier.
        name: String,
        /// Source location.
        span: Span,
    },

    /// Two values were expected to share a type (e.g. branches of an
    /// `if`, both sides of `==`) but didn't.
    #[error("type mismatch at byte {span:?}: expected {expected}, found {found}")]
    Mismatch {
        /// Type the checker expected based on context.
        expected: Type,
        /// Type the checker actually saw.
        found: Type,
        /// Source location of the mismatched expression.
        span: Span,
    },

    /// An operator was applied to operands whose types are not allowed.
    #[error(
        "invalid operands for `{operator}` at byte {span:?}: \
         expected {expected_description}, found {left} and {right}"
    )]
    InvalidOperands {
        /// Operator symbol or name.
        operator: String,
        /// Description of acceptable operand types.
        expected_description: String,
        /// Type of the left operand.
        left: Type,
        /// Type of the right operand.
        right: Type,
        /// Source location of the operator.
        span: Span,
    },

    /// A unary operator was applied to a type that doesn't support it.
    #[error("invalid operand for `{operator}` at byte {span:?}: found {operand}")]
    InvalidUnary {
        /// Operator symbol.
        operator: String,
        /// Operand type encountered.
        operand: Type,
        /// Source location.
        span: Span,
    },

    /// Function called with the wrong number of arguments.
    #[error(
        "wrong number of arguments at byte {span:?}: \
         expected {expected}, found {found}"
    )]
    WrongArity {
        /// Expected argument count.
        expected: usize,
        /// Actual argument count.
        found: usize,
        /// Source location of the call.
        span: Span,
    },

    /// A non-callable expression appeared in call position.
    #[error("type {found} is not callable at byte {span:?}")]
    NotCallable {
        /// Type the checker found at the callee position.
        found: Type,
        /// Source location.
        span: Span,
    },

    /// `if` (without `?`) used a possibly-unknown Trilean condition.
    /// Per SPEC §7.1.1, plain `if` requires a definite Trilean — use
    /// `if?` to treat `Unknown` as `False`, or call `.assume_known()`.
    #[error(
        "condition at byte {span:?} may be `unknown`; \
         use `if?` or `.assume_known()` to be explicit"
    )]
    AmbiguousCondition {
        /// Source location of the condition.
        span: Span,
    },

    /// `if` condition (or `while` / guard) is not `Trilean`.
    #[error(
        "condition at byte {span:?} must be `Trilean`, found {found}"
    )]
    NonTrileanCondition {
        /// Type encountered.
        found: Type,
        /// Source location.
        span: Span,
    },

    /// A duplicate name was declared in the same scope.
    #[error("duplicate declaration of `{name}` at byte {span:?}")]
    DuplicateName {
        /// The duplicated name.
        name: String,
        /// Source location of the second declaration.
        span: Span,
    },

    /// `null` literal used in a context that doesn't expect a nullable.
    #[error("`null` literal at byte {span:?} is only valid where a `T?` is expected")]
    NullLiteralInNonNullableContext {
        /// Source location.
        span: Span,
    },

    /// `?.`, `?:`, or `!!` applied to a non-nullable receiver.
    #[error(
        "`{operator}` requires a nullable receiver, found {found} at byte {span:?}"
    )]
    NotNullable {
        /// Operator symbol.
        operator: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        span: Span,
    },

    /// Match arm body types disagree.
    #[error(
        "match arm at byte {span:?} returns {found} but earlier arms return {expected}"
    )]
    MatchArmMismatch {
        /// Type of earlier arms.
        expected: Type,
        /// Type of this arm.
        found: Type,
        /// Source location of the offending arm.
        span: Span,
    },

    /// Tuple index out of range.
    #[error(
        "tuple index {index} out of range at byte {span:?} \
         (tuple has {length} element(s))"
    )]
    TupleIndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Tuple length.
        length: usize,
        /// Source location.
        span: Span,
    },

    /// Field access on a non-tuple type (until structs land in v0.2).
    #[error(
        "type {found} has no field or method named `{member}` at byte {span:?}"
    )]
    UnknownMember {
        /// Member name as written.
        member: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        span: Span,
    },
}

impl TypeError {
    /// Returns the byte span of the error for diagnostic anchoring.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::UnknownType { span, .. }
            | Self::UndefinedName { span, .. }
            | Self::Mismatch { span, .. }
            | Self::InvalidOperands { span, .. }
            | Self::InvalidUnary { span, .. }
            | Self::WrongArity { span, .. }
            | Self::NotCallable { span, .. }
            | Self::AmbiguousCondition { span }
            | Self::NonTrileanCondition { span, .. }
            | Self::DuplicateName { span, .. }
            | Self::NullLiteralInNonNullableContext { span }
            | Self::NotNullable { span, .. }
            | Self::MatchArmMismatch { span, .. }
            | Self::TupleIndexOutOfRange { span, .. }
            | Self::UnknownMember { span, .. } => span.clone(),
        }
    }
}

//! Type-checker error types.

use miette::Diagnostic;
use thiserror::Error;
use triet_syntax::Span;

use crate::types::Type;

/// An error raised while type-checking a `Program`.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum TypeError {
    /// A type expression names a type the checker doesn't recognize.
    #[error("unknown type `{name}`")]
    #[diagnostic(code(triet::typecheck::E1001), help("built-in types are: Trit, Tryte, Integer, Long, Trilean, String"))]
    UnknownType {
        /// The unrecognized name.
        name: String,
        /// Source location.
        #[label("unknown type")]
        span: Span,
    },

    /// An identifier is referenced but not bound in scope.
    #[error("undefined name `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1002),
        help("did you forget to declare this variable with `let`, or define this function with `fn`?")
    )]
    UndefinedName {
        /// The unbound identifier.
        name: String,
        /// Source location.
        #[label("not found in scope")]
        span: Span,
    },

    /// Two values were expected to share a type but didn't.
    #[error("type mismatch: expected {expected}, found {found}")]
    #[diagnostic(code(triet::typecheck::E1003))]
    Mismatch {
        /// Type the checker expected based on context.
        expected: Type,
        /// Type the checker actually saw.
        found: Type,
        /// Source location of the mismatched expression.
        #[label("expected `{expected}`, found `{found}`")]
        span: Span,
    },

    /// An operator was applied to operands whose types are not allowed.
    #[error("invalid operands for `{operator}`: expected {expected_description}, found {left} and {right}")]
    #[diagnostic(code(triet::typecheck::E1004))]
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
        #[label("`{operator}` applied to `{left}` and `{right}`")]
        span: Span,
    },

    /// A unary operator was applied to a type that doesn't support it.
    #[error("invalid operand for `{operator}`: found {operand}")]
    #[diagnostic(code(triet::typecheck::E1005), help("`-`/`!`/`not` work on numeric types (Trit, Tryte, Integer, Long) and Trilean"))]
    InvalidUnary {
        /// Operator symbol.
        operator: String,
        /// Operand type encountered.
        operand: Type,
        /// Source location.
        #[label("cannot apply `{operator}` to `{operand}`")]
        span: Span,
    },

    /// Function called with the wrong number of arguments.
    #[error("wrong number of arguments: expected {expected}, found {found}")]
    #[diagnostic(code(triet::typecheck::E1006))]
    WrongArity {
        /// Expected argument count.
        expected: usize,
        /// Actual argument count.
        found: usize,
        /// Source location of the call.
        #[label("expected {expected} argument(s), got {found}")]
        span: Span,
    },

    /// A non-callable expression appeared in call position.
    #[error("type {found} is not callable")]
    #[diagnostic(code(triet::typecheck::E1007), help("only functions and closures can be called with `(...)`"))]
    NotCallable {
        /// Type the checker found at the callee position.
        found: Type,
        /// Source location.
        #[label("`{found}` is not a function")]
        span: Span,
    },

    /// `if` (without `?`) used a possibly-unknown Trilean condition.
    #[error("condition may be `unknown`")]
    #[diagnostic(
        code(triet::typecheck::E1008),
        help("replace `if` with `if?` to treat unknown as false, or call `.assume_known()` if you are certain the value is known")
    )]
    AmbiguousCondition {
        /// Source location of the condition.
        #[label("this condition could be `unknown`")]
        span: Span,
    },

    /// `if` condition is not `Trilean`.
    #[error("condition must be `Trilean`, found {found}")]
    #[diagnostic(code(triet::typecheck::E1009), help("condition expressions must evaluate to a `Trilean` value (true, false, or unknown)"))]
    NonTrileanCondition {
        /// Type encountered.
        found: Type,
        /// Source location.
        #[label("this is `{found}`, not `Trilean`")]
        span: Span,
    },

    /// A duplicate name was declared in the same scope.
    #[error("duplicate declaration of `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1010),
        help("rename one of the declarations, or remove the duplicate")
    )]
    DuplicateName {
        /// The duplicated name.
        name: String,
        /// Source location of the second declaration.
        #[label("`{name}` already declared in this scope")]
        span: Span,
    },

    /// `null` literal used in a context that doesn't expect a nullable.
    #[error("`null` literal is only valid where a `T?` is expected")]
    #[diagnostic(
        code(triet::typecheck::E1011),
        help("wrap the expected type in `T?` (e.g. `Integer?`) to allow null")
    )]
    NullLiteralInNonNullableContext {
        /// Source location.
        #[label("`null` is not valid here")]
        span: Span,
    },

    /// `?.`, `?:`, or `!!` applied to a non-nullable receiver.
    #[error("`{operator}` requires a nullable receiver, found {found}")]
    #[diagnostic(
        code(triet::typecheck::E1012),
        help("`{operator}` only works on nullable types (e.g. `Integer?`); the receiver `{found}` is not nullable")
    )]
    NotNullable {
        /// Operator symbol.
        operator: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        #[label("`{found}` is not nullable")]
        span: Span,
    },

    /// Match arm body types disagree.
    #[error("match arm returns {found} but earlier arms return {expected}")]
    #[diagnostic(code(triet::typecheck::E1013), help("all arms of a `match` must have the same return type"))]
    MatchArmMismatch {
        /// Type of earlier arms.
        expected: Type,
        /// Type of this arm.
        found: Type,
        /// Source location of the offending arm.
        #[label("this arm returns `{found}`")]
        span: Span,
    },

    /// Tuple index out of range.
    #[error("tuple index {index} out of range (tuple has {length} element(s))")]
    #[diagnostic(code(triet::typecheck::E1014))]
    TupleIndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Tuple length.
        length: usize,
        /// Source location.
        #[label("index {index} exceeds tuple length {length}")]
        span: Span,
    },

    /// Field access on a type without that member.
    #[error("type {found} has no field or method named `{member}`")]
    #[diagnostic(code(triet::typecheck::E1015))]
    UnknownMember {
        /// Member name as written.
        member: String,
        /// Receiver type.
        found: Type,
        /// Source location.
        #[label("`{found}` has no member `{member}`")]
        span: Span,
    },

    /// Assignment target is bound `let` (immutable).
    #[error("cannot assign to immutable binding `{name}`")]
    #[diagnostic(
        code(triet::typecheck::E1016),
        help("declare this binding with `let mut {name} = ...` to allow reassignment")
    )]
    AssignToImmutable {
        /// Target binding name.
        name: String,
        /// Source location of the assignment statement.
        #[label("`{name}` is immutable")]
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
            | Self::UnknownMember { span, .. }
            | Self::AssignToImmutable { span, .. } => span.clone(),
        }
    }
}

//! Runtime error types raised during interpretation.

use thiserror::Error;
use triet_syntax::Span;

/// An error encountered while running a Triết program.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    /// `main` function not found in the program.
    #[error("program has no `main` function")]
    NoMainFunction,

    /// A name was looked up at runtime but not bound. The type checker
    /// usually catches this; this variant covers cases where the
    /// program ran without type-checking.
    #[error("undefined name `{name}` at byte {span:?}")]
    UndefinedName {
        /// The unbound identifier.
        name: String,
        /// Source location where the lookup failed.
        span: Span,
    },

    /// A condition required to be a definite Trilean turned out to be
    /// `Unknown`; the user can avoid this with `if?`/`while?` or
    /// `.assume_known()`.
    #[error("condition was `unknown` (use `if?` or `.assume_known()` to handle) at byte {span:?}")]
    UnknownCondition {
        /// Source location of the condition.
        span: Span,
    },

    /// `match` was non-exhaustive — no arm matched the scrutinee.
    #[error("no `match` arm matched the value at byte {span:?}")]
    NonExhaustiveMatch {
        /// Source location of the `match` expression.
        span: Span,
    },

    /// A built-in or arithmetic operation panicked (overflow, division
    /// by zero, force-unwrap on null, ...).
    #[error("runtime panic at byte {span:?}: {message}")]
    Panic {
        /// Cause of the panic.
        message: String,
        /// Source location.
        span: Span,
    },

    /// A function was called with the wrong number of arguments. The
    /// type checker catches this for direct calls; this guards
    /// dynamically-resolved closures.
    #[error("wrong argument count at byte {span:?}: expected {expected}, found {found}")]
    WrongArity {
        /// Expected argument count.
        expected: usize,
        /// Actual argument count.
        found: usize,
        /// Source location of the call.
        span: Span,
    },

    /// Operator applied to incompatible value kinds — runtime fallback
    /// after the type checker. Carries the operator and a free-form
    /// description.
    #[error("type error at byte {span:?}: {message}")]
    TypeError {
        /// Free-form description.
        message: String,
        /// Source location.
        span: Span,
    },
}

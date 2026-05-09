//! Runtime error types raised during interpretation.

use miette::Diagnostic;
use thiserror::Error;
use triet_syntax::Span;

/// An error encountered while running a Triết program.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum RuntimeError {
    /// `main` function not found in the program.
    #[error("program has no `main` function")]
    #[diagnostic(code(triet::runtime::E2001), help("add a `fn main() {{ ... }}` to your program"))]
    NoMainFunction,

    /// A name was looked up at runtime but not bound.
    #[error("undefined name `{name}`")]
    #[diagnostic(code(triet::runtime::E2002))]
    UndefinedName {
        /// The unbound identifier.
        name: String,
        /// Source location where the lookup failed.
        #[label("not found in any scope")]
        span: Span,
    },

    /// A condition required to be a definite Trilean turned out to be `Unknown`.
    #[error("condition was `unknown`")]
    #[diagnostic(
        code(triet::runtime::E2003),
        help("use `if?` or `while?` to treat unknown as false, or call `.assume_known()`")
    )]
    UnknownCondition {
        /// Source location of the condition.
        #[label("this condition evaluated to `unknown`")]
        span: Span,
    },

    /// `match` was non-exhaustive.
    #[error("no `match` arm matched the value")]
    #[diagnostic(code(triet::runtime::E2004), help("add a wildcard arm `_ => ...` to handle this value"))]
    NonExhaustiveMatch {
        /// Source location of the `match` expression.
        #[label("no arm matched")]
        span: Span,
    },

    /// A built-in or arithmetic operation panicked.
    #[error("{message}")]
    #[diagnostic(code(triet::runtime::E2005))]
    Panic {
        /// Cause of the panic.
        message: String,
        /// Source location.
        #[label("panic occurred here")]
        span: Span,
    },

    /// Wrong number of arguments at runtime (dynamic calls).
    #[error("wrong argument count: expected {expected}, found {found}")]
    #[diagnostic(code(triet::runtime::E2006))]
    WrongArity {
        /// Expected argument count.
        expected: usize,
        /// Actual argument count.
        found: usize,
        /// Source location of the call.
        #[label("called here")]
        span: Span,
    },

    /// Operator applied to incompatible value kinds.
    #[error("{message}")]
    #[diagnostic(code(triet::runtime::E2007))]
    TypeError {
        /// Free-form description.
        message: String,
        /// Source location.
        #[label("type error")]
        span: Span,
    },
}

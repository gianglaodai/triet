//! Parser error types.

use miette::Diagnostic;
use thiserror::Error;
use triet_lexer::{LexError, Span};

/// An error encountered while parsing a token stream into an AST.
///
/// The parser accumulates errors in a `Vec<ParseError>` (with recovery)
/// and returns them all at once when `parse()` finishes. Each error
/// carries a precise [`Span`] for diagnostics.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum ParseError {
    /// A specific token was expected but a different one (or end-of-input)
    /// was found.
    #[error("expected {expected}, found {found}")]
    #[diagnostic(code(triet::parse::E0001))]
    UnexpectedToken {
        /// What the parser was looking for (e.g. `"`)`"`).
        expected: String,
        /// What was actually found (debug-formatted).
        found: String,
        /// Where the unexpected token sits in source.
        #[label("expected {expected} here")]
        span: Span,
    },

    /// End of input reached while still expecting more tokens.
    #[error("unexpected end of input; expected {expected}")]
    #[diagnostic(
        code(triet::parse::E0002),
        help("the file ended before the parser could finish — check for missing `}}` or `)`")
    )]
    UnexpectedEof {
        /// Description of what was expected.
        expected: String,
        /// Empty span at end-of-input, useful for diagnostic anchoring.
        #[label("end of file")]
        span: Span,
    },

    /// Two same-class no-chain operators appeared in sequence.
    #[error("operators of class {class} cannot be chained")]
    #[diagnostic(
        code(triet::parse::E0003),
        help("wrap each comparison in parentheses, e.g. `(a < b) and (b < c)`")
    )]
    ChainedNoChainOperator {
        /// Description of the operator class (e.g. `"comparison"`).
        class: String,
        /// Span of the second offending operator.
        #[label("second `{class}` operator")]
        span: Span,
    },

    /// An f-string interpolation `{...}` parsed an invalid expression.
    #[error("invalid expression inside f-string interpolation: {message}")]
    #[diagnostic(code(triet::parse::E0004))]
    InvalidInterpolation {
        /// Description of the underlying issue.
        message: String,
        /// Span of the interpolation block.
        #[label("inside this interpolation")]
        span: Span,
    },

    /// A literal value is malformed.
    #[error("invalid literal: {message}")]
    #[diagnostic(code(triet::parse::E0005))]
    InvalidLiteral {
        /// What is wrong with the literal.
        message: String,
        /// Where the literal is in source.
        #[label("invalid literal")]
        span: Span,
    },

    /// `break expr` appeared outside a `loop`.
    #[error("`break` with a value is only allowed inside `loop`")]
    #[diagnostic(
        code(triet::parse::E0006),
        help("`break expr` (break-with-value) is only valid inside a `loop {{ }}` block; use plain `break` in `for`/`while`")
    )]
    BreakValueOutsideLoop {
        /// Span of the `break` keyword.
        #[label("`break` used here")]
        span: Span,
    },

    /// Left-hand side of `=` is not an assignable target.
    #[error("invalid assignment target: {description}")]
    #[diagnostic(
        code(triet::parse::E0007),
        help("v0.1 only allows simple identifiers as assignment targets, e.g. `count = 1`")
    )]
    InvalidAssignmentTarget {
        /// Why the LHS is not assignable.
        description: String,
        /// Span of the offending target expression.
        #[label("not a valid assignment target")]
        span: Span,
    },

    /// An item is being defined with a name that is reserved per
    /// ADR-0005 — currently the OS-namespace roots `std`, `sys`, `dev`,
    /// `usr`, `core`. Path keywords (`crate`, `self`, `super`) are
    /// reserved at the lexer level and surface as [`Self::UnexpectedToken`].
    #[error("`{name}` is a reserved name and cannot be used for an item")]
    #[diagnostic(
        code(triet::parse::E0008),
        help(
            "`std`, `sys`, `dev`, `usr`, `core` are kept for the standard \
             library and OS-native namespaces (ADR-0005). Choose a different \
             name."
        )
    )]
    ReservedItemName {
        /// The reserved name the user tried to define.
        name: String,
        /// Where the offending identifier sits.
        #[label("reserved name")]
        span: Span,
    },

    /// Underlying lexer error encountered before parsing finished.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lex(#[from] LexError),
}

impl ParseError {
    /// Returns the byte span associated with this error.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::UnexpectedToken { span, .. }
            | Self::UnexpectedEof { span, .. }
            | Self::ChainedNoChainOperator { span, .. }
            | Self::InvalidInterpolation { span, .. }
            | Self::InvalidLiteral { span, .. }
            | Self::BreakValueOutsideLoop { span }
            | Self::InvalidAssignmentTarget { span, .. }
            | Self::ReservedItemName { span, .. } => span.clone(),
            Self::Lex(_) => 0..0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_helper_returns_correct_range() {
        let error = ParseError::UnexpectedToken {
            expected: "`)`".to_owned(),
            found: "`,`".to_owned(),
            span: 5..6,
        };
        assert_eq!(error.span(), 5..6);
    }

    #[test]
    fn display_includes_span_and_context() {
        let error = ParseError::UnexpectedToken {
            expected: "`)`".to_owned(),
            found: "`,`".to_owned(),
            span: 5..6,
        };
        let message = error.to_string();
        assert!(message.contains("expected"));
        assert!(message.contains("`)`"));
        assert!(message.contains("`,`"));
    }
}

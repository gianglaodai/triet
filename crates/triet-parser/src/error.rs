//! Parser error types.

use thiserror::Error;
use triet_lexer::{LexError, Span};

/// An error encountered while parsing a token stream into an AST.
///
/// The parser accumulates errors in a `Vec<ParseError>` (with recovery)
/// and returns them all at once when `parse()` finishes. Each error
/// carries a precise [`Span`] for diagnostics.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    /// A specific token was expected but a different one (or end-of-input)
    /// was found.
    #[error("expected {expected}, found {found} at byte {span:?}")]
    UnexpectedToken {
        /// What the parser was looking for (e.g. `"`)`"`).
        expected: String,
        /// What was actually found (debug-formatted).
        found: String,
        /// Where the unexpected token sits in source.
        span: Span,
    },

    /// End of input reached while still expecting more tokens.
    #[error("unexpected end of input; expected {expected} at byte {span:?}")]
    UnexpectedEof {
        /// Description of what was expected.
        expected: String,
        /// Empty span at end-of-input, useful for diagnostic anchoring.
        span: Span,
    },

    /// Two same-class no-chain operators appeared in sequence.
    ///
    /// Triết forbids chaining comparison (`<`, `<=`, `>`, `>=`),
    /// equality (`==`, `!=`), and range (`..`, `..=`) operators within
    /// the same level. Wrap subexpressions in parentheses to disambiguate.
    #[error("operators of class {class} cannot be chained at byte {span:?}")]
    ChainedNoChainOperator {
        /// Description of the operator class (e.g. `"comparison"`).
        class: String,
        /// Span of the second offending operator.
        span: Span,
    },

    /// An f-string interpolation `{...}` was opened but its contents did
    /// not parse as a valid expression.
    #[error("invalid expression inside f-string interpolation at byte {span:?}: {message}")]
    InvalidInterpolation {
        /// Description of the underlying issue.
        message: String,
        /// Span of the interpolation block.
        span: Span,
    },

    /// A literal value (e.g. integer with suffix mismatch) is malformed.
    #[error("invalid literal at byte {span:?}: {message}")]
    InvalidLiteral {
        /// What is wrong with the literal.
        message: String,
        /// Where the literal is in source.
        span: Span,
    },

    /// A `break expr` appeared outside a `loop` — only `loop` allows
    /// break-with-value.
    #[error("`break` with a value is only allowed inside `loop`, found at byte {span:?}")]
    BreakValueOutsideLoop {
        /// Span of the `break` keyword.
        span: Span,
    },

    /// Underlying lexer error encountered before parsing finished.
    #[error(transparent)]
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
            | Self::BreakValueOutsideLoop { span } => span.clone(),
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

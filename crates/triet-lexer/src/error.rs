//! Lexer error types.

use std::ops::Range;

use thiserror::Error;

/// Byte range in source: half-open `[start, end)`.
pub type Span = Range<usize>;

/// Lexical error encountered while tokenizing source.
#[derive(Clone, Debug, Default, Error, PartialEq, Eq)]
pub enum LexError {
    /// Placeholder used by `logos` when no rule matches; the [`crate::lex`]
    /// driver replaces this with a more informative variant.
    #[default]
    #[error("unrecognized token")]
    Unrecognized,

    /// Character or sequence does not match any token rule.
    #[error("unexpected character at byte {span:?}: {snippet:?}")]
    UnexpectedCharacter {
        /// Byte range where the unexpected character was found.
        span: Span,
        /// The offending source slice.
        snippet: String,
    },

    /// Numeric literal exceeds `i128` range during lexing.
    #[error("numeric literal at byte {span:?} overflows i128 — too large to lex")]
    NumericOverflow {
        /// Byte range of the overflowing literal.
        span: Span,
    },

    /// Ternary literal uses an unrecognized digit (must be `+`, `0`, `-`, `_`).
    #[error("invalid ternary digit {character:?} at byte {span:?}")]
    InvalidTernaryDigit {
        /// Byte range of the bad digit.
        span: Span,
        /// The offending character.
        character: char,
    },

    /// Block comment was opened but never closed.
    #[error("unterminated block comment starting at byte {span:?}")]
    UnterminatedBlockComment {
        /// Byte range of the comment opener.
        span: Span,
    },

    /// String literal was opened but never closed before EOF.
    #[error("unterminated string literal starting at byte {span:?}")]
    UnterminatedString {
        /// Byte range of the string opener.
        span: Span,
    },

    /// Escape sequence inside a string literal is not recognized.
    #[error("invalid escape sequence {sequence:?} at byte {span:?}")]
    InvalidEscape {
        /// Byte range of the escape sequence.
        span: Span,
        /// The offending escape sequence (including backslash).
        sequence: String,
    },

    /// A `}` appeared in an f-string body without a matching `{`. Use `}}`
    /// for a literal `}`.
    #[error("unexpected `}}` in f-string body at byte {span:?}; use `}}}}` for a literal `}}`")]
    UnmatchedFStringBrace {
        /// Byte range of the offending `}`.
        span: Span,
    },
}

impl LexError {
    /// Shift every span field by `offset`. Used by the [`crate::Lexer`]
    /// driver to convert errors emitted by an inner logos lexer (whose
    /// spans are relative to a source slice) into absolute spans.
    pub(crate) fn shift_span(mut self, offset: usize) -> Self {
        let shift = |span: &mut Span| {
            span.start += offset;
            span.end += offset;
        };
        match &mut self {
            Self::UnexpectedCharacter { span, .. }
            | Self::NumericOverflow { span }
            | Self::InvalidTernaryDigit { span, .. }
            | Self::UnterminatedBlockComment { span }
            | Self::UnterminatedString { span }
            | Self::InvalidEscape { span, .. }
            | Self::UnmatchedFStringBrace { span } => shift(span),
            Self::Unrecognized => {}
        }
        self
    }
}

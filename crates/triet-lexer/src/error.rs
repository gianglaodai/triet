//! Lexer error types.

use std::ops::Range;

use miette::Diagnostic;
use thiserror::Error;

/// Byte range in source: half-open `[start, end)`.
pub type Span = Range<usize>;

/// Lexical error encountered while tokenizing source.
#[derive(Clone, Debug, Default, Error, Diagnostic, PartialEq, Eq)]
pub enum LexError {
    /// Placeholder used by `logos` when no rule matches; the [`crate::lex`]
    /// driver replaces this with a more informative variant.
    #[default]
    #[error("unrecognized token")]
    #[diagnostic(code(triet::lex::E0000))]
    Unrecognized,

    /// Character or sequence does not match any token rule.
    #[error("unexpected character {snippet:?}")]
    #[diagnostic(code(triet::lex::E0001))]
    UnexpectedCharacter {
        /// Byte range where the unexpected character was found.
        #[label("unexpected character")]
        span: Span,
        /// The offending source slice.
        snippet: String,
    },

    /// Numeric literal exceeds `i128` range during lexing.
    #[error("numeric literal overflows i128 — too large to lex")]
    #[diagnostic(code(triet::lex::E0002))]
    NumericOverflow {
        /// Byte range of the overflowing literal.
        #[label("this number is too large")]
        span: Span,
    },

    /// Ternary literal uses an unrecognized digit (must be `+`, `0`, `-`, `_`).
    #[error("invalid ternary digit {character:?}")]
    #[diagnostic(
        code(triet::lex::E0003),
        help("balanced ternary literals only accept `+`, `0`, `-`, and `_`")
    )]
    InvalidTernaryDigit {
        /// Byte range of the bad digit.
        #[label("invalid digit")]
        span: Span,
        /// The offending character.
        character: char,
    },

    /// Block comment was opened but never closed.
    #[error("unterminated block comment")]
    #[diagnostic(code(triet::lex::E0004), help("add `*/` to close this block comment"))]
    UnterminatedBlockComment {
        /// Byte range of the comment opener.
        #[label("starts here")]
        span: Span,
    },

    /// String literal was opened but never closed before EOF.
    #[error("unterminated string literal")]
    #[diagnostic(
        code(triet::lex::E0005),
        help("add a closing `\"` to terminate the string")
    )]
    UnterminatedString {
        /// Byte range of the string opener.
        #[label("starts here")]
        span: Span,
    },

    /// Escape sequence inside a string literal is not recognized.
    #[error("invalid escape sequence {sequence:?}")]
    #[diagnostic(
        code(triet::lex::E0006),
        help("valid escape sequences: \\n, \\t, \\r, \\\\, \\\", \\u{{XXXX}}")
    )]
    InvalidEscape {
        /// Byte range of the escape sequence.
        #[label("invalid escape")]
        span: Span,
        /// The offending escape sequence (including backslash).
        sequence: String,
    },

    /// A `}` appeared in an f-string body without a matching `{`.
    #[error("unexpected `}}` in f-string body; use `}}}}` for a literal `}}`")]
    #[diagnostic(
        code(triet::lex::E0007),
        help("to write a literal `}}` in an f-string, escape it as `}}}}`")
    )]
    UnmatchedFStringBrace {
        /// Byte range of the offending `}`.
        #[label("unexpected `}}`")]
        span: Span,
    },
}

impl LexError {
    /// Shift every span field by `offset`.
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

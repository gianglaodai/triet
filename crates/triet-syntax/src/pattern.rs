//! Patterns used in `match` arms, `let` destructuring, and `for` loops.

use crate::{
    arena::PatternId,
    numeric::{NumericSuffix, TrileanValue},
};

/// A pattern matched against a value.
///
/// Recursive children (sub-patterns of tuples / or-patterns) are stored
/// as `PatternId` handles into the AST `Arena`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Concrete literal: `0`, `5_tryte`, `"hi"`, `true`.
    Literal(LiteralPattern),

    /// Variable binding — captures the matched value into `name`.
    Variable(String),

    /// Wildcard `_` — matches anything, binds nothing.
    Wildcard,

    /// Tuple destructuring: `(a, b, _)`.
    Tuple(Vec<PatternId>),

    /// Or-pattern: `1 | 2 | 3`. Matches if any sub-pattern matches.
    Or(Vec<PatternId>),

    /// Range pattern: `0..=9` (inclusive) or `0..9` (exclusive).
    Range {
        /// Lower bound (inclusive).
        start: LiteralPattern,
        /// Upper bound.
        end: LiteralPattern,
        /// Whether the upper bound is inclusive (`..=`) or exclusive (`..`).
        inclusive: bool,
    },

    /// `null` pattern — matches the null marker of a nullable type.
    Null,

    /// Enum variant pattern: `Some(x)`, `None`.
    EnumVariant {
        /// Enum type name (may be absent in simple contexts).
        name: Option<String>,
        /// Variant name.
        variant_name: String,
        /// Optional sub-pattern for the payload.
        payload: Option<PatternId>,
    },
}

/// Literal forms allowed inside patterns.
///
/// A subset of expression literals (no f-string, no `null` here — `null`
/// has its own pattern variant for clarity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiteralPattern {
    /// Decimal integer with optional suffix.
    Integer {
        /// Numeric value.
        value: i128,
        /// Optional `_trit`/`_tryte`/`_integer`/`_long` suffix.
        suffix: Option<NumericSuffix>,
    },
    /// Balanced ternary literal value (already converted from `0t...` form).
    Ternary(i128),
    /// Trilean literal: `true`, `false`, `unknown`.
    Trilean(TrileanValue),
    /// String literal.
    String(String),
}

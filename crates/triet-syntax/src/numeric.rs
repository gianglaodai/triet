//! Small enums shared across AST nodes for numeric and trilean literals.

/// Optional suffix attached to a decimal integer literal.
///
/// Mirrors `triet_lexer::NumericSuffix`. Defined separately so the syntax
/// crate has no compile-time dependency on the lexer; the parser converts
/// between the two enums when constructing AST nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericSuffix {
    /// `_trit`.
    Trit,
    /// `_tryte`.
    Tryte,
    /// `_integer` (redundant with the default but explicitly allowed).
    Integer,
    /// `_long`.
    Long,
}

/// One of the three trilean values: `True`, `False`, `Unknown`.
///
/// Mirrors `triet_logic::Trilean` but lives in syntax to avoid pulling in
/// the logic crate at AST level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrileanValue {
    /// `false` literal.
    False,
    /// `unknown` literal.
    Unknown,
    /// `true` literal.
    True,
}

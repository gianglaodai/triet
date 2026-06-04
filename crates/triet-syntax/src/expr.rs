//! Auxiliary expression-AST types not generated from the schema.
//!
//! The `Expr` enum and the binary/unary operator enums are schema-generated
//! (`crate::generated`). What remains here are the small helper types that the
//! generated `Expr` variants reference through `crate::expr::…` paths: outcome
//! arms, match arms, lambda params, and f-string segments.

use crate::arena::{ExprId, PatternId, TypeId};

/// Which arm of an outcome value is being constructed or matched.
/// Mirrors the three states of a balanced ternary discriminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutcomeArm {
    /// `~+` — `Trit::Positive` arm (success).
    Positive,
    /// `~0` — `Trit::Zero` arm (null state; `T?~E` only).
    Zero,
    /// `~-` — `Trit::Negative` arm (failure).
    Negative,
}

/// A single arm of a `match` expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm {
    /// Pattern matched.
    pub pattern: PatternId,
    /// Optional `if` guard (extra boolean condition).
    pub guard: Option<ExprId>,
    /// Expression evaluated when this arm matches.
    pub body: ExprId,
}

/// A parameter of a closure/lambda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LambdaParam {
    /// Parameter name.
    pub name: String,
    /// Optional explicit type — closures often elide this for inference.
    pub type_annotation: Option<TypeId>,
}

/// Parsed segments of an f-string body.
///
/// Decision A (see SPEC.md): f-strings are parsed at compile time so the
/// type checker can validate interpolated expressions and report errors
/// at their precise location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FStringSegments {
    /// Sequence of literal text and interpolated expressions.
    pub parts: Vec<FStringPart>,
}

/// One segment of an f-string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FStringPart {
    /// Plain text run (escapes already processed).
    Text(String),
    /// Interpolated expression `{expr}` or `{expr:format_spec}`.
    Interpolation {
        /// Expression to evaluate and stringify.
        expression: ExprId,
        /// Optional format spec after `:` — stored raw, parsed later.
        format_spec: Option<String>,
    },
}

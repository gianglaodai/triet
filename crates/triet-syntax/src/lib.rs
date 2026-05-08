//! Triết syntax: AST types and source spans.
//!
//! Defines the abstract syntax tree the parser produces and that the type
//! checker and interpreter consume. See SPEC.md §12 for the corresponding
//! grammar.
//!
//! # Module organization
//!
//! - [`span`] — `Span` and `Spanned<T>` for tracking source locations
//! - [`numeric`] — small enums shared across nodes (`NumericSuffix`,
//!   `TrileanValue`)
//! - [`type_ast`] — type expressions (annotations, generics)
//! - [`pattern`] — patterns for `match` arms and `let` destructuring
//! - [`expr`] — expressions and operator enums
//! - [`stmt`] — statements and blocks
//! - [`item`] — top-level items and the `Program` root

#![warn(missing_docs)]

pub mod expr;
pub mod item;
pub mod numeric;
pub mod pattern;
pub mod span;
pub mod stmt;
pub mod type_ast;

pub use expr::{
    BinaryOperator, Expr, FStringPart, FStringSegments, LambdaParam, MatchArm, UnaryOperator,
};
pub use item::{
    FunctionBody, FunctionDef, FunctionParam, ImportPath, Item, ParameterPassing, Program,
};
pub use numeric::{NumericSuffix, TrileanValue};
pub use pattern::{LiteralPattern, Pattern};
pub use span::{Span, Spanned};
pub use stmt::{Block, Stmt};
pub use type_ast::TypeExpr;

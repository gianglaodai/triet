//! Triết syntax: AST types, source spans, and arena allocation.
//!
//! Defines the abstract syntax tree the parser produces and that the type
//! checker and interpreter consume. See SPEC.md §12 for the corresponding
//! grammar.
//!
//! # Storage model — arena allocation
//!
//! Recursive AST nodes (`Expr`, `Pattern`, `TypeExpr`, `Stmt`) live in
//! typed sub-arenas inside an [`Arena`]. AST node fields refer to
//! children via small `*Id` handles instead of `Box<T>`, eliminating
//! visual noise from data definitions and keeping related nodes
//! contiguous in memory. This mirrors how `rustc`, the Swift compiler,
//! and Mojo's IR represent ASTs.
//!
//! A [`Program`] owns its `Arena`. The parser emits a fully-built
//! `Program`; downstream consumers (typechecker, interpreter) borrow it
//! immutably.
//!
//! # Module organization
//!
//! - [`span`] — `Span` and `Spanned<T>` for tracking source locations
//! - [`arena`] — `Arena` and the `*Id` handle types
//! - [`numeric`] — small enums shared across nodes (`NumericSuffix`,
//!   `TrileanValue`)
//! - [`type_ast`] — type expressions (annotations, generics)
//! - [`pattern`] — patterns for `match` arms and `let` destructuring
//! - [`expr`] — expressions and operator enums
//! - [`stmt`] — statements and blocks
//! - [`item`] — top-level items and the `Program` root

#![warn(missing_docs)]

pub mod arena;
pub mod expr;
pub mod item;
pub mod numeric;
pub mod pattern;
pub mod span;
pub mod stmt;
pub mod type_ast;
pub mod visibility;

pub use arena::{Arena, ExprId, PatternId, StmtId, TypeId};
pub use expr::{
    BinaryOperator, Expr, FStringPart, FStringSegments, LambdaParam, MatchArm, OutcomeArm,
    UnaryOperator,
};
pub use item::{
    EnumDef, EnumVariant, FunctionBody, FunctionDef, FunctionParam, GenericBound, ImportFrom,
    ImportName, ImportPath, Item, ModuleContent, ModuleDecl, ParameterPassing, Program, StructDef,
    StructField, TypeParam,
};
pub use numeric::{NumericSuffix, TrileanValue};
pub use pattern::{LiteralPattern, Pattern};
pub use span::{Span, Spanned};
pub use stmt::{Block, Stmt};
pub use type_ast::{ReferenceForm, TypeExpr};
pub use visibility::Visibility;

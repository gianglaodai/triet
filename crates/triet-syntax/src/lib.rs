//! Triết syntax.

#![warn(missing_docs)]

pub mod arena;
pub mod expr;
pub mod generated;
pub mod item;
pub mod numeric;
pub mod pattern;
pub mod span;
pub mod stmt;
pub mod type_ast;
pub mod visibility;

pub use arena::{Arena, ExprId, PatternId, StmtId, TypeId};
pub use expr::{FStringPart, FStringSegments, LambdaParam, MatchArm, OutcomeArm};
pub use item::{EnumVariant, GenericBound, ImportName, ImportPath, StructField, TypeParam};
pub use numeric::{NumericSuffix, TrileanValue};
pub use pattern::{LiteralPattern, Pattern};
pub use span::{Span, Spanned};
pub use type_ast::{ReferenceForm, TypeExpr};

// ── Generated types ──
pub use generated::{
    BinaryOperator, EnumDef, Expr, FunctionBody, FunctionDef, FunctionParam, Import, Item,
    ModuleContent, ModuleItem, ParameterPassing, Program, Stmt, StructDef, UnaryOperator,
    Visibility,
};

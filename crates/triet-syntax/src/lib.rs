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
pub use expr::{FStringPart, FStringSegments, LambdaParameter, MatchArm, OutcomeArm};
pub use item::{EnumVariant, GenericBound, ImportName, ImportPath, StructField, TypeParameter};
pub use numeric::{NumericSuffix, TrileanValue};
pub use pattern::{LiteralPattern, Pattern};
pub use span::{Span, Spanned};
pub use type_ast::{ReferenceForm, TypeExpr};

// ── Generated types ──
pub use generated::{
    BinaryOperator, EnumDefinition, Expr, FunctionBody, FunctionDefinition, FunctionParameter,
    Import, Item, ModuleContent, ModuleItem, ParameterPassing, Program, Stmt, StructDefinition,
    UnaryOperator, Visibility,
};

/// Resolution of an enum variant to its enum type and discriminant.
///
/// Produced by the type checker during name resolution and consumed by
/// the lowerer to emit correct MIR without re-scanning enum layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumVariantResolution {
    /// The enum type name (e.g. `"Color"`).
    pub enum_name: String,
    /// The variant name (e.g. `"Red"`).
    pub variant_name: String,
    /// The integer discriminant value (0, 1, 2, …).
    pub discriminant: i64,
    /// Whether this variant has a payload.
    pub has_payload: bool,
}

/// Maps expression IDs to resolved enum variants.
pub type ExprResolutions = std::collections::HashMap<ExprId, EnumVariantResolution>;

/// Maps pattern IDs to resolved enum variants.
pub type PatternResolutions = std::collections::HashMap<PatternId, EnumVariantResolution>;

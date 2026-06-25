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
    BinaryOperator, CapabilityLevel, EnumDefinition, Expr, FunctionBody, FunctionDefinition,
    FunctionParameter, ImplementationDefinition, Item, MethodSignature, ModuleContent, ModuleItem,
    ParameterPassing, Program, Stmt, StructDefinition, TraitDefinition, UnaryOperator, Visibility,
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

/// Resolution of a trait method call to its concrete implementation.
///
/// ADR-0061 T4 Tier 1 static dispatch. Produced by the type checker when
/// `a.compare(b)` resolves through the `impl_table`; consumed by the
/// lowerer (T5) to emit a direct `CallDispatch` to the mangled function.
///
/// Mirrors [`EnumVariantResolution`]: minimal payload, keyed by `ExprId`.
/// Only the mangled `concrete_fn` is carried — the return type already
/// lives on the target `Body`'s MIR signature, so re-storing it here
/// would be redundant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodResolution {
    /// Mangled concrete function name `Type$Trait$method` (ADR-0061 §2.4).
    pub concrete_fn: String,
}

/// Maps method-call expression IDs to their resolved concrete function.
pub type MethodResolutions = std::collections::HashMap<ExprId, MethodResolution>;

/// Mangle a trait method into its concrete function name `Type$Trait$method`.
///
/// ADR-0061 §2.4. **Single source of truth** — the type checker (building
/// the `impl_table` + annotating calls) and the lowerer (naming the emitted
/// `Body`) MUST both call this so the dispatch callee always matches an
/// emitted function. Do not inline the `$`-join anywhere else.
#[must_use]
pub fn mangle_trait_method(for_type: &str, trait_name: &str, method_name: &str) -> String {
    format!("{for_type}${trait_name}${method_name}")
}

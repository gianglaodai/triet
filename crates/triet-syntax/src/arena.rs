//! Arena allocation for AST nodes — `Box<T>`-free, compiler-style storage.
//!
//! Inspired by `rustc`, the Swift compiler, and Mojo's internal IR: every
//! recursive node lives in a typed sub-arena and is referenced by a
//! lightweight `*Id` handle. This eliminates `Box<T>` from the AST data
//! definitions and keeps related nodes contiguous in memory for cache-
//! friendly traversal.
//!
//! # Conceptual model
//!
//! Each AST module declares its node types with `*Id` references where
//! recursion would otherwise occur. The [`Arena`] holds the actual node
//! data; an `*Id` is a small handle (4 bytes) that the arena resolves
//! into a real `Spanned<T>` reference. Reference patterns are typed: a
//! `PatternId` cannot be used where an `ExprId` is expected.

use crate::{expr::Expr, pattern::Pattern, span::Spanned, stmt::Stmt, type_ast::TypeExpr};

macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        ///
        /// Construct via [`Arena`] allocation methods. The raw index is
        /// hidden to prevent fabricating handles that don't refer to a
        /// real arena entry.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl $name {
            /// Returns the raw arena index. Useful only for diagnostics
            /// and stable ordering — do not use to fabricate IDs.
            #[must_use]
            pub const fn as_u32(self) -> u32 {
                self.0
            }
        }
    };
}

define_id!(ExprId, "Handle to a `Spanned<Expr>` stored in an `Arena`.");
define_id!(
    PatternId,
    "Handle to a `Spanned<Pattern>` stored in an `Arena`."
);
define_id!(
    TypeId,
    "Handle to a `Spanned<TypeExpr>` stored in an `Arena`."
);
define_id!(StmtId, "Handle to a `Spanned<Stmt>` stored in an `Arena`.");

/// Storage for all recursive AST nodes belonging to a single `Program`.
///
/// The arena owns the actual `Spanned<T>` data; AST node fields hold only
/// `*Id` handles. To inspect a child, call the appropriate accessor
/// (e.g. [`Arena::expression`]) with the handle.
#[derive(Clone, Debug, Default)]
pub struct Arena {
    expressions: Vec<Spanned<Expr>>,
    patterns: Vec<Spanned<Pattern>>,
    types: Vec<Spanned<TypeExpr>>,
    statements: Vec<Spanned<Stmt>>,
}

impl Arena {
    /// Construct an empty arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            expressions: Vec::new(),
            patterns: Vec::new(),
            types: Vec::new(),
            statements: Vec::new(),
        }
    }

    /// Insert a spanned expression, returning its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` expressions are allocated. This is
    /// a hard limit per program; in practice no realistic source will
    /// approach it.
    pub fn alloc_expression(&mut self, node: Spanned<Expr>) -> ExprId {
        let index = u32::try_from(self.expressions.len())
            .expect("expression arena exceeded u32::MAX entries");
        self.expressions.push(node);
        ExprId(index)
    }

    /// Insert a spanned pattern, returning its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` patterns are allocated.
    pub fn alloc_pattern(&mut self, node: Spanned<Pattern>) -> PatternId {
        let index =
            u32::try_from(self.patterns.len()).expect("pattern arena exceeded u32::MAX entries");
        self.patterns.push(node);
        PatternId(index)
    }

    /// Insert a spanned type expression, returning its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` type expressions are allocated.
    pub fn alloc_type(&mut self, node: Spanned<TypeExpr>) -> TypeId {
        let index = u32::try_from(self.types.len()).expect("type arena exceeded u32::MAX entries");
        self.types.push(node);
        TypeId(index)
    }

    /// Insert a spanned statement, returning its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` statements are allocated.
    pub fn alloc_statement(&mut self, node: Spanned<Stmt>) -> StmtId {
        let index = u32::try_from(self.statements.len())
            .expect("statement arena exceeded u32::MAX entries");
        self.statements.push(node);
        StmtId(index)
    }

    /// Look up an expression by its handle.
    #[must_use]
    pub fn expression(&self, id: ExprId) -> &Spanned<Expr> {
        &self.expressions[id.0 as usize]
    }

    /// Look up a pattern by its handle.
    #[must_use]
    pub fn pattern(&self, id: PatternId) -> &Spanned<Pattern> {
        &self.patterns[id.0 as usize]
    }

    /// Look up a type expression by its handle.
    #[must_use]
    pub fn type_expression(&self, id: TypeId) -> &Spanned<TypeExpr> {
        &self.types[id.0 as usize]
    }

    /// Look up a statement by its handle.
    #[must_use]
    pub fn statement(&self, id: StmtId) -> &Spanned<Stmt> {
        &self.statements[id.0 as usize]
    }

    /// Total number of expression nodes stored.
    #[must_use]
    pub const fn expression_count(&self) -> usize {
        self.expressions.len()
    }

    /// Total number of pattern nodes stored.
    #[must_use]
    pub const fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// Total number of type expression nodes stored.
    #[must_use]
    pub const fn type_count(&self) -> usize {
        self.types.len()
    }

    /// Total number of statement nodes stored.
    #[must_use]
    pub const fn statement_count(&self) -> usize {
        self.statements.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{expr::Expr, numeric::TrileanValue, pattern::Pattern, type_ast::TypeExpr};

    #[test]
    fn new_arena_is_empty() {
        let arena = Arena::new();
        assert_eq!(arena.expression_count(), 0);
        assert_eq!(arena.pattern_count(), 0);
        assert_eq!(arena.type_count(), 0);
        assert_eq!(arena.statement_count(), 0);
    }

    #[test]
    fn alloc_returns_sequential_handles() {
        let mut arena = Arena::new();
        let first = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral {
                value: 1,
                suffix: None,
            },
            0..1,
        ));
        let second = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral {
                value: 2,
                suffix: None,
            },
            2..3,
        ));
        assert_eq!(first.as_u32(), 0);
        assert_eq!(second.as_u32(), 1);
    }

    #[test]
    fn alloc_round_trips_through_lookup() {
        let mut arena = Arena::new();
        let id =
            arena.alloc_expression(Spanned::new(Expr::TrileanLiteral(TrileanValue::True), 5..9));
        let stored = arena.expression(id);
        assert_eq!(stored.span, 5..9);
        assert!(matches!(
            stored.node,
            Expr::TrileanLiteral(TrileanValue::True),
        ));
    }

    #[test]
    fn typed_handles_prevent_kind_confusion_at_compile_time() {
        // This test exists primarily to document the safety invariant:
        // the `*Id` types are distinct, so the type system rejects
        // mixing them. Compile-time success of the alloc calls below
        // confirms each kind has its own arena slot.
        let mut arena = Arena::new();
        let _expr_id = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral {
                value: 0,
                suffix: None,
            },
            0..1,
        ));
        let _pattern_id = arena.alloc_pattern(Spanned::new(Pattern::Wildcard, 2..3));
        let _type_id = arena.alloc_type(Spanned::new(TypeExpr::Named("Integer".to_owned()), 4..11));
        // Counts should match what we allocated.
        assert_eq!(arena.expression_count(), 1);
        assert_eq!(arena.pattern_count(), 1);
        assert_eq!(arena.type_count(), 1);
    }

    #[test]
    fn handles_are_copy_and_cheaply_comparable() {
        let mut arena = Arena::new();
        let id = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral {
                value: 0,
                suffix: None,
            },
            0..1,
        ));
        let copied = id;
        assert_eq!(id, copied);
    }
}

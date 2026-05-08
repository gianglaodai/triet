//! Statements and blocks.

use crate::arena::{ExprId, PatternId, StmtId, TypeId};

/// A statement — a unit of execution that may not produce a value.
///
/// Note that many constructs (e.g. `if`, `match`) are *expressions* in
/// Triết and live in `Expr`; they appear as statements only via
/// [`Stmt::ExprStmt`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let name = value` or `let mut name: T = value`.
    Let {
        /// Variable name.
        name: String,
        /// Whether the binding is mutable (`let mut`).
        mutable: bool,
        /// Optional type annotation.
        type_annotation: Option<TypeId>,
        /// Initializer expression.
        value: ExprId,
    },

    /// `const NAME = value` or `const NAME: T = value`. Compile-time constant.
    Const {
        /// Constant name (uppercase by convention, not enforced syntactically).
        name: String,
        /// Optional type annotation.
        type_annotation: Option<TypeId>,
        /// Initializer (must be a constant expression — checked later).
        value: ExprId,
    },

    /// `return` or `return expr`.
    Return(Option<ExprId>),

    /// `break` (any loop) or `break expr` (only valid in `loop`).
    Break(Option<ExprId>),

    /// `continue`.
    Continue,

    /// `for pattern in iterable { ... }`.
    For {
        /// Loop variable (pattern allows tuple destructuring, e.g. `(idx, item)`).
        variable: PatternId,
        /// Iterable expression (range, collection, iterator).
        iterable: ExprId,
        /// Loop body.
        body: Block,
    },

    /// `while condition { ... }` or `while? condition { ... }`.
    While {
        /// Loop condition.
        condition: ExprId,
        /// Loop body.
        body: Block,
        /// `true` for `while?`, `false` for plain `while`.
        treat_unknown_as_false: bool,
    },

    /// `loop { ... }` — infinite loop, exits via `break expr`.
    Loop(Block),

    /// An expression used as a statement (its value is discarded).
    ExprStmt(ExprId),
}

/// A brace-delimited block: a list of statements with an optional trailing
/// expression that produces the block's value.
///
/// Both fields hold `*Id` handles; the actual `Spanned<Stmt>` and
/// `Spanned<Expr>` data lives in the AST `Arena`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// Statements executed in order.
    pub statements: Vec<StmtId>,
    /// Optional trailing expression — when present, the block evaluates
    /// to it. When absent, the block yields `Unit`.
    pub final_expression: Option<ExprId>,
}

impl Block {
    /// An empty block — useful for tests and synthesized branches.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            statements: Vec::new(),
            final_expression: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arena::Arena,
        expr::Expr,
        numeric::TrileanValue,
        span::Spanned,
    };

    #[test]
    fn empty_block_has_no_statements_or_value() {
        let block = Block::empty();
        assert!(block.statements.is_empty());
        assert!(block.final_expression.is_none());
    }

    #[test]
    fn let_statement_captures_mutability() {
        let mut arena = Arena::new();
        let value_immutable = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral { value: 5, suffix: None },
            8..9,
        ));
        let value_mutable = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral { value: 0, suffix: None },
            12..13,
        ));
        let immutable = Stmt::Let {
            name: "x".to_owned(),
            mutable: false,
            type_annotation: None,
            value: value_immutable,
        };
        let mutable = Stmt::Let {
            name: "y".to_owned(),
            mutable: true,
            type_annotation: None,
            value: value_mutable,
        };
        match (&immutable, &mutable) {
            (Stmt::Let { mutable: false, .. }, Stmt::Let { mutable: true, .. }) => {}
            _ => panic!("mutability flag did not roundtrip"),
        }
    }

    #[test]
    fn while_distinguishes_question_variant() {
        let mut arena = Arena::new();
        let true_cond = arena.alloc_expression(Spanned::new(
            Expr::TrileanLiteral(TrileanValue::True),
            6..10,
        ));
        let unknown_cond = arena.alloc_expression(Spanned::new(
            Expr::TrileanLiteral(TrileanValue::Unknown),
            7..14,
        ));
        let normal = Stmt::While {
            condition: true_cond,
            body: Block::empty(),
            treat_unknown_as_false: false,
        };
        let question = Stmt::While {
            condition: unknown_cond,
            body: Block::empty(),
            treat_unknown_as_false: true,
        };
        match (&normal, &question) {
            (
                Stmt::While { treat_unknown_as_false: false, .. },
                Stmt::While { treat_unknown_as_false: true, .. },
            ) => {}
            _ => panic!("while? flag did not differentiate"),
        }
    }
}

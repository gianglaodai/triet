//! Block helper type.
//!
//! The `Stmt` enum is schema-generated (`crate::generated::Stmt`). `Block`
//! remains here as a parser-side helper: the parser accumulates a block's
//! statements and trailing expression into a `Block` before allocating it as
//! an `Expr::Block` node. It is not part of the schema AST.

use crate::arena::{ExprId, StmtId};

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

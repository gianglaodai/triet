//! Top-level items appearing at module/file scope.

use crate::{
    expr::Expr,
    span::Spanned,
    stmt::Block,
    type_ast::TypeExpr,
};

/// A top-level item in a `.tt` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Function definition.
    Function(FunctionDef),

    /// Module-level constant: `const PI = 3`.
    Const {
        /// Constant name.
        name: String,
        /// Optional type annotation.
        type_annotation: Option<Spanned<TypeExpr>>,
        /// Initializer expression (must be constant — checked later).
        value: Spanned<Expr>,
    },

    /// Type alias: `type Username = String`.
    TypeAlias {
        /// Alias name (the new identifier).
        name: String,
        /// The type this alias resolves to.
        target: Spanned<TypeExpr>,
    },

    /// Module import: `import std.io`. Minimal v0.1 form — full module
    /// system is v0.2+.
    Import(ImportPath),
}

/// A function definition: `fn name(params) -> Return { body }` or with `=`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDef {
    /// Function name.
    pub name: String,
    /// Parameters in declaration order.
    pub parameters: Vec<FunctionParam>,
    /// Optional return type annotation. Required for block bodies; may
    /// be inferred for single-expression bodies.
    pub return_type: Option<Spanned<TypeExpr>>,
    /// Body — either a block or a single expression.
    pub body: FunctionBody,
}

/// A parameter declaration in a function signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionParam {
    /// Parameter name.
    pub name: String,
    /// Required type annotation (Triết does not infer parameter types).
    pub type_annotation: Spanned<TypeExpr>,
    /// How the caller's value reaches the function (Mojo-style).
    pub passing: ParameterPassing,
}

/// How a function parameter is passed (Mojo-aligned, see SPEC §10.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterPassing {
    /// Default — read-only borrow. No keyword in source.
    Borrowed,
    /// `mut` keyword — caller's value can be mutated.
    Mutable,
    /// `owned` keyword — ownership transfers into the function (rare).
    Owned,
}

/// A function body — either a brace-delimited block or a single expression.
///
/// Triết supports both `fn foo() -> T { stmt; expr }` and `fn foo() -> T = expr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionBody {
    /// `{ ... }` form.
    Block(Block),
    /// `= expr` form (single expression).
    Expression(Spanned<Expr>),
}

/// A dotted import path: `import std.io.println` → `["std", "io", "println"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportPath {
    /// Dot-separated segments, in order.
    pub segments: Vec<String>,
}

/// Root of the AST — a parsed `.tt` source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// Top-level items in source order.
    pub items: Vec<Spanned<Item>>,
}

impl Program {
    /// Construct an empty program.
    #[must_use]
    pub const fn empty() -> Self {
        Self { items: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize) -> crate::span::Span {
        start..end
    }

    #[test]
    fn empty_program_has_no_items() {
        let program = Program::empty();
        assert!(program.items.is_empty());
    }

    #[test]
    fn function_with_block_body() {
        let function = FunctionDef {
            name: "main".to_owned(),
            parameters: Vec::new(),
            return_type: None,
            body: FunctionBody::Block(Block::empty()),
        };
        assert_eq!(function.name, "main");
        assert!(matches!(function.body, FunctionBody::Block(_)));
    }

    #[test]
    fn function_with_expression_body() {
        let function = FunctionDef {
            name: "double".to_owned(),
            parameters: vec![FunctionParam {
                name: "n".to_owned(),
                type_annotation: Spanned::new(
                    TypeExpr::Named("Integer".to_owned()),
                    span(11, 18),
                ),
                passing: ParameterPassing::Borrowed,
            }],
            return_type: None,
            body: FunctionBody::Expression(Spanned::new(
                Expr::Identifier("n".to_owned()),
                span(22, 23),
            )),
        };
        assert!(matches!(function.body, FunctionBody::Expression(_)));
        assert_eq!(function.parameters.len(), 1);
    }

    #[test]
    fn parameter_passing_modes_are_distinct() {
        let modes = [
            ParameterPassing::Borrowed,
            ParameterPassing::Mutable,
            ParameterPassing::Owned,
        ];
        let unique = std::collections::HashSet::<_>::from_iter(modes);
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn import_path_preserves_segments() {
        let path = ImportPath {
            segments: vec!["std".to_owned(), "io".to_owned(), "println".to_owned()],
        };
        assert_eq!(path.segments.len(), 3);
        assert_eq!(path.segments[0], "std");
    }
}

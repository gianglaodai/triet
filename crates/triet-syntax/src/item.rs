//! Top-level items and the `Program` root.

use crate::{
    arena::{Arena, ExprId, TypeId},
    span::Spanned,
    stmt::Block,
};

/// A top-level item in a `.tri` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Function definition.
    Function(FunctionDef),

    /// Module-level constant: `const PI = 3`.
    Const {
        /// Constant name.
        name: String,
        /// Optional type annotation.
        type_annotation: Option<TypeId>,
        /// Initializer expression (must be constant — checked later).
        value: ExprId,
    },

    /// Type alias: `type Username = String`.
    TypeAlias {
        /// Alias name (the new identifier).
        name: String,
        /// The type this alias resolves to.
        target: TypeId,
    },

    /// Struct definition: `struct Point { x: Integer, y: Integer }`.
    Struct(StructDef),

    /// Enum definition: `enum Option { Some(Integer), None }`.
    Enum(EnumDef),

    /// Module import: `import std.io`. Minimal v0.1 form — full module
    /// system is v0.2+.
    Import(ImportPath),
}

/// A struct definition with named fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDef {
    /// Struct name.
    pub name: String,
    /// Fields in declaration order.
    pub fields: Vec<StructField>,
}

/// A single field in a struct definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructField {
    /// Field name.
    pub name: String,
    /// Field type annotation.
    pub type_annotation: TypeId,
}

/// An enum definition with named variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDef {
    /// Enum name.
    pub name: String,
    /// Variants in declaration order.
    pub variants: Vec<EnumVariant>,
}

/// A single variant in an enum definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumVariant {
    /// Variant name.
    pub name: String,
    /// Optional payload type. `None` = unit variant (`None`),
    /// `Some(TypeId)` = tuple variant (`Some(Integer)`).
    pub payload: Option<TypeId>,
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
    pub return_type: Option<TypeId>,
    /// Body — either a block or a single expression.
    pub body: FunctionBody,
}

/// A parameter declaration in a function signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionParam {
    /// Parameter name.
    pub name: String,
    /// Required type annotation (Triết does not infer parameter types).
    pub type_annotation: TypeId,
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
    Expression(ExprId),
}

/// A dotted import path: `import std.io.println` → `["std", "io", "println"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportPath {
    /// Dot-separated segments, in order.
    pub segments: Vec<String>,
}

/// Root of the AST — a parsed `.tri` source file.
///
/// A `Program` owns its `Arena` so all `*Id` handles in items remain
/// valid for the program's lifetime.
#[derive(Clone, Debug, Default)]
pub struct Program {
    /// Arena holding every recursive AST node referenced by `items`.
    pub arena: Arena,
    /// Top-level items in source order.
    pub items: Vec<Spanned<Item>>,
}

impl Program {
    /// Construct an empty program (no items, empty arena).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            arena: Arena::new(),
            items: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        expr::Expr,
        type_ast::TypeExpr,
    };

    #[test]
    fn empty_program_has_no_items() {
        let program = Program::empty();
        assert!(program.items.is_empty());
        assert_eq!(program.arena.expression_count(), 0);
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
        let mut arena = Arena::new();
        let integer_type = arena.alloc_type(Spanned::new(
            TypeExpr::Named("Integer".to_owned()),
            11..18,
        ));
        let body = arena.alloc_expression(Spanned::new(
            Expr::Identifier("n".to_owned()),
            22..23,
        ));
        let function = FunctionDef {
            name: "double".to_owned(),
            parameters: vec![FunctionParam {
                name: "n".to_owned(),
                type_annotation: integer_type,
                passing: ParameterPassing::Borrowed,
            }],
            return_type: None,
            body: FunctionBody::Expression(body),
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
        let unique: std::collections::HashSet<_> = modes.into_iter().collect();
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

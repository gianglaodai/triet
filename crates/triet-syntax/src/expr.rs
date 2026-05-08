//! Expressions — the part of the AST that has a value.

use crate::{
    arena::{ExprId, PatternId, TypeId},
    numeric::{NumericSuffix, TrileanValue},
    stmt::Block,
};

/// An expression — anything that evaluates to a value.
///
/// Recursive children are stored as `*Id` handles into the `Arena`
/// owning this AST. To traverse, look up the handle via the arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    // === Literals ===
    /// Decimal integer literal: `42`, `5_tryte`.
    IntegerLiteral {
        /// Numeric value.
        value: i128,
        /// Optional explicit type suffix.
        suffix: Option<NumericSuffix>,
    },
    /// Balanced ternary literal `0t+0-+`, already converted to its numeric value.
    TernaryLiteral {
        /// Numeric value (already decoded from balanced ternary digits).
        value: i128,
    },
    /// Trilean literal: `true`, `false`, `unknown`.
    TrileanLiteral(TrileanValue),
    /// Plain string literal: `"hello"`.
    StringLiteral(String),
    /// F-string with parsed interpolation segments (decision A).
    FStringLiteral(FStringSegments),
    /// `null` literal — only valid where a nullable type `T?` is expected.
    NullLiteral,

    // === Variables and access ===
    /// Identifier reference: `x`, `add`, `Integer`.
    Identifier(String),
    /// Field access: `point.x` (v0.2 with structs; AST is ready).
    FieldAccess {
        /// Object whose field is accessed.
        object: ExprId,
        /// Field name.
        field: String,
    },
    /// Tuple index: `pair.0`, `triple.2`.
    TupleIndex {
        /// Tuple expression.
        tuple: ExprId,
        /// Zero-based index.
        index: usize,
    },

    // === Calls ===
    /// Function call: `add(1, 2)`.
    Call {
        /// Function expression (often an `Identifier`).
        callee: ExprId,
        /// Positional arguments.
        arguments: Vec<ExprId>,
    },
    /// Method call with explicit receiver: `n.to_tryte()`.
    MethodCall {
        /// Receiver expression.
        receiver: ExprId,
        /// Method name.
        method: String,
        /// Positional arguments.
        arguments: Vec<ExprId>,
    },

    // === Operators ===
    /// Binary operator: `a + b`, `x and y`, `a => b`.
    BinaryOp {
        /// The operator.
        operator: BinaryOperator,
        /// Left operand.
        left: ExprId,
        /// Right operand.
        right: ExprId,
    },
    /// Unary operator: `-x`, `!flag`, `not cond`.
    UnaryOp {
        /// The operator.
        operator: UnaryOperator,
        /// Operand.
        operand: ExprId,
    },

    // === Nullable-specific operators (sugar over T?) ===
    /// Safe field access: `point?.x` — yields null if the object is null.
    SafeFieldAccess {
        /// Receiver, possibly null.
        object: ExprId,
        /// Field name.
        field: String,
    },
    /// Safe method call: `name?.to_upper()`.
    SafeMethodCall {
        /// Receiver, possibly null.
        receiver: ExprId,
        /// Method name.
        method: String,
        /// Positional arguments.
        arguments: Vec<ExprId>,
    },
    /// Elvis operator `?:` — replace null with default.
    ElvisOp {
        /// Possibly-null value.
        object: ExprId,
        /// Fallback value evaluated only if `object` is null.
        default: ExprId,
    },
    /// Force unwrap `!!` — panic if null.
    ForceUnwrap(ExprId),

    // === Control flow as expressions ===
    /// Conditional with `if` (and `if?` via `treat_unknown_as_false`).
    If {
        /// Condition expression. Must yield `Trilean`.
        condition: ExprId,
        /// Branch taken when condition is `True`.
        then_branch: Block,
        /// Optional `else` branch; when condition is `False` (or
        /// `Unknown` under `if?`), this branch runs. If absent, the
        /// expression yields `Unit`.
        else_branch: Option<Block>,
        /// `true` for `if?`, `false` for plain `if`.
        treat_unknown_as_false: bool,
    },
    /// Pattern matching expression.
    Match {
        /// Value being matched.
        scrutinee: ExprId,
        /// Match arms in order.
        arms: Vec<MatchArm>,
    },
    /// Block as expression: `{ stmt; stmt; expr }`. Value is final expr or `Unit`.
    Block(Block),

    // === Composite ===
    /// Tuple literal: `(1, true)`, `(a, b, c)`.
    Tuple(Vec<ExprId>),
    /// Closure / lambda: `|x| x + 1`, `|x: Integer| -> Integer { ... }`.
    Lambda {
        /// Closure parameters.
        parameters: Vec<LambdaParam>,
        /// Optional return type annotation.
        return_type: Option<TypeId>,
        /// Body expression.
        body: ExprId,
    },
    /// Range expression: `0..100` (exclusive), `0..=100` (inclusive).
    Range {
        /// Lower bound.
        start: ExprId,
        /// Upper bound.
        end: ExprId,
        /// Whether `..=` (true) or `..` (false).
        inclusive: bool,
    },
}

/// A single arm of a `match` expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm {
    /// Pattern matched.
    pub pattern: PatternId,
    /// Optional `if` guard (extra boolean condition).
    pub guard: Option<ExprId>,
    /// Expression evaluated when this arm matches.
    pub body: ExprId,
}

/// A parameter of a closure/lambda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LambdaParam {
    /// Parameter name.
    pub name: String,
    /// Optional explicit type — closures often elide this for inference.
    pub type_annotation: Option<TypeId>,
}

/// Parsed segments of an f-string body.
///
/// Decision A (see SPEC.md): f-strings are parsed at compile time so the
/// type checker can validate interpolated expressions and report errors
/// at their precise location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FStringSegments {
    /// Sequence of literal text and interpolated expressions.
    pub parts: Vec<FStringPart>,
}

/// One segment of an f-string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FStringPart {
    /// Plain text run (escapes already processed).
    Text(String),
    /// Interpolated expression `{expr}` or `{expr:format_spec}`.
    Interpolation {
        /// Expression to evaluate and stringify.
        expression: ExprId,
        /// Optional format spec after `:` — stored raw, parsed later.
        format_spec: Option<String>,
    },
}

/// Binary (two-operand) operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    // Arithmetic
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `%%`
    Modulo,

    // Comparison — return Trilean (never produces Unknown for `==`/`!=`)
    /// `==`
    Equal,
    /// `!=`
    NotEqual,
    /// `<`
    LessThan,
    /// `<=`
    LessEqual,
    /// `>`
    GreaterThan,
    /// `>=`
    GreaterEqual,

    // Universal logic (identical in Ł3 and K3)
    /// `&&` or `and`
    And,
    /// `||` or `or`
    Or,

    // Łukasiewicz Ł3 (default)
    /// `^` or `xor`
    Xor,
    /// `<=>` or `iff`
    Iff,
    /// `=>` or `implies`
    Implies,

    // Kleene K3 variants
    /// `~^` or `kleene_xor`
    KleeneXor,
    /// `<~>` or `kleene_iff`
    KleeneIff,
    /// `~>` or `kleene_implies`
    KleeneImplies,
}

/// Unary (single-operand) operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    /// Numeric negation `-x` or trilean negation when operand is `Trilean`.
    Negate,
    /// Logical NOT: `!x` or `not x`.
    Not,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arena::Arena,
        pattern::Pattern,
        span::{Span, Spanned},
    };

    fn span(start: usize, end: usize) -> Span {
        start..end
    }

    fn lit_int(arena: &mut Arena, value: i128, range: Span) -> ExprId {
        arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral { value, suffix: None },
            range,
        ))
    }

    #[test]
    fn binary_op_holds_two_operands() {
        let mut arena = Arena::new();
        let left = lit_int(&mut arena, 1, span(0, 1));
        let right = lit_int(&mut arena, 2, span(4, 5));
        let plus = arena.alloc_expression(Spanned::new(
            Expr::BinaryOp {
                operator: BinaryOperator::Add,
                left,
                right,
            },
            span(0, 5),
        ));
        if let Expr::BinaryOp {
            operator,
            left: lid,
            right: rid,
        } = &arena.expression(plus).node
        {
            assert_eq!(*operator, BinaryOperator::Add);
            assert_eq!(*lid, left);
            assert_eq!(*rid, right);
        } else {
            panic!("expected BinaryOp");
        }
    }

    #[test]
    fn if_expr_distinguishes_normal_from_question_variant() {
        let mut arena = Arena::new();
        let true_cond = arena.alloc_expression(Spanned::new(
            Expr::TrileanLiteral(TrileanValue::True),
            span(3, 7),
        ));
        let unknown_cond = arena.alloc_expression(Spanned::new(
            Expr::TrileanLiteral(TrileanValue::Unknown),
            span(4, 11),
        ));
        let normal = Expr::If {
            condition: true_cond,
            then_branch: Block::empty(),
            else_branch: None,
            treat_unknown_as_false: false,
        };
        let question = Expr::If {
            condition: unknown_cond,
            then_branch: Block::empty(),
            else_branch: None,
            treat_unknown_as_false: true,
        };
        match (&normal, &question) {
            (
                Expr::If { treat_unknown_as_false: false, .. },
                Expr::If { treat_unknown_as_false: true, .. },
            ) => {}
            _ => panic!("flag did not differentiate variants"),
        }
    }

    #[test]
    fn fstring_segments_can_mix_text_and_interpolation() {
        let mut arena = Arena::new();
        let name = arena.alloc_expression(Spanned::new(
            Expr::Identifier("name".to_owned()),
            span(9, 13),
        ));
        let segments = FStringSegments {
            parts: vec![
                FStringPart::Text("hello, ".to_owned()),
                FStringPart::Interpolation {
                    expression: name,
                    format_spec: None,
                },
                FStringPart::Text("!".to_owned()),
            ],
        };
        assert_eq!(segments.parts.len(), 3);
    }

    #[test]
    fn match_arm_guard_is_optional() {
        let mut arena = Arena::new();
        let true_body = arena.alloc_expression(Spanned::new(
            Expr::TrileanLiteral(TrileanValue::True),
            span(5, 9),
        ));
        let wildcard = arena.alloc_pattern(Spanned::new(Pattern::Wildcard, span(0, 1)));
        let arm_no_guard = MatchArm {
            pattern: wildcard,
            guard: None,
            body: true_body,
        };
        let var_pat = arena.alloc_pattern(Spanned::new(
            Pattern::Variable("x".to_owned()),
            span(0, 1),
        ));
        let guard_expr = arena.alloc_expression(Spanned::new(
            Expr::Identifier("ok".to_owned()),
            span(8, 10),
        ));
        let arm_with_guard = MatchArm {
            pattern: var_pat,
            guard: Some(guard_expr),
            body: true_body,
        };
        assert!(arm_no_guard.guard.is_none());
        assert!(arm_with_guard.guard.is_some());
    }

    #[test]
    fn unary_and_binary_operator_enums_are_distinct() {
        // Smoke test that both enums coexist and exhaustive matching works.
        let _ = BinaryOperator::Implies;
        let _ = UnaryOperator::Not;
    }

    #[test]
    fn integer_literal_construction() {
        let mut arena = Arena::new();
        let id = arena.alloc_expression(Spanned::new(
            Expr::IntegerLiteral { value: 42, suffix: Some(NumericSuffix::Tryte) },
            span(0, 8),
        ));
        if let Expr::IntegerLiteral { value, suffix } = &arena.expression(id).node {
            assert_eq!(*value, 42);
            assert_eq!(*suffix, Some(NumericSuffix::Tryte));
        } else {
            panic!();
        }
    }
}

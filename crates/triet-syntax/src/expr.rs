//! Expressions — the part of the AST that has a value.

use crate::{
    numeric::{NumericSuffix, TrileanValue},
    pattern::Pattern,
    span::Spanned,
    stmt::Block,
    type_ast::TypeExpr,
};

/// An expression — anything that evaluates to a value.
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
        object: Box<Spanned<Self>>,
        /// Field name.
        field: String,
    },
    /// Tuple index: `pair.0`, `triple.2`.
    TupleIndex {
        /// Tuple expression.
        tuple: Box<Spanned<Self>>,
        /// Zero-based index.
        index: usize,
    },

    // === Calls ===
    /// Function call: `add(1, 2)`.
    Call {
        /// Function expression (often an `Identifier`).
        callee: Box<Spanned<Self>>,
        /// Positional arguments.
        arguments: Vec<Spanned<Self>>,
    },
    /// Method call with explicit receiver: `n.to_tryte()`.
    MethodCall {
        /// Receiver expression.
        receiver: Box<Spanned<Self>>,
        /// Method name.
        method: String,
        /// Positional arguments.
        arguments: Vec<Spanned<Self>>,
    },

    // === Operators ===
    /// Binary operator: `a + b`, `x and y`, `a => b`.
    BinaryOp {
        /// The operator.
        operator: BinaryOperator,
        /// Left operand.
        left: Box<Spanned<Self>>,
        /// Right operand.
        right: Box<Spanned<Self>>,
    },
    /// Unary operator: `-x`, `!flag`, `not cond`.
    UnaryOp {
        /// The operator.
        operator: UnaryOperator,
        /// Operand.
        operand: Box<Spanned<Self>>,
    },

    // === Nullable-specific operators (sugar over T?) ===
    /// Safe call: `name?.length` — yields null if the receiver is null.
    SafeFieldAccess {
        /// Receiver, possibly null.
        object: Box<Spanned<Self>>,
        /// Field name.
        field: String,
    },
    /// Safe method call: `name?.to_upper()`.
    SafeMethodCall {
        /// Receiver, possibly null.
        receiver: Box<Spanned<Self>>,
        /// Method name.
        method: String,
        /// Positional arguments.
        arguments: Vec<Spanned<Self>>,
    },
    /// Elvis operator `?:` — replace null with default.
    ElvisOp {
        /// Possibly-null value.
        object: Box<Spanned<Self>>,
        /// Fallback value evaluated only if `object` is null.
        default: Box<Spanned<Self>>,
    },
    /// Force unwrap `!!` — panic if null.
    ForceUnwrap(Box<Spanned<Self>>),

    // === Control flow as expressions ===
    /// Conditional with `if` (and `if?` via `treat_unknown_as_false`).
    If {
        /// Condition expression. Must yield `Trilean`.
        condition: Box<Spanned<Self>>,
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
        scrutinee: Box<Spanned<Self>>,
        /// Match arms in order.
        arms: Vec<MatchArm>,
    },
    /// Block as expression: `{ stmt; stmt; expr }`. Value is final expr or `Unit`.
    Block(Block),

    // === Composite ===
    /// Tuple literal: `(1, true)`, `(a, b, c)`.
    Tuple(Vec<Spanned<Self>>),
    /// Closure / lambda: `|x| x + 1`, `|x: Integer| -> Integer { ... }`.
    Lambda {
        /// Closure parameters.
        parameters: Vec<LambdaParam>,
        /// Optional return type annotation.
        return_type: Option<Spanned<TypeExpr>>,
        /// Body expression.
        body: Box<Spanned<Self>>,
    },
    /// Range expression: `0..100` (exclusive), `0..=100` (inclusive).
    Range {
        /// Lower bound.
        start: Box<Spanned<Self>>,
        /// Upper bound.
        end: Box<Spanned<Self>>,
        /// Whether `..=` (true) or `..` (false).
        inclusive: bool,
    },
}

/// A single arm of a `match` expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm {
    /// Pattern matched.
    pub pattern: Spanned<Pattern>,
    /// Optional `if` guard (extra boolean condition).
    pub guard: Option<Spanned<Expr>>,
    /// Expression evaluated when this arm matches.
    pub body: Spanned<Expr>,
}

/// A parameter of a closure/lambda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LambdaParam {
    /// Parameter name.
    pub name: String,
    /// Optional explicit type — closures often elide this for inference.
    pub type_annotation: Option<Spanned<TypeExpr>>,
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
        expression: Spanned<Expr>,
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

    fn span(start: usize, end: usize) -> crate::span::Span {
        start..end
    }

    #[test]
    fn integer_literal_construction() {
        let expr = Expr::IntegerLiteral { value: 42, suffix: None };
        if let Expr::IntegerLiteral { value, suffix } = expr {
            assert_eq!(value, 42);
            assert!(suffix.is_none());
        } else {
            panic!();
        }
    }

    #[test]
    fn binary_op_holds_two_operands() {
        let left = Box::new(Spanned::new(
            Expr::IntegerLiteral { value: 1, suffix: None },
            span(0, 1),
        ));
        let right = Box::new(Spanned::new(
            Expr::IntegerLiteral { value: 2, suffix: None },
            span(4, 5),
        ));
        let expr = Expr::BinaryOp {
            operator: BinaryOperator::Add,
            left,
            right,
        };
        if let Expr::BinaryOp { operator, .. } = expr {
            assert_eq!(operator, BinaryOperator::Add);
        } else {
            panic!();
        }
    }

    #[test]
    fn if_expr_distinguishes_normal_from_question_variant() {
        let normal = Expr::If {
            condition: Box::new(Spanned::new(
                Expr::TrileanLiteral(TrileanValue::True),
                span(3, 7),
            )),
            then_branch: Block::empty(),
            else_branch: None,
            treat_unknown_as_false: false,
        };
        let question = Expr::If {
            condition: Box::new(Spanned::new(
                Expr::TrileanLiteral(TrileanValue::Unknown),
                span(4, 11),
            )),
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
        let segments = FStringSegments {
            parts: vec![
                FStringPart::Text("hello, ".to_owned()),
                FStringPart::Interpolation {
                    expression: Spanned::new(
                        Expr::Identifier("name".to_owned()),
                        span(9, 13),
                    ),
                    format_spec: None,
                },
                FStringPart::Text("!".to_owned()),
            ],
        };
        assert_eq!(segments.parts.len(), 3);
    }

    #[test]
    fn unary_and_binary_operator_enums_are_distinct() {
        // Smoke test that both enums coexist and exhaustive matching works.
        let _ = BinaryOperator::Implies;
        let _ = UnaryOperator::Not;
    }

    #[test]
    fn match_arm_guard_is_optional() {
        let arm_no_guard = MatchArm {
            pattern: Spanned::new(Pattern::Wildcard, span(0, 1)),
            guard: None,
            body: Spanned::new(Expr::TrileanLiteral(TrileanValue::True), span(5, 9)),
        };
        let arm_with_guard = MatchArm {
            pattern: Spanned::new(Pattern::Variable("x".to_owned()), span(0, 1)),
            guard: Some(Spanned::new(
                Expr::Identifier("ok".to_owned()),
                span(8, 10),
            )),
            body: Spanned::new(Expr::TrileanLiteral(TrileanValue::True), span(14, 18)),
        };
        assert!(arm_no_guard.guard.is_none());
        assert!(arm_with_guard.guard.is_some());
    }

}

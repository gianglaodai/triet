//! Expression type inference.
//!
//! Implements `Checker::infer_expression` and every per-expression
//! sub-check (binary/unary, calls, methods, field/index, nullable
//! operators, `if`, `match`, lambdas, ranges).
//!
//! The dispatcher `infer_expression` is `pub(super)` so the rest of
//! `check` can call it; helper sub-checks stay private to this file.

use triet_syntax::{
    BinaryOperator, Expr, ExprId, FStringPart, MatchArm, NumericSuffix, ReferenceForm, Span,
    UnaryOperator,
};

use crate::{error::TypeError, types::Type};

use super::Checker;
use super::MoveState;
use super::methods::builtin_method_type;

impl Checker<'_> {
    /// Infer the type of an expression. Records errors for malformed
    /// shapes; returns `Type::Unknown` as a recovery placeholder so a
    /// single mismatch doesn't trigger a cascade.
    #[allow(clippy::too_many_lines)]
    pub(super) fn infer_expression(&mut self, id: ExprId) -> Type {
        let span = self.arena.expression(id).span.clone();
        let node = self.arena.expression(id).node.clone();

        match node {
            Expr::IntegerLiteral { value, suffix } => {
                // ADR-0044 Q2: range-check Integer literal (E1036).
                let is_integer_ty = matches!(suffix, Some(NumericSuffix::Integer) | None);
                if is_integer_ty {
                    let max_i128 = triet_core::Integer::MAX.to_i128();
                    if value > max_i128 || value < -max_i128 {
                        // Truncation is safe: value already passed i128 range check.
                        #[allow(clippy::cast_possible_truncation)]
                        self.errors.push(TypeError::IntegerLiteralOverflow {
                            value: value as i64,
                            max: triet_core::Integer::MAX.to_i64(),
                            span: span.clone(),
                        });
                    }
                }
                match suffix {
                    Some(NumericSuffix::Trit) => Type::Trit,
                    Some(NumericSuffix::Tryte) => Type::Tryte,
                    Some(NumericSuffix::Long) => Type::Long,
                    Some(NumericSuffix::Integer) | None => Type::Integer,
                }
            }
            Expr::TernaryLiteral { value } => {
                let max_i128 = triet_core::Integer::MAX.to_i128();
                if value > max_i128 || value < -max_i128 {
                    #[allow(clippy::cast_possible_truncation)]
                    self.errors.push(TypeError::IntegerLiteralOverflow {
                        value: value as i64,
                        max: triet_core::Integer::MAX.to_i64(),
                        span: span.clone(),
                    });
                }
                Type::Integer
            }
            Expr::TritLiteral { .. } => Type::Trit,
            // Control-flow-as-expression forms in the schema AST. The
            // parser currently lowers `while` / `return` to statements
            // (`Stmt::While` / `Stmt::Return`), so these arms are
            // dormant; handle them minimally for exhaustiveness.
            Expr::While { condition, body } => {
                let _ = self.infer_expression(condition);
                let _ = self.infer_expression(body);
                Type::Unit
            }
            Expr::Return { value } => {
                if let Some(value) = value {
                    let _ = self.infer_expression(value);
                }
                Type::Unit
            }
            // ADR-0021 §2.1: `true` / `false` literals are Trilean!
            // (statically proven non-Unknown). Only `unknown` literal
            // produces generic Trilean.
            Expr::TrileanLiteral { value } => match value {
                triet_syntax::numeric::TrileanValue::Unknown => Type::TRILEAN,
                triet_syntax::numeric::TrileanValue::True
                | triet_syntax::numeric::TrileanValue::False => Type::TRILEAN_KNOWN,
            },
            Expr::StringLiteral { .. } => Type::String,
            Expr::FStringLiteral { segments } => {
                for part in &segments {
                    if let FStringPart::Interpolation { expression, .. } = part {
                        // The expression must be type-checkable; its type
                        // is converted via Display by the runtime.
                        let _ = self.infer_expression(*expression);
                    }
                }
                Type::String
            }
            Expr::NullLiteral => {
                // v0.7.4.3-error.2 (ADR-0020 §10.3): `null` keyword is
                // deprecated. Emit W2001 warning with fix-hint pointing
                // to `~0` canonical literal. Keeps inferring as
                // `Nullable(Unknown)` for backwards-compat through v1.0.
                self.errors.push(TypeError::NullDeprecated { span });
                Type::Nullable(Box::new(Type::Unknown))
            }
            Expr::Identifier { name } => {
                // v0.9.x.atomic.7d: E2420 UseAfterMove check per
                // ADR-0025 §5.1 + ADR-0031 §4. Fires only on moved
                // local bindings (params + lets); ignored for
                // functions, types, imports (not movable values).
                self.check_used(&name, &span);
                // Try variable/function binding first, then overloads, then enum variant.
                if let Some(ty) = self.env.lookup(&name).cloned() {
                    ty
                } else if let Some(candidates) = self.env.lookup_all(&name) {
                    // Return the first overload candidate. This is a
                    // deliberate Bậc A choice — overloaded functions
                    // (like `len`) are never used as values; the only
                    // context that reaches here is an errant non-call
                    // use. Actual overload resolution happens in
                    // `check_call` where argument types are visible.
                    candidates.into_iter().next().unwrap_or(Type::Unknown)
                } else if let Some(resolution) = self.resolve_enum_variant(&name, &span) {
                    // Record the resolution so the lowerer doesn't
                    // need to re-scan enum layouts by string match.
                    let enum_name = resolution.enum_name.clone();
                    self.expr_resolutions.insert(id, resolution);
                    // Look up the enum type in the environment.
                    self.env
                        .lookup(&enum_name)
                        .cloned()
                        .unwrap_or(Type::Unknown)
                } else {
                    self.errors.push(TypeError::UndefinedName { name, span });
                    Type::Unknown
                }
            }
            Expr::BinaryOp {
                operator,
                left,
                right,
            } => self.check_binary_op(operator, left, right, span),
            Expr::UnaryOp {
                // The parser collapses `-` / `!` / `not` / `~!` to a single
                // unary form; `check_unary_negate` dispatches on the operand
                // type (arithmetic on Integer, logical on Trilean). The
                // schema's separate `Not` / `KleeneNot` variants are dormant
                // until the parser distinguishes them.
                operator: UnaryOperator::Negate | UnaryOperator::Not | UnaryOperator::KleeneNot,
                operand,
            } => self.check_unary_negate(operand, span),
            // v0.9.x.atomic.7b: borrow expression typecheck per ADR-0031
            // §4. Infer the operand's type T, wrap into
            // `Type::Reference(form, T)`. Borrow checker enforcement
            // (consume-once E2420 fires .7d; NLL + lifetime elision
            // defer v0.10 per §10.1 backlog).
            Expr::Borrow { form, operand } => self.check_borrow(form, operand, span),
            Expr::Call { callee, arguments } => {
                // Record enum variant constructor resolution before
                // delegating to check_call. The lowerer needs the
                // call expression's ID → resolution mapping.
                //
                // Two syntaxes are supported:
                //   bare:    `SomeInt(5)`  — callee is Identifier
                //   qualified: `CD.SomeInt(5)` — callee is FieldAccess
                match &self.arena.expression(callee).node {
                    Expr::Identifier { name: variant_name } => {
                        if let Some(resolution) = self.resolve_enum_variant(variant_name, &span)
                            && resolution.has_payload
                        {
                            self.expr_resolutions.insert(id, resolution);
                        }
                    }
                    Expr::FieldAccess { object, field } => {
                        if let Expr::Identifier { name: enum_name } =
                            &self.arena.expression(*object).node
                            && let Some(enum_ty) = self.env.lookup(enum_name).cloned()
                            && let Type::UserEnum { variants, .. } = &enum_ty
                            && let Some(variant_idx) = variants.iter().position(|(n, _)| n == field)
                        {
                            let (_, payload) = &variants[variant_idx];
                            self.expr_resolutions.insert(
                                id,
                                triet_syntax::EnumVariantResolution {
                                    enum_name: enum_name.clone(),
                                    variant_name: field.clone(),
                                    discriminant: variant_idx as i64,
                                    has_payload: payload.is_some(),
                                },
                            );
                        }
                    }
                    _ => {}
                }
                self.check_call(callee, &arguments, span)
            }
            Expr::MethodCall {
                receiver,
                method,
                arguments,
            } => {
                // Detect qualified enum variant construction:
                // `OptionA.SomeInt(42)` parses as MethodCall.
                if let Expr::Identifier { name: enum_name } = &self.arena.expression(receiver).node
                    && let Some(Type::UserEnum { variants, .. }) =
                        self.env.lookup(enum_name).cloned()
                    && let Some(variant_idx) = variants.iter().position(|(n, _)| n == &method)
                {
                    let (_, payload) = &variants[variant_idx];
                    self.expr_resolutions.insert(
                        id,
                        triet_syntax::EnumVariantResolution {
                            enum_name: enum_name.clone(),
                            variant_name: method.clone(),
                            discriminant: variant_idx as i64,
                            has_payload: payload.is_some(),
                        },
                    );
                }
                self.check_method_call(receiver, &method, &arguments, span)
            }
            Expr::FieldAccess { object, field } => {
                self.check_field_access(id, object, &field, span)
            }
            Expr::TupleIndex { tuple, index } => self.check_tuple_index(tuple, index, span),
            Expr::SafeFieldAccess { object, field } => {
                self.check_safe_field_access(object, &field, span)
            }
            Expr::SafeMethodCall {
                receiver,
                method,
                arguments,
            } => self.check_safe_method_call(receiver, &method, &arguments, span),
            Expr::ElvisOp { object, default } => self.check_elvis(object, default, span),
            Expr::ForceUnwrap { operand: inner } => self.check_force_unwrap(inner, span),
            Expr::If {
                condition,
                then_branch,
                else_branch,
                treat_unknown_as_false,
            } => self.check_if(
                condition,
                then_branch,
                else_branch,
                treat_unknown_as_false,
                span,
            ),
            Expr::Match { scrutinee, arms } => self.check_match(scrutinee, &arms, span),
            Expr::Block {
                statements,
                final_expr,
            } => self.check_block(&statements, final_expr),
            Expr::Tuple { elements } => {
                let types = elements.iter().map(|e| self.infer_expression(*e)).collect();
                Type::Tuple(types)
            }
            Expr::Lambda {
                parameters,
                return_type_annotation,
                body,
            } => {
                let param_types: Vec<Type> = parameters
                    .iter()
                    .map(|p| match p.type_annotation {
                        Some(id) => self.resolve_type(id),
                        None => Type::Unknown,
                    })
                    .collect();
                self.env.push_frame();
                for (param, ty) in parameters.iter().zip(param_types.iter()) {
                    self.env.declare(&param.name, ty.clone());
                }
                let body_ty = self.infer_expression(body);
                let declared_return = return_type_annotation.map(|id| self.resolve_type(id));
                let return_ty = declared_return.unwrap_or_else(|| body_ty.clone());
                self.env.pop_frame();
                Type::Function {
                    // Closure literals don't introduce generic type
                    // parameters at v0.7.4.1 (Q2-A minimal scope —
                    // only function declarations get type params).
                    type_params: Vec::new(),
                    parameters: param_types,
                    return_type: Box::new(return_ty),
                }
            }
            Expr::Range { start, end, .. } => {
                let start_ty = self.infer_expression(start);
                let end_ty = self.infer_expression(end);
                if !start_ty.matches(&end_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: start_ty.clone(),
                        found: end_ty,
                        span: self.arena.expression(end).span.clone(),
                    });
                }
                Type::Range(Box::new(start_ty))
            }
            Expr::StructLiteral {
                struct_name,
                fields,
            } => self.check_struct_literal(&struct_name, &fields, span),
            Expr::EnumLiteral {
                name,
                variant_name,
                payload,
            } => self.check_enum_literal(&name, &variant_name, payload.as_ref(), span),

            // v0.7.4.3-error.2 (ADR-0020 §2-§3): outcome expressions.
            // Constructor: infer payload, defer type construction to
            // context (caller's return type or `let` annotation drives
            // the value_type / error_type / allow_null_state choice).
            // Constructors alone are AMBIGUOUS without context — type
            // inference happens at the matches() check against the
            // expected type, which we cannot know here. Return Unknown
            // and let context-sensitive checks (assignment, return,
            // function-call arg) catch shape mismatches.
            Expr::OutcomeConstructor { arm, payload } => {
                if let Some(inner) = payload {
                    self.infer_expression(inner);
                }
                self.check_outcome_constructor_context(arm, span)
            }
            Expr::OutcomeArmHandler {
                inner,
                arm,
                capture_name,
                body,
            } => self.check_outcome_arm_handler(inner, arm, capture_name.as_deref(), body, span),
            Expr::OutcomePropagate {
                expr,
                capture_var,
                early_return,
            } => {
                // `capture_var` is `String` ("" means `|_|` discard).
                let capture = if capture_var.is_empty() {
                    None
                } else {
                    Some(capture_var.as_str())
                };
                self.check_outcome_propagate(expr, capture, early_return, span)
            }
            Expr::OutcomeDefault {
                expr,
                default_value,
            } => self.check_outcome_default(expr, default_value),
        }
    }

    /// Check an outcome constructor (`~+ value` / `~0` / `~- error`)
    /// against the active context. Context resolution order
    /// (v0.7.4.3-debt.3 / WA-5):
    ///
    /// 1. **`expected_type_stack.last()`** — local site context from a
    ///    `let x: T = …` annotation, struct field position, or call
    ///    argument position. Tightest scope, consulted first.
    /// 2. **`current_return_type`** — surrounding function's return
    ///    type. Only consulted if the local stack is empty.
    /// 3. **None** — return `Type::Unknown` (downstream `.matches()`
    ///    catches the shape mismatch).
    ///
    /// For v0.7.4.3-error.2 the constructor returns:
    ///
    /// - `~+ payload`: `Type::Outcome { value=typeof(payload), error=Unknown, allow_null=?}`
    /// - `~- payload`: `Type::Outcome { value=Unknown, error=typeof(payload), allow_null=? }`
    /// - `~0`: validated against the local + return context; widens
    ///   to `Nullable<T>` when the local context expects `T?`.
    ///
    /// E1025 fires only when the most-specific applicable context is
    /// a binary outcome `T~E` (`allow_null_state = false`). A `let x:
    /// T? = ~0` inside a function returning `T~E` no longer false-
    /// positives because the local-site context (T?) supersedes the
    /// surrounding return-type context.
    fn check_outcome_constructor_context(
        &mut self,
        arm: triet_syntax::OutcomeArm,
        span: Span,
    ) -> Type {
        use triet_syntax::OutcomeArm;
        // Most-specific applicable context. Local site wins.
        let local_context = self.expected_type_stack.last().cloned();
        let surrounding_context = self.current_return_type.clone();
        let resolved_context = local_context.or(surrounding_context);

        // E1025: `~0` against a binary outcome `T~E`. Only fire when
        // the SELECTED context (most-specific) expects no null state.
        if matches!(arm, OutcomeArm::Zero)
            && let Some(Type::Outcome {
                allow_null_state: false,
                ..
            }) = &resolved_context
        {
            self.errors
                .push(TypeError::NullStateInBinaryOutcome { span });
            return Type::Unknown;
        }

        // For `~0` whose local context is a plain `Nullable<T>`,
        // return that Nullable so the let-binding's `.matches()`
        // check succeeds. ADR-0010 Addendum §D (v0.7.4.3-error.6a)
        // already ensured runtime cross-tolerance between `~0` and
        // `Constant::Null`; the typecheck path here closes the loop
        // by giving the literal a concrete Nullable shape.
        if matches!(arm, OutcomeArm::Zero)
            && let Some(Type::Nullable(_)) = &resolved_context
        {
            return resolved_context.clone().unwrap_or(Type::Unknown);
        }

        // Outcome context (T~E or T?~E) — return it directly so the
        // surrounding `.matches()` can tighten the constructor's shape.
        match &resolved_context {
            Some(Type::Outcome { .. }) => resolved_context.clone().unwrap_or(Type::Unknown),
            _ => Type::Unknown,
        }
    }

    /// Check `inner ~? |capture| early_return` per ADR-0020 §3.1.
    ///
    /// 1. Inner must be Outcome (else this operator is meaningless).
    /// 2. Caller's `current_return_type` must be Outcome (E1028).
    /// 3. Inner's error type must match caller's error type — explicit
    ///    Check `inner ~+> |v| body` / `~0> body` / `~-> |e| body`.
    ///
    /// For the `Negative` arm, delegates to `check_outcome_propagate`
    /// (identical semantics when body is early-return). Other arms
    /// are stubs for v0.7.4.3-error.4.
    fn check_outcome_arm_handler(
        &mut self,
        inner: ExprId,
        arm: triet_syntax::OutcomeArm,
        capture_name: Option<&str>,
        body: ExprId,
        span: Span,
    ) -> Type {
        use triet_syntax::OutcomeArm;
        if arm == OutcomeArm::Negative {
            self.check_outcome_propagate(inner, capture_name, body, span)
        } else {
            // Stub — pending v0.7.4.3-error.5.
            let _ = self.infer_expression(inner);
            let _ = self.infer_expression(body);
            Type::Unknown
        }
    }

    ///    conversion required (E1029) when they differ.
    /// 4. Capture name binds in the early-return form's scope.
    fn check_outcome_propagate(
        &mut self,
        inner: ExprId,
        capture_name: Option<&str>,
        early_return: ExprId,
        span: Span,
    ) -> Type {
        let inner_ty = self.infer_expression(inner);
        // E1028: caller must be fallible.
        let caller_outcome = if matches!(&self.current_return_type, Some(Type::Outcome { .. })) {
            self.current_return_type.clone()
        } else {
            self.errors
                .push(TypeError::PropagateInNonFallibleContext { span: span.clone() });
            None
        };
        // E1029: error-type compatibility check.
        if let (
            Type::Outcome {
                error_type: inner_e,
                value_type: inner_v,
                allow_null_state: inner_null,
            },
            Some(Type::Outcome {
                error_type: outer_e,
                ..
            }),
        ) = (&inner_ty, &caller_outcome)
        {
            if !inner_e.matches(outer_e) {
                self.errors.push(TypeError::ErrorTypeMismatch {
                    inner_error: (**inner_e).clone(),
                    outer_error: (**outer_e).clone(),
                    span,
                });
            }
            // Bind capture inside early_return scope.
            self.env.push_frame();
            if let Some(name) = capture_name {
                self.env.declare(name, (**inner_e).clone());
            }
            self.infer_expression(early_return);
            self.env.pop_frame();
            // Propagate evaluates to inner's success type. For T?~E,
            // the success-path also threads the null state through.
            return if *inner_null {
                Type::Nullable(Box::new((**inner_v).clone()))
            } else {
                (**inner_v).clone()
            };
        }
        // Unknown inner — still check early_return scope.
        self.env.push_frame();
        if let Some(name) = capture_name {
            self.env.declare(name, Type::Unknown);
        }
        self.infer_expression(early_return);
        self.env.pop_frame();
        Type::Unknown
    }

    /// Check `inner ~: default` per ADR-0020 §3.2. Result type is the
    /// inner's success type. Default expression must match success type.
    fn check_outcome_default(&mut self, inner: ExprId, default: ExprId) -> Type {
        let inner_ty = self.infer_expression(inner);
        let default_ty = self.infer_expression(default);
        if let Type::Outcome {
            value_type,
            allow_null_state,
            ..
        } = &inner_ty
        {
            let expected = if *allow_null_state {
                Type::Nullable(Box::new((**value_type).clone()))
            } else {
                (**value_type).clone()
            };
            if !expected.matches(&default_ty) {
                self.errors.push(TypeError::Mismatch {
                    expected: expected.clone(),
                    found: default_ty,
                    span: self.arena.expression(default).span.clone(),
                });
            }
            return expected;
        }
        Type::Unknown
    }

    fn check_binary_op(
        &mut self,
        operator: BinaryOperator,
        left: ExprId,
        right: ExprId,
        span: Span,
    ) -> Type {
        let left_ty = self.infer_expression(left);
        let right_ty = self.infer_expression(right);

        match operator {
            BinaryOperator::Add
            | BinaryOperator::Sub
            | BinaryOperator::Mul
            | BinaryOperator::Div
            | BinaryOperator::Mod
            | BinaryOperator::Pow => {
                if !(left_ty.is_numeric() || matches!(left_ty, Type::Unknown))
                    || !(right_ty.is_numeric() || matches!(right_ty, Type::Unknown))
                    || !left_ty.matches(&right_ty)
                {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description: "two numeric operands of the same type".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                left_ty
            }
            BinaryOperator::Eq | BinaryOperator::Ne => {
                if !left_ty.matches(&right_ty) {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description: "two operands of the same type".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                // ADR-0021 §2.2: comparison refinement propagation.
                eq_result_type(&left_ty, &right_ty)
            }
            BinaryOperator::Lt | BinaryOperator::Le | BinaryOperator::Gt | BinaryOperator::Ge => {
                if !(left_ty.is_numeric() || matches!(left_ty, Type::Unknown))
                    || !(right_ty.is_numeric() || matches!(right_ty, Type::Unknown))
                    || !left_ty.matches(&right_ty)
                {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description: "two numeric operands of the same type".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                // ADR-0021 §2.2: numeric ordering is total — result is
                // Trilean! (no Unknown propagation from Integer/Tryte/
                // Long; Trit has total ordering too).
                Type::TRILEAN_KNOWN
            }
            BinaryOperator::LukAnd
            | BinaryOperator::LukOr
            | BinaryOperator::LukXor
            | BinaryOperator::LukIff
            | BinaryOperator::LukImplies
            | BinaryOperator::KleeneXor
            | BinaryOperator::KleeneIff
            | BinaryOperator::KleeneImplies => {
                // Accept either refined or generic Trilean on each
                // side; refinement of the result is computed below.
                let left_ok = left_ty.is_trilean() || matches!(left_ty, Type::Unknown);
                let right_ok = right_ty.is_trilean() || matches!(right_ty, Type::Unknown);
                if !left_ok || !right_ok {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description: "two `Trilean` operands".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                // ADR-0021 §2.3: Łukasiewicz / Kleene preserve refinement
                // when both operands are Trilean! — Ł3 truth tables for
                // {True, False} inputs never produce Unknown.
                if left_ty.is_refined_trilean() && right_ty.is_refined_trilean() {
                    Type::TRILEAN_KNOWN
                } else {
                    Type::TRILEAN
                }
            }
        }
    }

    fn check_unary_negate(&mut self, operand: ExprId, span: Span) -> Type {
        let ty = self.infer_expression(operand);
        if ty.is_numeric() || ty.is_trilean() || matches!(ty, Type::Unknown) {
            // ADR-0021 §2.3: negation preserves refinement (cannot
            // introduce Unknown from non-Unknown input).
            ty
        } else {
            self.errors.push(TypeError::InvalidUnary {
                operator: "-/!/not".to_owned(),
                operand: ty,
                span,
            });
            Type::Unknown
        }
    }

    /// v0.9.x.atomic.7b: borrow expression typecheck per ADR-0031 §4.
    /// Infers the operand's type `T`, wraps into
    /// `Type::Reference(form, T)`. Borrow-of-borrow is refused per §2
    /// last bullet ("Nested borrow expression — refused by typecheck").
    /// Other operand restrictions (IDENT + field-access only) are
    /// enforced by the parser; typecheck trusts the parser's grammar.
    ///
    /// v0.9.x.atomic.7d: for **owning** forms (`StrongFrozen` /
    /// `StrongMutable`), mark the operand's base identifier as Moved
    /// for E2420 `UseAfterMove` enforcement per ADR-0025 §5.1. `&0`
    /// and `&-` do NOT consume — those forms borrow / observe without
    /// transferring ownership.
    fn check_borrow(&mut self, form: ReferenceForm, operand: ExprId, span: Span) -> Type {
        let inner = self.infer_expression(operand);
        if let Type::Reference(..) = inner {
            self.errors.push(TypeError::InvalidUnary {
                operator: "&+/&0/&-".to_owned(),
                operand: inner,
                span,
            });
            return Type::Unknown;
        }
        if matches!(
            form,
            ReferenceForm::StrongFrozen | ReferenceForm::StrongMutable
        ) && let Some(base) = self.extract_base_identifier(operand)
        {
            self.mark_moved(&base, span);
        }
        Type::Reference(form, Box::new(inner))
    }

    #[allow(clippy::too_many_lines)]
    fn check_call(&mut self, callee: ExprId, arguments: &[ExprId], span: Span) -> Type {
        // ── Overload resolution ──
        // If the callee is a simple identifier that has overloaded
        // signatures but NO regular binding, try each candidate and
        // pick the first match. A regular binding (user-defined or
        // prelude) takes precedence.
        if let Expr::Identifier { name } = &self.arena.expression(callee).node
            && self.env.lookup(name).is_none()
            && let Some(candidates) = self.env.lookup_all(name)
        {
            return self.resolve_overload(name, &candidates, arguments, &span);
        }

        let callee_ty = self.infer_expression(callee);

        // Try function call first.
        if let Type::Function {
            type_params,
            parameters,
            return_type,
        } = callee_ty.clone()
        {
            // v0.9.x.atomic.6: E2530 conservative fire conditions per
            // ADR-0028 §10. Detect `compare_exchange(..., success_ord,
            // failure_ord)` calls where success is weaker than failure
            // — semantically nonsensical. Gating signal is dual:
            //
            // 1. Callee identifier name is `compare_exchange` (covers
            //    both `from sys.atomic import compare_exchange` direct
            //    use and `sys.atomic.compare_exchange` qualified use via
            //    field access).
            // 2. Function signature shape matches: 5 parameters with
            //    params[3] and params[4] both `Type::UserEnum { name:
            //    "Ordering", .. }`.
            //
            // Why both: the name alone risks false positives if a user
            // defines an unrelated `compare_exchange`; the shape alone
            // would flag any 2-Ordering function. Conservative scope per
            // §10 — narrow & always-wrong cases only. Aliased imports
            // (`as cas`) escape detection; documented limitation pending
            // corpus need (§10 deferred patterns).
            self.check_atomic_ordering(callee, &parameters, arguments, &span);
            if arguments.len() != parameters.len() {
                self.errors.push(TypeError::WrongArity {
                    expected: parameters.len(),
                    found: arguments.len(),
                    span: span.clone(),
                });
            }
            // Generic function inference per Q2-A (v0.7.4.1):
            // walk param/arg pairs and bind TypeParam(name) → concrete
            // arg type into `sub_map`. Reuses existing
            // [`Type::substitute`] machinery shared with generic enum
            // constructors. Empty `type_params` short-circuits to the
            // pre-v0.7.4.1 path.
            let mut sub_map: std::collections::HashMap<String, Type> =
                std::collections::HashMap::new();
            for (i, argument) in arguments.iter().enumerate() {
                let arg_ty = self.infer_expression(*argument);
                if let Some(expected) = parameters.get(i) {
                    if !type_params.is_empty() {
                        extract_type_params(expected, &arg_ty, &mut sub_map);
                    }
                    // Substitute already-bound type params before
                    // comparing — handles `function f<T>(a: T, b: T)`
                    // where the second arg must match the first's
                    // inferred T.
                    let expected_sub = expected.substitute(&sub_map);
                    if !expected_sub.matches(&arg_ty) {
                        self.errors.push(TypeError::Mismatch {
                            expected: expected_sub,
                            found: arg_ty,
                            span: self.arena.expression(*argument).span.clone(),
                        });
                    }
                }
            }
            if type_params.is_empty() {
                return *return_type;
            }
            // Enforce bounds
            for tp in type_params {
                if matches!(tp.bound, Some(triet_syntax::GenericBound::Send))
                    && let Some(arg_ty) = sub_map.get(&tp.name)
                    && !arg_ty.is_send()
                {
                    self.errors.push(crate::error::TypeError::Concurrency(
                        crate::error::ConcurrencyError::NotSendCannotCrossBoundary {
                            ty: arg_ty.to_string(),
                            span: span.clone(),
                        },
                    ));
                }
            }
            return return_type.substitute(&sub_map);
        }

        // Try qualified enum variant construction: `CD.SomeInt(5)`.
        // The callee is a FieldAccess into an enum type for a payload
        // variant. The resolution was already recorded by infer_expression.
        if let Expr::FieldAccess { object, field } = &self.arena.expression(callee).node
            && let Expr::Identifier { name: enum_name } = &self.arena.expression(*object).node
            && let Some(Type::UserEnum { variants, .. }) = self.env.lookup(enum_name).cloned()
            && let Some((_variant_name, payload)) = variants.iter().find(|(n, _)| n == field)
        {
            // Check arity and payload type.
            match (arguments.len(), payload) {
                (1, Some(expected_ty)) => {
                    let arg_ty = self.infer_expression(arguments[0]);
                    if !expected_ty.matches(&arg_ty) {
                        self.errors.push(TypeError::Mismatch {
                            expected: (**expected_ty).clone(),
                            found: arg_ty,
                            span: self.arena.expression(arguments[0]).span.clone(),
                        });
                    }
                }
                (0, None) => {} // unit variant — OK
                (n, Some(_)) => {
                    self.errors.push(TypeError::WrongArity {
                        expected: 1,
                        found: n,
                        span: span.clone(),
                    });
                }
                (n, None) if n > 0 => {
                    self.errors.push(TypeError::WrongArity {
                        expected: 0,
                        found: n,
                        span: span.clone(),
                    });
                }
                _ => {}
            }
            return self.env.lookup(enum_name).cloned().unwrap_or(Type::Unknown);
        }

        // Try enum variant construction: `Some(5)`, `None`.
        if let Expr::Identifier {
            name: ref callee_name,
        } = self.arena.expression(callee).node
            && let Some(enum_ty) = self.lookup_enum_variant(callee_name)
        {
            let Type::UserEnum {
                name: _enum_name,
                type_params,
                variants,
            } = &enum_ty
            else {
                unreachable!()
            };
            let (_, payload) = variants
                .iter()
                .find(|(n, _)| n == callee_name)
                .expect("lookup_enum_variant guarantees this");
            // Infer type arguments from the call arguments when
            // the variant payload uses type params.
            let mut sub_map: std::collections::HashMap<String, Type> =
                std::collections::HashMap::new();
            match (arguments.len(), payload) {
                (1, Some(expected_ty)) => {
                    let arg_ty = self.infer_expression(arguments[0]);
                    // If the payload type is a TypeParam, infer from arg.
                    if let Type::TypeParam(tp) = expected_ty.as_ref() {
                        sub_map.insert(tp.clone(), arg_ty);
                    } else if !expected_ty.matches(&arg_ty) {
                        self.errors.push(TypeError::Mismatch {
                            expected: (**expected_ty).clone(),
                            found: arg_ty,
                            span: self.arena.expression(arguments[0]).span.clone(),
                        });
                    }
                }
                (0, None) => {} // unit variant — no inference needed
                (n, Some(_)) => {
                    self.errors.push(TypeError::WrongArity {
                        expected: 1,
                        found: n,
                        span,
                    });
                }
                (n, None) => {
                    self.errors.push(TypeError::WrongArity {
                        expected: 0,
                        found: n,
                        span,
                    });
                }
            }
            // If we have type params without inferred concrete types,
            // leave them as TypeParam (will be caught by context).
            if !sub_map.is_empty() || type_params.is_empty() {
                return enum_ty.substitute(&sub_map);
            }
            return enum_ty.clone();
        }

        if !matches!(callee_ty, Type::Unknown) {
            self.errors.push(TypeError::NotCallable {
                found: callee_ty,
                span,
            });
        }
        Type::Unknown
    }

    /// Try each overloaded function signature for `name` against the
    /// given arguments. Returns the return type of the first candidate
    /// whose parameter types all match. Emits `UndefinedName` if no
    /// candidate matches (the name exists in overloads but no signature
    /// fits the argument types).
    fn resolve_overload(
        &mut self,
        name: &str,
        candidates: &[Type],
        arguments: &[ExprId],
        span: &Span,
    ) -> Type {
        // Infer argument types once.
        let arg_tys: Vec<Type> = arguments
            .iter()
            .map(|a| self.infer_expression(*a))
            .collect();

        for candidate in candidates {
            if let Type::Function {
                type_params,
                parameters,
                return_type,
            } = candidate
            {
                if !type_params.is_empty() {
                    continue; // skip generic overloads in Bậc A
                }
                if arguments.len() != parameters.len() {
                    continue;
                }
                let all_match = parameters
                    .iter()
                    .zip(&arg_tys)
                    .all(|(expected, found)| expected.matches(found));
                if all_match {
                    return *return_type.clone();
                }
            }
        }

        // No matching overload — list candidates in the error message.
        let candidate_list: Vec<String> = candidates
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        self.errors.push(TypeError::NoMatchingOverload {
            name: name.to_owned(),
            candidates: candidate_list.join(", "),
            span: span.clone(),
        });
        Type::Unknown
    }

    /// Scan the type environment for an enum variant with the given
    /// name. Returns the enum type that owns the variant.
    fn lookup_enum_variant(&self, name: &str) -> Option<Type> {
        // Scan the root frame only — enum type definitions live at
        // module scope, not inside function/block scopes. Variable
        // bindings (like `let c: Option<Integer> = ...`) have
        // UserEnum as their type and would shadow the generic
        // definition if we scanned inner frames.
        for binding in self.env.frames.first()?.names.values() {
            if let Type::UserEnum { variants, .. } = &binding.ty
                && variants.iter().any(|(n, _)| n == name)
            {
                return Some(binding.ty.clone());
            }
        }
        None
    }

    /// Resolve a bare enum variant name to its full resolution data.
    ///
    /// Returns `None` if the variant is not found or is ambiguous
    /// (present in multiple enum types). On ambiguity, emits
    /// [`TypeError::AmbiguousEnumVariant`].
    ///
    /// This is the canonical resolution point — the lowerer consumes
    /// the resolution map rather than re-scanning enum layouts.
    fn resolve_enum_variant(
        &mut self,
        name: &str,
        span: &Span,
    ) -> Option<crate::EnumVariantResolution> {
        let root_frame = self.env.frames.first()?;
        let mut matches: Vec<(&str, &Vec<(String, Option<Box<Type>>)>)> = Vec::new();
        for binding in root_frame.names.values() {
            if let Type::UserEnum {
                name: enum_name,
                variants,
                ..
            } = &binding.ty
                && variants.iter().any(|(n, _)| n == name)
            {
                matches.push((enum_name.as_str(), variants));
            }
        }
        match matches.len() {
            0 => None,
            1 => {
                let (enum_name, variants) = matches[0];
                let variant = variants.iter().find(|(n, _)| n == name)?;
                let disc = variants
                    .iter()
                    .position(|(n, _)| n == name)
                    .map(|i| i as i64)?;
                Some(crate::EnumVariantResolution {
                    enum_name: enum_name.to_string(),
                    variant_name: name.to_string(),
                    discriminant: disc,
                    has_payload: variant.1.is_some(),
                })
            }
            _ => {
                self.errors.push(TypeError::AmbiguousEnumVariant {
                    variant: name.to_string(),
                    enum_a: matches[0].0.to_string(),
                    enum_b: matches[1].0.to_string(),
                    span: span.clone(),
                });
                None
            }
        }
    }

    fn check_method_call(
        &mut self,
        receiver: ExprId,
        method: &str,
        arguments: &[ExprId],
        span: Span,
    ) -> Type {
        let receiver_ty = self.infer_expression(receiver);
        if let Some(return_ty) = builtin_method_type(&receiver_ty, method, arguments.len()) {
            for argument in arguments {
                let _ = self.infer_expression(*argument);
            }
            return return_ty;
        }

        if matches!(receiver_ty, Type::Unknown) {
            return Type::Unknown;
        }

        // Qualified enum variant construction via method-call syntax:
        // `OptionA.SomeInt(42)` parses as MethodCall.
        if let Type::UserEnum { variants, .. } = &receiver_ty
            && let Some((_variant_name, payload)) = variants.iter().find(|(n, _)| n == method)
        {
            // Check arity and payload type.
            match (arguments.len(), payload) {
                (1, Some(expected_ty)) => {
                    let arg_ty = self.infer_expression(arguments[0]);
                    if !expected_ty.matches(&arg_ty) {
                        self.errors.push(TypeError::Mismatch {
                            expected: (**expected_ty).clone(),
                            found: arg_ty,
                            span: self.arena.expression(arguments[0]).span.clone(),
                        });
                    }
                }
                (0, None) => {} // unit variant via method call — unusual but valid
                (n, Some(_)) => {
                    self.errors.push(TypeError::WrongArity {
                        expected: 1,
                        found: n,
                        span,
                    });
                }
                _ => {}
            }
            return receiver_ty.clone();
        }

        self.errors.push(TypeError::UnknownMember {
            member: method.to_owned(),
            found: receiver_ty,
            span,
        });
        Type::Unknown
    }

    fn check_struct_literal(
        &mut self,
        name: &str,
        fields: &[(String, ExprId)],
        span: Span,
    ) -> Type {
        let ty = self.env.lookup(name).cloned().unwrap_or_else(|| {
            self.errors.push(TypeError::UndefinedName {
                name: name.to_owned(),
                span: span.clone(),
            });
            Type::Unknown
        });
        let Type::UserStruct {
            name: _,
            fields: def_fields,
            ..
        } = &ty
        else {
            self.errors.push(TypeError::Mismatch {
                expected: Type::Unknown,
                found: ty,
                span,
            });
            return Type::Unknown;
        };
        // Check field names and types.
        for (field_name, value) in fields {
            let def = def_fields.iter().find(|(n, _)| n == field_name);
            let Some((_, expected_ty)) = def else {
                self.errors.push(TypeError::UnknownMember {
                    member: field_name.clone(),
                    found: ty.clone(),
                    span: self.arena.expression(*value).span.clone(),
                });
                continue;
            };
            // v0.7.4.3-debt.6: push the field's expected type so
            // outcome constructors (`~0` especially) can resolve their
            // shape against it. Extends the let-binding push from
            // .debt.3 to struct field positions. Without this,
            // `Foo { tag: ~0 }` inside a function returning `T~E`
            // raises false-positive E1025.
            let value_ty = self.with_expected(expected_ty.clone(), |s| s.infer_expression(*value));
            if !expected_ty.matches(&value_ty) {
                self.errors.push(TypeError::Mismatch {
                    expected: expected_ty.clone(),
                    found: value_ty,
                    span: self.arena.expression(*value).span.clone(),
                });
            }
        }
        ty.clone()
    }

    fn check_enum_literal(
        &mut self,
        name: &str,
        variant_name: &str,
        payload: Option<&ExprId>,
        span: Span,
    ) -> Type {
        let ty = self.env.lookup(name).cloned().unwrap_or_else(|| {
            self.errors.push(TypeError::UndefinedName {
                name: name.to_owned(),
                span: span.clone(),
            });
            Type::Unknown
        });
        let Type::UserEnum {
            name: _, variants, ..
        } = &ty
        else {
            self.errors.push(TypeError::Mismatch {
                expected: Type::Unknown,
                found: ty,
                span,
            });
            return Type::Unknown;
        };
        let Some((_, def_payload)) = variants.iter().find(|(n, _)| n == variant_name) else {
            self.errors.push(TypeError::UnknownMember {
                member: variant_name.to_owned(),
                found: ty.clone(),
                span,
            });
            return Type::Unknown;
        };
        match (payload, def_payload) {
            (Some(val_expr), Some(expected_ty)) => {
                let val_ty = self.infer_expression(*val_expr);
                if !expected_ty.matches(&val_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: (**expected_ty).clone(),
                        found: val_ty,
                        span: self.arena.expression(*val_expr).span.clone(),
                    });
                }
            }
            (None, None) => {} // unit variant — OK
            (Some(_), None) => {
                self.errors.push(TypeError::WrongArity {
                    expected: 0,
                    found: 1,
                    span,
                });
            }
            (None, Some(_)) => {
                self.errors.push(TypeError::WrongArity {
                    expected: 1,
                    found: 0,
                    span,
                });
            }
        }
        ty.clone()
    }

    fn check_field_access(
        &mut self,
        field_access_id: ExprId,
        object: ExprId,
        field: &str,
        span: Span,
    ) -> Type {
        let object_ty = self.infer_expression(object);
        if matches!(object_ty, Type::Unknown) {
            return Type::Unknown;
        }
        // Struct field access.
        if let Type::UserStruct { fields, .. } = &object_ty {
            if let Some((_, field_ty)) = fields.iter().find(|(n, _)| n == field) {
                return field_ty.clone();
            }
            self.errors.push(TypeError::UnknownMember {
                member: field.to_owned(),
                found: object_ty,
                span,
            });
            return Type::Unknown;
        }
        // Qualified enum unit variant: `CD.None`, `Color.Red`.
        // Resolve at typecheck time; lowerer consumes the resolution map.
        if let Type::UserEnum {
            name: enum_name,
            variants,
            ..
        } = &object_ty
        {
            if let Some(variant_idx) = variants.iter().position(|(n, _)| n == field) {
                let (_variant_name, payload) = &variants[variant_idx];
                let has_payload = payload.is_some();
                // Record resolution for the lowerer — same path as
                // bare-variant Identifier and Call constructor.
                self.expr_resolutions.insert(
                    field_access_id,
                    triet_syntax::EnumVariantResolution {
                        enum_name: enum_name.clone(),
                        variant_name: field.to_string(),
                        discriminant: variant_idx as i64,
                        has_payload,
                    },
                );
                // Return the enum type itself. If the variant has a
                // payload, the user must use call syntax:
                // `CD.SomeInt(5)` which routes through
                // `Expr::Call` + `Expr::FieldAccess`.
                if has_payload {
                    // Payload variant accessed without args — this
                    // is the enum type (the variant isn't being
                    // constructed, just referenced).
                    return object_ty.clone();
                }
                return object_ty.clone();
            }
            self.errors.push(TypeError::UnknownMember {
                member: field.to_owned(),
                found: object_ty.clone(),
                span,
            });
            return Type::Unknown;
        }
        self.errors.push(TypeError::UnknownMember {
            member: field.to_owned(),
            found: object_ty,
            span,
        });
        Type::Unknown
    }

    fn check_tuple_index(&mut self, tuple: ExprId, index: usize, span: Span) -> Type {
        let tuple_ty = self.infer_expression(tuple);
        match tuple_ty {
            Type::Tuple(elements) => {
                if let Some(element_type) = elements.get(index) {
                    element_type.clone()
                } else {
                    self.errors.push(TypeError::TupleIndexOutOfRange {
                        index,
                        length: elements.len(),
                        span,
                    });
                    Type::Unknown
                }
            }
            Type::Unknown => Type::Unknown,
            other => {
                self.errors.push(TypeError::UnknownMember {
                    member: index.to_string(),
                    found: other,
                    span,
                });
                Type::Unknown
            }
        }
    }

    fn check_safe_field_access(&mut self, object: ExprId, field: &str, span: Span) -> Type {
        let object_ty = self.infer_expression(object);
        if !object_ty.is_nullable() && !matches!(object_ty, Type::Unknown) {
            self.errors.push(TypeError::NotNullable {
                operator: "?.".to_owned(),
                found: object_ty,
                span,
            });
            return Type::Unknown;
        }
        // For v0.1, we don't have struct fields, so report unknown.
        self.errors.push(TypeError::UnknownMember {
            member: field.to_owned(),
            found: object_ty,
            span,
        });
        Type::Unknown
    }

    fn check_safe_method_call(
        &mut self,
        receiver: ExprId,
        method: &str,
        arguments: &[ExprId],
        span: Span,
    ) -> Type {
        let receiver_ty = self.infer_expression(receiver);
        if !receiver_ty.is_nullable() && !matches!(receiver_ty, Type::Unknown) {
            self.errors.push(TypeError::NotNullable {
                operator: "?.".to_owned(),
                found: receiver_ty,
                span,
            });
            return Type::Unknown;
        }
        let inner = receiver_ty.unwrap_nullable().clone();
        if let Some(return_ty) = builtin_method_type(&inner, method, arguments.len()) {
            for argument in arguments {
                let _ = self.infer_expression(*argument);
            }
            return Type::Nullable(Box::new(return_ty));
        }
        self.errors.push(TypeError::UnknownMember {
            member: method.to_owned(),
            found: inner,
            span,
        });
        Type::Unknown
    }

    fn check_elvis(&mut self, object: ExprId, default: ExprId, span: Span) -> Type {
        let object_ty = self.infer_expression(object);
        let default_ty = self.infer_expression(default);
        if !object_ty.is_nullable() && !matches!(object_ty, Type::Unknown) {
            self.errors.push(TypeError::NotNullable {
                operator: "?:".to_owned(),
                found: object_ty,
                span,
            });
            return default_ty;
        }
        let inner = object_ty.unwrap_nullable().clone();
        if !inner.matches(&default_ty) {
            self.errors.push(TypeError::Mismatch {
                expected: inner.clone(),
                found: default_ty,
                span: self.arena.expression(default).span.clone(),
            });
        }
        inner
    }

    fn check_force_unwrap(&mut self, inner: ExprId, span: Span) -> Type {
        let ty = self.infer_expression(inner);
        if !ty.is_nullable() && !matches!(ty, Type::Unknown) {
            self.errors.push(TypeError::NotNullable {
                operator: "!!".to_owned(),
                found: ty.clone(),
                span,
            });
            return ty;
        }
        ty.unwrap_nullable().clone()
    }

    fn check_if(
        &mut self,
        condition: ExprId,
        then_branch: ExprId,
        else_branch: Option<ExprId>,
        treat_unknown_as_false: bool,
        span: Span,
    ) -> Type {
        let cond_ty = self.infer_expression(condition);
        let cond_span = self.arena.expression(condition).span.clone();
        self.check_condition_type(cond_ty, treat_unknown_as_false, cond_span);

        // v0.9.x.atomic.7d: branch-aware move tracking per ADR-0031 §4
        // join semantics. Snapshot before each branch, walk, restore;
        // join the two end states with any-branch-moves-wins.
        let snapshot = self.snapshot_moves();
        let then_ty = self.infer_expression(then_branch);
        let after_then = std::mem::replace(&mut self.move_states, snapshot.clone());

        match else_branch {
            None => {
                // No else branch: the "didn't enter" path stays at
                // `snapshot`. Join with `after_then`.
                self.move_states = Checker::join_moves(after_then, snapshot);
                Type::Unit
            }
            Some(block) => {
                let else_ty = self.infer_expression(block);
                let after_else = std::mem::take(&mut self.move_states);
                self.move_states = Checker::join_moves(after_then, after_else);
                if let Ok(unified) = try_unify(&then_ty, &else_ty) {
                    unified
                } else {
                    self.errors.push(TypeError::Mismatch {
                        expected: then_ty.clone(),
                        found: else_ty,
                        span,
                    });
                    then_ty
                }
            }
        }
    }

    fn check_match(&mut self, scrutinee: ExprId, arms: &[MatchArm], span: Span) -> Type {
        let scrutinee_ty = self.infer_expression(scrutinee);

        // E1035: reject ~- arm on nullable type (T? has no error state).
        if matches!(scrutinee_ty, Type::Nullable(_)) {
            for arm in arms {
                if let triet_syntax::Pattern::OutcomeArm {
                    arm: triet_syntax::OutcomeArm::Negative,
                    ..
                } = &self.arena.pattern(arm.pattern).node
                {
                    self.errors.push(TypeError::NegativeArmOnNullable {
                        span: self.arena.pattern(arm.pattern).span.clone(),
                    });
                }
            }
        }

        let mut arm_type: Option<Type> = None;

        // v0.9.x.atomic.7d: branch-aware move tracking per ADR-0031 §4.
        // Snapshot before each arm so each starts from the same state;
        // join all post-arm states with any-arm-moves-wins. Mirrors the
        // `check_if` else-branch handling but iterates over N arms.
        let pre_match = self.snapshot_moves();
        let mut joined_post: Option<std::collections::HashMap<String, MoveState>> = None;

        for arm in arms {
            pre_match.clone_into(&mut self.move_states);
            self.env.push_frame();
            self.bind_pattern(arm.pattern, &scrutinee_ty);
            if let Some(guard) = arm.guard {
                let guard_ty = self.infer_expression(guard);
                let guard_span = self.arena.expression(guard).span.clone();
                self.check_condition_type(guard_ty, false, guard_span);
            }
            let body_ty = self.infer_expression(arm.body);
            self.env.pop_frame();
            let after_arm = std::mem::take(&mut self.move_states);
            joined_post = Some(match joined_post {
                None => after_arm,
                Some(acc) => Checker::join_moves(acc, after_arm),
            });

            match &arm_type {
                None => arm_type = Some(body_ty),
                Some(expected) => match try_unify(expected, &body_ty) {
                    Ok(unified) => arm_type = Some(unified),
                    Err(()) => {
                        self.errors.push(TypeError::MatchArmMismatch {
                            expected: expected.clone(),
                            found: body_ty,
                            span: self.arena.expression(arm.body).span.clone(),
                        });
                    }
                },
            }
        }

        // v0.9.x.atomic.7d: set move-state to the joined post-match
        // result. If `joined_post` is None (empty match), keep the
        // pre-match snapshot.
        self.move_states = joined_post.unwrap_or(pre_match);

        // v0.7.4.3-error.2 (ADR-0020 §5.1): exhaustiveness check for
        // outcome scrutinee. Binary T~E requires ~+ and ~-; ternary
        // T?~E requires all three (~+, ~0, ~-). Wildcard `_` covers
        // any missing arm.
        if let Type::Outcome {
            allow_null_state, ..
        } = &scrutinee_ty
        {
            self.check_outcome_exhaustiveness(arms, *allow_null_state, span.clone());
        }

        // Nullable T? exhaustiveness: requires ~+ and ~0 (or _ wildcard).
        if let Type::Nullable(_) = &scrutinee_ty {
            self.check_nullable_exhaustiveness(arms, span.clone());
        }

        // Enum exhaustiveness: requires all variants covered (or _ wildcard).
        if let Type::UserEnum { name, .. } = &scrutinee_ty {
            self.check_enum_exhaustiveness(name, arms, span.clone());
        }

        arm_type.unwrap_or_else(|| {
            // Empty match — flag as a type error.
            self.errors.push(TypeError::Mismatch {
                expected: Type::Unknown,
                found: Type::Unit,
                span,
            });
            Type::Unknown
        })
    }

    /// Verify a match on outcome covers required arms (E1026). Wildcard
    /// `_` arm covers any missing arm.
    fn check_outcome_exhaustiveness(
        &mut self,
        arms: &[MatchArm],
        allow_null_state: bool,
        span: Span,
    ) {
        use triet_syntax::OutcomeArm as Arm;
        // Check for wildcard short-circuit.
        for arm in arms {
            if matches!(
                self.arena.pattern(arm.pattern).node,
                triet_syntax::Pattern::Wildcard
            ) {
                return;
            }
        }
        let mut has_pos = false;
        let mut has_neg = false;
        let mut has_zero = false;
        for arm in arms {
            if let triet_syntax::Pattern::OutcomeArm {
                arm: outcome_arm, ..
            } = &self.arena.pattern(arm.pattern).node
            {
                match outcome_arm {
                    Arm::Positive => has_pos = true,
                    Arm::Negative => has_neg = true,
                    Arm::Zero => has_zero = true,
                }
            }
        }
        let mut missing = Vec::new();
        if !has_pos {
            missing.push("`~+`");
        }
        if !has_neg {
            missing.push("`~-`");
        }
        if allow_null_state && !has_zero {
            missing.push("`~0`");
        }
        if !missing.is_empty() {
            self.errors.push(TypeError::NonExhaustiveOutcomeMatch {
                missing: missing.join(", "),
                span,
            });
        }
    }

    /// Verify a match on nullable `T?` covers required arms (E1026).
    /// Requires `~+` (present) and `~0` (null). Wildcard `_` short-circuits.
    fn check_nullable_exhaustiveness(&mut self, arms: &[MatchArm], span: Span) {
        // Wildcard short-circuit.
        for arm in arms {
            if matches!(
                self.arena.pattern(arm.pattern).node,
                triet_syntax::Pattern::Wildcard
            ) {
                return;
            }
        }
        let mut has_pos = false;
        let mut has_zero = false;
        for arm in arms {
            if let triet_syntax::Pattern::OutcomeArm {
                arm: outcome_arm, ..
            } = &self.arena.pattern(arm.pattern).node
            {
                match outcome_arm {
                    triet_syntax::OutcomeArm::Positive => has_pos = true,
                    triet_syntax::OutcomeArm::Zero => has_zero = true,
                    // ~- on nullable is rejected by E1035 — ignore here.
                    triet_syntax::OutcomeArm::Negative => {}
                }
            }
        }
        let mut missing = Vec::new();
        if !has_pos {
            missing.push("`~+`");
        }
        if !has_zero {
            missing.push("`~0`");
        }
        if !missing.is_empty() {
            self.errors.push(TypeError::NonExhaustiveOutcomeMatch {
                missing: missing.join(", "),
                span,
            });
        }
    }

    /// Verify a match on an enum covers all variants (E1026). Wildcard
    /// `_` arm short-circuits. Scans `self.items` for the enum definition
    /// by matching the scrutinee type name against each `Item::Enum`.
    fn check_enum_exhaustiveness(&mut self, enum_name: &str, arms: &[MatchArm], span: Span) {
        // Wildcard short-circuit.
        for arm in arms {
            if matches!(
                self.arena.pattern(arm.pattern).node,
                triet_syntax::Pattern::Wildcard
            ) {
                return;
            }
        }
        // Find the enum definition.
        let variants: Vec<String> = self
            .items
            .iter()
            .find_map(|item| match &item.node {
                triet_syntax::Item::Enum { def } if def.name == enum_name => {
                    Some(def.variants.iter().map(|v| v.name.clone()).collect())
                }
                _ => None,
            })
            .unwrap_or_default();
        if variants.is_empty() {
            return; // Can't verify — type resolution issue.
        }
        // Use resolved patterns: the typechecker populates pattern_resolutions
        // for every enum variant arm during bind_pattern. AST patterns are
        // Variable("Red") — the resolution tells us it's a Color::Red variant.
        let mut covered: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for arm in arms {
            if let Some(res) = self.pattern_resolutions.get(&arm.pattern) {
                covered.insert(res.variant_name.as_str());
            }
        }
        let missing: Vec<String> = variants
            .iter()
            .filter(|v| !covered.contains(v.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            self.errors.push(TypeError::NonExhaustiveEnumMatch {
                missing: missing.join(", "),
                span,
            });
        }
    }

    /// v0.9.x.atomic.6: E2530 `InvalidAtomicOrdering` check per ADR-0028
    /// §10. Conservative scope — fires only on the `compare_exchange`
    /// success-weaker-than-failure pattern. The Pointer-Relaxed
    /// `fetch_*` pattern defers until the `Pointer` type lands (currently
    /// not parseable per ADR-0028 §2).
    ///
    /// Detection requires both: (1) the callee identifier name is
    /// `compare_exchange`, and (2) the resolved function signature has
    /// 5 parameters with the last two being the `Ordering` enum. This
    /// dual gate guards against false positives on user-defined
    /// look-alike functions and on different functions with two
    /// `Ordering` parameters.
    fn check_atomic_ordering(
        &mut self,
        callee: ExprId,
        parameters: &[Type],
        arguments: &[ExprId],
        span: &Span,
    ) {
        if !callee_name_is(&self.arena.expression(callee).node, "compare_exchange") {
            return;
        }
        if parameters.len() != 5 || arguments.len() != 5 {
            return;
        }
        if !is_ordering_enum(&parameters[3]) || !is_ordering_enum(&parameters[4]) {
            return;
        }
        let Some((s_name, s_strength)) =
            ordering_strength(&self.arena.expression(arguments[3]).node)
        else {
            return;
        };
        let Some((f_name, f_strength)) =
            ordering_strength(&self.arena.expression(arguments[4]).node)
        else {
            return;
        };
        if s_strength < f_strength {
            self.errors.push(TypeError::Concurrency(
                crate::error::ConcurrencyError::InvalidAtomicOrdering {
                    success: s_name.to_string(),
                    failure: f_name.to_string(),
                    span: span.clone(),
                },
            ));
        }
    }
}

/// Return true when the callee expression resolves syntactically to an
/// identifier or field-access with the given name. Aliased imports
/// (`from … import X as Y`) intentionally escape detection — see
/// ADR-0028 §10 conservative scope.
fn callee_name_is(node: &Expr, expected: &str) -> bool {
    match node {
        Expr::Identifier { name } => name == expected,
        Expr::FieldAccess { field, .. } => field == expected,
        _ => false,
    }
}

/// Return true when `ty` is the user-defined `Ordering` enum (declared
/// in `std/sys/atomic.tri` per ADR-0028 §3).
fn is_ordering_enum(ty: &Type) -> bool {
    matches!(ty, Type::UserEnum { name, .. } if name == "Ordering")
}

/// Decode an argument expression as an `Ordering` variant reference,
/// returning the variant name and its strength rank (Relaxed=0,
/// Synchronized=1, Strict=2 per ADR-0028 §3). Returns `None` when the
/// argument is anything other than a bare identifier on a known variant
/// (dynamic ordering values escape v0.9 detection — corpus-deferred).
fn ordering_strength(node: &Expr) -> Option<(&'static str, u8)> {
    let Expr::Identifier { name } = node else {
        return None;
    };
    match name.as_str() {
        "Relaxed" => Some(("Relaxed", 0)),
        "Synchronized" => Some(("Synchronized", 1)),
        "Strict" => Some(("Strict", 2)),
        _ => None,
    }
}

/// Compute the result type of `==` / `!=` between two operands per
/// ADR-0021 §2.2. Returns `Trilean!` when neither operand can introduce
/// Unknown (Integer/Tryte/Long/String/Unit/Trit comparisons + refined
/// Trilean-Trilean), otherwise generic `Trilean`. Mismatched operand
/// types fall through to generic `Trilean` (callers handle the
/// mismatch error separately).
///
/// `Trit == Trit` returns generic `Trilean` — `Trit::Zero` acts as
/// Unknown discriminator per ADR-0010 §3, so the equality propagates.
const fn eq_result_type(left: &Type, right: &Type) -> Type {
    match (left, right) {
        // Nullable / outcome / Trit on either side: Unknown propagates.
        (Type::Nullable(_) | Type::Outcome { .. } | Type::Trit, _)
        | (_, Type::Nullable(_) | Type::Outcome { .. } | Type::Trit) => Type::TRILEAN,
        // Trilean × Trilean: refined only when both sides refined.
        (Type::Trilean { refined: l }, Type::Trilean { refined: r }) => {
            if *l && *r {
                Type::TRILEAN_KNOWN
            } else {
                Type::TRILEAN
            }
        }
        // Trilean × non-Trilean (or vice versa) shouldn't typecheck —
        // matches() guard upstream rejects. Defensive: Trilean side
        // pollutes refinement.
        (Type::Trilean { refined: false }, _) | (_, Type::Trilean { refined: false }) => {
            Type::TRILEAN
        }
        (Type::Trilean { refined: true }, _) | (_, Type::Trilean { refined: true }) => {
            Type::TRILEAN_KNOWN
        }
        // Two non-nullable, non-Trilean, non-Trit primitives: total
        // equality, never Unknown.
        _ => Type::TRILEAN_KNOWN,
    }
}

/// Attempt to unify two branch types (for `if`/`else` and `match` arms).
///
/// Direct match: if `a.matches(b)`, return `a`.
/// Reverse match: if `b.matches(a)`, return `b`.
/// Null-widening: if one side is `Nullable(X)` and the other is `T`,
///   wrap in `Nullable(T)` and return it. Handles `if { "x" } else { null }`
///   where then=String, else=Nullable(Unknown) → Nullable(String)=String?.
fn try_unify(a: &Type, b: &Type) -> Result<Type, ()> {
    if a.matches(b) {
        return Ok(a.clone());
    }
    if b.matches(a) {
        return Ok(b.clone());
    }
    // Null-widening: if one side is a nullable, wrap the other side.
    if a.is_nullable() {
        let wrapped = Type::Nullable(Box::new(b.clone()));
        if wrapped.matches(a) {
            return Ok(wrapped);
        }
    }
    if b.is_nullable() {
        let wrapped = Type::Nullable(Box::new(a.clone()));
        if wrapped.matches(b) {
            return Ok(wrapped);
        }
    }
    Err(())
}

/// Walk a parameter type alongside the concrete argument type and bind
/// `TypeParam(name)` slots in `sub_map`. Supports composites: `Nullable`,
/// `Tuple`, `Range`, `Vector` (via `UserStruct` shape if added later),
/// and the arms reachable from generic stdlib stubs. Conflicting
/// bindings (e.g. `f<T>(a: T, b: T)` called with `(Integer, String)`)
/// leave the first binding intact — `check_call`'s subsequent
/// `expected.matches` emits the user-visible `TypeError`.
///
/// v0.7.4.1 (ADR-0019 Addendum §A7).
fn extract_type_params(
    param: &Type,
    arg: &Type,
    sub_map: &mut std::collections::HashMap<String, Type>,
) {
    match (param, arg) {
        (Type::TypeParam(name), concrete) => {
            // v0.7.4.3-debt.4 (WA-7): prefer concrete bindings over
            // TypeParam ones. The naïve `or_insert_with` semantic
            // lets the first arg "poison" `sub_map[T]` with an
            // unbound `TypeParam("T")` — e.g. `push(new(), 99)`
            // where `new<T>() -> Vector<T>` returns
            // `Vector<TypeParam("T"))` because T cannot be inferred
            // from a zero-arg call. Pre-fix, that self-binding
            // blocked the subsequent `99` arg from setting T =
            // Integer; now we replace the poisoned binding when a
            // concrete arg comes in.
            //
            // The replacement is restricted: only swap when the
            // EXISTING binding is a `TypeParam` AND the new
            // `concrete` is NOT a `TypeParam`. This preserves the
            // "first concrete wins" semantic for `f<T>(a: T, b: T)`
            // called with `(Integer, String)` (T stays Integer).
            match sub_map.get(name) {
                None => {
                    sub_map.insert(name.clone(), concrete.clone());
                }
                Some(existing)
                    if matches!(existing, Type::TypeParam(_))
                        && !matches!(concrete, Type::TypeParam(_)) =>
                {
                    sub_map.insert(name.clone(), concrete.clone());
                }
                _ => {} // keep existing
            }
        }
        (Type::Nullable(p_inner), Type::Nullable(a_inner)) => {
            extract_type_params(p_inner, a_inner, sub_map);
        }
        // Subtype rule: T ⊂ T?. If param is Nullable(TypeParam) and
        // arg is concrete, bind T to the arg's bare type.
        (Type::Nullable(p_inner), concrete) => {
            extract_type_params(p_inner, concrete, sub_map);
        }
        (Type::Tuple(p_elems), Type::Tuple(a_elems)) if p_elems.len() == a_elems.len() => {
            for (p, a) in p_elems.iter().zip(a_elems.iter()) {
                extract_type_params(p, a, sub_map);
            }
        }
        (Type::Range(p_inner), Type::Range(a_inner)) => {
            extract_type_params(p_inner, a_inner, sub_map);
        }
        // User-defined generic types: match by name, walk type-param
        // slots positionally. Catches `Vector<T>` / `HashMap<K, V>`
        // once they land as user types (v0.7.4.2+ stdlib stubs).
        (
            Type::UserStruct {
                name: p_name,
                fields: p_fields,
                ..
            },
            Type::UserStruct {
                name: a_name,
                fields: a_fields,
                ..
            },
        ) if p_name == a_name && p_fields.len() == a_fields.len() => {
            for ((_, p_ty), (_, a_ty)) in p_fields.iter().zip(a_fields.iter()) {
                extract_type_params(p_ty, a_ty, sub_map);
            }
        }
        (
            Type::UserEnum {
                name: p_name,
                variants: p_variants,
                ..
            },
            Type::UserEnum {
                name: a_name,
                variants: a_variants,
                ..
            },
        ) if p_name == a_name && p_variants.len() == a_variants.len() => {
            for ((_, p_pl), (_, a_pl)) in p_variants.iter().zip(a_variants.iter()) {
                if let (Some(p_box), Some(a_box)) = (p_pl, a_pl) {
                    extract_type_params(p_box, a_box, sub_map);
                }
            }
        }
        // Function types (closure params): walk parameters + return.
        (
            Type::Function {
                parameters: p_params,
                return_type: p_ret,
                ..
            },
            Type::Function {
                parameters: a_params,
                return_type: a_ret,
                ..
            },
        ) if p_params.len() == a_params.len() => {
            for (p, a) in p_params.iter().zip(a_params.iter()) {
                extract_type_params(p, a, sub_map);
            }
            extract_type_params(p_ret, a_ret, sub_map);
        }
        // v0.7.4.3-error.2: Outcome → walk value + error types when
        // both sides share the same allow_null_state. Mismatched
        // null-states are concrete type errors (caught by .matches()).
        (
            Type::Outcome {
                value_type: p_v,
                error_type: p_e,
                allow_null_state: p_null,
            },
            Type::Outcome {
                value_type: a_v,
                error_type: a_e,
                allow_null_state: a_null,
            },
        ) if p_null == a_null => {
            extract_type_params(p_v, a_v, sub_map);
            extract_type_params(p_e, a_e, sub_map);
        }
        // Concrete/concrete mismatches and shapes not above are
        // left for `expected.matches(arg)` to surface as TypeError.
        _ => {}
    }
}

/// Map a `BinaryOperator` to its source-code symbol for diagnostics.
const fn operator_symbol(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Sub => "-",
        BinaryOperator::Mul => "*",
        BinaryOperator::Div => "/",
        BinaryOperator::Mod => "%%",
        BinaryOperator::Pow => "**",
        BinaryOperator::Eq => "==",
        BinaryOperator::Ne => "!=",
        BinaryOperator::Lt => "<",
        BinaryOperator::Le => "<=",
        BinaryOperator::Gt => ">",
        BinaryOperator::Ge => ">=",
        BinaryOperator::LukAnd => "and",
        BinaryOperator::LukOr => "or",
        BinaryOperator::LukXor => "xor",
        BinaryOperator::LukIff => "iff",
        BinaryOperator::LukImplies => "implies",
        BinaryOperator::KleeneXor => "kleene_xor",
        BinaryOperator::KleeneIff => "kleene_iff",
        BinaryOperator::KleeneImplies => "kleene_implies",
    }
}

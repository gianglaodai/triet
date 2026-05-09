//! Expression type inference.
//!
//! Implements `Checker::infer_expression` and every per-expression
//! sub-check (binary/unary, calls, methods, field/index, nullable
//! operators, `if`, `match`, lambdas, ranges).
//!
//! The dispatcher `infer_expression` is `pub(super)` so the rest of
//! `check` can call it; helper sub-checks stay private to this file.

use triet_syntax::{
    BinaryOperator, Block, Expr, ExprId, FStringPart, MatchArm, NumericSuffix, Span,
    UnaryOperator,
};

use crate::{error::TypeError, types::Type};

use super::Checker;
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
            Expr::IntegerLiteral { suffix, .. } => match suffix {
                Some(NumericSuffix::Trit) => Type::Trit,
                Some(NumericSuffix::Tryte) => Type::Tryte,
                Some(NumericSuffix::Long) => Type::Long,
                Some(NumericSuffix::Integer) | None => Type::Integer,
            },
            Expr::TernaryLiteral { .. } => Type::Integer,
            Expr::TrileanLiteral(_) => Type::Trilean,
            Expr::StringLiteral(_) => Type::String,
            Expr::FStringLiteral(segments) => {
                for part in &segments.parts {
                    if let FStringPart::Interpolation { expression, .. } = part {
                        // The expression must be type-checkable; its type
                        // is converted via Display by the runtime.
                        let _ = self.infer_expression(*expression);
                    }
                }
                Type::String
            }
            Expr::NullLiteral => {
                // In balanced ternary, `null` is a legitimate value for
                // any `T?` type. It infers as `?(?)` and is narrowed by
                // context — e.g., `if { "x" } else { null }` resolves
                // to `String` via branch unification + widening.
                Type::Nullable(Box::new(Type::Unknown))
            }
            Expr::Identifier(name) => {
                // Try variable/function binding first, then enum variant.
                if let Some(ty) = self.env.lookup(&name).cloned() {
                    ty
                } else if let Some(enum_ty) = self.lookup_enum_variant(&name) {
                    enum_ty
                } else {
                    self.errors.push(TypeError::UndefinedName { name, span });
                    Type::Unknown
                }
            }
            Expr::BinaryOp { operator, left, right } => {
                self.check_binary_op(operator, left, right, span)
            }
            Expr::UnaryOp { operator: UnaryOperator::Negate, operand } => {
                self.check_unary_negate(operand, span)
            }
            Expr::Call { callee, arguments } => self.check_call(callee, &arguments, span),
            Expr::MethodCall { receiver, method, arguments } => {
                self.check_method_call(receiver, &method, &arguments, span)
            }
            Expr::FieldAccess { object, field } => self.check_field_access(object, &field, span),
            Expr::TupleIndex { tuple, index } => self.check_tuple_index(tuple, index, span),
            Expr::SafeFieldAccess { object, field } => {
                self.check_safe_field_access(object, &field, span)
            }
            Expr::SafeMethodCall { receiver, method, arguments } => {
                self.check_safe_method_call(receiver, &method, &arguments, span)
            }
            Expr::ElvisOp { object, default } => self.check_elvis(object, default, span),
            Expr::ForceUnwrap(inner) => self.check_force_unwrap(inner, span),
            Expr::If { condition, then_branch, else_branch, treat_unknown_as_false } => self
                .check_if(
                    condition,
                    &then_branch,
                    else_branch.as_ref(),
                    treat_unknown_as_false,
                    span,
                ),
            Expr::Match { scrutinee, arms } => self.check_match(scrutinee, &arms, span),
            Expr::Block(block) => self.check_block(&block),
            Expr::Tuple(elements) => {
                let types = elements
                    .iter()
                    .map(|e| self.infer_expression(*e))
                    .collect();
                Type::Tuple(types)
            }
            Expr::Lambda { parameters, return_type, body } => {
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
                let declared_return = return_type.map(|id| self.resolve_type(id));
                let return_ty = declared_return.unwrap_or_else(|| body_ty.clone());
                self.env.pop_frame();
                Type::Function {
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
            Expr::StructLiteral { name, fields } => {
                self.check_struct_literal(&name, &fields, span)
            }
            Expr::EnumLiteral { name, variant_name, payload } => {
                self.check_enum_literal(&name, &variant_name, payload.as_ref(), span)
            }
        }
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
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
            | BinaryOperator::Power => {
                if !(left_ty.is_numeric()
                    || matches!(left_ty, Type::Unknown))
                    || !(right_ty.is_numeric() || matches!(right_ty, Type::Unknown))
                    || !left_ty.matches(&right_ty)
                {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description:
                            "two numeric operands of the same type".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                left_ty
            }
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
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
                Type::Trilean
            }
            BinaryOperator::LessThan
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqual => {
                if !(left_ty.is_numeric() || matches!(left_ty, Type::Unknown))
                    || !(right_ty.is_numeric() || matches!(right_ty, Type::Unknown))
                    || !left_ty.matches(&right_ty)
                {
                    self.errors.push(TypeError::InvalidOperands {
                        operator: operator_symbol(operator).to_owned(),
                        expected_description:
                            "two numeric operands of the same type".to_owned(),
                        left: left_ty,
                        right: right_ty,
                        span,
                    });
                    return Type::Unknown;
                }
                Type::Trilean
            }
            BinaryOperator::And
            | BinaryOperator::Or
            | BinaryOperator::Xor
            | BinaryOperator::Iff
            | BinaryOperator::Implies
            | BinaryOperator::KleeneXor
            | BinaryOperator::KleeneIff
            | BinaryOperator::KleeneImplies => {
                let left_ok = left_ty.matches(&Type::Trilean);
                let right_ok = right_ty.matches(&Type::Trilean);
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
                Type::Trilean
            }
        }
    }

    fn check_unary_negate(&mut self, operand: ExprId, span: Span) -> Type {
        let ty = self.infer_expression(operand);
        if ty.is_numeric() || ty.matches(&Type::Trilean) || matches!(ty, Type::Unknown) {
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

    fn check_call(&mut self, callee: ExprId, arguments: &[ExprId], span: Span) -> Type {
        let callee_ty = self.infer_expression(callee);

        // Try function call first.
        if let Type::Function {
            parameters,
            return_type,
        } = callee_ty.clone()
        {
            if arguments.len() != parameters.len() {
                self.errors.push(TypeError::WrongArity {
                    expected: parameters.len(),
                    found: arguments.len(),
                    span,
                });
            }
            for (i, argument) in arguments.iter().enumerate() {
                let arg_ty = self.infer_expression(*argument);
                if let Some(expected) = parameters.get(i)
                    && !expected.matches(&arg_ty)
                {
                    self.errors.push(TypeError::Mismatch {
                        expected: expected.clone(),
                        found: arg_ty,
                        span: self.arena.expression(*argument).span.clone(),
                    });
                }
            }
            return *return_type;
        }

        // Try enum variant construction: `Some(5)`, `None`.
        if let Expr::Identifier(ref callee_name) =
            self.arena.expression(callee).node
            && let Some(enum_ty) = self.lookup_enum_variant(callee_name) {
                let Type::UserEnum { name: _enum_name, type_params, variants } = &enum_ty
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
                && variants.iter().any(|(n, _)| n == name) {
                    return Some(binding.ty.clone());
                }
        }
        None
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
        let Type::UserStruct { name: _, fields: def_fields, .. } = &ty else {
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
            let value_ty = self.infer_expression(*value);
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
        let Type::UserEnum { name: _, variants, .. } = &ty else {
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

    fn check_field_access(&mut self, object: ExprId, field: &str, span: Span) -> Type {
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

    fn check_safe_field_access(
        &mut self,
        object: ExprId,
        field: &str,
        span: Span,
    ) -> Type {
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
        then_branch: &Block,
        else_branch: Option<&Block>,
        treat_unknown_as_false: bool,
        span: Span,
    ) -> Type {
        let cond_ty = self.infer_expression(condition);
        let cond_span = self.arena.expression(condition).span.clone();
        self.check_condition_type(cond_ty, treat_unknown_as_false, cond_span);

        let then_ty = self.check_block(then_branch);

        match else_branch {
            None => Type::Unit,
            Some(block) => {
                let else_ty = self.check_block(block);
                if let Ok(unified) = try_unify(&then_ty, &else_ty) { unified } else {
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
        let mut arm_type: Option<Type> = None;

        for arm in arms {
            self.env.push_frame();
            self.bind_pattern(arm.pattern, &scrutinee_ty);
            if let Some(guard) = arm.guard {
                let guard_ty = self.infer_expression(guard);
                let guard_span = self.arena.expression(guard).span.clone();
                self.check_condition_type(guard_ty, false, guard_span);
            }
            let body_ty = self.infer_expression(arm.body);
            self.env.pop_frame();

            match &arm_type {
                None => arm_type = Some(body_ty),
                Some(expected) => {
                    match try_unify(expected, &body_ty) {
                        Ok(unified) => arm_type = Some(unified),
                        Err(()) => {
                            self.errors.push(TypeError::MatchArmMismatch {
                                expected: expected.clone(),
                                found: body_ty,
                                span: self.arena.expression(arm.body).span.clone(),
                            });
                        }
                    }
                }
            }
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

/// Map a `BinaryOperator` to its source-code symbol for diagnostics.
const fn operator_symbol(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Modulo => "%%",
        BinaryOperator::Power => "**",
        BinaryOperator::Equal => "==",
        BinaryOperator::NotEqual => "!=",
        BinaryOperator::LessThan => "<",
        BinaryOperator::LessEqual => "<=",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::GreaterEqual => ">=",
        BinaryOperator::And => "and",
        BinaryOperator::Or => "or",
        BinaryOperator::Xor => "xor",
        BinaryOperator::Iff => "iff",
        BinaryOperator::Implies => "implies",
        BinaryOperator::KleeneXor => "kleene_xor",
        BinaryOperator::KleeneIff => "kleene_iff",
        BinaryOperator::KleeneImplies => "kleene_implies",
    }
}

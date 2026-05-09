//! The actual type-checker: walks a `Program`, accumulates `TypeError`s.

use triet_syntax::{
    Arena, BinaryOperator, Block, Expr, ExprId, FStringPart, FunctionBody, FunctionDef, Item,
    MatchArm, NumericSuffix, Pattern, PatternId, Program, Span, Spanned, Stmt, StmtId, TypeExpr,
    TypeId, UnaryOperator,
};

use crate::{env::TypeEnvironment, error::TypeError, types::Type};

/// Type-check a `Program`, returning all errors found.
///
/// Returns an empty `Vec` on success. The checker accumulates errors
/// rather than aborting on the first one, so a single call can surface
/// every problem at once. `Type::Unknown` is used as a recovery
/// placeholder so cascading errors don't compound.
#[must_use]
pub fn check(program: &Program) -> Vec<TypeError> {
    let mut checker = Checker::new(program);
    checker.check_program();
    checker.errors
}

/// Type-checker state.
struct Checker<'p> {
    arena: &'p Arena,
    items: &'p [Spanned<Item>],
    env: TypeEnvironment,
    /// The function whose body is currently being checked (for return-
    /// type enforcement). `None` at top level.
    current_return_type: Option<Type>,
    errors: Vec<TypeError>,
}

impl<'p> Checker<'p> {
    fn new(program: &'p Program) -> Self {
        Self {
            arena: &program.arena,
            items: &program.items,
            env: TypeEnvironment::with_prelude(),
            current_return_type: None,
            errors: Vec::new(),
        }
    }

    fn check_program(&mut self) {
        // Pass 1: register every top-level function/const so calls and
        // references can resolve forward.
        for item in self.items {
            self.declare_item(item);
        }
        // Pass 2: check bodies.
        for item in self.items {
            self.check_item(item);
        }
    }

    // ====================================================================
    // Items
    // ====================================================================

    fn declare_item(&mut self, item: &Spanned<Item>) {
        match &item.node {
            Item::Function(def) => {
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| self.resolve_type(p.type_annotation))
                    .collect();
                let return_type = def
                    .return_type
                    .map_or(Type::Unit, |id| self.resolve_type(id));
                let function_type = Type::Function {
                    parameters,
                    return_type: Box::new(return_type),
                };
                self.declare_or_record_dup(&def.name, function_type, item.span.clone());
            }
            Item::Const {
                name,
                type_annotation,
                value,
                ..
            } => {
                let declared = type_annotation.map(|id| self.resolve_type(id));
                let inferred = self.infer_expression(*value);
                let ty = match declared {
                    Some(annotated) => {
                        if !annotated.matches(&inferred) {
                            self.errors.push(TypeError::Mismatch {
                                expected: annotated.clone(),
                                found: inferred,
                                span: self.arena.expression(*value).span.clone(),
                            });
                        }
                        annotated
                    }
                    None => inferred,
                };
                self.declare_or_record_dup(name, ty, item.span.clone());
            }
            Item::TypeAlias { .. } => {
                // V0.1: type aliases are accepted syntactically but the
                // checker does not yet expand them. Names registered in
                // declare_or_record_dup are not used as type names.
            }
            Item::Import(_) => {
                // V0.1 imports are syntactic placeholders.
            }
        }
    }

    fn check_item(&mut self, item: &Spanned<Item>) {
        if let Item::Function(def) = &item.node {
            self.check_function(def);
        }
    }

    fn check_function(&mut self, def: &FunctionDef) {
        let return_type = def
            .return_type
            .map_or(Type::Unit, |id| self.resolve_type(id));

        self.env.push_frame();
        self.current_return_type = Some(return_type.clone());

        for parameter in &def.parameters {
            let ty = self.resolve_type(parameter.type_annotation);
            self.env.declare(&parameter.name, ty);
        }

        match &def.body {
            FunctionBody::Block(block) => {
                let body_ty = self.check_block(block);
                if !return_type.matches(&body_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: return_type,
                        found: body_ty,
                        span: block_span(self.arena, block),
                    });
                }
            }
            FunctionBody::Expression(expr) => {
                let body_ty = self.infer_expression(*expr);
                if !return_type.matches(&body_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: return_type,
                        found: body_ty,
                        span: self.arena.expression(*expr).span.clone(),
                    });
                }
            }
        }

        self.current_return_type = None;
        self.env.pop_frame();
    }

    // ====================================================================
    // Statements / blocks
    // ====================================================================

    fn check_block(&mut self, block: &Block) -> Type {
        self.env.push_frame();
        for stmt_id in &block.statements {
            self.check_statement(*stmt_id);
        }
        let value_type = block
            .final_expression
            .map_or(Type::Unit, |id| self.infer_expression(id));
        self.env.pop_frame();
        value_type
    }

    fn check_statement(&mut self, id: StmtId) {
        let stmt = self.arena.statement(id).clone();
        match stmt.node {
            Stmt::Let {
                name,
                mutable,
                type_annotation,
                value,
            } => {
                let ty = self.check_initializer(type_annotation, value);
                self.env.declare_with_mut(&name, ty, mutable);
            }
            Stmt::Assign { target, value } => {
                self.check_assignment(&target, value, stmt.span.clone());
            }
            Stmt::Const {
                name,
                type_annotation,
                value,
            } => {
                let ty = self.check_initializer(type_annotation, value);
                self.env.declare(&name, ty);
            }
            Stmt::Return(value) => {
                let actual = value.map_or(Type::Unit, |id| self.infer_expression(id));
                if let Some(expected) = self.current_return_type.clone()
                    && !expected.matches(&actual)
                {
                    let span = value.map_or(stmt.span.clone(), |id| {
                        self.arena.expression(id).span.clone()
                    });
                    self.errors.push(TypeError::Mismatch {
                        expected,
                        found: actual,
                        span,
                    });
                }
            }
            Stmt::Break(value) => {
                // For v0.1, break-with-value is allowed only inside `loop`;
                // we don't track loop context here, so just type-check.
                if let Some(id) = value {
                    let _ = self.infer_expression(id);
                }
            }
            Stmt::Continue => {}
            Stmt::For {
                variable,
                iterable,
                body,
            } => {
                let iter_ty = self.infer_expression(iterable);
                let element_ty = match &iter_ty {
                    Type::Range(inner) => (**inner).clone(),
                    _ => Type::Unknown,
                };
                self.env.push_frame();
                self.bind_pattern(variable, &element_ty);
                let _ = self.check_block(&body);
                self.env.pop_frame();
            }
            Stmt::While {
                condition,
                body,
                treat_unknown_as_false,
            } => {
                let cond_ty = self.infer_expression(condition);
                let cond_span = self.arena.expression(condition).span.clone();
                self.check_condition_type(cond_ty, treat_unknown_as_false, cond_span);
                let _ = self.check_block(&body);
            }
            Stmt::Loop(body) => {
                let _ = self.check_block(&body);
            }
            Stmt::ExprStmt(expr) => {
                let _ = self.infer_expression(expr);
            }
        }
    }

    /// Shared logic for `let` / `const` initializers: resolve the
    /// optional annotation, infer the value, and verify they agree.
    /// Returns the binding's final type (annotation if present, else
    /// inferred). On mismatch, records a `Mismatch` error and falls
    /// back to the annotated type for downstream checking.
    fn check_initializer(&mut self, type_annotation: Option<TypeId>, value: ExprId) -> Type {
        let declared = type_annotation.map(|tid| self.resolve_type(tid));
        let inferred = self.infer_expression(value);
        match declared {
            Some(annotated) => {
                if !annotated.matches(&inferred) {
                    self.errors.push(TypeError::Mismatch {
                        expected: annotated.clone(),
                        found: inferred,
                        span: self.arena.expression(value).span.clone(),
                    });
                }
                annotated
            }
            None => inferred,
        }
    }

    fn check_assignment(&mut self, target: &str, value: ExprId, stmt_span: Span) {
        let value_ty = self.infer_expression(value);
        let value_span = self.arena.expression(value).span.clone();
        let Some(binding) = self.env.lookup_binding(target).cloned() else {
            self.errors.push(TypeError::UndefinedName {
                name: target.to_owned(),
                span: stmt_span,
            });
            return;
        };
        if !binding.mutable {
            self.errors.push(TypeError::AssignToImmutable {
                name: target.to_owned(),
                span: stmt_span,
            });
        }
        if !binding.ty.matches(&value_ty) {
            self.errors.push(TypeError::Mismatch {
                expected: binding.ty,
                found: value_ty,
                span: value_span,
            });
        }
    }

    // ====================================================================
    // Expressions
    // ====================================================================

    fn infer_expression(&mut self, id: ExprId) -> Type {
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
                self.errors.push(TypeError::NullLiteralInNonNullableContext { span });
                Type::Unknown
            }
            Expr::Identifier(name) => self.env.lookup(&name).cloned().unwrap_or_else(|| {
                self.errors.push(TypeError::UndefinedName { name, span });
                Type::Unknown
            }),
            Expr::BinaryOp { operator, left, right } => self.check_binary_op(operator, left, right, span),
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
            Expr::If { condition, then_branch, else_branch, treat_unknown_as_false } => {
                self.check_if(condition, &then_branch, else_branch.as_ref(), treat_unknown_as_false, span)
            }
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
        let Type::Function {
            parameters,
            return_type,
        } = callee_ty.clone()
        else {
            if !matches!(callee_ty, Type::Unknown) {
                self.errors.push(TypeError::NotCallable {
                    found: callee_ty,
                    span,
                });
            }
            return Type::Unknown;
        };

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

        *return_type
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

    fn check_field_access(&mut self, object: ExprId, field: &str, span: Span) -> Type {
        let object_ty = self.infer_expression(object);
        if matches!(object_ty, Type::Unknown) {
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
                if !then_ty.matches(&else_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: then_ty.clone(),
                        found: else_ty,
                        span,
                    });
                }
                then_ty
            }
        }
    }

    fn check_condition_type(
        &mut self,
        cond_ty: Type,
        treat_unknown_as_false: bool,
        span: Span,
    ) {
        match cond_ty {
            Type::Trilean | Type::Unknown => {
                // Plain `if` requires a definite Trilean. The checker
                // can't tell statically whether a Trilean is "always
                // known", so we accept any Trilean here and rely on
                // `if?` for explicit unknown handling. A future pass
                // could refine this.
                let _ = treat_unknown_as_false;
            }
            other => {
                self.errors.push(TypeError::NonTrileanCondition {
                    found: other,
                    span,
                });
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
                    if !expected.matches(&body_ty) {
                        self.errors.push(TypeError::MatchArmMismatch {
                            expected: expected.clone(),
                            found: body_ty,
                            span: self.arena.expression(arm.body).span.clone(),
                        });
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

    // ====================================================================
    // Patterns
    // ====================================================================

    fn bind_pattern(&mut self, id: PatternId, scrutinee: &Type) {
        let pattern = self.arena.pattern(id).node.clone();
        match pattern {
            Pattern::Wildcard | Pattern::Null => {}
            Pattern::Variable(name) => {
                self.env.declare(&name, scrutinee.clone());
            }
            Pattern::Tuple(children) => {
                if let Type::Tuple(elements) = scrutinee {
                    for (child, element_type) in children.iter().zip(elements.iter()) {
                        self.bind_pattern(*child, element_type);
                    }
                } else {
                    for child in children {
                        self.bind_pattern(child, &Type::Unknown);
                    }
                }
            }
            Pattern::Or(alternatives) => {
                // Each alternative shares the same scrutinee shape; we
                // only bind from the first to avoid binding the same
                // variable to potentially differing types.
                if let Some(first) = alternatives.first() {
                    self.bind_pattern(*first, scrutinee);
                }
            }
            Pattern::Range { .. } | Pattern::Literal(_) => {}
        }
    }

    // ====================================================================
    // Type-expression resolution + helpers
    // ====================================================================

    fn resolve_type(&mut self, id: TypeId) -> Type {
        let span = self.arena.type_expression(id).span.clone();
        match self.arena.type_expression(id).node.clone() {
            TypeExpr::Named(name) => match name.as_str() {
                "Trit" => Type::Trit,
                "Tryte" => Type::Tryte,
                "Integer" => Type::Integer,
                "Long" => Type::Long,
                "Trilean" => Type::Trilean,
                "String" => Type::String,
                "Unit" => Type::Unit,
                _ => {
                    self.errors.push(TypeError::UnknownType { name, span });
                    Type::Unknown
                }
            },
            TypeExpr::Generic { name, .. } => {
                // V0.1 has no user-defined generics; flag as unknown
                // unless we add a built-in like Option<T> later.
                self.errors.push(TypeError::UnknownType { name, span });
                Type::Unknown
            }
            TypeExpr::Tuple(elements) => {
                Type::Tuple(elements.iter().map(|t| self.resolve_type(*t)).collect())
            }
            TypeExpr::Nullable(inner) => Type::Nullable(Box::new(self.resolve_type(inner))),
            TypeExpr::Function {
                parameters,
                return_type,
            } => Type::Function {
                parameters: parameters.iter().map(|t| self.resolve_type(*t)).collect(),
                return_type: Box::new(self.resolve_type(return_type)),
            },
        }
    }

    fn declare_or_record_dup(&mut self, name: &str, ty: Type, span: Span) {
        if !self.env.declare(name, ty) {
            self.errors.push(TypeError::DuplicateName {
                name: name.to_owned(),
                span,
            });
        }
    }
}

// ====================================================================
// Free helpers
// ====================================================================

fn block_span(arena: &Arena, block: &Block) -> Span {
    if let Some(id) = block.final_expression {
        arena.expression(id).span.clone()
    } else if let Some(stmt_id) = block.statements.last() {
        arena.statement(*stmt_id).span.clone()
    } else {
        0..0
    }
}

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

/// Built-in method type rules for the v0.1 prelude. Returns the
/// method's *return type* if `(receiver, method, arity)` matches a
/// known built-in; otherwise `None` (which the caller turns into an
/// `UnknownMember` error).
fn builtin_method_type(receiver: &Type, method: &str, arity: usize) -> Option<Type> {
    use Type::{Integer, Long, String, Trilean, Tryte};
    match (receiver, method, arity) {
        (Tryte, "to_integer", 0) => Some(Integer),
        (Tryte, "to_long", 0) => Some(Long),
        (Integer, "to_tryte", 0) => Some(Tryte),
        (Integer, "to_long", 0) => Some(Long),
        (Long, "to_integer", 0) => Some(Integer),
        (Long, "to_tryte", 0) => Some(Tryte),

        // Try-conversions return Nullable<T>.
        (Tryte | Long, "try_to_integer", 0) => {
            Some(Type::Nullable(Box::new(Integer)))
        }
        (Integer | Long, "try_to_tryte", 0) => {
            Some(Type::Nullable(Box::new(Tryte)))
        }
        (Integer | Tryte, "try_to_long", 0) => {
            Some(Type::Nullable(Box::new(Long)))
        }

        // Overflow-handled arithmetic — must match self-type.
        (Tryte, "add_and_truncate" | "add_and_saturate" | "subtract_and_truncate"
                | "subtract_and_saturate" | "multiply_and_truncate"
                | "multiply_and_saturate", 1) => Some(Tryte),
        (Integer, "add_and_truncate" | "add_and_saturate" | "subtract_and_truncate"
                  | "subtract_and_saturate" | "multiply_and_truncate"
                  | "multiply_and_saturate", 1) => Some(Integer),
        (Tryte, "try_add" | "try_subtract" | "try_multiply" | "try_divide"
              | "try_modulo", 1) => Some(Type::Nullable(Box::new(Tryte))),
        (Integer, "try_add" | "try_subtract" | "try_multiply" | "try_divide"
                | "try_modulo", 1) => Some(Type::Nullable(Box::new(Integer))),

        // Trilean
        (Trilean, "to_trit", 0) => Some(Type::Trit),
        (Trilean, "assume_known", 0) => Some(Trilean),

        // String
        (String, "length", 0) => Some(Integer),

        // Range — no methods in v0.1 yet; iteration handled by `for`.
        _ => None,
    }
}


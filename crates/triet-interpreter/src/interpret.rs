//! Tree-walking interpreter.

use std::{collections::HashMap, rc::Rc};

use triet_core::{Integer, Long, Tryte};
use triet_logic::Trilean;
use triet_syntax::{
    Arena, BinaryOperator, Block, Expr, ExprId, FStringPart, FunctionBody, Item, MatchArm,
    PatternId, Program, Span, Stmt, StmtId, TrileanValue, UnaryOperator,
};

use crate::{
    env::ValueEnvironment,
    error::RuntimeError,
    value::{Closure, FunctionRef, Value},
};

/// Result of evaluating a statement / block — one of: a value (block
/// final-expression), or a control-flow signal that propagates up
/// through the call stack.
#[derive(Debug)]
enum Outcome {
    Value(Value),
    Return(Value),
    Break(Value),
    Continue,
}

/// Run a `Program` by calling its `main` function with no arguments.
///
/// # Errors
///
/// Returns a [`RuntimeError`] on runtime failure (panic, unknown
/// condition, missing main, etc.).
pub fn run(program: &Program) -> Result<Value, RuntimeError> {
    let mut interpreter = Interpreter::new(program);
    interpreter.install_program_items();
    let main = interpreter
        .env
        .lookup("main")
        .cloned()
        .ok_or(RuntimeError::NoMainFunction)?;
    interpreter.invoke(&main, Vec::new(), 0..0)
}

/// Internal entry point for `interpret_resolved.rs`.
pub(crate) fn run_resolved_internal(
    program: &triet_modules::ResolvedProgram,
    globals: std::collections::HashMap<triet_modules::AbsolutePath, Value>,
) -> Result<Value, RuntimeError> {
    let mut interpreter = Interpreter::new_for_module(program, program.root, &globals);
    let main = interpreter
        .env
        .lookup("main")
        .cloned()
        .ok_or(RuntimeError::NoMainFunction)?;
    interpreter.invoke(&main, Vec::new(), 0..0)
}

/// Call a top-level function by name, passing positional arguments.
///
/// Useful for tests that exercise specific functions without needing
/// `main`.
///
/// # Errors
///
/// Returns a [`RuntimeError`] on runtime failure.
pub fn call_function(
    program: &Program,
    name: &str,
    arguments: Vec<Value>,
) -> Result<Value, RuntimeError> {
    let mut interpreter = Interpreter::new(program);
    interpreter.install_program_items();
    let function =
        interpreter
            .env
            .lookup(name)
            .cloned()
            .ok_or_else(|| RuntimeError::UndefinedName {
                name: name.to_owned(),
                span: 0..0,
            })?;
    interpreter.invoke(&function, arguments, 0..0)
}

pub(crate) struct Interpreter<'p> {
    arena: &'p Arena,
    items: &'p [triet_syntax::Spanned<Item>],
    env: ValueEnvironment,
    resolved_program: Option<&'p triet_modules::ResolvedProgram>,
    resolved_globals: Option<&'p std::collections::HashMap<triet_modules::AbsolutePath, Value>>,
    /// The module currently executing — used for cross-module variant
    /// lookups (`from std.result import Ok` style).
    current_module_id: Option<triet_modules::ModuleId>,
}

impl<'p> Interpreter<'p> {
    pub(crate) fn new(program: &'p Program) -> Self {
        let mut env = ValueEnvironment::new();
        crate::builtins::install(&mut env);
        Self {
            arena: &program.arena,
            items: &program.items,
            env,
            resolved_program: None,
            resolved_globals: None,
            current_module_id: None,
        }
    }

    pub(crate) fn new_for_module(
        program: &'p triet_modules::ResolvedProgram,
        module_id: triet_modules::ModuleId,
        globals: &'p std::collections::HashMap<triet_modules::AbsolutePath, Value>,
    ) -> Self {
        let module = program.module(module_id);
        let mut env = ValueEnvironment::new();
        crate::builtins::install(&mut env);

        // Seed environment with globals available to this module.
        // 1. Module's own items.
        for item in &module.items {
            if let Item::Function(def) = &item.node {
                let path = triet_modules::AbsolutePath::new(module.path.clone(), def.name.clone());
                if let Some(val) = globals.get(&path) {
                    env.declare(&def.name, val.clone());
                }
            } else if let Item::Const { name, .. } = &item.node {
                let path = triet_modules::AbsolutePath::new(module.path.clone(), name.clone());
                if let Some(val) = globals.get(&path) {
                    env.declare(name, val.clone());
                }
            }
        }

        // 2. Imported bindings.
        for (local_name, abs_path) in &module.bindings {
            // Stdlib has no AbsolutePath in `globals` yet, we use builtins.
            if abs_path.module_path().root() == Some("std") {
                // Stdlib functions are handled by `builtins::install()`,
                // but if we aliased them (`from std.io import println as p`),
                // we should map them here. We can just lookup `abs_path.name()`
                // in the `env` if it's already installed by builtins.
                if let Some(val) = env.lookup(abs_path.name()).cloned() {
                    env.declare(local_name, val);
                }
                continue;
            }
            if let Some(val) = globals.get(abs_path) {
                env.declare(local_name, val.clone());
            }
        }

        Self {
            arena: program.arena(module),
            items: &module.items,
            env,
            resolved_program: Some(program),
            resolved_globals: Some(globals),
            current_module_id: Some(module_id),
        }
    }

    pub(crate) fn evaluate_expression_public(&mut self, id: ExprId) -> Result<Value, RuntimeError> {
        self.evaluate_expression(id)
    }

    /// Bind every top-level item into the environment so that calls to
    /// any function (forward or backward) resolve.
    fn install_program_items(&mut self) {
        for item in self.items {
            match &item.node {
                Item::Function(def) => {
                    let function = Value::Function(Rc::new(FunctionRef {
                        def: def.clone(),
                        module_id: None,
                    }));
                    self.env.declare(&def.name, function);
                }
                Item::Const { name, value, .. } => {
                    // Evaluate const initializer eagerly. Errors here
                    // propagate; the type checker should have caught
                    // most issues already.
                    if let Ok(constant) = self.evaluate_expression(*value) {
                        self.env.declare(name, constant);
                    }
                }
                Item::TypeAlias { .. }
                | Item::Import(_)
                | Item::ImportFrom(_)
                | Item::Struct(_)
                | Item::Enum(_) => {
                    // No runtime effect at this stage. The module loader
                    // (v0.2.x.6) will materialize import bindings; for now
                    // they're parser-only AST.
                }
                Item::Module(_) => {
                    // Module declarations are parser-level only until the
                    // module loader (v0.2.x.6) lands. Inline submodules
                    // are not yet recursed into; file-bound modules are
                    // not yet resolved.
                }
            }
        }
    }

    // ====================================================================
    // Statements
    // ====================================================================

    fn execute_block(&mut self, block: &Block) -> Result<Outcome, RuntimeError> {
        self.env.push_frame();
        for &stmt_id in &block.statements {
            match self.execute_statement(stmt_id)? {
                Outcome::Value(_) => {}
                other => {
                    self.env.pop_frame();
                    return Ok(other);
                }
            }
        }
        let outcome = match block.final_expression {
            Some(id) => Outcome::Value(self.evaluate_expression(id)?),
            None => Outcome::Value(Value::Unit),
        };
        self.env.pop_frame();
        Ok(outcome)
    }

    fn execute_statement(&mut self, id: StmtId) -> Result<Outcome, RuntimeError> {
        let stmt = self.arena.statement(id).clone();
        match stmt.node {
            Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
                let computed = self.evaluate_expression(value)?;
                self.env.declare(&name, computed);
                Ok(Outcome::Value(Value::Unit))
            }
            Stmt::Assign { target, value } => {
                let computed = self.evaluate_expression(value)?;
                if !self.env.assign(&target, computed) {
                    return Err(RuntimeError::UndefinedName {
                        name: target,
                        span: stmt.span,
                    });
                }
                Ok(Outcome::Value(Value::Unit))
            }
            Stmt::Return(value) => {
                let result = match value {
                    Some(id) => self.evaluate_expression(id)?,
                    None => Value::Unit,
                };
                Ok(Outcome::Return(result))
            }
            Stmt::Break(value) => {
                let result = match value {
                    Some(id) => self.evaluate_expression(id)?,
                    None => Value::Unit,
                };
                Ok(Outcome::Break(result))
            }
            Stmt::Continue => Ok(Outcome::Continue),
            Stmt::For {
                variable,
                iterable,
                body,
            } => self.execute_for(variable, iterable, &body, stmt.span),
            Stmt::While {
                condition,
                body,
                treat_unknown_as_false,
            } => self.execute_while(condition, &body, treat_unknown_as_false, stmt.span),
            Stmt::Loop(body) => self.execute_loop(&body),
            Stmt::ExprStmt(expr) => {
                let _ = self.evaluate_expression(expr)?;
                Ok(Outcome::Value(Value::Unit))
            }
        }
    }

    fn execute_for(
        &mut self,
        variable: PatternId,
        iterable: ExprId,
        body: &Block,
        span: Span,
    ) -> Result<Outcome, RuntimeError> {
        let mut iter_value = self.evaluate_expression(iterable)?;
        while let Some(element) = advance_iterator(&mut iter_value, &span)? {
            self.env.push_frame();
            self.bind_pattern(variable, &element);
            let outcome = self.execute_block(body)?;
            self.env.pop_frame();
            match outcome {
                Outcome::Value(_) | Outcome::Continue => {}
                Outcome::Break(_) => break,
                ret @ Outcome::Return(_) => return Ok(ret),
            }
        }
        Ok(Outcome::Value(Value::Unit))
    }

    fn execute_while(
        &mut self,
        condition: ExprId,
        body: &Block,
        treat_unknown_as_false: bool,
        span: Span,
    ) -> Result<Outcome, RuntimeError> {
        loop {
            let cond_value = self.evaluate_expression(condition)?;
            let Value::Trilean(trilean) = cond_value else {
                return Err(RuntimeError::TypeError {
                    message: "while condition must be Trilean".to_owned(),
                    span,
                });
            };
            let proceed = match trilean {
                Trilean::True => true,
                Trilean::False => false,
                Trilean::Unknown => {
                    if treat_unknown_as_false {
                        false
                    } else {
                        return Err(RuntimeError::UnknownCondition { span });
                    }
                }
            };
            if !proceed {
                break;
            }
            match self.execute_block(body)? {
                Outcome::Value(_) | Outcome::Continue => {}
                Outcome::Break(_) => break,
                ret @ Outcome::Return(_) => return Ok(ret),
            }
        }
        Ok(Outcome::Value(Value::Unit))
    }

    fn execute_loop(&mut self, body: &Block) -> Result<Outcome, RuntimeError> {
        loop {
            match self.execute_block(body)? {
                Outcome::Value(_) | Outcome::Continue => {}
                Outcome::Break(value) => return Ok(Outcome::Value(value)),
                ret @ Outcome::Return(_) => return Ok(ret),
            }
        }
    }

    // ====================================================================
    // Expressions
    // ====================================================================

    fn evaluate_expression(&mut self, id: ExprId) -> Result<Value, RuntimeError> {
        let span = self.arena.expression(id).span.clone();
        let node = self.arena.expression(id).node.clone();
        match node {
            Expr::IntegerLiteral { value, suffix } => {
                Ok(integer_literal_value(value, suffix, &span)?)
            }
            Expr::TernaryLiteral { value } => Ok(integer_literal_value(value, None, &span)?),
            Expr::TrileanLiteral(value) => Ok(Value::Trilean(match value {
                TrileanValue::False => Trilean::False,
                TrileanValue::Unknown => Trilean::Unknown,
                TrileanValue::True => Trilean::True,
            })),
            Expr::StringLiteral(text) => Ok(Value::from_string(text)),
            Expr::FStringLiteral(segments) => {
                let mut output = String::new();
                for part in &segments.parts {
                    match part {
                        FStringPart::Text(text) => output.push_str(text),
                        FStringPart::Interpolation { expression, .. } => {
                            let value = self.evaluate_expression(*expression)?;
                            output.push_str(&value.to_string());
                        }
                    }
                }
                Ok(Value::from_string(output))
            }
            Expr::NullLiteral => Ok(Value::Null),
            Expr::Identifier(name) => {
                match self.env.lookup(&name).cloned() {
                    Some(value) => Ok(value),
                    None => {
                        // Try enum unit variant: `None`.
                        if let Some((enum_name, variant_name)) =
                            self.find_enum_variant_for_construction(&name)
                        {
                            Ok(Value::EnumVariant {
                                name: enum_name,
                                variant: variant_name,
                                payload: None,
                            })
                        } else {
                            Err(RuntimeError::UndefinedName { name, span })
                        }
                    }
                }
            }
            Expr::BinaryOp {
                operator,
                left,
                right,
            } => self.evaluate_binary(operator, left, right, span),
            Expr::UnaryOp {
                operator: UnaryOperator::Negate,
                operand,
            } => self.evaluate_negate(operand, span),
            Expr::Call { callee, arguments } => self.evaluate_call(callee, &arguments, span),
            Expr::MethodCall {
                receiver,
                method,
                arguments,
            } => self.evaluate_method_call(receiver, &method, &arguments, span),
            Expr::FieldAccess { object, field } => {
                let value = self.evaluate_expression(object)?;
                match value {
                    Value::Struct { fields, .. } => {
                        fields
                            .get(&field)
                            .cloned()
                            .ok_or_else(|| RuntimeError::TypeError {
                                message: format!("no field `{field}`"),
                                span: span.clone(),
                            })
                    }
                    _ => Err(RuntimeError::TypeError {
                        message: "field access on non-struct value".to_string(),
                        span,
                    }),
                }
            }
            Expr::TupleIndex { tuple, index } => {
                let value = self.evaluate_expression(tuple)?;
                let Value::Tuple(elements) = value else {
                    return Err(RuntimeError::TypeError {
                        message: "tuple index on non-tuple".to_owned(),
                        span,
                    });
                };
                elements.get(index).cloned().ok_or(RuntimeError::Panic {
                    message: format!("tuple index {index} out of range"),
                    span,
                })
            }
            Expr::SafeFieldAccess { object, field } => {
                let value = self.evaluate_expression(object)?;
                if value.is_null() {
                    Ok(Value::Null)
                } else {
                    Err(RuntimeError::TypeError {
                        message: format!("no field `{field}`"),
                        span,
                    })
                }
            }
            Expr::SafeMethodCall {
                receiver,
                method,
                arguments,
            } => {
                let value = self.evaluate_expression(receiver)?;
                if value.is_null() {
                    return Ok(Value::Null);
                }
                let mut arg_values = Vec::with_capacity(arguments.len());
                for &arg in &arguments {
                    arg_values.push(self.evaluate_expression(arg)?);
                }
                self.dispatch_method(value, &method, arg_values, span)
            }
            Expr::ElvisOp { object, default } => {
                let value = self.evaluate_expression(object)?;
                if value.is_null() {
                    self.evaluate_expression(default)
                } else {
                    Ok(value)
                }
            }
            Expr::ForceUnwrap(inner) => {
                let value = self.evaluate_expression(inner)?;
                if value.is_null() {
                    Err(RuntimeError::Panic {
                        message: "force-unwrap (`!!`) on null".to_owned(),
                        span,
                    })
                } else {
                    Ok(value)
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
                treat_unknown_as_false,
            } => self.evaluate_if(
                condition,
                &then_branch,
                else_branch.as_ref(),
                treat_unknown_as_false,
                span,
            ),
            Expr::Match { scrutinee, arms } => self.evaluate_match(scrutinee, &arms, span),
            Expr::Block(block) => match self.execute_block(&block)? {
                Outcome::Value(value) => Ok(value),
                _ => Err(RuntimeError::Panic {
                    message: "block produced control-flow outside its enclosing function/loop"
                        .to_owned(),
                    span,
                }),
            },
            Expr::Tuple(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for &element in &elements {
                    values.push(self.evaluate_expression(element)?);
                }
                Ok(Value::from_tuple(values))
            }
            Expr::Lambda {
                parameters,
                return_type,
                body,
            } => {
                let captured = self.env.clone();
                Ok(Value::Lambda(Rc::new(Closure {
                    parameters,
                    return_type,
                    body,
                    captured_env: captured,
                })))
            }
            Expr::Range {
                start,
                end,
                inclusive,
            } => {
                let start_value = self.evaluate_expression(start)?;
                let end_value = self.evaluate_expression(end)?;
                let (Value::Integer(s), Value::Integer(e)) = (start_value, end_value) else {
                    return Err(RuntimeError::TypeError {
                        message: "range bounds must be Integer".to_owned(),
                        span,
                    });
                };
                Ok(Value::Range {
                    start: s,
                    end: e,
                    inclusive,
                })
            }
            Expr::StructLiteral { name, fields } => {
                let mut field_values = HashMap::new();
                for (field_name, value_expr) in fields {
                    let val = self.evaluate_expression(value_expr)?;
                    field_values.insert(field_name.clone(), val);
                }
                Ok(Value::Struct {
                    name,
                    fields: field_values,
                })
            }
            Expr::EnumLiteral {
                name,
                variant_name,
                payload,
            } => {
                let payload_value = match payload {
                    Some(expr) => Some(Box::new(self.evaluate_expression(expr)?)),
                    None => None,
                };
                Ok(Value::EnumVariant {
                    name,
                    variant: variant_name,
                    payload: payload_value,
                })
            }
        }
    }

    fn evaluate_binary(
        &mut self,
        operator: BinaryOperator,
        left: ExprId,
        right: ExprId,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let lhs = self.evaluate_expression(left)?;
        let rhs = self.evaluate_expression(right)?;
        execute_binary(operator, lhs, rhs, span)
    }

    fn evaluate_negate(&mut self, operand: ExprId, span: Span) -> Result<Value, RuntimeError> {
        let value = self.evaluate_expression(operand)?;
        match value {
            Value::Trit(t) => Ok(Value::Trit(-t)),
            Value::Tryte(t) => Ok(Value::Tryte(-t)),
            Value::Integer(i) => Ok(Value::Integer(-i)),
            Value::Long(l) => Ok(Value::Long(-l)),
            Value::Trilean(t) => Ok(Value::Trilean(t.not())),
            other => Err(RuntimeError::TypeError {
                message: format!("cannot negate {other}"),
                span,
            }),
        }
    }

    fn evaluate_call(
        &mut self,
        callee: ExprId,
        arguments: &[ExprId],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // Enum variant construction: `Some(5)`, `None`.
        if let Expr::Identifier(ref name) = self.arena.expression(callee).node {
            if let Some(enum_ty) = self.env.lookup(name).cloned()
                && let Value::EnumVariant { .. } = &enum_ty
            {
                // The name resolves to an enum variant value — this
                // shouldn't happen (variants aren't stored as values).
            }
            // Check if `name` is a variant of any registered enum.
            if let Some((enum_name, variant_name)) = self.find_enum_variant_for_construction(name) {
                let payload = if arguments.is_empty() {
                    None
                } else {
                    Some(Box::new(self.evaluate_expression(arguments[0])?))
                };
                return Ok(Value::EnumVariant {
                    name: enum_name,
                    variant: variant_name,
                    payload,
                });
            }
        }

        let function = self.evaluate_expression(callee)?;
        let mut arg_values = Vec::with_capacity(arguments.len());
        for &arg in arguments {
            arg_values.push(self.evaluate_expression(arg)?);
        }
        self.invoke(&function, arg_values, span)
    }

    /// Scan registered enum types for a variant named `name`. Returns
    /// `(enum_type_name, variant_name)` if found.
    ///
    /// Two scopes are searched in order:
    /// 1. Current module's own enum declarations.
    /// 2. Cross-module variant imports — when the current module's
    ///    bindings include `name → AbsolutePath(other_module, Enum)`
    ///    (from `from X import Variant`), we follow the path to the
    ///    enum's variant list. Aliased variant imports are rejected
    ///    at resolver time (E2107), so `name` matches both the local
    ///    binding key and the variant spelling.
    fn find_enum_variant_for_construction(&self, name: &str) -> Option<(String, String)> {
        for item in self.items {
            if let Item::Enum(def) = &item.node {
                for variant in &def.variants {
                    if variant.name == name {
                        return Some((def.name.clone(), variant.name.clone()));
                    }
                }
            }
        }

        // Cross-module variant imports — only attempted in multi-module
        // mode, when `new_for_module` set up the resolved-program ref.
        let program = self.resolved_program?;
        let module_id = self.current_module_id?;
        let module = program.module(module_id);
        let abs_path = module.bindings.get(name)?;
        let source_id = program.find_module(abs_path.module_path())?;
        let source_module = program.module(source_id);
        let source_arena = program.arena(source_module);
        for item in &source_module.items {
            if let Item::Enum(def) = &item.node
                && def.name == abs_path.name()
            {
                for variant in &def.variants {
                    if variant.name == name {
                        // `source_arena` reference suppresses unused-binding
                        // warnings when no payload introspection is needed.
                        let _ = source_arena;
                        return Some((def.name.clone(), variant.name.clone()));
                    }
                }
            }
        }
        None
    }

    fn evaluate_method_call(
        &mut self,
        receiver: ExprId,
        method: &str,
        arguments: &[ExprId],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let receiver_value = self.evaluate_expression(receiver)?;
        let mut arg_values = Vec::with_capacity(arguments.len());
        for &arg in arguments {
            arg_values.push(self.evaluate_expression(arg)?);
        }
        self.dispatch_method(receiver_value, method, arg_values, span)
    }

    fn dispatch_method(
        &mut self,
        receiver: Value,
        method: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match (&receiver, method) {
            // Integer ↔ Tryte conversions
            (Value::Integer(i), "to_tryte") => {
                let value = i.to_i64();
                Tryte::new(i16::try_from(value).unwrap_or(i16::MAX))
                    .map(Value::Tryte)
                    .ok_or(RuntimeError::Panic {
                        message: "Integer → Tryte overflow".to_owned(),
                        span,
                    })
            }
            (Value::Tryte(t), "to_integer") => Integer::new(i64::from(t.to_i16()))
                .map(Value::Integer)
                .ok_or(RuntimeError::Panic {
                    message: "Tryte → Integer overflow".to_owned(),
                    span,
                }),

            // Long widening (always lossless)
            (Value::Tryte(t), "to_long") => Ok(Value::Long(Long::from(*t))),
            (Value::Integer(i), "to_long") => Ok(Value::Long(Long::from(*i))),

            // Long narrowing (panic on overflow, with try / saturate variants)
            (Value::Long(l), "to_integer") => {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.to_integer()))
                    .map(Value::Integer)
                    .map_err(|_| RuntimeError::Panic {
                        message: "Long → Integer overflow".to_owned(),
                        span: span.clone(),
                    })
            }
            (Value::Long(l), "to_tryte") => {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.to_tryte()))
                    .map(Value::Tryte)
                    .map_err(|_| RuntimeError::Panic {
                        message: "Long → Tryte overflow".to_owned(),
                        span: span.clone(),
                    })
            }
            (Value::Long(l), "to_integer_and_saturate") => {
                Ok(Value::Integer(l.to_integer_and_saturate()))
            }
            (Value::Long(l), "try_to_integer") => {
                Ok(l.try_to_integer().map_or(Value::Null, Value::Integer))
            }
            (Value::Long(l), "try_to_tryte") => {
                Ok(l.try_to_tryte().map_or(Value::Null, Value::Tryte))
            }

            // Tryte/Integer → Long with try_* (always Some, but exposed
            // for API symmetry with `try_to_*` narrowing methods).
            (Value::Tryte(t), "try_to_long") => Ok(Value::Long(Long::from(*t))),
            (Value::Integer(i), "try_to_long") => Ok(Value::Long(Long::from(*i))),

            // Long overflow-aware arithmetic — mirrors Integer's surface.
            (Value::Long(l), "add_and_truncate") => {
                binary_long_method(l, arguments, Long::add_and_truncate, span)
            }
            (Value::Long(l), "add_and_saturate") => {
                binary_long_method(l, arguments, Long::add_and_saturate, span)
            }
            (Value::Long(l), "try_add") => {
                binary_long_try_method(l, arguments, Long::try_add, span)
            }
            (Value::Long(l), "subtract_and_truncate") => {
                binary_long_method(l, arguments, Long::subtract_and_truncate, span)
            }
            (Value::Long(l), "subtract_and_saturate") => {
                binary_long_method(l, arguments, Long::subtract_and_saturate, span)
            }
            (Value::Long(l), "try_subtract") => {
                binary_long_try_method(l, arguments, Long::try_subtract, span)
            }
            (Value::Long(l), "multiply_and_truncate") => {
                binary_long_method(l, arguments, Long::multiply_and_truncate, span)
            }
            (Value::Long(l), "multiply_and_saturate") => {
                binary_long_method(l, arguments, Long::multiply_and_saturate, span)
            }
            (Value::Long(l), "try_multiply") => {
                binary_long_try_method(l, arguments, Long::try_multiply, span)
            }

            // Integer overflow-aware arithmetic
            (Value::Integer(i), "add_and_truncate") => {
                binary_int_method(i, arguments, triet_core::Integer::add_and_truncate, span)
            }
            (Value::Integer(i), "add_and_saturate") => {
                binary_int_method(i, arguments, triet_core::Integer::add_and_saturate, span)
            }
            (Value::Integer(i), "try_add") => {
                binary_int_try_method(i, arguments, triet_core::Integer::try_add, span)
            }
            (Value::Integer(i), "subtract_and_truncate") => binary_int_method(
                i,
                arguments,
                triet_core::Integer::subtract_and_truncate,
                span,
            ),
            (Value::Integer(i), "subtract_and_saturate") => binary_int_method(
                i,
                arguments,
                triet_core::Integer::subtract_and_saturate,
                span,
            ),

            // Trilean
            (Value::Trilean(t), "assume_known") => match t {
                Trilean::Unknown => Err(RuntimeError::Panic {
                    message: "assume_known() on Trilean::Unknown".to_owned(),
                    span,
                }),
                known => Ok(Value::Trilean(*known)),
            },
            (Value::Trilean(t), "to_trit") => Ok(Value::Trit(t.to_trit())),

            // Iterable adapters — `.enumerate()` produces `(index, item)`
            // tuples. V0.1 only enumerates ranges; nesting works because
            // the runtime advances iterators recursively.
            (Value::Range { .. } | Value::Enumerate { .. }, "enumerate") => Ok(Value::Enumerate {
                inner: Box::new(receiver.clone()),
                next_index: 0,
            }),

            // String
            (Value::String(text), "length") => {
                let chars = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
                Integer::new(chars)
                    .map(Value::Integer)
                    .ok_or(RuntimeError::Panic {
                        message: "string length exceeds Integer range".to_owned(),
                        span,
                    })
            }

            (other, name) => Err(RuntimeError::TypeError {
                message: format!("no method `{name}` on value {other}"),
                span,
            }),
        }
    }

    fn invoke(
        &mut self,
        function: &Value,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match function {
            Value::Function(reference) => self.invoke_function(reference, arguments, span),
            Value::Lambda(closure) => self.invoke_closure(closure, arguments, span),
            Value::Builtin(builtin) => builtin(&arguments),
            other => Err(RuntimeError::TypeError {
                message: format!("not callable: {other}"),
                span,
            }),
        }
    }

    fn invoke_function(
        &mut self,
        reference: &Rc<FunctionRef>,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let def = &reference.def;
        if arguments.len() != def.parameters.len() {
            return Err(RuntimeError::WrongArity {
                expected: def.parameters.len(),
                found: arguments.len(),
                span,
            });
        }

        // Save current context
        let saved_env = std::mem::replace(&mut self.env, ValueEnvironment::new());
        let saved_arena = self.arena;
        let saved_items = self.items;

        if let (Some(resolved), Some(globals), Some(mod_id)) = (
            self.resolved_program,
            self.resolved_globals,
            reference.module_id,
        ) {
            // Module-aware call: switch arena, items, and populate env from globals.
            let module = resolved.module(mod_id);
            self.arena = resolved.arena(module);
            self.items = &module.items;
            crate::builtins::install(&mut self.env);

            for item in self.items {
                if let Item::Function(def) = &item.node {
                    let path =
                        triet_modules::AbsolutePath::new(module.path.clone(), def.name.clone());
                    if let Some(val) = globals.get(&path) {
                        self.env.declare(&def.name, val.clone());
                    }
                } else if let Item::Const { name, .. } = &item.node {
                    let path = triet_modules::AbsolutePath::new(module.path.clone(), name.clone());
                    if let Some(val) = globals.get(&path) {
                        self.env.declare(name, val.clone());
                    }
                }
            }

            for (local_name, abs_path) in &module.bindings {
                if abs_path.module_path().root() == Some("std") {
                    if let Some(val) = self.env.lookup(abs_path.name()).cloned() {
                        self.env.declare(local_name, val);
                    }
                    continue;
                }
                if let Some(val) = globals.get(abs_path) {
                    self.env.declare(local_name, val.clone());
                }
            }
        } else {
            // Single-file legacy call: populate from current items.
            crate::builtins::install(&mut self.env);
            for item in self.items {
                match &item.node {
                    Item::Function(def) => {
                        self.env.declare(
                            &def.name,
                            Value::Function(Rc::new(FunctionRef {
                                def: def.clone(),
                                module_id: None,
                            })),
                        );
                    }
                    Item::Const { name, .. } => {
                        if let Some(constant) = saved_env.lookup(name).cloned() {
                            self.env.declare(name, constant);
                        }
                    }
                    // Items that don't need pre-declaration in the environment.
                    Item::Struct { .. }
                    | Item::Enum { .. }
                    | Item::TypeAlias { .. }
                    | Item::Module { .. }
                    | Item::Import { .. }
                    | Item::ImportFrom { .. } => {}
                }
            }
        }

        self.env.push_frame();
        for (parameter, value) in def.parameters.iter().zip(arguments) {
            self.env.declare(&parameter.name, value);
        }

        let result = match &def.body {
            FunctionBody::Block(block) => match self.execute_block(block)? {
                Outcome::Value(value) | Outcome::Return(value) => value,
                Outcome::Break(_) => {
                    return Err(RuntimeError::Panic {
                        message: "`break` outside loop".to_owned(),
                        span,
                    });
                }
                Outcome::Continue => {
                    return Err(RuntimeError::Panic {
                        message: "`continue` outside loop".to_owned(),
                        span,
                    });
                }
            },
            FunctionBody::Expression(expr) => self.evaluate_expression(*expr)?,
        };

        self.env.pop_frame();
        self.env = saved_env;
        self.arena = saved_arena;
        self.items = saved_items;
        Ok(result)
    }

    fn invoke_closure(
        &mut self,
        closure: &Rc<Closure>,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if arguments.len() != closure.parameters.len() {
            return Err(RuntimeError::WrongArity {
                expected: closure.parameters.len(),
                found: arguments.len(),
                span,
            });
        }
        let saved_env = std::mem::replace(&mut self.env, closure.captured_env.clone());
        self.env.push_frame();
        for (parameter, value) in closure.parameters.iter().zip(arguments) {
            self.env.declare(&parameter.name, value);
        }
        let result = self.evaluate_expression(closure.body)?;
        self.env.pop_frame();
        self.env = saved_env;
        Ok(result)
    }

    fn evaluate_if(
        &mut self,
        condition: ExprId,
        then_branch: &Block,
        else_branch: Option<&Block>,
        treat_unknown_as_false: bool,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let cond_value = self.evaluate_expression(condition)?;
        let Value::Trilean(trilean) = cond_value else {
            return Err(RuntimeError::TypeError {
                message: "if condition must be Trilean".to_owned(),
                span,
            });
        };
        let take_then = match trilean {
            Trilean::True => true,
            Trilean::False => false,
            Trilean::Unknown => {
                if treat_unknown_as_false {
                    false
                } else {
                    return Err(RuntimeError::UnknownCondition { span });
                }
            }
        };

        let target = if take_then {
            Some(then_branch)
        } else {
            else_branch
        };
        match target {
            Some(block) => match self.execute_block(block)? {
                Outcome::Value(value) => Ok(value),
                Outcome::Return(_) => Err(RuntimeError::Panic {
                    message: "`return` from if-expression body propagated unexpectedly".to_owned(),
                    span,
                }),
                _ => Err(RuntimeError::Panic {
                    message: "control flow leaked from if branch".to_owned(),
                    span,
                }),
            },
            None => Ok(Value::Unit),
        }
    }

    fn evaluate_match(
        &mut self,
        scrutinee: ExprId,
        arms: &[MatchArm],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let value = self.evaluate_expression(scrutinee)?;

        for arm in arms {
            self.env.push_frame();
            if pattern_matches(self.arena, arm.pattern, &value, &mut |name, value| {
                self.env.declare(name, value);
            }) {
                let guard_passed = match arm.guard {
                    None => true,
                    Some(guard) => match self.evaluate_expression(guard)? {
                        Value::Trilean(Trilean::True) => true,
                        Value::Trilean(_) => false,
                        _ => false,
                    },
                };
                if guard_passed {
                    let result = self.evaluate_expression(arm.body);
                    self.env.pop_frame();
                    return result;
                }
            }
            self.env.pop_frame();
        }

        Err(RuntimeError::NonExhaustiveMatch { span })
    }

    fn bind_pattern(&mut self, id: PatternId, value: &Value) {
        let _ = pattern_matches(self.arena, id, value, &mut |name, v| {
            self.env.declare(name, v);
        });
    }
}

mod eval;

use eval::{
    advance_iterator, binary_int_method, binary_int_try_method, binary_long_method,
    binary_long_try_method, execute_binary, integer_literal_value, pattern_matches,
};

//! Tree-walking interpreter.

use std::rc::Rc;

use triet_core::{Integer, Long, Trit, Tryte};
use triet_logic::Trilean;
use triet_syntax::{
    Arena, BinaryOperator, Block, Expr, ExprId, FStringPart, FunctionBody, Item,
    LiteralPattern, MatchArm, NumericSuffix, Pattern, PatternId, Program, Span, Stmt, StmtId,
    TrileanValue, UnaryOperator,
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
    let function = interpreter
        .env
        .lookup(name)
        .cloned()
        .ok_or_else(|| RuntimeError::UndefinedName {
            name: name.to_owned(),
            span: 0..0,
        })?;
    interpreter.invoke(&function, arguments, 0..0)
}

struct Interpreter<'p> {
    arena: &'p Arena,
    items: &'p [triet_syntax::Spanned<Item>],
    env: ValueEnvironment,
}

impl<'p> Interpreter<'p> {
    fn new(program: &'p Program) -> Self {
        let mut env = ValueEnvironment::new();
        crate::builtins::install(&mut env);
        Self {
            arena: &program.arena,
            items: &program.items,
            env,
        }
    }

    /// Bind every top-level item into the environment so that calls to
    /// any function (forward or backward) resolve.
    fn install_program_items(&mut self) {
        for item in self.items {
            match &item.node {
                Item::Function(def) => {
                    let function = Value::Function(Rc::new(FunctionRef {
                        def: def.clone(),
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
                Item::TypeAlias { .. } | Item::Import(_) => {
                    // No runtime effect in v0.1.
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
            Stmt::For { variable, iterable, body } => {
                self.execute_for(variable, iterable, &body, stmt.span)
            }
            Stmt::While { condition, body, treat_unknown_as_false } => {
                self.execute_while(condition, &body, treat_unknown_as_false, stmt.span)
            }
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
            Expr::IntegerLiteral { value, suffix } => Ok(integer_literal_value(value, suffix, &span)?),
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
            Expr::Identifier(name) => self
                .env
                .lookup(&name)
                .cloned()
                .ok_or(RuntimeError::UndefinedName { name, span }),
            Expr::BinaryOp { operator, left, right } => self.evaluate_binary(operator, left, right, span),
            Expr::UnaryOp { operator: UnaryOperator::Negate, operand } => self.evaluate_negate(operand, span),
            Expr::Call { callee, arguments } => self.evaluate_call(callee, &arguments, span),
            Expr::MethodCall { receiver, method, arguments } => {
                self.evaluate_method_call(receiver, &method, &arguments, span)
            }
            Expr::FieldAccess { object: _, field } => Err(RuntimeError::TypeError {
                message: format!("no field `{field}` on value"),
                span,
            }),
            Expr::TupleIndex { tuple, index } => {
                let value = self.evaluate_expression(tuple)?;
                let Value::Tuple(elements) = value else {
                    return Err(RuntimeError::TypeError {
                        message: "tuple index on non-tuple".to_owned(),
                        span,
                    });
                };
                elements
                    .get(index)
                    .cloned()
                    .ok_or(RuntimeError::Panic {
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
            Expr::SafeMethodCall { receiver, method, arguments } => {
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
            Expr::If { condition, then_branch, else_branch, treat_unknown_as_false } => {
                self.evaluate_if(condition, &then_branch, else_branch.as_ref(), treat_unknown_as_false, span)
            }
            Expr::Match { scrutinee, arms } => self.evaluate_match(scrutinee, &arms, span),
            Expr::Block(block) => match self.execute_block(&block)? {
                Outcome::Value(value) => Ok(value),
                _ => Err(RuntimeError::Panic {
                    message: "block produced control-flow outside its enclosing function/loop".to_owned(),
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
            Expr::Lambda { parameters, return_type, body } => {
                let captured = self.env.clone();
                Ok(Value::Lambda(Rc::new(Closure {
                    parameters,
                    return_type,
                    body,
                    captured_env: captured,
                })))
            }
            Expr::Range { start, end, inclusive } => {
                let start_value = self.evaluate_expression(start)?;
                let end_value = self.evaluate_expression(end)?;
                let (Value::Integer(s), Value::Integer(e)) = (start_value, end_value) else {
                    return Err(RuntimeError::TypeError {
                        message: "range bounds must be Integer".to_owned(),
                        span,
                    });
                };
                Ok(Value::Range { start: s, end: e, inclusive })
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
        let function = self.evaluate_expression(callee)?;
        let mut arg_values = Vec::with_capacity(arguments.len());
        for &arg in arguments {
            arg_values.push(self.evaluate_expression(arg)?);
        }
        self.invoke(&function, arg_values, span)
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
            (Value::Long(l), "try_to_integer") => Ok(l
                .try_to_integer()
                .map_or(Value::Null, Value::Integer)),
            (Value::Long(l), "try_to_tryte") => Ok(l
                .try_to_tryte()
                .map_or(Value::Null, Value::Tryte)),

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
            (Value::Integer(i), "add_and_truncate") => binary_int_method(
                i,
                arguments,
                triet_core::Integer::add_and_truncate,
                span,
            ),
            (Value::Integer(i), "add_and_saturate") => binary_int_method(
                i,
                arguments,
                triet_core::Integer::add_and_saturate,
                span,
            ),
            (Value::Integer(i), "try_add") => binary_int_try_method(
                i,
                arguments,
                triet_core::Integer::try_add,
                span,
            ),
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
            (Value::Range { .. } | Value::Enumerate { .. }, "enumerate") => {
                Ok(Value::Enumerate {
                    inner: Box::new(receiver.clone()),
                    next_index: 0,
                })
            }

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

        // Save and replace env: top-level functions see only the
        // module-level bindings (functions/consts), not the caller's
        // locals. We achieve this by saving the current env and
        // restoring it after the call; the call itself runs in a fresh
        // frame stack containing globals.
        let saved_env = std::mem::replace(&mut self.env, ValueEnvironment::new());
        // Re-install builtins + program items into the fresh env. We
        // share the actual Function/Builtin Values from the saved env
        // by walking it and copying every binding flagged as "global".
        // For simplicity, we directly re-install builtins and items.
        crate::builtins::install(&mut self.env);
        for item in self.items {
            match &item.node {
                Item::Function(def) => {
                    self.env.declare(
                        &def.name,
                        Value::Function(Rc::new(FunctionRef { def: def.clone() })),
                    );
                }
                Item::Const { name, .. } => {
                    if let Some(constant) = saved_env.lookup(name).cloned() {
                        self.env.declare(name, constant);
                    }
                }
                _ => {}
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

        let target = if take_then { Some(then_branch) } else { else_branch };
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

// ====================================================================
// Free helpers
// ====================================================================

fn integer_literal_value(
    value: i128,
    suffix: Option<NumericSuffix>,
    span: &Span,
) -> Result<Value, RuntimeError> {
    match suffix {
        Some(NumericSuffix::Trit) => match Trit::from_i8(value as i8) {
            Some(trit) if i128::from(trit.to_i8()) == value => Ok(Value::Trit(trit)),
            _ => Err(RuntimeError::Panic {
                message: format!("integer literal {value} out of Trit range"),
                span: span.clone(),
            }),
        },
        Some(NumericSuffix::Tryte) => Tryte::new(value as i16)
            .map(Value::Tryte)
            .ok_or_else(|| RuntimeError::Panic {
                message: format!("integer literal {value} out of Tryte range"),
                span: span.clone(),
            }),
        Some(NumericSuffix::Long) => {
            // Long range strictly contains i128; the lexer produces an
            // i128 value, so this is always lossless.
            Ok(Value::Long(Long::from_i128(value)))
        }
        Some(NumericSuffix::Integer) | None => Integer::new(value as i64)
            .map(Value::Integer)
            .ok_or_else(|| RuntimeError::Panic {
                message: format!("integer literal {value} out of Integer range"),
                span: span.clone(),
            }),
    }
}

fn execute_binary(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    match operator {
        BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply
        | BinaryOperator::Divide | BinaryOperator::Modulo | BinaryOperator::Power => {
            execute_arithmetic(operator, left, right, span)
        }
        BinaryOperator::Equal => Ok(Value::Trilean(if left == right { Trilean::True } else { Trilean::False })),
        BinaryOperator::NotEqual => Ok(Value::Trilean(if left == right { Trilean::False } else { Trilean::True })),
        BinaryOperator::LessThan
        | BinaryOperator::LessEqual
        | BinaryOperator::GreaterThan
        | BinaryOperator::GreaterEqual => execute_comparison(operator, left, right, span),
        BinaryOperator::And => trilean_op(left, right, span, triet_logic::Trilean::and),
        BinaryOperator::Or => trilean_op(left, right, span, triet_logic::Trilean::or),
        BinaryOperator::Xor => trilean_op(left, right, span, triet_logic::Trilean::xor),
        BinaryOperator::Iff => trilean_op(left, right, span, triet_logic::Trilean::iff),
        BinaryOperator::Implies => trilean_op(left, right, span, triet_logic::Trilean::implies),
        BinaryOperator::KleeneXor => trilean_op(left, right, span, triet_logic::Trilean::kleene_xor),
        BinaryOperator::KleeneIff => trilean_op(left, right, span, triet_logic::Trilean::kleene_iff),
        BinaryOperator::KleeneImplies => trilean_op(left, right, span, triet_logic::Trilean::kleene_implies),
    }
}

fn execute_arithmetic(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Tryte(a), Value::Tryte(b)) => apply_tryte_arithmetic(operator, a, b, span).map(Value::Tryte),
        (Value::Integer(a), Value::Integer(b)) => {
            apply_integer_arithmetic(operator, a, b, span).map(Value::Integer)
        }
        (Value::Long(a), Value::Long(b)) => {
            apply_long_arithmetic(operator, a, b, span).map(Value::Long)
        }
        (left, right) => Err(RuntimeError::TypeError {
            message: format!("arithmetic on incompatible types {left} and {right}"),
            span,
        }),
    }
}

fn apply_long_arithmetic(
    operator: BinaryOperator,
    a: Long,
    b: Long,
    span: Span,
) -> Result<Long, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => long_power(a, b),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Long ({a} {operator:?} {b})"),
        span,
    })
}

fn long_power(base: Long, exponent: Long) -> Long {
    let mut result = Long::ONE;
    let count = exponent.try_to_i128().unwrap_or(0);
    if count < 0 {
        return Long::ZERO;
    }
    for _ in 0..count {
        result = result * base;
    }
    result
}

fn apply_tryte_arithmetic(
    operator: BinaryOperator,
    a: Tryte,
    b: Tryte,
    span: Span,
) -> Result<Tryte, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => panic!("** on Tryte not supported in v0.1"),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Tryte ({a} {operator:?} {b})"),
        span,
    })
}

fn apply_integer_arithmetic(
    operator: BinaryOperator,
    a: Integer,
    b: Integer,
    span: Span,
) -> Result<Integer, RuntimeError> {
    use std::panic::{self, AssertUnwindSafe};
    let result = panic::catch_unwind(AssertUnwindSafe(|| match operator {
        BinaryOperator::Add => a + b,
        BinaryOperator::Subtract => a - b,
        BinaryOperator::Multiply => a * b,
        BinaryOperator::Divide => a / b,
        BinaryOperator::Modulo => a % b,
        BinaryOperator::Power => integer_power(a, b),
        _ => unreachable!(),
    }));
    result.map_err(|_| RuntimeError::Panic {
        message: format!("arithmetic panic on Integer ({a} {operator:?} {b})"),
        span,
    })
}

fn integer_power(base: Integer, exponent: Integer) -> Integer {
    let mut result = Integer::ONE;
    let mut count = exponent.to_i64();
    if count < 0 {
        return Integer::ZERO;
    }
    while count > 0 {
        result = result * base;
        count -= 1;
    }
    result
}

fn execute_comparison(
    operator: BinaryOperator,
    left: Value,
    right: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
    let ordering = match (&left, &right) {
        (Value::Tryte(a), Value::Tryte(b)) => a.to_i64().cmp(&b.to_i64()),
        (Value::Integer(a), Value::Integer(b)) => a.to_i64().cmp(&b.to_i64()),
        (Value::Long(a), Value::Long(b)) => a.cmp(b),
        _ => {
            return Err(RuntimeError::TypeError {
                message: format!("comparison on incompatible types {left} and {right}"),
                span,
            });
        }
    };
    let result = match operator {
        BinaryOperator::LessThan => ordering.is_lt(),
        BinaryOperator::LessEqual => ordering.is_le(),
        BinaryOperator::GreaterThan => ordering.is_gt(),
        BinaryOperator::GreaterEqual => ordering.is_ge(),
        _ => unreachable!(),
    };
    Ok(Value::Trilean(if result { Trilean::True } else { Trilean::False }))
}

fn trilean_op(
    left: Value,
    right: Value,
    span: Span,
    op: impl Fn(Trilean, Trilean) -> Trilean,
) -> Result<Value, RuntimeError> {
    let (Value::Trilean(a), Value::Trilean(b)) = (left, right) else {
        return Err(RuntimeError::TypeError {
            message: "logic operator requires Trilean operands".to_owned(),
            span,
        });
    };
    Ok(Value::Trilean(op(a, b)))
}

fn binary_int_method(
    receiver: &Integer,
    arguments: Vec<Value>,
    op: impl Fn(Integer, Integer) -> Integer,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Integer(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects an Integer argument".to_owned(),
            span,
        });
    };
    Ok(Value::Integer(op(*receiver, other)))
}

fn binary_int_try_method(
    receiver: &Integer,
    arguments: Vec<Value>,
    op: impl Fn(Integer, Integer) -> Option<Integer>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Integer(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects an Integer argument".to_owned(),
            span,
        });
    };
    Ok(match op(*receiver, other) {
        Some(integer) => Value::Integer(integer),
        None => Value::Null,
    })
}

fn binary_long_method(
    receiver: &Long,
    arguments: Vec<Value>,
    op: impl Fn(Long, Long) -> Long,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Long(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects a Long argument".to_owned(),
            span,
        });
    };
    Ok(Value::Long(op(*receiver, other)))
}

/// Advance any iterable `Value` by one step. Returns `Ok(None)` when the
/// iterator is exhausted, `Ok(Some(elem))` for the next element, or an
/// error if the value isn't iterable.
///
/// Internal counterpart to a future user-facing `Iterator` trait
/// (planned for v0.2 with generics). Today the dispatch is a `match`
/// over the small set of iterable `Value` shapes — Range and Enumerate.
fn advance_iterator(
    value: &mut Value,
    span: &Span,
) -> Result<Option<Value>, RuntimeError> {
    match value {
        Value::Range { start, end, inclusive } => {
            let stop = if *inclusive {
                start.to_i64() > end.to_i64()
            } else {
                start.to_i64() >= end.to_i64()
            };
            if stop {
                return Ok(None);
            }
            let current = *start;
            // Advance by one. Saturating — in pathological loops where
            // start ≈ MAX this prevents a panic; the bound check above
            // still stops the loop the next iteration.
            *start = start.add_and_saturate(Integer::ONE);
            Ok(Some(Value::Integer(current)))
        }
        Value::Enumerate { inner, next_index } => {
            match advance_iterator(inner, span)? {
                None => Ok(None),
                Some(elem) => {
                    let idx = *next_index;
                    *next_index = next_index.saturating_add(1);
                    let index_value = Value::Integer(
                        Integer::new(idx).unwrap_or(Integer::MAX),
                    );
                    Ok(Some(Value::from_tuple(vec![index_value, elem])))
                }
            }
        }
        other => Err(RuntimeError::TypeError {
            message: format!("`for` expects an iterable, found {other}"),
            span: span.clone(),
        }),
    }
}

fn binary_long_try_method(
    receiver: &Long,
    arguments: Vec<Value>,
    op: impl Fn(Long, Long) -> Option<Long>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut iter = arguments.into_iter();
    let Some(Value::Long(other)) = iter.next() else {
        return Err(RuntimeError::TypeError {
            message: "method expects a Long argument".to_owned(),
            span,
        });
    };
    Ok(match op(*receiver, other) {
        Some(value) => Value::Long(value),
        None => Value::Null,
    })
}

/// Test whether `value` matches `pattern`, calling `bind` for every
/// variable introduced in the pattern. Returns true on a match.
fn pattern_matches<F>(
    arena: &Arena,
    pattern_id: PatternId,
    value: &Value,
    bind: &mut F,
) -> bool
where
    F: FnMut(&str, Value),
{
    let pattern = &arena.pattern(pattern_id).node;
    match pattern {
        Pattern::Wildcard => true,
        Pattern::Null => value.is_null(),
        Pattern::Variable(name) => {
            bind(name, value.clone());
            true
        }
        Pattern::Tuple(elements) => match value {
            Value::Tuple(tuple_values) => {
                if elements.len() != tuple_values.len() {
                    return false;
                }
                elements
                    .iter()
                    .zip(tuple_values.iter())
                    .all(|(child, child_value)| pattern_matches(arena, *child, child_value, bind))
            }
            _ => false,
        },
        Pattern::Or(alternatives) => alternatives
            .iter()
            .any(|alt| pattern_matches(arena, *alt, value, bind)),
        Pattern::Range { start, end, inclusive } => match value {
            Value::Integer(i) => {
                let start_i = literal_pattern_to_i64(start);
                let end_i = literal_pattern_to_i64(end);
                let v = i.to_i64();
                if *inclusive {
                    v >= start_i && v <= end_i
                } else {
                    v >= start_i && v < end_i
                }
            }
            Value::Tryte(t) => {
                let start_i = literal_pattern_to_i64(start);
                let end_i = literal_pattern_to_i64(end);
                let v = t.to_i64();
                if *inclusive {
                    v >= start_i && v <= end_i
                } else {
                    v >= start_i && v < end_i
                }
            }
            _ => false,
        },
        Pattern::Literal(literal) => literal_matches(literal, value),
    }
}

fn literal_pattern_to_i64(literal: &LiteralPattern) -> i64 {
    match literal {
        LiteralPattern::Integer { value, .. } | LiteralPattern::Ternary(value) => {
            i64::try_from(*value).unwrap_or(i64::MAX)
        }
        _ => 0,
    }
}

fn literal_matches(literal: &LiteralPattern, value: &Value) -> bool {
    match (literal, value) {
        (LiteralPattern::Integer { value: lit, .. }, Value::Integer(i)) => {
            i.to_i64() == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Integer { value: lit, .. }, Value::Tryte(t)) => {
            i64::from(t.to_i16()) == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Ternary(lit), Value::Integer(i)) => {
            i.to_i64() == i64::try_from(*lit).unwrap_or(i64::MAX)
        }
        (LiteralPattern::Trilean(lit), Value::Trilean(value)) => {
            matches!(
                (lit, value),
                (TrileanValue::True, Trilean::True)
                    | (TrileanValue::False, Trilean::False)
                    | (TrileanValue::Unknown, Trilean::Unknown)
            )
        }
        (LiteralPattern::String(lit), Value::String(actual)) => &**actual == lit,
        _ => false,
    }
}

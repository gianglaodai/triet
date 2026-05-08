//! Built-in functions exposed to the runtime.
//!
//! V0.1 deliberately ships a thin prelude — `print`, `println`,
//! `read_line`, `to_string`, and a few helpers. Larger libraries are
//! deferred. Built-ins are written as plain Rust functions matching
//! the [`crate::value::BuiltinFn`] signature.

use std::io::{self, BufRead, Write};

use crate::{error::RuntimeError, value::Value};

/// Register every built-in into the value environment.
pub(crate) fn install(env: &mut crate::env::ValueEnvironment) {
    env.declare("print", Value::Builtin(builtin_print));
    env.declare("println", Value::Builtin(builtin_println));
    env.declare("read_line", Value::Builtin(builtin_read_line));
    env.declare("to_string", Value::Builtin(builtin_to_string));
    env.declare("tryte_to_string", Value::Builtin(builtin_to_string));
    env.declare("long_to_string", Value::Builtin(builtin_to_string));
    env.declare("trilean_to_string", Value::Builtin(builtin_to_string));
    env.declare("length", Value::Builtin(builtin_length));
}

fn builtin_print(args: &[Value]) -> Result<Value, RuntimeError> {
    let text = args.first().map_or_else(String::new, ToString::to_string);
    print!("{text}");
    let _ = io::stdout().flush();
    Ok(Value::Unit)
}

fn builtin_println(args: &[Value]) -> Result<Value, RuntimeError> {
    let text = args.first().map_or_else(String::new, ToString::to_string);
    println!("{text}");
    Ok(Value::Unit)
}

fn builtin_read_line(_args: &[Value]) -> Result<Value, RuntimeError> {
    let mut line = String::new();
    let stdin = io::stdin();
    stdin.lock().read_line(&mut line).map_err(|error| {
        RuntimeError::Panic {
            message: format!("read_line failed: {error}"),
            span: 0..0,
        }
    })?;
    // Strip the trailing newline if present.
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(Value::from_string(line))
}

fn builtin_to_string(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args.first().ok_or(RuntimeError::WrongArity {
        expected: 1,
        found: 0,
        span: 0..0,
    })?;
    Ok(Value::from_string(value.to_string()))
}

fn builtin_length(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args.first().ok_or(RuntimeError::WrongArity {
        expected: 1,
        found: 0,
        span: 0..0,
    })?;
    match value {
        Value::String(text) => {
            let chars = text.chars().count();
            // Convert to Integer; if it overflows the Integer range,
            // fall back to MAX (graceful). For v0.1 this is never an
            // issue at sane string sizes.
            let int = triet_core::Integer::new(i64::try_from(chars).unwrap_or(i64::MAX))
                .unwrap_or(triet_core::Integer::MAX);
            Ok(Value::Integer(int))
        }
        other => Err(RuntimeError::TypeError {
            message: format!("length expected String, found {other}"),
            span: 0..0,
        }),
    }
}

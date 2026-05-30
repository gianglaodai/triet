//! Built-in functions exposed to the runtime.
//!
//! V0.1 deliberately ships a thin prelude — `print`, `println`,
//! `read_line`, `to_string`, and a few helpers. Larger libraries are
//! deferred. Built-ins are written as plain Rust functions matching
//! the [`crate::value::BuiltinFn`] signature.
//!
//! v0.10.x.interp.1 adds `sys.atomic.*` parity per [ADR-0031 §10.7] —
//! mirrors VM's `path_to_builtin` dispatch for the 10 atomic ops from
//! [ADR-0028 §4]. `compare_exchange` is structurally implemented but
//! short-circuits to a `RuntimeError::TypeError` because the interpreter
//! lacks Outcome support (parity gap tracked v0.10.x.interp.2+).
//!
//! [ADR-0028 §4]: ../../../docs/decisions/0028-atomic-primitive.md
//! [ADR-0031 §10.7]: ../../../docs/decisions/0031-borrow-expression-syntax.md

use std::{
    cell::RefCell,
    io::{self, BufRead, Write},
    rc::Rc,
};

use triet_core::{Integer, Tryte};

use crate::{error::RuntimeError, value::Value};

/// Map a stdlib module path and function name to its builtin implementation.
pub(crate) fn get_builtin(
    module: &triet_modules::ModulePath,
    name: &str,
) -> Option<crate::value::BuiltinFn> {
    let module_str = module.to_string();
    match (module_str.as_str(), name) {
        ("std.io", "print") => Some(builtin_print),
        ("std.io", "println") => Some(builtin_println),
        ("std.io", "read_line") => Some(builtin_read_line),
        ("std.text", "len") => Some(builtin_length),
        // other to_string conversions if needed in std.text
        ("std.assert", "assert") => Some(builtin_assert),
        ("std.assert", "assert_eq") => Some(builtin_assert_eq),
        // v0.10.x.interp.1 (ADR-0028 §4 + ADR-0031 §10.7): atomic builtins
        // mirror VM dispatch paths from `triet_ir::vm::path_to_builtin`.
        ("sys.atomic", "new") => Some(builtin_atomic_new),
        ("sys.atomic", "load") => Some(builtin_atomic_load),
        ("sys.atomic", "store") => Some(builtin_atomic_store),
        ("sys.atomic", "swap") => Some(builtin_atomic_swap),
        ("sys.atomic", "compare_exchange") => Some(builtin_atomic_compare_exchange),
        ("sys.atomic", "fetch_add") => Some(builtin_atomic_fetch_add),
        ("sys.atomic", "fetch_sub") => Some(builtin_atomic_fetch_sub),
        ("sys.atomic", "fetch_bitwise_and") => Some(builtin_atomic_fetch_bitwise_and),
        ("sys.atomic", "fetch_bitwise_or") => Some(builtin_atomic_fetch_bitwise_or),
        ("sys.atomic", "fetch_bitwise_xor") => Some(builtin_atomic_fetch_bitwise_xor),
        _ => None,
    }
}

/// Legacy installer for the single-file runtime (v0.1.x fallback).
pub(crate) fn install(env: &mut crate::env::ValueEnvironment) {
    env.declare("print", Value::Builtin(builtin_print));
    env.declare("println", Value::Builtin(builtin_println));
    env.declare("read_line", Value::Builtin(builtin_read_line));
    env.declare("to_string", Value::Builtin(builtin_to_string));
    env.declare("tryte_to_string", Value::Builtin(builtin_to_string));
    env.declare("long_to_string", Value::Builtin(builtin_to_string));
    env.declare("trilean_to_string", Value::Builtin(builtin_to_string));
    env.declare("length", Value::Builtin(builtin_length));
    env.declare("assert", Value::Builtin(builtin_assert));
    env.declare("assert_eq", Value::Builtin(builtin_assert_eq));
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
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|error| RuntimeError::Panic {
            message: format!("read_line failed: {error}"),
            span: 0..0,
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

fn builtin_assert(args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args.first().ok_or(RuntimeError::WrongArity {
        expected: 1,
        found: 0,
        span: 0..0,
    })?;

    if matches!(value, Value::Trilean(triet_logic::Trilean::True)) {
        Ok(Value::Unit)
    } else {
        Err(RuntimeError::Panic {
            message: format!("assertion failed: expected True, got {value}"),
            span: 0..0,
        })
    }
}

fn builtin_assert_eq(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 2 {
        return Err(RuntimeError::WrongArity {
            expected: 2,
            found: args.len(),
            span: 0..0,
        });
    }

    let left = &args[0];
    let right = &args[1];

    if left == right {
        Ok(Value::Unit)
    } else {
        Err(RuntimeError::Panic {
            message: format!("assertion failed: {left} != {right}"),
            span: 0..0,
        })
    }
}

// ── Atomic builtins (v0.10.x.interp.1, ADR-0028 §4 + ADR-0031 §10.7) ──
//
// All ops take `&+ Atomic<T>` as the first arg per ADR-0028 §5. In the
// tree-walking interpreter, references erase at runtime (per ADR-0026 v2
// §7) — the shim receives the `Value::Atomic` directly. Atomicity is
// single-thread no-op per ADR-0028 §9 (dev tier); the `Ordering` arg is
// validated at typecheck (E1040), discarded here.

fn extract_atomic(value: &Value, op_name: &str) -> Result<Rc<RefCell<Value>>, RuntimeError> {
    match value {
        Value::Atomic(rc) => Ok(rc.clone()),
        other => Err(RuntimeError::TypeError {
            message: format!("{op_name}: expected Atomic<T>, found {other}"),
            span: 0..0,
        }),
    }
}

fn builtin_atomic_new(args: &[Value]) -> Result<Value, RuntimeError> {
    let initial = args.first().ok_or(RuntimeError::WrongArity {
        expected: 1,
        found: 0,
        span: 0..0,
    })?;
    Ok(Value::Atomic(Rc::new(RefCell::new(initial.clone()))))
}

fn builtin_atomic_load(args: &[Value]) -> Result<Value, RuntimeError> {
    // Signature: (atom: &+ Atomic<T>, ordering: Ordering) -> T.
    // Ordering is the second positional arg; ignored on single-thread VM.
    let atomic = args.first().ok_or(RuntimeError::WrongArity {
        expected: 2,
        found: 0,
        span: 0..0,
    })?;
    let cell = extract_atomic(atomic, "atomic.load")?;
    let snapshot = cell.borrow().clone();
    Ok(snapshot)
}

fn builtin_atomic_store(args: &[Value]) -> Result<Value, RuntimeError> {
    // Signature: (atom: &+ Atomic<T>, value: T, ordering: Ordering) -> Unit.
    let atomic = args.first().ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 0,
        span: 0..0,
    })?;
    let new_value = args.get(1).ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 1,
        span: 0..0,
    })?;
    let cell = extract_atomic(atomic, "atomic.store")?;
    *cell.borrow_mut() = new_value.clone();
    Ok(Value::Unit)
}

fn builtin_atomic_swap(args: &[Value]) -> Result<Value, RuntimeError> {
    // Signature: (atom: &+ Atomic<T>, value: T, ordering: Ordering) -> T.
    // Returns previous value.
    let atomic = args.first().ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 0,
        span: 0..0,
    })?;
    let new_value = args.get(1).ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 1,
        span: 0..0,
    })?;
    let cell = extract_atomic(atomic, "atomic.swap")?;
    let prev = cell.borrow().clone();
    *cell.borrow_mut() = new_value.clone();
    Ok(prev)
}

fn builtin_atomic_compare_exchange(_args: &[Value]) -> Result<Value, RuntimeError> {
    // compare_exchange returns `T~CompareExchangeFailed<T>` per ADR-0028
    // §4.1 — an Outcome type. The tree-walking interpreter does NOT
    // model ADR-0020 Outcome values (Value enum has no Outcome variant);
    // attempting to fabricate one via EnumVariant would not interoperate
    // with `~?` / `~:` postfix operators. Deferred to a later interp
    // parity pass that adds Outcome support. The structural intercept
    // (this function) prevents the stack overflow that the recursive
    // stdlib stub would otherwise cause.
    Err(RuntimeError::TypeError {
        message: "atomic.compare_exchange: interpreter parity deferred — runs through VM only \
             until Outcome support lands (ADR-0028 §4.1 / ADR-0020 / ADR-0031 §10.7)"
            .to_owned(),
        span: 0..0,
    })
}

/// Shared dispatch for `fetch_add` / `fetch_sub` — operates on `Tryte` or
/// `Integer` per ADR-0028 §4.2. Returns PREVIOUS value (pre-modification).
fn atomic_fetch_arithmetic(
    args: &[Value],
    op_name: &str,
    op: ArithmeticOp,
) -> Result<Value, RuntimeError> {
    let atomic = args.first().ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 0,
        span: 0..0,
    })?;
    let delta = args.get(1).ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 1,
        span: 0..0,
    })?;
    let cell = extract_atomic(atomic, op_name)?;
    let current = cell.borrow().clone();
    let new_value = match op {
        ArithmeticOp::Add => arithmetic_add(&current, delta, op_name)?,
        ArithmeticOp::Sub => arithmetic_sub(&current, delta, op_name)?,
    };
    *cell.borrow_mut() = new_value;
    Ok(current)
}

/// Shared dispatch for `fetch_bitwise_{and,or,xor}` per ADR-0028 Addendum
/// §4.3 — binary-semantic ops on `Atomic<Integer>` only. Returns PREVIOUS.
fn atomic_fetch_bitwise(
    args: &[Value],
    op_name: &str,
    op: BitwiseOp,
) -> Result<Value, RuntimeError> {
    let atomic = args.first().ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 0,
        span: 0..0,
    })?;
    let mask = args.get(1).ok_or(RuntimeError::WrongArity {
        expected: 3,
        found: 1,
        span: 0..0,
    })?;
    let cell = extract_atomic(atomic, op_name)?;
    let current = cell.borrow().clone();
    let (a_raw, b_raw) = match (&current, mask) {
        (Value::Integer(a), Value::Integer(b)) => (a.to_i64(), b.to_i64()),
        _ => {
            return Err(RuntimeError::TypeError {
                message: format!("{op_name}: expected Atomic<Integer> with Integer mask"),
                span: 0..0,
            });
        }
    };
    let new_raw = match op {
        BitwiseOp::And => a_raw & b_raw,
        BitwiseOp::Or => a_raw | b_raw,
        BitwiseOp::Xor => a_raw ^ b_raw,
    };
    let new_int = Integer::new(new_raw).ok_or_else(|| RuntimeError::Panic {
        message: format!("{op_name}: Integer overflow"),
        span: 0..0,
    })?;
    *cell.borrow_mut() = Value::Integer(new_int);
    Ok(current)
}

#[derive(Clone, Copy)]
enum ArithmeticOp {
    Add,
    Sub,
}

#[derive(Clone, Copy)]
enum BitwiseOp {
    And,
    Or,
    Xor,
}

/// Element-wise addition for `AtomicValue` primitives. Mirrors the VM's
/// `arithmetic_add` (per `triet_ir::vm`) for `Tryte` and `Integer` per
/// ADR-0028 §4.2 type table. Other types raise `TypeError`.
fn arithmetic_add(a: &Value, b: &Value, op_name: &str) -> Result<Value, RuntimeError> {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => {
            let sum = x
                .to_i64()
                .checked_add(y.to_i64())
                .ok_or_else(|| RuntimeError::Panic {
                    message: format!("{op_name}: Integer overflow"),
                    span: 0..0,
                })?;
            let result = Integer::new(sum).ok_or_else(|| RuntimeError::Panic {
                message: format!("{op_name}: Integer out of balanced-ternary range"),
                span: 0..0,
            })?;
            Ok(Value::Integer(result))
        }
        (Value::Tryte(x), Value::Tryte(y)) => {
            let sum = i64::from(x.to_i16())
                .checked_add(i64::from(y.to_i16()))
                .ok_or_else(|| RuntimeError::Panic {
                    message: format!("{op_name}: Tryte overflow"),
                    span: 0..0,
                })?;
            let raw = i16::try_from(sum).map_err(|_| RuntimeError::Panic {
                message: format!("{op_name}: Tryte out of range"),
                span: 0..0,
            })?;
            let result = Tryte::new(raw).ok_or_else(|| RuntimeError::Panic {
                message: format!("{op_name}: Tryte out of balanced-ternary range"),
                span: 0..0,
            })?;
            Ok(Value::Tryte(result))
        }
        _ => Err(RuntimeError::TypeError {
            message: format!(
                "{op_name}: arithmetic on Atomic only supports Tryte/Integer per ADR-0028 §4.2"
            ),
            span: 0..0,
        }),
    }
}

/// Element-wise subtraction; paired with `arithmetic_add` per ADR-0028 §4.2.
fn arithmetic_sub(a: &Value, b: &Value, op_name: &str) -> Result<Value, RuntimeError> {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => {
            let diff = x
                .to_i64()
                .checked_sub(y.to_i64())
                .ok_or_else(|| RuntimeError::Panic {
                    message: format!("{op_name}: Integer overflow"),
                    span: 0..0,
                })?;
            let result = Integer::new(diff).ok_or_else(|| RuntimeError::Panic {
                message: format!("{op_name}: Integer out of balanced-ternary range"),
                span: 0..0,
            })?;
            Ok(Value::Integer(result))
        }
        (Value::Tryte(x), Value::Tryte(y)) => {
            let diff = i64::from(x.to_i16())
                .checked_sub(i64::from(y.to_i16()))
                .ok_or_else(|| RuntimeError::Panic {
                    message: format!("{op_name}: Tryte overflow"),
                    span: 0..0,
                })?;
            let raw = i16::try_from(diff).map_err(|_| RuntimeError::Panic {
                message: format!("{op_name}: Tryte out of range"),
                span: 0..0,
            })?;
            let result = Tryte::new(raw).ok_or_else(|| RuntimeError::Panic {
                message: format!("{op_name}: Tryte out of balanced-ternary range"),
                span: 0..0,
            })?;
            Ok(Value::Tryte(result))
        }
        _ => Err(RuntimeError::TypeError {
            message: format!(
                "{op_name}: arithmetic on Atomic only supports Tryte/Integer per ADR-0028 §4.2"
            ),
            span: 0..0,
        }),
    }
}

/// Concrete-state equality for `AtomicValue` primitives per ADR-0028 §2.
/// Used by `compare_exchange` (currently deferred — see
/// `builtin_atomic_compare_exchange`). Public-in-module for future
/// interp parity work; intentionally `pub(super)`-style kept private.
#[allow(dead_code)]
fn atomic_value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Trit(x), Value::Trit(y)) => x == y,
        (Value::Tryte(x), Value::Tryte(y)) => x == y,
        (Value::Integer(x), Value::Integer(y)) => x == y,
        (Value::Trilean(x), Value::Trilean(y)) => x == y,
        _ => false,
    }
}

fn builtin_atomic_fetch_add(args: &[Value]) -> Result<Value, RuntimeError> {
    atomic_fetch_arithmetic(args, "atomic.fetch_add", ArithmeticOp::Add)
}

fn builtin_atomic_fetch_sub(args: &[Value]) -> Result<Value, RuntimeError> {
    atomic_fetch_arithmetic(args, "atomic.fetch_sub", ArithmeticOp::Sub)
}

fn builtin_atomic_fetch_bitwise_and(args: &[Value]) -> Result<Value, RuntimeError> {
    atomic_fetch_bitwise(args, "atomic.fetch_bitwise_and", BitwiseOp::And)
}

fn builtin_atomic_fetch_bitwise_or(args: &[Value]) -> Result<Value, RuntimeError> {
    atomic_fetch_bitwise(args, "atomic.fetch_bitwise_or", BitwiseOp::Or)
}

fn builtin_atomic_fetch_bitwise_xor(args: &[Value]) -> Result<Value, RuntimeError> {
    atomic_fetch_bitwise(args, "atomic.fetch_bitwise_xor", BitwiseOp::Xor)
}

// Suppress warning for the unused helper kept for future parity work.
const _: fn(&Value, &Value) -> bool = atomic_value_eq;

#[cfg(test)]
mod atomic_tests {
    use super::*;
    use triet_core::Trit;
    use triet_logic::Trilean;

    fn integer(n: i64) -> Value {
        Value::Integer(Integer::new(n).unwrap())
    }

    fn ordering_placeholder() -> Value {
        // Ordering arg is validated at typecheck; runtime ignores it.
        // Any value works; use Unit for clarity.
        Value::Unit
    }

    #[test]
    fn atomic_new_wraps_initial_value() {
        let result = builtin_atomic_new(&[integer(42)]).unwrap();
        match result {
            Value::Atomic(cell) => assert_eq!(*cell.borrow(), integer(42)),
            other => panic!("expected Atomic, got {other}"),
        }
    }

    #[test]
    fn atomic_load_returns_current_value() {
        let atom = builtin_atomic_new(&[integer(7)]).unwrap();
        let loaded = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(loaded, integer(7));
    }

    #[test]
    fn atomic_store_updates_value() {
        let atom = builtin_atomic_new(&[integer(0)]).unwrap();
        builtin_atomic_store(&[atom.clone(), integer(99), ordering_placeholder()]).unwrap();
        let loaded = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(loaded, integer(99));
    }

    #[test]
    fn atomic_swap_returns_previous_value() {
        let atom = builtin_atomic_new(&[integer(10)]).unwrap();
        let prev =
            builtin_atomic_swap(&[atom.clone(), integer(20), ordering_placeholder()]).unwrap();
        assert_eq!(prev, integer(10));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(20));
    }

    #[test]
    fn atomic_fetch_add_returns_previous_and_increments() {
        let atom = builtin_atomic_new(&[integer(5)]).unwrap();
        let prev =
            builtin_atomic_fetch_add(&[atom.clone(), integer(3), ordering_placeholder()]).unwrap();
        assert_eq!(prev, integer(5));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(8));
    }

    #[test]
    fn atomic_fetch_sub_returns_previous_and_decrements() {
        let atom = builtin_atomic_new(&[integer(10)]).unwrap();
        let prev =
            builtin_atomic_fetch_sub(&[atom.clone(), integer(4), ordering_placeholder()]).unwrap();
        assert_eq!(prev, integer(10));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(6));
    }

    #[test]
    fn atomic_fetch_bitwise_and_masks_bits() {
        let atom = builtin_atomic_new(&[integer(0b1111)]).unwrap();
        let prev = builtin_atomic_fetch_bitwise_and(&[
            atom.clone(),
            integer(0b1010),
            ordering_placeholder(),
        ])
        .unwrap();
        assert_eq!(prev, integer(0b1111));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(0b1010));
    }

    #[test]
    fn atomic_fetch_bitwise_or_sets_bits() {
        let atom = builtin_atomic_new(&[integer(0b1010)]).unwrap();
        let prev = builtin_atomic_fetch_bitwise_or(&[
            atom.clone(),
            integer(0b0101),
            ordering_placeholder(),
        ])
        .unwrap();
        assert_eq!(prev, integer(0b1010));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(0b1111));
    }

    #[test]
    fn atomic_fetch_bitwise_xor_toggles_bits() {
        let atom = builtin_atomic_new(&[integer(0b1100)]).unwrap();
        let prev = builtin_atomic_fetch_bitwise_xor(&[
            atom.clone(),
            integer(0b1010),
            ordering_placeholder(),
        ])
        .unwrap();
        assert_eq!(prev, integer(0b1100));
        let now = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(now, integer(0b0110));
    }

    #[test]
    fn atomic_compare_exchange_defers_to_outcome_parity() {
        let atom = builtin_atomic_new(&[integer(1)]).unwrap();
        let err = builtin_atomic_compare_exchange(&[
            atom,
            integer(1),
            integer(2),
            ordering_placeholder(),
            ordering_placeholder(),
        ])
        .unwrap_err();
        assert!(matches!(err, RuntimeError::TypeError { .. }));
    }

    #[test]
    fn atomic_handles_share_cell_state() {
        // Two clones of the same Atomic handle observe each other's writes
        // — exercises Rc<RefCell<Value>> sharing per ADR-0028 §5.
        let atom = builtin_atomic_new(&[integer(0)]).unwrap();
        let alias = atom.clone();
        builtin_atomic_store(&[alias, integer(42), ordering_placeholder()]).unwrap();
        let loaded = builtin_atomic_load(&[atom, ordering_placeholder()]).unwrap();
        assert_eq!(loaded, integer(42));
    }

    #[test]
    fn atomic_load_rejects_non_atomic_arg() {
        let err = builtin_atomic_load(&[integer(7), ordering_placeholder()]).unwrap_err();
        assert!(matches!(err, RuntimeError::TypeError { .. }));
    }

    #[test]
    fn atomic_value_eq_matches_primitives() {
        assert!(atomic_value_eq(&integer(5), &integer(5)));
        assert!(!atomic_value_eq(&integer(5), &integer(6)));
        assert!(atomic_value_eq(
            &Value::Trit(Trit::Positive),
            &Value::Trit(Trit::Positive)
        ));
        assert!(atomic_value_eq(
            &Value::Trilean(Trilean::True),
            &Value::Trilean(Trilean::True)
        ));
        // Cross-type comparisons always false.
        assert!(!atomic_value_eq(&integer(0), &Value::Trit(Trit::Zero)));
    }
}

//! Method type rules for the v0.1 prelude.
//!
//! Lookup table mapping `(receiver_type, method_name, arity)` to a
//! return type. Pure data — no `Checker` state needed. Lives here so
//! the table can grow large without bloating `check.rs`.

use crate::types::Type;

/// Returns the method's *return type* if `(receiver, method, arity)`
/// matches a known built-in; otherwise `None` (which the caller turns
/// into an `UnknownMember` error).
pub(super) fn builtin_method_type(receiver: &Type, method: &str, arity: usize) -> Option<Type> {
    use Type::{Integer, Long, String, Tryte};
    // ADR-0021: Trilean is now a struct variant — handle separately
    // since we want both `Trilean` and `Trilean!` receivers to dispatch
    // to the same methods.
    if let Type::Trilean { .. } = receiver {
        return trilean_method_type(method, arity);
    }
    match (receiver, method, arity) {
        (Tryte, "to_integer", 0) => Some(Integer),
        (Tryte, "to_long", 0) => Some(Long),
        (Integer, "to_tryte", 0) => Some(Tryte),
        (Integer, "to_long", 0) => Some(Long),
        (Long, "to_integer", 0) => Some(Integer),
        (Long, "to_tryte", 0) => Some(Tryte),

        // Try-conversions return Nullable<T>.
        (Tryte | Long, "try_to_integer", 0) => Some(Type::Nullable(Box::new(Integer))),
        (Integer | Long, "try_to_tryte", 0) => Some(Type::Nullable(Box::new(Tryte))),
        (Integer | Tryte, "try_to_long", 0) => Some(Type::Nullable(Box::new(Long))),

        // Overflow-handled arithmetic — must match self-type.
        (
            Tryte,
            "add_and_truncate"
            | "add_and_saturate"
            | "subtract_and_truncate"
            | "subtract_and_saturate"
            | "multiply_and_truncate"
            | "multiply_and_saturate",
            1,
        ) => Some(Tryte),
        (
            Integer,
            "add_and_truncate"
            | "add_and_saturate"
            | "subtract_and_truncate"
            | "subtract_and_saturate"
            | "multiply_and_truncate"
            | "multiply_and_saturate",
            1,
        ) => Some(Integer),
        (Tryte, "try_add" | "try_subtract" | "try_multiply" | "try_divide" | "try_modulo", 1) => {
            Some(Type::Nullable(Box::new(Tryte)))
        }
        (Integer, "try_add" | "try_subtract" | "try_multiply" | "try_divide" | "try_modulo", 1) => {
            Some(Type::Nullable(Box::new(Integer)))
        }

        // (Trilean methods handled by `trilean_method_type` above — both
        // refined and generic Trilean dispatch through that branch.)

        // String
        (String, "length", 0) => Some(Integer),

        // Iterables — `.enumerate()` pairs each element with a 0-based
        // Integer index. Result is `Range<(Integer, T)>` so the existing
        // `for` typing path handles destructuring.
        (Type::Range(inner), "enumerate", 0) => Some(Type::Range(Box::new(Type::Tuple(vec![
            Integer,
            (**inner).clone(),
        ])))),

        // Range — only `.enumerate()` for now; other adapters arrive
        // with v0.2 generics + Iterator trait.
        _ => check_outcome_unwrap_method(receiver, method, arity),
    }
}

/// Resolve methods on a `Trilean` / `Trilean!` receiver per ADR-0021.
/// Both refinement states dispatch identically — widening / narrowing
/// happens via the result type, not the receiver match.
fn trilean_method_type(method: &str, arity: usize) -> Option<Type> {
    match (method, arity) {
        ("to_trit", 0) => Some(Type::Trit),
        // ADR-0021 §2.4 + §8: `.assume_known(message)` requires a
        // message argument (matches `feedback_explicit_strictness` —
        // panic-possible ops MUST be verbose methods with msg). Returns
        // `Trilean!` so callers can chain into plain `if`.
        ("assume_known", 1) => Some(Type::TRILEAN_KNOWN),
        _ => None,
    }
}

/// Resolve `.unwrap_value(message)` / `.unwrap_error(message)` on an
/// `Outcome` receiver per [ADR-0020] §3 + `feedback_explicit_strictness`:
/// panic-possible ops MUST be verbose methods with a message argument,
/// never property access. Returns `Some(value_type)` for `unwrap_value`
/// and `Some(error_type)` for `unwrap_error`; `None` for anything else.
/// The message argument is not type-checked here (the surrounding
/// `check_method_call` runs `infer_expression` on each arg for side
/// effects); enforcing `String` is left for a later strictness pass.
///
/// [ADR-0020]: ../../../../docs/decisions/0020-outcome-error-handling.md
fn check_outcome_unwrap_method(receiver: &Type, method: &str, arity: usize) -> Option<Type> {
    let Type::Outcome {
        value_type,
        error_type,
        ..
    } = receiver
    else {
        return None;
    };
    match (method, arity) {
        ("unwrap_value", 1) => Some((**value_type).clone()),
        ("unwrap_error", 1) => Some((**error_type).clone()),
        _ => None,
    }
}

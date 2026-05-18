//! v0.7.4.3-debt.3 — integration tests for the WA-5 fix.
//!
//! Pre-fix: `let x: T? = ~0` inside a function returning `T~E` raised
//! a false-positive E1025 because the typecheck only consulted
//! `current_return_type` and ignored the let-binding's annotation.
//! The fix adds `expected_type_stack` consulted before
//! `current_return_type`, so the most-specific local context wins.
//!
//! Tests pin both positive (no E1025) and negative (E1025 still
//! fires when truly invalid) behavior.

use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Run check + return error codes encountered (empty == clean).
/// Treats warnings as benign — caller already drives positive vs
/// negative checks via the error code set.
fn check_codes(src: &str) -> Vec<String> {
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    diagnostics.iter().map(|e| format!("{e}")).collect()
}

#[test]
fn nullable_local_inside_binary_outcome_function_is_clean() {
    // The canonical lexer-port pattern that surfaced WA-5: a
    // fallible function uses a nullable local that may legitimately
    // hold a null marker.
    let src = r"
        function lex_step() -> Integer~String = {
            let suffix: Integer? = ~0
            let other: Integer? = ~0
            ~+ 42
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "no errors expected; got: {errors:#?}",);
}

#[test]
fn struct_field_nullable_inside_binary_outcome_function_is_clean() {
    // A struct literal field's expected type also threads through
    // the expected-type stack (will land alongside WA-5 — for now
    // this just confirms the let-binding case works).
    let src = r"
        struct Acc {
            value: Integer,
            tag: Integer?,
        }

        function build_acc() -> Acc~String = {
            let tagless: Integer? = ~0
            ~+ Acc { value: 1, tag: tagless }
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(errors.is_empty(), "no errors expected; got: {errors:#?}");
}

#[test]
fn bare_outcome_zero_in_binary_return_still_raises_e1025() {
    // Without a tighter local context, the surrounding return type
    // is the only context. `~0` against `Integer~String` must still
    // fire E1025 — the fix is targeted, not a blanket weakening.
    let src = r"
        function bad_return() -> Integer~String = {
            ~0
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("requires outcome type with null state")),
        "expected E1025; got: {errors:?}",
    );
}

#[test]
fn outcome_zero_without_annotation_falls_back_to_return_type() {
    // No annotation on the let — the expected-type stack is empty,
    // so the resolver falls back to `current_return_type`. Same
    // behavior as before WA-5: E1025 fires.
    let src = r"
        function bad_let() -> Integer~String = {
            let x = ~0
            ~+ 1
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("requires outcome type with null state")),
        "expected E1025; got: {errors:?}",
    );
}

#[test]
fn ternary_outcome_return_accepts_outcome_zero_arm() {
    // Sanity: `~0` against `T?~E` was never flagged. WA-5 fix
    // should preserve this path.
    let src = r"
        function maybe_lookup(id: Integer) -> Integer?~String = {
            if id == 1 { ~+ 100 }
            else if id == 2 { ~- 'not allowed' }
            else { ~0 }
        }
        function main() {}
    "
    .replace('\'', "\"");
    let errors = check_codes(&src);
    assert!(errors.is_empty(), "no errors expected; got: {errors:?}");
}

//! Integration tests for diagnostic formats.
use miette::Diagnostic;
use std::ops::Range;
use triet_typecheck::{BorrowError, ConcurrencyError, TypeError};

// Dummy span for testing
const fn dummy_span() -> Range<usize> {
    0..0
}

#[test]
fn e2400_borrow_lifetime_inference_failed_format() {
    let err = BorrowError::BorrowLifetimeInferenceFailed {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `-> &0 String` to `-> &+ String`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("[Fix 3]"));
}

#[test]
fn e2402_borrow_in_struct_field_format() {
    let err = BorrowError::BorrowInStructField {
        field_name: "my_field".to_string(),
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `&0 String` or `&- String` to `&+ String`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Remove `my_field` field"));
}

#[test]
fn e2403_escaping_borrow_format() {
    let err = BorrowError::EscapingBorrow { span: dummy_span() };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change the return type to `&+ T`"));
    assert!(help.contains("[Fix 2]"));
}

#[test]
fn e2410_cannot_mutate_frozen_owner_format() {
    // v0.10.x.borrow.3: skeleton message corrected — E2410 fires on
    // `&+ T` frozen-owner mutation, NOT `&0 T`. Fix suggestions point
    // toward `&+ mutable T`.
    let err = BorrowError::CannotMutateFrozenOwner {
        field: "name".to_string(),
        ty: "User".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(
        help.contains("Change `&+ User` to `&+ mutable User`"),
        "expected new &+ → &+ mutable fix text, got: {help}"
    );
    assert!(help.contains("[Fix 2]"));
}

#[test]
fn e2411_cannot_promote_frozen_to_mutable_format() {
    // v0.10.x.borrow.3: skeleton message corrected — E2411 is about
    // `&+ T` → `&+ mutable T` promotion (not `&0` → `&-`).
    let err = BorrowError::CannotPromoteFrozenToMutable {
        ty: "User".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(
        help.contains("`&+ User`") && help.contains("`&+ mutable User`"),
        "expected new &+ → &+ mutable fix text, got: {help}"
    );
    assert!(help.contains("[Fix 2]"));
}

#[test]
fn e2420_use_after_move_format() {
    let err = BorrowError::UseAfterMove {
        name: "foo".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change argument to `&0 foo` or `&- foo`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Use `foo.clone()`"));
}

#[test]
fn e2421_self_ownership_paradox_format() {
    let err = BorrowError::SelfOwnershipParadox { span: dummy_span() };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Replace the self-reference"));
}

#[test]
fn e2422_non_terminating_construction_format() {
    let err = BorrowError::NonTerminatingConstruction {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `&+ String` to `(&+ String)?~E`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Change `&+ String` to `Vector<&+ String>`"));
}

#[test]
fn e2430_namespace_inference_failed_format() {
    let err = BorrowError::NamespaceInferenceFailed { span: dummy_span() };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change to fully qualified capability path"));
}

#[test]
fn e2440_borrow_exclusivity_violation_format() {
    // v0.10.x.borrow.1: skeleton expanded — diagnostic now carries
    // both borrow forms + both creation spans per ADR-0025 §2.2.
    let err = BorrowError::BorrowExclusivityViolation {
        base: "bar".to_string(),
        first_form: "&0".to_string(),
        second_form: "&0 mutable".to_string(),
        first_span: dummy_span(),
        span: dummy_span(),
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("&0 mutable bar") && msg.contains("&0 bar"),
        "expected message to mention both forms, got: {msg}"
    );
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("[Fix 3]"));
    assert!(
        help.contains("Shorten the lifetime"),
        "expected canonical Fix 1 text, got: {help}"
    );
}

#[test]
fn e2500_not_send_cannot_cross_boundary_format() {
    let err = ConcurrencyError::NotSendCannotCrossBoundary {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `&0 T` or `&- T` to `&+ T`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Wrap type in `Actor<T>`"));
}

#[test]
fn e2510_scope_ref_leakage_format() {
    let err = ConcurrencyError::ScopeRefLeakage { span: dummy_span() };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Remove escaping assignment"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Change `&0 T` to `&+ T`"));
}

#[test]
fn e2520_mutable_share_anti_pattern_format() {
    let err = ConcurrencyError::MutableShareAntiPattern { span: dummy_span() };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change shared mutable state to Actor messaging"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Remove concurrency boundaries"));
}

// ===== v0.8.x.completion.3: error code namespace verification =====
// Guard against namespace regression like v0.8.x.review.2 (triet::borrow::E25XX
// was incorrectly used for ConcurrencyError before fix). These tests verify
// code() string matches ADR-0026 (actor::) for ConcurrencyError and ADR-0025
// (borrow::) for BorrowError. Without these, future refactors could silently
// re-mistag.

#[test]
fn e2400_code_uses_borrow_namespace() {
    let err = BorrowError::BorrowLifetimeInferenceFailed {
        ty: "T".to_string(),
        span: dummy_span(),
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::borrow::E2400");
}

#[test]
fn e2500_code_uses_actor_namespace() {
    let err = ConcurrencyError::NotSendCannotCrossBoundary {
        ty: "T".to_string(),
        span: dummy_span(),
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(
        code, "triet::actor::E2500",
        "ConcurrencyError must use triet::actor::E25XX per ADR-0026 v2 \
         (NOT triet::borrow::E25XX — see v0.8.x.review.2 fix)"
    );
}

#[test]
fn e2510_code_uses_actor_namespace() {
    let err = ConcurrencyError::ScopeRefLeakage { span: dummy_span() };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::actor::E2510");
}

#[test]
fn e2520_code_uses_actor_namespace() {
    let err = ConcurrencyError::MutableShareAntiPattern { span: dummy_span() };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::actor::E2520");
}

// ===== v0.9.x.atomic.1: E1040 AtomicValue diagnostic format =====

#[test]
fn e1040_non_atomic_value_type_format() {
    let err = TypeError::NonAtomicValueType {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `Atomic<String>` to `Atomic<Integer>`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Mutex<String>"));
    assert!(help.contains("[Fix 3]"));
    assert!(help.contains("Long (81-trit)"));
}

#[test]
fn e1040_code_uses_typecheck_namespace() {
    let err = TypeError::NonAtomicValueType {
        ty: "T".to_string(),
        span: dummy_span(),
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::typecheck::E1040");
}

// ===== v0.9.x.atomic.6: E2530 InvalidAtomicOrdering diagnostic format =====

#[test]
fn e2530_invalid_atomic_ordering_format() {
    let err = ConcurrencyError::InvalidAtomicOrdering {
        success: "Relaxed".to_string(),
        failure: "Strict".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `success=Relaxed` to `success=Strict`"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Change `failure=Strict` to `failure=Relaxed`"));
    assert!(help.contains("[Fix 3]"));
    assert!(help.contains("Change both to `Synchronized`"));
}

#[test]
fn e2530_code_uses_actor_namespace() {
    let err = ConcurrencyError::InvalidAtomicOrdering {
        success: "Relaxed".to_string(),
        failure: "Strict".to_string(),
        span: dummy_span(),
    };
    let code = err.code().unwrap().to_string();
    assert_eq!(code, "triet::actor::E2530");
}

//! Integration tests for diagnostic formats.
use miette::Diagnostic;
use std::ops::Range;
use triet_typecheck::{BorrowError, ConcurrencyError};

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
    let err = BorrowError::CannotMutateFrozenOwner {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `&0 String` to `&- String`"));
}

#[test]
fn e2411_cannot_promote_frozen_to_mutable_format() {
    let err = BorrowError::CannotPromoteFrozenToMutable {
        ty: "String".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Change `&0 String` to `&- String`"));
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
    let err = BorrowError::BorrowExclusivityViolation {
        name: "bar".to_string(),
        span: dummy_span(),
    };
    let help = err.help().unwrap().to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("Move the immutable borrow out of scope"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("Move the mutation statement later"));
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

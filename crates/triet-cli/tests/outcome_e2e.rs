//! End-to-end tests for Outcome syntax (v0.7.4.3-error.3b, ADR-0020).
//!
//! Each test threads a `.tri` source string through the full pipeline:
//! parse → modules-resolve → typecheck → lower → VM execute. The VM's
//! return value is asserted against the expected string form.
//!
//! Covers the four AST shapes added in v0.7.4.3-error.1 (Outcome
//! constructors `~+ / ~0 / ~-`, postfix `~?` propagate, postfix `~:`
//! default, `.unwrap_value(msg)` / `.unwrap_error(msg)` methods) plus
//! `match` arms over outcome scrutinees.
//!
//! All programs are self-contained — no stdlib imports — so the tests
//! exercise lowering only, with no resolver side-effects.

// Most snippets embed `"..."` for unwrap message arguments, so the
// `r#"..."#` form is needed for those — apply consistently across the
// file (clippy::pedantic flags the few that don't strictly need it).
#![allow(clippy::needless_raw_string_hashes)]

use miette::Diagnostic;
use triet_ir::{FuncId, RuntimeValue, Vm, lower_program};

/// Parse + resolve + lower a single-source program, returning the
/// VM's result from calling `main()` with no arguments.
fn run(source: &str) -> RuntimeValue {
    let resolved = triet_modules::load_program_from_source(source)
        .expect("parse + resolve should succeed");
    // Typecheck for parity with the CLI — surfaces regressions early.
    // Warnings (W2001 NullDeprecated) are allowed; hard errors fail.
    let errors = triet_typecheck::check_resolved(&resolved);
    let hard: Vec<_> = errors
        .iter()
        .filter(|e| e.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(hard.is_empty(), "typecheck failed:\n{hard:#?}");
    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .map_or(FuncId(0), |f| f.id);
    let mut vm = Vm::new(ir);
    vm.execute(main_id, Vec::new()).expect("VM should not error")
}

// ── Constructors ────────────────────────────────────────────────────

#[test]
fn outcome_positive_constructor_unwrap_value_yields_payload() {
    let source = r#"
        function main() -> Integer = {
            let outcome: Integer~String = ~+ 42
            outcome.unwrap_value("expected success")
        }
    "#;
    assert_eq!(run(source).to_string(), "42");
}

#[test]
fn outcome_negative_constructor_unwrap_error_yields_payload() {
    let source = r#"
        function main() -> String = {
            let outcome: Integer~String = ~- "boom"
            outcome.unwrap_error("expected failure")
        }
    "#;
    assert_eq!(run(source).to_string(), "boom");
}

#[test]
fn outcome_null_constructor_on_ternary_outcome() {
    // T?~E null arm — match dispatches to the ~0 branch.
    let source = r#"
        function main() -> Integer = {
            let outcome: Integer?~String = ~0
            match outcome {
                ~+ value => value,
                ~0 => -1,
                ~- _ => 0
            }
        }
    "#;
    assert_eq!(run(source).to_string(), "-1");
}

// ── Default operator (~:) ───────────────────────────────────────────

#[test]
fn outcome_default_returns_payload_on_success() {
    let source = r#"
        function main() -> Integer = {
            let outcome: Integer~String = ~+ 7
            outcome ~: 99
        }
    "#;
    assert_eq!(run(source).to_string(), "7");
}

#[test]
fn outcome_default_returns_default_on_failure() {
    let source = r#"
        function main() -> Integer = {
            let outcome: Integer~String = ~- "oops"
            outcome ~: 99
        }
    "#;
    assert_eq!(run(source).to_string(), "99");
}

// ── Propagate operator (~?) with explicit capture ───────────────────

// NOTE: ADR-0020 §3.1 examples use `return ~- err` as the divergent
// early-return form. `return` is currently a Stmt (not Expr), so the
// re-emit form `~- err` is used instead — the lowerer treats the
// failure arm of `~?` as terminating (emits `Ret` over the early-
// return value), so functionally identical for now. Unbraced `return`
// in expression position is deferred per ADR-0019 Addendum §A7.
#[test]
fn outcome_propagate_passes_through_success_payload() {
    let source = r#"
        function compute() -> Integer~String = ~+ 100
        function main() -> Integer~String = {
            let value: Integer = compute() ~? |err| ~- err
            ~+ value
        }
    "#;
    let result = run(source);
    assert_eq!(result.to_string(), "~+(100)");
}

#[test]
fn outcome_propagate_early_returns_on_failure() {
    let source = r#"
        function compute() -> Integer~String = ~- "io_failure"
        function main() -> Integer~String = {
            let value: Integer = compute() ~? |err| ~- err
            ~+ value
        }
    "#;
    let result = run(source);
    // Failure arm Ret-s before reaching ~+ value.
    assert_eq!(result.to_string(), "~-(io_failure)");
}

// ── Pattern matching ────────────────────────────────────────────────

#[test]
fn outcome_match_binds_payload_on_each_arm() {
    let source = r#"
        function classify(outcome: Integer~String) -> String = {
            match outcome {
                ~+ value => "ok",
                ~- error => error
            }
        }
        function main() -> String = {
            classify(~- "disk_full")
        }
    "#;
    assert_eq!(run(source).to_string(), "disk_full");
}

#[test]
fn outcome_match_ternary_three_arms() {
    let source = r#"
        function classify(outcome: Integer?~String) -> Integer = {
            match outcome {
                ~+ value => value,
                ~0 => 0,
                ~- _ => -1
            }
        }
        function main() -> Integer = {
            classify(~+ 42) + classify(~0) + classify(~- "err")
        }
    "#;
    // 42 + 0 + (-1) = 41
    assert_eq!(run(source).to_string(), "41");
}

// ── Unwrap method panic paths ───────────────────────────────────────

#[test]
fn unwrap_value_on_failure_arm_panics_e2210() {
    let source = r#"
        function main() -> Integer = {
            let outcome: Integer~String = ~- "boom"
            outcome.unwrap_value("expected success")
        }
    "#;
    let resolved = triet_modules::load_program_from_source(source).unwrap();
    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .map_or(FuncId(0), |f| f.id);
    let mut vm = Vm::new(ir);
    let err = vm.execute(main_id, Vec::new()).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("E2210") && message.contains("unwrap_value"),
        "expected E2210 with unwrap_value mention, got {message:?}",
    );
}

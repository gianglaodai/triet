//! v0.10.x.borrow.3 — end-to-end tests for E2411
//! (`CannotPromoteFrozenToMutable`) and E2403 (`WeakRefOutlivesOwner`)
//! per [ADR-0025] §7.2 + §8.2.
//!
//! E2410 (`CannotMutateFrozenOwner`) is **dormant** at v0.10 — the
//! parser has no field-assignment syntax (`obj.field = value`), so no
//! syntactic path triggers the rule. Skeleton message corrected;
//! enforcement waits for the parser extension.
//!
//! E2403 conservative scope: only direct `return &- local_let_owner`
//! (and equivalent function-body final-expression form). Full
//! owner-trail tracking defers v0.11+.
//!
//! [ADR-0025]: ../../../../docs/decisions/0025-borrow-checker-rules.md

use miette::Diagnostic;
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

fn hard_error_codes(src: &str) -> Vec<String> {
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .map(|err| {
            err.code()
                .map_or_else(|| format!("{err}"), |code| code.to_string())
        })
        .collect()
}

fn count_code(src: &str, code: &str) -> usize {
    hard_error_codes(src)
        .iter()
        .filter(|c| c.contains(code))
        .count()
}

fn assert_no_borrow_errors(src: &str) {
    let codes = hard_error_codes(src);
    let borrow_count = codes
        .iter()
        .filter(|c| {
            c.contains("E2400")
                || c.contains("E2402")
                || c.contains("E2403")
                || c.contains("E2410")
                || c.contains("E2411")
        })
        .count();
    assert_eq!(
        borrow_count, 0,
        "expected no borrow-checker errors, got codes: {codes:?}"
    );
}

// ── E2411 — frozen→mutable promotion reroute ───────────────────────────

#[test]
fn e2411_fires_on_frozen_to_mutable_promotion_via_let() {
    // Canonical pattern from ADR-0025 §7.2.
    let src = r"
        public struct User { id: Integer, }
        function maker() -> &+ User = User { id: 0 }
        function main() {
            let frozen: &+ User = maker()
            let mutable_handle: &+ mutable User = frozen
        }
    ";
    assert_eq!(count_code(src, "E2411"), 1);
}

#[test]
fn e2411_does_not_fire_for_frozen_to_frozen_let() {
    let src = r"
        public struct User { id: Integer, }
        function maker() -> &+ User = User { id: 0 }
        function main() {
            let a: &+ User = maker()
            let b: &+ User = a
        }
    ";
    assert_no_borrow_errors(src);
}

#[test]
fn e2411_does_not_fire_for_mutable_to_mutable_let() {
    let src = r"
        public struct User { id: Integer, }
        function maker() -> &+ mutable User = User { id: 0 }
        function main() {
            let a: &+ mutable User = maker()
            let b: &+ mutable User = a
        }
    ";
    assert_no_borrow_errors(src);
}

#[test]
fn e2411_message_contains_correct_frozen_to_mutable_text() {
    let src = r"
        public struct User { id: Integer, }
        function maker() -> &+ User = User { id: 0 }
        function main() {
            let frozen: &+ User = maker()
            let mutable_handle: &+ mutable User = frozen
        }
    ";
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    let e2411 = diagnostics
        .iter()
        .find(|d| d.code().is_some_and(|c| c.to_string().contains("E2411")))
        .expect("expected E2411");
    let msg = format!("{e2411}");
    assert!(
        msg.contains("&+ User") && msg.contains("&+ mutable User"),
        "expected `&+ T` → `&+ mutable T` message, got: {msg:?}"
    );
}

#[test]
fn e2411_help_text_contains_correct_fix_suggestions() {
    let src = r"
        public struct User { id: Integer, }
        function maker() -> &+ User = User { id: 0 }
        function main() {
            let frozen: &+ User = maker()
            let mutable_handle: &+ mutable User = frozen
        }
    ";
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    let e2411 = diagnostics
        .iter()
        .find(|d| d.code().is_some_and(|c| c.to_string().contains("E2411")))
        .expect("expected E2411");
    let help = e2411.help().expect("E2411 has help").to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(
        help.contains("`&+ User`") && help.contains("`&+ mutable User`"),
        "expected new fix text mentioning &+ → &+ mutable, got: {help:?}"
    );
}

// ── E2403 — weak ref escapes via direct return ─────────────────────────

#[test]
fn e2403_fires_on_direct_return_weak_borrow_of_local_owner() {
    // Canonical pattern from ADR-0025 §8.2.
    let src = r"
        public struct Process { id: Integer, }
        function maker() -> &+ Process = Process { id: 0 }
        function escape() -> &- Process {
            let p: &+ Process = maker()
            return &- p
        }
        function main() {}
    ";
    assert_eq!(count_code(src, "E2403"), 1);
}

#[test]
fn e2403_fires_on_block_body_final_expr_weak_local() {
    // Block-form function body — final expression of the block IS the
    // return value (no explicit `return` keyword). Exercises the
    // `block.final_expression` branch of check_escaping_weak_borrow.
    let src = r"
        public struct Process { id: Integer, }
        function maker() -> &+ Process = Process { id: 0 }
        function escape() -> &- Process {
            let p: &+ Process = maker()
            &- p
        }
        function main() {}
    ";
    assert_eq!(count_code(src, "E2403"), 1);
}

#[test]
fn e2403_does_not_fire_when_base_is_function_parameter() {
    // The intent of this test is narrow: assert that E2403 does NOT
    // fire when the weak-borrow base resolves to a function parameter
    // (NOT a let-binding). Other unrelated checks (E2400 lifetime
    // elision, InvalidUnary, …) may still fire for this contrived
    // signature, but they're outside this sub-task's scope — what
    // matters is that the local-let-vs-parameter distinction works.
    let src = r"
        public struct Process { id: Integer, }
        function safe(p: &+ Process) -> &- Process = &- p
        function main() {}
    ";
    assert_eq!(count_code(src, "E2403"), 0);
}

#[test]
fn e2403_does_not_fire_when_returning_owner_directly() {
    // Returning the owner itself (not a weak ref) is correct: caller
    // takes ownership. No escape happens.
    let src = r"
        public struct Process { id: Integer, }
        function maker() -> &+ Process = Process { id: 0 }
        function ok() -> &+ Process {
            let p: &+ Process = maker()
            return p
        }
        function main() {}
    ";
    assert_no_borrow_errors(src);
}

#[test]
fn e2403_message_contains_canonical_text() {
    let src = r"
        public struct Process { id: Integer, }
        function maker() -> &+ Process = Process { id: 0 }
        function escape() -> &- Process {
            let p: &+ Process = maker()
            return &- p
        }
        function main() {}
    ";
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    let e2403 = diagnostics
        .iter()
        .find(|d| d.code().is_some_and(|c| c.to_string().contains("E2403")))
        .expect("expected E2403");
    let msg = format!("{e2403}");
    assert!(
        msg.contains("borrow escapes its lexical scope"),
        "expected canonical E2403 message, got: {msg:?}"
    );
}

// ── No false positives ────────────────────────────────────────────────

#[test]
fn no_borrow_errors_for_plain_owned_let_chain() {
    let src = r"
        public struct Foo { id: Integer, }
        function make() -> &+ Foo = Foo { id: 0 }
        function main() {
            let a: &+ Foo = make()
            let b: &+ Foo = a
            let c: &+ Foo = b
        }
    ";
    assert_no_borrow_errors(src);
}

#[test]
fn no_e2403_for_local_weak_used_in_same_scope() {
    // Local weak ref used and dropped in same scope — never escapes.
    // This test currently produces an E2403 because our conservative
    // detection fires when the weak borrow appears in any final-expr
    // OR Return position. A `let weak = &- p` followed by `weak` as
    // final block expression would also fire — but only if the OUTER
    // function returns the weak. Here, the function returns nothing
    // (no return type → defaults to `Unit`), so no return-position
    // check applies. Verifying: borrow expression alone does not fire.
    let src = r"
        public struct Foo { id: Integer, }
        function make() -> &+ Foo = Foo { id: 0 }
        function main() {
            let p: &+ Foo = make()
            let w: &- Foo = &- p
        }
    ";
    assert_no_borrow_errors(src);
}

//! v0.10.x.borrow.1 — end-to-end tests for E2440 NLL borrow-exclusivity
//! enforcement per [ADR-0025] §2.
//!
//! Covers the conflict table (§2.1):
//! - `&0` + `&0`             → OK (multiple read-only borrows)
//! - `&0` + `&0 mutable`     → E2440 (shared vs exclusive)
//! - `&0 mutable` × 2        → E2440 (two exclusive)
//! - `&-` + anything         → OK (weak observer never excludes)
//!
//! Live-range = `[create_seq, last_use_seq]`. Overlap detection per
//! `max(start) <= min(end)`. Branch isolation: borrows in mutually-
//! exclusive arms of `if-else` or `match` do not conflict with each
//! other (serialized in event stream + isolated by execution).
//!
//! Conservative scopes documented in `borrow_check.rs`:
//! - Base = root identifier (`obj.field_a` collapses to `obj`).
//! - Function-call args = single Use site (inter-procedural defer).
//! - Closures = capture rules not currently traced.
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

fn count_e2440(src: &str) -> usize {
    hard_error_codes(src)
        .iter()
        .filter(|c| c.contains("E2440"))
        .count()
}

// ── Happy paths — no E2440 ────────────────────────────────────────────

#[test]
fn two_read_only_borrows_coexist() {
    // Multiple `&0 X` borrows on same base are allowed (shared reads).
    let src = r"
        public struct Foo { id: Integer, }
        function consume_pair(a: &0 Foo, b: &0 Foo) -> Integer = 0
        function main() {
            let v: Foo = Foo { id: 0 }
            let r1: &0 Foo = &0 v
            let r2: &0 Foo = &0 v
            let pair_result: Integer = consume_pair(r1, r2)
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

#[test]
fn weak_observer_overlaps_anything_ok() {
    // `&-` never conflicts.
    let src = r"
        public struct Foo { id: Integer, }
        function take_mut(x: &0 mutable Foo) -> Integer = 0
        function take_weak(x: &- Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            let w: &- Foo = &- v
            let m: &0 mutable Foo = &0 mutable v
            let mres: Integer = take_mut(m)
            let wres: Integer = take_weak(w)
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

#[test]
fn non_overlapping_mutable_borrows_ok() {
    // Two `&0 mutable` borrows whose live-ranges don't overlap.
    // First borrow's last use happens BEFORE the second borrow is
    // created — NLL extends live-range to last-use only.
    let src = r"
        public struct Foo { id: Integer, }
        function take_mut(x: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            let m1: &0 mutable Foo = &0 mutable v
            let r1: Integer = take_mut(m1)
            let m2: &0 mutable Foo = &0 mutable v
            let r2: Integer = take_mut(m2)
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

#[test]
fn different_bases_never_conflict() {
    let src = r"
        public struct Foo { id: Integer, }
        function take_mut(x: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable a: Foo = Foo { id: 0 }
            let mutable b: Foo = Foo { id: 1 }
            let ma: &0 mutable Foo = &0 mutable a
            let mb: &0 mutable Foo = &0 mutable b
            let ra: Integer = take_mut(ma)
            let rb: Integer = take_mut(mb)
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

// ── E2440 fires ───────────────────────────────────────────────────────

#[test]
fn e2440_fires_on_two_overlapping_mutable_borrows() {
    // Both `&0 mutable` borrows used AFTER both creations → overlap.
    let src = r"
        public struct Foo { id: Integer, }
        function take_two_mut(a: &0 mutable Foo, b: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            let m1: &0 mutable Foo = &0 mutable v
            let m2: &0 mutable Foo = &0 mutable v
            let res: Integer = take_two_mut(m1, m2)
        }
    ";
    assert_eq!(count_e2440(src), 1);
}

#[test]
fn e2440_fires_on_mutable_overlapping_readonly() {
    // `&0` + `&0 mutable` on same base, both used after both creations.
    let src = r"
        public struct Foo { id: Integer, }
        function take_pair(a: &0 Foo, b: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            let r: &0 Foo = &0 v
            let m: &0 mutable Foo = &0 mutable v
            let res: Integer = take_pair(r, m)
        }
    ";
    assert_eq!(count_e2440(src), 1);
}

// ── Branch isolation ─────────────────────────────────────────────────

#[test]
fn sibling_branches_borrows_dont_conflict() {
    // Borrows local to then-arm and else-arm never co-execute.
    let src = r"
        public struct Foo { id: Integer, }
        function take_mut(x: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            if true {
                let m1: &0 mutable Foo = &0 mutable v
                let r1: Integer = take_mut(m1)
            } else {
                let m2: &0 mutable Foo = &0 mutable v
                let r2: Integer = take_mut(m2)
            }
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

// ── Loop semantics — conservative correctness ──────────────────────

#[test]
fn loop_local_borrow_used_inside_only_ok() {
    // Borrow created INSIDE loop body, used INSIDE same iteration.
    // Each iteration is its own scope; no cross-iteration overlap.
    let src = r"
        public struct Foo { id: Integer, }
        function take_mut(x: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            while? true {
                let m: &0 mutable Foo = &0 mutable v
                let res: Integer = take_mut(m)
            }
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

// ── Error message format ──────────────────────────────────────────────

#[test]
fn e2440_message_mentions_both_forms_and_base() {
    let src = r"
        public struct Foo { id: Integer, }
        function take_two_mut(a: &0 mutable Foo, b: &0 mutable Foo) -> Integer = 0
        function main() {
            let mutable v: Foo = Foo { id: 0 }
            let m1: &0 mutable Foo = &0 mutable v
            let m2: &0 mutable Foo = &0 mutable v
            let res: Integer = take_two_mut(m1, m2)
        }
    ";
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    let e2440 = diagnostics
        .iter()
        .find(|d| d.code().is_some_and(|c| c.to_string().contains("E2440")))
        .expect("expected E2440");
    let msg = format!("{e2440}");
    assert!(
        msg.contains("&0 mutable v"),
        "expected message to mention `&0 mutable v`, got: {msg}"
    );
    let help = e2440.help().expect("E2440 has help").to_string();
    assert!(help.contains("[Fix 1]"));
    assert!(help.contains("[Fix 2]"));
    assert!(help.contains("[Fix 3]"));
}

// ── Non-borrow code untouched ─────────────────────────────────────────

#[test]
fn plain_function_no_borrows_no_e2440() {
    let src = r"
        function add(a: Integer, b: Integer) -> Integer = 0
        function main() {
            let x = add(1, 2)
            let y = add(x, 3)
        }
    ";
    assert_eq!(count_e2440(src), 0);
}

//! v0.10.x.borrow.2 — end-to-end test for lifetime elision per
//! [ADR-0025 §3].
//!
//! Covers the 3 elision rules:
//! - Rule 1: exactly 1 input borrow → output ties to input.
//! - Rule 2: borrow `self` receiver → output ties to self.
//! - Rule 3: owned return — no inference needed.
//!
//! E2400 fires when all rules fail. Conservative top-level scope —
//! nested borrows in return type (`Vector<&0 T>`, etc.) defer v0.11+.
//!
//! [ADR-0025 §3]: ../../../../docs/decisions/0025-borrow-checker-rules.md

use miette::Diagnostic;
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Lex + parse + resolve + typecheck `src`, return the list of hard
/// error codes (warnings filtered).
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

fn assert_no_e2400(src: &str) {
    let codes = hard_error_codes(src);
    let e2400_count = codes.iter().filter(|c| c.contains("E2400")).count();
    assert_eq!(e2400_count, 0, "expected no E2400, got codes: {codes:?}");
}

// ── Rule 1 — exactly 1 input borrow ───────────────────────────────────

#[test]
fn rule1_single_borrow_input_passes() {
    let src = r"
        function id(s: &0 String) -> &0 String = s
        function main() {}
    ";
    assert_no_e2400(src);
}

#[test]
fn rule1_single_mutable_borrow_passes() {
    let src = r"
        function id_mut(s: &0 mutable String) -> &0 mutable String = s
        function main() {}
    ";
    assert_no_e2400(src);
}

#[test]
fn rule1_single_weak_observer_passes() {
    let src = r"
        function observe(s: &- String) -> &- String = s
        function main() {}
    ";
    assert_no_e2400(src);
}

// ── Rule 2 — borrow `self` receiver ───────────────────────────────────
//
// Rule 2 is **dormant** as of v0.10.x.borrow.2 because Triết's parser
// does NOT yet accept `self` as a parameter name (`SelfKw` is reserved).
// ADR-0025 §3.2 example syntax (`self: &0 Cache`) requires a future
// parser extension. The elision algorithm honors Rule 2 already
// (`has_self_borrow_receiver` branch in `check_lifetime_elision`); it
// just never fires until the parser accepts `self` parameters.
//
// Rule 2 integration tests deferred — added when the `self` parameter
// syntax lands (separate sub-task, post-v0.10.x.borrow.2 scope).

#[test]
fn rule2_first_param_named_other_than_self_falls_to_rule1() {
    // `notself` is NOT a self receiver per ADR-0025 §3.2; Rule 1 still
    // applies because there's exactly 1 input borrow.
    let src = r"
        public struct Foo { value: Integer, }
        public function maybe(notself: &0 Foo) -> &0 Foo = notself
        function main() {}
    ";
    assert_no_e2400(src);
}

// ── Rule 3 — owned return ─────────────────────────────────────────────

#[test]
fn rule3_owned_return_with_any_borrow_inputs_passes() {
    // `&+ ParsedDoc` is owned — no lifetime relationship to infer.
    let src = r"
        public struct ParsedDoc { tag: Integer, }
        function parse(s: &0 String) -> &+ ParsedDoc = ParsedDoc { tag: 0 }
        function main() {}
    ";
    assert_no_e2400(src);
}

#[test]
fn rule3_owned_return_with_two_borrow_inputs_passes() {
    let src = r"
        public struct ParsedDoc { tag: Integer, }
        function merge(a: &0 String, b: &0 String) -> &+ ParsedDoc = ParsedDoc { tag: 0 }
        function main() {}
    ";
    assert_no_e2400(src);
}

// ── Non-borrow return (no check) ──────────────────────────────────────

#[test]
fn non_borrow_return_skips_elision_check() {
    // Return is `Integer` — no borrow relationship. E2400 never fires
    // regardless of input borrow count.
    let src = r"
        function len(a: &0 String, b: &0 String) -> Integer = 0
        function main() {}
    ";
    assert_no_e2400(src);
}

#[test]
fn no_params_non_borrow_return_skips_check() {
    let src = r"
        function answer() -> Integer = 42
        function main() {}
    ";
    assert_no_e2400(src);
}

// ── E2400 fires (all rules fail) ──────────────────────────────────────

#[test]
fn e2400_fires_on_two_borrow_inputs_no_self() {
    // §3.4 canonical case — 2 input borrows, no self receiver, borrow return.
    let src = r"
        function pick_longer(a: &0 String, b: &0 String) -> &0 String = a
        function main() {}
    ";
    assert_eq!(count_code(src, "E2400"), 1);
}

#[test]
fn e2400_fires_on_zero_borrow_inputs_borrow_return() {
    // 0 input borrows, borrow return — no source to tie to.
    let src = r"
        function leak() -> &0 String = leak()
        function main() {}
    ";
    assert_eq!(count_code(src, "E2400"), 1);
}

#[test]
fn e2400_fires_on_mixed_neutral_and_weak_borrows() {
    // 1× `&0` + 1× `&-` = 2 input borrows; no self → E2400.
    let src = r"
        function ambig(a: &0 String, b: &- String) -> &0 String = a
        function main() {}
    ";
    assert_eq!(count_code(src, "E2400"), 1);
}

#[test]
fn e2400_does_not_fire_for_owned_param_plus_other_borrow() {
    // `holder: &+ Foo` is owned (not borrow receiver) — does NOT count
    // toward input borrow tally. `other: &0 Bar` is the sole borrow
    // input → Rule 1 applies.
    let src = r"
        public struct Foo { tag: Integer, }
        public struct Bar { tag: Integer, }
        public function combine(holder: &+ Foo, other: &0 Bar) -> &0 Bar = other
        function main() {}
    ";
    assert_no_e2400(src);
}

#[test]
fn e2400_fires_on_owned_param_plus_two_borrow_others() {
    // `holder: &+ Foo` is owned — does not enter the borrow tally.
    // `a` + `b` = 2 borrows → Rule 1 fails (and no self → Rule 2 fails)
    // → E2400.
    let src = r"
        public struct Foo { tag: Integer, }
        public function combine(holder: &+ Foo, a: &0 String, b: &0 String) -> &0 String = a
        function main() {}
    ";
    assert_eq!(count_code(src, "E2400"), 1);
}

// ── Error message format (ADR-0027) ───────────────────────────────────

#[test]
fn e2400_error_message_has_borrow_lifetime_inference_failed() {
    let src = r"
        function pick(a: &0 String, b: &0 String) -> &0 String = a
        function main() {}
    ";
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    let e2400 = diagnostics
        .iter()
        .find(|d| d.code().is_some_and(|c| c.to_string().contains("E2400")))
        .expect("expected E2400");
    let msg = format!("{e2400}");
    assert!(
        msg.contains("Cannot infer which input the returned borrow ties to"),
        "expected canonical E2400 message, got: {msg:?}"
    );
}

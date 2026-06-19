//! ADR-0064 §8 — compile-time scalar-match exhaustiveness (Campaign
//! Typecheck-Exhaustiveness, Mục 1).
//!
//! A `match` on Integer/Trilean/Trit that omits a value without a catch-all
//! is non-exhaustive → E1026 at typecheck, instead of relying on the lower
//! GAP-2 runtime trap.
//!
//! The Variable-binding catch-all (`other =>`, decision #2) is verified HERE
//! rather than via an integration fixture: the lower does not yet support a
//! variable pattern in a scalar match (it refuses with "unsupported pattern
//! in Integer match"), so a `// EXPECT:` run-fixture cannot exercise it. This
//! test pins the typecheck behavior directly.

use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Run check + return error displays (empty == clean).
fn check_codes(src: &str) -> Vec<String> {
    let resolved = load_program_from_source(src).expect("load");
    let diagnostics = check_resolved(&resolved);
    diagnostics.iter().map(|e| format!("{e}")).collect()
}

fn has_e1026(errors: &[String]) -> bool {
    errors.iter().any(|e| e.contains("E1026"))
}

// ── Positive: catch-all forms suppress E1026 ──

#[test]
fn integer_variable_binding_is_catch_all() {
    // `other =>` binds the value — a catch-all per ADR-0064 §8 decision #2.
    let src = r"
        function classify(x: Integer) -> Integer = match x {
            1 => 10,
            other => other,
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(
        !has_e1026(&errors),
        "variable binding is a catch-all — no E1026 expected; got: {errors:#?}"
    );
}

#[test]
fn integer_wildcard_is_catch_all() {
    let src = r"
        function classify(x: Integer) -> Integer = match x {
            1 => 10,
            _ => 99,
        }
        function main() {}
    ";
    let errors = check_codes(src);
    assert!(
        !has_e1026(&errors),
        "wildcard is a catch-all — no E1026 expected; got: {errors:#?}"
    );
}

// ── Negative: missing arm without catch-all → E1026 ──

#[test]
fn integer_without_catch_all_is_non_exhaustive() {
    let src = r"
        function classify(x: Integer) -> Integer = match x {
            1 => 10,
            2 => 20,
        }
        function main() {}
    ";
    assert!(
        has_e1026(&check_codes(src)),
        "Integer match without a wildcard must raise E1026"
    );
}

#[test]
fn trilean_missing_unknown_is_non_exhaustive() {
    let src = r"
        function f(t: Trilean) -> Integer = match t {
            true => 1,
            false => 2,
        }
        function main() {}
    ";
    assert!(
        has_e1026(&check_codes(src)),
        "Trilean match missing `unknown` must raise E1026"
    );
}

#[test]
fn trit_missing_zero_is_non_exhaustive() {
    let src = r"
        function g(tr: Trit) -> Integer = match tr {
            -1_trit => 1,
            1_trit => 3,
        }
        function main() {}
    ";
    assert!(
        has_e1026(&check_codes(src)),
        "Trit match missing `0_trit` must raise E1026"
    );
}

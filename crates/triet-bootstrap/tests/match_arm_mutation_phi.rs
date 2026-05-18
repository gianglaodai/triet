//! v0.7.4.3-debt.5 — integration tests for WA-1 (match-arm mutation
//! phi-merge + bare-identifier-as-enum-variant pattern resolution).
//!
//! Pre-fix two bugs interacted:
//!
//! 1. `lower_match_expr` did not phi-merge mutable outer-scope vars
//!    across arms. Each arm's rebind statically overwrote the
//!    outer-scope binding, so the lowerer's `resolve_var` after the
//!    match returned the LAST arm's SSA value regardless of which
//!    arm ran at runtime.
//! 2. Bare-identifier patterns (`Pattern::Variable(name)`) where
//!    `name` is a known unit-enum variant were treated as catch-all
//!    variable bindings instead of variant tag checks. Pre-fix this
//!    was latently masked by bug 1: even though the wrong arm ran,
//!    the static-last-write semantics often coincidentally pointed
//!    `end` / `suffix` / etc. at the SSA value the correct arm
//!    would have produced (because that value was actually computed
//!    BEFORE the match started, e.g. `word_end` in `lex_decimal_integer`).
//!
//! Fix:
//! - `lower_match_expr` now phi-merges mutable vars per arm (mirroring
//!   `lower_if_expr`).
//! - `lower_pattern_test` + `bind_pattern_vars` rewrite
//!   `Pattern::Variable(name)` to an `EnumVariant` tag check when
//!   `name` is in `variant_index`.

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

fn run_source(source: &str) {
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    assert!(diagnostics.is_empty(), "type errors: {diagnostics:#?}");

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv");
    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main()")
        .id;
    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("VM must run main() to completion");
}

/// Match-arm phi-merge: a `while` loop that calls `push` inside one
/// arm of a `match`. Pre-fix this lost the Vector mutation across
/// iterations.
#[test]
fn while_match_arm_push_preserves_vector_across_iterations() {
    let source = r"
        from std.collections.vector import new, push, length
        from std.assert import assert

        enum Mode {
            NormalMode,
            OtherMode,
        }

        function main() {
            let mutable bag: Vector<Integer> = new()
            let mutable cursor: Integer = 0
            let m: Mode = NormalMode
            while cursor < 3 {
                match m {
                    NormalMode => {
                        bag = push(bag, cursor)
                        cursor = cursor + 1
                    },
                    OtherMode => {
                        cursor = cursor + 1
                    },
                }
            }
            assert(length(bag) == 3)
        }
    ";
    run_source(source);
}

/// Bare unit-variant identifier in pattern position. Pre-fix this
/// dispatched ALL inputs to arm 0 because `Pattern::Variable` was a
/// catch-all.
#[test]
fn bare_unit_variant_pattern_dispatches_correctly() {
    let source = r"
        from std.assert import assert

        enum E { A, B }

        function classify(e: E) -> Integer = {
            match e {
                A => 1,
                B => 99,
            }
        }

        function main() {
            assert(classify(A) == 1)
            assert(classify(B) == 99)
        }
    ";
    run_source(source);
}

/// Match arm body rebinds an outer mutable, then post-match read
/// must observe the executed arm's rebind. Without phi-merge the
/// post-match read would return the LAST-statically-lowered arm's
/// value regardless of which arm ran.
#[test]
fn match_arm_rebind_observable_after_match() {
    let source = r"
        from std.assert import assert

        enum E { A, B }

        function process(e: E) -> Integer = {
            let mutable result: Integer = 0
            match e {
                A => {
                    result = 1
                },
                B => {
                    result = 99
                },
            }
            result
        }

        function main() {
            assert(process(A) == 1)
            assert(process(B) == 99)
        }
    ";
    run_source(source);
}

/// Wildcard arm WITH outer-scope mutations across both arms — phi
/// must merge the post-arm values pre-fix produced random values
/// because the static-last-write semantic picked the wildcard arm's
/// rebind even when arm 0 ran.
#[test]
fn wildcard_arm_with_mutations_phi_merges() {
    let source = r"
        from std.assert import assert

        enum E { A, B, C }

        function process(e: E) -> Integer = {
            let mutable result: Integer = 0
            match e {
                A => {
                    result = 1
                },
                _ => {
                    result = 99
                },
            }
            result
        }

        function main() {
            assert(process(A) == 1)
            assert(process(B) == 99)
            assert(process(C) == 99)
        }
    ";
    run_source(source);
}

/// Original `lex_decimal_integer` pattern: match on suffix-of-digit-run
/// where arm 0 is the unit `NoSuffix` and arm 1 is wildcard that
/// rebinds outer mutables (`suffix` + `end`). Mirrors the lexer-port
/// regression that surfaced this debt sub-task.
#[test]
fn lex_decimal_integer_pattern_matches_correctly() {
    let source = r"
        from std.assert import assert

        enum NumericSuffix { NoSuffix, TritSuffix }

        function suffix_for(name: String) -> NumericSuffix = {
            if name == 'trit' { TritSuffix }
            else { NoSuffix }
        }

        function process(input: String) -> Integer = {
            let mutable end: Integer = 0
            let mutable suffix: NumericSuffix = NoSuffix
            let candidate: NumericSuffix = suffix_for(input)
            match candidate {
                NoSuffix => { },
                _ => {
                    suffix = candidate
                    end = 42
                },
            }
            end
        }

        function main() {
            assert(process('trit') == 42)
            assert(process('xyz') == 0)
        }
    "
    .replace('\'', "\"");
    run_source(&source);
}

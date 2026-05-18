//! v0.7.4.3-debt.2 — integration test for the WA-2 fix.
//!
//! Pre-fix: field access on a struct value extracted from a `T~E`
//! outcome via `~?` resolved field names alphabetically instead of
//! declared order. The fix wires `value_outcome_value_struct` /
//! `func_return_outcome_value_struct` so the success-arm payload
//! carries the same struct identity tracking as a direct call result.
//!
//! Reproducer mirrors the lexer-port pattern that surfaced the bug:
//! a struct with `spanned` (non-Integer) FIRST and `new_cursor`
//! (Integer) SECOND, returned wrapped in `Outcome`. Pre-fix
//! `step.new_cursor` returned the SpannedToken-shaped runtime value
//! because `n` sorts before `s`; post-fix it returns the correct
//! Integer field.

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Build + VM-run a source program. The source's own `assert(...)`
/// calls do the correctness check — VM panic on assertion failure
/// surfaces as E2205 from `vm.execute`.
fn run_source(source: &str) {
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    // Test source uses `~0` exclusively, so no W2001 warnings to
    // filter. Treat any diagnostic as a failure.
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

/// Canonical reproducer: struct with non-alphabetical declared field
/// order, extracted from a `T~E` via `~?`, then accessed by name.
#[test]
fn field_access_after_outcome_unwrap_uses_declared_order() {
    // `Step { spanned, new_cursor }` declared order is NOT
    // alphabetical (`n` < `s`). Pre-fix, `step.new_cursor` would
    // return the Inner-shaped first field instead of the Integer
    // second field, and the assert would fail with E2201
    // TypeMismatch at runtime.
    let source = r"
        from std.assert import assert

        struct Inner {
            tag: Integer,
            label: Integer,
        }

        struct Step {
            spanned: Inner,
            new_cursor: Integer,
        }

        public function make_step() -> Step~Integer = {
            ~+ Step {
                spanned: Inner { tag: 11, label: 22 },
                new_cursor: 999,
            }
        }

        public function caller() -> Integer~Integer = {
            let step: Step = make_step() ~? |err| ~- err
            assert(step.new_cursor == 999)
            assert(step.spanned.tag == 11)
            ~+ step.new_cursor
        }

        function main() {
            let r: Integer = caller() ~: 0
            assert(r == 999)
        }
    ";
    run_source(source);
}

/// Match-arm `OutcomeUnwrapValue` path — `match outcome { ~+ x => ... }`
/// must carry the struct identity onto the bound `x` so `x.field`
/// resolves declared order.
#[test]
fn field_access_after_match_outcome_arm_uses_declared_order() {
    let source = r"
        from std.assert import assert

        struct Item {
            label: Integer,
            id: Integer,
        }

        public function lookup() -> Item~Integer = {
            ~+ Item { label: 42, id: 7 }
        }

        function main() {
            match lookup() {
                ~+ found => {
                    assert(found.label == 42)
                    assert(found.id == 7)
                },
                ~- err => {
                    assert(false)
                },
            }
        }
    ";
    run_source(source);
}

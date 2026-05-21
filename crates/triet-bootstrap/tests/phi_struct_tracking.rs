//! v0.7.5.4a — integration tests for five pre-existing
//! `value_struct_types` / `value_outcome_value_struct` propagation
//! gaps in the lowerer that surfaced wiring `parse_program` into
//! `compiler/parser.tri`:
//!
//! 1. **While-loop phi.** `let mutable state: T = …; while … {
//!    state = step.state }` — the phi at the loop header didn't
//!    inherit pre-loop struct identity, so post-loop
//!    `state.field` fell back to `field_idx=0`. Fix:
//!    pre-propagate the pre-loop value's struct identity onto
//!    `phi_dest` BEFORE the body lowers, mirroring the
//!    user-declared `let mutable name: T` contract (rebinds must
//!    preserve T).
//!
//! 2. **Match-arm mutated-var phi.** `match … { … => state =
//!    step.state, _ => {} }` — the per-arm mutated-var phi at
//!    the merge didn't propagate struct identity when every arm
//!    agreed. Fix: same shared-identity pattern as
//!    `lower_match_expr`'s expression-merge phi.
//!
//! 3. **Match-expression merge phi.** `match … { ~+ … => ~+ X,
//!    _ => ~+ X }` — the expression `merge_dest` didn't carry
//!    `value_struct_types` or `value_outcome_value_struct` when
//!    every arm agreed. Fix: parallel to `lower_outcome_default`
//!    + `lower_while_loop`'s phi propagation.
//!
//! 4. **If-expression merge phi.** `if cond { ~+ X } else { ~+ X
//!    }` — same two-incoming merge as match, same gap. Fix: same
//!    propagation pattern.
//!
//! 5. **Outcome constructor literal-side propagation.** `~+
//!    StructValue { … }` — the constructed `Outcome`'s
//!    `value_outcome_value_struct` slot wasn't seeded from the
//!    payload's struct identity, so a subsequent `~?` unwrap
//!    dropped the identity. Fix: literal-side analogue of the
//!    call-site `func_return_outcome_value_struct` seeding.
//!
//! 6. **Let-with-type-annotation seeding.** `let p: T = get(v,
//!    i)!!` — the `Vector<T>` element extraction is opaque to
//!    the lowerer (T-generic), so `p` ended up untracked even
//!    though the user wrote the type. Fix: when the value didn't
//!    already have struct tracking, seed from the let's
//!    `type_annotation`.

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

/// Pin #1: while-loop phi inherits pre-loop struct identity, so
/// post-loop field access on a `mutable` variable rebound across
/// iterations still finds the right field.
#[test]
fn while_phi_preserves_struct_identity_across_mutable_rebind() {
    let source = r"
        from std.assert import assert

        struct Counter {
            value: Integer,
            limit: Integer,
        }

        function step(c: Counter) -> Counter =
            Counter { value: c.value + 1, limit: c.limit }

        function main() {
            let mutable c: Counter = Counter { value: 0, limit: 3 }
            while c.value < c.limit {
                c = step(c)
            }
            assert(c.value == 3)
            assert(c.limit == 3)
        }
    ";
    run_source(source);
}

/// Pin #2 + #3: match-arm mutated-var phi propagates identity
/// when every arm produces the same struct, AND the match
/// expression's own `merge_dest` inherits identity when every arm's
/// body resolves to the same struct.
#[test]
fn match_phis_preserve_struct_identity_across_arms() {
    let source = r"
        from std.assert import assert

        struct Choice {
            tag: Integer,
            payload: Integer,
        }

        function pick(which: Integer) -> Choice =
            match which {
                0 => Choice { tag: 0, payload: 100 },
                _ => Choice { tag: 1, payload: 200 },
            }

        function main() {
            let a: Choice = pick(0)
            let b: Choice = pick(1)
            assert(a.tag == 0)
            assert(a.payload == 100)
            assert(b.tag == 1)
            assert(b.payload == 200)
        }
    ";
    run_source(source);
}

/// Pin #4: if-expression merge propagates identity when both
/// branches produce the same struct.
#[test]
fn if_merge_preserves_struct_identity() {
    let source = r"
        from std.assert import assert

        struct Box {
            value: Integer,
        }

        function pick(flag: Trilean!) -> Box =
            if flag {
                Box { value: 10 }
            } else {
                Box { value: 20 }
            }

        function main() {
            assert(pick(true).value == 10)
            assert(pick(false).value == 20)
        }
    ";
    run_source(source);
}

/// Pin #5: `~+ StructValue` Outcome constructor seeds
/// `value_outcome_value_struct`, so a `~?` unwrap of the
/// constructed Outcome recovers the struct identity. (Pre-fix the
/// caller's `~+ StructLit { … }` lost tracking the moment it was
/// wrapped, so the success-arm unwrap had nothing to propagate.)
#[test]
fn outcome_constructor_literal_propagates_struct_identity() {
    let source = r"
        from std.assert import assert

        struct ParseResult {
            value: Integer,
            cursor: Integer,
        }

        function build() -> ParseResult~Integer =
            ~+ ParseResult { value: 99, cursor: 7 }

        function consume() -> ParseResult~Integer = {
            let r: ParseResult = build() ~? |err| ~- err
            ~+ ParseResult { value: r.value + 1, cursor: r.cursor + 1 }
        }

        function main() {
            let outcome: ParseResult~Integer = consume()
            match outcome {
                ~+ r => {
                    assert(r.value == 100)
                    assert(r.cursor == 8)
                },
                ~- _ => assert(false),
            }
        }
    ";
    run_source(source);
}

/// Pin #6: `let p: T = get(v, i)!!` seeds `p` with the let's
/// declared struct type, so the field access on a Vector element
/// resolves correctly. Pre-fix: `get` is generic so its dest had
/// no struct tracking, and the let lowering ignored the user's
/// annotation.
#[test]
fn let_annotation_seeds_struct_tracking_for_vector_elements() {
    let source = r"
        from std.assert import assert
        from std.collections.vector import new, push, length, get

        struct Cell {
            kind: Integer,
            payload: Integer,
        }

        function sum_cells(cells: Vector<Cell>) -> Integer = {
            let mutable total: Integer = 0
            let mutable i: Integer = 0
            let n: Integer = length(cells)
            while i < n {
                let c: Cell = get(cells, i)!!
                total = total + c.payload
                i = i + 1
            }
            total
        }

        function main() {
            let mutable cells: Vector<Cell> = new()
            cells = push(cells, Cell { kind: 0, payload: 10 })
            cells = push(cells, Cell { kind: 1, payload: 20 })
            cells = push(cells, Cell { kind: 0, payload: 30 })
            assert(sum_cells(cells) == 60)
        }
    ";
    run_source(source);
}

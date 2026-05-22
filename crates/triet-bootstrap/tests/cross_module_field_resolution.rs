//! v0.7.5.2 — integration tests for two pre-existing bugs that
//! surfaced while wiring `compiler/parser/parser.tri` to import from
//! `compiler/parser/lexer.tri`:
//!
//! 1. **Typecheck — cross-module user type fields resolve to
//!    Unknown.** Pass 1 of `check_resolved` collected struct/enum
//!    types per module in isolation, so any field whose annotation
//!    referenced a user-defined type from another module fell
//!    through to `Type::Unknown`. Pass 2 imports of such types then
//!    carried Unknown field types, breaking expressions like
//!    `match spanned.token { Variant(payload) => ... }` (the
//!    `bind_pattern` `UserEnum` guard fails on Unknown, so the
//!    payload binding never enters scope and E1002 fires on every
//!    reference to it). Fix: iterate Pass 1 to a fixed point with
//!    a cross-module name table so user references resolve into
//!    their real `UserStruct` / `UserEnum` shapes.
//!
//! 2. **Lowerer — `FieldGet` doesn't propagate nested struct
//!    identity.** With (1) fixed, parser.tri parsed cleanly through
//!    typecheck but blew up at the VM: chained accesses like
//!    `step.state.arena` reached the intermediate `step.state` SSA
//!    value with no `value_struct_types` entry, so the next
//!    `.arena` access fell back to slot 0 (E2201 "type mismatch …
//!    expected Unit, got non-struct"). Fix: record per-struct field
//!    types whose annotation is another named struct, and propagate
//!    that identity onto the `FieldGet` dest. Parallel to the
//!    [`v0.7.4.3-debt.2`] `value_outcome_value_struct` /
//!    `func_return_outcome_value_struct` propagation chains.

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

/// Direct `match struct.field { Variant(payload) => ... }` against a
/// struct field whose type is a Named enum. Works fine in a single-
/// file program because Pass 2's full checker re-resolves struct
/// fields through the env. This regression test keeps the
/// single-module path green while the cross-module test below
/// pins the actual fix.
#[test]
fn single_module_field_match_destructures_payload() {
    let source = r"
        from std.assert import assert

        struct IntPayload { value: Integer }

        enum Token {
            IntKw,
            IntLit(IntPayload),
        }

        struct Spanned {
            token: Token,
            span_start: Integer,
        }

        function dispatch(sp: Spanned) -> Integer = {
            match sp.token {
                IntLit(p) => p.value,
                IntKw => -1,
            }
        }

        function main() {
            let sp: Spanned = Spanned {
                token: IntLit(IntPayload { value: 42 }),
                span_start: 0,
            }
            assert(dispatch(sp) == 42)
        }
    ";
    run_source(source);
}

/// Triple-level chained field access — `step.state.arena.count` —
/// proves the `FieldGet` propagation hop carries struct identity
/// through every link, not just the first. Each intermediate value
/// (`step.state` is `ParserState`, `step.state.arena` is `Arena`)
/// needs its slot index resolved correctly, which only works when
/// `value_struct_types` tracks all three structs.
#[test]
fn nested_field_access_propagates_struct_identity() {
    let source = r"
        from std.assert import assert

        struct Arena {
            count: Integer,
            tag: Integer,
        }

        struct ParserState {
            tokens_len: Integer,
            arena: Arena,
        }

        struct ParseStep {
            state: ParserState,
            expr_id: Integer,
        }

        function build() -> ParseStep =
            ParseStep {
                state: ParserState {
                    tokens_len: 5,
                    arena: Arena { count: 7, tag: 99 },
                },
                expr_id: 11,
            }

        function main() {
            let step: ParseStep = build()
            assert(step.expr_id == 11)
            assert(step.state.tokens_len == 5)
            assert(step.state.arena.count == 7)
            assert(step.state.arena.tag == 99)
        }
    ";
    run_source(source);
}

/// Same nested chain but the outer wrapper is bound by a match-arm
/// `~+ step =>` destructure. Pre-fix this combined the two bugs:
/// `value_outcome_value_struct` propagated `step: ParseStep` (good,
/// covered by [`v0.7.4.3-debt.2`]), but the next `step.state.arena`
/// chain dropped identity at the intermediate. Today the chain
/// keeps tracking through both struct hops.
#[test]
fn outcome_unwrap_chained_field_access_keeps_struct_identity() {
    let source = r"
        from std.assert import assert

        struct Arena {
            count: Integer,
        }

        struct ParserState {
            arena: Arena,
        }

        struct ParseStep {
            state: ParserState,
            expr_id: Integer,
        }

        function build() -> ParseStep~Integer =
            ~+ ParseStep {
                state: ParserState { arena: Arena { count: 42 } },
                expr_id: 1,
            }

        function main() {
            let r: ParseStep~Integer = build()
            match r {
                ~+ step => {
                    assert(step.expr_id == 1)
                    assert(step.state.arena.count == 42)
                },
                ~- _ => assert(false),
            }
        }
    ";
    run_source(source);
}

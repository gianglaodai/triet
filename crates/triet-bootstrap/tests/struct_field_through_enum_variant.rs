//! v0.7.5.1 — integration test for the enum-variant payload struct
//! field-access fix.
//!
//! Pre-fix: `match e { Variant(p) => p.field }` always read slot 0
//! of `p` regardless of which field was named, because
//! `bind_pattern_vars` for `Pattern::EnumVariant` extracted the
//! payload via `EnumPayload` but never set `value_struct_types` on
//! the bound SSA value. Surfaced while drafting the
//! `compiler/parser.tri` arena scaffolding (`BinaryOp(p) =>
//! format_expr(a, p.left)` returned the operator enum, not the
//! left-operand ID, blowing up downstream `get_expression`).
//!
//! Fix: Pass 1a.2 populates `variant_payload_struct` (`variant_name`
//! → `struct_name`) for variants whose payload is a Named struct;
//! `bind_pattern_vars` (`EnumVariant` arm) propagates that onto the
//! `EnumPayload` dest. Parallel to the `OutcomeArm` path covered by
//! `struct_field_through_outcome.rs` (v0.7.4.3-debt.2).

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

/// Canonical reproducer: a struct with three Integer fields wrapped
/// in an enum variant payload. Pre-fix, all three `p.*` accesses
/// returned the value of the alphabetically-first field; post-fix
/// each returns its declared-order value.
#[test]
fn field_access_after_enum_variant_match_uses_declared_order() {
    let source = r"
        from std.assert import assert

        struct BinOp {
            op_idx: Integer,
            left: Integer,
            right: Integer,
        }

        enum Expr {
            Bin(BinOp),
        }

        function main() {
            let payload: BinOp = BinOp { op_idx: 7, left: 100, right: 200 }
            let e: Expr = Bin(payload)
            match e {
                Bin(p) => {
                    assert(p.op_idx == 7)
                    assert(p.left == 100)
                    assert(p.right == 200)
                },
            }
        }
    ";
    run_source(source);
}

/// The lexer-port pattern that surfaced this: heterogeneous field
/// types (enum + Integer + Integer). Pre-fix, both `p.left` and
/// `p.right` returned the `BinaryOperator` enum because slot 0 held
/// the operator, even though the field name asked for the Integer
/// slots.
#[test]
fn field_access_through_enum_payload_with_mixed_field_types() {
    let source = r"
        from std.assert import assert

        enum Op { OpAdd, OpMul }

        struct BinOp {
            operator: Op,
            left: Integer,
            right: Integer,
        }

        enum Expr {
            IntLit(Integer),
            Binary(BinOp),
        }

        function main() {
            let payload: BinOp = BinOp { operator: OpMul, left: 42, right: 84 }
            let e: Expr = Binary(payload)
            match e {
                IntLit(_) => {
                    assert(false)
                },
                Binary(p) => {
                    assert(p.left == 42)
                    assert(p.right == 84)
                    match p.operator {
                        OpAdd => assert(false),
                        OpMul => assert(true),
                    }
                },
            }
        }
    ";
    run_source(source);
}

/// Nested arena pattern: enum variant holds an index, and the body
/// uses that index to look up another enum value via `get`. Mirrors
/// `compiler/parser.tri`'s `BinaryOp(p) => format_expr(a, p.left)`
/// recursive lookup.
#[test]
fn enum_payload_index_drives_arena_lookup() {
    let source = r"
        from std.assert import assert
        from std.collections.vector import new, push, length, get
        from std.io import println

        struct BinPayload {
            left: Integer,
            right: Integer,
        }

        enum Node {
            Leaf(Integer),
            Bin(BinPayload),
        }

        function main() {
            let mutable nodes: Vector<Node> = new()
            nodes = push(nodes, Leaf(10))
            nodes = push(nodes, Leaf(20))
            nodes = push(nodes, Bin(BinPayload { left: 0, right: 1 }))

            let top: Node = get(nodes, 2)!!
            match top {
                Leaf(_) => assert(false),
                Bin(p) => {
                    assert(p.left == 0)
                    assert(p.right == 1)
                    let l: Node = get(nodes, p.left)!!
                    let r: Node = get(nodes, p.right)!!
                    match l {
                        Leaf(v) => assert(v == 10),
                        Bin(_) => assert(false),
                    }
                    match r {
                        Leaf(v) => assert(v == 20),
                        Bin(_) => assert(false),
                    }
                },
            }
        }
    ";
    run_source(source);
}

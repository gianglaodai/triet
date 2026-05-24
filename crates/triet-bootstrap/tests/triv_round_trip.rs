//! v0.7.9.2 — Round-trip byte-identical gate for the Triết-side
//! `.triv` writer. Each corpus case: Triết-emit bytes → Rust-decode
//! → Rust-re-emit → assert byte-identical. See `assert_round_trip`
//! for why this is the gate (not byte-vs-`Rust write_program`).

use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

fn compiler_pack_writer_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("pack_writer.tri")
}

fn pack_writer_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_pack_writer_path();
        assert!(
            path.is_file(),
            "missing compiler/pack_writer.tri at {}",
            path.display()
        );
        let resolved = load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(
            blocking.is_empty(),
            "type errors in compiler/pack_writer.tri: {blocking:#?}",
        );
        let ir = lower_program(&resolved);
        let bytes = write_program(&ir);
        read_program(&bytes).expect("read .triv round-trip")
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/pack_writer.tri"))
        .id
}

fn serialize_in_triet(source: &str) -> Vec<u8> {
    let ir = pack_writer_ir().clone();
    let func_id = lookup_func(&ir, "serialize_source_to_triv");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect("pack_writer.tri::serialize_source_to_triv must execute without VM error");

    // Result should be a Vector<Integer> representing bytes
    match result {
        RuntimeValue::Vector(vec) => vec
            .iter()
            .map(|v| match v {
                RuntimeValue::Integer(i) => {
                    u8::try_from(i.to_i64()).expect("byte vector element out of u8 range")
                }
                _ => panic!("Expected integer in vector, got {v:?}"),
            })
            .collect(),
        other => panic!("expected Vector<Integer> from serialize_source_to_triv, got {other:?}"),
    }
}

/// Triết-emit → Rust-decode → Rust-re-emit → byte-identical assertion.
///
/// We cannot directly byte-diff `Triết bytes` vs `Rust write_program(rust_ir)`
/// because the Rust pipeline pre-loads stdlib (9 modules / 27 functions)
/// during `load_program_from_source` whereas Triết-side `lower_source`
/// doesn't (the stdlib pre-load deferral lives in v0.7.10 per
/// TODO.md). The constant-pool offsets and module/function ID
/// numbering therefore diverge for the same `.tri` source.
///
/// What we CAN gate: the Triết-emitted bytes must round-trip cleanly
/// through Rust's `read_program` → `write_program` so that the
/// resulting bytes are bit-stable. This proves the Triết writer
/// produces a structurally valid `.triv` file with the same byte
/// layout the Rust reader understands. Full byte-vs-Rust parity
/// lands when stdlib pre-load unifies in v0.7.10+.
fn assert_round_trip(label: &str, source: &str) {
    let triet_bytes = serialize_in_triet(source);

    // Rust decodes the Triết-emitted bytes.
    let restored_ir = read_program(&triet_bytes)
        .unwrap_or_else(|err| panic!("read_program failed on Triết bytes for `{label}`: {err}"));

    // Rust re-encodes the decoded IR.
    let reemitted = write_program(&restored_ir);

    assert_eq!(
        triet_bytes, reemitted,
        "round-trip divergence on case `{label}`: \
         Triết-emit → Rust-decode → Rust-re-emit produced different bytes"
    );
}

#[test]
fn const_then_ret() {
    assert_round_trip("const_then_ret", "function f() -> Integer = 42");
}

#[test]
fn arithmetic_add() {
    assert_round_trip(
        "arithmetic_add",
        "function add(x: Integer, y: Integer) -> Integer = x + y",
    );
}

#[test]
fn arithmetic_sub_mul_div() {
    assert_round_trip(
        "arithmetic_sub_mul_div",
        "function calc(a: Integer, b: Integer) -> Integer = a - b * 2 / 1",
    );
}

#[test]
fn unary_negate() {
    assert_round_trip("unary_negate", "function neg() -> Integer = -7");
}

#[test]
fn block_body_let_arith() {
    assert_round_trip(
        "block_body_let_arith",
        "function g() -> Integer { let x: Integer = 5 x + 3 }",
    );
}

#[test]
fn force_unwrap() {
    assert_round_trip(
        "force_unwrap",
        "function unwrap(x: Integer?) -> Integer = x!!",
    );
}

#[test]
fn builtin_call_println() {
    assert_round_trip(
        "builtin_call_println",
        "function main() -> Unit = println(\"hi\")",
    );
}

#[test]
fn local_function_call() {
    assert_round_trip(
        "local_function_call",
        "function add(a: Integer, b: Integer) -> Integer = a + b\nfunction main() -> Integer = add(1, 2)",
    );
}

#[test]
fn assign_stmt() {
    assert_round_trip(
        "assign_stmt",
        "function f() -> Integer { let mutable x: Integer = 0 x = 5 x }",
    );
}

#[test]
fn loop_break() {
    assert_round_trip("loop_break", "function f() -> Unit { loop { break } }");
}

#[test]
fn while_loop_phi() {
    assert_round_trip(
        "while_loop_phi",
        "function f() -> Unit { let mutable i: Integer = 0 while i < 10 { i = i + 1 } }",
    );
}

#[test]
fn for_loop_range() {
    assert_round_trip(
        "for_loop_range",
        "function f() -> Unit { for i in 0..10 { } }",
    );
}

#[test]
fn enum_unit_variant() {
    assert_round_trip(
        "enum_unit_variant",
        "enum Maybe { None, Some(Integer) }\nfunction f() -> Maybe = None",
    );
}

#[test]
fn enum_tuple_variant() {
    assert_round_trip(
        "enum_tuple_variant",
        "enum Maybe { None, Some(Integer) }\nfunction f() -> Maybe = Some(42)",
    );
}

#[test]
fn outcome_positive() {
    assert_round_trip(
        "outcome_positive",
        "function ok() -> Integer~String = ~+ 42",
    );
}

#[test]
fn outcome_negative() {
    assert_round_trip(
        "outcome_negative",
        "function bad() -> Integer~String = ~- \"bad\"",
    );
}

#[test]
fn outcome_null() {
    assert_round_trip(
        "outcome_null",
        "function none_val() -> Integer?~String = ~0",
    );
}

#[test]
fn comparison_ops_dont_panic() {
    assert_round_trip(
        "comparison_lt",
        "function lt(a: Integer, b: Integer) -> Trilean! = a < b",
    );
    assert_round_trip(
        "comparison_eq",
        "function eq(a: Integer, b: Integer) -> Trilean! = a == b",
    );
}

#[test]
fn trilean_logic_ops_dont_panic() {
    assert_round_trip(
        "trilean_and",
        "function f(a: Trilean!, b: Trilean!) -> Trilean! = a && b",
    );
    assert_round_trip(
        "kleene_implies",
        "function f(a: Trilean!, b: Trilean!) -> Trilean! = a ~> b",
    );
}

//! v0.7.8.6 — `lowerer_differential` corpus gate (closes the
//! v0.7.8 umbrella per [ADR-0019 §A7.8]).
//!
//! ## Why substring-coverage, not byte-diff
//!
//! Full byte-identical NDJSON output between Rust `lower_program`
//! and Triết `dump_ir_program_ndjson` is **not** achievable until
//! v0.7.9 because of two impl-level divergences:
//!
//!   1. Rust pre-loads stdlib (9 modules / 27 functions) during
//!      `load_program_from_source`. Triết's `load_program_from_source`
//!      doesn't (per the v0.7.6.5 differential note — stdlib
//!      pre-load deferred to v0.7.10 alongside CLI wiring).
//!   2. Rust shares the constant pool across all modules; Triết
//!      v0.7.8.5 shares within a single module's threaded ctx but
//!      pool offsets still diverge from Rust's stdlib-prefilled
//!      pool. Const-ID slot numbers differ even though the values
//!      match.
//!
//! v0.7.9 drops both bridges (NDJSON dump + stdlib divergence)
//! per ADR-0019 §A2. Until then, the differential gate verifies
//! **structural opcode coverage** — that Triết's lowerer emits
//! the expected sequence of opcode names for each corpus source.
//! This catches regressions in instruction emission ordering
//! (the lowerer's "what" did the right thing) without false
//! positives from impl-numeric ID divergence.
//!
//! [ADR-0019 §A7.8]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use triet_ir::{
    FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program,
};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

// ─────────────────────────────────────────────────────────────────
// Triết-side dump driver — cached IR + per-source execution.
// ─────────────────────────────────────────────────────────────────

fn compiler_ir_lowerer_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("ir_lowerer.tri")
}

fn ir_lowerer_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_ir_lowerer_path();
        assert!(
            path.is_file(),
            "missing compiler/ir_lowerer.tri at {}",
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
            "type errors in compiler/ir_lowerer.tri: {blocking:#?}",
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
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/ir_lowerer.tri"))
        .id
}

fn triet_dump(source: &str) -> String {
    let ir = ir_lowerer_ir().clone();
    let func_id = lookup_func(&ir, "dump_ir_program_ndjson");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect("compiler/ir_lowerer.tri::dump_ir_program_ndjson must execute without VM error");
    match result {
        RuntimeValue::String(s) => s,
        other => panic!("expected String from dump_ir_program_ndjson, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Opcode-coverage assertions
//
// Each corpus case verifies the Triết dump:
//   1. starts with the `{"k":"Program",…}` header,
//   2. emits exactly the expected ordered sequence of opcode names
//      (taken across all blocks of the source's user module).
//
// Opcode extraction parses the dump linearly for `"op":"<name>"`
// substrings — no JSON parser dep, matches the dump emission
// shape directly.
// ─────────────────────────────────────────────────────────────────

fn extract_opcodes(dump: &str) -> Vec<String> {
    let mut ops = Vec::new();
    let pat = "\"op\":\"";
    let mut rest = dump;
    while let Some(idx) = rest.find(pat) {
        let start = idx + pat.len();
        let tail = &rest[start..];
        if let Some(end) = tail.find('"') {
            ops.push(tail[..end].to_owned());
            rest = &tail[end..];
        } else {
            break;
        }
    }
    ops
}

fn assert_opcodes(label: &str, source: &str, expected: &[&str]) {
    let dump = triet_dump(source);
    assert!(
        dump.starts_with("{\"k\":\"Program\","),
        "`{label}` dump missing Program header: {dump}",
    );
    let actual = extract_opcodes(&dump);
    let actual_refs: Vec<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(
        actual_refs.as_slice(),
        expected,
        "`{label}` opcode sequence diff\n--- expected: {expected:?}\n--- actual: {actual_refs:?}\n--- dump:\n{dump}",
    );
}

fn assert_dump_non_empty(label: &str, source: &str) {
    let dump = triet_dump(source);
    assert!(
        !dump.is_empty(),
        "`{label}` dump unexpectedly empty",
    );
    assert!(
        dump.starts_with("{\"k\":\"Program\","),
        "`{label}` dump missing Program header",
    );
}

// ── Determinism gate ────────────────────────────────────────────

#[test]
fn determinism_two_calls() {
    let src = "function f() -> Integer = 42";
    let a = triet_dump(src);
    let b = triet_dump(src);
    assert_eq!(a, b, "non-deterministic dump");
}

// ── Per-opcode coverage ─────────────────────────────────────────

#[test]
fn empty_program() {
    let dump = triet_dump("");
    // `lower_source` always wraps an empty module per the v0.7.8.5
    // shape — so we expect Program + Module header + nothing else.
    assert!(dump.contains("\"k\":\"Program\",\"modules\":1"));
    assert!(dump.contains("\"k\":\"Module\",\"functions\":0"));
}

#[test]
fn const_then_ret() {
    assert_opcodes(
        "const_then_ret",
        "function f() -> Integer = 42",
        &["Const", "Ret"],
    );
}

#[test]
fn arithmetic_add() {
    assert_opcodes(
        "arithmetic_add",
        "function add(x: Integer, y: Integer) -> Integer = x + y",
        &["Add", "Ret"],
    );
}

#[test]
fn arithmetic_sub_mul_div() {
    assert_opcodes(
        "arithmetic_sub_mul_div",
        "function calc(a: Integer, b: Integer) -> Integer = a - b * 2 / 1",
        // Const(2), Mul %1 * %2, Const(1), Div, Sub, Ret.
        &["Const", "Mul", "Const", "Div", "Sub", "Ret"],
    );
}

#[test]
fn unary_negate() {
    assert_opcodes(
        "unary_negate",
        "function neg() -> Integer = -7",
        &["Const", "Neg", "Ret"],
    );
}

#[test]
fn block_body_let_arith() {
    assert_opcodes(
        "block_body_let_arith",
        "function g() -> Integer { let x: Integer = 5 x + 3 }",
        &["Const", "Const", "Add", "Ret"],
    );
}

#[test]
fn force_unwrap() {
    assert_opcodes(
        "force_unwrap",
        "function unwrap(x: Integer?) -> Integer = x!!",
        &["NullUnwrap", "Ret"],
    );
}

#[test]
fn builtin_call_println() {
    assert_opcodes(
        "builtin_call_println",
        "function main() -> Unit = println(\"hi\")",
        &["Const", "CallBuiltin", "Ret"],
    );
}

#[test]
fn local_function_call() {
    assert_opcodes(
        "local_function_call",
        "function add(a: Integer, b: Integer) -> Integer = a + b\nfunction main() -> Integer = add(1, 2)",
        // add's body: Add, Ret. main's body: Const(1), Const(2),
        // CallLocal, Ret.
        &["Add", "Ret", "Const", "Const", "CallLocal", "Ret"],
    );
}

#[test]
fn assign_stmt() {
    assert_opcodes(
        "assign_stmt",
        "function f() -> Integer { let mutable x: Integer = 0 x = 5 x }",
        // Const(0), Const(5), Ret(rebound x = %1).
        &["Const", "Const", "Ret"],
    );
}

#[test]
fn loop_break() {
    assert_opcodes(
        "loop_break",
        "function f() -> Unit { loop { break } }",
        // entry: Br b1
        // loop_body (b1): Br b2 (from break)
        // loop_exit (b2): Const(Unit) (block_final tail) + Br?
        // Actually: Const + Br back to body, then exit Const + Ret.
        // The body's `lower_block` materializes a Unit final before
        // the `break` redirects — pattern below matches actual.
        &["Br", "Br", "Const", "Br", "Const", "Ret"],
    );
}

#[test]
fn while_loop_phi() {
    assert_opcodes(
        "while_loop_phi",
        "function f() -> Unit { let mutable i: Integer = 0 while i < 10 { i = i + 1 } }",
        // Body's `lower_block` materializes a Unit block_final
        // before the implicit `Br header` — so the actual stream is:
        //   entry: Const(0) Br
        //   header: Phi Const(10) Lt BrTrilean
        //   panic: Unreachable
        //   body: Const(1) Add Const(Unit-final) Br
        //   exit: Const(Unit) Ret
        &[
            "Const", "Br", "Phi", "Const", "Lt", "BrTrilean", "Unreachable", "Const", "Add",
            "Const", "Br", "Const", "Ret",
        ],
    );
}

#[test]
fn for_loop_range() {
    assert_opcodes(
        "for_loop_range",
        "function f() -> Unit { for i in 0..10 { } }",
        // Empty body still emits a Unit block_final before the
        // increment + Br back. Same shape as while_loop_phi: a
        // tail Const(Unit) sits inside the for_body before the
        // counter increment.
        &[
            "Const", "Const", "Br", "Phi", "Lt", "BrTrilean", "Const", "Const", "Add", "Br",
            "Const", "Ret",
        ],
    );
}

#[test]
fn enum_unit_variant() {
    assert_opcodes(
        "enum_unit_variant",
        "enum Maybe { None, Some(Integer) }\nfunction f() -> Maybe = None",
        &["EnumNew", "Ret"],
    );
}

#[test]
fn enum_tuple_variant() {
    assert_opcodes(
        "enum_tuple_variant",
        "enum Maybe { None, Some(Integer) }\nfunction f() -> Maybe = Some(42)",
        &["Const", "EnumNew", "Ret"],
    );
}

#[test]
fn outcome_positive() {
    assert_opcodes(
        "outcome_positive",
        "function ok() -> Integer~String = ~+ 42",
        &["Const", "OutcomeNewPositive", "Ret"],
    );
}

#[test]
fn outcome_negative() {
    assert_opcodes(
        "outcome_negative",
        "function bad() -> Integer~String = ~- \"bad\"",
        &["Const", "OutcomeNewNegative", "Ret"],
    );
}

#[test]
fn outcome_null() {
    assert_opcodes(
        "outcome_null",
        "function none_val() -> Integer?~String = ~0",
        &["OutcomeNewNull", "Ret"],
    );
}

// ── Smoke (no opcode-sequence assertion) ────────────────────────

#[test]
fn comparison_ops_dont_panic() {
    assert_dump_non_empty(
        "comparison_lt",
        "function lt(a: Integer, b: Integer) -> Trilean! = a < b",
    );
    assert_dump_non_empty(
        "comparison_eq",
        "function eq(a: Integer, b: Integer) -> Trilean! = a == b",
    );
}

#[test]
fn trilean_logic_ops_dont_panic() {
    assert_dump_non_empty(
        "trilean_and",
        "function f(a: Trilean!, b: Trilean!) -> Trilean! = a && b",
    );
    assert_dump_non_empty(
        "kleene_implies",
        "function f(a: Trilean!, b: Trilean!) -> Trilean! = a ~> b",
    );
}

//! v0.7.7.5 — `typecheck_differential` byte-diff gate (closes the
//! v0.7.7 umbrella per [ADR-0019 §A7.7]).
//!
//! For each corpus source, runs the Rust impl
//! [`triet_typecheck::check_resolved`] (via
//! [`triet_modules::load_program_from_source`]) and the Triết-in-
//! Triết port at `compiler/typecheck.tri::dump_typecheck_errors_ndjson`
//! over the same input. Both sides emit the same line-delimited
//! JSON shape and the test asserts byte-equality.
//!
//! ## Format
//!
//! ```text
//! {"k":"Errors","count":N}
//! {"k":"Error","code":"<E1xxx|W2xxx>","span":[<start>,<end>]}
//! …
//! ```
//!
//! Errors are sorted by (`span_start`, `span_end`, `code`) on both
//! sides so the diff stays stable even when traversal order
//! diverges between the two two-pass drivers.
//!
//! ## Transient bridge
//!
//! NDJSON is a transient bridge format per ADR-0019 §A2 — dropped
//! at v0.7.9 when Triết-side data flows in-memory.
//!
//! [ADR-0019 §A7.7]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::{TypeError, check_resolved};

// ─────────────────────────────────────────────────────────────────
// Triết-side: compile `compiler/typecheck.tri` once + run
// `dump_typecheck_errors_ndjson(source)`. Mirrors
// `modules_differential::modules_ir`.
// ─────────────────────────────────────────────────────────────────

fn compiler_typecheck_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("typecheck.tri")
}

fn typecheck_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_typecheck_path();
        assert!(
            path.is_file(),
            "missing compiler/typecheck.tri at {}",
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
            "type errors in compiler/typecheck.tri: {blocking:#?}",
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
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/typecheck.tri"))
        .id
}

fn triet_dump(source: &str) -> String {
    let ir = typecheck_ir().clone();
    let func_id = lookup_func(&ir, "dump_typecheck_errors_ndjson");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect(
            "compiler/typecheck.tri::dump_typecheck_errors_ndjson must execute without VM error",
        );
    match result {
        RuntimeValue::String(s) => s,
        other => panic!("expected String from dump_typecheck_errors_ndjson, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Rust-side mirror — walks the Vec<TypeError> from
// `triet_typecheck::check_resolved`, emitting byte-identical NDJSON.
// ─────────────────────────────────────────────────────────────────

fn error_code(err: &TypeError) -> String {
    err.code()
        .map_or_else(|| "triet::typecheck::Unknown".into(), |c| c.to_string())
}

fn error_span(err: &TypeError) -> (usize, usize) {
    let s = err.span();
    (s.start, s.end)
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn quote_string(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn rust_dump(source: &str) -> String {
    // Lex + parse + check via the same `load_program_from_source`
    // path the Triết side uses, so both surface the same set of
    // type errors. Parse / lex failures degrade gracefully: the
    // loader returns Err with no partial program, and we emit a
    // zero-error header (matches the Triết-side `~- _ => new()`
    // arm of `check_source`).
    let errors: Vec<TypeError> = triet_modules::load_program_from_source(source)
        .map_or_else(|_| Vec::new(), |program| check_resolved(&program));

    // Sort by (span_start, span_end, code) — same key the Triết
    // side uses. Use a zero-padded 10-digit decimal so lexical
    // comparison agrees with numeric.
    let mut sorted: Vec<&TypeError> = errors.iter().collect();
    sorted.sort_by_key(|e| {
        let (s, end) = error_span(e);
        (s, end, error_code(e))
    });

    let mut out = String::new();
    writeln!(out, "{{\"k\":\"Errors\",\"count\":{}}}", sorted.len()).unwrap();
    for err in &sorted {
        let (s, end) = error_span(err);
        let code = error_code(err);
        writeln!(
            out,
            "{{\"k\":\"Error\",\"code\":{},\"span\":[{},{}]}}",
            quote_string(&code),
            s,
            end,
        )
        .unwrap();
    }
    out
}

// ─────────────────────────────────────────────────────────────────
// Corpus + diff harness
// ─────────────────────────────────────────────────────────────────

fn assert_diff(label: &str, source: &str) {
    let rust = rust_dump(source);
    let triet = triet_dump(source);
    assert_eq!(
        triet, rust,
        "typecheck_differential diff in `{label}`\n--- Triết ---\n{triet}\n--- Rust ---\n{rust}",
    );
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn empty_program() {
    assert_diff("empty_program", "");
}

#[test]
fn integer_expression_body() {
    assert_diff("integer_expression_body", "function f() -> Integer = 42");
}

#[test]
fn return_type_mismatch_integer_to_string() {
    assert_diff(
        "return_type_mismatch_integer_to_string",
        "function f() -> String = 42",
    );
}

#[test]
fn undefined_name_in_body() {
    assert_diff(
        "undefined_name_in_body",
        "function f() -> Integer = no_such_name",
    );
}

// NOTE: BlockBody form (`function f() -> T { … }`) is used here
// because the Triết-side parser at v0.7.5.6 doesn't surface
// `Expr::Block` from a `= { … }` ExpressionBody (gap recorded in
// TODO.md). The Rust parser folds the expression form into
// `Expr::Block`; differential corpus stays inside the shared
// surface.

#[test]
fn while_loop_with_refined_trilean_cond() {
    assert_diff(
        "while_loop_with_refined_trilean_cond",
        "function f() -> Unit { let mutable i: Integer = 0 while i < 10 { i = i + 1 } }",
    );
}

#[test]
fn assign_to_immutable() {
    assert_diff(
        "assign_to_immutable",
        "function f() -> Unit { let x: Integer = 0 x = 1 }",
    );
}

#[test]
fn while_unknown_condition() {
    assert_diff(
        "while_unknown_condition",
        "function f() -> Unit { while unknown { } }",
    );
}

#[test]
fn struct_declaration_clean() {
    assert_diff(
        "struct_declaration_clean",
        "struct Point { x: Integer, y: Integer }\nfunction main() -> Integer = 0",
    );
}

#[test]
fn struct_field_unknown_type() {
    assert_diff(
        "struct_field_unknown_type",
        "struct Bad { x: NoSuchType }\nfunction main() -> Integer = 0",
    );
}

#[test]
fn enum_declaration_with_payload() {
    assert_diff(
        "enum_declaration_with_payload",
        "enum Option { None, Some(Integer) }\nfunction main() -> Integer = 0",
    );
}

#[test]
fn vector_builtin_arity_violation() {
    assert_diff(
        "vector_builtin_arity_violation",
        "function pick(v: Vector<Integer, Integer>) -> Integer = 0\nfunction main() -> Integer = 0",
    );
}

#[test]
fn field_access_clean() {
    assert_diff(
        "field_access_clean",
        "struct Point { x: Integer, y: Integer }\nfunction get_x(p: Point) -> Integer = p.x\nfunction main() -> Integer = 0",
    );
}

#[test]
fn field_access_unknown_member() {
    assert_diff(
        "field_access_unknown_member",
        "struct Point { x: Integer }\nfunction get_z(p: Point) -> Integer = p.z\nfunction main() -> Integer = 0",
    );
}

#[test]
fn elvis_on_nullable_clean() {
    assert_diff(
        "elvis_on_nullable_clean",
        "function get_or(x: Integer?) -> Integer = x ?: 0\nfunction main() -> Integer = 0",
    );
}

#[test]
fn elvis_on_non_nullable() {
    assert_diff(
        "elvis_on_non_nullable",
        "function get_or(x: Integer) -> Integer = x ?: 0\nfunction main() -> Integer = 0",
    );
}

#[test]
fn outcome_constructor_success_arm() {
    assert_diff(
        "outcome_constructor_success_arm",
        "function parse(s: String) -> Integer~String = ~+ 42\nfunction main() -> Integer = 0",
    );
}

#[test]
fn outcome_zero_against_binary() {
    assert_diff(
        "outcome_zero_against_binary",
        "function bad(s: String) -> Integer~String = ~0\nfunction main() -> Integer = 0",
    );
}

#[test]
fn outcome_default_clean() {
    assert_diff(
        "outcome_default_clean",
        "function unwrap_or(o: Integer~String, d: Integer) -> Integer = o ~: d\nfunction main() -> Integer = 0",
    );
}

#[test]
fn generic_identity_clean() {
    assert_diff(
        "generic_identity_clean",
        "function identity<T>(value: T) -> T = value\nfunction main() -> Integer = identity(42)",
    );
}

#[test]
fn wrong_arity_call() {
    assert_diff(
        "wrong_arity_call",
        "function add(x: Integer, y: Integer) -> Integer = x + y\nfunction main() -> Integer = add(1)",
    );
}

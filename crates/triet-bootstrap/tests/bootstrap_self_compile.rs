//! v0.7.9.5 — Self-hosted compiler end-to-end gate.
//!
//! Compiles `compiler/factorial.tri` through both pipelines and
//! asserts the Rust-side toolchain accepts the Triết-side
//! `.khi` output as semantically equivalent:
//!
//!   - **Rust side**: `parse → load_program_from_source_no_stdlib →
//!     lower_program → write_program → AbiMetadata::empty →
//!     write_khi` (the no-stdlib variant exists for parity with
//!     the Triết loader, which doesn't pre-load stdlib until v0.7.10).
//!   - **Triết side**: load `compiler/main.tri` IR once, invoke
//!     `main(["build", source, "-o", out, "--pkg", "compiler"])`,
//!     read back the written `.khi` bytes.
//!
//! Acceptance per Q1-A (decided 2026-05-24): **strict byte-identical**
//! on a stripped fixture. v0.7.9.5 originally landed with a content-
//! equivalent assertion (same module count, function name + arity,
//! block count) because three Triết-side encoder bugs and one
//! lowerer bug surfaced during gate implementation. All four are
//! now closed:
//!
//!   1. `v0.7.x.runtime-fix.struct-in-enum-payload-identity`
//!      (worked around in `compiler/main.tri::SourceResult` —
//!      `SourceLoaded(StringPayload)` lost the String content;
//!      switched to direct `SourceLoaded(String)`).
//!   2. `v0.7.x.runtime-fix.write-function-table-module-prefix`
//!      (Triết-side `lower_module` now seeds `["khi"]` segments).
//!   3. `v0.7.x.runtime-fix.block-emission-order`
//!      (Triết-side `write_code_module_funcs` now sorts blocks by
//!      `id.raw` before serialization, matching Rust's
//!      `BTreeMap<BlockId, BasicBlock>` wire order).
//!   4. `v0.7.x.runtime-fix.while-body-scope-pop-order`
//!      (Triết-side `lower_while_loop` snapshots body-end SSA
//!      values **before** popping the body scope, mirroring
//!      Rust's `lowerer.rs:2191-2197` order).
//!
//! The fixture intentionally uses iterative factorial (`while` +
//! `let mutable`) rather than recursive (`if cond { … } else { … }`)
//! because Triết-side `compiler/ir_lowerer.tri::lower_expression`
//! falls through to a no-op `_ => fresh_value(ctx)` for `IfExpr`,
//! `MatchExpr`, `MethodCallExpr`, `ElvisOpExpr`,
//! `OutcomePropagateExpr`, `OutcomeDefaultExpr`, and `BlockExpr` —
//! the v0.7.8 lowerer port is structurally complete (passed the
//! now-deleted `lowerer_differential` shape gate) but semantically
//! incomplete for these expression variants. Tracked as
//! `v0.7.x.runtime-fix.lowerer-incomplete-expr`.
//!
//! Closes the v0.7.9 umbrella: from this commit forward, Triết-side
//! data flows in-memory across the lex → parse → modules →
//! typecheck → lower → pack-write pipeline with no NDJSON bridge
//! files.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use tempfile::TempDir;
use triet_core::Trit;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, write_program};
use triet_modules::load_program_from_source_no_stdlib;
use triet_pack::{AbiMetadata, SemVer, read_khi, write_khi};
use triet_typecheck::check_resolved;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn factorial_source_path() -> PathBuf {
    workspace_root().join("compiler").join("factorial.tri")
}

fn compiler_main_path() -> PathBuf {
    workspace_root().join("compiler").join("main.tri")
}

/// Cached IR of `compiler/main.tri` — load + typecheck + lower runs
/// once per test invocation across the whole module (~22 kLOC of
/// Triết source). Same `OnceLock` pattern as
/// `triet_in_triet_compile.rs`.
fn main_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_main_path();
        assert!(path.is_file(), "missing main.tri at {}", path.display());
        let resolved = triet_modules::load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(blocking.is_empty(), "type errors: {blocking:#?}");
        lower_program(&resolved)
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}`"))
        .id
}

fn string_vec(items: &[&str]) -> RuntimeValue {
    RuntimeValue::Vector(
        items
            .iter()
            .map(|s| RuntimeValue::String((*s).to_owned()))
            .collect(),
    )
}

/// Drive the Triết-side `main.tri::main` with `build` subcommand
/// over `compiler/factorial.tri`, return the bytes written to the
/// `-o` path. Stages the source into a tempfile so each test owns
/// its own output location.
fn triet_emit_factorial(pkg_name: &str) -> Vec<u8> {
    let source = fs::read_to_string(factorial_source_path()).expect("read factorial.tri");
    let temp = TempDir::new().expect("tempdir");
    let source_path = temp.path().join("factorial.tri");
    let out_path = temp.path().join("factorial.khi");
    fs::write(&source_path, &source).expect("stage factorial source");

    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main");
    let mut vm = Vm::new(ir);

    let source_str = source_path.to_str().expect("UTF-8 path").to_owned();
    let out_str = out_path.to_str().expect("UTF-8 path").to_owned();

    let argv = string_vec(&[
        "build",
        &source_str,
        "-o",
        &out_str,
        "--pkg",
        pkg_name,
    ]);
    let result = vm
        .execute(func_id, vec![argv])
        .expect("main(build) must execute without VM error");
    match result {
        RuntimeValue::Trit(Trit::Positive) => {}
        other => panic!("expected Trit::Positive from main, got {other:?}"),
    }

    fs::read(&out_path).expect("read emitted .khi")
}

/// Mirror the Triết-side `serialize_source_to_khi` shape in
/// Rust: lex+parse+lower a single user module (no stdlib, no
/// typecheck), encode to `.triv`, wrap with empty ABI metadata,
/// emit `.khi`.
fn rust_emit_factorial(source: &str, pkg_name: &str) -> Vec<u8> {
    let resolved = load_program_from_source_no_stdlib(source)
        .expect("no-stdlib loader must accept factorial fixture");
    let ir = lower_program(&resolved);
    let code = write_program(&ir);
    let meta = AbiMetadata::empty(pkg_name, SemVer::new(0, 0, 0));
    write_khi(&meta, &code)
}

/// The umbrella gate. Asserts the Rust-emitted and Triết-emitted
/// `.khi` outputs over `compiler/factorial.tri` are byte-for-
/// byte identical. If this assertion ever fires, the Triết-impl
/// compiler has drifted from the Rust-impl compiler — investigate
/// before bumping the v0.7 phase.
#[test]
fn factorial_self_compile_byte_identical() {
    let source = fs::read_to_string(factorial_source_path()).expect("read factorial.tri");
    let pkg_name = "compiler";

    let rust_bytes = rust_emit_factorial(&source, pkg_name);
    let triet_bytes = triet_emit_factorial(pkg_name);

    if rust_bytes != triet_bytes {
        let first_diff = rust_bytes
            .iter()
            .zip(triet_bytes.iter())
            .position(|(r, t)| r != t)
            .unwrap_or_else(|| rust_bytes.len().min(triet_bytes.len()));
        panic!(
            "byte mismatch: rust={}B triet={}B, first differing byte at offset {first_diff}",
            rust_bytes.len(),
            triet_bytes.len()
        );
    }
}

/// Independent determinism check — two Triết-side runs of the same
/// source must produce identical bytes. Catches non-deterministic
/// `HashMap` iteration / pointer-identity hashing / etc. without
/// depending on the Rust pipeline mirror.
#[test]
fn triet_emit_is_deterministic() {
    let first = triet_emit_factorial("compiler");
    let second = triet_emit_factorial("compiler");
    assert_eq!(first, second, "Triết-side emit must be deterministic");
}

/// Triết-emitted output remains decodable by the Rust reader, with
/// the embedded `.triv` code section starting with the expected
/// magic. Catches wire-format drift independently of the byte-
/// identical gate.
#[test]
fn triet_emit_decodes_via_rust_reader() {
    let bytes = triet_emit_factorial("compiler");
    let (metadata, code_section) =
        read_khi(&bytes).expect("Rust read_khi must accept Triết output");
    assert_eq!(metadata.pkg_name, "compiler");
    assert_eq!(metadata.abi_version, 2);
    assert!(!metadata.iface_hash.is_zero());
    assert!(!metadata.impl_hash.is_zero());
    assert_eq!(
        &code_section[..4],
        b"triv",
        "embedded code section should start with `triv` magic"
    );
}

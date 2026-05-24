//! v0.7.9.4 — Triết-in-Triết compile smoke.
//!
//! Loads `compiler/main.tri`, the self-hosted CLI driver, invokes
//! its `main(argv)` with the `build` subcommand, and asserts the
//! resulting `.tripack` file is decoded cleanly by the Rust
//! `triet_pack::read_tripack`.
//!
//! Per Q2 (decided): minimal end-to-end gate — pipeline runs +
//! Rust decoder accepts the output. Byte-identical comparison with
//! Rust `serialize_source_to_tripack` defers to v0.7.9.5 because
//! Triết still doesn't pre-load the stdlib (deferred to v0.7.10),
//! so the constant-pool offsets and stdlib-side function IDs
//! diverge from the Rust toolchain.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic as _;
use tempfile::TempDir;
use triet_core::Trit;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program};
use triet_logic::Trilean;
use triet_modules::load_program;
use triet_pack::read_tripack;
use triet_typecheck::check_resolved;

fn compiler_main_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("main.tri")
}

/// Load + typecheck + lower `compiler/main.tri` once for the whole
/// test module. The transitive load brings in `parser`, `modules`,
/// `typecheck`, `ir_lowerer`, and `pack_writer` — about half a
/// megabyte of Triết source — so the `OnceLock` cache matters.
fn main_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_main_path();
        assert!(path.is_file(), "missing main.tri at {}", path.display());
        let resolved = load_program(&path).expect("load_program");
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

/// `main_smoke()` exercises `parse_args` + subcommand dispatch
/// without touching the filesystem. Catches structural regressions
/// (`Args` struct field shifts, `classify_subcommand` mis-dispatch)
/// long before the full `main` invocation does.
#[test]
fn main_smoke_dispatches_args_correctly() {
    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main_smoke");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![])
        .expect("main_smoke must execute without VM error");
    match result {
        RuntimeValue::Trilean(Trilean::True) => {}
        other => panic!("expected Trilean::True, got {other:?}"),
    }
}

/// End-to-end gate: build subcommand reads a source file, emits a
/// `.tripack`, and the Rust decoder agrees the result is a valid
/// pack. This is the v0.7.9.4 acceptance bar — full byte-identical
/// comparison is v0.7.9.5 after the stdlib-preload divergence
/// closes.
#[test]
fn main_build_emits_readable_tripack() {
    let temp = TempDir::new().expect("tempdir");
    let source_path = temp.path().join("source.tri");
    let out_path = temp.path().join("out.tripack");
    fs::write(&source_path, "function f() -> Integer = 42").expect("write source");

    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main");
    let mut vm = Vm::new(ir);

    let argv = string_vec(&[
        "build",
        source_path.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
        "--pkg",
        "selfhost_test",
    ]);

    let result = vm
        .execute(func_id, vec![argv])
        .expect("main(build) must execute without VM error");

    match result {
        RuntimeValue::Trit(Trit::Positive) => {}
        other => panic!("expected Trit::Positive from main, got {other:?}"),
    }

    assert!(
        out_path.exists(),
        "main(build) did not write the `.tripack` file at {}",
        out_path.display()
    );

    let bytes = fs::read(&out_path).expect("read out.tripack");
    assert_eq!(
        &bytes[..4],
        b"trip",
        "Triết-emitted file must start with `trip` magic"
    );

    let (metadata, code_section) =
        read_tripack(&bytes).expect("Rust read_tripack must accept Triết main(build) output");
    assert_eq!(metadata.pkg_name, "selfhost_test");
    assert_eq!(
        metadata.abi_version, 2,
        "abi_version stays at 2 per ADR-0014"
    );
    assert!(
        !metadata.iface_hash.is_zero(),
        "iface_hash should be computed BLAKE3, not zero sentinel"
    );
    assert!(
        !metadata.impl_hash.is_zero(),
        "impl_hash should be computed BLAKE3, not zero sentinel"
    );
    assert_eq!(
        &code_section[..4],
        b"triv",
        "embedded code section should be a complete .triv file"
    );
}

/// Unknown subcommand exits with `Trit::Negative` — confirms the
/// dispatcher's error path actually runs (not silently treated as
/// success).
#[test]
fn main_unknown_subcommand_returns_negative() {
    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main");
    let mut vm = Vm::new(ir);

    let argv = string_vec(&["nonsense", "irrelevant.tri"]);
    let result = vm.execute(func_id, vec![argv]).expect("main exec");
    match result {
        RuntimeValue::Trit(Trit::Negative) => {}
        other => panic!("expected Trit::Negative for unknown subcommand, got {other:?}"),
    }
}

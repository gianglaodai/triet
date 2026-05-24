//! v0.7.9.1 — Synthetic round-trip test for `.triv` serialization
//! primitives in `compiler/pack_writer.tri`.
//!
//! Loads `compiler/pack_writer.tri` from the filesystem (so its
//! `from khi.ir_lowerer import …` resolves against the sibling
//! `compiler/ir_lowerer.tri`), compiles it via the full Rust
//! pipeline, then invokes `pack_writer_smoke_main()` inside the VM.
//! The Triết-side `main` builds a synthetic `TypeTag` arena (all 11
//! discriminators) + `ConstantPool` (all 8 variants), encodes →
//! decodes → re-encodes, and asserts byte-identical at every hop.
//! On success the function returns `Outcome::Positive(Trilean::True)`;
//! anything else (Negative outcome, false, panic) fails the test.

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
        // Round-trip the IR through write/read to confirm Rust-side
        // serde itself is healthy. The Triết smoke runs against the
        // restored IR.
        let bytes = write_program(&ir);
        read_program(&bytes).expect("read .triv round-trip on compiler/pack_writer.tri")
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

#[test]
fn pack_writer_smoke_main_returns_positive_true() {
    let ir = pack_writer_ir().clone();
    let func_id = lookup_func(&ir, "pack_writer_smoke_main");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![])
        .expect("pack_writer_smoke_main must execute without VM error");

    // Expected shape: Outcome::Positive(Trilean::True). Anything else
    // — a Negative arm carrying a TrivError, or a Positive arm
    // carrying False — signals a regression in the Triết-side
    // writer/reader symmetry.
    match result {
        RuntimeValue::Outcome {
            discriminator,
            payload,
        } => {
            assert_eq!(
                discriminator,
                triet_core::Trit::Positive,
                "expected Positive outcome, got discriminator {discriminator:?} with payload {payload:?}",
            );
            match payload {
                Some(boxed) => match *boxed {
                    RuntimeValue::Trilean(triet_logic::Trilean::True) => {}
                    other => panic!("Positive outcome but payload is not Trilean::True: {other:?}"),
                },
                None => panic!("Positive outcome with no payload"),
            }
        }
        other => panic!("expected Outcome runtime value, got {other:?}"),
    }
}

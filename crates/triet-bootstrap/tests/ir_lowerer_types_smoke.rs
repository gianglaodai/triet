//! v0.7.8.1 — smoke test for the Triết-in-Triết IR-lowerer's
//! types scaffolding at `compiler/ir_lowerer.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. The Triết-side
//! `main()` constructs every `Instruction` variant + the
//! `TypeTag` / `Constant` / `Operand` / `BuiltinName` / `BasicBlock`
//! / `Function` / `IrModule` / `IrProgram` surfaces and asserts
//! each roundtrips through its display / opcode-name / terminator-
//! detection / well-formedness helpers.
//!
//! The full `lowerer_differential` test (raw `.triv` byte-diff vs
//! the Rust lowerer) lands at v0.7.8.{5,6} after the
//! `lower_program` driver itself ships in v0.7.8.{2,3,4}.
//!
//! See [ADR-0019 §A7.8](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

use std::path::PathBuf;

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

fn compiler_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join(format!("{name}.tri"))
}

#[test]
fn ir_lowerer_types_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("ir_lowerer");
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
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/ir_lowerer.tri smoke main() must complete without VM error");
}

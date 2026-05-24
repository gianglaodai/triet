//! v0.7.11.2 — smoke test for the Triết-in-Triết multi-file
//! `lower_program` driver at `compiler/driver.tri::driver_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `driver_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke covers:
//!
//!   - Hand-built 1-module `ResolvedProgram` lowers via
//!     `lower_program` to an `IrProgram` with one function (`add`).
//!   - The resulting `IrModule.path_segments` is patched from the
//!     source `ModulePath::crate_root()` (`["khi"]`), NOT the
//!     `lower_module_with_imports` placeholder.
//!   - The function body still contains the expected `Add`
//!     instruction (baseline behavior preserved).
//!
//! `driver.tri` lives outside `ir_lowerer.tri` because importing
//! `ResolvedProgram` directly would force a `module modules;`
//! declaration that cycles with `pack_writer.tri`'s `module
//! ir_lowerer;` graph. The driver gets to declare both as siblings.
//!
//! Single-module scope per v0.7.11.2 — true multi-file pipeline
//! activation lifts in v0.7.11.4 when `compiler/main.tri`
//! switches to `load_program_from_source_with_stdlib +
//! check_resolved + lower_program`.
//!
//! See ADR-0019 §A7.11 + v0.7.10 deferral list in TODO.md.

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
fn driver_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("driver");
    assert!(
        path.is_file(),
        "missing compiler/driver.tri at {}",
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
        "type errors in compiler/driver.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("driver_smoke_main"))
        .expect("missing driver_smoke_main() in compiler/driver.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/driver.tri driver_smoke_main() must complete without VM error");
}

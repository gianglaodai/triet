//! v0.7.6.2 — smoke test for the Triết-in-Triết loader logic at
//! `compiler/modules.tri::load_program_from_source`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `loader_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke exercises every loader code path the
//! v0.7.6.2 sub-task ships: empty crate, single-item crate, inline
//! submodule, nested inline submodules, external declaration in
//! in-memory mode (`FileNotFound` expected), sibling inline modules,
//! and a mix of items and modules.
//!
//! Filesystem `load_program` and stdlib pre-loading exercise lands
//! at v0.7.6.5 (`modules_differential` byte-diff gate). Cycle
//! detection (v0.7.6.3) and name resolution (v0.7.6.4) are
//! out-of-scope for this smoke.
//!
//! See [ADR-0019 §A7.6](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

use std::path::PathBuf;

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

fn compiler_modules_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("modules_root.tri")
}

#[test]
fn modules_loader_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_modules_path();
    assert!(
        path.is_file(),
        "missing compiler/modules.tri at {}",
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
        "type errors in compiler/modules.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let loader_smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("loader_smoke_main"))
        .expect("missing loader_smoke_main() in compiler/modules.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(loader_smoke_id, vec![])
        .expect("compiler/modules.tri loader_smoke_main() must complete without VM error");
}

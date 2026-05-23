//! v0.7.6.1 — smoke test for the Triết-in-Triết modules type
//! scaffolding at `compiler/modules.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. The Triết-side
//! `main()` constructs every `ModulePath` / `AbsolutePath` / `Module`
//! / `ResolvedProgram` / `LoaderError` variant and asserts each
//! roundtrips through its display / code / span / message helpers.
//!
//! The full `modules_differential` test (NDJSON `ResolvedProgram`
//! snapshot diff against the Rust impl) lands at v0.7.6.5 after the
//! loader / cycle detector / resolver logic itself ships in
//! v0.7.6.{2,3,4}.
//!
//! See [ADR-0019 §A7.6](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

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
fn modules_types_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("modules_root");
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

    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/modules.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/modules.tri smoke main() must complete without VM error");
}

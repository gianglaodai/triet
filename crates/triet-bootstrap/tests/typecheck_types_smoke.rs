//! v0.7.7.1 — smoke test for the Triết-in-Triết typechecker type
//! scaffolding at `compiler/typecheck.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. The Triết-side
//! `main()` constructs every `TypeKind` variant + `TypeError`
//! variant + exercises `TypeEnvironment` push/pop/declare/lookup
//! and asserts each roundtrips through its display / code / span
//! helpers.
//!
//! The full `typecheck_differential` test (NDJSON `TypeError` list
//! diff against the Rust impl) lands at v0.7.7.5 after the
//! checker logic itself ships in v0.7.7.{2,3,4}.
//!
//! See [ADR-0019 §A7.7](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

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
fn typecheck_types_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("typecheck");
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
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/typecheck.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/typecheck.tri smoke main() must complete without VM error");
}

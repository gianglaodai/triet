//! v0.7.7.2 — smoke test for the Triết-in-Triết typechecker checker
//! at `compiler/typecheck.tri::control_flow_smoke_main`.
//!
//! Builds `compiler/typecheck.tri` to `.triv`, round-trips through
//! the wire reader, then runs `control_flow_smoke_main()` end-to-end
//! on the VM. The Triết-side smoke exercises every code path the
//! v0.7.7.2 sub-task ships: empty programs, expression-body
//! functions, block bodies with `let` + arithmetic, return-type
//! mismatch, undefined names, `while` with refined-Trilean
//! condition, assign-to-immutable, plain `true`/`false` literals
//! typed as `Trilean!`, mixed-numeric arithmetic, unary negate,
//! and the `Trilean` (unknown) condition E1033 path.
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
fn typecheck_control_flow_smoke_main_passes_all_asserts() {
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

    let smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("control_flow_smoke_main"))
        .expect("missing control_flow_smoke_main() in compiler/typecheck.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/typecheck.tri control_flow_smoke_main() must complete without VM error");
}

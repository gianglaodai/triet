//! v0.7.7.3 — smoke test for the Triết-in-Triết typechecker's
//! function-call + generic-inference path at
//! `compiler/typecheck.tri::generic_smoke_main`.
//!
//! Builds `compiler/typecheck.tri` to `.triv`, round-trips through
//! the wire reader, then runs `generic_smoke_main()` end-to-end on
//! the VM. The Triết-side smoke exercises: prelude function calls
//! (`println`, `to_string`), user-defined function calls with
//! correct arity + argument types, wrong-arity error (E1006),
//! argument-type mismatch (E1003), `NotCallable` on non-function
//! values (E1007), single-`TypeParam` generic inference where the
//! return type is unified from the argument, and the post-
//! substitution return-type mismatch case.
//!
//! Structs/enums/patterns/outcome cases ship in v0.7.7.4. The full
//! byte-diff `typecheck_differential` gate lands v0.7.7.5.
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
fn typecheck_generic_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("generic_smoke_main"))
        .expect("missing generic_smoke_main() in compiler/typecheck.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/typecheck.tri generic_smoke_main() must complete without VM error");
}

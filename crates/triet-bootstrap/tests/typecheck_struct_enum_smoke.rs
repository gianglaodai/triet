//! v0.7.7.4 — smoke test for the Triết-in-Triết typechecker's
//! struct + enum + tuple/generic-type + pattern surface at
//! `compiler/typecheck.tri::struct_enum_smoke_main`.
//!
//! Builds `compiler/typecheck.tri` to `.triv`, round-trips through
//! the wire reader, then runs `struct_enum_smoke_main()` end-to-end
//! on the VM. The Triết-side smoke covers:
//!   - `StructItem` / `EnumItem` declaration (pass 1 registration)
//!   - Field-type resolution under a type-param frame
//!   - `TupleType` resolution → `alloc_tuple`
//!   - `GenericType` built-in expansion: `Vector<T>` / `HashMap<K,V>`
//!     as pseudo `UserStruct` shells with `__element` / `__key`,
//!     `__value` field slots (per ADR-0019 Addendum §A7)
//!   - Generic user types: arity check + `TypeParam` substitution
//!   - `bind_pattern` for the For-loop variable binding: identifier
//!     (single binding), wildcard (no binding), with proper
//!     frame push/pop scoping so loop variables don't leak.
//!
//! `MatchExpr`-driven pattern binding (`Tuple` / `Or` / `EnumVariant`
//! / `OutcomeArm` in match arms) defers to v0.7.7.5 once the parser
//! surfaces `Expr::Match` — gap recorded in TODO.md (v0.7.5.6 ent).
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
fn typecheck_struct_enum_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("struct_enum_smoke_main"))
        .expect("missing struct_enum_smoke_main() in compiler/typecheck.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/typecheck.tri struct_enum_smoke_main() must complete without VM error",
    );
}

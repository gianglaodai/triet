//! v0.7.7.4b — smoke test for the Triết-in-Triết typechecker's
//! Outcome / Elvis / Range / `FieldAccess` body-side inference at
//! `compiler/typecheck.tri::outcome_field_smoke_main`.
//!
//! Builds `compiler/typecheck.tri` to `.triv`, round-trips through
//! the wire reader, then runs `outcome_field_smoke_main()` end-to-end
//! on the VM. The Triết-side smoke covers the previously-deferred
//! `infer_unknown` placeholders in `infer_expression`:
//!   - `RangeExpr` → `Range<T>` (endpoint types must match)
//!   - `FieldAccessExpr` → struct field lookup, E1015 on miss
//!   - `ElvisOpExpr` → unwrap `Nullable<T>` to `T`, default must
//!     match, E1012 on non-nullable object
//!   - `OutcomeConstructorExpr` → typed against the surrounding
//!     return-type context (Outcome / Nullable); E1025 for `~0`
//!     against binary `T~E`
//!   - `OutcomeDefaultExpr` (`~:`) → result is value-type
//!     (or `Nullable<value>` when `allow_null_state`)
//!
//! `OutcomePropagateExpr` (`~?`) typecheck logic is wired in
//! `infer_outcome_propagate` mirroring Rust's `check_outcome_propagate`,
//! but its smoke assertion defers to v0.7.7.5 — the capture-binding
//! path hits a Triết-side runtime regression that needs root-cause
//! investigation on the VM side (`declare_with_mut` reports
//! `HashMap` field as Unit at runtime).
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
fn typecheck_outcome_field_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("outcome_field_smoke_main"))
        .expect("missing outcome_field_smoke_main() in compiler/typecheck.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/typecheck.tri outcome_field_smoke_main() must complete without VM error");
}

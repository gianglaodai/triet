//! v0.7.11.1b — smoke test for the Triết-in-Triết IR lowerer's
//! `OutcomeDefaultExpr` (`~:`) arm at
//! `compiler/ir_lowerer.tri::outcome_default_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `outcome_default_smoke_main()` end-to-end on
//! the VM. The Triết-side smoke covers:
//!
//!   - `function f(x: Integer~String) -> Integer = x ~: 0` lowers
//!     to the canonical 4-block shape (entry, `oc_default_success`,
//!     `oc_default_fallback`, `oc_default_merge`).
//!   - Entry ends in `BrTrilean` (preceded by `OutcomeDiscriminant`).
//!     Positive arm → success, Zero/Negative both route to fallback
//!     (matches ADR-0020 §3.3: null and failure share the default
//!     evaluation block).
//!   - `oc_default_success` starts with `OutcomeUnwrapValue`.
//!   - `oc_default_merge` starts with a `Phi` carrying exactly two
//!     incoming edges (success-end + fallback-end).
//!
//! See [ADR-0020] + the v0.7.x.runtime-fix.lowerer-incomplete-expr
//! deferred bug entry in TODO.md.
//!
//! [ADR-0020]: ../../../../docs/decisions/0020-outcome-error-handling.md

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
fn ir_lowerer_outcome_default_smoke_main_passes_all_asserts() {
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

    let smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("outcome_default_smoke_main"))
        .expect("missing outcome_default_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri outcome_default_smoke_main() must complete without VM error");
}

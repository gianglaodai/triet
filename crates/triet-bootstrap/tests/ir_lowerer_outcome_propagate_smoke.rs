//! v0.7.11.1c — smoke test for the Triết-in-Triết IR lowerer's
//! `OutcomePropagateExpr` (`~?`) arm at
//! `compiler/ir_lowerer.tri::outcome_propagate_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `outcome_propagate_smoke_main()` end-to-end
//! on the VM. The Triết-side smoke covers:
//!
//!   - `x ~? |err| ~- err` inside an `Integer~String`-returning
//!     function lowers to the canonical 5-block shape (entry +
//!     `oc_prop_success` / `oc_prop_null` / `oc_prop_failure` /
//!     `oc_prop_merge`).
//!   - Entry's `BrTrilean` has three DISTINCT target blocks (unlike
//!     Elvis / `OutcomeDefault` which collapsed Unknown+False).
//!   - `oc_prop_failure` is divergent (ADR-0020 §3.1) — terminates
//!     via `Ret`, no Br to merge. First instruction is
//!     `OutcomeUnwrapError` to bind the capture name.
//!   - `oc_prop_merge` starts with a `Phi` carrying exactly two
//!     incoming edges (success + null only — failure Ret-ed).
//!
//! See [ADR-0020 §3.1] + the v0.7.x.runtime-fix.lowerer-incomplete-expr
//! deferred bug entry in TODO.md.
//!
//! [ADR-0020 §3.1]: ../../../../docs/decisions/0020-outcome-error-handling.md

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
fn ir_lowerer_outcome_propagate_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("outcome_propagate_smoke_main"))
        .expect("missing outcome_propagate_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/ir_lowerer.tri outcome_propagate_smoke_main() must complete without VM error",
    );
}

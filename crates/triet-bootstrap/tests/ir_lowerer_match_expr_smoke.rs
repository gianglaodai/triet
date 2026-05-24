//! v0.7.11.1f2 — smoke test for the Triết-in-Triết IR lowerer's
//! `MatchExpr` arm at `compiler/ir_lowerer.tri::match_expr_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `match_expr_smoke_main()` end-to-end on the
//! VM. The Triết-side smoke covers:
//!
//!   - 2-arm literal match `match x { 0 => 100, _ => 0 }` lowers to
//!     a 5-block shape (entry + arm-0-body + test-1 + arm-1-body +
//!     merge). Entry ends in `BrTrilean` from the literal pattern's
//!     `Eq` test; last arm (wildcard) is unconditional `Br`.
//!   - Merge block starts with a `Phi` carrying exactly 2 incoming
//!     edges (one per arm).
//!   - 3-arm outcome match `match res { ~+ v => v, ~- _ => -1, ~0
//!     => 0 }` over `Integer?~String` lowers to 7 blocks. Final
//!     merge has 3 Phi incoming edges.
//!
//! Per-mutated-var phi is OMITTED at v0.7.11.1f2 because the Triết-
//! side `Expr` enum has no `Block` expression yet — match arm
//! bodies can only be single expressions (no `AssignStmt`). When a
//! future sub-task adds `BlockExpr`, the lowerer's mutation-phi
//! pass needs porting; tracked as
//! `v0.7.x.runtime-fix.match-arm-mutation-phi-port`.
//!
//! See [ADR-0020 §5] (outcome arm patterns) +
//! v0.7.x.runtime-fix.lowerer-incomplete-expr deferred bug entry.
//!
//! [ADR-0020 §5]: ../../../../docs/decisions/0020-outcome-error-handling.md

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
fn ir_lowerer_match_expr_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("match_expr_smoke_main"))
        .expect("missing match_expr_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri match_expr_smoke_main() must complete without VM error");
}

//! v0.7.11.1e2 — smoke test for the Triết-in-Triết IR lowerer's
//! `IfExpr` arm at `compiler/ir_lowerer.tri::if_expr_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `if_expr_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke covers:
//!
//!   - Plain `if cond { 1 } else { 0 }` with `Trilean!` condition
//!     lowers to 5 blocks (entry + then + else + merge +
//!     `if_unknown_panic` per ADR-0010 / SPEC §7.1.1: plain `if`
//!     panics on Unknown).
//!   - Entry ends in `BrTrilean` with 3 distinct target blocks.
//!   - Merge block starts with a `Phi` carrying exactly 2 incoming
//!     edges (then-end + else-end).
//!   - `if?` form on a `Trilean` (Unknown-possible) condition
//!     produces a 4-block shape (no panic block; Unknown→else per
//!     `treat_unknown_as_false`).
//!   - `if?`'s `BrTrilean` has `unknown_block ≡ false_block` (both
//!     route to else arm).
//!
//! Phi-stitch for mutated vars is exercised by lower-level call
//! sites once .1f2 (`MatchExpr`) lands and Stage 2 compiles main.tri.
//!
//! See [ADR-0010 §C] (Trilean refinement) + ADR-0019 §A7.11.
//!
//! [ADR-0010 §C]: ../../../../docs/decisions/0010-ternary-native-ir.md

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
fn ir_lowerer_if_expr_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("if_expr_smoke_main"))
        .expect("missing if_expr_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri if_expr_smoke_main() must complete without VM error");
}

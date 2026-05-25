//! v0.7.12.4 — smoke test for the Triết-in-Triết `BlockExpr` arm
//! at `compiler/ir_lowerer.tri::block_expr_smoke_main`.
//!
//! Verifies:
//!
//!   - Match arm with block-body (`pattern => { stmt; expr }`)
//!     parses + lowers + activates the v0.7.12.3 mutation-phi pass.
//!     Mutation inside the block reaches the enclosing match arm
//!     because `BlockExpr` lowers via `lower_block_inline` (no
//!     scope push/pop). The merge block carries 2 Phi instructions:
//!     result value + mutated var.
//!   - `BlockExpr` as a let RHS (`let y = { let inner = 5; inner + 1 }`)
//!     lowers without VM error in a single block. Inner `let`
//!     binding leaks to the outer scope at the IR level per the
//!     v0.7.12.4 design note (harmless because typecheck enforces
//!     source-level scoping).
//!
//! See [ADR-0019 §6] + the v0.7.12 sub-task plan in TODO.md.
//!
//! [ADR-0019 §6]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

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
fn ir_lowerer_block_expr_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("block_expr_smoke_main"))
        .expect("missing block_expr_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri block_expr_smoke_main() must complete without VM error");
}

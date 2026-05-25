//! v0.7.12.3 — smoke test for the Triết-in-Triết match-arm
//! mutation-phi infrastructure at
//! `compiler/ir_lowerer.tri::match_mutation_phi_infra_smoke_main`.
//!
//! Verifies that:
//!
//!   - `walk_expr_for_assigns` + `walk_block_for_assigns` +
//!     `vec_extend_unique` compile, typecheck, and run inside the
//!     VM without panic when invoked on a typical match expression.
//!   - The 2-arm match-expression baseline shape from v0.7.11.1f2
//!     (5 blocks, Phi-with-2-incoming at merge) is preserved when
//!     no arm mutates — the mutation-phi pass is dead code today
//!     because Triết-side `Expr` has no `BlockExpr` and match-arm
//!     bodies are single expressions (no `AssignStmt`).
//!
//! When v0.7.12.4 lands `BlockExpr` in the parser + lowerer, match
//! arm bodies with block forms `=> { stmt; stmt; expr }` will
//! become eligible. At that point this test will gain a
//! `with-mutation` companion exercising the actual phi merge.
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
fn ir_lowerer_match_mutation_phi_infra_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("match_mutation_phi_infra_smoke_main"))
        .expect("missing match_mutation_phi_infra_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/ir_lowerer.tri match_mutation_phi_infra_smoke_main() must complete without VM error",
    );
}

//! v0.7.8.3 — smoke test for the Triết-in-Triết IR lowerer's
//! function-call + simple control-flow surface at
//! `compiler/ir_lowerer.tri::call_loop_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `call_loop_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke covers:
//!
//!   - `CallBuiltin`: `println("hi")` resolves to the `Println`
//!     builtin variant via the name lookup table.
//!   - `CallLocal`: forward / backward function calls resolve
//!     through the per-module `func_table` (`FuncId` raw =
//!     source-order index).
//!   - `AssignStmt`: rebinds the target name in the innermost
//!     scope (no `Phi` insertion needed at v0.7.8.3 because
//!     `While`/`For` — the contexts that require SSA stitching
//!     across iterations — defer to v0.7.8.3b).
//!   - `Loop` / `Break` / `Continue`: produces the canonical
//!     3-block shape (`entry` → `loop_body` → `loop_exit`) and the
//!     loop stack correctly resolves break/continue targets.
//!
//! While / For lowering needs phi-node insertion at the loop
//! header — that lands at v0.7.8.3b once the functional-style
//! patch-up primitive is in place.
//!
//! See [ADR-0019 §A7.8](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

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
fn ir_lowerer_call_loop_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("call_loop_smoke_main"))
        .expect("missing call_loop_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/ir_lowerer.tri call_loop_smoke_main() must complete without VM error",
    );
}

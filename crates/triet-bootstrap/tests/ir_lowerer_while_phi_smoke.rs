//! v0.7.8.3b — smoke test for the Triết-in-Triết IR lowerer's
//! While-loop + phi-node insertion at
//! `compiler/ir_lowerer.tri::while_phi_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `while_phi_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke covers:
//!
//!   - `while?` form (`treat_unknown_as_false`) over an empty body →
//!     canonical 4-block shape (entry / `while_header` /
//!     `while_body` / `while_exit`).
//!   - Plain `while` with a mutating body — `let mutable i = 0
//!     while i < 10 { i = i + 1 }` — spawns the `while_unknown_panic`
//!     block too, totaling 5 blocks.
//!   - The loop header carries a `Phi` instruction for the
//!     assigned variable `i`. The patch-up helper
//!     `patch_phi_incoming` rebuilds the `BasicBlock`'s
//!     instructions vector to replace the placeholder incoming
//!     edge with the two-edge `[pre_loop, body_end]` form.
//!   - The header's terminator is `BrTrilean` (ADR-0010
//!     ternary-native branch).
//!
//! `For` loop lowering needs the same phi-insertion primitive plus
//! `Range` counter synthesis — defers to v0.7.8.3c. Break-with-value
//! also needs a tail-phi at the exit block and defers alongside.
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
fn ir_lowerer_while_phi_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("while_phi_smoke_main"))
        .expect("missing while_phi_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri while_phi_smoke_main() must complete without VM error");
}

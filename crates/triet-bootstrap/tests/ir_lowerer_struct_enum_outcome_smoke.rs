//! v0.7.8.4 — smoke test for the Triết-in-Triết IR lowerer's
//! struct + enum + outcome + For-loop surface at
//! `compiler/ir_lowerer.tri::struct_enum_outcome_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `struct_enum_outcome_smoke_main()` end-to-end
//! on the VM. Covers:
//!
//!   - `StructItem` / `EnumItem` declaration tracking (pass 1a
//!     builds `struct_fields` / `variant_index` / `enum_variants`).
//!   - Bare-identifier promotion to `EnumNew` for unit variants
//!     (e.g. `None` → `EnumNew { variant_idx: 0, payload: ~0 }`).
//!   - Tuple-variant call promotion (`Some(42)` → `EnumNew { …,
//!     payload: ValueOp }`).
//!   - `OutcomeConstructorExpr` opcodes: `~+ x` → `OutcomeNewPositive`,
//!     `~- e` → `OutcomeNewNegative`, `~0` → `OutcomeNewNull` (no
//!     payload).
//!   - `For` loop over `Range` with counter `Phi` node at the
//!     header — 4 blocks (entry / `for_header` / `for_body` /
//!     `for_exit`).
//!
//! `FieldGet` `field_idx` resolution awaits the ADR-0023
//! `ValueKind` tracking port (defers to v0.7.8.4+). `MatchExpr`-driven
//! pattern binding inside lowering defers alongside the parser
//! gap.
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
fn ir_lowerer_struct_enum_outcome_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("struct_enum_outcome_smoke_main"))
        .expect("missing struct_enum_outcome_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/ir_lowerer.tri struct_enum_outcome_smoke_main() must complete without VM error",
    );
}

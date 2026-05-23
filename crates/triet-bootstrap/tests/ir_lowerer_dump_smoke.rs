//! v0.7.8.5 — smoke test for the Triết-in-Triết IR lowerer's
//! NDJSON dump driver at
//! `compiler/ir_lowerer.tri::ir_dump_smoke_main`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `ir_dump_smoke_main()` end-to-end on the VM.
//! Covers the `dump_ir_program_ndjson(source) -> String` driver
//! that produces a stable structural representation of an
//! `IrProgram`:
//!
//!   - Empty program → `{"k":"Program","modules":1}\n{…}` (always
//!     wraps in a module per the `lower_source` shape).
//!   - `function f() -> Integer = 42` → expected line sequence
//!     `Program → Module → Function → Block → Const + Ret`.
//!   - Determinism: two calls on the same source produce the same
//!     bytes.
//!   - 2-param arithmetic: `add(x, y) = x + y` → `Add %2 = %0, %1`
//!     + `Ret %2`.
//!   - Outcome constructor — verifies the dump produces non-empty
//!     output without crashing.
//!
//! Per ADR-0019 §A2 the NDJSON dump is a transitional bridge
//! through v0.7.8; v0.7.9 drops it when Triết-side data flows
//! in-memory. The byte-diff gate against the Rust mirror lands at
//! v0.7.8.6.
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
fn ir_lowerer_dump_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("ir_dump_smoke_main"))
        .expect("missing ir_dump_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![]).expect(
        "compiler/ir_lowerer.tri ir_dump_smoke_main() must complete without VM error",
    );
}

//! v0.7.8.2 — smoke test for the Triết-in-Triết IR lowerer's
//! `lower_source` driver at `compiler/ir_lowerer.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `lower_source_smoke_main()` end-to-end on the
//! VM. The Triết-side smoke covers the v0.7.8.2 lowering path:
//!
//!   - empty program → 0-function `IrProgram`
//!   - `function f() -> Integer = 42` → 1 function, 1 entry block,
//!     `Const + Ret` (exactly 2 instructions)
//!   - `function g() -> Integer { let x: Integer = 5 x + 3 }` →
//!     `Const + Const + Add + Ret`
//!   - `function add(x: Integer, y: Integer) -> Integer = x + y` →
//!     two parameter slots, parameter types resolve to `Integer`
//!
//! Function calls + control flow (While / For / Loop / Break /
//! Continue / Assign) defer to v0.7.8.3; structs/enums/pattern/
//! Outcome opcodes defer to v0.7.8.4; the full
//! `lowerer_differential` byte-diff gate lands v0.7.8.6.
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
fn ir_lowerer_lower_source_smoke_main_passes_all_asserts() {
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
        .find(|f| f.name.as_deref() == Some("lower_source_smoke_main"))
        .expect("missing lower_source_smoke_main() in compiler/ir_lowerer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(smoke_id, vec![])
        .expect("compiler/ir_lowerer.tri lower_source_smoke_main() must complete without VM error");
}

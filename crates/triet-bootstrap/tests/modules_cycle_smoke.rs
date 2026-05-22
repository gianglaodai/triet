//! v0.7.6.3 — smoke test for the Triết-in-Triết cycle detector at
//! `compiler/modules.tri::detect_cycles`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `cycle_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke exercises 8 corpus cases mirroring the
//! Rust impl's `crates/triet-modules/src/cycle.rs` tests
//! (adapted for inline-module-only since the in-memory loader
//! can't read external files):
//!
//!   1. 2-cycle a↔b (must flag, trace mentions a + b)
//!   2. 3-cycle a → b → c → a (must flag, trace mentions a, b, c)
//!   3. Diamond a→b, a→c, b→d, c→d (no false positive)
//!   4. Stdlib `from std.io import println` (no false positive)
//!   5. No imports at all (no false positive)
//!   6. Two independent 2-cycles (must flag both)
//!   7. Self-import (no false positive — parent-drop guard)
//!   8. Acyclic chain a → b → c (no false positive)
//!
//! Filesystem cycle detection lands at v0.7.6.5
//! (`modules_differential` byte-diff gate). Name resolution
//! (v0.7.6.4) is out-of-scope for this smoke.
//!
//! See [ADR-0019 §A7.6](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

use std::path::PathBuf;

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

fn compiler_modules_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("modules.tri")
}

#[test]
fn modules_cycle_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_modules_path();
    assert!(
        path.is_file(),
        "missing compiler/modules.tri at {}",
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
        "type errors in compiler/modules.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let cycle_smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("cycle_smoke_main"))
        .expect("missing cycle_smoke_main() in compiler/modules.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(cycle_smoke_id, vec![])
        .expect("compiler/modules.tri cycle_smoke_main() must complete without VM error");
}

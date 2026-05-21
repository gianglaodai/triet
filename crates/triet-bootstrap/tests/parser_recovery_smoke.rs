//! v0.7.5.6a — smoke test for the Triết-in-Triết parser error
//! recovery layer at `compiler/parser.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. Beyond the
//! v0.7.5.{1..5b} coverage, the Triết-side `main()` now exercises
//! the new recovery path: `parse_program` returns
//! `ParseProgramStep { program, errors: Vector<ParseError> }`
//! instead of the v0.7.5.4 `~ParseError` outcome, and the driver
//! synchronizes to the next item keyword (or after `;`) on
//! failure, with a defensive force-advance guard against
//! sync-stop-without-progress. Smoke asserts cover broken
//! middle/leading/trailing items + multi-error accumulation.
//!
//! The closing `parser_differential` NDJSON gate (per ADR-0019
//! §A2 transient bridge) lands at v0.7.5.6b.
//!
//! See [ADR-0019 §A7.5](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

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
fn parser_recovery_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("parser");
    assert!(
        path.is_file(),
        "missing compiler/parser.tri at {}",
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
        "type errors in compiler/parser.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/parser.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/parser.tri smoke main() must complete without VM error");
}

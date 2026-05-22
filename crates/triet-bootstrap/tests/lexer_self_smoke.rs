//! v0.7.4.3 — smoke test for the hand-rolled Triết-in-Triết lexer at
//! `compiler/parser/lexer.tri`. Builds the source to `.triv` and runs it
//! through the VM end-to-end. The Triết-side `main()` exercises 8
//! token-shape scenarios and asserts each via `assert(...)`, so any
//! mismatch panics with E2205 — surfacing failure as a test error
//! without the harness needing to capture stdout.
//!
//! The full byte-diff differential test against `triet-lexer/`
//! lands at v0.7.4.4 (`lexer_differential.rs`). This smoke gate
//! provides earlier regression protection — every lowerer change
//! that breaks the port surfaces here before the differential pass.
//!
//! See [ADR-0019 §A7.4](../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md).

use std::path::PathBuf;

use triet_ir::{Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

/// Walk up from the bootstrap manifest dir to the workspace root and
/// join `compiler/<name>.tri`. Mirrors `bootstrap_determinism.rs`'s
/// `example_path` convention.
fn compiler_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join(format!("{name}.tri"))
}

/// Build `compiler/parser/lexer.tri` to bytes and round-trip through the VM
/// reader to confirm `.triv` survives the wire format. Then run
/// `main()` — the Triết-side asserts gate correctness.
#[test]
fn lexer_self_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("parser/lexer");
    assert!(
        path.is_file(),
        "missing compiler/parser/lexer.tri at {}",
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
        "type errors in compiler/parser/lexer.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/parser/lexer.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/parser/lexer.tri smoke main() must complete without VM error");
}

//! v0.7.5.5a — smoke test for the Triết-in-Triết full type grammar
//! at `compiler/parser/parser.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. The Triết-side
//! `main()` exercises the type-level surface that v0.7.5.5a brings
//! online: `Trilean!` refinement, generic instantiations (single
//! arg + multi-arg + nested), tuple types (incl. singleton tuple
//! `(T,)` vs grouping `(T)`), function types `(T1, T2) -> R`,
//! chained nullable `T?…?`, and outcome types (`T~E` binary +
//! `T?~E` ternary).
//!
//! The full `parser_differential` gate (NDJSON AST diff against
//! the Rust impl) lands at v0.7.5.6 once `.5b` (full pattern
//! grammar) ships.
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
fn parser_type_smoke_main_passes_all_asserts() {
    use miette::Diagnostic;

    let path = compiler_path("parser/parser");
    assert!(
        path.is_file(),
        "missing compiler/parser/parser.tri at {}",
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
        "type errors in compiler/parser/parser.tri: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);
    let restored = read_program(&bytes).expect("read .triv round-trip");

    // parser.tri imports `compiler/parser/lexer.tri` via `module lexer;`,
    // so both files contribute a `main()`. The entry module's
    // items come first; picking the first matching function selects
    // parser.tri's smoke main, whose v0.7.5.5a surface now runs all
    // type-layer asserts in addition to the v0.7.5.{1,2,3,4} ones.
    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in compiler/parser/parser.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(main_id, vec![])
        .expect("compiler/parser/parser.tri smoke main() must complete without VM error");
}

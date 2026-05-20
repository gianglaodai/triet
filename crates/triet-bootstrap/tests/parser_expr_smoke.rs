//! v0.7.5.2 — smoke test for the Triết-in-Triết Pratt expression
//! parser at `compiler/parser.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. The Triết-side
//! `main()` lexes a corpus of expression snippets (arithmetic +
//! precedence, comparison, logic, postfix, outcome / elvis / range)
//! and asserts each parses into the expected s-expression shape.
//!
//! The full `parser_differential` gate (NDJSON AST diff against the
//! Rust impl) lands at v0.7.5.6 after the remaining parser layers
//! ship in v0.7.5.{3, 4, 5}.
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
fn parser_expr_smoke_main_passes_all_asserts() {
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

    // parser.tri imports `compiler/lexer.tri` via `module lexer;`, so
    // both files contribute a `main()` to the lowered program. The
    // loader places the entry module's items first, so picking the
    // first matching function selects parser.tri's smoke main. If
    // future loader changes break this ordering, the test will
    // surface (lexer's smoke prints different output and the asserts
    // would still gate correctness).
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

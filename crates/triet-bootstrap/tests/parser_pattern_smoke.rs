//! v0.7.5.5b — smoke test for the Triết-in-Triết full pattern
//! grammar at `compiler/parser/parser.tri`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `main()` end-to-end on the VM. Beyond the
//! v0.7.5.3 minimal pattern grammar (Wildcard / Identifier), the
//! Triết-side `main()` now exercises: literal patterns (Integer /
//! Ternary / String / Trilean), negative integer literals, range
//! patterns (`..` exclusive / `..=` inclusive, incl. negative
//! bounds `-5..=5`), tuple patterns (incl. singleton `(x,)` vs
//! grouping `(x)`), top-level or-patterns (`1 | 2 | 3`), the
//! bare `null` keyword pattern, enum variant patterns (`Some(x)`
//! / `None` / `Some(_)` / `Cell((c, d))` with explicit tuple
//! payload), and outcome arm patterns (`~+ value` / `~- err` /
//! `~0` / `~+ _`) per ADR-0020 §5.
//!
//! The full `parser_differential` gate (NDJSON AST diff against
//! the Rust impl) lands at v0.7.5.6 — the next and final sub-task
//! of the v0.7.5 umbrella.
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
fn parser_pattern_smoke_main_passes_all_asserts() {
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
    // parser.tri's smoke main, which now runs the v0.7.5.{1..5b}
    // assertion stack in a single end-to-end VM execution.
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

//! v0.7.4.2 — VM-side end-to-end test for stdlib `.tri` stubs.
//!
//! Exercises `from std.collections.vector import …` / `from std.path
//! import …` / `from std.text import parse_integer` etc. and verifies
//! that builtin dispatch via `path_to_builtin` (`triet-ir/src/vm.rs`)
//! produces correct runtime output.
//!
//! **Interpreter is NOT exercised** for these stubs at v0.7.4.2.
//! `triet-interpreter::builtins::install` only registers the v0.2
//! prelude (print/println/length/assert/...); the 19 v0.7.3 builtins
//! are VM-only. Interpreter parity is tracked as a deferred item in
//! [ADR-0019 Addendum §A7] — defers cleanly to v0.7.4.3+ when the
//! interpreter is dropped as a development tier per VISION §4.3.
//!
//! [ADR-0019 Addendum §A7]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use triet_ir::{lower_program, write_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

/// Compile a Triết program to `.triv` bytes and run it via the VM,
/// returning captured stdout as a String. Panics on any pipeline
/// error — these tests pin successful builtin dispatch, not error
/// paths.
fn run_program_via_vm(source: &str) -> String {
    use miette::Diagnostic;
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    // v0.7.4.3-error.2: filter W2001 NullDeprecated warnings — stdlib
    // stubs still use `null` until v0.7.4.3-error.4 migration tool.
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(blocking.is_empty(), "type errors: {blocking:?}");
    let ir = lower_program(&resolved);
    let bytes = write_program(&ir);

    // Round-trip through .triv → VM (matches `dao build` + `dao run`
    // path exactly). Reading back proves the wire format survives
    // the new builtin IDs end-to-end.
    let restored = triet_ir::read_program(&bytes).expect("read .triv");
    let main_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main()")
        .id;
    let mut vm = triet_ir::Vm::new(restored);

    // VM prints directly via `println!` from `BuiltinName::Println`.
    // Output capture not available without refactoring VM IO; these
    // tests pin successful execution (no panic) — assert(...) inside
    // the source surfaces incorrect dispatch as a test failure.
    vm.execute(main_id, vec![]).expect("vm execute");
    String::new()
}

#[test]
fn vector_stdlib_path_dispatches_correctly() {
    // Build a vector with 3 elements, verify length + get via builtins
    // resolved through `from std.collections.vector import …`.
    let source = r"
        from std.collections.vector import new, push, length, get
        from std.assert import assert

        function main() {
            let v0: Vector<Integer> = new()
            let v1: Vector<Integer> = push(v0, 100)
            let v2: Vector<Integer> = push(v1, 200)
            let v3: Vector<Integer> = push(v2, 300)
            let n: Integer = length(v3)
            assert(n == 3)
            let mid: Integer = get(v3, 1)!!
            assert(mid == 200)
        }
    ";
    let _ = run_program_via_vm(source);
}

#[test]
fn hashmap_stdlib_path_dispatches_correctly() {
    let source = r#"
        from std.collections.hashmap import new, insert, get, contains
        from std.assert import assert

        function main() {
            let m0: HashMap<String, Integer> = new()
            let m1: HashMap<String, Integer> = insert(m0, "alpha", 1)
            let m2: HashMap<String, Integer> = insert(m1, "beta", 2)
            assert(contains(m2, "alpha"))
            let v: Integer = get(m2, "beta")!!
            assert(v == 2)
        }
    "#;
    let _ = run_program_via_vm(source);
}

#[test]
fn path_stdlib_dispatches_correctly() {
    let source = r#"
        from std.path import join, basename, parent
        from std.assert import assert

        function main() {
            let p: String = join("/etc", "config.toml")
            assert(p == "/etc/config.toml")
            assert(basename(p) == "config.toml")
            assert(parent(p)!! == "/etc")
        }
    "#;
    let _ = run_program_via_vm(source);
}

#[test]
fn string_and_parse_integer_dispatch_correctly() {
    let source = r#"
        from std.string import substring, index_of
        from std.text import parse_integer
        from std.assert import assert, assert_eq

        function main() {
            assert(substring("hello", 1, 4) == "ell")
            assert_eq(parse_integer("42")!!, 42)
            let absent = parse_integer("not_a_number")
            assert(absent == null)
            let found: Integer = index_of("hello", "ll")!!
            assert(found == 2)
        }
    "#;
    let _ = run_program_via_vm(source);
}

#[test]
fn io_fs_dispatch_with_tempfile_round_trip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("v074_2.txt");
    let path_str = path.to_string_lossy().to_string();
    // Embed the temp path into source as a literal — keeps the test
    // self-contained without exposing a `parameterized run` shim.
    let source = format!(
        r#"
        from std.io.fs import write, read, exists
        from std.assert import assert

        function main() {{
            let p: String = "{path_str}"
            let ok = write(p, "v0.7.4.2 stdlib stub round trip")
            assert(ok)
            assert(exists(p))
            let contents: String = read(p)!!
            assert(contents == "v0.7.4.2 stdlib stub round trip")
        }}
    "#
    );
    let _ = run_program_via_vm(&source);
}

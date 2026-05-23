//! v0.7.6.5 — `modules_differential` test (closes the v0.7.6
//! modules umbrella per [ADR-0019 §A7.6]).
//!
//! For each corpus source, runs the Rust impl
//! [`triet_modules::load_program_from_source`] and the
//! Triết-in-Triết port at
//! `compiler/modules.tri::dump_resolved_program_ndjson` over the
//! same input. Both sides emit the same line-delimited JSON shape
//! (one Program header + one line per `Module` + one line per
//! `Error`) and the test asserts byte-equality.
//!
//! ## Format
//!
//! ```text
//! {"k":"Program","modules_root":N,"errors":E}
//! {"k":"Module","idx":I,"path":"<dot.path>","arena":A,"parent":<I|null>,"children":[…],"items":N,"bindings":[["name","abs"],…]}
//! …
//! {"k":"Error","code":"<E21xx>","span":[<start>,<end>]}
//! ```
//!
//! ## Stdlib filter
//!
//! Triết's in-memory loader doesn't pre-load stdlib (env-var-read
//! builtin missing — see TODO.md `v0.7.x.runtime-fix.const-items`
//! and the v0.7.6.2 deferral note for context). Both sides filter
//! modules whose root segment is `std` / `sys` / `dev` / `usr` /
//! `core` so the diff is on user-authored modules only. Stdlib
//! pre-load gate lands v0.7.10 alongside CLI wiring per
//! ADR-0019 §A7.10.
//!
//! ## Transient bridge
//!
//! NDJSON is a transient bridge format per ADR-0019 §A2 — dropped
//! at v0.7.9 when Triết-side data flows in-memory.
//!
//! [ADR-0019 §A7.6]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md
//! [ADR-0019 §A2]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use miette::Diagnostic;
use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_modules::{
    AbsolutePath, LoaderError, Module, ModuleId, ModulePath, ResolvedProgram, load_program,
    load_program_from_source,
};
use triet_typecheck::check_resolved;

// ─────────────────────────────────────────────────────────────────
// Triết-side: compile `compiler/modules.tri` once + run
// `dump_resolved_program_ndjson(source)`. Mirrors
// `parser_differential::parser_ir`.
// ─────────────────────────────────────────────────────────────────

fn compiler_modules_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("compiler")
        .join("modules_root.tri")
}

fn modules_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
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
        read_program(&bytes).expect("read .triv round-trip")
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}` in compiler/modules.tri"))
        .id
}

fn triet_dump(source: &str) -> String {
    let ir = modules_ir().clone();
    let func_id = lookup_func(&ir, "dump_resolved_program_ndjson");
    let mut vm = Vm::new(ir);
    let result = vm
        .execute(func_id, vec![RuntimeValue::String(source.to_owned())])
        .expect("compiler/modules.tri::dump_resolved_program_ndjson must execute without VM error");
    match result {
        RuntimeValue::String(s) => s,
        other => panic!("expected String from dump_resolved_program_ndjson, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Rust-side mirror — walks triet_modules::ResolvedProgram, emitting
// byte-identical NDJSON.
// ─────────────────────────────────────────────────────────────────

fn is_stdlib_module(module: &Module) -> bool {
    matches!(
        module.path.root(),
        Some("std" | "sys" | "dev" | "usr" | "core")
    )
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn quote_string(s: &str) -> String {
    format!("\"{}\"", json_escape(s))
}

fn absolute_path_display(p: &AbsolutePath) -> String {
    format!("{p}")
}

fn module_path_display(p: &ModulePath) -> String {
    format!("{p}")
}

const fn error_code(err: &LoaderError) -> &'static str {
    err.code()
}

fn error_span(err: &LoaderError) -> (usize, usize) {
    let s = err.span();
    (s.start, s.end)
}

fn parent_path_text(program: &ResolvedProgram, parent: Option<ModuleId>) -> String {
    parent.map_or_else(
        || "null".to_owned(),
        |id| {
            let parent_mod = &program.modules[id.raw()];
            quote_string(&module_path_display(&parent_mod.path))
        },
    )
}

fn dump_module_line(program: &ResolvedProgram, module: &Module, out: &mut String) {
    out.push_str("{\"k\":\"Module\",\"path\":");
    out.push_str(&quote_string(&module_path_display(&module.path)));
    out.push_str(",\"parent\":");
    out.push_str(&parent_path_text(program, module.parent));

    // Children — collect non-stdlib child paths and sort
    // lexicographically. Mirrors Triết-side `sort_string_vector`.
    out.push_str(",\"children\":[");
    let mut child_paths: Vec<String> = module
        .children
        .iter()
        .filter_map(|&id| {
            let child = &program.modules[id.raw()];
            if is_stdlib_module(child) {
                None
            } else {
                Some(module_path_display(&child.path))
            }
        })
        .collect();
    child_paths.sort();
    let mut first = true;
    for p in &child_paths {
        if first {
            first = false;
        } else {
            out.push(',');
        }
        out.push_str(&quote_string(p));
    }
    out.push(']');

    out.push_str(",\"items\":");
    write!(out, "{}", module.items.len()).unwrap();

    // Bindings — Triết-side keys come from HashMap.keys() which is
    // sorted (BTreeMap-backed per v0.7.3.3 §A2). Rust-side uses
    // HashMap, so we sort here to match.
    out.push_str(",\"bindings\":[");
    let sorted: BTreeMap<&String, &AbsolutePath> = module.bindings.iter().collect();
    let mut first = true;
    for (name, abs) in sorted {
        if first {
            first = false;
        } else {
            out.push(',');
        }
        out.push('[');
        out.push_str(&quote_string(name));
        out.push(',');
        out.push_str(&quote_string(&absolute_path_display(abs)));
        out.push(']');
    }
    out.push_str("]}");
}

fn dump_error_line(err: &LoaderError, out: &mut String) {
    let (s, e) = error_span(err);
    out.push_str("{\"k\":\"Error\",\"code\":");
    out.push_str(&quote_string(error_code(err)));
    out.push_str(",\"span\":[");
    write!(out, "{s}").unwrap();
    out.push(',');
    write!(out, "{e}").unwrap();
    out.push_str("]}");
}

fn rust_dump(source: &str) -> String {
    // Mirror the Triết-side load_program_from_source: a successful
    // load returns ResolvedProgram with empty errors; a failure
    // returns Vec<LoaderError>. We need the partial program too
    // to walk modules, but the public Rust API only returns
    // Result. For diagnostic-mode (errors emitted), the Rust impl
    // returns Err with no partial program — so we emit a header
    // with zero modules and the error lines.
    let (program, errors): (Option<ResolvedProgram>, Vec<LoaderError>) =
        match load_program_from_source(source) {
            Ok(p) => (Some(p), Vec::new()),
            Err(errs) => (None, errs),
        };

    let mut out = String::new();
    let module_count = program.as_ref().map_or(0, |p| {
        p.modules.iter().filter(|m| !is_stdlib_module(m)).count()
    });
    out.push_str("{\"k\":\"Program\",\"modules\":");
    write!(out, "{module_count}").unwrap();
    out.push_str(",\"errors\":");
    write!(out, "{}", errors.len()).unwrap();
    out.push_str("}\n");

    if let Some(p) = &program {
        // Sort modules by path so the diff is stable across
        // loader insertion-order differences (Rust pre-loads
        // stdlib; Triết doesn't).
        let mut user_modules: Vec<&Module> =
            p.modules.iter().filter(|m| !is_stdlib_module(m)).collect();
        user_modules.sort_by_key(|a| module_path_display(&a.path));
        for module in user_modules {
            dump_module_line(p, module, &mut out);
            out.push('\n');
        }
    }

    for err in &errors {
        dump_error_line(err, &mut out);
        out.push('\n');
    }

    out
}

// ─────────────────────────────────────────────────────────────────
// Corpus + diff harness
// ─────────────────────────────────────────────────────────────────

fn assert_diff(label: &str, source: &str) {
    let rust = rust_dump(source);
    let triet = triet_dump(source);
    assert_eq!(
        triet, rust,
        "modules_differential diff in `{label}`\n--- Triết ---\n{triet}\n--- Rust ---\n{rust}",
    );
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn empty_program() {
    assert_diff("empty_program", "");
}

#[test]
fn single_function() {
    assert_diff("single_function", "function f() -> Integer = 0");
}

#[test]
fn child_module_bound_in_parent() {
    assert_diff(
        "child_module_bound_in_parent",
        "module helper { function aid() -> Integer = 1 }",
    );
}

#[test]
fn nested_inline_modules() {
    assert_diff(
        "nested_inline_modules",
        "module outer { module inner { function deep() -> Integer = 0 } }",
    );
}

#[test]
fn sibling_inline_modules() {
    assert_diff("sibling_inline_modules", "module a { } module b { }");
}

#[test]
fn from_import_absolute() {
    assert_diff(
        "from_import_absolute",
        "module helper { public function aid() -> Integer = 1 }\nfrom crate.helper import aid",
    );
}

#[test]
fn from_import_with_alias() {
    assert_diff(
        "from_import_with_alias",
        "module helper { public function aid() -> Integer = 1 }\nfrom crate.helper import aid as helper_aid",
    );
}

#[test]
fn visibility_violation_private_import() {
    assert_diff(
        "visibility_violation_private_import",
        "module helper { function secret() -> Integer = 1 }\nfrom crate.helper import secret",
    );
}

#[test]
fn unresolved_import_nonexistent_module() {
    assert_diff(
        "unresolved_import_nonexistent_module",
        "from crate.nonexistent import foo",
    );
}

#[test]
fn reserved_namespace_sys_import() {
    assert_diff(
        "reserved_namespace_sys_import",
        "from sys.io import openf",
    );
}

#[test]
fn self_keyword_inside_submodule() {
    assert_diff(
        "self_keyword_inside_submodule",
        "module outer { module inner { public function b() -> Integer = 1 }\nfrom self.inner import b }",
    );
}

#[test]
fn two_cycle() {
    assert_diff(
        "two_cycle",
        "module a { from crate.b import x }\nmodule b { from crate.a import y }",
    );
}

#[test]
fn mixed_items_and_modules() {
    assert_diff(
        "mixed_items_and_modules",
        "function f() -> Integer = 0\nmodule helper { }\nfunction g() -> Integer = 1",
    );
}

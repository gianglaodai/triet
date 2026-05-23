//! v0.7.6.4 — smoke test for the Triết-in-Triết name resolver at
//! `compiler/modules.tri::resolve_names`.
//!
//! Builds the source to `.triv`, round-trips through the wire
//! reader, then runs `resolver_smoke_main()` end-to-end on the VM.
//! The Triết-side smoke exercises 8 corpus cases mirroring the
//! Rust impl's `crates/triet-modules/src/resolver.rs` tests
//! (adapted for inline-module-only since the in-memory loader
//! can't read external files):
//!
//!   1. Own function bound in crate root (`function f()` → bindings[f] = crate.f)
//!   2. Child module bound in parent (`module helper { … }` → bindings[helper] = crate.helper)
//!   3. From-import binds with absolute path (`from crate.helper import aid`)
//!   4. From-import with alias (`from crate.helper import aid as helper_aid`)
//!   5. Visibility violation: importing a private item raises E2103
//!   6. Unresolved import: module doesn't exist raises E2104
//!   7. Reserved namespace `sys.*` raises E2102
//!   8. `self.X` path keyword expansion (inside submodule)
//!
//! Stdlib pre-load + filesystem `super.X` lands at v0.7.6.5
//! (`modules_differential` byte-diff gate). The `super.X` resolver
//! code is in place but the in-memory smoke doesn't exercise it
//! (would need a 3-level inline nesting).
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
        .join("modules_root.tri")
}

#[test]
fn modules_resolver_smoke_main_passes_all_asserts() {
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

    let resolver_smoke_id = restored
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("resolver_smoke_main"))
        .expect("missing resolver_smoke_main() in compiler/modules.tri")
        .id;

    let mut vm = Vm::new(restored);
    vm.execute(resolver_smoke_id, vec![])
        .expect("compiler/modules.tri resolver_smoke_main() must complete without VM error");
}

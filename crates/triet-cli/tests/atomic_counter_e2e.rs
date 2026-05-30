//! v0.9.x.atomic.7e — end-to-end test for `examples/atomic_counter/`
//! demo per ADR-0031 §9 Phương án A.
//!
//! Loads the actual demo source from disk, type-checks it, lowers
//! to IR, then runs `main()` on the single-thread VM (ADR-0028 §9
//! dev tier). Asserts the returned previous-value matches the
//! atomic's initial state (0) — proving the full pipeline works:
//!
//! - Borrow expression syntax `&+ counter` (ADR-0031 §1).
//! - `sys.atomic.new` builtin dispatch via path interception
//!   (ADR-0028 §1 + ADR-0019 Addendum §A7).
//! - `sys.atomic.fetch_add` builtin dispatch.
//! - E2420 `UseAfterMove` consume-once tracking (ADR-0025 §5.1).
//! - Capability gate `sys.atomic grant` (ADR-0028 §8) via dao.package.
//!
//! Run via in-process VM rather than `dao run` CLI because the
//! tree-walking interpreter doesn't intercept `sys.atomic.*` builtin
//! paths — mirrors the `outcome_propagate.tri` VM-only precedent.
//! Interpreter parity tracked in ADR-0031 §10 backlog.

use std::path::PathBuf;

use miette::Diagnostic;
use triet_ir::{RuntimeValue, Vm, lower_program};
use triet_modules::load_program;
use triet_typecheck::check_resolved;

/// Walk up from the CLI manifest dir to the workspace root, then to
/// `examples/atomic_counter/atomic_counter.tri`. Mirrors the
/// `compiler_path` convention from bootstrap tests.
fn demo_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("examples")
        .join("atomic_counter")
        .join("atomic_counter.tri")
}

#[test]
fn atomic_counter_demo_runs_end_to_end_on_vm() {
    let path = demo_path();
    assert!(path.is_file(), "missing demo source at {}", path.display());

    let resolved = load_program(&path).expect("load_program (resolver + stdlib)");

    let diagnostics = check_resolved(&resolved);
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(
        blocking.is_empty(),
        "demo source has type errors: {blocking:#?}",
    );

    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main() in demo")
        .id;

    let mut vm = Vm::new(ir);
    let result = vm
        .execute(main_id, Vec::new())
        .expect("VM should execute demo without error");

    // Demo's main() returns `prev` from `fetch_add(&+ counter, 1, Synchronized)`.
    // Counter was initialized to 0, so the pre-increment value is 0.
    match result {
        RuntimeValue::Integer(i) => assert_eq!(
            i.to_i64(),
            0,
            "expected previous-value 0 (counter initial); got {i}"
        ),
        other => panic!("expected Integer return value, got {other:?}"),
    }
}

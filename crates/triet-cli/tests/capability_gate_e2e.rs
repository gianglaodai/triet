//! Integration tests for capability gating.
use std::{fs, process::Command};
use tempfile::TempDir;

/// Run `dao check` in a given working directory.
fn run_dao_check(cwd: &std::path::Path, target: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dao"));
    cmd.args(["check", target]).current_dir(cwd);
    cmd.output().expect("failed to execute CLI")
}

#[test]
fn e2e_capability_granted() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.raw_thread grant\nrequires sys.atomic grant\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.raw_thread.spawn\nimport sys.atomic.fetch_add\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success(), "Expected OK when granted");
}

#[test]
fn e2e_capability_missing_manifest() {
    let temp = TempDir::new().unwrap();
    // No dao.package
    fs::write(
        temp.path().join("main.tri"),
        "import sys.raw_thread.spawn\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2200")); // MissingCapabilityClaim
}

#[test]
fn e2e_capability_manifest_missing_claim() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.atomic.fetch_add\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2200")); // MissingCapabilityClaim
}

#[test]
fn e2e_capability_denied() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.raw_thread deny\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.raw_thread.spawn\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2201")); // SelfContradictoryCapability
}

#[test]
fn e2e_capability_deferred() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.atomic defer\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.atomic.fetch_add\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    // At compile-time, defer is accepted.
    assert!(output.status.success());
}

#[test]
fn e2e_atomic_type_parsing() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.atomic grant\n",
    )
    .unwrap();
    // v0.9.x.atomic.5b: `Atomic<T>` is a built-in type — no import needed.
    // Stdlib `sys.atomic` ships only `Ordering` + builtins (per ADR-0028 §8).
    fs::write(
        temp.path().join("main.tri"),
        "function use_atomic(a: &+ Atomic<Integer>) {}\nfunction main() {}\n",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_borrow_exclusivity_atomic() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.atomic grant\n",
    )
    .unwrap();
    // v0.9.x.atomic.5b: stdlib `sys.atomic.fetch_add` now has real signature
    // `(&+ Atomic<Integer>, Integer, Ordering) -> Integer`. Use `&+ Atomic<T>`
    // (frozen owner, interior mutability per ADR-0028 §5) to compile cleanly.
    fs::write(
        temp.path().join("main.tri"),
        "
        from sys.atomic import Synchronized, fetch_add
        function use_atomic(a: &+ Atomic<Integer>) {
            let old = fetch_add(a, 1, Synchronized)
        }
        function main() {}
        ",
    )
    .unwrap();

    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_capability_sys_io_granted() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.io grant\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.io.println\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_capability_sys_io_missing() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import sys.io.println\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2200"));
}

#[test]
fn e2e_capability_from_import_multiple() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.atomic grant\n",
    )
    .unwrap();
    // v0.9.x.atomic.5b: `Atomic` is built-in type — not imported. Test multi-name
    // `from import` against real stdlib items (`Ordering` enum + `fetch_add` fn).
    fs::write(
        temp.path().join("main.tri"),
        "from sys.atomic import Ordering, fetch_add\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_capability_from_import_aliased() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.raw_thread grant\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "from sys.raw_thread import spawn as launch_thread\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_capability_dev_root() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires dev.ffi grant\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import dev.ffi.Ptr\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_capability_usr_root() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires usr.app grant\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "import usr.app.Plugin\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_ambient_std_root_no_manifest() {
    let temp = TempDir::new().unwrap();
    // std is ambient and does not require capability claim!
    fs::write(
        temp.path().join("main.tri"),
        "import std.math.max\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_ambient_core_root_no_manifest() {
    let temp = TempDir::new().unwrap();
    // core is ambient and does not require capability claim!
    fs::write(
        temp.path().join("main.tri"),
        "import core.ops.Add\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_manifest_parse_error() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test_pkg\nversion 1.0.0\nrequires sys.raw_thread UNKNOWN_LEVEL\n",
    )
    .unwrap();
    fs::write(temp.path().join("main.tri"), "function main() {}\n").unwrap();
    // dao check will fail parsing the manifest before capability checking
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
}

#[test]
fn e2e_duplicate_capability_decl() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test\nversion 1.0.0\nrequires sys.io grant\nrequires sys.io grant\n",
    )
    .unwrap();
    fs::write(temp.path().join("main.tri"), "function main() {}\n").unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2204")); // DuplicateCapabilityDecl
}

#[test]
fn e2e_invalid_capability_root() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test\nversion 1.0.0\nrequires invalid.root grant\n",
    )
    .unwrap();
    fs::write(temp.path().join("main.tri"), "function main() {}\n").unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2206")); // InvalidCapabilityRoot
}

#[test]
fn e2e_atomic_counter_demo_check() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("dao.package"),
        "format_version 1\nname test\nversion 1.0.0\nrequires sys.raw_thread grant\nrequires sys.atomic grant\n",
    )
    .unwrap();
    // v0.9.x.atomic.5b: real stdlib signature requires `Ordering` arg.
    fs::write(
        temp.path().join("main.tri"),
        "from sys.atomic import Synchronized, fetch_add\nfrom sys.raw_thread import spawn\nfunction spawn_worker(counter: &+ Atomic<Integer>) {\n    let old = fetch_add(counter, 1, Synchronized)\n}\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(output.status.success());
}

#[test]
fn e2e_atomic_counter_no_manifest() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "from sys.atomic import fetch_add\nfrom sys.raw_thread import spawn\nfunction main() {}\n",
    )
    .unwrap();
    let output = run_dao_check(temp.path(), "main.tri");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should emit 2 E2200 errors (sys.atomic, sys.raw_thread) deduplicated
    assert_eq!(stderr.matches("E2200").count(), 2);
}

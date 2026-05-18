//! End-to-end integration tests for the v0.7.4.3-error capstone
//! (`demos/05-error-handling/`). Runs the demo through the production
//! VM path (`triet build` → `triet run .triv`) and verifies each
//! pipeline arm produces the expected output line.
//!
//! Per the demo's README, the interpreter tier only supports `~0` —
//! the full outcome opcodes (`~+`/`~-`/`~?`/`~:`) are deferred to
//! the "interpreter parity" follow-up. So this test invokes the
//! release-built `triet` binary to exercise the VM path.

use std::path::{Path, PathBuf};
use std::process::Command;

use miette::Diagnostic;

/// Locate the demo's `main.tri` entry point relative to the test's
/// `CARGO_MANIFEST_DIR` (the `triet-cli` crate directory).
fn demo_main() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("demos")
        .join("05-error-handling")
        .join("main.tri")
}

/// Path to the release-built `triet` binary. Tests assume
/// `cargo build --release` has run; the CI harness does this before
/// the test pass per `differential_tests.rs` precedent.
fn triet_bin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("release")
        .join("triet")
}

#[test]
fn error_handling_demo_loads_and_typechecks() {
    let resolved =
        triet_modules::load_program(&demo_main()).expect("demo should parse + resolve");
    let diagnostics = triet_typecheck::check_resolved(&resolved);
    let hard_errors: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(
        hard_errors.is_empty(),
        "type errors in error-handling demo: {hard_errors:#?}",
    );
}

#[test]
fn error_handling_demo_builds_to_triv() {
    let demo = demo_main();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let output_triv = tempdir.path().join("capstone.triv");
    let exit = Command::new(triet_bin())
        .arg("build")
        .arg(&demo)
        .arg("-o")
        .arg(&output_triv)
        .output()
        .expect("failed to execute triet build");
    assert!(
        exit.status.success(),
        "triet build failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&exit.stdout),
        String::from_utf8_lossy(&exit.stderr),
    );
    assert!(
        output_triv.exists(),
        ".triv output should exist after build",
    );
}

/// Full-pipeline run through the VM path. Verifies that every line in
/// the demo's expected output appears in stdout, in order. This is
/// the load-bearing capstone test — if any feature in the
/// v0.7.4.3-error stack regresses, this test fires.
#[test]
fn error_handling_demo_runs_and_emits_expected_lines() {
    let demo = demo_main();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let output_triv = tempdir.path().join("capstone.triv");

    let build = Command::new(triet_bin())
        .arg("build")
        .arg(&demo)
        .arg("-o")
        .arg(&output_triv)
        .output()
        .expect("failed to execute triet build");
    assert!(build.status.success(), "build failed: {build:?}");

    let run = Command::new(triet_bin())
        .arg("run")
        .arg(&output_triv)
        .output()
        .expect("failed to execute triet run");
    assert!(
        run.status.success(),
        "run failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    let expected_lines = [
        // ── Pipeline success ──
        "ok    : UserAccount{age=25, nick=alice}",
        // ── Pipeline failure at each stage ──
        "age<=0: ConfigError[age]: must be strictly positive",
        "old   : ConfigError[age_range]: value outside allowed [lo, hi] window",
        "noname: ConfigError[nickname]: must not be empty",
        // ── Ternary outcome, all three arms ──
        "admin : role=admin",
        "mod   : role=moderator",
        "noroll: role=(none assigned)",
        "bad   : ConfigError[registry]: invalid user id (must be > 0)",
        "range : ConfigError[registry]: id out of range (max 100)",
    ];
    for line in &expected_lines {
        assert!(
            stdout.contains(line),
            "missing expected line {line:?}\nfull stdout:\n{stdout}",
        );
    }
}

/// Pipeline order check: success line must come before the first
/// failure line in stdout. Pins the `register_user` ordering — the
/// pipeline doesn't reorder cases.
#[test]
fn error_handling_demo_stdout_lines_in_expected_order() {
    let demo = demo_main();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let output_triv = tempdir.path().join("capstone.triv");

    Command::new(triet_bin())
        .arg("build")
        .arg(&demo)
        .arg("-o")
        .arg(&output_triv)
        .output()
        .expect("build");
    let run = Command::new(triet_bin())
        .arg("run")
        .arg(&output_triv)
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();

    let pos_ok = stdout.find("ok    :").expect("missing 'ok' line");
    let pos_age = stdout.find("age<=0:").expect("missing 'age<=0' line");
    let pos_admin = stdout.find("admin :").expect("missing 'admin' line");
    let pos_range = stdout.find("range :").expect("missing 'range' line");

    assert!(pos_ok < pos_age, "success line should precede failures");
    assert!(pos_age < pos_admin, "register_user block precedes role-lookup block");
    assert!(pos_admin < pos_range, "role-lookup arms in declared order");
}

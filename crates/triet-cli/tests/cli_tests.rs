//! CLI integration tests.

use std::{
    fs,
    process::{Command, Output},
};

use tempfile::TempDir;

/// Helper to run the `triet` CLI binary.
fn run_cli(args: &[&str], cwd: &std::path::Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_triet"));
    cmd.args(args).current_dir(cwd);
    cmd.output().expect("failed to execute CLI")
}

fn run_cli_snapshot(args: &[&str], cwd: &std::path::Path) -> String {
    let output = run_cli(args, cwd);
    let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Replace the random temp directory path with a stable string
    let cwd_str = cwd.to_str().unwrap();
    stderr = stderr.replace(cwd_str, "<TEMP_DIR>");

    // Replace backslashes with forward slashes for Windows compatibility
    stderr = stderr.replace('\\', "/");

    stderr
}

#[test]
fn single_file_backward_compat() {
    let temp = TempDir::new().unwrap();
    let src = "function main() -> Integer = 42";
    let file_path = temp.path().join("main.tri");
    fs::write(&file_path, src).unwrap();

    let output = run_cli(&["run", "main.tri"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn multi_file_success() {
    let temp = TempDir::new().unwrap();

    // helper.tri
    fs::write(
        temp.path().join("helper.tri"),
        "public constant VALUE: Integer = 100",
    )
    .unwrap();

    // main.tri
    fs::write(
        temp.path().join("main.tri"),
        "module helper\nfrom crate.helper import VALUE\nfunction main() -> Integer = VALUE",
    )
    .unwrap();

    let output = run_cli(&["run", "main.tri"], temp.path());
    assert!(
        output.status.success(),
        "run failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "100");
}

#[test]
fn cyclic_import_error() {
    let temp = TempDir::new().unwrap();

    fs::write(
        temp.path().join("a.tri"),
        "module b\nfrom crate.b import VALUE\npublic constant VALUE: Integer = 1",
    )
    .unwrap();

    fs::write(
        temp.path().join("b.tri"),
        "module a\nfrom crate.a import VALUE\npublic constant VALUE: Integer = 2",
    )
    .unwrap();

    let stderr = run_cli_snapshot(&["check", "a.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn file_not_found_error() {
    let temp = TempDir::new().unwrap();

    fs::write(
        temp.path().join("main.tri"),
        "module missing\nfunction main() -> Integer = 0",
    )
    .unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn visibility_violation() {
    let temp = TempDir::new().unwrap();

    fs::write(
        temp.path().join("secret.tri"),
        "constant HIDDEN: Integer = 42", // private by default
    )
    .unwrap();

    fs::write(
        temp.path().join("main.tri"),
        "module secret\nfrom crate.secret import HIDDEN\nfunction main() -> Integer = HIDDEN",
    )
    .unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn reserved_namespace() {
    let temp = TempDir::new().unwrap();

    fs::write(
        temp.path().join("main.tri"),
        "from sys.core import system\nfunction main() -> Integer = 0",
    )
    .unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

// ── Error exit code propagation (v0.3 safety audit) ──────────────────

#[test]
fn file_not_found_exit_code_2() {
    let temp = TempDir::new().unwrap();
    let output = run_cli(&["run", "nonexistent.tri"], temp.path());
    // Load errors → exit code 2
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn type_error_exit_code_3() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "function main() -> Integer = true",
    )
    .unwrap();
    let output = run_cli(&["run", "main.tri"], temp.path());
    // Type errors → exit code 3
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn runtime_error_exit_code_4() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "function main() -> Integer = 1 / 0",
    )
    .unwrap();
    let output = run_cli(&["run", "main.tri"], temp.path());
    // Runtime errors → exit code 4
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn build_and_run_triv_round_trip() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("hello.tri"),
        r#"function main() { println("hello round trip") }"#,
    )
    .unwrap();

    // Build step.
    let build = run_cli(&["build", "hello.tri", "-o", "hello.triv"], temp.path());
    assert!(
        build.status.success(),
        "build failed: {:?}",
        String::from_utf8_lossy(&build.stderr)
    );

    // Run the .triv file.
    let run = run_cli(&["run", "hello.triv"], temp.path());
    assert!(
        run.status.success(),
        "run failed: {:?}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout.trim(), "hello round trip");
}

#[test]
fn build_default_output_name() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("prog.tri"),
        "function main() -> Integer = 1",
    )
    .unwrap();

    let build = run_cli(&["build", "prog.tri"], temp.path());
    assert!(
        build.status.success(),
        "build failed: {:?}",
        String::from_utf8_lossy(&build.stderr)
    );
    // Default output should be prog.triv.
    assert!(temp.path().join("prog.triv").exists());
}

#[test]
fn run_triv_with_corrupted_file_exit_code_5() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("bad.triv"), b"not a valid triv file").unwrap();
    let output = run_cli(&["run", "bad.triv"], temp.path());
    // Corrupted .triv → exit code 5
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn run_source_auto_detects_tri_extension() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "function main() -> Integer = 99",
    )
    .unwrap();
    // .tri should still run through the interpreter.
    let output = run_cli(&["run", "main.tri"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "99");
}

// ── v0.5.8: cross-module enum variant import ────────────────────────

/// `from std.result import Result, Ok, Err` — variant imports bind to
/// the parent enum so the constructor + pattern resolve at runtime.
#[test]
fn variant_import_from_std_result_runs() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        r#"from std.result import Result, Ok, Err

function divide(a: Integer, b: Integer) -> Result<Integer, String> = if b == 0 { Err("division by zero") } else { Ok(a / b) }

function main() -> Integer = match divide(10, 2) {
    Ok(v) => v,
    Err(_) => -1,
}
"#,
    )
    .unwrap();
    let output = run_cli(&["run", "main.tri"], temp.path());
    assert!(
        output.status.success(),
        "run failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "5");
}

/// `from std.result import Ok as MyOk` is rejected with E2107 —
/// variant aliasing isn't supported.
#[test]
fn aliased_variant_import_rejected_by_cli() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("main.tri"),
        "from std.result import Ok as MyOk\nfunction main() -> Integer = 0",
    )
    .unwrap();
    let output = run_cli(&["check", "main.tri"], temp.path());
    assert!(!output.status.success(), "expected check to fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E2107") || stderr.contains("cannot be imported under an alias"),
        "stderr: {stderr}"
    );
}

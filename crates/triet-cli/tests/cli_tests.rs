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
    ).unwrap();
    
    // main.tri
    fs::write(
        temp.path().join("main.tri"),
        "module helper\nfrom crate.helper import VALUE\nfunction main() -> Integer = VALUE",
    ).unwrap();

    let output = run_cli(&["run", "main.tri"], temp.path());
    assert!(output.status.success(), "run failed: {:?}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "100");
}

#[test]
fn cyclic_import_error() {
    let temp = TempDir::new().unwrap();
    
    fs::write(
        temp.path().join("a.tri"),
        "module b\nfrom crate.b import VALUE\npublic constant VALUE: Integer = 1",
    ).unwrap();
    
    fs::write(
        temp.path().join("b.tri"),
        "module a\nfrom crate.a import VALUE\npublic constant VALUE: Integer = 2",
    ).unwrap();

    let stderr = run_cli_snapshot(&["check", "a.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn file_not_found_error() {
    let temp = TempDir::new().unwrap();
    
    fs::write(
        temp.path().join("main.tri"),
        "module missing\nfunction main() -> Integer = 0",
    ).unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn visibility_violation() {
    let temp = TempDir::new().unwrap();
    
    fs::write(
        temp.path().join("secret.tri"),
        "constant HIDDEN: Integer = 42", // private by default
    ).unwrap();
    
    fs::write(
        temp.path().join("main.tri"),
        "module secret\nfrom crate.secret import HIDDEN\nfunction main() -> Integer = HIDDEN",
    ).unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

#[test]
fn reserved_namespace() {
    let temp = TempDir::new().unwrap();
    
    fs::write(
        temp.path().join("main.tri"),
        "from sys.core import system\nfunction main() -> Integer = 0",
    ).unwrap();

    let stderr = run_cli_snapshot(&["check", "main.tri"], temp.path());
    insta::assert_snapshot!(stderr);
}

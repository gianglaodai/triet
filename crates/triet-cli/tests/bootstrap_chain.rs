//! Integration tests for compiler bootstrap.
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
#[ignore = "Slow on VM dev tier (>15min)"]
fn e2e_bootstrap_chain() {
    let temp = TempDir::new().unwrap();

    // Ensure target/release directory exists (optional, but good practice per user request)
    let release_dir = std::env::current_dir()
        .unwrap()
        .join("target")
        .join("release");
    fs::create_dir_all(&release_dir).unwrap();

    let trietc_stage1 = release_dir.join("trietc.khi");
    let trietc_stage2 = temp.path().join("trietc2.khi");

    let dao_bin = env!("CARGO_BIN_EXE_dao");

    // Cargo runs tests with CWD = crates/triet-cli. We need the workspace root.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    // Phase 1: Build the Stage 1 compiler using the Rust-based CLI
    println!("--- Building Stage 1 compiler ---");
    let status = Command::new(dao_bin)
        .current_dir(workspace_root)
        .arg("build")
        .arg("compiler/main.tri")
        .arg("-o")
        .arg(&trietc_stage1)
        .status()
        .expect("Failed to execute dao build for Stage 1");
    assert!(status.success(), "Stage 1 compilation failed");
    assert!(trietc_stage1.exists(), "Stage 1 binary not found");

    // Phase 2: Verify Stage 1 can compile a simple file (smoke test)
    println!("--- Running Stage 1 smoke test ---");
    let hw_src = temp.path().join("hello.tri");
    fs::write(&hw_src, "function main() {}\n").unwrap();
    let hw_out = temp.path().join("hello.khi");

    let status = Command::new(dao_bin)
        .current_dir(workspace_root)
        .arg("run")
        .arg(&trietc_stage1)
        .arg("--")
        .arg("build")
        .arg(&hw_src)
        .arg("-o")
        .arg(&hw_out)
        .status()
        .expect("Failed to execute dao run for smoke test");
    assert!(status.success(), "Stage 1 smoke test failed");
    assert!(hw_out.exists(), "Smoke test output not found");

    // Phase 3: Build the Stage 2 compiler using the Stage 1 compiler
    println!("--- Building Stage 2 compiler ---");
    let status = Command::new(dao_bin)
        .current_dir(workspace_root)
        .arg("run")
        .arg(&trietc_stage1)
        .arg("--")
        .arg("build")
        .arg("compiler/main.tri")
        .arg("-o")
        .arg(&trietc_stage2)
        .status()
        .expect("Failed to execute dao run for Stage 2");
    assert!(status.success(), "Stage 2 compilation failed");
    assert!(trietc_stage2.exists(), "Stage 2 binary not found");

    // Phase 4: Verify Idempotence (byte-identical)
    println!("--- Verifying Idempotence ---");
    let b1 = fs::read(&trietc_stage1).unwrap();
    let b2 = fs::read(&trietc_stage2).unwrap();

    assert_eq!(
        b1, b2,
        "Stage 1 and Stage 2 compiler binaries are NOT byte-identical!"
    );
    println!("Success: Stage 1 and Stage 2 are byte-identical.");
}

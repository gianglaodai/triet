//! Integration tests for `dao store` subcommands.
//!
//! Each test points the CLI at a fresh `$TRIET_STORE` (`TempDir`) so
//! cases run in isolation and never touch the user's real store.

use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;
use triet_pack::{
    AbiMetadata, FunctionExport, Param, SemVer, TermIfaceHash, TermImplHash, TypeRef, Visibility,
    write_khi,
};

/// Run the `triet` binary with the given args + `TRIET_STORE` env.
fn run_cli(args: &[&str], store: &std::path::Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dao"));
    cmd.args(args)
        .env("TRIET_STORE", store)
        // Don't let HOME affect store resolution if TRIET_STORE is set —
        // but unset just to be defensive against test pollution.
        .env_remove("HOME");
    cmd.output().expect("CLI to execute")
}

fn mk_pack(name: &str, version: SemVer, body_suffix: u8) -> Vec<u8> {
    let mut meta = AbiMetadata::empty(name, version);
    meta.exports.push(FunctionExport {
        name: "f".into(),
        module_path: String::new(),
        visibility: Visibility::Public,
        type_params: Vec::new(),
        params: vec![Param {
            name: "x".into(),
            type_ref: TypeRef::Primitive(0x02),
        }],
        return_type: TypeRef::Primitive(0x02),
        body_offset: 0,
        iface_hash_term: TermIfaceHash::default(),
        impl_hash_term: TermImplHash::default(),
    });
    write_khi(&meta, &[body_suffix])
}

#[test]
fn empty_store_list_prints_placeholder() {
    let store = TempDir::new().unwrap();
    let out = run_cli(&["store", "list"], store.path());
    assert!(
        out.status.success(),
        "expected success, stderr: {:?}",
        out.stderr
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("(store is empty)"), "got: {stdout:?}");
}

#[test]
fn import_then_list_shows_pack() {
    let store = TempDir::new().unwrap();
    let pack_path = store.path().join("foo.khi");
    fs::write(&pack_path, mk_pack("foo", SemVer::new(1, 2, 3), 0x42)).unwrap();

    let import_out = run_cli(
        &["store", "import", pack_path.to_str().unwrap()],
        store.path(),
    );
    assert!(
        import_out.status.success(),
        "import failed: {:?}",
        String::from_utf8_lossy(&import_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&import_out.stdout);
    assert!(stdout.contains("Installed"), "got: {stdout:?}");

    let list_out = run_cli(&["store", "list"], store.path());
    assert!(list_out.status.success());
    let list_stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_stdout.contains("foo"),
        "list missing pkg name: {list_stdout:?}"
    );
    assert!(
        list_stdout.contains("1.2.3"),
        "list missing version: {list_stdout:?}"
    );
}

#[test]
fn import_emits_error_on_missing_file() {
    let store = TempDir::new().unwrap();
    let out = run_cli(&["store", "import", "ghost.khi"], store.path());
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("can't read") || stderr.contains("Error"),
        "got stderr: {stderr:?}"
    );
}

#[test]
fn gc_runs_on_empty_store() {
    let store = TempDir::new().unwrap();
    let out = run_cli(&["store", "gc"], store.path());
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Garbage-collected"), "got: {stdout:?}");
    assert!(stdout.contains("0 pkg dirs"));
}

#[test]
fn list_full_flag_shows_long_hash() {
    let store = TempDir::new().unwrap();
    let pack_path = store.path().join("bar.khi");
    fs::write(&pack_path, mk_pack("bar", SemVer::new(0, 1, 0), 0x01)).unwrap();
    let _ = run_cli(
        &["store", "import", pack_path.to_str().unwrap()],
        store.path(),
    );

    let short = run_cli(&["store", "list"], store.path());
    let full = run_cli(&["store", "list", "--full"], store.path());
    let short_stdout = String::from_utf8_lossy(&short.stdout);
    let full_stdout = String::from_utf8_lossy(&full.stdout);

    // Short form uses an ellipsis character; full form doesn't.
    assert!(
        short_stdout.contains('…'),
        "short form missing ellipsis: {short_stdout:?}"
    );
    assert!(
        !full_stdout.contains('…'),
        "full form should not have ellipsis: {full_stdout:?}"
    );
    // Full form has a 64-char hex hash somewhere in the line.
    let has_64hex = full_stdout.lines().any(|line| {
        line.split_whitespace()
            .any(|tok| tok.len() == 64 && tok.chars().all(|c| c.is_ascii_hexdigit()))
    });
    assert!(has_64hex, "full form missing 64-hex hash: {full_stdout:?}");
}

/// Run with a custom `$HOME` and no `$TRIET_STORE` — exercises the
/// `$HOME/.triet/store` fallback path in `resolve_store_root`.
fn run_cli_with_home(args: &[&str], home: &std::path::Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dao"));
    cmd.args(args).env_remove("TRIET_STORE").env("HOME", home);
    cmd.output().expect("CLI to execute")
}

#[test]
fn store_root_falls_back_to_home_when_env_unset() {
    // `$TRIET_STORE` unset → CLI uses `$HOME/.triet/store`. Verifies
    // the second arm of `resolve_store_root` actually runs and produces
    // a usable store.
    let home = TempDir::new().unwrap();
    let out = run_cli_with_home(&["store", "list"], home.path());
    assert!(
        out.status.success(),
        "expected success, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Store layout should exist under $HOME/.triet/store/.
    let store_root = home.path().join(".triet").join("store");
    for sub in ["term", "mod", "pkg", "names", "roots", "tmp"] {
        assert!(
            store_root.join(sub).is_dir(),
            "missing {sub} under {}",
            store_root.display()
        );
    }
}

#[test]
fn store_root_errors_when_both_env_unset() {
    // Neither $TRIET_STORE nor $HOME → `resolve_store_root` returns
    // an explicit error rather than silently picking a default.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dao"));
    cmd.args(["store", "list"])
        .env_remove("TRIET_STORE")
        .env_remove("HOME");
    let out = cmd.output().expect("CLI to execute");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("HOME") || stderr.contains("TRIET_STORE"),
        "expected explanatory error mentioning HOME/TRIET_STORE, got: {stderr:?}"
    );
}

#[test]
fn json_list_emits_one_object_per_line() {
    let store = TempDir::new().unwrap();
    let pack_path = store.path().join("baz.khi");
    fs::write(&pack_path, mk_pack("baz", SemVer::new(2, 0, 0), 0x99)).unwrap();
    let _ = run_cli(
        &["store", "import", pack_path.to_str().unwrap()],
        store.path(),
    );

    let out = run_cli(&["--json", "store", "list"], store.path());
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Each row should be a JSON-ish object with "pkg" and "version".
    assert!(stdout.contains("\"pkg\":\"baz\""), "got: {stdout:?}");
    assert!(stdout.contains("\"version\":\"2.0.0\""), "got: {stdout:?}");
}

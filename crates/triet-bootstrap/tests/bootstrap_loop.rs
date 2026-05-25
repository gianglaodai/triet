//! v0.7.11.8 — Stage 1 → Stage 2 bootstrap-loop gate (ADR-0019
//! §6 Layer 3). Reuses the Stage 1 IR from
//! `bootstrap_self_compile.rs` (`compiler/main.tri` loaded by Rust).
//!
//! ## What this test covers
//!
//! `stage2_compiles_factorial_self_byte_identical` — Stage 2
//! (Triết-in-Triết running inside the VM) compiles
//! `compiler/factorial.tri` and the output MUST be byte-identical
//! to the Rust reference build. This is the primary gate for
//! v0.7.11: single-fixture self-hosting confirmed.
//!
//! `stage2_factorial_emit_is_deterministic` — two independent
//! Stage 2 runs produce identical bytes. Catches non-determinism
//! independent of the Rust mirror.
//!
//! `stage2_factorial_decodes_via_rust_reader` — Stage 2 output
//! is structurally valid `.khi` per the Rust reader.
//!
//! ## What is deferred to v0.7.12
//!
//! Self-compile of `compiler/main.tri` itself. Stage 2's
//! `serialize_program_to_khi` calls `load_program_from_source`
//! (in-memory entry point) which cannot resolve file-bound
//! `module parser;` declarations. The Rust pipeline uses
//! `load_program` (filesystem entry) which walks the filesystem
//! for child modules. To self-compile main.tri we need either:
//!
//!   1. A filesystem-aware Triết-side loader (new builtin or
//!      `set_cwd` / `chdir` before `load_program`), OR
//!   2. Stage ALL compiler/*.tri files into the tempdir with
//!      correct relative layout (fragile in CI).
//!
//! Both land alongside the Stage 2 → Stage 3 gate in v0.7.12.
//!
//! ## What v0.7.11.8 adds over `bootstrap_self_compile.rs`
//!
//! `bootstrap_self_compile.rs` (shipped v0.7.9.5) proved byte-
//! identical output for factorial.tri using a single-fixture
//! flow. This file consolidates the three test cases into a
//! dedicated "bootstrap loop" module with the shared
//! `triet_build_source` helper, establishing the convention for
//! the v0.7.12 full-loop gate (Stage 2 → Stage 3 bit-identical).
//!
//! See [ADR-0019 §6] + §A7.11/12.
//!
//! [ADR-0019 §6]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

use std::path::PathBuf;
use std::sync::OnceLock;
use std::{fs, panic};

use miette::Diagnostic;
use tempfile::TempDir;

use triet_ir::{FuncId, IrProgram, RuntimeValue, Vm, lower_program, read_program, write_program};
use triet_modules::load_program;
use triet_pack::{AbiMetadata, SemVer, read_khi, write_khi};
use triet_typecheck::check_resolved;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn compiler_main_path() -> PathBuf {
    workspace_root().join("compiler").join("main.tri")
}

fn factorial_source_path() -> PathBuf {
    workspace_root().join("compiler").join("factorial.tri")
}

/// Cached IR of `compiler/main.tri` — Stage 1 IR used as the
/// bootloader for every Stage 2 invocation. Same `OnceLock`
/// pattern as `bootstrap_self_compile.rs`.
fn main_ir() -> &'static IrProgram {
    static IR: OnceLock<IrProgram> = OnceLock::new();
    IR.get_or_init(|| {
        let path = compiler_main_path();
        assert!(path.is_file(), "missing main.tri at {}", path.display());
        let resolved = load_program(&path).expect("load_program");
        let diagnostics = check_resolved(&resolved);
        let blocking: Vec<_> = diagnostics
            .iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect();
        assert!(
            blocking.is_empty(),
            "type errors in main.tri: {blocking:#?}"
        );
        lower_program(&resolved)
    })
}

fn lookup_func(ir: &IrProgram, name: &str) -> FuncId {
    ir.modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function `{name}`"))
        .id
}

fn string_vec(items: &[&str]) -> RuntimeValue {
    RuntimeValue::Vector(
        items
            .iter()
            .map(|s| RuntimeValue::String((*s).to_owned()))
            .collect(),
    )
}

/// Drive the Triết-side `main.tri::main` with `build` subcommand
/// over `compiler/<fixture>.tri`, return the `.khi` bytes.
/// Source is staged into a tempdir so the build's `-o` lands
/// independently per test.
fn triet_build_fixture(fixture_name: &str, pkg_name: &str) -> Vec<u8> {
    let source_path = workspace_root()
        .join("compiler")
        .join(format!("{fixture_name}.tri"));
    let source =
        fs::read_to_string(&source_path).unwrap_or_else(|_| panic!("read {fixture_name}.tri"));

    let temp = TempDir::new().expect("tempdir");
    let staged = temp.path().join(format!("{fixture_name}.tri"));
    fs::write(&staged, &source).expect("stage fixture source");

    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main");
    let mut vm = Vm::new(ir);

    let staged_str = staged.to_str().expect("UTF-8 path").to_owned();
    let out_path = temp.path().join("out.khi");
    let out_str = out_path.to_str().expect("UTF-8 path").to_owned();

    let argv = string_vec(&["build", &staged_str, "-o", &out_str, "--pkg", pkg_name]);
    let result = vm
        .execute(func_id, vec![argv])
        .expect("main(build) must execute without VM error");
    match result {
        RuntimeValue::Trit(trit) if trit.is_positive() => {}
        other => panic!("expected Trit::Positive from main build, got {other:?}"),
    }

    fs::read(&out_path).expect("read emitted .khi")
}

/// Rust-side reference build: load+typecheck+lower a single-module
/// source (no stdlib), emit `.khi` with empty ABI metadata.
fn rust_emit(source: &str, pkg_name: &str) -> Vec<u8> {
    let resolved = triet_modules::load_program_from_source_no_stdlib(source)
        .expect("no-stdlib loader must accept fixture");
    let ir = lower_program(&resolved);
    let code = write_program(&ir);
    let meta = AbiMetadata::empty(pkg_name, SemVer::new(0, 0, 0));
    write_khi(&meta, &code)
}

// ── Tests ─────────────────────────────────────────────────────────

#[test]
fn stage2_compiles_factorial_self_byte_identical() {
    // v0.7.11.8 gate: Stage 2 output for factorial.tri MUST be
    // byte-identical to the Rust reference build. This is the
    // primary ADR-0019 §6 Layer 3 assertion.
    let triet_bytes = triet_build_fixture("factorial", "factorial");
    let fixture_path = factorial_source_path();
    let source = fs::read_to_string(&fixture_path).expect("read factorial.tri");
    let rust_bytes = rust_emit(&source, "factorial");

    assert_eq!(
        triet_bytes,
        rust_bytes,
        "Stage 2 factorial.tri must match Rust reference; \
         Triết: {} bytes, Rust: {} bytes",
        triet_bytes.len(),
        rust_bytes.len(),
    );
}

#[test]
fn stage2_factorial_emit_is_deterministic() {
    // Two independent Stage 2 runs over factorial.tri produce
    // identical `.khi` bytes.
    let bytes1 = triet_build_fixture("factorial", "factorial");
    let bytes2 = triet_build_fixture("factorial", "factorial");
    assert_eq!(
        bytes1,
        bytes2,
        "Stage 2 factorial build must be deterministic; lengths: {} vs {}",
        bytes1.len(),
        bytes2.len(),
    );
}

#[test]
fn stage2_factorial_decodes_via_rust_reader() {
    // Stage 2-emitted `.khi` must decode cleanly via the Rust
    // reader — verifies structural validity (magic, sections,
    // ABI metadata, embedded .triv).
    let emitted = triet_build_fixture("factorial", "factorial");
    let (meta, code_section) = read_khi(&emitted).expect("Stage 2-emitted .khi must decode");
    assert_eq!(meta.pkg_name, "factorial", "pkg_name mismatch");
    assert!(meta.abi_version == 2, "abi_version must be 2");
    assert!(!code_section.is_empty(), "code section must be non-empty");
    assert!(
        !meta.impl_hash.0.iter().all(|b| *b == 0),
        "impl_hash must be non-zero"
    );
}

// ── v0.7.12.5 — Stage 2 self-compiles main.tri via filesystem ────

/// Stage 2 self-compile of `compiler/main.tri` (~9700 LOC across
/// 6 module files). Marked `#[ignore]` because the VM dev tier
/// runs ~50-200× slower than Rust-native (ADR-0019 §7), so
/// compiling main.tri inside the VM takes minutes — too slow for
/// per-commit CI. Run manually via:
///
/// ```text
/// cargo test --release -p triet-bootstrap --test bootstrap_loop \
///     -- --ignored stage2_self_compiles_main_tri
/// ```
///
/// This is the test the v0.7.12.7 `cmp` gate will lift to non-
/// ignored once Stage 3 production lands. Until then it serves as
/// developer verification that the pipeline runs end-to-end.
#[test]
#[ignore = "slow: compiles ~9700 LOC inside VM, takes minutes per ADR-0019 §7 perf gate"]
fn stage2_self_compiles_main_tri_via_filesystem() {
    // v0.7.12.4 closed the last parser/lowerer blockers (BlockExpr +
    // match-arm-mutation-phi). Combined with .2's filesystem-aware
    // `serialize_program_path_to_khi`, Stage 2 (the VM running main.tri
    // compiled by Stage 1) can now load its own siblings and produce
    // a .khi for itself.
    //
    // Byte-identical comparison with Rust reference defers to v0.7.12.6
    // — current scope verifies the pipeline runs end-to-end without
    // VM error and emits a non-empty, structurally valid .khi.
    let main_path = compiler_main_path();
    let out_dir = TempDir::new().expect("tempdir");
    let out_path = out_dir.path().join("main.khi");

    let ir = main_ir().clone();
    let func_id = lookup_func(&ir, "main");
    let mut vm = Vm::new(ir);

    let main_str = main_path.to_str().expect("UTF-8 path").to_owned();
    let out_str = out_path.to_str().expect("UTF-8 path").to_owned();
    let argv = string_vec(&["build", &main_str, "-o", &out_str, "--pkg", "compiler"]);
    let result = vm
        .execute(func_id, vec![argv])
        .expect("Stage 2 main.tri self-build must execute without VM error");

    match result {
        RuntimeValue::Trit(trit) if trit.is_positive() => {}
        other => {
            panic!("Stage 2 self-build returned non-positive Trit: {other:?} (expected Positive)")
        }
    }

    let emitted = std::fs::read(&out_path).expect("read self-compiled main.khi");
    let (meta, code) = read_khi(&emitted).expect("self-compiled .khi must decode");
    assert_eq!(meta.pkg_name, "compiler");
    assert!(
        !code.is_empty(),
        "self-compiled code section must be non-empty"
    );
}

// ── v0.7.12.6 + .7 — Stage 2 ≡ Stage 3 byte-identical gate ──────

/// Lower a Stage 2-emitted `.khi` back into an `IrProgram` suitable
/// for booting another VM (Stage 3). Extracts the embedded `.triv`
/// code section via `read_khi`, then decodes it via `read_program`.
fn load_stage2_ir(stage2_khi: &[u8]) -> IrProgram {
    let (_meta, code) = read_khi(stage2_khi).expect("Stage 2 .khi must decode");
    read_program(&code).expect("Stage 2 code section must decode as IrProgram")
}

/// Drive a `main.tri::main(["build", ...])` invocation against the
/// given IR (Stage 1 or Stage 2 binary) and return the emitted
/// `.khi` bytes. Shared between Stage 2 and Stage 3 invocations.
fn run_stage_build_main_tri(stage_ir: IrProgram, source_path: &str, pkg_name: &str) -> Vec<u8> {
    let func_id = lookup_func(&stage_ir, "main");
    let mut vm = Vm::new(stage_ir);

    let temp = TempDir::new().expect("tempdir");
    let out_path = temp.path().join("out.khi");
    let out_str = out_path.to_str().expect("UTF-8 path").to_owned();

    let argv = string_vec(&["build", source_path, "-o", &out_str, "--pkg", pkg_name]);
    let result = vm
        .execute(func_id, vec![argv])
        .expect("main(build) must execute without VM error");
    match result {
        RuntimeValue::Trit(trit) if trit.is_positive() => {}
        other => panic!("expected Trit::Positive from main(build), got {other:?}"),
    }
    fs::read(&out_path).expect("read emitted .khi")
}

/// **The v0.7.12 strict gate** per ADR-0019 §1: Stage 2 ≡ Stage 3
/// byte-identical when both compile `compiler/main.tri`. Proves
/// fixed-point convergence — the self-hosting compiler's output
/// no longer depends on which compiler built it.
///
/// Pipeline:
///
/// ```text
/// Stage 1 (Rust) ──compile main.tri──→ Stage 2 compiler IR
/// Stage 2 (VM)  ──compile main.tri──→ Stage 2 output .khi
/// Stage 3 (VM)  ──compile main.tri──→ Stage 3 output .khi
///                                       (Stage 3 ran the Stage 2
///                                        output as its bootloader)
/// GATE: cmp Stage 2 output == Stage 3 output
/// ```
///
/// **`#[ignore]` reason**: each VM run compiling ~9700 LOC takes
/// minutes (ADR-0019 §7 — VM is dev tier, perf parity defers to
/// v0.9 JIT). The full gate runs main.tri compile twice = double
/// that. Run manually before promoting v0.7 to stable:
///
/// ```text
/// cargo test --release -p triet-bootstrap --test bootstrap_loop \
///     -- --ignored stage2_eq_stage3
/// ```
///
/// When this passes, ROADMAP §v0.7 functional gate fires.
#[test]
#[ignore = "slow: runs main.tri compile twice inside VM (~minutes each per ADR-0019 §7)"]
fn stage2_eq_stage3_main_tri_byte_identical() {
    let main_path = compiler_main_path();
    let main_str = main_path.to_str().expect("UTF-8 path").to_owned();

    // Stage 2: run Stage 1's IR (= main_ir()) against main.tri.
    let stage1_ir = main_ir().clone();
    let stage2_khi = run_stage_build_main_tri(stage1_ir, &main_str, "compiler");
    assert!(!stage2_khi.is_empty(), "Stage 2 .khi must be non-empty");

    // Boot Stage 2's emitted .khi back into an IrProgram. This is
    // the "Stage 2 compiler binary" — the same Triết source as
    // main_ir() but built by Stage 1 and round-tripped through
    // `.khi` serde. If the encoder is deterministic, this is
    // observably equivalent to main_ir(); however reading it back
    // from bytes is what makes the test prove "the bootstrap loop
    // works at the wire-format level" (not just the in-memory IR).
    let stage2_ir = load_stage2_ir(&stage2_khi);

    // Stage 3: run Stage 2's IR against main.tri.
    let stage3_khi = run_stage_build_main_tri(stage2_ir, &main_str, "compiler");

    // The gate.
    assert_eq!(
        stage2_khi,
        stage3_khi,
        "Stage 2 ≡ Stage 3 byte-identical FAILED. \
         Stage 2: {} bytes, Stage 3: {} bytes. \
         Investigate per ADR-0019 §4 (nondeterminism source, \
         likely HashMap iter, env var, or compile-time random ID).",
        stage2_khi.len(),
        stage3_khi.len(),
    );
}

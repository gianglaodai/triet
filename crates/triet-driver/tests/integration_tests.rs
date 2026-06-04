//! Integration tests — end-to-end pipeline: .tri → compile → execute → assert.
//!
//! Each `.tri` file in `tests/fixtures/` contains a `// EXPECT: <value>` comment
//! on the first line. The test harness parses the file, runs it through the full
//! Track B pipeline (parse → typecheck → lower → MIR verify → borrowck → JIT),
//! executes `main()`, and asserts the printed result matches EXPECT.
//!
//! Adding a new test is a single file — no Rust code required.
//!
//! # Regressions
//!
//! If a change to the compiler breaks one of these tests, `cargo test` will fail.
//! This is the safety net that replaces the deleted 1637-test Track A oracle.

use std::fs;
use std::path::Path;

use triet_jit::mir_lower::{CompiledFunction, JitContext, ShimSymbol};

/// Parse the first line of a `.tri` file, expecting `// EXPECT: <value>`.
/// Returns the expected i64 value.
fn parse_expected(source: &str) -> Option<i64> {
    let first_line = source.lines().next()?;
    let expect_prefix = "// EXPECT: ";
    first_line
        .strip_prefix(expect_prefix)
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Run one `.tri` file through the full pipeline and return the i64 result.
fn run_fixture(source: &str) -> Result<i64, String> {
    // ── Phase 1: Parse ──
    let (program, parse_errors) = triet_parser::parse(source);
    if !parse_errors.is_empty() {
        return Err(format!("parse errors: {parse_errors:?}"));
    }

    // ── Phase 2: Typecheck (blocking) ──
    let type_errors = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        return Err(format!("type errors: {type_errors:?}"));
    }

    // ── Phase 3: Lower to MIR ──
    let bodies = triet_lower::lower_program(&program).map_err(|e| format!("lowerer error: {e}"))?;

    if bodies.is_empty() {
        return Err("no functions to compile".into());
    }

    // ── Phase 3.5: MIR verification ──
    for body in &bodies {
        body.verify()
            .map_err(|e| format!("MIR verification error: {e}"))?;
    }

    // ── Phase 4: Borrow check (blocking) ──
    for body in &bodies {
        let result = triet_borrowck::checker::check_body(body);
        if !result.is_ok() {
            return Err(format!("borrow errors: {:?}", result.errors));
        }
    }

    // ── Phase 5: JIT compile ──
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let shims = &[ShimSymbol::fn_2_1(
        "__triet_pow",
        triet_jit::mir_lower::__triet_pow,
    )];
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .map_err(|e| format!("JIT compilation error: {e}"))?;

    // ── Phase 6: Find main and execute ──
    let main_body = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .ok_or("no main function found")?;

    if !main_body.signature.params.is_empty() {
        return Err(format!(
            "main() has {} params — Bậc A JIT does not support arguments to main()",
            main_body.signature.params.len()
        ));
    }

    let func: &CompiledFunction = compiled
        .get(&main_body.signature.name)
        .ok_or("main not found in compiled output")?;

    #[allow(unsafe_code)]
    let result: i64 = unsafe { func.call_i64_0() };
    Ok(result)
}

/// Discover all `.tri` fixtures and run them.
#[test]
fn integration_test_corpus() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");

    let mut entries: Vec<_> = fs::read_dir(&fixtures_dir)
        .expect("failed to read fixtures directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "tri"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut passed = 0usize;
    let mut failed = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let source = fs::read_to_string(&path).expect("failed to read fixture");

        let expected = match parse_expected(&source) {
            Some(v) => v,
            None => {
                failed.push((file_name, "missing or invalid // EXPECT: comment".into()));
                continue;
            }
        };

        match run_fixture(&source) {
            Ok(actual) if actual == expected => {
                passed += 1;
                eprintln!("  PASS {file_name} → {actual}");
            }
            Ok(actual) => {
                failed.push((file_name, format!("expected {expected}, got {actual}")));
            }
            Err(e) => {
                failed.push((file_name, e));
            }
        }
    }

    if !failed.is_empty() {
        eprintln!("\n=== FAILURES ===");
        for (name, err) in &failed {
            eprintln!("  FAIL {name}: {err}");
        }
        panic!(
            "{} of {} integration tests failed ({} passed)",
            failed.len(),
            entries.len(),
            passed
        );
    }

    eprintln!(
        "\nAll {passed} integration tests passed ({} fixtures)",
        entries.len()
    );
}

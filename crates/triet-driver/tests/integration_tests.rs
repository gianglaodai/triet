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

/// What the test expects: either a successful run with a specific output,
/// or a pipeline error containing a specific substring.
enum Expected {
    /// Pipeline must succeed and `main()` must return this value.
    Value(i64),
    /// Pipeline must fail (at any phase) with this substring in the error.
    Error(String),
}

/// Parse the first line of a `.tri` file for a test directive.
/// Supports `// EXPECT: <value>` (positive) or `// ERROR: <code>` (negative).
fn parse_directive(source: &str) -> Option<Expected> {
    let first_line = source.lines().next()?;
    if let Some(val) = first_line.strip_prefix("// EXPECT: ") {
        val.trim().parse::<i64>().ok().map(Expected::Value)
    } else {
        first_line
            .strip_prefix("// ERROR: ")
            .map(|code| Expected::Error(code.trim().to_string()))
    }
}

/// Run one `.tri` file through the full pipeline.
///
/// Collects errors from every phase so that a `// ERROR: E2440` directive
/// can match a borrowck error even when earlier phases (typecheck) also
/// produce errors. Returns `Ok(value)` only if ALL phases pass.
fn run_fixture(source: &str) -> Result<i64, String> {
    let mut errors: Vec<String> = Vec::new();

    // ── Phase 1: Parse ──
    let (program, parse_errors) = triet_parser::parse(source);
    if !parse_errors.is_empty() {
        errors.push(format!("parse errors: {parse_errors:?}"));
        // Can't continue without a valid AST
        return Err(errors.join(" | "));
    }

    // ── Phase 2: Typecheck ──
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    if !type_errors.is_empty() {
        for err in &type_errors {
            errors.push(format!("type error: {err}"));
        }
        // Don't return — type errors don't prevent lowering, and we want
        // to reach borrowck so // ERROR: E2440 can match.
    }

    // ── Phase 3: Lower to MIR ──
    let bodies =
        match triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions) {
            Ok(b) => b,
            Err(e) => {
                errors.push(format!("lowerer error: {e}"));
                return Err(errors.join(" | "));
            }
        };

    if bodies.is_empty() {
        errors.push("no functions to compile".into());
        return Err(errors.join(" | "));
    }

    // ── Phase 3.5: MIR verification ──
    for body in &bodies {
        if let Err(e) = body.verify() {
            errors.push(format!("MIR verification: {e}"));
            return Err(errors.join(" | "));
        }
    }

    // ── Phase 4: Borrow check ──
    // ADR-0045 §2/B4 + ADR-0046 §3: collect callee signatures for
    // cross-call loan propagation (PropagatedLoan) via return_borrow_map.
    let callee_sigs: std::collections::BTreeMap<String, triet_mir::FunctionSignature> = bodies
        .iter()
        .map(|b| (b.signature.name.clone(), b.signature.clone()))
        .collect();
    for body in &bodies {
        let result = triet_borrowck::checker::check_body_with(body, &callee_sigs);
        if !result.is_ok() {
            for err in &result.errors {
                errors.push(format!("borrow error: {err}"));
            }
        }
    }

    // If any phase had errors, return them all now
    if !errors.is_empty() {
        return Err(errors.join(" | "));
    }

    // ── Phase 5: JIT compile ──
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let shims = &[
        ShimSymbol::fn_2_1("__triet_pow", triet_jit::mir_lower::__triet_pow),
        ShimSymbol::fn_2_1(
            "__triet_string_alloc",
            triet_jit::mir_lower::__triet_string_alloc,
        ),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            triet_jit::mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0(
            "__triet_string_free",
            triet_jit::mir_lower::__triet_string_free,
        ),
        ShimSymbol::fn_5_0(
            "__triet_string_concat",
            triet_jit::mir_lower::__triet_string_concat,
        ),
        ShimSymbol::fn_4_1("__triet_string_eq", triet_jit::mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1(
            "__triet_string_len",
            triet_jit::mir_lower::__triet_string_len,
        ),
        // Vector shims (ADR-0040 §5; ADR-0077: alloc gains stride arg)
        ShimSymbol::fn_3_1(
            "__triet_vector_alloc",
            triet_jit::mir_lower::__triet_vector_alloc,
        ),
        ShimSymbol::fn_1_0(
            "__triet_vector_free",
            triet_jit::mir_lower::__triet_vector_free,
        ),
        ShimSymbol::fn_1_1(
            "__triet_vector_len",
            triet_jit::mir_lower::__triet_vector_len,
        ),
        ShimSymbol::fn_2_1(
            "__triet_vector_push",
            triet_jit::mir_lower::__triet_vector_push,
        ),
        ShimSymbol::fn_2_1(
            "__triet_vector_get",
            triet_jit::mir_lower::__triet_vector_get,
        ),
        ShimSymbol::fn_2_1(
            "__triet_vector_pop",
            triet_jit::mir_lower::__triet_vector_pop,
        ),
        ShimSymbol::fn_3_1(
            "__triet_hashmap_alloc",
            triet_jit::mir_lower::__triet_hashmap_alloc,
        ),
        ShimSymbol::fn_1_0(
            "__triet_hashmap_free",
            triet_jit::mir_lower::__triet_hashmap_free,
        ),
        ShimSymbol::fn_1_1(
            "__triet_hashmap_len",
            triet_jit::mir_lower::__triet_hashmap_len,
        ),
        ShimSymbol::fn_3_1(
            "__triet_hashmap_insert",
            triet_jit::mir_lower::__triet_hashmap_insert,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get",
            triet_jit::mir_lower::__triet_hashmap_get,
        ),
        ShimSymbol::fn_3_1(
            "__triet_hashmap_remove",
            triet_jit::mir_lower::__triet_hashmap_remove,
        ),
        // ADR-0047: contains shims
        ShimSymbol::fn_4_1(
            "__triet_string_contains",
            triet_jit::mir_lower::__triet_string_contains,
        ),
        ShimSymbol::fn_2_1(
            "__triet_vector_contains",
            triet_jit::mir_lower::__triet_vector_contains,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_contains",
            triet_jit::mir_lower::__triet_hashmap_contains,
        ),
        // ADR-0048/0049: mutable borrow — clear + append
        ShimSymbol::fn_1_1(
            "__triet_string_clear",
            triet_jit::mir_lower::__triet_string_clear,
        ),
        ShimSymbol::fn_2_1(
            "__triet_string_append",
            triet_jit::mir_lower::__triet_string_append,
        ),
        // ADR-0069 Lát 3: capability runtime policy hook (defer mint gate).
        ShimSymbol::fn_1_1("__triet_cap_check", triet_jit::mir_lower::__triet_cap_check),
    ];
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .map_err(|e| format!("JIT compilation error: {e}"))?;

    // ── Phase 6: Find main and execute ──
    let main_body = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .ok_or("no main function found")?;

    if !main_body.signature.parameters.is_empty() {
        return Err(format!(
            "main() has {} parameters — Bậc A JIT does not support arguments to main()",
            main_body.signature.parameters.len()
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

        let expected = match parse_directive(&source) {
            Some(v) => v,
            None => {
                failed.push((
                    file_name,
                    "missing or invalid directive (use // EXPECT: <value> or // ERROR: <code>)"
                        .into(),
                ));
                continue;
            }
        };

        match expected {
            Expected::Value(val) => match run_fixture(&source) {
                Ok(actual) if actual == val => {
                    passed += 1;
                    eprintln!("  PASS {file_name} → {actual}");
                }
                Ok(actual) => {
                    failed.push((file_name, format!("expected {val}, got {actual}")));
                }
                Err(e) => {
                    failed.push((file_name, format!("expected {val}, got error: {e}")));
                }
            },
            Expected::Error(code) => match run_fixture(&source) {
                Ok(actual) => {
                    failed.push((
                        file_name,
                        format!(
                            "expected error containing '{code}', but pipeline succeeded with {actual}"
                        ),
                    ));
                }
                Err(e) if e.contains(&code) => {
                    passed += 1;
                    eprintln!("  PASS {file_name} → error matches '{code}'");
                }
                Err(e) => {
                    failed.push((
                        file_name,
                        format!("expected error containing '{code}', got: {e}"),
                    ));
                }
            },
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

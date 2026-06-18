//! ADR-0062 Heap-Nullable Lát 1 — route-lower Drop-count test for `String?`.
//!
//! Runs the REAL pipeline (parse → typecheck → lower → JIT) on `.tri` programs
//! that bind a `String?` local, swapping `__triet_string_free` for a counting
//! shim. Proves the ptr-sentinel repr: free reads `ptr@0`, so a non-null
//! `String?` is freed exactly once and a null `String?` (`~0`, ptr@0 ==
//! NULL_SENTINEL) is freed zero times (shim no-ops on the sentinel).
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - Non-null count == 1, null count == 0 — distinguishes the two repr states.
//!   - Poison the Drop slot-read offset (slot@0 → slot@8) → free reads `len`,
//!     not `ptr` → non-null count wrong / crash. Proves the offset is
//!     load-bearing, not incidental.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it counts only frees of LIVE
/// allocations — a null `String?` (ptr@0 == NULL_SENTINEL) frees nothing.
#[unsafe(no_mangle)]
extern "C" fn __snull_count_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    FREE_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Replicates the driver's source→bodies pipeline (main.rs phases 1-3).
fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, expr_resolutions, pattern_resolutions, method_resolutions) =
        triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(
        &program,
        &expr_resolutions,
        &pattern_resolutions,
        &method_resolutions,
    )
    .expect("lowering failed")
}

/// Real shim set, but `__triet_string_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __snull_count_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ]
}

/// Compile `source`, call `main`, return (`main`'s result, free count).
fn run_counting(source: &str) -> (i64, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("String? program must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (result, FREE_COUNT.load(Ordering::SeqCst))
}

#[test]
fn nonnull_string_nullable_freed_once() {
    // f() returns a non-null String?; main binds it (owned → dropped at scope
    // exit). The free reads ptr@0 = a real allocation → counted once.
    let (result, frees) = run_counting(
        "function f() -> String? = \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let r = f();\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        frees, 1,
        "non-null String? must be freed exactly once (free reads ptr@0)"
    );
}

#[test]
fn null_string_nullable_freed_zero() {
    // f() returns `~0` (null); main binds it. Drop calls free(ptr@0 ==
    // NULL_SENTINEL, _) → shim no-ops → zero live frees. Proves the sentinel
    // landed in ptr@0.
    let (result, frees) = run_counting(
        "function f() -> String? = ~0\n\
         function main() -> Integer {\n\
         \x20   let r = f();\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        frees, 0,
        "null String? (ptr@0 == NULL_SENTINEL) must free nothing"
    );
}

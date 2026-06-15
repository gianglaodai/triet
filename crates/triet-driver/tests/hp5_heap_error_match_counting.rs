//! HP.5 — route-lower counting test for matching a heap-ERROR Outcome.
//!
//! Runs the REAL pipeline (parse → typecheck → lower → JIT) on a `.tri`
//! program that matches an `Integer~String` and binds the String error,
//! but swaps `__triet_string_free` for a counting shim. Proves the bound
//! heap error is freed exactly once.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - Poison the type fix (neg-arm bind reverts to value_type) → the JIT
//!     refuses with "type 'Integer' is not a known struct" → compile fails.
//!   - Poison the neg-arm Deinit(scrut) → count becomes 2 (double-free).
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{JitContext, ShimSymbol};

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0` guard so it counts only frees of LIVE allocations — a
/// tombstoned (Deinit'd) value's Drop still calls free but frees nothing.
#[unsafe(no_mangle)]
extern "C" fn __hp5_count_free(ptr: i64, cap: i64) {
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

#[test]
fn heap_error_match_frees_exactly_once() {
    use triet_jit::mir_lower;

    // make_err takes the Negative arm with a heap String error; main matches
    // and binds it. The bound String is freed once on scope exit.
    // The scrutinee is a `let`-bound local so it is OWNED and dropped at
    // scope exit (like fixtures 137/138). The match's Deinit(scrut) is thus
    // load-bearing: without it, Drop(scrut) would free the heap error a
    // second time. (A directly-matched call result is not owned/dropped, so
    // it would not exercise the Deinit — see fixture 142 for that shape.)
    let bodies = lower_source(
        "function make_err() -> Integer~String = ~- \"boom\"\n\
         function main() -> Integer {\n\
         \x20   let o: Integer~String = make_err();\n\
         \x20   let r = match o { ~+ x => x  ~- e => 7 };\n\
         \x20   return r;\n\
         }",
    );
    for body in &bodies {
        body.verify().expect("MIR verify");
    }

    // Real shim set, but `__triet_string_free` swapped for the counter.
    let shims = &[
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __hp5_count_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ];

    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("HP.5 heap-error match must JIT-compile (no 'not a known struct')");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };

    assert_eq!(result, 7, "negative arm returns 7");
    assert_eq!(
        FREE_COUNT.load(Ordering::SeqCst),
        1,
        "the bound heap error must be freed exactly once (no leak, no double-free)"
    );
}

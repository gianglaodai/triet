//! ADR-0055 §4 — route-lower counting test for the parity-return-heap cell.
//!
//! Runs the REAL pipeline (parse → typecheck → lower → JIT) on a `.tri`
//! program whose block-form body returns an owned heap local that is ALSO
//! scope-pop `Drop`'d — the MIR shape `Drop(_s); Return(_s)`. This is the
//! RANH GIỚI SINH TỬ: if the escaping value were freed by its own scope-pop
//! Drop, this would double-free. ADR-0054 Return-leniency makes that Drop a
//! no-op, so the String is freed EXACTLY ONCE (by the caller's scope exit).
//!
//! Swaps `__triet_string_free` for a counting shim and asserts free == 1.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - Poison the ADR-0055 body-path merge (block-body discards its tail) →
//!     `f` returns unit, `main` no longer observes 5 → result assertion fails.
//!   - A double-free in the parity path → count becomes 2.
//!   - A leak (Drop wrongly elided as escape on BOTH sides) → count becomes 0.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{JitContext, ShimSymbol};

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0` guard so a tombstoned/zeroed slot's Drop frees nothing.
#[unsafe(no_mangle)]
extern "C" fn __cfg_parity_count_free(ptr: i64, cap: i64) {
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
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

#[test]
fn parity_return_heap_frees_exactly_once() {
    use triet_jit::mir_lower;

    // `f` block-body tail = an owned String local (no explicit `return`).
    // The owned local is both returned AND scope-pop Drop'd → the parity
    // hazard. `main` binds the escaped String and reads its length, then
    // drops it once at scope exit.
    let bodies = lower_source(
        "function f() -> String { let s = \"hello\"; s }\n\
         function main() -> Integer {\n\
         \x20   let r: String = f();\n\
         \x20   return len(r);\n\
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
        ShimSymbol::fn_2_0("__triet_string_free", __cfg_parity_count_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ];

    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("parity-return-heap must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };

    assert_eq!(
        result, 5,
        "block-body tail returns the String → len(\"hello\") = 5"
    );
    assert_eq!(
        FREE_COUNT.load(Ordering::SeqCst),
        1,
        "the returned heap value must be freed exactly once (no leak, no double-free)"
    );
}

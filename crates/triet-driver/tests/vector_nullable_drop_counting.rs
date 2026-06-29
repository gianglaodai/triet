//! ADR-0062 Heap-Nullable Lát 4.8 — route-lower free-count test for
//! `Vector<Integer>?` (single i64 handle, ptr-sentinel in the handle).
//!
//! Vector? has no 24-byte slot to offset-poison (unlike String?), so the §8
//! sentinel-vs-zero hazard is pinned by drop-counting instead: a null `Vector?`
//! (`~0`, handle == NULL_SENTINEL) frees zero times (the shim no-ops on the
//! sentinel), a non-null one frees exactly once. The present-arm move-out
//! (named scrutinee) must still free exactly once — the M1 tombstone zeroes the
//! moved-from handle so the scrutinee Drop is a no-op.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - null → free-count 0, non-null → 1 (poison null-routing → count wrong).
//!   - present-arm move → 1 (poison M1 var-zero else-branch → count 2).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

// ADR-0071 infra WO: serialize the in-binary parallel tests — they share
// the global free counter(s); cargo runs tests in this file concurrently,
// so without this lock the store(0)+call+load races. Reset happens UNDER
// the lock (each test holds it across the `run*` call).
static TEST_LOCK: Mutex<()> = Mutex::new(());

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_vector_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it counts only LIVE frees.
#[unsafe(no_mangle)]
extern "C" fn __vnull_count_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    FREE_COUNT.fetch_add(1, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Real shim set, but `__triet_vector_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __vnull_count_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_get", mir_lower::__triet_vector_get),
    ]
}

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
        .expect("Vector? program must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (result, FREE_COUNT.load(Ordering::SeqCst))
}

#[test]
fn nonnull_vector_nullable_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (result, frees) = run_counting(
        "function f() -> Vector<Integer>? = vector_new()\n\
         function main() -> Integer {\n\
         \x20   let r = f();\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(frees, 1, "non-null Vector? must be freed exactly once");
}

#[test]
fn null_vector_nullable_freed_zero() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (result, frees) = run_counting(
        "function f() -> Vector<Integer>? = ~0\n\
         function main() -> Integer {\n\
         \x20   let r = f();\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        frees, 0,
        "null Vector? (handle == NULL_SENTINEL) must free nothing"
    );
}

#[test]
fn present_arm_move_out_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Named scrutinee → MIR drops both the arm-local and the scrutinee. The M1
    // tombstone (var-zero else-branch) nulls the moved-from handle so only ONE
    // live free remains. Poison M1 var-zero → count 2 → RED.
    let (result, frees) = run_counting(
        "function f() -> Vector<Integer>? = vector_new()\n\
         function main() -> Integer {\n\
         \x20   let x = f();\n\
         \x20   let n = match x {\n\
         \x20       ~+ v => len(v),\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         \x20   return n;\n\
         }",
    );
    assert_eq!(result, 0, "non-null empty vector: len == 0");
    assert_eq!(
        frees, 1,
        "present-arm move-out must free exactly once (poison M1 var-zero → 2)"
    );
}

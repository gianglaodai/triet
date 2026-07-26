//! ADR-0089 Slice 2c (ForceUnwrap `!!`, Mirror-Elvis) — heap free-count teeth.
//!
//! `expr!!` on a present heap-nullable (`String?`) MUST move the payload out
//! exactly once — no leak (FREE==0), no double-free (FREE==2). The present
//! arm does `Assign { dest: result, source: obj_val }` (PA-3c identity); for
//! a NAMED local this is a move (borrowck tracks the source `Moved` —
//! confirmed independently by the `499_force_unwrap_use_after_move_canary.tri`
//! fixture), so the string is freed exactly once when `result` goes out of
//! scope. For an RVALUE temp (`f()!!`), the temp produced by the call is
//! force-unwrapped and consumed in the same expression — same single-free
//! expectation, different producer shape.
//!
//! Mirrors the `heap_nullable_return_present_counting.rs` infra: real
//! pipeline (parse → typecheck → lower → JIT), with `__triet_string_free`
//! swapped for a counter that mirrors the real free's
//! `ptr == 0 || ptr == NULL_SENTINEL` guard so only live allocations count.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

// Serialize — tests in this file share the global free counter.
static TEST_LOCK: Mutex<()> = Mutex::new(());

static STR_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __fuc_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Real shim set, with `__triet_string_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __fuc_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ]
}

/// Compile `source`, call `main`, return (`main`'s result, String free count).
fn run(source: &str) -> (i64, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    STR_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (result, STR_FREES.load(Ordering::SeqCst))
}

#[test]
fn named_local_force_unwrap_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // `x!!` moves the String payload out of the named local `x` (PA-3c
    // Assign identity — non-Copy source ⇒ move). The unwrapped `s` is the
    // sole owner and is freed exactly once at scope exit.
    let (result, str_frees) = run("function f() -> String? = \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let x = f();\n\
         \x20   let s = x!!;\n\
         \x20   return len(s);\n\
         }");
    assert_eq!(result, 2, "len(\"hi\") == 2");
    assert_eq!(
        str_frees, 1,
        "named-local `!!` unwrap must free the String exactly once \
         (leak-tooth: FREE==0 is a leak; double-free-tooth: FREE==2 means \
         both `x` and `s` freed the same allocation)"
    );
}

#[test]
fn rvalue_temp_force_unwrap_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // `f()!!` force-unwraps the call-result temp directly (no named local
    // binding it first) and consumes it in the same expression via `len`.
    let (result, str_frees) = run("function f() -> String? = \"hi\"\n\
         function main() -> Integer {\n\
         \x20   return len(f()!!);\n\
         }");
    assert_eq!(result, 2, "len(\"hi\") == 2");
    assert_eq!(
        str_frees, 1,
        "rvalue-temp `!!` unwrap must free the String exactly once"
    );
}

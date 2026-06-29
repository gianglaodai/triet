//! WO-0073 — ADR-0072 §6 heap-nullable-return red-flag: free-count test for the
//! EXPLICIT-present `~+ <heap>` form on a `T?` return.
//!
//! ADR-0072 expected-type propagation newly opened the `function f() -> T? = ~+ <heap>`
//! path (an explicit `~+` present arm wrapping a heap value: String / Vector /
//! HashMap). The bare-widening forms (`= "hi"`, `= vector_new()`, `= hashmap_new()`)
//! and the null form (`= ~0`) are already locked by the Lát-1 counting tests
//! (`{string,vector,hashmap}_nullable_drop_counting.rs`). This file closes the
//! remaining gap: the explicit `~+ <heap>` present arm. Probe O showed all four
//! shapes exit 0 (no double-free crash) — but exit-0 does NOT rule out a LEAK
//! (FREE==0), so we pin the count.
//!
//! Runs the REAL pipeline (parse → typecheck → lower → JIT) and swaps the three
//! heap free shims for counters that mirror the real free's
//! `ptr == 0 || ptr == NULL_SENTINEL` guard, so only live allocations count.
//!
//! TWO return shapes carry DIFFERENT hazards — the load-bearing guard differs,
//! so the teeth that bite differ (Mentor O verified each by blood):
//!
//!   - expr-body (`= ~+ x`): the lowerer escapes BY OMISSION — the callee emits
//!     NO free at all for the returned value (O verified: drop the shim's
//!     `ptr==0` guard AND turn M4 off → total free-shim calls == 1). Double-free
//!     is structurally impossible here, so the M4-tooth is INERT for these
//!     cells. The live guard is the leak-tooth: no-op the CALLER drop →
//!     FREE==0 → RED.
//!
//!   - named-local (`{ let s; return ~+ s; }`): `flush_all_for_return` DOES emit
//!     `Drop(s)`, and the M4 return-escape (mir_lower.rs:1977-1984) is what
//!     suppresses it so the value escapes. M4 is load-bearing → the
//!     double-free-tooth (gỡ M4) makes the callee ALSO free → FREE==2 → RED
//!     (O verified for all three heap types). The leak-tooth bites here too.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

// ADR-0071 infra WO: serialize the in-binary parallel tests — they share
// the global free counter(s); cargo runs tests in this file concurrently,
// so without this lock the store(0)+call+load races. Reset happens UNDER
// the lock (each test holds it across the `run` call).
static TEST_LOCK: Mutex<()> = Mutex::new(());

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static HMAP_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __hnrp_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __hnrp_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __hnrp_hmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    HMAP_FREES.fetch_add(1, Ordering::SeqCst);
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

/// Real shim set, but the three heap free shims swapped for counters.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __hnrp_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __hnrp_vec_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __hnrp_hmap_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
    ]
}

/// Compile `source`, call `main`, return (`main`'s result, str/vec/hmap frees).
fn run(source: &str) -> (i64, usize, usize, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    STR_FREES.store(0, Ordering::SeqCst);
    VEC_FREES.store(0, Ordering::SeqCst);
    HMAP_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (
        result,
        STR_FREES.load(Ordering::SeqCst),
        VEC_FREES.load(Ordering::SeqCst),
        HMAP_FREES.load(Ordering::SeqCst),
    )
}

#[test]
fn present_string_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell A: `f() -> String? = ~+ "hi"` (explicit present arm, EXPR-BODY).
    // The body IS the tail expression → the lowerer escapes BY OMISSION: it emits
    // NO callee Drop for the returned value. The caller's scope-drop frees it once
    // → FREE_str == 1. The JIT M4 return-escape is INERT here (no callee Drop to
    // skip); double-free is structurally impossible (see header).
    let (result, str_frees, _, _) = run("function f() -> String? = ~+ \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let r: String? = f();\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        str_frees, 1,
        "explicit ~+ String? (expr-body) freed exactly once \
         (leak-tooth: no-op caller drop-glue → 0 → RED; double-free-tooth (gỡ M4) \
          INERT — escape-by-omission, no callee Drop)"
    );
}

#[test]
fn present_vector_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell B: `f() -> Vector<Integer>? = ~+ vector_new()` (expr-body, scalar handle).
    // Same escape-by-omission as Cell A: no callee Drop; caller frees once.
    // M4-tooth INERT; live guard = leak-tooth.
    let (result, _, vec_frees, _) = run("function f() -> Vector<Integer>? = ~+ vector_new()\n\
         function main() -> Integer {\n\
         \x20   let r: Vector<Integer>? = f();\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        vec_frees, 1,
        "explicit ~+ Vector? (expr-body) freed exactly once \
         (leak-tooth → 0 → RED; double-free-tooth INERT — escape-by-omission)"
    );
}

#[test]
fn present_hashmap_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell C: `f() -> HashMap<Integer, Integer>? = ~+ hashmap_new()` (expr-body,
    // scalar handle). Same escape-by-omission as Cell A; M4-tooth INERT; live
    // guard = leak-tooth.
    let (result, _, _, hmap_frees) = run(
        "function f() -> HashMap<Integer, Integer>? = ~+ hashmap_new()\n\
         function main() -> Integer {\n\
         \x20   let r: HashMap<Integer, Integer>? = f();\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        hmap_frees, 1,
        "explicit ~+ HashMap? (expr-body) freed exactly once \
         (leak-tooth → 0 → RED; double-free-tooth INERT — escape-by-omission)"
    );
}

#[test]
fn present_string_return_match_consumed_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell D: `f() -> String? = ~+ "hi"` (expr-body), consumed by a match present
    // arm `~+ s => len(s)`. Callee escapes by omission (no callee Drop, M4-tooth
    // INERT). The matched String is moved into `s`, len()'d, then freed once by
    // the arm → result == 2 AND FREE_str == 1.
    let (result, str_frees, _, _) = run("function f() -> String? = ~+ \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let n = match f() {\n\
         \x20       ~+ s => len(s),\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         \x20   return n;\n\
         }");
    assert_eq!(result, 2, "present arm: len(\"hi\") == 2");
    assert_eq!(
        str_frees, 1,
        "match-consumed ~+ String? (expr-body) freed exactly once \
         (leak-tooth → 0 → RED; double-free-tooth INERT — escape-by-omission)"
    );
}

// ── Named-local explicit-return cells (E/F/G) ───────────────────────────────
// `{ let s: T = …; return ~+ s; }` — here `flush_all_for_return` emits a
// `Drop(s)` AND `s` is in the `Return` terminator's values, so the M4
// return-escape (mir_lower.rs:1977-1984) is LOAD-BEARING: it suppresses that
// callee Drop so the heap value escapes via sret to the caller, freed once.
// The double-free-tooth (gỡ M4) makes the callee also free → FREE==2 → RED.

#[test]
fn named_local_string_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell E: M4 load-bearing — double-free-tooth (gỡ M4) → FREE==2 → RED.
    let (result, str_frees, _, _) = run("function f() -> String? {\n\
         \x20   let s: String = \"hi\";\n\
         \x20   return ~+ s;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let r: String? = f();\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        str_frees, 1,
        "named-local ~+ String? return freed exactly once \
         (M4 load-bearing: gỡ M4 → callee also frees → 2)"
    );
}

#[test]
fn named_local_vector_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell F: M4 load-bearing — double-free-tooth (gỡ M4) → FREE==2 → RED.
    let (result, _, vec_frees, _) = run("function f() -> Vector<Integer>? {\n\
         \x20   let v: Vector<Integer> = vector_new();\n\
         \x20   return ~+ v;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let r: Vector<Integer>? = f();\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        vec_frees, 1,
        "named-local ~+ Vector? return freed exactly once (M4 load-bearing)"
    );
}

#[test]
fn named_local_hashmap_return_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Cell G: M4 load-bearing — double-free-tooth (gỡ M4) → FREE==2 → RED.
    let (result, _, _, hmap_frees) = run("function f() -> HashMap<Integer, Integer>? {\n\
         \x20   let h: HashMap<Integer, Integer> = hashmap_new();\n\
         \x20   return ~+ h;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let r: HashMap<Integer, Integer>? = f();\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        hmap_frees, 1,
        "named-local ~+ HashMap? return freed exactly once (M4 load-bearing)"
    );
}

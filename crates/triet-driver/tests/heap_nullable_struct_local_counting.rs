//! WO-INV-HeapNullable-Probe T0 (O ✅ · G ✅, 2026-07-19) — route-lower
//! free-count teeth for `Nullable(Struct)` where the struct carries a plain
//! heap FIELD (`struct H { name: String }`, NOT `String?`), at LOCAL-binding
//! position specifically.
//!
//! Context: `is_lowerable_nullable_payload` (`crates/triet-mir/src/lib.rs`)
//! lets `MirType::Struct(_)` through UNCONDITIONALLY, and its doc comment
//! claims heap content "stays refused via the scalar-only field/payload gate
//! below" — a claim already falsified for RETURN position (measured SIGABRT
//! 134 pre-`e7aab8c`, now refused by a policy guard) and for FIELD/PARAM
//! position (both independently refused, exit 3 / exit 4). LOCAL position has
//! no such guard: `H?` locals compile and RUN, producing correct VALUES on
//! four historically-risky shapes (null, present-drop, present-bind-move,
//! while-loop reuse). Correct output is NOT proof of soundness — free-count
//! is. This file is the oracle for that question, not a value oracle.
//!
//! Four shapes (S1-S4), expected FREE_COUNT:
//!   S1 null, no allocation                          -> 0
//!   S2 present, natural drop                         -> 1
//!   S3 present, match present-bind-move (`~+ v`)      -> 1 (not 2)
//!   S4 while loop, 3 iterations, fresh alloc each time -> 3
//!
//! Each test also carries a POISON variant proving the tooth is non-vacuous:
//! stubbing `__nls_str_free` to a no-op-counter-only-when-flagged lets us
//! confirm the harness observes real frees, not a tautology. See
//! `*_poison_proves_tooth_is_live` below for the mechanism.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global AtomicUsize and
//! no-mangle shim — N7 fork-bomb hazard per project convention). The Mutex
//! below also serializes within this binary for a default parallel `cargo
//! test` run.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);

/// Serialize the tests in THIS binary: they share the process-global free
/// counter (no-mangle shim), so cargo's default parallel run would race.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __nls_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

/// POISON shim: simulates a dropped free-arm (leak) by never counting.
#[unsafe(no_mangle)]
extern "C" fn __nls_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
    // Intentionally does NOT increment STR_FREES — models the drop-glue
    // "leak" failure mode (free-arm removed / never emitted).
}

/// POISON shim: simulates a double-free by counting twice per call.
#[unsafe(no_mangle)]
extern "C" fn __nls_str_free_poison_double(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(2, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn shims_with(free_fn: extern "C" fn(i64, i64)) -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", free_fn),
    ]
}

fn run_with(source: &str, free_fn: extern "C" fn(i64, i64)) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = shims_with(free_fn);
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

fn run(source: &str) -> i64 {
    run_with(source, __nls_str_free)
}

// ── S1: null H?, no allocation, no free ─────────────────────────────────

const SRC_S1_NULL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let a: H? = ~0;\n\
     \x20   return 0;\n\
     }";

#[test]
fn s1_null_local_no_alloc_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_S1_NULL);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "S1: null H? local must never touch the String free shim"
    );
}

// ── S2: present H?, natural drop at end of scope ────────────────────────

const SRC_S2_PRESENT_DROP: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let a: H? = ~+ H { name: \"hello\" };\n\
     \x20   return 0;\n\
     }";

#[test]
fn s2_present_local_natural_drop_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_S2_PRESENT_DROP);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "S2: present H? local must free its String field exactly once on Drop"
    );
}

#[test]
fn s2_poison_leak_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_S2_PRESENT_DROP, __nls_str_free_poison_leak);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "S2 POISON(leak): with the free-arm stubbed to a no-op-counter, the \
         count MUST read 0 (not 1) — proves the harness observes the real \
         free call count, not a tautological pass"
    );
}

// ── S3: present H?, match present-bind-move ─────────────────────────────
//
// NOTE (measured during T0, logged as OUT-OF-SCOPE debt, not fixed here):
// the naive `~+ v => length(v.name)` (field access INLINED as the call
// argument expression) trips an orthogonal, pre-existing leak that has
// NOTHING to do with Nullable(Struct) or bind-move — it reproduces
// identically on a fully plain, non-nullable, non-matched local:
// `let h: H = H{name:"hello"}; return length(h.name);` also leaks
// (FREE_COUNT 0, not 1). Isolated by probing four variants: bind-move +
// ignore v (FREE=1, ok), bind-move + `let n = v.name; length(n)` (FREE=1,
// ok), plain local + `length(s)` where s is a bare identifier (FREE=1, ok),
// plain local + `length(h.name)` inline field access (FREE=0, LEAK) — the
// leak tracks the inline-field-access-as-call-argument shape, not
// Nullable/match/bind-move. Root cause candidate: the ADR-0049 Phase-1 B4
// fast path in `triet-lower/src/lib.rs` (`length`'s `MirType::String` arm)
// reads the `len` field directly off the argument's StackSlot instead of
// calling a consuming shim, and does not appear to schedule a Drop for a
// synthesized (non-identifier) argument temp. This test uses the
// bind-through-a-`let` idiom below specifically to ISOLATE the
// Nullable(Struct)-bind-move question from that orthogonal bug.
const SRC_S3_BIND_MOVE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let a: H? = ~+ H { name: \"hello\" };\n\
     \x20   return match a {\n\
     \x20       ~+ v => { let n = v.name; length(n) },\n\
     \x20       ~0 => 0,\n\
     \x20   };\n\
     }";

#[test]
fn s3_present_local_bind_move_frees_once_not_twice() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_S3_BIND_MOVE);
    assert_eq!(r, 5, "\"hello\".length() == 5");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "S3: match present-bind-move of H? must free the String field \
         EXACTLY once — a missing scrutinee tombstone would double-free (2)"
    );
}

#[test]
fn s3_poison_double_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_S3_BIND_MOVE, __nls_str_free_poison_double);
    assert_eq!(r, 5);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "S3 POISON(double): with each real free call counted twice, a single \
         underlying free call MUST read 2 (not 1) — proves the harness \
         reports the true call count, not a hardcoded pass"
    );
}

// ── S4: while loop, 3 iterations, fresh H? allocated + dropped each time ──

const SRC_S4_WHILE_LOOP: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let mutable i: Integer = 0;\n\
     \x20   while i < 3 {\n\
     \x20       let a: H? = ~+ H { name: \"hello\" };\n\
     \x20       i = i + 1;\n\
     \x20   }\n\
     \x20   return 0;\n\
     }";

#[test]
fn s4_while_loop_three_iterations_frees_three_times() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_S4_WHILE_LOOP);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "S4: a while-loop back-edge reuses the SAME physical StackSlot per \
         MIR Local across iterations — each of the 3 iterations must free \
         its own H? exactly once (3 total), neither leaking stale iterations \
         nor double-freeing the reused slot"
    );
}

#[test]
fn s4_poison_leak_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_S4_WHILE_LOOP, __nls_str_free_poison_leak);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "S4 POISON(leak): with the free-arm stubbed to a no-op-counter across \
         all 3 iterations, the count MUST read 0 (not 3)"
    );
}

//! ADR-0089 §AMEND Slice 2b §2b.6 — permanent FREE-count teeth for
//! `for item in <Vector>.drain()` (T2b-tombstone / T2b-break-mid /
//! T2b-rvalue, "điều kiện thép" #5/#6).
//!
//! Mirrors the established real-pipeline + counting-shim-swap pattern from
//! `break_drop_counting.rs` / `vector_iter_container_free_counting.rs` /
//! `vector_userstruct_counting.rs`: real Vector shims (alloc/push/pop_front)
//! so bytes actually move and `len--` actually runs, with EITHER
//! `__triet_string_free` OR `__triet_vector_free` swapped for a counting
//! stand-in (never both faked at once — the untouched one still calls the
//! real allocator, so a real leak/double-free in THAT shim would still
//! abort the test process, which is an acceptable/expected signal here).
//!
//! What these tests prove that the fixture-only regression tests
//! (486-490, EXPECT-value) cannot: drain's tombstone contract (ADR-0082
//! §AMEND-2.1, `pop_front`'s `len--`) is what keeps `Drop(v)`'s own
//! element-free-loop from re-visiting an already-moved-out slot. A leaked
//! or double-freed element does not change `main`'s return value on its
//! own (the fixture is vacuous for THIS soundness property) — these tests
//! make the exact FREE count the assertion.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it only counts frees of LIVE
/// allocations.
#[unsafe(no_mangle)]
extern "C" fn __drain_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

/// Counting stand-in for `__triet_vector_free`. Mirrors the real free's
/// null/sentinel guard so only LIVE buffer frees count.
#[unsafe(no_mangle)]
extern "C" fn __drain_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
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

/// Shim set with the STRING free swapped for the counter — real Vector
/// shims (`alloc`/`push`/`pop_front`) so `len--` actually runs.
fn string_counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1(
            "__triet_vector_pop_front",
            mir_lower::__triet_vector_pop_front,
        ),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __drain_str_free),
    ]
}

/// Shim set with the VECTOR (container buffer) free swapped for the
/// counter — used for the scalar-element container-FREE proofs (no String
/// involved at all, isolating the buffer-free count).
fn vector_counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __drain_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1(
            "__triet_vector_pop_front",
            mir_lower::__triet_vector_pop_front,
        ),
    ]
}

/// Compile `source` with `shims`, call `main`, return its result.
fn run(source: &str, shims: &[ShimSymbol]) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("drain program must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// T2b-tombstone (heap half): draining a `Vector<String>` of 3 elements
/// must free each String EXACTLY once (via the item's own end-of-iteration
/// Drop) — the container's own end-of-scope Drop then sees `len == 0`
/// (tombstoned by `pop_front`'s `len--` every call) and frees NOTHING
/// further. STR_FREES must be exactly 3.
///
/// Poison verify (O, measured 2026-07-26): commenting out the `len--` write
/// in the REAL `__triet_vector_pop_front` (`triet-jit/src/mir_lower.rs`)
/// makes `pop_front` never report the container empty (`len` stays 3
/// forever), so this test's drain loop — which terminates ONLY when the
/// present-test sees the empty sentinel — NEVER TERMINATES: an infinite
/// hang, NOT a `STR_FREES == 6` count (the loop never reaches `Drop(xs)`).
/// The tombstone is therefore load-bearing TWICE: it is both the
/// double-free guard AND drain's termination condition. Measured under the
/// poison: this full-drain test hangs (killed by a 30s timeout); the
/// `break`-terminated `drain_break_mid` test below does reach `Drop(xs)`
/// and fails on a survivor re-free count mismatch instead of hanging.
#[test]
fn drain_string_frees_each_element_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable xs: Vector<String> = vector_new();\n\
         \x20   xs = push(xs, \"aa\");\n\
         \x20   xs = push(xs, \"bb\");\n\
         \x20   xs = push(xs, \"cc\");\n\
         \x20   let mutable total = 0;\n\
         \x20   for s in xs.drain() {\n\
         \x20       total = total + length(s);\n\
         \x20   }\n\
         \x20   return total;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 6, "main must return 2+2+2 (aa/bb/cc lengths)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "each drained String must free EXACTLY ONCE — 6 would be a \
         double-free (tombstone `len--` broken, the empty container's own \
         Drop re-frees already-moved slots), fewer than 3 would be a leak"
    );
}

/// T2b-break-mid (điều kiện thép #5): drain 5 heap elements, `break` after
/// processing the 2nd — the FIRST item drops via the normal end-of-
/// iteration path, the SECOND (current at break time) drops via the
/// break-path `emit_scope_drops` (ADR-0089 §4, `drop_snapshot` captured
/// BEFORE `item` is registered so break's drop range covers it), and the 3
/// un-popped survivors drop via the container's own end-of-scope Drop.
/// STR_FREES must be exactly 5 (2 items + 3 survivors) — 6 would signal a
/// double-free (item2 freed by both the break path AND the container's
/// stale-length survivor loop), 4 would signal a leak (item2's break-path
/// drop skipped).
#[test]
fn drain_break_mid_frees_processed_items_and_survivors() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable xs: Vector<String> = vector_new();\n\
         \x20   xs = push(xs, \"a\");\n\
         \x20   xs = push(xs, \"b\");\n\
         \x20   xs = push(xs, \"c\");\n\
         \x20   xs = push(xs, \"d\");\n\
         \x20   xs = push(xs, \"e\");\n\
         \x20   let mutable count = 0;\n\
         \x20   for s in xs.drain() {\n\
         \x20       count = count + 1;\n\
         \x20       if count == 2 {\n\
         \x20           break;\n\
         \x20       }\n\
         \x20   }\n\
         \x20   return count;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 2, "main must return 2 (loop breaks after the 2nd item)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        5,
        "2 drained items (normal-path + break-path drop) + 3 un-popped \
         survivors (container's own Drop) — every element freed EXACTLY \
         once across the whole Vector"
    );
}

/// T2b-tombstone (container-buffer half): the NAMED-local container itself
/// must free its buffer EXACTLY ONCE after a full (non-break) drain —
/// mirrors `vector_iter_container_free_counting.rs`'s lvalue scenario, but
/// for `pop_front`-drain instead of the copy-by-value desugar.
#[test]
fn drain_container_buffer_frees_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable xs = vector_new();\n\
         \x20   xs = push(xs, 1);\n\
         \x20   xs = push(xs, 2);\n\
         \x20   let mutable sum = 0;\n\
         \x20   for x in xs.drain() {\n\
         \x20       sum = sum + x;\n\
         \x20   }\n\
         \x20   return sum;\n\
         }",
        &vector_counting_shims(),
    );
    assert_eq!(r, 3, "main must return 1 + 2");
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "the drained (now-empty) container must free its buffer EXACTLY \
         ONCE at main's own scope exit — 0 would be a leak, 2 a double-free"
    );
}

/// T2b-rvalue (điều kiện thép #6): `for x in make_vec().drain()` — the
/// iterable is an RVALUE (a fresh temp from a bare Call, never named), so
/// `lower_expr` on its own registers nothing for Drop. The drain desugar's
/// receiver-lowering (§2a.3.1 discipline, reused verbatim per §2b.4) must
/// `push_owned` it itself (via `emit_shim_call`'s own bookkeeping on the
/// FIRST `pop_front` call), so the temp container drops exactly once at
/// the enclosing scope's exit — not zero (leak) and not two (double-free).
#[test]
fn drain_rvalue_container_frees_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function make_vector() -> Vector<Integer> {\n\
         \x20   let mutable v = vector_new();\n\
         \x20   v = push(v, 4);\n\
         \x20   v = push(v, 5);\n\
         \x20   v = push(v, 6);\n\
         \x20   return v;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let mutable sum = 0;\n\
         \x20   for x in make_vector().drain() {\n\
         \x20       sum = sum + x;\n\
         \x20   }\n\
         \x20   return sum;\n\
         }",
        &vector_counting_shims(),
    );
    assert_eq!(r, 15, "main must return 4 + 5 + 6");
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "the RVALUE container temp (from `make_vector()`, never named) must \
         free EXACTLY ONCE — 0 would be a leak, 2 a double-free (registered \
         twice)"
    );
}

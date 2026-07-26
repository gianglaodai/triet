//! ADR-0089 §AMEND Slice 2d §2d.3 — permanent FREE-count teeth for
//! `for item in <&0 mutable Vector>.drain()` (T2d-tombstone / T2d-break-mid,
//! "điều kiện thép" G #3/#B).
//!
//! Mirrors `drain_iter_counting.rs` byte-for-byte in structure/discipline
//! (real Vector shims — alloc/push/pop_front — so `len--` actually runs,
//! with ONLY `__triet_string_free` swapped for a counting stand-in; the
//! untouched `__triet_vector_free` still calls the real allocator, so a
//! real leak/double-free on the CONTAINER side would still abort the test
//! process). The difference from Slice 2b's `drain_iter_counting.rs`:
//! `drain_it` here takes `v: &0 mutable Vector<String>` (a BORROW
//! receiver, Slice 2d) instead of owning the Vector by value — `main`
//! keeps `xs` and drops it itself at its own scope exit.
//!
//! What these tests prove that the fixture-only regressions (506/509,
//! EXPECT-value) cannot: the container-survives contract (§2d.2 —
//! `is_reference()==true` on the receiver skips `push_owned`/`Drop` inside
//! `drain_it`, so the buffer is NEVER touched by that function's own
//! scope exit) does not, by itself, guarantee the ELEMENT frees are
//! correct — a leaked or double-freed `String` element does not change
//! `main`'s return value (the fixtures are vacuous for this soundness
//! property). These tests make the exact FREE count the assertion.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static STR_FREES: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it only counts frees of LIVE
/// allocations.
#[unsafe(no_mangle)]
extern "C" fn __drain_borrow_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
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
/// shims (`alloc`/`push`/`pop_front`) so `len--` actually runs, and the
/// REAL `__triet_vector_free` so the container-buffer free path (run by
/// `main`'s own `Drop(xs)`, never by `drain_it`'s borrow-receiver) is
/// exercised for real — a corrupted/double-freed buffer would abort the
/// test process.
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
        ShimSymbol::fn_2_0("__triet_string_free", __drain_borrow_str_free),
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
        .expect("drain-via-borrow program must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// T2d-tombstone (heap half): draining a `Vector<String>` of 3 elements
/// through a `&0 mutable` BORROW receiver (`drain_it`, a separate
/// function — NOT `main` itself) must free each String EXACTLY once (via
/// the item's own end-of-iteration Drop inside `drain_it`). `drain_it`
/// itself never touches the container buffer (`is_reference()==true`
/// skips `push_owned`/`Drop` on the receiver, §2d.2) — the buffer is only
/// freed later, by `main`'s OWN `Drop(xs)` at its scope exit, and by then
/// `len == 0` (tombstoned), so that free frees NO further elements.
/// STR_FREES must be exactly 3.
#[test]
fn drain_borrow_receiver_frees_each_element_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function drain_it(v: &0 mutable Vector<String>) -> Integer {\n\
         \x20   let mutable total = 0;\n\
         \x20   for item in v.drain() {\n\
         \x20       total = total + length(item);\n\
         \x20   }\n\
         \x20   return total;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let mutable xs: Vector<String> = vector_new();\n\
         \x20   xs = push(xs, \"aa\");\n\
         \x20   xs = push(xs, \"bb\");\n\
         \x20   xs = push(xs, \"cc\");\n\
         \x20   let total = drain_it(&0 mutable xs);\n\
         \x20   return total;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 6, "main must return 2+2+2 (aa/bb/cc lengths)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "each drained String must free EXACTLY ONCE via drain_it's own \
         loop-body drop — 6 would be a double-free (the container's Drop, \
         run later by `main`, re-freeing already-moved slots because the \
         tombstone `len--` was broken), fewer than 3 would be a leak"
    );
}

/// T2d-break-mid (§2d.3, "điều kiện thép" #B): drain 5 heap elements
/// through a `&0 mutable` borrow receiver, `break` after processing the
/// 2nd. `drain_it` returns having popped 2 of 5 — the buffer's `len == 3`
/// (tombstoned by `pop_front`'s `len--` on each of the 2 pops). `main`
/// (the OWNER of `xs`) then drops `xs` at ITS OWN scope exit —
/// `emit_vector_element_free_loop` reads the CURRENT `len` (3) and frees
/// ONLY the 3 survivors ("c"/"d"/"e"), never touching the 2 already-
/// popped-and-consumed items ("a"/"b", freed by drain_it's own
/// normal-path + break-path drops). STR_FREES must be exactly 5 (2 items +
/// 3 survivors) — 6 would signal a double-free (a survivor re-freed, or an
/// already-popped item re-freed because `len` wasn't tombstoned), 4 would
/// signal a leak (one drop skipped).
#[test]
fn drain_borrow_receiver_break_mid_frees_processed_items_and_survivors() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(
        "function drain_it(v: &0 mutable Vector<String>) -> Integer {\n\
         \x20   let mutable count = 0;\n\
         \x20   for item in v.drain() {\n\
         \x20       count = count + 1;\n\
         \x20       if count == 2 {\n\
         \x20           break;\n\
         \x20       }\n\
         \x20   }\n\
         \x20   return count;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let mutable xs: Vector<String> = vector_new();\n\
         \x20   xs = push(xs, \"a\");\n\
         \x20   xs = push(xs, \"b\");\n\
         \x20   xs = push(xs, \"c\");\n\
         \x20   xs = push(xs, \"d\");\n\
         \x20   xs = push(xs, \"e\");\n\
         \x20   let count = drain_it(&0 mutable xs);\n\
         \x20   return count;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 2, "drain_it must return 2 (breaks after the 2nd item)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        5,
        "2 drained items (drain_it's own normal-path + break-path drop) + \
         3 survivors (main's own Drop(xs), since the borrow receiver never \
         touches the buffer) — every element freed EXACTLY once across the \
         whole Vector, 0 leak, 0 double-free"
    );
}

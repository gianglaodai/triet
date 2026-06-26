//! ADR-0062 Heap-Nullable Lát 4 — route-lower free-count test for the
//! present-arm move-out of a `String?` match.
//!
//! The `match x { ~+ s => .., ~0 => .. }` lowering binds the payload via
//! `s = move scrutinee` (whole 24-byte slot copy). With a **named** scrutinee
//! (`let x = f(); match x`) the MIR drops BOTH the arm-local `s` AND the
//! scrutinee `x` in the merge block — two Drops of what would be the same
//! pointer. Memory safety here rests entirely on the M1 zeroing-on-move
//! tombstone (`triet-jit/src/mir_lower.rs:1322`, `stack_store(0, slot, 0)`):
//! it nulls the scrutinee's ptr@0 after the move so the scrutinee's Drop is a
//! free-shim no-op, leaving exactly ONE live free.
//!
//! This tooth locks that mechanism with an explicit free-count, NOT an
//! incidental double-free crash (a forgiving allocator may not abort → the
//! double-free would be a silent leak/UB). Mirror of the Lát 1
//! `string_nullable_drop_counting` test.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - Non-null present-arm move → free-count == 1 (M1 tombstone load-bearing).
//!   - Poison M1 (slot@0 → slot@8) → scrutinee not tombstoned → both Drops free
//!     the same live pointer → count == 2 → RED.
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

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it counts only frees of LIVE
/// allocations — a tombstoned (ptr@0 == 0) scrutinee frees nothing.
#[unsafe(no_mangle)]
extern "C" fn __smatch_count_free(ptr: i64, cap: i64) {
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

/// Real shim set, but `__triet_string_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __smatch_count_free),
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
fn present_arm_move_out_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Named scrutinee `let x = f()` → MIR drops both the arm-local `s`
    // (`~+ s => len(s)`) AND the scrutinee `x` in the merge block. The M1
    // tombstone nulls the scrutinee's ptr@0 after `s = move x`, so the
    // scrutinee Drop is a no-op → exactly ONE live free.
    //
    // Poison M1 (mir_lower.rs:1322, slot@0 → slot@8): the scrutinee keeps its
    // live ptr@0 → both Drops free it → this count becomes 2 → RED.
    let (result, frees) = run_counting(
        "function f() -> String? = \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let x = f();\n\
         \x20   let r = match x {\n\
         \x20       ~+ s => len(s),\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         \x20   return r;\n\
         }",
    );
    assert_eq!(result, 2, "non-null present arm: len(\"hi\") == 2");
    assert_eq!(
        frees, 1,
        "present-arm move-out must free exactly once — the M1 tombstone makes \
         the scrutinee Drop a no-op (poison slot@0→slot@8 → count 2)"
    );
}

#[test]
fn method_return_present_arm_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // ADR-0062 Lát 4.5: a `String?` arriving via a trait METHOD sret return
    // (`b.get() -> String?`), then matched. Same M1-tombstone safety as the
    // free-function case — the sret return path must not introduce an extra
    // free or skip the tombstone. Non-null present arm → free-count == 1.
    //
    // Poison M1 (mir_lower.rs:1322, slot@0 → slot@8) → scrutinee not
    // tombstoned → both Drops free the same live pointer → count == 2 → RED.
    let (result, frees) = run_counting(
        "trait Box { function get(self) -> String? }\n\
         implement Box for Integer {\n\
         \x20   function get(self) -> String? = \"hi\"\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let b = 5;\n\
         \x20   let r = b.get();\n\
         \x20   let n = match r {\n\
         \x20       ~+ s => len(s),\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         \x20   return n;\n\
         }",
    );
    assert_eq!(
        result, 2,
        "method-return non-null present arm: len(\"hi\") == 2"
    );
    assert_eq!(
        frees, 1,
        "method-return present-arm move-out must free exactly once \
         (poison M1 slot@0→slot@8 → count 2)"
    );
}

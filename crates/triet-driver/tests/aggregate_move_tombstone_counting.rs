//! WO-Aggregate-Move-Tombstone (O+G, 2026-07-27) — route-lower pointer-dedup
//! free/leak teeth for the aggregate-move double-free fixed at two sites in
//! `crates/triet-lower/src/lib.rs`:
//!
//!   - Site A (`Stmt::Let`'s `is_struct_widening` branch, ~2240-2266): now
//!     emits `Deinit(v)` before `return Ok(())` for non-Copy `v`.
//!   - Site B (`Stmt::Assignment`, both the `Expr::Identifier` arm and the
//!     field/projection `_` arm, ~2372-2420): now emits `Deinit(v)` after the
//!     `Assign` for non-Copy `v` (guarded `v != orig` on the identifier arm
//!     to not zero a self-assignment's only live copy).
//!
//! Bug (measured on `04cb5d3`, unpatched): `struct Leaf { s: String }`,
//! `let a: Leaf? = p;` (widening) or `a = p;` (reassign) moved a heap-bearing
//! struct WITHOUT tombstoning the source — both `p` and the destination
//! ended up in `owned_locals`, so both got a scope-end `Drop`, freeing the
//! SAME String allocation twice: `free(): double free detected in tcache 2`
//! (exit 134).
//!
//! ⚠ A raw free-COUNT (like `struct_assign_move_counting.rs`'s
//! `AtomicUsize`) is NOT enough here — freeing the SAME pointer twice and
//! freeing TWO DISTINCT legitimate allocations both produce count==2. This
//! harness records the actual POINTER VALUE on every alloc/free call and
//! asserts on the DEDUPED SET: `distinct_freed == N` AND `duplicate_frees ==
//! 0` (double-free signal), separately from `distinct_allocated -
//! distinct_freed` (leak signal — a pointer that was allocated but never
//! freed).
//!
//! ⚠ RAM: run with `--exact --test-threads=1` (process-global shim state +
//! no-mangle symbols — the N7 fork-bomb hazard per project convention). The
//! Mutex below also serializes within this binary for a default parallel
//! `cargo test` run.
#![allow(unsafe_code)]

use std::sync::Mutex;

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

/// Every pointer returned by a real allocation (`__triet_string_from_bytes`
/// wraps through to the real allocator — this is NOT a stub).
static ALLOCATED: Mutex<Vec<i64>> = Mutex::new(Vec::new());
/// Every pointer passed to `__triet_string_free` (also wraps through to the
/// real deallocator, so a genuine double-free would abort the process before
/// this harness could report — see the poison tests below for how liveness
/// is proven WITHOUT letting a real double-free crash the test binary).
static FREED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

/// Serialize the tests in THIS binary: shared process-global state.
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn reset() {
    ALLOCATED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    FREED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[unsafe(no_mangle)]
extern "C" fn __amt_str_from_bytes(src: i64, len: i64) -> i64 {
    let ptr = mir_lower::__triet_string_from_bytes(src, len);
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        ALLOCATED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ptr);
    }
    ptr
}

/// Real free — records the pointer, THEN actually deallocates. A genuine
/// double-free (same pointer twice) will show up as a duplicate in `FREED`
/// on the FIRST occurrence's `dedup` check; the second call still reaches
/// the real glibc `free`, so an unpatched double-free still aborts the
/// process (as it did pre-fix) — this shim does not hide that, it only adds
/// bookkeeping on top.
#[unsafe(no_mangle)]
extern "C" fn __amt_str_free(ptr: i64, cap: i64) {
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        FREED.lock().unwrap_or_else(|e| e.into_inner()).push(ptr);
    }
    mir_lower::__triet_string_free(ptr, cap);
}

/// POISON shim: never frees (models a dropped free-arm / leak).
#[unsafe(no_mangle)]
extern "C" fn __amt_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
    // Intentionally does not record and does not call the real free.
}

/// POISON shim: records + frees the SAME pointer TWICE per call, simulating
/// a double-free WITHOUT touching the real allocator twice (so the test
/// binary doesn't abort) — proves the dedup harness would catch a real one.
#[unsafe(no_mangle)]
extern "C" fn __amt_str_free_poison_double(ptr: i64, cap: i64) {
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        let mut freed = FREED.lock().unwrap_or_else(|e| e.into_inner());
        freed.push(ptr);
        freed.push(ptr);
    }
    let _ = cap; // does NOT call the real free — avoid an actual double-free abort
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
        ShimSymbol::fn_2_1("__triet_string_from_bytes", __amt_str_from_bytes),
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
    run_with(source, __amt_str_free)
}

/// Returns (distinct_allocated, distinct_freed, duplicate_free_count).
fn dedup_stats() -> (usize, usize, usize) {
    let allocated = ALLOCATED.lock().unwrap_or_else(|e| e.into_inner());
    let freed = FREED.lock().unwrap_or_else(|e| e.into_inner());
    let distinct_allocated: std::collections::HashSet<i64> = allocated.iter().copied().collect();
    let distinct_freed: std::collections::HashSet<i64> = freed.iter().copied().collect();
    let dup_count = freed.len() - distinct_freed.len();
    (distinct_allocated.len(), distinct_freed.len(), dup_count)
}

// ── #1: widening `let a: Leaf? = p;` — source tombstoned, no leak ──────────

const SRC_1_WIDEN_LET: &str = "struct Leaf { s: String }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   let a: Leaf? = p;\n\
     \x20   return match a {\n\
     \x20       ~+ v => length(v.s),\n\
     \x20       ~0 => 0,\n\
     \x20   };\n\
     }";

#[test]
fn widen_let_frees_source_once_no_leak() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_1_WIDEN_LET);
    assert_eq!(
        r, 2,
        "\"hi\".length() == 2 — value must survive the widening move"
    );
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1, "exactly one String allocated (\"hi\")");
    assert_eq!(freed, 1, "exactly one distinct pointer freed");
    assert_eq!(dup, 0, "no pointer freed twice (no double-free)");
}

#[test]
fn widen_let_poison_leak_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run_with(SRC_1_WIDEN_LET, __amt_str_free_poison_leak);
    assert_eq!(r, 2);
    let (allocated, freed, _dup) = dedup_stats();
    assert_eq!(allocated, 1);
    assert_eq!(
        freed, 0,
        "POISON(leak): stubbed free-arm must observe 0 frees, not 1 — proves \
         the harness counts real free calls, not a tautology"
    );
}

#[test]
fn widen_let_poison_double_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run_with(SRC_1_WIDEN_LET, __amt_str_free_poison_double);
    assert_eq!(r, 2);
    let (_allocated, freed_distinct, dup) = dedup_stats();
    assert_eq!(
        freed_distinct, 1,
        "POISON(double): the single real free call maps to ONE distinct pointer"
    );
    assert_eq!(
        dup, 1,
        "POISON(double): that one pointer was recorded twice — proves dedup \
         stats detect a real double-free, not just count it as \"2 frees\""
    );
}

// ── #6: `a = p;` where `a: Leaf?` — source tombstoned, no leak (old dest
//    was `~0`, no prior allocation to lose) ─────────────────────────────────

const SRC_6_REASSIGN_NULLABLE: &str = "struct Leaf { s: String }\n\
     function main() -> Integer = {\n\
     \x20   let mutable a: Leaf? = ~0;\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   a = p;\n\
     \x20   return match a {\n\
     \x20       ~+ v => length(v.s),\n\
     \x20       ~0 => 0,\n\
     \x20   };\n\
     }";

#[test]
fn reassign_nullable_frees_source_once_no_leak() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_6_REASSIGN_NULLABLE);
    assert_eq!(r, 2);
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(
        allocated, 1,
        "only \"hi\" allocated — the old `~0` dest had no allocation"
    );
    assert_eq!(freed, 1);
    assert_eq!(dup, 0, "no double-free");
}

// ── #7: `a = p;` where `a: Leaf` (NOT nullable) — CÂU 1: source tombstoned
//    (no double-free) BUT the OLD dest value ("aaa") is never freed (LEAK).
//    This is the Câu 1(b) finding — reported to O, NOT fixed here (G ruling:
//    drop-old-dest-before-overwrite is a semantics change, out of this WO's
//    scope). The assertions below encode the MEASURED behavior, not a claim
//    that it is correct. ─────────────────────────────────────────────────

const SRC_7_REASSIGN_PLAIN: &str = "struct Leaf { s: String }\n\
     function main() -> Integer = {\n\
     \x20   let mutable a = Leaf { s: \"aaa\" };\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   a = p;\n\
     \x20   return length(a.s);\n\
     }";

#[test]
fn reassign_plain_no_double_free_but_leaks_old_dest() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_7_REASSIGN_PLAIN);
    assert_eq!(
        r, 2,
        "\"hi\".length() == 2 — a.s reads the NEW value after reassign"
    );
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(
        allocated, 2,
        "TWO Strings allocated: \"aaa\" (old dest) and \"hi\" (source)"
    );
    assert_eq!(
        dup, 0,
        "no double-free — \"hi\"'s single pointer is freed exactly once"
    );
    assert_eq!(
        freed, 1,
        "CÂU 1(b) FINDING: only ONE distinct pointer is freed (\"hi\"'s). \
         \"aaa\"'s pointer (the old dest value, overwritten by `a = p` \
         without first being dropped) is NEVER freed — a LEAK, not a \
         double-free. allocated(2) - freed(1) == 1 leaked pointer. Reported \
         to O per the WO ruling; drop-old-dest-before-overwrite is a \
         separate semantics decision, not fixed by this patch."
    );
}

#[test]
fn reassign_plain_poison_double_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run_with(SRC_7_REASSIGN_PLAIN, __amt_str_free_poison_double);
    assert_eq!(r, 2);
    let (_allocated, freed_distinct, dup) = dedup_stats();
    assert_eq!(freed_distinct, 1);
    assert_eq!(
        dup, 1,
        "POISON(double): proves the harness would catch a double-free of \
         the source pointer if the Deinit(v) fix regressed"
    );
}

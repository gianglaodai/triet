//! WO-5 (Mentor G, KHẨN, 2026-07-20) Bước ① — measure whether a LOCAL
//! heap-bearing `Struct?` (`Leaf { s: String }`, bound as `Leaf?`) leaks its
//! `String` field on scope-exit Drop. This is the "giá trị mù trước leak"
//! WO-5 §1 mandates be measured BEFORE any fix — no one had counted FREE
//! for this shape.
//!
//! Confirmed via `./target/release/triet-driver run` on `19a7708` (WO-5
//! base) BEFORE writing this harness: `SRC_LOCAL_NULLABLE` runs to
//! completion, exit 0, value 0 (no crash) — matches WO-5 §0b's claim that
//! the LOCAL position (unlike the Vector-element position, exit 134) is
//! silent, not a trap. Whether "silent" means "leaks" or "frees correctly"
//! is exactly what this harness settles by counting, not guessing.
//!
//! `SRC_LOCAL_CONTROL` (bare, non-nullable `Leaf`) is the required control:
//! the ordinary struct-Drop path is independently known-good (fixture
//! 338/340 exercise it against a real allocator), so it must read FREE=1
//! dup=0 — if the control itself doesn't read clean, the harness is broken,
//! not the finding.
//!
//! Reading the Drop-lowering code (`crates/triet-jit/src/mir_lower.rs:3337-
//! 3454`) BEFORE measuring: `Statement::Drop` on a `Nullable(Struct)` local
//! first checks `ty.is_copy(Some(body))` (line 3339) — `MirType::is_copy`
//! (`crates/triet-mir/src/lib.rs:718`) correctly delegates `Nullable(inner)
//! .is_copy(body)` to `inner.is_copy(body)`, and for `Struct("Leaf")` with a
//! `body` present it recurses the REAL field list and returns `false` (the
//! `String` field is not Copy) — so this does NOT short-circuit here, unlike
//! the naive top-level "Struct is always Copy" assumption that causes the
//! OTHER bug (`is_lowerable_nullable_payload`, `triet-mir/src/lib.rs:1679-
//! 1687`, unconditional `matches!(t, MirType::Struct(_))`). Drop then hits
//! the `struct_drop` arm (line 3388-3454): `Nullable(inner) => Some((name,
//! niche: 8, is_nullable: true))`, tag-guards via `struct_slots`, and frees
//! leaves at the CORRECT `+8`-shifted offset. This reads as a DIFFERENT,
//! already-correct code path from the Vector-element bug (which computes
//! `eff` by stripping `Nullable` BEFORE calling `emit_heap_free_at`, losing
//! the tag-guard/+8-shift that this Drop arm keeps inline) — but reading
//! code is not proof; the assertions below are.
//!
//! ⚠ RAM: `--exact --test-threads=1` (process-global `AtomicUsize`/`Mutex`
//! state and `no_mangle` shim symbols shared with any other test binary
//! loaded in the same process — N7 fork-bomb hazard, matches sibling
//! harnesses' warning).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static DUP_FREES: AtomicUsize = AtomicUsize::new(0);
static SEEN_PTRS: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn reset_counters() {
    STR_FREES.store(0, Ordering::SeqCst);
    DUP_FREES.store(0, Ordering::SeqCst);
    SEEN_PTRS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Records one free-call for `ptr`: bumps the raw counter always, and bumps
/// `DUP_FREES` if this exact pointer value was already seen this test — a
/// raw free-count of 1 alone cannot distinguish "one distinct pointer freed
/// once" from "one pointer freed twice while a different one leaks", both
/// of which would read FREE==1 on the raw counter alone (mirrors WO-1's
/// `is_empty_temp_leak_counting.rs` dedup rationale).
fn record_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
    let mut seen = SEEN_PTRS.lock().unwrap_or_else(|e| e.into_inner());
    if seen.contains(&ptr) {
        DUP_FREES.fetch_add(1, Ordering::SeqCst);
    } else {
        seen.push(ptr);
    }
}

#[unsafe(no_mangle)]
extern "C" fn __wo5_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    record_free(ptr);
}

/// POISON shim: simulates a leak (never frees, never counts) — proves the
/// raw counter is observing real free calls, not always reading the healthy
/// number by construction.
#[unsafe(no_mangle)]
extern "C" fn __wo5_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: frees the SAME pointer twice per call — proves the dedup
/// counter, not just the raw counter, is actually live.
#[unsafe(no_mangle)]
extern "C" fn __wo5_str_free_poison_dup(ptr: i64, cap: i64) {
    let _ = cap;
    record_free(ptr);
    record_free(ptr);
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
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
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
    run_with(source, __wo5_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// SRC_LOCAL_NULLABLE: `let a: Leaf? = ~+ Leaf { s: "hi" }`, heap-bearing
// Struct? LOCAL (Leaf has a String field). Confirmed via
// `./target/release/triet-driver run`: exit 0, value 0, on `19a7708`.
// ══════════════════════════════════════════════════════════════════════

const SRC_LOCAL_NULLABLE: &str = "struct Leaf { s: String }\n\
     function main() -> Integer {\n\
     \x20   let a: Leaf? = ~+ Leaf { s: \"hi\" }\n\
     \x20   return 0\n\
     }";

#[test]
fn wo5_local_nullable_struct_heap_field_free_count() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_LOCAL_NULLABLE);
    assert_eq!(r, 0, "main returns the literal 0");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    // WO-5 §1 MANDATE: this is the measurement, not a pre-decided pass/fail.
    // Record whichever number comes out — do not adjust the shape to match
    // an assumed answer.
    eprintln!("WO-5 §1 MEASUREMENT — local `Leaf?` heap field: FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "MEASURED (not assumed): local heap-bearing Struct? Drop frees the \
         String field exactly once — the tag-guarded struct_drop arm at \
         mir_lower.rs:3388-3454 (niche=8, is_nullable=true) IS exercised \
         and IS correct for this shape, unlike the Vector-element sibling \
         bug (WO-5 §0)"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// SRC_LOCAL_CONTROL: bare (non-nullable) `Leaf` local — the ordinary
// struct-Drop path, independently known-good (fixtures 338/340 exercise it
// against a REAL allocator). Must read FREE=1 dup=0; if not, the harness
// itself is broken, not the finding above.
// ══════════════════════════════════════════════════════════════════════

const SRC_LOCAL_CONTROL: &str = "struct Leaf { s: String }\n\
     function main() -> Integer {\n\
     \x20   let a: Leaf = Leaf { s: \"hi\" }\n\
     \x20   return 0\n\
     }";

#[test]
fn wo5_local_control_bare_struct_free_count() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_LOCAL_CONTROL);
    assert_eq!(r, 0);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONTROL — local bare `Leaf`: FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "control: the ordinary (non-nullable) struct-Drop path must free \
         the String field exactly once"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof: poison the free shim on the CONTROL (the shape with a
// known live free) and confirm both counters move away from the healthy
// value — proves the raw counter AND the dedup counter observe real free
// calls, not just always reading the healthy numbers by construction.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_control_proves_raw_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_LOCAL_CONTROL, __wo5_str_free_poison_leak);
    assert_eq!(r, 0);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONTROL POISON(leak): FREE={free} dup={dup}");
    assert_eq!(
        free, 0,
        "poison-leak (free shim never counts) must read 0, not the healthy \
         value — proves the counter observes real free calls"
    );
    assert_eq!(dup, 0);
}

#[test]
fn poison_dup_on_control_proves_dedup_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_LOCAL_CONTROL, __wo5_str_free_poison_dup);
    assert_eq!(r, 0);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONTROL POISON(dup): FREE={free} dup={dup}");
    assert_eq!(
        free, 2,
        "poison-dup double-counts the single real free call"
    );
    assert_eq!(
        dup, 1,
        "poison-dup must be caught by the dedup counter: the same pointer \
         value was recorded twice"
    );
}

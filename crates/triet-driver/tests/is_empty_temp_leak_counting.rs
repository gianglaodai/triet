//! WO-1 (Mentor O · 2026-07-20, G approved) — `is_empty` owned-String
//! fast-path temp leak, the LAST member of the
//! fast-path-bypasses-`emit_shim_call` family. `length`/`len`'s owned-String
//! fast path was fixed at WO-ShimTempOwnership
//! (`heap_shim_temp_leak_counting.rs`); `is_empty`'s OWN owned-String fast
//! path 95 lines below it (`crates/triet-lower/src/lib.rs`, the
//! `"is_empty" =>` arm's `matches!(arg_ty, MirType::String)` branch)
//! bypassed `emit_shim_call` identically and was missed — O measured
//! FREE=0 for an anonymous owned-String temp passed directly to
//! `is_empty` (this WO's §2 evidence table).
//!
//! **FIXED** (same commit as this file): the owned-String branch of
//! `"is_empty" =>` now calls `c.push_owned(arg)` before reading the `len`
//! field projection, mirroring `length`'s fix exactly.
//!
//! Unlike the sibling harness, this file also DEDUPS freed pointers: a
//! raw free-CALL count of 1 cannot distinguish the healthy case (one
//! distinct pointer freed once) from a broken "fix" that frees one
//! pointer twice while a different one silently leaks — both would read
//! FREE==1. Every assertion below therefore checks BOTH `free == 1` AND
//! `dup == 0`.
//!
//! Shapes (mirrors WO-1 §5):
//!   IE-A      is_empty("hello")        — anonymous literal temp (was FREE=0)
//!   IE-A-ctrl is_empty(s), s let-bound  — isolated control, already correct pre-fix
//!             (arg already registered via `Stmt::Let`; `push_owned` is a
//!             no-op dedup here, per WO-1 §7's "load-bearing only for the
//!             anonymous case" claim)
//!   IE-B      is_empty(h.name)          — anonymous field-access move-out temp (was FREE=0)
//!   LEN-A     length("hello")           — sibling-fix regression guard (WO-ShimTempOwnership)
//!   LEN-B     length(h.name)            — sibling-fix regression guard
//!
//! Every `.tri` source string below was independently run through
//! `./target/release/triet-driver run` (the FULL pipeline, including
//! borrowck) and confirmed `exit 0` with the value asserted here, BEFORE
//! being wired into this counting harness. `lower_source()` below stops at
//! parse→typecheck→lower and never runs borrowck — a shape it accepts is
//! not proof the real driver would accept it (WO-1 §7 pitfall; O's own
//! harness tripped this once measuring three shapes the driver actually
//! refuses with E2423).
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global `AtomicUsize`/
//! `Mutex` state and `no_mangle` shim symbols shared with any other test
//! binary loaded in the same process — N7 fork-bomb hazard, see
//! `heap_shim_temp_leak_counting.rs`).
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

/// Records one free-call for `ptr`: bumps the raw counter always, and
/// bumps `DUP_FREES` if this exact pointer value was already seen this
/// test — the dedup half of the WO-1 §5.1 mandate.
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
extern "C" fn __ietl_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    record_free(ptr);
}

/// POISON shim: simulates the pre-fix leak (never frees, never counts) —
/// used to prove the raw counter is observing real free calls, not just
/// always reading the healthy number by construction.
#[unsafe(no_mangle)]
extern "C" fn __ietl_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: frees the SAME pointer twice per call — proves the dedup
/// counter (`DUP_FREES`), not just the raw counter, is actually live.
#[unsafe(no_mangle)]
extern "C" fn __ietl_str_free_poison_dup(ptr: i64, cap: i64) {
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
    run_with(source, __ietl_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// IE-A: is_empty("hello") — anonymous LITERAL rvalue temp, owned-String
// fast path (bypasses emit_shim_call). Confirmed via
// `./target/release/triet-driver run`: exit 0, value -1.
// ══════════════════════════════════════════════════════════════════════

const SRC_IE_A: &str = "function main() -> Integer = {\n\
     \x20   return is_empty(\"hello\")\n\
     }";

#[test]
fn ie_a_is_empty_inline_literal() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_IE_A);
    assert_eq!(r, -1, "is_empty(\"hello\") is false -> Trilean-encoded -1");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("IE-A (is_empty inline-literal): FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "WO-1 fix: push_owned on is_empty's owned-String fast path must \
         free the literal's anonymous temp exactly once"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// IE-A-ctrl: is_empty(s), s let-bound — isolated control. Already correct
// pre-fix (arg is registered via Stmt::Let; push_owned is a no-op dedup
// here) — must stay green under the §6 poison protocol, proving the
// tooth is specific to the anonymous-temp case, not a blanket change.
// ══════════════════════════════════════════════════════════════════════

const SRC_IE_A_CTRL: &str = "function main() -> Integer = {\n\
     \x20   let s = \"hello\"\n\
     \x20   return is_empty(s)\n\
     }";

#[test]
fn ie_a_control_is_empty_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_IE_A_CTRL);
    assert_eq!(r, -1);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("IE-A-ctrl (is_empty let-bound): FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "let-bound s is already registered by Stmt::Let — freed once \
         regardless of the WO-1 fix"
    );
    assert_eq!(dup, 0);
}

// ══════════════════════════════════════════════════════════════════════
// IE-B: is_empty(h.name) — anonymous FIELD-ACCESS move-out temp, owned-
// String fast path. Confirmed via triet-driver run: exit 0, value -1.
// ══════════════════════════════════════════════════════════════════════

const SRC_IE_B: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" }\n\
     \x20   return is_empty(h.name)\n\
     }";

#[test]
fn ie_b_is_empty_field_access() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_IE_B);
    assert_eq!(r, -1, "is_empty(h.name) with name=\"hello\" is false -> -1");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("IE-B (is_empty field-access): FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "WO-1 fix: h.name's move-out temp must be freed exactly once"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// LEN-A / LEN-B: length() sibling-fix regression guards (already fixed by
// WO-ShimTempOwnership). Must stay green — proves this WO's edit to
// is_empty's arm did not disturb length's arm 95 lines away.
// ══════════════════════════════════════════════════════════════════════

const SRC_LEN_A: &str = "function main() -> Integer = {\n\
     \x20   return length(\"hello\")\n\
     }";

#[test]
fn len_a_length_inline_literal_regression_guard() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_LEN_A);
    assert_eq!(r, 5);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("LEN-A (length inline-literal, regression guard): FREE={free} dup={dup}");
    assert_eq!(free, 1, "WO-ShimTempOwnership sibling fix must still hold");
    assert_eq!(dup, 0);
}

const SRC_LEN_B: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" }\n\
     \x20   return length(h.name)\n\
     }";

#[test]
fn len_b_length_field_access_regression_guard() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_LEN_B);
    assert_eq!(r, 5);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("LEN-B (length field-access, regression guard): FREE={free} dup={dup}");
    assert_eq!(free, 1, "WO-ShimTempOwnership sibling fix must still hold");
    assert_eq!(dup, 0);
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof: poison the free shim on IE-A-ctrl (the shape with a
// live free) and confirm the counters move away from the healthy value
// in both directions — proves the raw counter AND the dedup counter both
// observe real free-call behavior, not just always reading the healthy
// numbers by construction.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_ie_a_ctrl_proves_raw_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_IE_A_CTRL, __ietl_str_free_poison_leak);
    assert_eq!(r, -1);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("IE-A-ctrl POISON(leak): FREE={free} dup={dup}");
    assert_eq!(
        free, 0,
        "poison-leak (free shim never counts) must read 0, not the \
         healthy value — proves the counter observes real free calls"
    );
    assert_eq!(dup, 0);
}

#[test]
fn poison_dup_on_ie_a_ctrl_proves_dedup_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_IE_A_CTRL, __ietl_str_free_poison_dup);
    assert_eq!(r, -1);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("IE-A-ctrl POISON(dup): FREE={free} dup={dup}");
    // The healthy case frees exactly 1 distinct pointer once; poisoning
    // the shim to record that SAME pointer twice must move BOTH counters:
    // raw FREE doubles (1 -> 2) and dup goes from 0 -> 1 (the second call
    // on the already-seen pointer). This is the scenario WO-1 §5.1 names:
    // a raw-FREE-only check could be fooled by a leak-cancels-a-double-free
    // coincidence; the dedup counter cannot be.
    assert_eq!(
        free, 2,
        "poison-dup double-counts the single real free call"
    );
    assert_eq!(
        dup, 1,
        "poison-dup must be caught by the dedup counter: the same \
         pointer value was recorded twice"
    );
}

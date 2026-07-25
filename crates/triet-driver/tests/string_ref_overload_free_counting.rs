//! WO-Str0Coverage (Mentor O · 2026-07-25) — FREE-count teeth for the new
//! `&0 String` builtin overloads added in `crates/triet-typecheck/src/env.rs`:
//! `len(&0 String)` (was missing from the ADR-0059 C.2 block; String's
//! sibling `&0 Vector`/`&0 HashMap` overloads were already there) and
//! `eq(&0 String, String)` / `eq(String, &0 String)` / `eq(&0 String, &0
//! String)` (was a single owned-only `declare`, now `declare_overload`'d
//! with the 3 `&0` combos per Phương án A — explicit overloads only, no
//! coercion rule in `check_call`).
//!
//! Both `len` and `eq` lower through `emit_shim_call` (triet-lower/src/
//! lib.rs) with `arg_consumes = [false, ...]` for every position — i.e.
//! borrow semantics for BOTH the owned-String and the `&0`-reference form.
//! For a named (`let`-bound) local passed as `&0 x`, the MIR arg is a
//! Reference-typed temp (`_N = &0 _M`), NOT the owning String local `_M`
//! itself — `_M` was already registered by `Stmt::Let`. The risk this
//! harness rules out: that routing a `&0`-wrapped local through `len`/`eq`
//! either (a) fails to register the owner at all (leak, FREE=0) or (b)
//! double-registers/double-frees it (FREE=2 for a single owner, or a
//! deduped-pointer collision) — neither should happen since `push_owned`
//! is called on the REFERENCE temp, not the owner, and is a no-op for an
//! already-`Stmt::Let`-registered owner (mirrors the `is_empty`
//! `IE-A-ctrl` control case in `is_empty_temp_leak_counting.rs`).
//!
//! WO-JIT-Concat (Mentor O · 2026-07-25) follow-up: `concat`'s `&0` combos,
//! initially halted here per LUẬT THÉP #4 (the JIT's `concat_sret`
//! marshaling class had no Reference-arg fallback, unlike `bung_fields`),
//! are now OPEN — `concat_sret`'s source-arg loop (mir_lower.rs) got the
//! same `is_reference()` fallback branch `bung_fields` already had,
//! mirrored verbatim. `concat` allocates a NEW String for its result (via
//! `__triet_string_alloc`, called directly inside `__triet_string_concat`),
//! so the 3 new-overload shapes below assert FREE=3 (both borrowed sources
//! PLUS the freshly-allocated result), not FREE=1/2 like len/eq — this is
//! the case the WO named explicitly: a leaked result reads FREE=2, a
//! double-freed arg reads dup>0.
//!
//! Every `.tri` source string below was independently run through
//! `./target/release/triet-driver run` (full pipeline incl. borrowck) and
//! confirmed exit 0 with the value asserted here, before being wired into
//! this counting harness.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global `AtomicUsize`/
//! `Mutex` state and `no_mangle` shim symbols shared with any other test
//! binary loaded in the same process — N7 fork-bomb hazard).
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
/// test — dedup half of the mandate (a raw free-CALL count alone cannot
/// distinguish "one distinct pointer freed once" from "one pointer freed
/// twice while a different one silently leaks").
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
extern "C" fn __sroc_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    record_free(ptr);
}

/// POISON shim: simulates a leak (never frees, never counts) — proves the
/// raw counter is observing real free calls, not always reading the
/// healthy number by construction.
#[unsafe(no_mangle)]
extern "C" fn __sroc_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: frees the SAME pointer twice per call — proves the dedup
/// counter, not just the raw counter, is actually live.
#[unsafe(no_mangle)]
extern "C" fn __sroc_str_free_poison_dup(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
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
    run_with(source, __sroc_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// LEN-REF: len(&0 s), s let-bound — the new len(&0 String) overload.
// Confirmed via triet-driver run: exit 0, value 5.
// ══════════════════════════════════════════════════════════════════════

const SRC_LEN_REF: &str = "function main() -> Integer {\n\
     \x20   let s = \"hello\";\n\
     \x20   return len(&0 s);\n\
     }";

#[test]
fn len_ref_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_LEN_REF);
    assert_eq!(r, 5, "len(&0 \"hello\") == 5");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("LEN-REF (len(&0 String)): FREE={free} dup={dup}");
    assert_eq!(
        free, 1,
        "s must be freed exactly once — push_owned on the reference temp \
         must not disturb s's own Stmt::Let-registered ownership"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// EQ-REF-REF / EQ-REF-OWNED / EQ-OWNED-REF: the 3 new eq(&0, ...) combos.
// Two distinct owners (a, b) each freed exactly once regardless of which
// side(s) are wrapped in &0 — eq's arg_consumes is [false,false,...] for
// every combo (borrow semantics), so a/b stay live and are freed by their
// own Stmt::Let/end-of-scope Drop either way.
// ══════════════════════════════════════════════════════════════════════

const SRC_EQ_REF_REF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hello\";\n\
     \x20   let b = \"hello\";\n\
     \x20   return eq(&0 a, &0 b);\n\
     }";

#[test]
fn eq_ref_ref_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_EQ_REF_REF);
    assert_eq!(r, 1, "eq(&0 \"hello\", &0 \"hello\") == 1");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("EQ-REF-REF (eq(&0,&0)): FREE={free} dup={dup}");
    assert_eq!(free, 2, "both a and b must be freed exactly once each");
    assert_eq!(dup, 0, "no pointer double-freed");
}

const SRC_EQ_REF_OWNED: &str = "function main() -> Integer {\n\
     \x20   let a = \"hello\";\n\
     \x20   let b = \"hello\";\n\
     \x20   return eq(&0 a, b);\n\
     }";

#[test]
fn eq_ref_owned_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_EQ_REF_OWNED);
    assert_eq!(r, 1, "eq(&0 \"hello\", \"hello\") == 1");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("EQ-REF-OWNED (eq(&0,owned)): FREE={free} dup={dup}");
    assert_eq!(free, 2, "both a and b must be freed exactly once each");
    assert_eq!(dup, 0, "no pointer double-freed");
}

const SRC_EQ_OWNED_REF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hello\";\n\
     \x20   let b = \"hello\";\n\
     \x20   return eq(a, &0 b);\n\
     }";

#[test]
fn eq_owned_ref_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_EQ_OWNED_REF);
    assert_eq!(r, 1, "eq(\"hello\", &0 \"hello\") == 1");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("EQ-OWNED-REF (eq(owned,&0)): FREE={free} dup={dup}");
    assert_eq!(free, 2, "both a and b must be freed exactly once each");
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof: poison the free shim on LEN-REF and EQ-REF-REF and
// confirm the counters move away from the healthy value in both
// directions — proves the raw counter AND the dedup counter both observe
// real free-call behavior, not just always reading the healthy numbers by
// construction.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_len_ref_proves_raw_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_LEN_REF, __sroc_str_free_poison_leak);
    assert_eq!(r, 5);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("LEN-REF POISON(leak): FREE={free} dup={dup}");
    assert_eq!(
        free, 0,
        "poison-leak (free shim never counts) must read 0, not the \
         healthy value — proves the counter observes real free calls"
    );
    assert_eq!(dup, 0);
}

#[test]
fn poison_dup_on_len_ref_proves_dedup_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_LEN_REF, __sroc_str_free_poison_dup);
    assert_eq!(r, 5);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("LEN-REF POISON(dup): FREE={free} dup={dup}");
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

#[test]
fn poison_leak_on_eq_ref_ref_proves_raw_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_EQ_REF_REF, __sroc_str_free_poison_leak);
    assert_eq!(r, 1);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("EQ-REF-REF POISON(leak): FREE={free} dup={dup}");
    assert_eq!(
        free, 0,
        "poison-leak must read 0 for BOTH owners, not the healthy value 2"
    );
    assert_eq!(dup, 0);
}

// ══════════════════════════════════════════════════════════════════════
// CONCAT-REF-REF / CONCAT-REF-OWNED / CONCAT-OWNED-REF: WO-JIT-Concat —
// the 3 new concat(&0, ...) combos, opened by mirroring bung_fields's
// is_reference() fallback into concat_sret. Each shape frees THREE
// distinct pointers: both source owners (a, b) PLUS the freshly allocated
// result `r` (read via `len(&0 r)` so `r` stays live to end of scope and
// gets its own Drop). Confirmed via triet-driver run before wiring in.
// ══════════════════════════════════════════════════════════════════════

const SRC_CONCAT_REF_REF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hello\";\n\
     \x20   let b = \" world\";\n\
     \x20   let r = concat(&0 a, &0 b);\n\
     \x20   return len(&0 r);\n\
     }";

#[test]
fn concat_ref_ref_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_CONCAT_REF_REF);
    assert_eq!(r, 11, "len(concat(&0 \"hello\", &0 \" world\")) == 11");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONCAT-REF-REF (concat(&0,&0)): FREE={free} dup={dup}");
    assert_eq!(
        free, 3,
        "a, b, and the freshly-allocated result r must each be freed \
         exactly once (2 -> leaked result; 2 with a collision -> arg \
         double-freed instead)"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

const SRC_CONCAT_REF_OWNED: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"yo\";\n\
     \x20   let r = concat(&0 a, b);\n\
     \x20   return len(&0 r);\n\
     }";

#[test]
fn concat_ref_owned_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_CONCAT_REF_OWNED);
    assert_eq!(r, 4, "len(concat(&0 \"hi\", \"yo\")) == 4");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONCAT-REF-OWNED (concat(&0,owned)): FREE={free} dup={dup}");
    assert_eq!(
        free, 3,
        "a, b, and result r must each be freed exactly once"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

const SRC_CONCAT_OWNED_REF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"yo\";\n\
     \x20   let r = concat(a, &0 b);\n\
     \x20   return len(&0 r);\n\
     }";

#[test]
fn concat_owned_ref_string_borrow_overload() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run(SRC_CONCAT_OWNED_REF);
    assert_eq!(r, 4, "len(concat(\"hi\", &0 \"yo\")) == 4");
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONCAT-OWNED-REF (concat(owned,&0)): FREE={free} dup={dup}");
    assert_eq!(
        free, 3,
        "a, b, and result r must each be freed exactly once"
    );
    assert_eq!(dup, 0, "no pointer double-freed");
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof for the concat shapes: poison the free shim on
// CONCAT-REF-REF and confirm the counters move away from the healthy
// value (3) in both directions.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_concat_ref_ref_proves_raw_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_CONCAT_REF_REF, __sroc_str_free_poison_leak);
    assert_eq!(r, 11);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONCAT-REF-REF POISON(leak): FREE={free} dup={dup}");
    assert_eq!(
        free, 0,
        "poison-leak must read 0 for all THREE pointers, not the healthy \
         value 3"
    );
    assert_eq!(dup, 0);
}

#[test]
fn poison_dup_on_concat_ref_ref_proves_dedup_counter_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_counters();
    let r = run_with(SRC_CONCAT_REF_REF, __sroc_str_free_poison_dup);
    assert_eq!(r, 11);
    let free = STR_FREES.load(Ordering::SeqCst);
    let dup = DUP_FREES.load(Ordering::SeqCst);
    eprintln!("CONCAT-REF-REF POISON(dup): FREE={free} dup={dup}");
    assert_eq!(
        free, 6,
        "poison-dup double-counts each of the 3 real free calls"
    );
    assert_eq!(
        dup, 3,
        "poison-dup must be caught by the dedup counter for all 3 pointers"
    );
}

//! WO-String-Eq-Content-Compare-And-Aggregate-Refuse §5(b) — paired
//! `(ptr, cap)` oracle for the `String ==`/`!=` content-compare dispatch
//! added to `Statement::BinaryOp` in `mir_lower.rs`.
//!
//! Mirrors `sret_string_field_counting.rs`'s harness shape exactly (same
//! paired-oracle rationale: a bare pointer count is blind to a corrupted
//! `cap`, only a `(ptr, cap)` pairing catches it).
//!
//! # Why this WO needs the SAME paired oracle, for a different reason
//!
//! `==`/`!=` reads `{ptr, len}` out of BOTH operands' `struct_slots` entries
//! via the new `load_string_fat` helper and calls `__triet_string_eq` — it
//! never touches `cap` and never writes to either slot. The risk this
//! harness is built to catch (per the WO's explicit warning, G's own
//! reminder) is NOT a cap-corruption bug like the sret WO — it's a
//! **double-free**: if a future change wired `__triet_string_eq` into the
//! `arg_consumes` consuming-argument machinery (the mechanism `Call`/
//! `MethodCall` lowering uses for shims that take ownership), the
//! operands' OWN scope-end `Drop` would free them a SECOND time. `==`
//! must NOT consume its operands — both `a` and `b` in `a == b` are still
//! live afterward and each gets exactly ONE real free from its own `Drop`.
//! This harness proves that with `distinct_alloc == distinct_freed` (no
//! leak) AND `dup_free == 0` (no double-free) for every fixture shape,
//! plus the `(ptr, cap)` pairing (proving the freed pointer's cap is
//! intact — an unrelated corruption would show as `cap_mismatches`).
//!
//! Both `__triet_string_alloc` and `__triet_string_from_bytes` are wrapped
//! per the WO even though only `from_bytes` fires for string-literal
//! fixtures below (`from_bytes` always allocs with `cap == len`,
//! `mir_lower.rs`'s `__triet_string_from_bytes` body) — `__triet_string_
//! alloc` is a direct shim reachable from other codegen paths, so the
//! harness stays correct if a future fixture exercises that path too.
#![allow(unsafe_code)]

use std::collections::HashSet;
use std::sync::Mutex;

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

/// `(ptr, cap)` recorded at alloc time — the EXPECTED cap for that pointer.
static ALLOC_CAP: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());
/// `(ptr, cap)` recorded at free time — the ACTUAL cap the freed call saw.
static FREED_CAP: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn reset() {
    ALLOC_CAP.lock().unwrap_or_else(|e| e.into_inner()).clear();
    FREED_CAP.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[unsafe(no_mangle)]
extern "C" fn __sec_string_alloc(len: i64, cap: i64) -> i64 {
    let ptr = mir_lower::__triet_string_alloc(len, cap);
    if ptr != 0 {
        ALLOC_CAP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((ptr, cap));
    }
    ptr
}

#[unsafe(no_mangle)]
extern "C" fn __sec_string_from_bytes(src: i64, len: i64) -> i64 {
    let ptr = mir_lower::__triet_string_from_bytes(src, len);
    if ptr != 0 {
        // from_bytes always allocs with cap == len (mir_lower.rs).
        ALLOC_CAP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((ptr, len));
    }
    ptr
}

#[unsafe(no_mangle)]
extern "C" fn __sec_string_free(ptr: i64, cap: i64) {
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        FREED_CAP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((ptr, cap));
    }
    mir_lower::__triet_string_free(ptr, cap);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn run(source: &str) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", __sec_string_alloc),
        ShimSymbol::fn_2_1("__triet_string_from_bytes", __sec_string_from_bytes),
        ShimSymbol::fn_2_0("__triet_string_free", __sec_string_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Paired dedup + cap-match stats.
struct Stats {
    distinct_alloc: usize,
    distinct_freed: usize,
    dup_free: usize,
    /// `(ptr, expected_cap_at_alloc, actual_cap_at_free)` for every freed
    /// pointer whose freed cap does NOT match its alloc-time cap.
    cap_mismatches: Vec<(i64, i64, i64)>,
}

fn stats() -> Stats {
    let allocs = ALLOC_CAP.lock().unwrap_or_else(|e| e.into_inner());
    let freed = FREED_CAP.lock().unwrap_or_else(|e| e.into_inner());
    let alloc_map: std::collections::HashMap<i64, i64> = allocs.iter().copied().collect();
    let distinct_alloc: HashSet<i64> = allocs.iter().map(|(p, _)| *p).collect();
    let distinct_freed: HashSet<i64> = freed.iter().map(|(p, _)| *p).collect();
    let dup_free = freed.len() - distinct_freed.len();
    let mut cap_mismatches = Vec::new();
    for (ptr, freed_cap) in freed.iter() {
        if let Some(expected) = alloc_map.get(ptr)
            && *expected != *freed_cap
        {
            cap_mismatches.push((*ptr, *expected, *freed_cap));
        }
    }
    Stats {
        distinct_alloc: distinct_alloc.len(),
        distinct_freed: distinct_freed.len(),
        dup_free,
        cap_mismatches,
    }
}

fn assert_two_distinct_no_dup_no_corruption(s: &Stats) {
    assert_eq!(s.distinct_alloc, 2, "two independent String allocations");
    assert_eq!(
        s.distinct_freed, 2,
        "== / != must NOT consume operands — both a and b free exactly once \
         via their own scope-end Drop, not zero (leak) or more (never freed \
         is impossible here, only a genuine leak would show fewer)"
    );
    assert_eq!(
        s.dup_free, 0,
        "arg_consumes for __triet_string_eq must be non-consuming — a \
         `consume` wiring would double-free (shim frees + Drop frees)"
    );
    assert!(
        s.cap_mismatches.is_empty(),
        "cap mismatch (ptr, expected, freed): {:?}",
        s.cap_mismatches
    );
}

const SRC_EQ_SAME: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"hi\";\n\
     \x20   if a == b { return 1; } else { return 0; }\n\
     }";

#[test]
fn eq_same_content_no_leak_no_dup_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_EQ_SAME);
    assert_eq!(r, 1, "\"hi\" == \"hi\" must content-compare TRUE");
    assert_two_distinct_no_dup_no_corruption(&stats());
}

const SRC_EQ_DIFF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"xx\";\n\
     \x20   if a == b { return 1; } else { return 0; }\n\
     }";

#[test]
fn eq_diff_content_same_length_no_leak_no_dup_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_EQ_DIFF);
    assert_eq!(r, 0, "\"hi\" == \"xx\" (same length) must be FALSE");
    assert_two_distinct_no_dup_no_corruption(&stats());
}

const SRC_NE_DIFF: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"xx\";\n\
     \x20   if a != b { return 1; } else { return 0; }\n\
     }";

#[test]
fn ne_diff_content_no_leak_no_dup_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_NE_DIFF);
    assert_eq!(r, 1, "\"hi\" != \"xx\" must be TRUE");
    assert_two_distinct_no_dup_no_corruption(&stats());
}

const SRC_EQ_EMPTY: &str = "function main() -> Integer {\n\
     \x20   let a = \"\";\n\
     \x20   let b = \"\";\n\
     \x20   if a == b { return 1; } else { return 0; }\n\
     }";

#[test]
fn eq_empty_no_leak_no_dup_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_EQ_EMPTY);
    assert_eq!(r, 1, "empty == empty must be TRUE");
    assert_two_distinct_no_dup_no_corruption(&stats());
}

const SRC_EQ_PREFIX: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   let b = \"hii\";\n\
     \x20   if a == b { return 1; } else { return 0; }\n\
     }";

#[test]
fn eq_prefix_len_differs_no_leak_no_dup_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_EQ_PREFIX);
    assert_eq!(
        r, 0,
        "\"hi\" == \"hii\" (prefix, len differs) must be FALSE"
    );
    assert_two_distinct_no_dup_no_corruption(&stats());
}

// ══════════════════════════════════════════════════════════════════════
// CONTROL — `a == a` (identity, ONE allocation used on both operand
// positions of the SAME local). Must stay a single alloc/single free —
// proves the dispatch doesn't double-count or double-free a shared local.
// ══════════════════════════════════════════════════════════════════════

const SRC_EQ_IDENTITY: &str = "function main() -> Integer {\n\
     \x20   let a = \"hi\";\n\
     \x20   if a == a { return 1; } else { return 0; }\n\
     }";

#[test]
fn eq_identity_single_alloc_single_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_EQ_IDENTITY);
    assert_eq!(r, 1, "a == a must be TRUE");
    let s = stats();
    assert_eq!(s.distinct_alloc, 1);
    assert_eq!(s.distinct_freed, 1);
    assert_eq!(s.dup_free, 0);
    assert!(
        s.cap_mismatches.is_empty(),
        "cap mismatch (ptr, expected, freed): {:?}",
        s.cap_mismatches
    );
}

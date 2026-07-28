//! WO-SRet-Aggregate-StringField-Corruption (O+G, 2026-07-29) — paired
//! `(ptr, cap)` oracle for the fat-String `len`/`cap` sync fixed in
//! `mir_lower.rs`'s `Statement::Assign` scalar-copy path:
//!
//! - **Hole A**: a `String`/`String?` field moved PROJECTED→PROJECTED
//!   (`_0.s = move _1.s`, the sret RETURN path — `_0` is the caller-supplied
//!   sret pointer local, excluded from `struct_slots`) never synced
//!   `len@+8`/`cap@+16`; only `ptr@0` crossed.
//! - **Hole B**: the STEP-4 construct-time sync guard
//!   (`matches!(dest_ty, MirType::String)`) excluded `Nullable(String)`, so a
//!   `String?` field built in a struct literal never got its len/cap synced
//!   either — independent of sret entirely.
//!
//! # Why a paired oracle, not a bare pointer count
//!
//! `param_aggregate_copyin_counting.rs` records only the POINTER on every
//! alloc/free call — sufficient for double-free/leak detection, but BLIND to
//! THIS bug: a cap-corrupted dest still has exactly one distinct alloc and
//! one distinct free of the SAME pointer (`alloc==free==1, dup==0` looks
//! perfectly healthy) even though the `cap` value handed to
//! `__triet_string_free` at Drop time is uninitialized stack garbage, not
//! the real allocation size.
//!
//! `__triet_string_from_bytes(src, len)` always calls
//! `__triet_string_alloc(len, len)` internally (`mir_lower.rs:~5554`, a
//! plain Rust intra-crate call — NOT through the JIT shim table), so
//! `cap == len` EXACTLY for every String this harness constructs. Recording
//! `map[ptr] = len` at alloc time gives an exact oracle for what the free
//! call's `cap` argument MUST equal; a mismatch is recorded and asserted on
//! after the run (not panicked-in-callback — an FFI-boundary panic across
//! JIT-compiled code is not sound, mirrors this repo's existing
//! record-then-assert convention).
//!
//! Both `__triet_string_alloc` and `__triet_string_from_bytes` are wrapped
//! (per the WO) even though only `from_bytes` fires for the fixtures below —
//! `__triet_string_alloc` IS reachable as a direct shim from other codegen
//! paths (`mir_lower.rs:4915`), so the harness stays correct if a future
//! fixture exercises that path too.
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
extern "C" fn __srsc_string_alloc(len: i64, cap: i64) -> i64 {
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
extern "C" fn __srsc_string_from_bytes(src: i64, len: i64) -> i64 {
    let ptr = mir_lower::__triet_string_from_bytes(src, len);
    if ptr != 0 {
        // mir_lower.rs:~5554 — from_bytes always allocs with cap == len.
        ALLOC_CAP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((ptr, len));
    }
    ptr
}

#[unsafe(no_mangle)]
extern "C" fn __srsc_string_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_1("__triet_string_alloc", __srsc_string_alloc),
        ShimSymbol::fn_2_1("__triet_string_from_bytes", __srsc_string_from_bytes),
        ShimSymbol::fn_2_0("__triet_string_free", __srsc_string_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
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

// ══════════════════════════════════════════════════════════════════════
// Hole A — sret return path, projected→projected field move.
// ══════════════════════════════════════════════════════════════════════

const SRC_HOLE_A: &str = "struct Leaf { s: String }\n\
     function make() -> Leaf {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return p;\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let l = make();\n\
     \x20   return length(l.s);\n\
     }";

#[test]
fn hole_a_sret_string_field_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_A);
    assert_eq!(r, 2, "\"hi\".length() == 2");
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

const SRC_HOLE_A_TWO_FIELDS: &str = "struct Two { a: String, b: String }\n\
     function make() -> Two {\n\
     \x20   let t = Two { a: \"hi\", b: \"world\" };\n\
     \x20   return t;\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let t = make();\n\
     \x20   return length(t.a) * 10 + length(t.b);\n\
     }";

#[test]
fn hole_a_sret_two_string_fields_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_A_TWO_FIELDS);
    assert_eq!(r, 25, "length(\"hi\")*10 + length(\"world\") == 25");
    let s = stats();
    assert_eq!(s.distinct_alloc, 2);
    assert_eq!(s.distinct_freed, 2);
    assert_eq!(s.dup_free, 0);
    assert!(
        s.cap_mismatches.is_empty(),
        "cap mismatch (ptr, expected, freed): {:?}",
        s.cap_mismatches
    );
}

const SRC_HOLE_A_DIRECT_LITERAL: &str = "struct Leaf { s: String }\n\
     function make() -> Leaf {\n\
     \x20   return Leaf { s: \"hi\" };\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let l = make();\n\
     \x20   return length(l.s);\n\
     }";

#[test]
fn hole_a_direct_struct_literal_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_A_DIRECT_LITERAL);
    assert_eq!(r, 2);
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

const SRC_HOLE_A_FORWARD_PARAM: &str = "struct Leaf { s: String }\n\
     function make() -> Leaf {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return p;\n\
     }\n\
     function take(p: Leaf) -> Integer {\n\
     \x20   return length(p.s);\n\
     }\n\
     function main() -> Integer {\n\
     \x20   return take(make());\n\
     }";

#[test]
fn hole_a_forward_param_copyin_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_A_FORWARD_PARAM);
    assert_eq!(r, 2);
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

// ══════════════════════════════════════════════════════════════════════
// Hole B — construct-time sync of a `String?` field, NO sret at all.
// ══════════════════════════════════════════════════════════════════════

const SRC_HOLE_B_READ: &str = "struct Leaf { s: String? }\n\
     function main() -> Integer {\n\
     \x20   let l = Leaf { s: ~+ \"hi\" };\n\
     \x20   return length(l.s!!);\n\
     }";

#[test]
fn hole_b_nullable_field_local_read_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_B_READ);
    assert_eq!(r, 2);
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

const SRC_HOLE_B_MATCH: &str = "struct Leaf { s: String? }\n\
     function main() -> Integer {\n\
     \x20   let l = Leaf { s: ~+ \"hi\" };\n\
     \x20   return match l.s {\n\
     \x20       ~+ v => length(v),\n\
     \x20       ~0 => 99,\n\
     \x20   };\n\
     }";

#[test]
fn hole_b_nullable_field_local_match_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_HOLE_B_MATCH);
    assert_eq!(r, 2);
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

// ══════════════════════════════════════════════════════════════════════
// Combined — Hole A AND Hole B, a `String?` field via sret.
// ══════════════════════════════════════════════════════════════════════

const SRC_COMBINED: &str = "struct Leaf { s: String? }\n\
     function make() -> Leaf {\n\
     \x20   let p = Leaf { s: ~+ \"hi\" };\n\
     \x20   return p;\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let l = make();\n\
     \x20   return length(l.s!!);\n\
     }";

#[test]
fn combined_sret_nullable_field_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_COMBINED);
    assert_eq!(r, 2);
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

// ══════════════════════════════════════════════════════════════════════
// CONTROL — nested struct sret return: already an aggregate word-copy
// (ty_total_size(Inner) == 24 > 8), unaffected by either hole. Must stay
// green whether or not the fix is present.
// ══════════════════════════════════════════════════════════════════════

const SRC_CONTROL_NESTED: &str = "struct Inner { s: String }\n\
     struct Outer { i: Inner, n: Integer }\n\
     function make() -> Outer {\n\
     \x20   let o = Outer { i: Inner { s: \"hi\" }, n: 3 };\n\
     \x20   return o;\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let o = make();\n\
     \x20   return length(o.i.s) + o.n;\n\
     }";

#[test]
fn control_nested_struct_cap_matches() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_CONTROL_NESTED);
    assert_eq!(r, 5, "length(\"hi\")=2 + n=3 == 5");
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

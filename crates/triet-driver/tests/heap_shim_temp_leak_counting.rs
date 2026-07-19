//! WO-LengthFastPathTempLeak T0 (O ✅ · G ✅, 2026-07-19) — is the leak found
//! while building WO-INV-HeapNullable-Probe's S3 tooth (`length(h.name)`,
//! FREE=0, RỈ CÂM) local to the hand-rolled `length()` fast path
//! (`triet-lower/src/lib.rs:2472-2479`), or does it reproduce on the general
//! SHIM-call argument path (`emit_shim_call`, e.g. `concat`/`contains`) too?
//!
//! O's own probe (independent of this harness) measured 7 shapes and
//! concluded the mechanism is NOT field-access-specific and NOT
//! `length()`-specific: it is "argument-lowering produces an anonymous
//! owned-heap temp (from a field-access move-out OR a string literal) that
//! is never registered for scope-end Drop (`push_owned`), UNLESS the callee
//! is a user function (which transfers ownership via the Call ABI + M3
//! Deinit-tombstone) or the temp is bound through a named `let`." O could
//! not measure the SHIM-call path directly — this harness's shim registry
//! didn't include `__triet_string_concat`/`__triet_string_contains` yet
//! (`Unsupported("shim '__triet_string_concat' not registered")`). This file
//! closes that gap and gets the actual numbers.
//!
//! Shapes (SH-A/B/C, each with a fully-`let`-bound control):
//!   SH-A: `concat("ab", "cd")`      — two LITERAL rvalue args (borrow-shim,
//!         `arg_consumes: [false,false,false,false]` in `builtin_shim_meta`)
//!   SH-B: `concat(h.name, "cd")`    — one FIELD-ACCESS + one LITERAL arg
//!   SH-C: `contains(h.name, "ell")` — FIELD-ACCESS + LITERAL, into a shim
//!         with NO `builtin_shim_meta` entry at all (`contains` is absent
//!         from the match in `crates/triet-mir/src/lib.rs`) — the M3
//!         zero-on-consume loop in the JIT (`mir_lower.rs:4718`) doesn't
//!         even run for it, by construction.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global AtomicUsize and
//! no-mangle shim — N7 fork-bomb hazard). `TEST_LOCK` Mutex serializes a
//! default parallel `cargo test` run within this binary.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __hstl_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

/// POISON shim: simulates a dropped free-arm (leak) — never counts.
#[unsafe(no_mangle)]
extern "C" fn __hstl_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: simulates a double-free — counts twice per real call.
#[unsafe(no_mangle)]
extern "C" fn __hstl_str_free_poison_double(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1(
            "__triet_string_contains",
            mir_lower::__triet_string_contains,
        ),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
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
    run_with(source, __hstl_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// SH-A: concat(LITERAL, LITERAL) — both args anonymous rvalue temps.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHA_INLINE: &str = "function main() -> Integer = {\n\
     \x20   let r = concat(\"ab\", \"cd\");\n\
     \x20   return length(r);\n\
     }";

#[test]
fn sha_concat_two_inline_literals() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHA_INLINE);
    assert_eq!(r, 4, "\"ab\"+\"cd\" concat length == 4");
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("SH-A (concat inline-literal x2): FREE={count}");
    assert_eq!(
        count, 1,
        "T0 measured: only `r` frees (1); both literal args to concat leak \
         (would be 3 if sound — see SH-A-ctrl below)"
    );
}

const SRC_SHA_CONTROL: &str = "function main() -> Integer = {\n\
     \x20   let a = \"ab\";\n\
     \x20   let b = \"cd\";\n\
     \x20   let r = concat(a, b);\n\
     \x20   return length(r);\n\
     }";

#[test]
fn sha_control_concat_two_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHA_CONTROL);
    assert_eq!(r, 4);
    eprintln!(
        "SH-A-ctrl (concat let-bound x2): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

// ══════════════════════════════════════════════════════════════════════
// SH-B: concat(FIELD-ACCESS, LITERAL) — mirrors the original repro shape.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHB_INLINE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let r = concat(h.name, \"cd\");\n\
     \x20   return length(r);\n\
     }";

#[test]
fn shb_concat_field_plus_inline_literal() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHB_INLINE);
    assert_eq!(r, 7, "\"hello\"+\"cd\" concat length == 7");
    eprintln!(
        "SH-B (concat field+inline-literal): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

const SRC_SHB_CONTROL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let n = h.name;\n\
     \x20   let lit = \"cd\";\n\
     \x20   let r = concat(n, lit);\n\
     \x20   return length(r);\n\
     }";

#[test]
fn shb_control_concat_both_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHB_CONTROL);
    assert_eq!(r, 7);
    eprintln!(
        "SH-B-ctrl (concat let-bound both): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

// ══════════════════════════════════════════════════════════════════════
// SH-C: contains(FIELD-ACCESS, LITERAL) — shim with NO builtin_shim_meta
// entry at all (M3 zero-on-consume loop doesn't run; irrelevant either way
// since `contains` never takes ownership — this isolates whether the ABSENCE
// of a meta entry changes anything, vs `concat`'s explicit all-false entry).
// ══════════════════════════════════════════════════════════════════════

const SRC_SHC_INLINE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let ok = contains(h.name, \"ell\");\n\
     \x20   return 0;\n\
     }";

#[test]
fn shc_contains_field_plus_inline_literal() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHC_INLINE);
    assert_eq!(r, 0);
    eprintln!(
        "SH-C (contains field+inline-literal): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

const SRC_SHC_CONTROL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let n = h.name;\n\
     \x20   let needle = \"ell\";\n\
     \x20   let ok = contains(n, needle);\n\
     \x20   return 0;\n\
     }";

#[test]
fn shc_control_contains_both_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHC_CONTROL);
    assert_eq!(r, 0);
    eprintln!(
        "SH-C-ctrl (contains let-bound both): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

// ══════════════════════════════════════════════════════════════════════
// SH-D: eq(FIELD-ACCESS, LITERAL) — third independent shim (`__triet_string_eq`,
// meta `arg_consumes: [false,false,false,false]`, same profile as `concat`),
// added to strengthen the "whole array" verdict with a 3rd data point.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHD_INLINE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   return eq(h.name, \"world\");\n\
     }";

#[test]
fn shd_eq_field_plus_inline_literal() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHD_INLINE);
    assert_eq!(r, -1, "\"hello\" != \"world\" -> Trilean-encoded -1");
    eprintln!(
        "SH-D (eq field+inline-literal): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

const SRC_SHD_CONTROL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let n = h.name;\n\
     \x20   let lit = \"world\";\n\
     \x20   return eq(n, lit);\n\
     }";

#[test]
fn shd_control_eq_both_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHD_CONTROL);
    assert_eq!(r, -1);
    eprintln!(
        "SH-D-ctrl (eq let-bound both): FREE={}",
        STR_FREES.load(Ordering::SeqCst)
    );
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof: poison the free shim on a fully-`let`-bound control
// (the shape with the most live frees, SH-B-ctrl expects 3) and confirm
// the count moves away from the healthy value in both directions.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_shb_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_SHB_CONTROL, __hstl_str_free_poison_leak);
    assert_eq!(r, 7);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("SH-B-ctrl POISON(leak): FREE={count}");
    assert_eq!(
        count, 0,
        "poison-leak (free shim never counts) must read 0, not whatever the \
         healthy count is — proves the counter observes real free calls"
    );
}

#[test]
fn poison_double_on_shb_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_SHB_CONTROL, __hstl_str_free_poison_double);
    assert_eq!(r, 7);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("SH-B-ctrl POISON(double): FREE={count}");
    // Healthy SH-B-ctrl frees 3 real calls (n, lit, r); poison-double reports
    // 2x that => 6. Whatever the healthy count turns out to be measured as,
    // the poison count must be exactly double it (asserted after T0 numbers
    // are read from the eprintln above) — pinned to the measured healthy
    // value 3*2=6 here since SH-B-ctrl is fully let-bound (all 3 owned
    // locals get a real Drop).
    assert_eq!(
        count, 6,
        "poison-double must read exactly 2x the healthy count (3 real frees)"
    );
}

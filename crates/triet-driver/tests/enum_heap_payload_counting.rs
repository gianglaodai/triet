//! ADR-0067 Lát 2 nhát 2b — route-lower free-count teeth for TOP-LEVEL
//! enum-payload heap (tag-switch drop-glue).
//!
//! The drop-glue reads the discriminant and frees ONLY the active variant's
//! heap payload via the correct per-type free shim. These counting tests pin
//! that behaviour with separate per-type counters + a cap recorder.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//! - R-enum-leak: drop the 2b-2 emit → Text's String never frees → str count 0.
//! - R-enum-double-free-move: drop the 2b-3 payload tombstone → enum move →
//!   caller + callee both free → count 2.
//! - R-enum-wrong-variant (G's xương sống): an enum with Text(String) AND
//!   Buf(Vector). Constructing Buf frees ONLY via vector_free (vec=1, str=0);
//!   constructing Text frees ONLY via string_free (str=1, vec=0). A cross-wired
//!   tag-switch (wrong arm / wrong shim) → wrong per-type counts (or SIGABRT).
//! - R-enum-cap: the cap passed to string_free must be the REAL cap (5 for
//!   "Giang"), not stack garbage. Poison 2b-0b (drop the cap copy) → cap != 5.
//!   The ONLY teeth that catches the enum-payload cap UB (counting ignores cap).
//!
//! ⚠ RAM: `--exact --test-threads=1` with ulimit -v (process-global AtomicUsize
//! and no-mangle shims — N7 fork-bomb hazard). The tests share counters, so a
//! Mutex serializes them (the gate runs `cargo test` parallel). Records-only
//! shims (no real dealloc) → a poisoned leak/double-free is an observable count,
//! not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_CAP: AtomicI64 = AtomicI64::new(-1);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __enum_str_free(ptr: i64, cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_CAP.store(cap, Ordering::SeqCst);
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __enum_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
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
    let shims = [
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __enum_str_free),
        ShimSymbol::fn_2_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __enum_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const MSG: &str = "enum Msg { Text(String), Code(Integer), Empty }\n";

/// R-enum-leak: Text(String) construct + drop → freed exactly once.
#[test]
fn enum_string_payload_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{MSG}function main() -> Integer = {{\n\
         \x20   let m = Msg::Text(\"Giang\");\n\
         \x20   return 0;\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2b: enum String payload must free exactly once on Drop"
    );
}

/// R-enum-double-free-move: enum whole-move → still freed exactly once.
#[test]
fn enum_heap_move_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{MSG}function take(m: Msg) -> Integer = {{\n\
         \x20   return 0\n\
         }}\n\
         function main() -> Integer = {{\n\
         \x20   let m = Msg::Text(\"Giang\");\n\
         \x20   return take(m);\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2b: enum arg-move must free the payload exactly once \
         (callee frees, caller tombstones payload@8 → Drop no-op)"
    );
}

/// Scalar-payload variant of a heap-capable enum → NO free (tag-switch matches
/// no heap arm for this disc).
#[test]
fn enum_scalar_variant_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{MSG}function main() -> Integer = {{\n\
         \x20   let m = Msg::Code(9);\n\
         \x20   return 0;\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "ADR-0067 2b: scalar variant Code(9) must free nothing"
    );
}

/// ⚔ R-enum-wrong-variant (G mandate): an enum with TWO heap variants of
/// DIFFERENT type. The tag-switch must dispatch the ACTIVE variant's shim only —
/// Buf(Vector) → vector_free (vec=1, str=0); Text(String) → string_free (str=1,
/// vec=0). A cross-wired arm calls the wrong shim → wrong per-type counts.
#[test]
fn enum_wrong_variant_dispatch() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let two = "enum Pair { Text(String), Buf(Vector<Integer>) }\n";

    // Active = Buf(Vector) → only vector_free fires.
    STR_FREES.store(0, Ordering::SeqCst);
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{two}function main() -> Integer = {{\n\
         \x20   let p = Pair::Buf(push(vector_new(), 1));\n\
         \x20   return 0;\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "Buf → vector freed once"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "Buf active → string_free must NOT fire (wrong-variant guard)"
    );

    // Active = Text(String) → only string_free fires.
    STR_FREES.store(0, Ordering::SeqCst);
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{two}function main() -> Integer = {{\n\
         \x20   let p = Pair::Text(\"hi\");\n\
         \x20   return 0;\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "Text → string freed once"
    );
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        0,
        "Text active → vector_free must NOT fire (wrong-variant guard)"
    );
}

/// R-enum-cap: the cap the drop-glue passes to string_free for an enum String
/// payload must be the REAL cap (5 for "Giang"), not stack garbage. Poison 2b-0b
/// (drop the cap copy) → garbage cap != 5.
#[test]
fn enum_string_payload_cap_preserved() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_CAP.store(-1, Ordering::SeqCst);
    let r = run(&format!(
        "{MSG}function main() -> Integer = {{\n\
         \x20   let m = Msg::Text(\"Giang\");\n\
         \x20   return 0;\n\
         }}"
    ));
    assert_eq!(r, 0);
    assert_eq!(
        STR_CAP.load(Ordering::SeqCst),
        5,
        "ADR-0067 2b-0b: enum String payload drop must free with the REAL cap \
         (5 for \"Giang\"), not stack garbage"
    );
}

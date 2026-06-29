//! ADR-0067 Lát 2 nhát 2b+ — route-lower free-count teeth for an ENUM field
//! sitting INSIDE a struct (the No-Box bridge: `collect_heap_leaves` →
//! `LeafKind::Enum` → `emit_enum_drop_glue_at` at the field's address).
//!
//! The struct drop walk reaches the enum leaf and runs the tag-switch core at
//! `copy_base_addr(local, field_offset)` so only the ACTIVE variant's heap
//! payload is freed via the correct per-type shim. These tests pin that with
//! separate per-type counters + a cap recorder.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//! - R-enum-in-struct-leak: drop the 2b+-A enum-leaf push → the enum field is
//!   never visited at drop → String never frees → str count 0 (leak).
//! - R-double-free-move: drop the 2b+-C Deinit enum tombstone (zero payload@abs+8)
//!   → assign-move → caller + callee both free → count 2.
//! - R-wrong-variant (G xương sống): a struct field enum with Text(String) AND
//!   Buf(Vector). Buf active → only vector_free; Text active → only string_free.
//!   A cross-wired tag-switch → wrong per-type counts.
//! - R-fat-store-cap: the cap passed to string_free must be the REAL cap (5 for
//!   "Giang"), not stack garbage. Poison death-line #2 (size enum field 8B) or
//!   the fat-store → cap != 5 (or SIGSEGV before we even get here).
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
extern "C" fn __eis_str_free(ptr: i64, cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_CAP.store(cap, Ordering::SeqCst);
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __eis_vec_free(ptr: i64) {
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
        ShimSymbol::fn_2_0("__triet_string_free", __eis_str_free),
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __eis_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const DECL: &str = "enum Msg { Text(String), Code(Integer), Empty }\n\
                    struct Wrapper { msg: Msg, tag: Integer }\n";

/// R-enum-in-struct-leak: Wrapper{Text(String)} construct + drop → freed once.
/// Also exercises death-line #2: `w.tag` (the scalar field AFTER the 32B enum)
/// must read 7 — a mis-sized 8B enum field would place tag inside the payload.
#[test]
fn enum_in_struct_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let w = Wrapper {{ msg: Msg::Text(\"Giang\"), tag: 7 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 7, "scalar field after the 32B enum must read 7 (layout)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2b+: enum-in-struct String payload must free exactly once on Drop"
    );
}

/// R-double-free-move: `let w2 = w` whole-struct move → still freed once.
#[test]
fn enum_in_struct_move_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let w = Wrapper {{ msg: Msg::Text(\"Giang\"), tag: 7 }};\n\
         \x20   let w2 = w;\n\
         \x20   return w2.tag;\n\
         }}"
    ));
    assert_eq!(r, 7);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2b+: enum-in-struct assign-move must free the payload exactly \
         once (Deinit zeroes payload@msg+8 → moved-from Drop no-op)"
    );
}

/// Scalar variant inside the struct → NO free (tag-switch matches no heap arm).
#[test]
fn enum_in_struct_scalar_variant_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let w = Wrapper {{ msg: Msg::Code(9), tag: 7 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 7);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "ADR-0067 2b+: scalar variant Code(9) in a struct field must free nothing"
    );
}

/// ⚔ R-wrong-variant (G mandate): a struct field enum with TWO heap variants of
/// DIFFERENT type. The field-address tag-switch must dispatch the ACTIVE
/// variant's shim only — Buf(Vector) → vector_free; Text(String) → string_free.
#[test]
fn enum_in_struct_wrong_variant_dispatch() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let decl = "enum Pair { Text(String), Buf(Vector<Integer>) }\n\
                struct WPair { p: Pair, tag: Integer }\n";

    // Active = Buf(Vector) → only vector_free fires.
    STR_FREES.store(0, Ordering::SeqCst);
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{decl}function main() -> Integer = {{\n\
         \x20   let w = WPair {{ p: Pair::Buf(push(vector_new(), 1)), tag: 7 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 7);
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
        "{decl}function main() -> Integer = {{\n\
         \x20   let w = WPair {{ p: Pair::Text(\"hi\"), tag: 7 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 7);
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

/// R-fat-store-cap: the cap the drop-glue passes to string_free for an enum
/// String payload inside a struct must be the REAL cap (5 for "Giang"), not
/// stack garbage. Poison death-line #2 / fat-store → garbage cap != 5.
#[test]
fn enum_in_struct_cap_preserved() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_CAP.store(-1, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let w = Wrapper {{ msg: Msg::Text(\"Giang\"), tag: 7 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 7);
    assert_eq!(
        STR_CAP.load(Ordering::SeqCst),
        5,
        "ADR-0067 2b+: enum-in-struct String payload drop must free with the REAL \
         cap (5 for \"Giang\"), not stack garbage"
    );
}

//! WO-NullableFieldMoveOut (ADR-0070 §AMEND Phase 4 + ADR-0076 §AMEND) —
//! route-lower free-count teeth for heap-`T?` field MOVE-OUT (`let s = b.s`
//! with `b.s: String?`/`Vector?`/`HashMap?`).
//!
//! The move-out is sound by STATIC TOMBSTONE, not a dynamic drop-flag: the JIT
//! zeroes the moved leaf's ptr/handle @field_off in the base slot
//! (`mir_lower.rs`, Site-2 `Nullable(inner) if inner.is_any_heap()` arm), so
//! `Drop(base)` reads ptr ∈ {0, NULL_SENTINEL} → the free shim no-ops. The
//! move-out dest gets its own real slot (Site-3 lowerer) and `Drop(dest)` frees
//! the heap exactly ONCE.
//!
//! "No crash" is NOT "no leak / no double-free" — these tests pin FREE_COUNT so
//! that:
//!   - present → FREE==1: a missing Site-2 tombstone double-frees (base + dest)
//!     → count 2 (red, the double-free probe Mentor O plugs by deleting the
//!     `Nullable(inner)` arm in `mir_lower.rs`); poisoning `is_copy(Nullable
//!     (heap))` → true makes the dest Copy (no Drop, no move-track) → leak,
//!     count 0.
//!   - null (`~0`) → FREE==0: the moved-out leaf is NULL_SENTINEL; both base
//!     and dest no-op.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global AtomicUsize and
//! no-mangle shims — N7 fork-bomb hazard). Records-only shims (no real dealloc)
//! so a poisoned leak/double-free is an observable count, not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static HMAP_FREES: AtomicUsize = AtomicUsize::new(0);

/// Serialize the tests in THIS binary: they share the process-global free
/// counters (no-mangle shims), so cargo's default parallel run would race.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __mo_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __mo_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __mo_hmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    HMAP_FREES.fetch_add(1, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __mo_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_3_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __mo_hmap_free),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __mo_str_free),
    ]
}

fn run(source: &str) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

// ── present field move-out → FREE==1 (double-free probe) ──
// `let s = b.s` moves the heap leaf into `s` and tombstones it in `b`. Both
// `b` and `s` are dropped at end-of-scope: the tombstone makes `Drop(b)` a
// no-op on the moved field, so the heap frees exactly ONCE via `Drop(s)`.
// Mentor O plugs the poison by deleting the `Nullable(inner)` arm in
// `mir_lower.rs` Site-2 → `Drop(b)` frees the live ptr a SECOND time → count 2.

#[test]
fn string_nullable_field_moveout_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bad { s: ~+ \"hi\" };\n\
         \x20   let s = b.s;\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "WO-NullableFieldMoveOut: present String? field move-out frees ONCE \
         (base tombstoned; missing Site-2 arm → FREE==2 double-free)"
    );
}

#[test]
fn vector_nullable_field_moveout_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { v: Vector<Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bag { v: ~+ push(vector_new(), 1) };\n\
         \x20   let x = b.v;\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "WO-NullableFieldMoveOut: present Vector? field move-out frees ONCE"
    );
}

#[test]
fn hashmap_nullable_field_moveout_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HMAP_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Cache { m: HashMap<Integer, Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let c = Cache { m: ~+ insert(hashmap_new(), 1, 100) };\n\
         \x20   let x = c.m;\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        HMAP_FREES.load(Ordering::SeqCst),
        1,
        "WO-NullableFieldMoveOut: present HashMap? field move-out frees ONCE"
    );
}

// ── null (`~0`) field move-out → FREE==0 ──
// A `~0` field stores NULL_SENTINEL; moving it into `s` carries the sentinel
// and tombstones the base. Both base and dest no-op at Drop → count 0.

#[test]
fn string_nullable_field_moveout_null_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bad { s: ~0 };\n\
         \x20   let s = b.s;\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "WO-NullableFieldMoveOut: null String? field move-out carries \
         NULL_SENTINEL → Drop is a no-op on both base and dest"
    );
}

#[test]
fn vector_nullable_field_moveout_null_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { v: Vector<Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bag { v: ~0 };\n\
         \x20   let x = b.v;\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        0,
        "WO-NullableFieldMoveOut: null Vector? field move-out → no free"
    );
}

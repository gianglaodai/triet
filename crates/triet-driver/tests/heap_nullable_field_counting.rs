//! ADR-0076 (WO-0076) — route-lower free-count teeth for heap-`T?` struct
//! FIELDS (`String?`/`Vector?`/`HashMap?`). The lift places a sentinel-bearing
//! slot at the field-offset (String? = 24B fat, Vector?/HashMap? = 8B handle)
//! and `collect_heap_leaves` pushes a `Nullable(inner) if inner.is_any_heap()`
//! arm so Drop frees it UNCONDITIONALLY (shim no-ops on NULL_SENTINEL / 0).
//!
//! "No crash" is NOT "no leak / no double-free" — these counting tests pin
//! FREE_COUNT so a future edit that drops the drop-arm (leak → 0), stores 0
//! instead of the sentinel, or fails to tombstone a moved base (double-free → 2)
//! is caught.
//!
//! Teeth (Mentor O re-verifies on the final tree — quét cả 3 biến thể, HP.3):
//!   - present → FREE==1: drop the `Nullable(heap)` arm in `collect_heap_leaves`
//!     → the field never frees → count 0 (leak, red).
//!   - null → FREE==0: a `~0` field stores NULL_SENTINEL; the shim no-ops →
//!     count 0. Poison store-0-only would still be 0 here, but a store of a
//!     bogus non-sentinel ptr would free garbage (crash) — see the run fixtures.
//!   - construct→move struct→drop → FREE==1: moving the whole struct into a
//!     callee and dropping it there frees the field ONCE; a missing move
//!     tombstone double-frees → count 2 (red).
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
extern "C" fn __nf_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __nf_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __nf_hmap_free(ptr: i64) {
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
        ShimSymbol::fn_1_0("__triet_vector_free", __nf_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_3_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __nf_hmap_free),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __nf_str_free),
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

#[test]
fn string_nullable_field_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bad { s: ~+ \"hi\" };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076: present String? field must free exactly once on Drop"
    );
}

#[test]
fn string_nullable_field_null_no_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bad { s: ~0 };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "ADR-0076: null String? field stores NULL_SENTINEL → Drop is a no-op"
    );
}

#[test]
fn vector_nullable_field_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { v: Vector<Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bag { v: ~+ push(vector_new(), 1) };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076: present Vector? field must free exactly once on Drop"
    );
}

#[test]
fn hashmap_nullable_field_present_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HMAP_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Cache { m: HashMap<Integer, Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let c = Cache { m: ~+ insert(hashmap_new(), 1, 100) };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        HMAP_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076: present HashMap? field must free exactly once on Drop"
    );
}

/// construct → move whole struct into a callee → drop in callee: FREE==1, no
/// double-free. A missing move tombstone on the moved base would free both the
/// caller's and the callee's copy → count 2 (red).
#[test]
fn nullable_heap_field_move_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function take(b: Bad) -> Integer = 0\n\
         function main() -> Integer = {\n\
         \x20   let b = Bad { s: ~+ \"hi\" };\n\
         \x20   return take(b);\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076: moving a struct with a heap-T? field frees the field ONCE \
         (no double-free between caller and callee)"
    );
}

// ── CASE B (Mentor O): present-bind-move on a heap-bearing nullable-aggregate ──
// `match present_Struct?/Enum? { ~+ v => … }` binds `v = move scrut` (a niche
// copy). Without the post-bind tombstone the scrutinee's join-point Drop frees
// the SAME heap a SECOND time (FREE==2, SIGABRT). The lowerer tombstones the
// scrutinee (tag/disc@0 → NULL_SENTINEL) so the niche tag acts as the drop-flag.
// Poison the `Deinit(scrut_local)` emit (lib.rs) → these go FREE==2 (red).

#[test]
fn string_nullable_field_match_bind_move_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bad { s: String? }\n\
         function main() -> Integer = {\n\
         \x20   let b: Bad? = Bad { s: ~+ \"hi\" };\n\
         \x20   return match b {\n\
         \x20       ~+ v => 1,\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 1);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076 CASE B: present-bind-move of a String? field frees ONCE \
         (scrutinee tombstoned; missing Deinit → FREE==2)"
    );
}

#[test]
fn vector_nullable_field_match_bind_move_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { v: Vector<Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let b: Bag? = Bag { v: ~+ push(vector_new(), 1) };\n\
         \x20   return match b {\n\
         \x20       ~+ x => 1,\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 1);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076 CASE B: present-bind-move of a Vector? field frees ONCE"
    );
}

#[test]
fn hashmap_nullable_field_match_bind_move_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HMAP_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Cache { m: HashMap<Integer, Integer>? }\n\
         function main() -> Integer = {\n\
         \x20   let c: Cache? = Cache { m: ~+ insert(hashmap_new(), 1, 100) };\n\
         \x20   return match c {\n\
         \x20       ~+ x => 1,\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 1);
    assert_eq!(
        HMAP_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076 CASE B: present-bind-move of a HashMap? field frees ONCE"
    );
}

/// Enum variant of CASE B: a present `Enum?` whose active variant carries a heap
/// payload, present-bound. The scrutinee Deinit sets disc@0 → NULL_SENTINEL so
/// the join-point enum tag-switch frees nothing; the payload frees once via `v`.
#[test]
fn enum_heap_payload_match_bind_move_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Bag { None, Has(String) }\n\
         function main() -> Integer = {\n\
         \x20   let b: Bag? = Bag::Has(\"hi\");\n\
         \x20   return match b {\n\
         \x20       ~+ v => 1,\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 1);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0076 CASE B: present-bind-move of a heap enum payload frees ONCE"
    );
}

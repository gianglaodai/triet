//! ADR-0066 Lát 1 nhát 1d (WO-1d, LOCK & SEAL) — route-lower free-count teeth
//! for heap-leaf struct FIELDS (Vector/HashMap), plus an isolation scalpel.
//!
//! The 1a/1b machinery is type-generic (is_any_heap drop-glue walk,
//! emit_heap_free_at per-arity dispatch), so Vector/HashMap fields already work.
//! "No crash" is NOT "no leak" though — these counting tests pin FREE_COUNT so a
//! future edit that drops a heap type out of the drop-glue is caught.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - R-leak-vec : cut the is_vec branch in emit_heap_free_at → Bag's Vector
//!     field never frees → vector count == 0 (leak).
//!   - R-leak-hmap: cut the is_hashmap branch → Cache's HashMap count == 0.
//!   - ISOLATION SCALPEL (G mandate): `Mixed { tags: Vector, name: String }` —
//!     cutting ONLY is_vec leaks the Vector (count 0) while the String in the
//!     SAME struct still frees (count 1). Proves the drop-glue dispatches
//!     per-field-type, not all-or-nothing.
//!
//! ⚠ RAM: run `--exact --test-threads=1` with ulimit -v (process-global
//! AtomicUsize and no-mangle shims — N7 fork-bomb hazard). Records-only shims
//! (no real dealloc) so a poisoned leak is an observable count, not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static HMAP_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_FREES: AtomicUsize = AtomicUsize::new(0);

/// Serialize the tests in THIS binary: they share the process-global free
/// counters (no-mangle shims), so cargo's default parallel run would race
/// (reset-then-read windows interleave). The gate runs `cargo test` without
/// `--test-threads=1`, so the lock — not the flag — is what makes the counts
/// deterministic here. `into_inner` ignores a poisoned lock (a panicking
/// poison-verification run doesn't wedge the others).
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __lock_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __lock_hmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    HMAP_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __lock_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
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
        ShimSymbol::fn_1_0("__triet_vector_free", __lock_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __lock_hmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // ADR-0083: struct-key hash/eq walkers reference these for String leaves.
        ShimSymbol::fn_2_1("__triet_string_hash", mir_lower::__triet_string_hash),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_2_0("__triet_string_free", __lock_str_free),
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
fn vector_field_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { items: Vector<Integer> }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bag { items: push(vector_new(), 1) };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1d: struct Vector field must free exactly once on Drop"
    );
}

/// ★ ADR-0083 §4 — KEY-DROP COUNTING (G-MANDATE, permanent): a `HashMap<KStr,
/// Integer>` key `KStr{name:String}` carries a String leaf. On map Drop the
/// JIT-emitted key-free loop must recurse into the struct key and free its
/// String leaf EXACTLY once. `insert` consumes (Moves) the key; M3 tombstones
/// the local so its own Drop is a no-op → the sole free is the map's. Poison
/// (RED): change `emit_hashmap_free_value`'s key gate back to
/// `key_ty.is_any_heap()` (false for a Struct) → the key-free loop is never
/// emitted → the String leaf LEAKS → count 0.
#[test]
fn hashmap_struct_key_string_leaf_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct KStr { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let m: HashMap<KStr, Integer> = hashmap_new();\n\
         \x20   let k = KStr { name: \"alice\" };\n\
         \x20   let m2 = insert(m, k, 42);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0083: struct-key String leaf must free exactly once on map Drop (key-free loop)"
    );
}

#[test]
fn hashmap_field_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HMAP_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Cache { m: HashMap<Integer, Integer> }\n\
         function main() -> Integer = {\n\
         \x20   let c = Cache { m: insert(hashmap_new(), 1, 100) };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        HMAP_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1d: struct HashMap field must free exactly once on Drop"
    );
}

/// ISOLATION SCALPEL (G mandate): a struct with BOTH a Vector and a String
/// field. In the healthy build both free once. The R-leak-vec poison (cut ONLY
/// the is_vec branch in emit_heap_free_at) must leave VEC_FREES==0 while
/// STR_FREES==1 — proving per-field-type dispatch, not all-or-nothing.
#[test]
fn mixed_vector_string_field_each_freed_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Mixed { tags: Vector<Integer>, name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mx = Mixed { tags: push(vector_new(), 1), name: \"hi\" };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1d: Vector field of a mixed struct must free once"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1d: String field of a mixed struct must free once \
         (survives an is_vec-only poison)"
    );
}

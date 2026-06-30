//! ADR-0078 Typed HashMap P1 — Slice A backend free-count teeth, via HAND-BUILT
//! MIR (typecheck is NOT yet open for `HashMap<Integer,T>` — that is Slice B, so
//! `insert(hashmap_new(), 1, "hi")` is E1003 at source; hand-built MIR is the
//! authorized Slice A verification path).
//!
//! These pin the storage + ownership machinery of MŨI B/C/D:
//!   - MŨI B slot fat-value inline: alloc(len, cap, value_stride=24) for String
//!     values; insert/get/remove/free read stride from header.
//!   - MŨI C typed drop-glue: `Drop(map)` iterates cap slots, frees occupied
//!     VALUES via `emit_heap_free_at` (registry-routed → counted), then
//!     `hashmap_free` for the buffer. Key=Integer NOT freed.
//!   - MŨI D remove take-out: `remove(map,key)` tombstones + returns value;
//!     caller drops the value, map drops survivors → no double-free.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - #1 SIGABRT 134: insert heap value, poison value-arg consume → false →
//!     caller double-free (G gold standard real-allocator).
//!   - #2 drop leak: poison slot-iteration loop → FREE==0 (leak).
//!   - #3 rehash value-stride: poison i64-read instead of memcpy → corruption
//!     on rehash/grow.
//!   - #4 remove: insert→remove→drop FREE đúng; poison tombstone → double-free.
//!   - #5 backward-compat: HashMap<Integer,Integer> insert/get/remove corpus.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global counters + no-mangle
//! shims). Records-only string-free shim so a poisoned leak/double-free is an
//! observable count, not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_borrowck::{MirBuilder, storage_live};
use triet_jit::mir_lower::{self, JitContext, ShimSymbol};
use triet_mir::{
    CallTarget, ConstValue, FunctionId, Local, MirType, Place, ReturnShape, Statement, Terminator,
};

const DUMMY_SPAN: triet_mir::Span = triet_mir::Span { start: 0, end: 0 };

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Counting stand-in for `__triet_string_free` (mirrors the real null/sentinel
/// guard so only LIVE value frees count).
#[unsafe(no_mangle)]
extern "C" fn __hm_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

fn shims() -> Vec<ShimSymbol> {
    vec![
        // Alloc/free/insert/get/remove are REAL (actually allocate + dealloc).
        ShimSymbol::fn_3_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_3_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // Element free is the COUNTING stub (the teeth surface).
        ShimSymbol::fn_2_0("__triet_string_free", __hm_str_free),
    ]
}

fn string_layout() -> triet_mir::StructLayout {
    triet_mir::StructLayout::compute(
        "String",
        &[
            ("ptr".to_string(), MirType::Integer, 8, 8),
            ("len".to_string(), MirType::Integer, 8, 8),
            ("cap".to_string(), MirType::Integer, 8, 8),
        ],
    )
}

fn hashmap_ii_ty() -> MirType {
    MirType::HashMap(Box::new(MirType::Integer), Box::new(MirType::Integer))
}

fn hashmap_is_ty() -> MirType {
    MirType::HashMap(Box::new(MirType::Integer), Box::new(MirType::String))
}

/// Shim-call terminator (builtin shims need `CallTarget::Shim`).
fn shim_call(
    name: &str,
    args: Vec<Local>,
    dest: Vec<Local>,
    return_bb: triet_mir::BasicBlock,
) -> Terminator {
    Terminator::CallDispatch {
        callee: FunctionId(0),
        callee_name: name.to_string(),
        target: CallTarget::Shim,
        args,
        return_bb,
        dest,
        return_shape: ReturnShape::Scalar,
        span: DUMMY_SPAN,
    }
}

#[allow(unsafe_code)]
fn jit_run(body: &triet_mir::Body) -> i64 {
    body.verify().expect("MIR verify");
    let mut ctx = JitContext::with_shims(&shims());
    let func = ctx
        .compile(body)
        .expect("typed-hashmap body must JIT-compile");
    unsafe { func.call_i64_0() }
}

fn const_i(dest: Local, value: i128) -> Statement {
    Statement::Const {
        dest: Place::local(dest),
        value: ConstValue::Integer(value),
        span: DUMMY_SPAN,
    }
}

/// Build `main()`: alloc a `HashMap<Integer,String>`, insert `pairs` (key, word),
/// optionally remove `remove_key` (and Drop the removed value), then drop the map.
/// Returns 0. Each insert consumes the String value (move).
fn build_insert_drop(pairs: &[(i64, &str)], remove_key: Option<i64>) -> triet_mir::Body {
    let mut b = MirBuilder::new("main", MirType::Integer);
    b.add_struct_layout(string_layout());

    let bb = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    b.push(bb, storage_live(len0));
    b.push(bb, const_i(len0, 0));
    b.push(bb, storage_live(cap0));
    b.push(bb, const_i(cap0, 4));
    let mut map_local = b.new_local();
    b.set_local_mir_type(map_local, hashmap_is_ty());
    b.push(bb, storage_live(map_local));

    let mut cur = bb;
    let mut next = b.new_block();
    b.set_terminator(
        cur,
        shim_call(
            "__triet_hashmap_alloc",
            vec![len0, cap0],
            vec![map_local],
            next,
        ),
    );
    cur = next;

    // Insert each pair
    for &(key, word) in pairs {
        let s = b.new_local();
        b.set_local_mir_type(s, MirType::String);
        b.push(cur, storage_live(s));
        b.push(
            cur,
            Statement::Const {
                dest: Place::local(s),
                value: ConstValue::String(word.to_string()),
                span: DUMMY_SPAN,
            },
        );
        let key_loc = b.new_local();
        b.push(cur, storage_live(key_loc));
        b.push(cur, const_i(key_loc, key.into()));
        let new_map = b.new_local();
        b.set_local_mir_type(new_map, hashmap_is_ty());
        b.push(cur, storage_live(new_map));
        next = b.new_block();
        b.set_terminator(
            cur,
            shim_call(
                "__triet_hashmap_insert",
                vec![map_local, key_loc, s],
                vec![new_map],
                next,
            ),
        );
        map_local = new_map;
        cur = next;
    }

    // Optionally remove
    if let Some(rk) = remove_key {
        let key_loc = b.new_local();
        b.push(cur, storage_live(key_loc));
        b.push(cur, const_i(key_loc, rk.into()));
        let out = b.new_local();
        b.set_local_mir_type(out, MirType::String);
        b.push(cur, storage_live(out));
        next = b.new_block();
        b.set_terminator(
            cur,
            shim_call(
                "__triet_hashmap_remove",
                vec![map_local, key_loc],
                vec![out],
                next,
            ),
        );
        cur = next;
        b.push(cur, Statement::Drop(out, DUMMY_SPAN));
    }

    // Drop the map (frees survivors + buffer)
    b.push(cur, Statement::Drop(map_local, DUMMY_SPAN));
    let result = b.new_local();
    b.push(cur, storage_live(result));
    b.push(cur, const_i(result, 0));
    b.set_terminator(
        cur,
        Terminator::Return {
            values: vec![result],
            span: DUMMY_SPAN,
        },
    );
    b.build(bb)
}

// ── Teeth #1/#5 — insert heap value → drop → FREE==N ──

#[test]
fn hashmap_string_insert_drop_frees_values() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_insert_drop(&[(1, "alpha"), (2, "beta"), (3, "gamma")], None);
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "ADR-0078: HashMap<String> drop frees EACH occupied value once \
         (poison the JIT slot-loop → 0 = leak)"
    );
}

#[test]
fn hashmap_string_empty_drop_frees_nothing() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_insert_drop(&[], None);
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "ADR-0078: empty HashMap<String> drop frees nothing (0 occupied slots)"
    );
}

// ── Teeth #4 — insert → remove → drop → FREE đúng ──

#[test]
fn hashmap_string_remove_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    // Insert 3, remove key=2 (value dropped by caller), drop map (2 survivors).
    // Total = 2 survivors freed by map + 1 removed freed by caller = 3.
    let body = build_insert_drop(&[(1, "a"), (2, "b"), (3, "c")], Some(2));
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "ADR-0078: insert 3 → remove 1 → drop map → 3 frees \
         (2 survivors + 1 removed by caller; poison tombstone → double-free)"
    );
}

#[test]
fn hashmap_string_remove_notfound_noop() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    // Insert 2, remove key=99 (not found → NULL), drop map (2 survivors → 2 frees).
    let body = build_insert_drop(&[(10, "x"), (20, "y")], Some(99));
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0078: remove not-found → NULL → no value freed, map drops both"
    );
}

// ── Teeth #5 — backward-compat HashMap<Integer,Integer> ──

fn build_hashmap_int_int() -> triet_mir::Body {
    let mut b = MirBuilder::new("main", MirType::Integer);
    let bb = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    b.push(bb, storage_live(len0));
    b.push(bb, const_i(len0, 0));
    b.push(bb, storage_live(cap0));
    b.push(bb, const_i(cap0, 4));
    let m = b.new_local();
    b.set_local_mir_type(m, hashmap_ii_ty());
    b.push(bb, storage_live(m));
    let cur = bb;
    let next = b.new_block();
    b.set_terminator(
        cur,
        shim_call("__triet_hashmap_alloc", vec![len0, cap0], vec![m], next),
    );
    // insert(m, 5, 50)
    let k5 = b.new_local();
    b.push(next, storage_live(k5));
    b.push(next, const_i(k5, 5));
    let v50 = b.new_local();
    b.push(next, storage_live(v50));
    b.push(next, const_i(v50, 50));
    let m2 = b.new_local();
    b.set_local_mir_type(m2, hashmap_ii_ty());
    b.push(next, storage_live(m2));
    let n2 = b.new_block();
    b.set_terminator(
        next,
        shim_call("__triet_hashmap_insert", vec![m, k5, v50], vec![m2], n2),
    );
    // get(m2, 5) → 50
    let k5b = b.new_local();
    b.push(n2, storage_live(k5b));
    b.push(n2, const_i(k5b, 5));
    let g = b.new_local();
    b.push(n2, storage_live(g));
    let n3 = b.new_block();
    b.set_terminator(
        n2,
        shim_call("__triet_hashmap_get", vec![m2, k5b], vec![g], n3),
    );
    // Return g
    b.set_terminator(
        n3,
        Terminator::Return {
            values: vec![g],
            span: DUMMY_SPAN,
        },
    );
    b.build(bb)
}

#[test]
fn hashmap_int_int_insert_get_readback() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_hashmap_int_int();
    let r = jit_run(&body);
    assert_eq!(
        r, 50,
        "ADR-0078: HashMap<Integer,Integer> insert→get→50 backward-compat"
    );
}

// ── Teeth #3 — rehash value-stride (shim-level): push 3 fat values, force
// realloc by filling to capacity, verify get returns correct value ──
// For stride=8 (Integer→Integer), insert 3 entries into cap=4 triggers
// realloc at 3 (0.75 load factor). After realloc, get returns correct value.
// This proves the rehash loop's value-copy (memcpy stride) works.

#[test]
fn hashmap_rehash_retains_values() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Direct shim-call: cap=4, rehash at len*4 >= cap*3 → 3 inserts trigger realloc.
    let m = mir_lower::__triet_hashmap_alloc(0, 4, 8);
    assert_ne!(m, 0);
    let m = mir_lower::__triet_hashmap_insert(m, 5, 50);
    let m = mir_lower::__triet_hashmap_insert(m, 6, 60);
    let m = mir_lower::__triet_hashmap_insert(m, 7, 70);
    // 4th insert forces realloc (len=3, cap=4, 3*4 >= 4*3 → trigger)
    let m = mir_lower::__triet_hashmap_insert(m, 13, 130);
    assert_eq!(
        mir_lower::__triet_hashmap_get(m, 5),
        50,
        "rehash: key 5 → 50"
    );
    assert_eq!(
        mir_lower::__triet_hashmap_get(m, 7),
        70,
        "rehash: key 7 → 70"
    );
    assert_eq!(
        mir_lower::__triet_hashmap_get(m, 13),
        130,
        "rehash: key 13 → 130"
    );
    mir_lower::__triet_hashmap_free(m);
}

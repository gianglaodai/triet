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

// ── Teeth #3 — rehash value-stride (fat-value MUST use memcpy, not i64) ──
// Insert 4 entries into cap=4 → triggers realloc (load factor 0.75: len*4
// >= cap*3 at len=3, the 4th insert triggers). With value_stride=24 (String
// fat 24B), the rehash loop must `copy_nonoverlapping(vptr, stride)` —
// the OLD i64-only read (`old_v.read() / nv.write()`) copied only the first
// 8B (the ptr), leaving len/cap in the new cell as uninitialized heap memory
// → the Drop free would call `free(ptr, garbage_cap)` → SIGABRT or leak.
//
// Poison: in `__triet_hashmap_insert`, replace the rehash loop's
// `copy_nonoverlapping(old_vptr, new_vptr, stride)` with
// `(new_vptr as *mut i64).write_unaligned((old_vptr as *const i64).read_unaligned())`
// → the fat-value MIR test below goes RED (FREE < 4 or SIGABRT).

/// Shim-level: fake fat elements (ptr=tag) survive rehash with stride=24.
#[test]
fn hashmap_rehash_fat_value_retains_full_cell() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Insert 4 → triggers realloc at cap=4, value_stride=24
    let m = mir_lower::__triet_hashmap_alloc(0, 4, 24);
    assert_ne!(m, 0);
    let mut cur = m;
    for (k, tag) in [(1, 101_i64), (2, 202), (3, 303), (4, 404)] {
        let fake = [tag, 5_i64, 8_i64]; // {ptr=tag, len=5, cap=8}
        cur = mir_lower::__triet_hashmap_insert(cur, k, fake.as_ptr() as i64);
    }
    assert_eq!(
        mir_lower::__triet_hashmap_get(cur, 1),
        101,
        "rehash fat: key 1 → ptr 101"
    );
    assert_eq!(
        mir_lower::__triet_hashmap_get(cur, 3),
        303,
        "rehash fat: key 3 → ptr 303"
    );
    assert_eq!(
        mir_lower::__triet_hashmap_get(cur, 4),
        404,
        "rehash fat: key 4 → ptr 404"
    );
    mir_lower::__triet_hashmap_free(cur);
}

/// MIR-level: insert 4 real Strings → rehash triggers → drop → FREE==4.
/// The rehash preserves full 24B value cells, so Drop frees each String.
/// Poison the rehash memcpy→i64 → FREE < 4 or SIGABRT (len/cap garbage).
#[test]
fn hashmap_rehash_fat_value_mir_drop_frees_all() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_insert_drop(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")], None);
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        4,
        "ADR-0078 tooth #3: rehash with 4 fat values → drop frees all 4          (poison rehash memcpy→i64 → FREE < 4 or SIGABRT)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Slice B (typecheck OPEN) — source-level subprocess tooth #1 (SIGABRT 134)
// ═══════════════════════════════════════════════════════════════════════════
//
// These tests use the FULL pipeline (parse → typecheck → lower → JIT) with
// REAL allocator shims (NOT counting stubs). A double-free in the real
// allocator → SIGABRT 134 → subprocess crashes → parent detects non-success.
//
// Tooth #1 (G GOLD: SIGABRT 134 real-allocator):
//   Poison: in `triet-mir/src/lib.rs`, change `__triet_hashmap_insert`'s
//   `arg_consumes` from `&[true, false, true]` to `&[true, false, false]`
//   (value arg NOT consumed). The JIT skips zeroing `s` after the call →
//   both the caller's `Drop(s)` AND the map's `Drop(m2)` free the same String
//   buffer → double-free → exit 134 (SIGABRT). The value MUST be a named
//   local (`let s = "hi"`) — a string literal inline (`insert(m,1,"hi")`)
//   has no drop obligation at the call site → poison is a no-op (VACUOUS).
//   Patched tree: exit 0.

const HM_STRING_SRC: &str = "\
    function main() -> Integer = {\
    \x20   let m: HashMap<Integer, String> = hashmap_new();\
    \x20   let s = \"hi\";\
    \x20   let m2 = insert(m, 1, s);\
    \x20   return 0;\
    }";

fn lower_hm_string_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn hm_string_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // REAL free, NOT the counting stub — double-free → SIGABRT 134.
        ShimSymbol::fn_2_0("__triet_string_free", mir_lower::__triet_string_free),
    ]
}

fn jit_run_real_shims(body: &triet_mir::Body) -> i64 {
    body.verify().expect("MIR verify");
    let mut ctx = JitContext::with_shims(&hm_string_shims());
    let func = ctx.compile(body).expect("must JIT-compile");
    unsafe { func.call_i64_0() }
}

fn hm_child_guard(test_name: &str, child_fn: impl FnOnce()) {
    if let Ok(name) = std::env::var("_TRIET_HM_STRING") {
        if name == test_name {
            child_fn();
        }
        std::process::exit(0);
    }
}

fn spawn_hm_child(test_name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env("_TRIET_HM_STRING", test_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
}

/// Tooth #1 (G GOLD): HashMap<Integer,String> insert→drop with REAL allocator.
/// Child exits cleanly → insert Value-consume is sound. Poison the consume
/// (`arg_consumes[2] = false`) → double-free in real allocator → SIGABRT 134
/// → child crashes → `success()` fails → RED.
#[test]
fn hashmap_string_insert_drop_real_alloc_sound() {
    hm_child_guard("hashmap_string_insert_drop_real_alloc_sound", || {
        let bodies = lower_hm_string_source(HM_STRING_SRC);
        let r = jit_run_real_shims(&bodies[0]);
        assert_eq!(r, 0, "HashMap<String> insert→drop must return 0");
    });
    let status = spawn_hm_child("hashmap_string_insert_drop_real_alloc_sound");
    assert!(
        status.success(),
        "ADR-0078 tooth #1: HashMap<String> insert→drop with real alloc must exit 0 \
         (poison insert value-consume → double-free → exit 134). Got {status:?}"
    );
}

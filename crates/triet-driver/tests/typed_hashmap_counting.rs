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
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // ADR-0083: struct-key hash/eq walkers reference these for String leaves.
        ShimSymbol::fn_2_1("__triet_string_hash", mir_lower::__triet_string_hash),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
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
    let m = mir_lower::__triet_hashmap_alloc(0, 4, 8, 24, 0, 0);
    assert_ne!(m, 0);
    let mut cur = m;
    for (k, tag) in [(1, 101_i64), (2, 202), (3, 303), (4, 404)] {
        let fake = [tag, 5_i64, 8_i64]; // {ptr=tag, len=5, cap=8}
        cur = mir_lower::__triet_hashmap_insert(cur, k, fake.as_ptr() as i64, 0);
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
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
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

// ═══════════════════════════════════════════════════════════════════════════
// ADR-0080 KM-P1b (source-level, typecheck NOW OPEN for `HashMap<String,V>`)
// ═══════════════════════════════════════════════════════════════════════════
//
// Full pipeline (parse → typecheck → lower → borrowck-implicit-via-JIT →
// JIT). G's priority order: ★SS first, then #4, #6, #8, #9.

/// ★SS (G TOP, mandatory): `HashMap<String,String>` — key AND value are both
/// heap. Insert 1 entry, Drop the map: key-loop (D.1) frees the key, value-
/// loop (ADR-0078) frees the value — independently, no double-count.
/// Poison → RED (apply ONE at a time, per project poison discipline):
/// (a) gut `emit_hashmap_key_free_loop`'s call site → count 2→1 (key leak).
/// (b) gut `emit_hashmap_value_free_loop`'s call site → count 2→1 (value leak).
#[test]
fn hashmap_string_string_insert_drop_frees_key_and_value() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);

    let src = "function main() -> Integer = {\
        \x20   let m: HashMap<String, String> = hashmap_new();\
        \x20   let k = \"alice\";\
        \x20   let v = \"hello\";\
        \x20   let m2 = insert(m, k, v);\
        \x20   return 0;\
        }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    bodies[0].verify().expect("MIR verify");

    let mut ctx = JitContext::with_shims(&shims());
    let func = ctx.compile(&bodies[0]).expect("must JIT-compile");
    let r = unsafe { func.call_i64_0() };
    assert_eq!(r, 0, "HashMap<String,String> insert→drop must return 0");

    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0080 ★SS: HashMap<String,String> insert→drop must free EXACTLY \
         1 key + 1 value (key-loop ∥ value-loop both fire, independently, \
         no leak, no double-count)"
    );
}

/// ★SS (c): remove-then-drop tombstone probe with the REAL allocator.
/// `remove` (D.5) frees the resident key + moves the value out to `removed`;
/// the map is now EMPTY (tombstoned slot). A subsequent map `Drop` must NOT
/// re-free the tombstoned slot's key.
///
/// ⚠ MEASURED (self-poison verified, O: reproduce before trusting): this is
/// DOUBLE defense-in-depth, NOT a single point of failure. D.5 (1) tombstones
/// state→2 AND (2) zeroes the key cell (`std::ptr::write_bytes`) after
/// surfacing it to `key_out_ptr`. Poisoning EITHER alone survives (the other
/// still saves it): removing just the zero-write leaves a stale ptr in the
/// cell, but the key-loop's `is_occ` (state==1 only) still skips state==2 —
/// no re-read. Poisoning just `is_occ` (treat state==2 as occupied too)
/// re-reads the cell, but it was already zeroed → sentinel-no-op R4, still
/// safe. Only poisoning BOTH simultaneously (is_occ AND the zero-write)
/// reaches SIGABRT 134 — verified by hand, not by this single test alone.
/// This test proves the OUTER invariant (no double-free under normal
/// operation); it does not by itself prove exactly which single line is
/// load-bearing, because none is alone.
#[test]
fn hashmap_string_string_remove_then_drop_no_double_free() {
    hm_child_guard(
        "hashmap_string_string_remove_then_drop_no_double_free",
        || {
            let src = "function main() -> Integer = {\
                \x20   let m: HashMap<String, String> = hashmap_new();\
                \x20   let k = \"alice\";\
                \x20   let v = \"hello\";\
                \x20   let m2 = insert(m, k, v);\
                \x20   let k2 = \"alice\";\
                \x20   let removed = remove(m2, k2);\
                \x20   return 0;\
                }";
            let bodies = lower_hm_string_source(src);
            let r = jit_run_real_shims(&bodies[0]);
            assert_eq!(r, 0, "HashMap<String,String> remove→drop must return 0");
        },
    );
    let status = spawn_hm_child("hashmap_string_string_remove_then_drop_no_double_free");
    assert!(
        status.success(),
        "ADR-0080 ★SS(c): remove-then-drop on HashMap<String,String> must exit \
         0 — a poisoned tombstone-skip re-frees the already-freed key/value → \
         SIGABRT 134. Got {status:?}"
    );
}

/// #4 (G priority, SIGABRT gold standard): insert=Move KEY — the key-arg
/// analog of the pre-existing VALUE tooth above. `k` MUST be a named local
/// (drop obligation at the call site) — a string literal inline has none,
/// making the poison vacuous (LUẬT NAMED-LOCAL).
/// Poison: `triet-mir/src/lib.rs` `__triet_hashmap_insert`'s `arg_consumes`
/// `[true, true, true]` → `[true, false, true]` (key NOT consumed) → the
/// JIT skips zeroing `k` after insert → BOTH the caller's `Drop(k)` (via
/// `k`'s own end-of-scope) AND the map's key-free-loop (D.1) free the SAME
/// pointer → double-free → exit 134. Patched tree: exit 0.
#[test]
fn hashmap_string_key_insert_move_sound() {
    hm_child_guard("hashmap_string_key_insert_move_sound", || {
        let src = "function main() -> Integer = {\
            \x20   let m: HashMap<String, Integer> = hashmap_new();\
            \x20   let k = \"hi\";\
            \x20   let m2 = insert(m, k, 1);\
            \x20   return 0;\
            }";
        let bodies = lower_hm_string_source(src);
        let r = jit_run_real_shims(&bodies[0]);
        assert_eq!(r, 0, "HashMap<String,Integer> insert→drop must return 0");
    });
    let status = spawn_hm_child("hashmap_string_key_insert_move_sound");
    assert!(
        status.success(),
        "ADR-0080 tooth #4: HashMap<String,_> insert→drop with real alloc must \
         exit 0 (poison insert key-consume → double-free → exit 134). Got {status:?}"
    );
}

/// #6: lookup-key BORROW — `get`'s key-arg must NOT be consumed, asymmetric
/// with insert's Move (Mũi D point 4). `k2` is used TWICE as the lookup key
/// across two separate `get` calls; if `get` wrongly consumed it (poisoned
/// `arg_consumes[1]=true` on `__triet_hashmap_get`), the SECOND use would be
/// E2420 (use-after-move) at borrowck — this program would stop compiling
/// (`type_errors`/lowering would no longer be clean). Green here proves the
/// borrow model; O flips the meta table to observe the E2420 REFUSE.
#[test]
fn hashmap_string_key_lookup_is_borrow_reusable() {
    let src = "function main() -> Integer = {\
        \x20   let m: HashMap<String, Integer> = hashmap_new();\
        \x20   let k = \"hi\";\
        \x20   let m2 = insert(m, k, 1);\
        \x20   let k2 = \"hi\";\
        \x20   let got1 = get(m2, k2);\
        \x20   let got2 = get(m2, k2);\
        \x20   return 0;\
        }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(
        type_errors.is_empty(),
        "ADR-0080 tooth #6: reusing a lookup key across two `get` calls must \
         typecheck clean (key is a BORROW, not consumed) — got {type_errors:?}"
    );
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    for body in &bodies {
        let result =
            triet_borrowck::checker::check_body_with(body, &std::collections::BTreeMap::new());
        assert!(
            result.is_ok(),
            "ADR-0080 tooth #6: reusing a lookup key across two `get` calls \
             must pass borrowck (key is a BORROW) — got {:?}",
            result.errors
        );
    }
}

/// #8: REFUSE `HashMap<K,V>` for `K ∉ {Integer, String}` — E1048, both
/// variants named in the WO (Tryte, and a user Struct).
#[test]
fn hashmap_key_type_tryte_refused() {
    let src = "function main() -> Integer = { let m: HashMap<Tryte, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        !type_errors.is_empty(),
        "ADR-0080 tooth #8: HashMap<Tryte,_> must be REFUSED at typecheck"
    );
    assert!(
        type_errors.iter().any(|e| e.to_string().contains("E1048")),
        "expected E1048 UnsupportedHashMapKey, got {type_errors:?}"
    );
}

/// ADR-0083 §5 — a Struct whose leaves are all hashable (scalar/String/nested)
/// is now an ACCEPTED HashMap key (was refused under ADR-0080). Repurposed from
/// the old `hashmap_key_type_struct_refused` (LUẬT 3): the guard it protected
/// (Struct-key refuse) was intentionally lifted this ADR — poison
/// `is_hashable_key` to only-Integer/String and this test goes RED (E1048
/// re-fires on Point).
#[test]
fn hashmap_hashable_struct_key_accepted() {
    let src = "struct Point { x: Integer, y: Integer }\n\
        function main() -> Integer = { let m: HashMap<Point, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        !type_errors.iter().any(|e| e.to_string().contains("E1048")),
        "ADR-0083: HashMap<Point,_> (all-scalar Struct key) must be ACCEPTED, got {type_errors:?}"
    );
}

/// ADR-0083 §5 — a Struct key with a NON-hashable leaf (Vector — mutable
/// collection) is REFUSED with E1048. Poison `is_hashable_leaf` (accept
/// everything) → this goes RED (compile succeeds).
#[test]
fn hashmap_nonhashable_struct_key_refused() {
    let src = "struct Bad { items: Vector<Integer> }\n\
        function main() -> Integer = { let m: HashMap<Bad, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        type_errors.iter().any(|e| e.to_string().contains("E1048")),
        "ADR-0083: HashMap<Bad{{Vector}},_> (non-hashable leaf) must be E1048, got {type_errors:?}"
    );
}

/// ADR-0083 §AMEND-1 (Slice 2) — an Enum key whose variant payloads are all
/// hashable (scalar/String/unit) is now ACCEPTED (was refused as Slice-2-
/// deferred under Slice 1). Repurposed from the old `hashmap_enum_key_refused`
/// (LUẬT 3): the guard it protected (Enum-key refuse) was intentionally lifted
/// by §AMEND-1 — poison `is_hashable_key`'s `UserEnum` arm to `false` and this
/// test goes RED (E1048 re-fires on Color).
#[test]
fn hashmap_hashable_enum_key_accepted() {
    let src = "enum Color { Red, Green }\n\
        function main() -> Integer = { let m: HashMap<Color, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        !type_errors.iter().any(|e| e.to_string().contains("E1048")),
        "ADR-0083 §AMEND-1: HashMap<Color,_> (scalar/unit Enum key) must be ACCEPTED, \
         got {type_errors:?}"
    );
}

/// ADR-0083 §AMEND-1 §OUT — a NULLABLE-enum key (`Enum?`) stays REFUSED (the
/// null sentinel collides with the discriminant). Poison: add a `Nullable`
/// unwrap to `is_hashable_key` accepting `Nullable(UserEnum)` → this goes RED.
#[test]
fn hashmap_nullable_enum_key_refused() {
    let src = "enum Color { Red, Green }\n\
        function main() -> Integer = { let m: HashMap<Color?, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        type_errors.iter().any(|e| e.to_string().contains("E1048")),
        "ADR-0083 §AMEND-1 §OUT: HashMap<Color?,_> (nullable-enum key) must be E1048, \
         got {type_errors:?}"
    );
}

/// ADR-0083 §AMEND-1, SUPERSEDED by ADR-0067 §AMEND (Enum-Payload-Aggregate
/// Sizing) — an Enum key whose variant carries an AGGREGATE payload (a
/// Struct/Enum > 8B) used to be REFUSED with E1048: the lowerer sized every
/// enum payload at a fixed 8B (no aggregate-payload fixup), so a nested
/// `Point{x,y}` (16B) was under-sized and the key marshal would silently
/// truncate it. ADR-0067 §AMEND closed that gap with a struct+enum
/// co-fixpoint (`triet-lower/src/lib.rs`) and lifted
/// `Type::is_hashable_enum_payload` to delegate to `is_hashable_leaf` — this
/// exact shape now typechecks clean. Reverting that delegation back to the
/// old scalar/String-only `matches!` would make this test RED again — that
/// IS the regression signal for the lift.
#[test]
fn hashmap_enum_aggregate_payload_key_now_hashable() {
    let src = "struct Point { x: Integer, y: Integer }\n\
        enum Shape { Dot(Point), None }\n\
        function main() -> Integer = { let m: HashMap<Shape, Integer> = hashmap_new(); return 0; }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, _, _) = triet_typecheck::check(&program);
    assert!(
        type_errors.is_empty(),
        "ADR-0067 §AMEND: HashMap<Shape{{Dot(Point)}},_> (aggregate enum payload) is now \
         hashable — expected 0 type errors, got {type_errors:?}"
    );
}

/// #9: `HashMap<Integer,V>` source-level backward-compat — must stay green
/// with K now generic (was hardcoded Integer pre-ADR-0080). Source-level
/// counterpart to the pre-existing hand-built-MIR
/// `hashmap_int_int_insert_get_readback` above.
#[test]
fn hashmap_integer_key_source_compat() {
    let src = "function main() -> Integer = {\
        \x20   let m: HashMap<Integer, Integer> = hashmap_new();\
        \x20   let m2 = insert(m, 1, 100);\
        \x20   let got = get(m2, 1);\
        \x20   return match got {\
        \x20       ~+ x => x,\
        \x20       ~0 => -1,\
        \x20   };\
        }";
    let (program, parse_errors) = triet_parser::parse(src);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(
        type_errors.is_empty(),
        "ADR-0080 tooth #9: HashMap<Integer,Integer> must stay green — {type_errors:?}"
    );
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    bodies[0].verify().expect("MIR verify");
    let mut ctx = JitContext::with_shims(&shims());
    let func = ctx.compile(&bodies[0]).expect("must JIT-compile");
    let r = unsafe { func.call_i64_0() };
    assert_eq!(
        r, 100,
        "HashMap<Integer,Integer> insert/get readback via source"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-0082 B-α Slice C (HashMap<_, aggregate> VALUE) — Mentor O WO, 2026-07-10
// ═══════════════════════════════════════════════════════════════════════════
//
// Scope: insert + Drop + alloc opened for a Struct/Enum VALUE. get/get_ref/
// remove/contains and ANY key-aggregate stay refused. `remove` typechecks
// fine (V is unconstrained for `remove<K,V>`) so it needs an explicit JIT
// refuse — tested source-level in
// `vector_userstruct_counting.rs::hashmap_struct_value_remove_refused_at_jit`
// (that test SUPERSEDES the former `hashmap_struct_value_refused_at_jit`,
// which asserted `insert` refused — Slice C deliberately flips that; see its
// doc comment for the LUẬT 3 repurposing rationale). `get`/`get_ref`/
// `contains` and any key-aggregate are unreachable from source at all
// (verified directly, Luật 4 — see the T5 section below); their guards are
// exercised via hand-built MIR here.

fn run_source(source: &str, shim_list: &[ShimSymbol]) -> i64 {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shim_list);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

fn shims_with_vector() -> Vec<ShimSymbol> {
    let mut v = shims();
    v.push(ShimSymbol::fn_3_1(
        "__triet_vector_alloc",
        mir_lower::__triet_vector_alloc,
    ));
    v.push(ShimSymbol::fn_1_0(
        "__triet_vector_free",
        mir_lower::__triet_vector_free,
    ));
    v.push(ShimSymbol::fn_2_1(
        "__triet_vector_push",
        mir_lower::__triet_vector_push,
    ));
    v
}

/// T1 (F1): `HashMap<Integer, User>` — a FAT (>8B) Struct value — insert 2,
/// drop → each element's String field freed exactly once. Poison F1 (revert
/// `emit_hashmap_value_free_loop`'s guard to `is_any_heap()`) → 0 (leak).
#[test]
fn hashmap_struct_value_insert_drop_frees_string_field() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, User> = hashmap_new();\n\
         \x20   let a = User { name: \"aa\" };\n\
         \x20   m = insert(m, 1, a);\n\
         \x20   let b = User { name: \"bb\" };\n\
         \x20   m = insert(m, 2, b);\n\
         \x20   return 0;\n\
         }",
        &shims(),
    );
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0082 Slice C: HashMap<_,User> drop must free each VALUE's String \
         field exactly once (== 0 ⇒ F1 regressed to is_any_heap() → leak)"
    );
}

/// T2 (F1 + F3-ĐẦU-A): `HashMap<Integer, Msg>` — a FAT heap-payload Enum
/// value. Insert 2, drop → each variant's String freed exactly once.
/// Poison F1 → 0 (leak). Poison F3-ĐẦU-A (remove the `enum_slots` branch
/// from the fat (>8B) value marshal, added this slice) → COMPILE-FAILS
/// (`hashmap_insert: fat value without a slot`) — a fat Enum value has no
/// entry in `struct_slots`, only `enum_slots`.
#[test]
fn hashmap_enum_value_insert_drop_frees_string_payload() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, Msg> = hashmap_new();\n\
         \x20   m = insert(m, 1, Msg::Text(\"aa\"));\n\
         \x20   m = insert(m, 2, Msg::Text(\"bb\"));\n\
         \x20   return 0;\n\
         }",
        &shims(),
    );
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0082 Slice C: HashMap<_,Msg> drop must free each ACTIVE variant's \
         String payload exactly once (== 0 ⇒ F1 leak; a compile error here ⇒ \
         F3-ĐẦU-A regressed — fat Enum value marshal missing enum_slots)"
    );
}

/// T3 (F3-ĐẦU-B, ⚔ O-added): `HashMap<Integer, Wrapper>` — an 8B-aggregate
/// value (`Wrapper` wraps exactly ONE `Vector<String>` handle, total_size==8
/// — a struct-slot-backed local, NOT a Cranelift Variable). Insert once,
/// drop → the WRAPPED Vector's 2 String elements freed (recursing through
/// the value's own drop-glue). Poison F3-ĐẦU-B (drop the `struct_slots`
/// `stack_load` branch in the `value_stride <= 8` marshal, falling back to
/// `use_var`) → the pushed handle reads 0/garbage → the wrapped Vector is
/// never reached by Drop → LEAK (0), reviving the exact Slice A/B C5/T9 bug
/// at the HashMap insert site.
#[test]
fn hashmap_8b_struct_value_insert_drop_frees_wrapped_vector() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "struct Wrapper { v: Vector<String> }\n\
         function main() -> Integer = {\n\
         \x20   let mutable inner: Vector<String> = vector_new();\n\
         \x20   inner = push(inner, \"x\");\n\
         \x20   inner = push(inner, \"y\");\n\
         \x20   let w = Wrapper { v: inner };\n\
         \x20   let mutable m: HashMap<Integer, Wrapper> = hashmap_new();\n\
         \x20   m = insert(m, 1, w);\n\
         \x20   return 0;\n\
         }",
        &shims_with_vector(),
    );
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0082 Slice C: HashMap<_,Wrapper(8B)> drop must recurse into the \
         wrapped Vector<String> and free both elements (== 0 ⇒ F3-ĐẦU-B \
         regressed — 8B value marshal read a garbage handle via use_var)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-0082 B-α Slice D-2 (HashMap<K,aggregate> remove by-value) — Mentor O.
// `remove` owns the value it returns (move-out, mirrors `vector_pop`'s
// D-1b) — VALUE aggregate opens here (key-aggregate stays refused, see
// `hashmap_struct_key_remove_refused_at_jit` below). The tag-marshal
// (Struct? tag@0/fields@+8, Enum? disc@0) is SHARED with `vector_pop` at
// the JIT dest-bind — D-2 only needed the out_ptr field_off fix + refuse
// narrowing (K+V → K-only) at the 2 `remove` call-sites.
//
// G-MANDATE state-gate: the value cell `remove` moves out is NEVER zeroed
// by the shim (unlike the resident KEY, which IS zeroed after surfacing) —
// soundness relies ENTIRELY on the tombstone `state → 2` write
// (mir_lower.rs ~5373) paired with the value-free-loop's `state_byte == 1`
// gate (~1441) to skip a removed cell on the map's OWN later Drop. Both
// halves are poison-verified below (GATE-A: neuter the gate to also match
// state==2 · GATE-B: skip the tombstone write) — either alone independently
// turns `hashmap_struct_value_remove_then_drop_no_double_free` red (FREE
// measured 3, not 2 — NOT the naive "2×N" shape; see that test's doc for
// why) or crashes outright (the T9 8B Wrapper sibling below, real
// `__triet_vector_free` double-free → SIGABRT).
// ═══════════════════════════════════════════════════════════════════════════

/// GATE-A/B anchor: `HashMap<Integer,User>` (fat Struct value) — insert 2,
/// remove 1 (caller drops the removed value via match+scope-end), drop the
/// map (1 survivor). Each String freed exactly once → 2. Poison the
/// state-gate (GATE-A: is_occ also matches state==2, OR GATE-B: tombstone
/// write skipped) → the removed cell's value is freed AGAIN by the map's
/// own Drop → measured 3, not 4 (map-drop frees BOTH the tombstoned "aa"
/// cell and the still-occupied "bb" survivor = 2, PLUS the caller's own
/// Drop of the removed "aa" = 1 more = 3 total; poison-verified on the
/// real tree, both GATE-A and GATE-B independently produce this same 3 —
/// not the naive "2×N" double-free shape of the scalar-String precedent
/// `hashmap_string_remove_then_drop_no_double_free` above, which removes
/// the ENTIRE map's only occupant instead of leaving a survivor).
#[test]
fn hashmap_struct_value_remove_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, User> = hashmap_new();\n\
         \x20   let a = User { name: \"aa\" };\n\
         \x20   m = insert(m, 1, a);\n\
         \x20   let b = User { name: \"bb\" };\n\
         \x20   m = insert(m, 2, b);\n\
         \x20   let out = remove(m, 1);\n\
         \x20   return match out {\n\
         \x20       ~+ u => 0,\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }",
        &shims(),
    );
    assert_eq!(r, 0, "main returns 0 (~+ arm taken, key 1 present)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "removed value (\"aa\") + survivor (\"bb\") — each String freed \
         exactly once. == 3 ⇒ state-gate breach (GATE-A/B) double-frees the \
         removed cell on map-drop (poison-measured, NOT a naive 2×N shape); \
         == 0/1 ⇒ tag/field marshal leak."
    );
}

/// Enum value sibling of the GATE-A/B anchor above.
#[test]
fn hashmap_enum_value_remove_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, Msg> = hashmap_new();\n\
         \x20   m = insert(m, 1, Msg::Text(\"aa\"));\n\
         \x20   m = insert(m, 2, Msg::Text(\"bb\"));\n\
         \x20   let out = remove(m, 1);\n\
         \x20   return match out {\n\
         \x20       ~+ msg => 0,\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }",
        &shims(),
    );
    assert_eq!(r, 0, "main returns 0 (~+ arm taken, key 1 present)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "removed value (\"aa\") + survivor (\"bb\") — each String freed \
         exactly once. == 3 ⇒ state-gate breach double-free (poison-\
         measured); == 0/1 ⇒ tag/field marshal leak (enum_slots out_ptr \
         branch)."
    );
}

/// T9 8B-path sibling: `Wrapper{v:Vector<String>}` (total_size==8) rides the
/// scalar-return T9 path at the dest-bind (shared with `vector_pop`). Remove
/// the only entry, match `~+`, drop the downcast copy — its inner
/// `Vector<String>` must free through the REAL `__triet_vector_free`
/// (`shims_with_vector`), independent of the `__triet_string_free` stub.
#[test]
fn hashmap_8b_wrapper_value_remove_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "struct Wrapper { v: Vector<String> }\n\
         function main() -> Integer = {\n\
         \x20   let mutable inner: Vector<String> = vector_new();\n\
         \x20   inner = push(inner, \"zz\");\n\
         \x20   let w = Wrapper { v: inner };\n\
         \x20   let mutable m: HashMap<Integer, Wrapper> = hashmap_new();\n\
         \x20   m = insert(m, 1, w);\n\
         \x20   let out = remove(m, 1);\n\
         \x20   return match out {\n\
         \x20       ~+ popped => 0,\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }",
        &shims_with_vector(),
    );
    assert_eq!(
        r, 0,
        "main returns 0 (~+ arm taken — tag correctly present)"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "removed Wrapper's inner Vector<String> must free its element \
         exactly once. == 0 ⇒ T9 field@+8 write missing/wrong (leaked/\
         garbage handle). A GATE-A/B state-gate breach does NOT show up as \
         a wrong STR_FREES count here (this map has no survivor, only the \
         one removed/tombstoned cell) — poison-measured, it SIGABRTs \
         instead (\"free(): double free detected in tcache\") via the REAL \
         `__triet_vector_free` (shims_with_vector) double-freeing the \
         Vector handle: caller drops the popped Wrapper's Vector once, \
         map-drop's state-gate breach frees the same stale handle again."
    );
}

/// String-key sibling: `HashMap<String,User>` remove must free BOTH the
/// resident KEY (ADR-0080 §AMEND-1 D.5, unaffected by D-2 — key stays a
/// scalar String even though the value is now an aggregate) and the
/// removed VALUE's String field, with no interference between the two
/// free paths.
#[test]
fn hashmap_string_key_struct_value_remove_frees_key_and_value() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_source(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<String, User> = hashmap_new();\n\
         \x20   let a = User { name: \"aa\" };\n\
         \x20   m = insert(m, \"k\", a);\n\
         \x20   let out = remove(m, \"k\");\n\
         \x20   return match out {\n\
         \x20       ~+ u => 0,\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }",
        &shims(),
    );
    assert_eq!(r, 0, "main returns 0 (~+ arm taken)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "resident key (\"k\") freed via D.5 out-param registry-route + \
         removed value's String field (\"aa\") freed via caller Drop — 2 \
         independent String frees, no map survivors left. == 3 ⇒ GATE-A/B \
         state-gate breach re-frees the tombstoned cell's stale value on \
         map-drop (poison-measured)."
    );
}

// ── T5 (F4): still-refused ops — get/get_ref/contains/key-aggregate ──
//
// `get`/`get_ref`/`contains` have NO overload for a Struct/Enum VALUE
// (`declare_overload`, a fixed enumerated set) — a source-level call is a
// TYPE ERROR (`NoMatchingOverload`), never reaching the JIT. Any HashMap
// with a Struct/Enum KEY is refused earlier still, at the type ANNOTATION
// itself (E1048 UnsupportedHashMapKey). Both boundaries were probed directly
// (Luật 4 — a throwaway typecheck-only probe, since removed) before writing
// these teeth. The JIT-side F4 guards for these 5 call-sites are therefore
// DEFENSE-IN-DEPTH, not reachable from valid source — exercised here via
// hand-built MIR (the same authorized path Slice A/ADR-0078 used before the
// frontend opened `HashMap<Integer,T>`).

fn user_struct_layout() -> triet_mir::StructLayout {
    triet_mir::StructLayout::compute("User", &[("name".to_string(), MirType::String, 24, 8)])
}

fn hashmap_iu_ty() -> MirType {
    MirType::HashMap(
        Box::new(MirType::Integer),
        Box::new(MirType::Struct("User".to_string())),
    )
}

fn hashmap_ku_ty() -> MirType {
    MirType::HashMap(
        Box::new(MirType::Struct("User".to_string())),
        Box::new(MirType::Integer),
    )
}

/// Build a minimal body: `storage_live(map_local: map_ty)` (never actually
/// populated by a real `alloc` — `compile()` only LOWERS the body, never
/// executes it, so the refuse guard is reached purely from `map_local`'s
/// declared TYPE) → ONE shim call `callee(map_local, key_local[, value_local])`.
/// Isolates a single call-site's guard without needing a working alloc first.
/// `key_is_struct` declares `key_loc` as `Struct("User")` (matching a
/// key-aggregate `map_ty`) instead of an `Integer` const — without this, a
/// poisoned key-aggregate guard would still (correctly, but for the WRONG
/// reason) hit the marshal's UNRELATED "fat key without a slot" Err, because
/// a plain Integer const has no `struct_slots` entry — masking whether the
/// intended guard fired at all.
fn build_single_hashmap_call(
    map_ty: MirType,
    callee: &str,
    key_is_struct: bool,
    value_arg: Option<i64>,
) -> triet_mir::Body {
    let mut b = MirBuilder::new("main", MirType::Integer);
    b.add_struct_layout(user_struct_layout());
    let bb = b.new_block();
    let map_local = b.new_local();
    b.set_local_mir_type(map_local, map_ty);
    b.push(bb, storage_live(map_local));
    let key_loc = b.new_local();
    b.push(bb, storage_live(key_loc));
    if key_is_struct {
        b.set_local_mir_type(key_loc, MirType::Struct("User".to_string()));
    } else {
        b.push(bb, const_i(key_loc, 1));
    }
    let mut args = vec![map_local, key_loc];
    if let Some(v) = value_arg {
        let val_loc = b.new_local();
        b.push(bb, storage_live(val_loc));
        b.push(bb, const_i(val_loc, v.into()));
        args.push(val_loc);
    }
    let out = b.new_local();
    b.push(bb, storage_live(out));
    let next = b.new_block();
    b.set_terminator(bb, shim_call(callee, args, vec![out], next));
    b.set_terminator(
        next,
        Terminator::Return {
            values: vec![out],
            span: DUMMY_SPAN,
        },
    );
    b.build(bb)
}

/// Build a minimal body that calls `__triet_hashmap_alloc(len, cap) -> map_ty`
/// — isolates the ALLOC call-site's guard (which reads `dest[0]`'s type, not
/// an arg type).
fn build_hashmap_alloc(map_ty: MirType) -> triet_mir::Body {
    let mut b = MirBuilder::new("main", MirType::Integer);
    b.add_struct_layout(user_struct_layout());
    let bb = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    b.push(bb, storage_live(len0));
    b.push(bb, const_i(len0, 0));
    b.push(bb, storage_live(cap0));
    b.push(bb, const_i(cap0, 4));
    let map_local = b.new_local();
    b.set_local_mir_type(map_local, map_ty);
    b.push(bb, storage_live(map_local));
    let next = b.new_block();
    b.set_terminator(
        bb,
        shim_call(
            "__triet_hashmap_alloc",
            vec![len0, cap0],
            vec![map_local],
            next,
        ),
    );
    b.set_terminator(
        next,
        Terminator::Return {
            values: vec![len0],
            span: DUMMY_SPAN,
        },
    );
    b.build(bb)
}

/// Compile `body` expecting a HARD `JitError` refuse. `extra_shims` MUST
/// register whichever callee the body's single call targets — else the
/// error would be a spurious "unknown shim" (a VACUOUS refuse, insensitive
/// to the guard under test).
fn expect_jit_refuse_mir(body: &triet_mir::Body, extra_shims: &[ShimSymbol]) -> String {
    let mut all_shims = shims();
    all_shims.extend_from_slice(extra_shims);
    let mut ctx = JitContext::with_shims(&all_shims);
    match ctx.compile(body) {
        Ok(_) => {
            panic!(
                "expected a JitError refuse, but compilation SUCCEEDED (silent leak/corruption risk)"
            )
        }
        Err(e) => format!("{e:?}"),
    }
}

#[test]
fn hashmap_struct_value_get_refused_at_jit() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_single_hashmap_call(hashmap_iu_ty(), "__triet_hashmap_get", false, None);
    let err = expect_jit_refuse_mir(&body, &[]);
    assert!(
        err.contains("Slice C") || err.contains("aggregate"),
        "HashMap<_,User> get must refuse with the Slice-C boundary message, got: {err}"
    );
}

#[test]
fn hashmap_struct_value_get_ref_refused_at_jit() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_single_hashmap_call(hashmap_iu_ty(), "__triet_hashmap_get_ref", false, None);
    let err = expect_jit_refuse_mir(
        &body,
        &[ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref",
            mir_lower::__triet_hashmap_get_ref,
        )],
    );
    assert!(
        err.contains("Slice C") || err.contains("aggregate"),
        "HashMap<_,User> get_ref must refuse with the Slice-C boundary message, got: {err}"
    );
}

#[test]
fn hashmap_struct_value_contains_refused_at_jit() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_single_hashmap_call(hashmap_iu_ty(), "__triet_hashmap_contains", false, None);
    let err = expect_jit_refuse_mir(
        &body,
        &[ShimSymbol::fn_2_1(
            "__triet_hashmap_contains",
            mir_lower::__triet_hashmap_contains,
        )],
    );
    assert!(
        err.contains("Slice C") || err.contains("aggregate"),
        "HashMap<_,User> contains must refuse with the Slice-C boundary message, got: {err}"
    );
}

/// ADR-0083 §2/§3 — a hashable Struct key (`User{name:String}`) is OPENED:
/// `alloc` emits the hash/eq walkers and passes their addresses instead of
/// refusing. Repurposed from `hashmap_struct_key_alloc_refused_at_jit`
/// (LUẬT 3): the struct-key refuse it protected is intentionally lifted this
/// ADR — poison the open (re-add `refuse_hashmap_enum_key`→refuse Struct, or
/// make `collect_key_leaves` Err) → compile fails → RED.
#[test]
fn hashmap_struct_key_alloc_compiles() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_hashmap_alloc(hashmap_ku_ty());
    let mut ctx = JitContext::with_shims(&shims());
    ctx.compile(&body)
        .expect("ADR-0083: hashable Struct-key alloc must compile (walkers emitted, no refuse)");
}

/// ADR-0083 — struct-key `insert` compiles (key marshalled by-pointer, walkers
/// referenced). Repurposed from `hashmap_struct_key_insert_refused_at_jit`.
#[test]
fn hashmap_struct_key_insert_compiles() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body =
        build_single_hashmap_call(hashmap_ku_ty(), "__triet_hashmap_insert", true, Some(100));
    let mut ctx = JitContext::with_shims(&shims());
    ctx.compile(&body)
        .expect("ADR-0083: hashable Struct-key insert must compile");
}

/// ADR-0083 — struct-key `remove` compiles (key surfaced via `key_out_ptr`
/// sized to `key_stride`, recursive resident-key free). Repurposed from
/// `hashmap_struct_key_remove_refused_at_jit`.
#[test]
fn hashmap_struct_key_remove_compiles() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = build_single_hashmap_call(hashmap_ku_ty(), "__triet_hashmap_remove", true, None);
    let mut ctx = JitContext::with_shims(&shims());
    ctx.compile(&body)
        .expect("ADR-0083: hashable Struct-key remove must compile");
}

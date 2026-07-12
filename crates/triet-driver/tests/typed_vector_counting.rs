//! ADR-0077 Typed Vector P1 — Slice A backend free-count teeth, via HAND-BUILT
//! MIR (typecheck is NOT yet open for `Vector<String>` — that is Slice B, so
//! `push(vector_new(), "hi")` is E1003 at source and route-lower is impossible
//! here; the WO authorizes hand-built MIR for Slice A).
//!
//! These pin the storage + ownership machinery of MŨI 2/3/4:
//!   - MŨI 2 stride-in-header: alloc(len, cap, stride=24) for String elements;
//!     push/get/free read stride from the header.
//!   - MŨI 3 typed drop-glue: `Drop(vec)` emits a JIT element-free loop that
//!     calls `__triet_string_free` PER ELEMENT (through the shim registry, so
//!     the counting stub sees them — a Rust-internal shim loop would not), then
//!     `__triet_vector_free` once for the buffer.
//!   - MŨI 4 by-pointer ABI: `push` passes a fat String element by-pointer
//!     (memcpy 24B); `pop` moves the last element out (len--), the popped
//!     element is owned by the caller (no double-free, no mid-array hole).
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - #1 leak: push N String → drop → STR_FREE == N. Poison the JIT element-
//!     free loop (`emit_vector_element_free_loop`) → 0 (leak).
//!   - #2 double-free (G mandate): push 2 → pop 1 → drop vec + drop popped →
//!     STR_FREE == 2. Poison pop's `len--` → vec drops 2 + popped 1 = 3 (or a
//!     double-free of the popped element).
//!   - #5 nested: Vector<Vector<String>> → drop → inner String freed (recursion
//!     through `emit_heap_free_at`'s Vector branch).
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
/// guard so only LIVE element frees count).
#[unsafe(no_mangle)]
extern "C" fn __tv_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

fn shims() -> Vec<ShimSymbol> {
    vec![
        // alloc/push/get/pop/buffer-free are REAL (actually allocate + dealloc).
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
        ShimSymbol::fn_2_1(
            "__triet_vector_pop_front",
            mir_lower::__triet_vector_pop_front,
        ),
        ShimSymbol::fn_2_1("__triet_vector_get", mir_lower::__triet_vector_get),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // Element free is the COUNTING stub (the teeth surface).
        ShimSymbol::fn_2_0("__triet_string_free", __tv_str_free),
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

fn vec_string_ty() -> MirType {
    MirType::Vector(Box::new(MirType::String))
}

/// Shim-call terminator (the `call_dispatch` helper hardcodes `CallTarget::Jit`;
/// builtin shims need `CallTarget::Shim`).
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
        .expect("typed-vector body must JIT-compile");
    unsafe { func.call_i64_0() }
}

/// Build `main()` that allocs a `Vector<String>`, pushes `words`, optionally
/// pops `pop_n` (dropping the popped strings), then drops the vector. Returns 0.
fn build_push_pop_drop(words: &[&str], pop_n: usize, pop_shim: &str) -> triet_mir::Body {
    let mut b = MirBuilder::new("main", MirType::Integer);
    b.add_struct_layout(string_layout());

    // bb0: const len=0, cap=2, then alloc.
    let bb_alloc = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    let mut vec_local = b.new_local();
    b.set_local_mir_type(vec_local, vec_string_ty());
    b.push(bb_alloc, storage_live(len0));
    b.push(bb_alloc, const_i(len0, 0));
    b.push(bb_alloc, storage_live(cap0));
    b.push(bb_alloc, const_i(cap0, 2));
    b.push(bb_alloc, storage_live(vec_local));

    let mut cur = bb_alloc;
    let mut next = b.new_block();
    b.set_terminator(
        cur,
        shim_call(
            "__triet_vector_alloc",
            vec![len0, cap0],
            vec![vec_local],
            next,
        ),
    );
    cur = next;

    // Push each word: build a String const, push it (consumes vec → new handle).
    for w in words {
        let s = b.new_local();
        b.set_local_mir_type(s, MirType::String);
        b.push(cur, storage_live(s));
        b.push(
            cur,
            Statement::Const {
                dest: Place::local(s),
                value: ConstValue::String((*w).to_string()),
                span: DUMMY_SPAN,
            },
        );
        let new_vec = b.new_local();
        b.set_local_mir_type(new_vec, vec_string_ty());
        b.push(cur, storage_live(new_vec));
        next = b.new_block();
        b.set_terminator(
            cur,
            shim_call(
                "__triet_vector_push",
                vec![vec_local, s],
                vec![new_vec],
                next,
            ),
        );
        vec_local = new_vec;
        cur = next;
    }

    // Pop `pop_n` elements (each into a String e), and Drop the popped string
    // (the caller now owns it). The vector is mutated in place (len--).
    for _ in 0..pop_n {
        let e = b.new_local();
        b.set_local_mir_type(e, MirType::String);
        b.push(cur, storage_live(e));
        next = b.new_block();
        b.set_terminator(cur, shim_call(pop_shim, vec![vec_local], vec![e], next));
        cur = next;
        b.push(cur, Statement::Drop(e, DUMMY_SPAN));
    }

    // Drop the vector (frees survivors + buffer), return 0.
    b.push(cur, Statement::Drop(vec_local, DUMMY_SPAN));
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
    b.build(bb_alloc)
}

fn const_i(dest: Local, value: i128) -> Statement {
    Statement::Const {
        dest: Place::local(dest),
        value: ConstValue::Integer(value),
        span: DUMMY_SPAN,
    }
}

/// Build `main()` for a nested `Vector<Vector<String>>`: two inner vectors each
/// holding ONE String, pushed into an outer vector, then the outer is dropped.
/// The outer's elements are 8B inner-vector handles (by-value push); dropping
/// the outer recurses through `emit_heap_free_at`'s Vector branch → each inner
/// vector frees its String element + buffer. STR_FREE must == 2.
fn build_nested_vec_vec_string() -> triet_mir::Body {
    let inner_ty = MirType::Vector(Box::new(MirType::String));
    let outer_ty = MirType::Vector(Box::new(inner_ty.clone()));
    let mut b = MirBuilder::new("main", MirType::Integer);
    b.add_struct_layout(string_layout());

    let bb = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    b.push(bb, storage_live(len0));
    b.push(bb, const_i(len0, 0));
    b.push(bb, storage_live(cap0));
    b.push(bb, const_i(cap0, 2));

    // Helper closure-like inline: build an inner Vector<String> with one string.
    let mut cur = bb;
    let build_inner = |b: &mut MirBuilder, cur: &mut triet_mir::BasicBlock, word: &str| -> Local {
        let iv = b.new_local();
        b.set_local_mir_type(iv, inner_ty.clone());
        b.push(*cur, storage_live(iv));
        let next = b.new_block();
        b.set_terminator(
            *cur,
            shim_call("__triet_vector_alloc", vec![len0, cap0], vec![iv], next),
        );
        *cur = next;
        let s = b.new_local();
        b.set_local_mir_type(s, MirType::String);
        b.push(*cur, storage_live(s));
        b.push(
            *cur,
            Statement::Const {
                dest: Place::local(s),
                value: ConstValue::String(word.to_string()),
                span: DUMMY_SPAN,
            },
        );
        let iv2 = b.new_local();
        b.set_local_mir_type(iv2, inner_ty.clone());
        b.push(*cur, storage_live(iv2));
        let next2 = b.new_block();
        b.set_terminator(
            *cur,
            shim_call("__triet_vector_push", vec![iv, s], vec![iv2], next2),
        );
        *cur = next2;
        iv2
    };

    let inner1 = build_inner(&mut b, &mut cur, "inner-a");
    let inner2 = build_inner(&mut b, &mut cur, "inner-b");

    // outer = alloc Vector<Vector<String>>
    let outer = b.new_local();
    b.set_local_mir_type(outer, outer_ty.clone());
    b.push(cur, storage_live(outer));
    let next = b.new_block();
    b.set_terminator(
        cur,
        shim_call("__triet_vector_alloc", vec![len0, cap0], vec![outer], next),
    );
    cur = next;

    // push inner1, inner2 into outer (8B handle elements, by-value).
    let outer1 = b.new_local();
    b.set_local_mir_type(outer1, outer_ty.clone());
    b.push(cur, storage_live(outer1));
    let next = b.new_block();
    b.set_terminator(
        cur,
        shim_call(
            "__triet_vector_push",
            vec![outer, inner1],
            vec![outer1],
            next,
        ),
    );
    cur = next;

    let outer2 = b.new_local();
    b.set_local_mir_type(outer2, outer_ty.clone());
    b.push(cur, storage_live(outer2));
    let next = b.new_block();
    b.set_terminator(
        cur,
        shim_call(
            "__triet_vector_push",
            vec![outer1, inner2],
            vec![outer2],
            next,
        ),
    );
    cur = next;

    b.push(cur, Statement::Drop(outer2, DUMMY_SPAN));
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

// ── Teeth #5 — nested Vector<Vector<String>> ──

#[test]
fn nested_vector_vector_string_drop_frees_inner_strings() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_nested_vec_vec_string();
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0077 nested P1: drop Vector<Vector<String>> recurses → each inner \
         vector's String freed once (poison the loop → inner Strings leak → 0)"
    );
}

// ── Teeth #3 — stride: push/get at element stride 24 (shim-level) ──
// A direct shim test (no JIT): alloc a Vector<String> buffer (stride 24), push
// 3 fake fat elements {ptr=tag,len,cap} by-pointer, and read each back via get.
// `get(v, i)` returns the element's first word (ptr) at `data + i*stride`. If
// the stride were wrongly 8, get(v,1)/get(v,2) would read mid-element garbage.
// The fake ptrs (111/222/333) are never dereferenced (only buffer is freed).

#[test]
fn stride_24_push_get_readback() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = mir_lower::__triet_vector_alloc(0, 4, 24);
    for tag in [111_i64, 222, 333] {
        let elem: [i64; 3] = [tag, 5, 8]; // {ptr=tag, len=5, cap=8}
        v = mir_lower::__triet_vector_push(v, elem.as_ptr() as i64);
    }
    assert_eq!(
        mir_lower::__triet_vector_get(v, 0),
        111,
        "element 0 ptr @ 0*24"
    );
    assert_eq!(
        mir_lower::__triet_vector_get(v, 1),
        222,
        "element 1 ptr @ 1*24"
    );
    assert_eq!(
        mir_lower::__triet_vector_get(v, 2),
        333,
        "element 2 ptr @ 2*24"
    );
    // Free the BUFFER only (fake element ptrs are not real allocations).
    mir_lower::__triet_vector_free(v);
}

// ── Teeth #4 — Vector<Point> (Copy struct) COMPILES at JIT (ADR-0082 B-α) ──
// ADR-0082 B-α (Slice A) moves the P1/P2 boundary `vector_of_userstruct_
// refused_at_jit` used to pin: `vector_elem_size` now resolves a real stride
// for `Struct` from `body.struct_layouts` (INV-B-α — same layout as the
// StackSlot repr) instead of refusing unconditionally. `Vector<UserStruct>`
// is exactly the Slice A target, so a by-value Copy-struct element (no heap
// leaf) must now compile — this replaces the old negative assertion.
// `Enum`/`Capability`/`Outcome` elements are UNCHANGED refuse (Slice B+, not
// this test's concern). Poison: revert the `Struct` arm of `vector_elem_size`
// to `Err` → this body would fail to compile again.

#[test]
fn vector_of_copy_struct_compiles_at_jit() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = MirBuilder::new("main", MirType::Integer);
    // ADR-0082 INV-B-α: `vector_elem_size`'s Struct arm looks up `Point` in
    // `body.struct_layouts` — same layout a StackSlot-backed `Point` local
    // would use.
    b.add_struct_layout(triet_mir::StructLayout::compute(
        "Point",
        &[
            ("x".to_string(), MirType::Integer, 8, 8),
            ("y".to_string(), MirType::Integer, 8, 8),
        ],
    ));
    let bb = b.new_block();
    let len0 = b.new_local();
    let cap0 = b.new_local();
    let v = b.new_local();
    b.set_local_mir_type(
        v,
        MirType::Vector(Box::new(MirType::Struct("Point".to_string()))),
    );
    b.push(bb, storage_live(len0));
    b.push(bb, const_i(len0, 0));
    b.push(bb, storage_live(cap0));
    b.push(bb, const_i(cap0, 2));
    b.push(bb, storage_live(v));
    let ret = b.new_block();
    b.set_terminator(
        bb,
        shim_call("__triet_vector_alloc", vec![len0, cap0], vec![v], ret),
    );
    let result = b.new_local();
    b.push(ret, storage_live(result));
    b.push(ret, const_i(result, 0));
    b.set_terminator(
        ret,
        Terminator::Return {
            values: vec![result],
            span: DUMMY_SPAN,
        },
    );
    let body = b.build(bb);

    body.verify().expect("MIR verify (structurally valid)");
    let mut ctx = JitContext::with_shims(&shims());
    ctx.compile(&body)
        .expect("Vector<Point> (Copy struct, ADR-0082 B-α Slice A) must compile at JIT");
}

// ── Teeth #1 — push N String → drop → STR_FREE == N ──

#[test]
fn vector_string_drop_frees_each_element() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_push_pop_drop(&["alpha", "beta", "gamma"], 0, "__triet_vector_pop");
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "ADR-0077: Vector<String> drop frees EACH element once (poison the JIT \
         element-free loop → 0 = leak)"
    );
}

#[test]
fn vector_string_empty_drop_frees_nothing() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let body = build_push_pop_drop(&[], 0, "__triet_vector_pop");
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "ADR-0077: empty Vector<String> drop frees nothing (len=0 → loop body \
         never runs, buffer-free only)"
    );
}

// ── Teeth #2 (G mandate) — push → pop → drop → no double-free ──

#[test]
fn vector_string_pop_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    // push 3, pop 1 (popped string dropped by caller), drop vector (2 survivors).
    // Total LIVE frees = 2 survivors + 1 popped = 3. A broken pop `len--` would
    // make the vector drop 3 AND the popped element double-free → 4 / SIGABRT.
    let body = build_push_pop_drop(&["a", "b", "c"], 1, "__triet_vector_pop");
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "ADR-0077 G mandate: push 3 → pop 1 → drop → exactly 3 frees \
         (2 survivors + 1 moved-out popped; no double-free)"
    );
}

#[test]
fn vector_string_pop_front_then_drop_no_double_free() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    // ADR-0082 permanent leak/double-free net for pop_front's NEW shift code
    // (the fixture harness catches crashes + wrong values, but a leak is
    // SILENT there — only this counting net observes FREE count). push 3,
    // pop_front 1 (popped string dropped by caller), drop vector (2
    // survivors). Total LIVE frees = 2 survivors + 1 moved-out = 3.
    //   - Dropping the shim's `len--` (B3 tombstone) → vector drops 3 AND the
    //     popped element double-frees → 4 / SIGABRT.
    //   - A broken B2 shift that dropped an element (or failed to move a
    //     survivor's handle down) → FREE != 3 (leak or double-free).
    let body = build_push_pop_drop(&["a", "b", "c"], 1, "__triet_vector_pop_front");
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        3,
        "ADR-0082: push 3 → pop_front 1 → drop → exactly 3 frees \
         (2 shifted survivors + 1 moved-out front; no leak, no double-free)"
    );
}

#[test]
fn vector_string_pop_all_then_drop() {
    let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    // push 2, pop 2 (both dropped by caller), drop empty vector → 2 frees total.
    let body = build_push_pop_drop(&["x", "y"], 2, "__triet_vector_pop");
    let r = jit_run(&body);
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0077: pop ALL → vector drops 0 survivors, 2 popped freed by caller"
    );
}

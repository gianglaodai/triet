//! Phase 2 (ADR-0070 read-side) — route-lower free-count teeth for a heap-owning
//! STRUCT field moved OUT of its parent:
//!
//!     let i = Inner { name: "Giang", n: 7 };
//!     let h = Holder { inner: i, tag: 5 };
//!     let m = h.inner;     // move the whole nested struct OUT
//!     return m.n;
//!
//! `m` takes ownership of Inner (and its String). The base `h.inner` must be
//! tombstoned RECURSIVELY at the leaf's ABSOLUTE offset in h's slot
//! (`collect_heap_leaves(Inner, field_off, ..)`) so h's scope-end Drop frees a
//! null ptr → no double-free. FREE_COUNT must be exactly 1.
//!
//! Teeth (Mentor O, independent):
//! - struct_field_moveout_frees_once: FREE_COUNT==1, cap==5, m.n reads 7 (proves
//!   m got a real slot + correct nested offsets). Poison the JIT Struct-arm in
//!   `mir_lower.rs` (the recursive tombstone) → h.inner stays live → BOTH h and m
//!   free it → count==2 (the double-free). Pins the recursive tombstone as
//!   LOAD-BEARING.
//! - sibling_field_after_moveout: after `let m = h.inner`, the sibling `h.tag`
//!   stays readable (partial move, not whole-base) → returns 5, FREE_COUNT==1.
//!
//! ⚠ Records-only shim + process-global AtomicUsize + Mutex serialize.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_CAP: AtomicI64 = AtomicI64::new(-1);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __sfm_str_free(ptr: i64, cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_CAP.store(cap, Ordering::SeqCst);
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
        ShimSymbol::fn_2_0("__triet_string_free", __sfm_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const DECL: &str = "struct Inner { name: String, n: Integer }\n\
                    struct Holder { inner: Inner, tag: Integer }\n";

/// The Phase 2 target: move a heap-owning struct field OUT. Free exactly once.
/// Poison the JIT recursive tombstone → count==2 (double-free returns).
#[test]
fn struct_field_moveout_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    STR_CAP.store(-1, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let i = Inner {{ name: \"Giang\", n: 7 }};\n\
         \x20   let h = Holder {{ inner: i, tag: 5 }};\n\
         \x20   let m = h.inner;\n\
         \x20   return m.n;\n\
         }}"
    ));
    assert_eq!(
        r, 7,
        "m.n must read 7 (m got a real slot + correct nested offset)"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "Phase 2: a heap-struct field moved out must free its String exactly once \
         (JIT recursively tombstones h.inner's leaf at the absolute offset)"
    );
    assert_eq!(
        STR_CAP.load(Ordering::SeqCst),
        5,
        "the freed cap must be the REAL cap (5 for \"Giang\"), not stack garbage"
    );
}

/// A sibling field stays live after a partial move-out (not a whole-base move).
#[test]
fn sibling_field_after_moveout() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{DECL}function main() -> Integer = {{\n\
         \x20   let i = Inner {{ name: \"Giang\", n: 7 }};\n\
         \x20   let h = Holder {{ inner: i, tag: 5 }};\n\
         \x20   let m = h.inner;\n\
         \x20   return h.tag;\n\
         }}"
    ));
    assert_eq!(
        r, 5,
        "the sibling field h.tag stays readable after moving h.inner"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "still exactly one free (m owns the String; h.inner is tombstoned)"
    );
}

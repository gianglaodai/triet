//! ADR-0082 §AMEND-3 — get-by-value COPY of a Copy-aggregate element
//! (`Vector<Agg>` / `HashMap<scalarK, Agg>`).
//!
//! Free-count teeth (route-lower via `lower_source`, real allocator shims
//! for vector/hashmap machinery, a records-only `__triet_string_free`
//! counting stub): `get(v,i)`/`get(m,k)` on an all-scalar (Copy) struct
//! element, in a function that ALSO owns an UNRELATED heap `String` local.
//! The unrelated String must be freed EXACTLY once (its own scope-end
//! Drop) — the get-by-value copy-out path (a hand-unrolled Cranelift
//! load/store loop + tag write, `mir_lower.rs`) must add ZERO frees of its
//! own (`Point` has no heap leaf, nothing for it to touch). This is the
//! free-count evidence Luật thép demands over an exit-code-only claim: a
//! stray corruption in the new copy-out code (wrong stride, wrong dest
//! offset, wrong NULL_SENTINEL branch) would most plausibly show up here as
//! the unrelated String's slot getting stomped — leaked (0) or freed twice
//! (2) — not as a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Counting stand-in for `__triet_string_free` (mirrors the real
/// null/sentinel guard so only LIVE frees count).
#[unsafe(no_mangle)]
extern "C" fn __gbv_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

fn run(source: &str) -> i64 {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = [
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_get_ref", mir_lower::__triet_vector_get_ref),
        // ADR-0082 §AMEND-3: the get-by-value shim reuses `_get_ref`'s Rust
        // function under a second registered symbol name (same convention
        // as triet-driver/src/main.rs — see triet-mir builtin_shim_meta for
        // why the MIR name must stay distinct: borrowck's `returns_borrow_of`
        // keys off it).
        ShimSymbol::fn_2_1("__triet_vector_get_copy", mir_lower::__triet_vector_get_ref),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref",
            mir_lower::__triet_hashmap_get_ref,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_copy",
            mir_lower::__triet_hashmap_get_ref,
        ),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // Element/leaf free is the COUNTING stub (the teeth surface).
        ShimSymbol::fn_2_0("__triet_string_free", __gbv_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// `get(v,i)` on an all-scalar `Vector<Point>` element, alongside an
/// UNRELATED heap `String` local — the String's own scope-end Drop is the
/// ONLY free; the copy-out path touches nothing.
#[test]
fn vector_get_copy_struct_untouched_by_unrelated_string_drop() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Point { x: Integer, y: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Point> = vector_new();\n\
         \x20   xs = push(xs, Point { x: 3, y: 4 });\n\
         \x20   let s = \"unrelated\";\n\
         \x20   let r = get(xs, 0);\n\
         \x20   let v = match r { ~+ p => p.x, ~0 => -1 };\n\
         \x20   return v;\n\
         }");
    assert_eq!(r, 3, "get(xs,0).x == 3");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "the ONLY free must be the unrelated String's own scope-end Drop \
         (== 0 ⇒ leaked, get-copy corrupted its slot; == 2 ⇒ double-freed)"
    );
}

/// Same shape for `HashMap<Integer, Point>` get-by-value — the value copy
/// must not touch the unrelated String either.
#[test]
fn hashmap_get_copy_struct_untouched_by_unrelated_string_drop() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Point { x: Integer, y: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, Point> = hashmap_new();\n\
         \x20   m = insert(m, 1, Point { x: 7, y: 9 });\n\
         \x20   let s = \"unrelated\";\n\
         \x20   let r = get(m, 1);\n\
         \x20   let v = match r { ~+ p => p.y, ~0 => -1 };\n\
         \x20   return v;\n\
         }");
    assert_eq!(r, 9, "get(m,1).y == 9");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "the ONLY free must be the unrelated String's own scope-end Drop \
         (== 0 ⇒ leaked, get-copy corrupted its slot; == 2 ⇒ double-freed)"
    );
}

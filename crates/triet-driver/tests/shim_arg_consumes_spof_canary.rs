//! WO-SPOF-3 (O recon, G ✅) — canary for the `builtin_shim_meta` SPOF
//! (`crates/triet-mir/src/lib.rs:1076`, field `arg_consumes`). That table is
//! read TIN-MÙ by three independent consumers — Lowerer `emit_shim_call`,
//! Borrowck, and JIT M3 zeroing (`crates/triet-jit/src/mir_lower.rs:4803-
//! 4871`, `if let Some(meta) = builtin_shim_meta(callee_name) { ... }`). If
//! the table lies about a shim's real ownership behavior, M3 either zeros a
//! slot that still owns its heap value (leak) or fails to zero one that no
//! longer does (double-free) — silently, no test currently pins the table's
//! CONTENT against the shim's ACTUAL free behavior. This file is that pin:
//! it counts real `String` frees through a counting `__triet_string_free`
//! shim (M3 fully ON — `JitContext::with_shims`, same as production, no
//! `JitOptions`/`disable_m3` knob) so a manual poison of one table row can be
//! observed turning a baseline-green shape RED.
//!
//! Structure mirrors `heap_shim_consuming_temp_counting.rs` (STR_FREES +
//! TEST_LOCK + counting shim), but this file owns its OWN symbol prefix
//! (`__sacs_*`) so it never collides with that file's `__hsct_*` no-mangle
//! symbols when both test binaries link into the same process image.
//!
//! # Manual poison-table teeth procedure (THỦ CÔNG — O cắm, không trong file)
//!
//! This file does NOT ship a poisoned shim or a `#[test]` that flips the
//! table — the poison is a source edit to `triet-mir/src/lib.rs`, done by
//! hand, one direction at a time, `cargo build` (recompiles the whole
//! workspace including the shim itself), run ONLY this file, observe,
//! `cp`-restore (NEVER `git checkout`/`restore` — see
//! `feedback_teeth_never_git_checkout`), rebuild clean, re-run to confirm
//! green again.
//!
//! **Chiều 1 (consume→borrow, Shape A):** in `builtin_shim_meta`, flip
//! `"__triet_vector_push"`'s `arg_consumes: &[true, true]` to
//! `&[true, false]` (element arg no longer marked consumed). Rebuild, run
//! `cargo test -p triet-driver --test shim_arg_consumes_spof_canary
//! shape_a -- --exact --test-threads=1`. Expected: Shape A goes RED —
//! the shim itself still MOVES the element string into the vector (real
//! ownership transfer is in the C-shim body, unaffected by the metadata
//! flip), but M3 no longer zeros the caller's `h.name` slot after the call,
//! so BOTH the container's own element-free (real free #1) AND the stale
//! caller slot's end-of-scope Drop (spurious free #2) fire on the SAME
//! pointer — the exact signal (SIGABRT double-free vs FREE==2 vs something
//! else) is DATA to record at poison time, not something to assume in
//! advance.
//!
//! **Chiều 2 (borrow→consume, Shape C):** in `builtin_shim_meta`, flip
//! `"__triet_vector_pop"`'s `arg_consumes: &[false]` to `&[true]` (vector
//! handle arg wrongly marked consumed). Rebuild, run the `shape_c` test
//! alone. Expected: Shape C goes RED — M3 now zeros the caller's `v3` slot
//! after `pop`, so `v3`'s end-of-scope Drop becomes a no-op and the ONE
//! element `pop` left behind inside the vector ("a") leaks instead of being
//! freed. Baseline is FREE==2 (see Shape C below); poisoned expectation is
//! FREE==1 (one real free lost) — but the exact number is DATA measured at
//! poison time, not asserted here in advance. The prior candidates for this
//! direction were investigated and rejected before landing on `pop`:
//! `length(s)` (owned `String`) never emits a `CallDispatch` to
//! `__triet_string_len` at all (lowerer's owned-`String` fast path reads the
//! `len` field directly — verified via a MIR dump, zero calls to that shim
//! in the compiled body), and `length(&0 s)` (borrow) DOES call the shim but
//! `&0 String` is `Copy` at the MIR level, so both M3
//! (`crates/triet-jit/src/mir_lower.rs:4809`, `!arg_ty.is_copy(...)` guard)
//! and borrowck (`crates/triet-borrowck` mutate-precheck) skip a poisoned
//! `arg_consumes` entry for a Copy-typed arg — either candidate would have
//! produced a VACUOUS (never-red) teeth. `pop(Vector<String>)`'s vector-
//! handle arg is a non-Copy heap handle read by-value through
//! `emit_shim_call` (`crates/triet-lower/src/lib.rs:3095`), the only
//! confirmed-reachable construct for this direction.
//!
//! ⚠ RAM: run `--exact --test-threads=1` for any of the below (process-
//! global `AtomicUsize` + no-mangle shim — N7 fork-bomb hazard per repo
//! convention). `TEST_LOCK` serializes a default parallel `cargo test`.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __sacs_str_free(ptr: i64, cap: i64) {
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

fn shims_with(free_fn: extern "C" fn(i64, i64)) -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", free_fn),
    ]
}

fn run_with(source: &str, free_fn: extern "C" fn(i64, i64)) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = shims_with(free_fn);
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

fn run(source: &str) -> i64 {
    run_with(source, __sacs_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// Shape A — push-element (Chiều 1: consume→borrow poison target).
// `Vector<String>`, element arg is a field-access move-out (`h.name`),
// consuming shim `__triet_vector_push`. Container's own Drop frees the one
// element it holds. FREE==1.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHAPE_A: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let v2 = push(v, h.name);\n\
     \x20   return 0;\n\
     }";

#[test]
fn shape_a_push_element_free_1() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHAPE_A);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("Shape A (push element): FREE={count}");
    assert_eq!(
        count, 1,
        "baseline: push's consuming element arg frees exactly once (the \
         container's own element-free) — Chiều 1 poison target"
    );
}

// ══════════════════════════════════════════════════════════════════════
// Shape B — insert-value (Chiều 1, secondary — no dedicated poison teeth
// required per WO, baseline-sound check only).
// `HashMap<Integer, String>`, value arg is a field-access move-out,
// consuming shim `__triet_hashmap_insert`. FREE==1.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHAPE_B: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let m: HashMap<Integer, String> = hashmap_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let m2 = insert(m, 1, h.name);\n\
     \x20   return 0;\n\
     }";

#[test]
fn shape_b_insert_value_free_1() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHAPE_B);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("Shape B (insert value): FREE={count}");
    assert_eq!(
        count, 1,
        "baseline: insert's consuming value arg frees exactly once (the \
         container's own value-free)"
    );
}

// ══════════════════════════════════════════════════════════════════════
// Shape C — pop (Chiều 2: borrow→consume poison target).
// `Vector<String>` with TWO elements pushed ("a", "b"); `pop(v3)` borrows
// the vector handle (non-consuming, `arg_consumes: &[false]`, mutates len
// in place) and moves the LAST element ("b") out into `x: String?`.
//
// Baseline FREE==2: end-of-scope drop of `v3` frees the one element it
// still holds ("a") = 1 free; drop of `x` frees the popped element ("b")
// = 1 free. Total 2. This is a DIFFERENT number from Shape A/B (G-approved
// deliberate mismatch) — the absolute count is not the point, the SIGNAL
// is: poisoning `__triet_vector_pop`'s `arg_consumes` from `&[false]` to
// `&[true]` makes M3 zero `v3` after the call, turning `v3`'s drop into a
// no-op — "a" leaks — and the count drops from 2 to (measured, expected
// around) 1. The exact poisoned number is DATA to record when O cắm, not
// asserted here.
// ══════════════════════════════════════════════════════════════════════

const SRC_SHAPE_C: &str = "function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let v2 = push(v, \"a\");\n\
     \x20   let v3 = push(v2, \"b\");\n\
     \x20   let x: String? = pop(v3);\n\
     \x20   return 0;\n\
     }";

#[test]
fn shape_c_pop_free_2() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_SHAPE_C);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("Shape C (pop): FREE={count}");
    assert_eq!(
        count, 2,
        "baseline: v3's own Drop frees the 1 surviving element (\"a\") + \
         x's Drop frees the popped element (\"b\") = 2. Chiều 2 poison \
         target: __triet_vector_pop arg_consumes &[false] -> &[true]"
    );
}

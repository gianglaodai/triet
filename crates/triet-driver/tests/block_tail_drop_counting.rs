//! CFG-tail Drop-ordering Lát 1 (Bug A) — route-lower free-count test for an
//! expression-block tail value that escapes the block scope.
//!
//! `let v = { let t = vector_new(); t }` returns the block's tail as its value.
//! Before the fix, Expr::Block returned the tail local directly and pop_scope()
//! dropped it as a block-local even though it escapes → the enclosing scope
//! dropped it AGAIN → double-free (and borrowck E2421 at compile time). The fix
//! moves the tail into a fresh result local (mirror Expr::If); the move
//! tombstones the tail local so the block's pop_scope drop is a no-op → the
//! escaped value is freed EXACTLY once by the enclosing scope.
//!
//! Teeth (Mentor O re-verifies; covers BOTH Vector and String per G's mandate):
//!   - block-init Vector freed exactly once; String freed exactly once.
//!   - Poison: revert the Assign-move to a direct tail return → the tail local
//!     is dropped both in the block AND by the enclosing scope → count == 2.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __blk_vec_count_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __blk_str_count_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __blk_str_count_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_2_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __blk_vec_count_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
    ]
}

fn run(source: &str) -> (i64, usize, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    VEC_FREES.store(0, Ordering::SeqCst);
    STR_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (
        result,
        VEC_FREES.load(Ordering::SeqCst),
        STR_FREES.load(Ordering::SeqCst),
    )
}

#[test]
fn block_init_vector_freed_once() {
    // Tail value (empty vector) escapes the block into `v`; the enclosing scope
    // frees it exactly once. Poison (direct tail return) → block + enclosing
    // both drop it → count 2.
    let (result, vec_frees, _) = run("function main() -> Integer {\n\
         \x20   let v: Vector<Integer> = { let t = vector_new(); t };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        vec_frees, 1,
        "block-tail Vector escapes → freed exactly once (poison → 2)"
    );
}

#[test]
fn block_init_string_freed_once() {
    let (result, _, str_frees) = run("function main() -> Integer {\n\
         \x20   let s: String = { let x = \"hello\"; x };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(result, 0, "main returns 0");
    assert_eq!(
        str_frees, 1,
        "block-tail String escapes → freed exactly once (poison → 2)"
    );
}

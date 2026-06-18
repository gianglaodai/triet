//! ADR-0062 Heap-Nullable Lát 5 (Bug B) — route-lower free-count test for a
//! `?+>` map whose body produces a HEAP value (String/Vector/HashMap).
//!
//! `some ?+> |x| mk(x)` where mk returns a heap type T → the map auto-wraps to
//! T?. Before the fix, the NullableMap result was typed from the INPUT (here
//! Integer?), so the heap output was stored in a scalar-typed result → Drop
//! called the wrong free shim (or none) → the heap allocation leaked or a
//! garbage value was freed. The fix retypes the result from the body (map →
//! U?, flatMap → U?), so Drop dispatches the correct free shim → the mapped
//! heap value is freed EXACTLY once.
//!
//! Teeth (Mentor O re-verifies; covers ALL THREE heap types per G's mandate):
//!   - heap-output map → freed exactly once.
//!   - Poison (revert the retype → result keeps the scalar input type) → Drop
//!     no-ops on the scalar result → the heap value LEAKS → count == 0 → RED.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static HMAP_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __nm_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __nm_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __nm_hmap_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    HMAP_FREES.fetch_add(1, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, expr_resolutions, pattern_resolutions, method_resolutions) =
        triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(
        &program,
        &expr_resolutions,
        &pattern_resolutions,
        &method_resolutions,
    )
    .expect("lowering failed")
}

fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __nm_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_2_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __nm_vec_free),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __nm_hmap_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_3_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
    ]
}

fn run(source: &str) -> (i64, usize, usize, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    STR_FREES.store(0, Ordering::SeqCst);
    VEC_FREES.store(0, Ordering::SeqCst);
    HMAP_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (
        result,
        STR_FREES.load(Ordering::SeqCst),
        VEC_FREES.load(Ordering::SeqCst),
        HMAP_FREES.load(Ordering::SeqCst),
    )
}

#[test]
fn map_string_output_freed_once() {
    // some ?+> |x| mk(x) : Integer? → String? (map auto-wraps). The mapped
    // String is freed exactly once. Poison (result keeps Integer?) → Drop
    // no-ops → count 0.
    let (_result, str_frees, _, _) = run("function mk(x: Integer) -> String = \"hi\"\n\
         function main() -> Integer {\n\
         \x20   let s: Integer? = 1;\n\
         \x20   let r = s ?+> |x| mk(x);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(str_frees, 1, "mapped String output freed exactly once");
}

#[test]
fn map_vector_output_freed_once() {
    let (_result, _, vec_frees, _) = run(
        "function mk(x: Integer) -> Vector<Integer> = vector_new()\n\
         function main() -> Integer {\n\
         \x20   let s: Integer? = 1;\n\
         \x20   let r = s ?+> |x| mk(x);\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(vec_frees, 1, "mapped Vector output freed exactly once");
}

#[test]
fn map_hashmap_output_freed_once() {
    let (_result, _, _, hmap_frees) = run(
        "function mk(x: Integer) -> HashMap<Integer, Integer> = hashmap_new()\n\
         function main() -> Integer {\n\
         \x20   let s: Integer? = 1;\n\
         \x20   let r = s ?+> |x| mk(x);\n\
         \x20   return 0;\n\
         }",
    );
    assert_eq!(hmap_frees, 1, "mapped HashMap output freed exactly once");
}

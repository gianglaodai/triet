//! ADR-0058 Lát 2 — counting test for heap Outcome if/else merge.
//!
//! Verifies: (1) no double-free through merge (free-count = 1),
//! (2) tombstone source prevents double-free (poison → count↑),
//! (3) leak-guard CẤM (poison thêm leak-guard → wild-free UB).
//!
//! Teeth (Mentor O re-verifies on final tree):
//!   - Poison tombstone (remove `stack_store(zero, src_slot, 0)`) → count=2.
//!   - Poison leak-guard (re-add `emit_outcome_drop_glue(dest)` for heap)
//!     → SIGABRT or garbage length.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{JitContext, ShimSymbol};

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`.
#[unsafe(no_mangle)]
extern "C" fn __adr58_count_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    FREE_COUNT.fetch_add(1, Ordering::SeqCst);
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

#[test]
fn heap_outcome_if_merge_frees_exactly_once() {
    use triet_jit::mir_lower;

    // if/else merge of two heap Outcome function calls. The merge result `_2`
    // receives `move _3` (slot copy) from whichever branch is taken. The
    // source (`_1` or `_3`) is tombstoned → Drop no-op (free count stays 1
    // for the String "xyz"). After match, the bound String `e` is dropped
    // once.
    let bodies = lower_source(
        "function make_ok() -> Integer~String = ~+ 42\n\
         function make_err() -> Integer~String = ~- \"xyz\"\n\
         function main() -> Integer {\n\
         \x20   let o = if false { make_ok() } else { make_err() };\n\
         \x20   return match o {\n\
         \x20       ~+ x => x\n\
         \x20       ~- e => len(e)\n\
         \x20   };\n\
         }",
    );
    for body in &bodies {
        body.verify().expect("MIR verify");
    }

    let shims = &[
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __adr58_count_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ];

    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("ADR-0058 Lát 2: heap Outcome if-merge must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };

    assert_eq!(result, 3, "len(\"xyz\") = 3");
    assert_eq!(
        FREE_COUNT.load(Ordering::SeqCst),
        1,
        "the String \"xyz\" must be freed exactly once (no leak, no double-free)"
    );
}

//! ADR-0066 Lát 1 nhát 1b (WO-1b) — route-lower free-count test for a
//! whole-struct ARG-MOVE across a function boundary.
//!
//! `take(p)` moves `Person{name:String}` by pointer. The callee frees `name`
//! via its by-pointer drop-glue (A); the caller emits `Deinit(p)` right after
//! the call (B emits it, C tombstones the heap field) so main's scope-end
//! Drop(p) reads ptr=0 → free no-op. FREE_COUNT must be EXACTLY 1 — no leak
//! (callee frees), no double-free (caller no-ops).
//!
//! This is a ROUTE-LOWER test (real `.tri` → lower → JIT), NOT a hand-built
//! mir_lower.rs test, BECAUSE the R1-arg poison lives in the LOWERER (the
//! to_zero `ctx_is_copy` filter): hand-built MIR bypasses the lowerer and
//! emits Deinit unconditionally, so it could never exercise R1-arg. A
//! route-lower test bites all three teeth:
//! - R1-arg poison (B): lowerer to_zero → is_copy(None) → no Deinit → caller +
//!   callee both free → count == 2.
//! - R1-deinit poison (C): JIT Deinit no longer walks struct heap fields →
//!   tombstone no-op → caller Drop frees again → count == 2.
//! - R-callee poison (A): JIT struct drop-glue removed → callee never frees
//!   (Unsupported / leak) → count != 1.
//!
//! ⚠ RAM: run with `--exact --test-threads=1` (process-global AtomicUsize +
//! no-mangle shim — the N7 fork-bomb hazard). A poisoned double-free aborts via
//! the real free shim; this counting shim records-only (no real dealloc) so the
//! count is observable without crashing the test runner.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static SAM_STR_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __sam_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    SAM_STR_FREES.fetch_add(1, Ordering::SeqCst);
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
fn arg_move_struct_frees_once() {
    let source = "struct Person { name: String }\n\
         function take(p: Person) -> Integer = {\n\
         \x20   return 0\n\
         }\n\
         function main() -> Integer = {\n\
         \x20   let p = Person { name: \"Giang\" };\n\
         \x20   return take(p);\n\
         }";

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
        ShimSymbol::fn_2_0("__triet_string_free", __sam_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    SAM_STR_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let _ = unsafe { main.call_i64_0() };

    assert_eq!(
        SAM_STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1b: arg-move must free the struct's heap field EXACTLY once \
         (callee frees, caller tombstones → Drop no-op)"
    );
}

//! ADR-0066 Lát 1 nhát 1c (WO-1c) — route-lower free-count test for an
//! ASSIGN-MOVE `let q = p` of a heap-struct.
//!
//! `let q = p` moves `Person{name:String}` into `q` (true-move, NOT a
//! pseudo-copy alias). The lowerer emits `Assign{q←p}` + `Deinit(p)` atomically
//! (D1+D2); the JIT's Deinit struct-walk (1b/C) tombstones p's heap-field ptr →
//! p's scope-end Drop is a no-op, q's Drop frees the String once. FREE_COUNT
//! must be EXACTLY 1 — no leak, no double-free.
//!
//! Route-lower (real `.tri` → lower → JIT), NOT hand-built mir_lower.rs: the
//! R1-assign poison lives in the LOWERER (the D2 Deinit emit) — hand-built MIR
//! bypasses it.
//!   - R1-assign poison (D2): drop the `Deinit(p)` emit → p not tombstoned →
//!     Drop(p) + Drop(q) both free → count == 2 (double-free).
//!
//! ⚠ RAM: run with `--exact --test-threads=1` (process-global AtomicUsize +
//! no-mangle shim — the N7 fork-bomb hazard). This counting shim records-only
//! (no real dealloc) so a poisoned double-free is observable as count == 2
//! without aborting the test runner.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static SMV_STR_FREES: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn __smv_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    SMV_STR_FREES.fetch_add(1, Ordering::SeqCst);
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
fn assign_move_struct_frees_once() {
    let source = "struct Person { name: String, age: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let p = Person { name: \"Giang\", age: 5 };\n\
         \x20   let q = p;\n\
         \x20   return q.age;\n\
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
        ShimSymbol::fn_2_0("__triet_string_free", __smv_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");

    SMV_STR_FREES.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };

    assert_eq!(result, 5, "q.age must read 5 through the moved struct");
    assert_eq!(
        SMV_STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0066 1c: assign-move must free the struct's heap field EXACTLY once \
         (q frees, p tombstoned → Drop no-op)"
    );
}

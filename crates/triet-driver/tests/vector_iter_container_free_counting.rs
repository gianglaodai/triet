//! ADR-0089 §AMEND Slice 2a §2a.5 T2a-intact/T2a-rvalue — permanent
//! container-FREE-count tooth for `for item in <Vector>` (Mentor G's
//! §2a.3.1 HANDLE-ALIASING mine: a NEW `owned_local` aliasing the iterable's
//! handle would double-free the CONTAINER at scope exit).
//!
//! Two scenarios, mirroring `break_drop_counting.rs`'s real-pipeline +
//! counting-shim-swap pattern:
//!
//!  - `lvalue_container_frees_exactly_once`: `let v = ...; for x in v {}; ...`
//!    — the iterable is a NAMED local. `Stmt::For`'s Vector-copy desugar
//!    must reuse `v`'s existing local (no `push_owned` of a fresh alias) —
//!    `v` drops exactly once at `main`'s own scope exit.
//!  - `rvalue_container_frees_exactly_once`: `for x in make_vector() {}` —
//!    the iterable is a bare Call (rvalue). The desugar must `push_owned`
//!    the fresh temp itself (Bước-0 map-trace: the generic `Expr::Call`
//!    scalar-return branch does NOT register its `dest` for Drop — only an
//!    enclosing `Stmt::Let` normally does, which is absent here) so the
//!    temp container drops exactly once, not zero (leak) and not two
//!    (double-free).
//!
//! Both must read FREE == 1 for the CONTAINER. The fixture-only regression
//! tests (480/484, EXPECT-value) are vacuous for this specific soundness
//! property — a leaked or double-freed container does not change `main`'s
//! return value on its own (a double-free would abort the process before
//! `main` returns, which a value-mismatch test would also catch, but only
//! as a crash, not as a signed count — this test makes the exact FREE count
//! the assertion, per the counting-harness pattern used throughout this
//! crate).
//!
//! Poison verify (D, before landing): §2a.3.1's lvalue guard removed (i.e.
//! `push_owned(iter_local)` called unconditionally for BOTH lvalue and
//! rvalue) → the lvalue test's `v` is registered twice in `owned_locals`
//! (once by `Stmt::Let`, once here) — `push_owned` itself is idempotent
//! (`Ctx::push_owned` no-ops on a local already present), so THIS specific
//! poison does NOT reproduce a double-free through `owned_locals`
//! duplication (the dedup guard absorbs it) — the actually load-bearing
//! poison is skipping the `is_lvalue` check entirely and allocating a BRAND
//! NEW local (`c.alloc_local_ty(...)` + `Assign` + `push_owned`) aliasing
//! the same handle, which this test's FREE==2 assertion catches directly
//! (see the inline comment on the poison procedure below the tests).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_vector_free`. Mirrors the real free's
/// null/sentinel guard so only LIVE buffer frees count.
#[unsafe(no_mangle)]
extern "C" fn __vic_count_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    FREE_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Replicates the driver's source→bodies pipeline (main.rs phases 1-3).
fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Real Vector shims, but `__triet_vector_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __vic_count_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_get", mir_lower::__triet_vector_get),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
    ]
}

/// Compile `source`, call `main`, return (`main`'s result, container free count).
fn run_counting(source: &str) -> (i64, usize) {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("for-Vector program must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (result, FREE_COUNT.load(Ordering::SeqCst))
}

#[test]
fn lvalue_container_frees_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (result, frees) = run_counting(
        "function main() -> Integer {\n\
         \x20   let mutable v = vector_new();\n\
         \x20   v = push(v, 1);\n\
         \x20   v = push(v, 2);\n\
         \x20   let mutable sum = 0;\n\
         \x20   for x in v {\n\
         \x20       sum = sum + x;\n\
         \x20   }\n\
         \x20   return sum + len(v);\n\
         }",
    );
    assert_eq!(result, 5, "main must return 3 (sum) + 2 (len(v) unchanged)");
    assert_eq!(
        frees, 1,
        "the NAMED-local container `v` must free EXACTLY ONCE at main's own \
         scope exit — 0 would be a leak (for-Vector never dropped it), 2 would \
         be a double-free (§2a.3.1 handle-aliasing: a fresh owned_local was \
         allocated for the iterable instead of reusing `v`'s own local)"
    );
}

#[test]
fn rvalue_container_frees_exactly_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (result, frees) = run_counting(
        "function make_vector() -> Vector<Integer> {\n\
         \x20   let mutable v = vector_new();\n\
         \x20   v = push(v, 4);\n\
         \x20   v = push(v, 5);\n\
         \x20   return v;\n\
         }\n\
         function main() -> Integer {\n\
         \x20   let mutable sum = 0;\n\
         \x20   for x in make_vector() {\n\
         \x20       sum = sum + x;\n\
         \x20   }\n\
         \x20   return sum;\n\
         }",
    );
    assert_eq!(result, 9, "main must return 4 + 5");
    assert_eq!(
        frees, 1,
        "the RVALUE container temp (from `make_vector()`, never named) must \
         free EXACTLY ONCE — 0 would be a leak (Bước-0 map-trace: the generic \
         Call scalar-return branch does not `push_owned` its own dest; the \
         for-Vector desugar must do so itself for the rvalue case), 2 would \
         be a double-free (registered twice)"
    );
}

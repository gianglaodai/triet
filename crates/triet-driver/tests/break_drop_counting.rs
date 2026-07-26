//! ADR-0089 §5 T-break-drop — permanent FREE-count tooth for the break path.
//!
//! `loop { let s = "x"; i+=1; if i==3 { break } }`: heap local `s` is
//! allocated fresh each of the 3 iterations and must be dropped each time —
//! iterations 1 and 2 via the structural `pop_scope` drop on the loop
//! back-edge, iteration 3 via `break`'s own `emit_scope_drops` (ADR-0089
//! §4) since the structural drop at the (now unreachable) end of the body
//! never runs on that path. So FREE must equal exactly 3 — the fixture-only
//! regression test (472_loop_break.tri, EXPECT-value) cannot see this: a
//! leaked/double-freed `s` on the break path does not change `main`'s
//! return value, so a value-only fixture is vacuous for this specific
//! soundness property. This permanent counting test is the actual tooth.
//!
//! Mirrors `string_nullable_drop_counting.rs` (real pipeline, real shims,
//! `__triet_string_free` swapped for a counting stand-in).
//!
//! Teeth (Mentor O verified by hand before this test existed): poison the
//! break-path drop emission (remove the `emit_scope_drops` call in the
//! `Stmt::Break` lowering arm, `crates/triet-lower/src/lib.rs`) → the
//! iteration-3 `s` leaks → FREE == 2, not 3 — this test goes red.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

// Serialize — this in-binary counter is shared across tests in this file,
// and cargo runs tests within a file concurrently by default.
static TEST_LOCK: Mutex<()> = Mutex::new(());

static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it only counts frees of LIVE
/// allocations.
#[unsafe(no_mangle)]
extern "C" fn __brkdrop_count_free(ptr: i64, cap: i64) {
    let _ = cap;
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

/// Real shim set, but `__triet_string_free` swapped for the counter.
fn counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_pow", mir_lower::__triet_pow),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __brkdrop_count_free),
        ShimSymbol::fn_5_0("__triet_string_concat", mir_lower::__triet_string_concat),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ]
}

/// Compile `source`, call `main`, return (`main`'s result, free count).
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
        .expect("break-drop program must JIT-compile");

    FREE_COUNT.store(0, Ordering::SeqCst);
    let main = compiled.get("main").expect("main compiled");
    let result = unsafe { main.call_i64_0() };
    (result, FREE_COUNT.load(Ordering::SeqCst))
}

#[test]
fn break_path_frees_heap_local_each_iteration() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (result, frees) = run_counting(
        "function main() -> Integer {\n\
         \x20   let mutable i = 0;\n\
         \x20   loop {\n\
         \x20       let s = \"x\";\n\
         \x20       i = i + 1;\n\
         \x20       if i == 3 { break; }\n\
         \x20   }\n\
         \x20   return i;\n\
         }",
    );
    assert_eq!(result, 3, "main must return 3 (i at break)");
    assert_eq!(
        frees, 3,
        "heap `s` must be freed once per iteration: 2 structural (back-edge) \
         + 1 via break's emit_scope_drops"
    );
}

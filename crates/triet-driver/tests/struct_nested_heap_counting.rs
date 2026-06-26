//! ADR-0067 Lát 2 nhát 2a — route-lower free-count teeth for NESTED-FLAT
//! heap-in-struct (`Outer { inner: Inner{ s: String } }`, non-recursive).
//!
//! The drop-glue and Deinit walk the layout RECURSIVELY at compile time
//! (`collect_heap_leaves`), so the inner String is freed / tombstoned at its
//! absolute offset. These counting tests prove FREE_COUNT==1 (no leak at the
//! nested tier, no double-free on move).
//!
//! Teeth (Mentor O re-verifies on the final tree):
//! - R-leak-nested: make `collect_heap_leaves` non-recursive (drop the Struct
//!   arm) → the inner String is never freed → count 0.
//! - R-double-free-nested: make the Deinit walk non-recursive → the inner leaf
//!   is not tombstoned after a move → caller + callee both free → count 2.
//!
//! ⚠ RAM: `--exact --test-threads=1` with ulimit -v (process-global AtomicUsize
//! and no-mangle shim — N7 fork-bomb hazard). The two tests share the counter,
//! so a Mutex serializes them (the gate runs `cargo test` parallel).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __nest_str_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_0("__triet_string_free", __nest_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Nested construct + drop → the inner String is freed exactly once.
/// R-leak-nested poison (collect_heap_leaves non-recursive) → count 0.
#[test]
fn nested_field_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Inner { s: String }\n\
         struct Outer { inner: Inner }\n\
         function main() -> Integer = {\n\
         \x20   let o = Outer { inner: Inner { s: \"Giang\" } };\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2a: nested struct's inner String must free exactly once on Drop"
    );
}

/// Nested whole-move across a boundary → still freed exactly once.
/// R-double-free-nested poison (Deinit walk non-recursive) → the inner leaf is
/// not tombstoned → caller + callee both free → count 2.
#[test]
fn nested_move_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Inner { s: String }\n\
         struct Outer { inner: Inner }\n\
         function take(o: Outer) -> Integer = {\n\
         \x20   return 0\n\
         }\n\
         function main() -> Integer = {\n\
         \x20   let o = Outer { inner: Inner { s: \"Giang\" } };\n\
         \x20   return take(o);\n\
         }");
    assert_eq!(r, 0);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0067 2a: nested arg-move must free the inner String exactly once \
         (callee frees, caller tombstones the nested leaf → Drop no-op)"
    );
}

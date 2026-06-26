//! ADR-0070 read-side — route-lower free-count teeth for a single heap-field
//! MOVE-OUT `let s = p.name`. The field's value moves into a fresh local; the
//! base's Drop must NOT free the moved leaf (Δ2 tombstone), so the heap value
//! frees EXACTLY once across `Drop(base)` + `Drop(dest)`.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - T-no-double-free (⚔ G #1): remove the Δ2 tombstone in mir_lower's
//!     Statement::Assign read-side block → the base slot keeps the moved ptr →
//!     `Drop(base)` frees it AND `Drop(dest)` frees it → FREE_COUNT == 2 (a
//!     clean RED count, not a segfault — the recording shim never deallocs).
//!   - vector variant: same, with a Vector field (8B thin handle).
//!
//! ⚠ RAM: process-global AtomicUsize + no-mangle shims (N7 fork-bomb hazard).
//! `TEST_LOCK` serializes the reset-then-read windows; recording-only shims
//! (no real dealloc) so a poisoned double-free is an observable count, not a
//! crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __mo_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __mo_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
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
        ShimSymbol::fn_2_0("__triet_string_free", __mo_str_free),
        ShimSymbol::fn_2_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __mo_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
    ]
}

fn run(source: &str) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = counting_shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// ⚔ G #1 — `let s = p.name` moves the String field out; the base `p` is
/// later dropped (it still owns `age`). The Δ2 tombstone zeroes `p`'s name
/// ptr so `Drop(p)` frees nothing — only `Drop(s)` frees → COUNT == 1.
/// Remove the tombstone → COUNT == 2.
#[test]
fn string_field_moveout_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Person { name: String, age: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let nm = \"hi\";\n\
         \x20   let p = Person { name: nm, age: 7 };\n\
         \x20   let s = p.name;\n\
         \x20   return p.age;\n\
         }");
    assert_eq!(r, 7);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0070: a moved-out String field must free exactly once \
         (base Drop must see the tombstoned ptr=0)"
    );
}

/// Vector field move-out (8B thin handle): same tombstone law.
#[test]
fn vector_field_moveout_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Bag { items: Vector<Integer>, n: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let b = Bag { items: push(vector_new(), 1), n: 9 };\n\
         \x20   let v = b.items;\n\
         \x20   return b.n;\n\
         }");
    assert_eq!(r, 9);
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0070: a moved-out Vector field must free exactly once"
    );
}

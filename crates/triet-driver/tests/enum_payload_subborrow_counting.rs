//! ADR-0084 §AMEND Lat B — free-count teeth for the enum payload SUB-BORROW
//! (formerly E1050-refused, now a zero-copy `&0 <payload>` bind).
//!
//! WO §4/§5: every heap payload kind (String/Vector/HashMap) needs its OWN
//! free-count proof — `is_any_heap()` must not paper over per-type gaps (the
//! Vector/HashMap silent-MISS this WO's report documents was found exactly
//! because String+Struct were the only O-measured kinds). Each test:
//! construct an enum LOCAL holding a heap payload, bind the payload through
//! a direct `&0` scrutinee (sub-borrow, NOT move-out), read through it, then
//! let the enum local drop at scope end. The heap payload must free EXACTLY
//! ONCE (from the enum's own Drop) — the sub-borrow binding itself must NOT
//! trigger a second free (that would be the double-free E1050 originally
//! existed to prevent) and must not leak (0 frees).
//!
//! Deliberately a BARE local (`let b = Bag::Items(inner); match &0 b {..}`),
//! not routed through an outer `Vector<Bag>` container — an earlier draft of
//! this file wrapped the enum in a `Vector<Bag>` and asserted `VEC_FREES ==
//! 1`; that FAILED with `VEC_FREES == 2` because the outer container's OWN
//! buffer free and the inner payload Vector's free are two DIFFERENT,
//! legitimate frees that both call `__triet_vector_free` — a test design
//! bug, not an implementation bug (caught before this file was reported as
//! passing). The bare-local form isolates exactly one heap object per test.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static VEC_FREES: AtomicUsize = AtomicUsize::new(0);
static HM_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __subborrow_str_free(ptr: i64, _cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __subborrow_vec_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    VEC_FREES.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
extern "C" fn __subborrow_hm_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    HM_FREES.fetch_add(1, Ordering::SeqCst);
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
    let callee_sigs: std::collections::BTreeMap<String, triet_mir::FunctionSignature> = bodies
        .iter()
        .map(|b| (b.signature.name.clone(), b.signature.clone()))
        .collect();
    for body in &bodies {
        let result = triet_borrowck::checker::check_body_with(body, &callee_sigs);
        assert!(result.is_ok(), "borrowck errors: {:?}", result.errors);
    }
    let shims = [
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __subborrow_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", __subborrow_vec_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __subborrow_hm_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// String payload sub-borrow: bind through `&0`, read `length(s)`, then let
/// the enum local drop — String must free EXACTLY ONCE.
#[test]
fn enum_string_payload_subborrow_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Ping(String), Pong }\n\
         function main() -> Integer {\n\
         \x20   let e = Msg::Ping(\"hi\");\n\
         \x20   let n = match &0 e { Msg::Ping(s) => length(s), Msg::Pong => 0, };\n\
         \x20   return n;\n\
         }");
    assert_eq!(r, 2, "sub-borrow must read the correct length");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0084 Lat B: String payload sub-borrow must free EXACTLY ONCE \
         (the enum's own Drop only) — 0 = leak, 2 = double-free (the exact \
         hazard E1050 existed to prevent)"
    );
}

/// Vector payload sub-borrow — the WO-flagged UNMEASURED kind. Bind through
/// `&0`, read `len(v)`, drop the enum local — Vector must free EXACTLY ONCE.
#[test]
fn enum_vector_payload_subborrow_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    VEC_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Bag { Items(Vector<Integer>), Empty }\n\
         function main() -> Integer {\n\
         \x20   let mutable inner: Vector<Integer> = vector_new();\n\
         \x20   inner = push(inner, 1);\n\
         \x20   inner = push(inner, 2);\n\
         \x20   inner = push(inner, 3);\n\
         \x20   let b = Bag::Items(inner);\n\
         \x20   let n = match &0 b { Bag::Items(v) => len(v), Bag::Empty => -1, };\n\
         \x20   return n;\n\
         }");
    assert_eq!(r, 3, "sub-borrow must read the correct length");
    assert_eq!(
        VEC_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0084 Lat B: Vector payload sub-borrow must free EXACTLY ONCE — \
         this is the exact case that was silent-MISS (garbage value) before \
         the handle-vs-inline-repr split; a leak (0) or double-free (2) here \
         would be the OTHER two ways this could still be unsound"
    );
}

/// HashMap payload sub-borrow — the WO-flagged UNMEASURED kind (sibling of
/// Vector). Bind through `&0`, read `len(m)`, drop the enum local — HashMap
/// must free EXACTLY ONCE.
#[test]
fn enum_hashmap_payload_subborrow_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HM_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Box { Kv(HashMap<Integer, Integer>), Empty }\n\
         function main() -> Integer {\n\
         \x20   let mutable inner: HashMap<Integer, Integer> = hashmap_new();\n\
         \x20   inner = insert(inner, 1, 10);\n\
         \x20   inner = insert(inner, 2, 20);\n\
         \x20   let b = Box::Kv(inner);\n\
         \x20   let n = match &0 b { Box::Kv(m) => len(m), Box::Empty => -1, };\n\
         \x20   return n;\n\
         }");
    assert_eq!(r, 2, "sub-borrow must read the correct length");
    assert_eq!(
        HM_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0084 Lat B: HashMap payload sub-borrow must free EXACTLY ONCE"
    );
}

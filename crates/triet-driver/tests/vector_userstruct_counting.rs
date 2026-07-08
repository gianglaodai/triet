//! ADR-0082 B-α Slice A — route-lower free-count TEETH for `Vector<UserStruct>`
//! (Mentor O, cemented on the final C1–C4 tree).
//!
//! Source-level `.tri` → frontend → lower → JIT, with a records-only
//! `__triet_string_free` counting stub (STR_FREES). Real vector shims
//! (alloc/push/free) so element bytes actually move; the String leaf inside
//! each struct element is what we count.
//!
//! The load-bearing scenario (D's blocking double-free, ADR-0082 §AMEND-1):
//!
//!     struct User { name: String }
//!     let a = User { name: "aa" };   // heap-owning struct in a NAMED local
//!     xs = push(xs, a);              // `a` consumed by push (byte-moved into buffer)
//!     ...                            // scope-end Drop(a) + Drop(xs)
//!
//! Healthy tree: each String freed EXACTLY once → STR_FREES == #elements.
//!
//! Teeth (poison-must-be-red, cp-snapshot cycles run by O):
//!  - T-DOUBLE (C2/T7): revert the M3 zero-on-move struct-slot branch in
//!    `mir_lower.rs` (3436) back to String-only → each named local `a`/`b` is
//!    NOT tombstoned → its slot's String ptr (already byte-moved into the
//!    vector buffer) is freed a 2nd time by Drop(a) → STR_FREES == 2*N.
//!  - T-LEAK (C4/T5): revert `aggregate_needs_drop` guard to `is_any_heap()` →
//!    a Struct element is skipped by the vector element-free loop → the String
//!    leaf inside every element LEAKS → STR_FREES == 0.
//!  - T-COPY (DP-5): `Vector<Point>` (all-scalar struct) must compile+run with
//!    ZERO string frees (no heap leaf → element loop a no-op, byte-compat).
//!  - T-NEST (DP-6): `Vector<Tagged>` where `Tagged { tags: Vector<String> }`
//!    → drop frees the inner Vector's String elements recursively.
//!
//! ⚠ Records-only shim + process-global AtomicUsize: a Mutex serializes the
//! shared counter (the gate runs `cargo test` in parallel).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Counting stand-in for `__triet_string_free` (mirrors the real null/sentinel
/// guard so only LIVE frees count — a tombstoned/moved slot holds 0/sentinel).
#[unsafe(no_mangle)]
extern "C" fn __vus_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

fn run(source: &str) -> i64 {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = [
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
        ShimSymbol::fn_2_1("__triet_vector_get", mir_lower::__triet_vector_get),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        // Element/leaf free is the COUNTING stub (the teeth surface).
        ShimSymbol::fn_2_0("__triet_string_free", __vus_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Lower + attempt JIT compile, expecting a HARD `JitError` refuse (the source
/// must typecheck+lower cleanly — the refuse is a backend boundary, not a type
/// error). Returns the error string for assertion.
fn compile_expect_refuse(source: &str) -> String {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(
        type_errors.is_empty(),
        "must typecheck (refuse is a JIT boundary): {type_errors:?}"
    );
    let bodies = triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed");
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    // Register the full shim superset (vector + string + hashmap) so compilation
    // reaches the intended aggregate-refuse point instead of tripping on a
    // missing-shim error first.
    let shims = [
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_4_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __vus_str_free),
    ];
    let mut ctx = JitContext::with_shims(&shims);
    match ctx.compile_multi(&body_refs) {
        Ok(_) => panic!("expected a JitError refuse, but compilation SUCCEEDED (silent leak risk)"),
        Err(e) => format!("{e:?}"),
    }
}

/// T-DOUBLE + T-LEAK anchor: two heap-bearing structs pushed from NAMED locals
/// into a `Vector<User>`, then dropped at scope end. Each String must be freed
/// EXACTLY once — 2 elements → STR_FREES == 2.
#[test]
fn vector_userstruct_named_push_drop_frees_each_string_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<User> = vector_new();\n\
         \x20   let a = User { name: \"aa\" };\n\
         \x20   xs = push(xs, a);\n\
         \x20   let b = User { name: \"bb\" };\n\
         \x20   xs = push(xs, b);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "ADR-0082 B-α: Vector<User> drop must free each element's String field \
         EXACTLY once (T7 tombstones the moved-from named local; T4/T5 drive the \
         recursive struct-element drop). == 4 ⇒ T7 double-free; == 0 ⇒ T5 leak."
    );
}

/// T-COPY (DP-5): an all-scalar struct element has NO heap leaf → the element
/// free-loop must stay a no-op → zero String frees, byte-compat with the
/// pre-B-α scalar Vector path. Compiles + runs.
#[test]
fn vector_copy_struct_push_drop_frees_nothing() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Point { x: Integer, y: Integer }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Point> = vector_new();\n\
         \x20   let p = Point { x: 3, y: 4 };\n\
         \x20   xs = push(xs, p);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "Copy struct element (no heap leaf) must free no String (DP-5 byte-compat)"
    );
}

/// T-NEST (DP-6): a struct element whose field is itself a heap collection
/// (`Vector<String>`) → dropping the outer `Vector<Tagged>` must recurse
/// through the struct leaf into the inner vector and free its String elements.
#[test]
fn vector_nested_struct_vector_string_drop_recurses() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Tagged { tags: Vector<String> }\n\
         function main() -> Integer = {\n\
         \x20   let mutable inner: Vector<String> = vector_new();\n\
         \x20   inner = push(inner, \"x\");\n\
         \x20   inner = push(inner, \"y\");\n\
         \x20   let t = Tagged { tags: inner };\n\
         \x20   let mutable xs: Vector<Tagged> = vector_new();\n\
         \x20   xs = push(xs, t);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "nested Vector of Tagged(tags: Vector<String>) drop must recurse 2 tiers \
         and free both inner Strings exactly once"
    );
}

/// T-REFUSE-HashMap (T8): a `HashMap` with a Struct/Enum key or value is Slice C
/// (value free-loops not wired for aggregates) — it MUST refuse EXPLICITLY at
/// the JIT, never compile-then-leak. Poison: remove the `refuse_hashmap_
/// aggregate_kv` guard → this compiles → the test's `is_err` flips.
#[test]
fn hashmap_struct_value_refused_at_jit() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let err = compile_expect_refuse(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, User> = hashmap_new();\n\
         \x20   let u = User { name: \"x\" };\n\
         \x20   m = insert(m, 1, u);\n\
         \x20   return 0;\n\
         }",
    );
    assert!(
        err.contains("Slice C") || err.contains("aggregate"),
        "HashMap<_,Struct> must refuse with the Slice-C boundary message, got: {err}"
    );
}

/// T-REFUSE-Enum: a `Vector<Enum>` by-value element is Slice B — `vector_elem_
/// size` still refuses `Enum` (only `Struct` opened in Slice A). No silent leak.
#[test]
fn vector_enum_element_refused_at_jit() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let err = compile_expect_refuse(
        "enum Color { Red, Green }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Color> = vector_new();\n\
         \x20   xs = push(xs, Color::Red);\n\
         \x20   return 0;\n\
         }",
    );
    assert!(
        err.contains("Enum") || err.contains("Slice B"),
        "Vector<Enum> must refuse (Slice B boundary), got: {err}"
    );
}

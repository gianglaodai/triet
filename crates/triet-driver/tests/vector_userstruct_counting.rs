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
        // `__triet_vector_pop` MUST be registered so a `pop` program reaches the
        // AM1 aggregate-move-out refuse guard, not a spurious missing-shim error
        // (that would be a VACUOUS refuse — poison-insensitive to the guard).
        ShimSymbol::fn_2_1("__triet_vector_pop", mir_lower::__triet_vector_pop),
        ShimSymbol::fn_4_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        // ADR-0082 Slice C (T5): `__triet_hashmap_remove` MUST be registered
        // so a `remove` program reaches the F4 K+V refuse guard, not a
        // spurious missing-shim error (VACUOUS refuse).
        ShimSymbol::fn_4_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
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

/// T5 (ADR-0082 Slice C, F4): SUPERSEDES the former `hashmap_struct_value_
/// refused_at_jit` — Slice C deliberately OPENS `insert` for a Struct/Enum
/// VALUE (see `hashmap_struct_value_insert_drop_frees_string_field` in
/// `typed_hashmap_counting.rs` for the new positive coverage), so the old
/// assertion ("insert refuses a Struct value") is now false BY DESIGN, not a
/// regression. `remove` is a read/move-out site that stays behind the K+V
/// `refuse_hashmap_aggregate_kv` guard (F4 keeps 3 sites — remove×2, get-
/// family×1 — on the full K+V refuse; only alloc+insert were nới for VALUE).
/// Renamed + re-targeted per LUẬT 3 (repurposing authorized by the Slice C
/// Work Order; O/G re-verify independently). Poison: remove the
/// `refuse_hashmap_aggregate_kv` guard at the `remove` call-site → this
/// compiles → the test's `is_err` flips.
#[test]
fn hashmap_struct_value_remove_refused_at_jit() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let err = compile_expect_refuse(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, User> = hashmap_new();\n\
         \x20   let u = User { name: \"x\" };\n\
         \x20   m = insert(m, 1, u);\n\
         \x20   let r = remove(m, 1);\n\
         \x20   return 0;\n\
         }",
    );
    assert!(
        err.contains("Slice C") || err.contains("aggregate"),
        "HashMap<_,Struct> remove must refuse with the Slice-C boundary message, got: {err}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-0082 B-α Slice B (Vector<Enum> push+drop) — AM3 teeth (Mentor O).
// `pop`/by-value move-out is REFUSED (WO-AMEND, deferred); only push+drop ship.
// ─────────────────────────────────────────────────────────────────────────

/// T-ENUM-LEAK anchor (BUG-1, INLINE — no named local to mask): two heap-bearing
/// enums pushed by INLINE constructor into a `Vector<Msg>`, dropped at scope end.
/// The ONLY frees come from the vector's element drop-glue, so a miswired
/// `aggregate_needs_drop` (Enum falling to `is_any_heap()`=false) is caught
/// directly: STR_FREES must be 2. Poison FIX-1 (revert the Enum arm of
/// `aggregate_needs_drop`) → element-free loop bails → LEAK → 0.
/// ⚠️ This tooth MUST stay INLINE: a NAMED-local variant is maskable — an
/// un-tombstoned local's own Drop frees the string, faking "2" while the buffer
/// leaks (the exact mirage WO-AMEND-2 uncovered). Inline is non-masking.
#[test]
fn vector_enum_inline_push_drop_frees_each_string_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Msg> = vector_new();\n\
         \x20   xs = push(xs, Msg::Text(\"aa\"));\n\
         \x20   xs = push(xs, Msg::Text(\"bb\"));\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "Vector<Msg> element drop-glue must free each variant's String once \
         (== 0 ⇒ BUG-1: aggregate_needs_drop misses Enum → leak)"
    );
}

/// T-ENUM-TOMBSTONE anchor (BUG-2, NAMED): the SAME two Strings pushed from NAMED
/// locals. After FIX-1 the vector frees both (== 2 baseline). FIX-2 tombstones
/// the moved-from enum local (zeroes the payload ptr @+8) so its end-of-scope
/// Drop is a no-op. Poison FIX-2 (drop the enum arm of the arg-consume zeroing)
/// → Drop(a)/Drop(b) free the already-moved Strings a SECOND time → 4.
/// Paired with the INLINE tooth above, the two signals separate the bugs
/// cleanly: poison FIX-1 → inline 0; poison FIX-2 → named 4.
#[test]
fn vector_enum_named_push_drop_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Msg> = vector_new();\n\
         \x20   let a = Msg::Text(\"aa\");\n\
         \x20   xs = push(xs, a);\n\
         \x20   let b = Msg::Text(\"bb\");\n\
         \x20   xs = push(xs, b);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "moved-from enum local must be tombstoned — each String freed once \
         (== 4 ⇒ BUG-2: local not tombstoned → double-free)"
    );
}

/// T-ENUM-ACTIVE-ARM: the drop-glue is a runtime tag-switch — only the ACTIVE
/// variant's heap payload is freed. Push a `Text(String)` (heap) and a
/// `Code(Integer)` (scalar) → exactly ONE String free (the Code element carries
/// no heap). Proves the disc discrimination: a broken disc marshal (S3a) would
/// mis-tag the Text element → its String leaks → 0, or mis-free the Code → crash.
#[test]
fn vector_enum_active_arm_only_frees_heap_variant() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Text(String), Code(Integer) }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Msg> = vector_new();\n\
         \x20   let a = Msg::Text(\"aa\");\n\
         \x20   xs = push(xs, a);\n\
         \x20   let b = Msg::Code(7);\n\
         \x20   xs = push(xs, b);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "only the ACTIVE Text arm carries a String — exactly 1 free"
    );
}

/// T-ENUM-SCALAR (DP-5 analog): an all-scalar enum (`Color`, total_size==8, no
/// heap payload) rides the 8B push path (S3b) — must compile+run with ZERO
/// string frees. Byte-compat with the scalar Vector path; proves S3b routes the
/// enum slot without dragging in a bogus free.
#[test]
fn vector_scalar_enum_push_drop_frees_nothing() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Color { Red, Green, Blue }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Color> = vector_new();\n\
         \x20   xs = push(xs, Color::Green);\n\
         \x20   xs = push(xs, Color::Red);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "scalar enum has no heap leaf — zero string frees"
    );
}

/// T-ENUM-NEST: an enum variant whose payload is itself a heap collection
/// (`Tags(Vector<String>)`) → dropping the outer `Vector<Wrap>` recurses through
/// the enum ACTIVE arm into the inner vector and frees its String elements.
#[test]
fn vector_nested_enum_vector_string_drop_recurses() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Wrap { Tags(Vector<String>), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable inner: Vector<String> = vector_new();\n\
         \x20   inner = push(inner, \"x\");\n\
         \x20   inner = push(inner, \"y\");\n\
         \x20   let w = Wrap::Tags(inner);\n\
         \x20   let mutable xs: Vector<Wrap> = vector_new();\n\
         \x20   xs = push(xs, w);\n\
         \x20   return 0;\n\
         }");
    assert_eq!(r, 0, "main returns 0");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "nested Vector<Wrap(Tags: Vector<String>)> drop must recurse 2 tiers"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-0082 B-α Slice D-1a (Vector<Enum> pop by-value) — Mentor O.
// `Nullable(Enum)` shares the enum's own disc-sentinel slot layout (no tag
// word, ADR-0065 Lát 1) — pop needs only `enum_slots` routing at the 3
// marshal sites (out_ptr, fat dest-bind, T9 8B dest-bind), no ABI redesign.
// `Vector<Struct>` pop STAYS REFUSED (`vector_struct_pop_refused_at_jit`
// below) — its `Nullable(Struct)` dest uses a tag-prepended slot (ADR-0076
// Phương án A) that the existing marshal never wrote correctly; deferred to
// Slice D-1b.
// ─────────────────────────────────────────────────────────────────────────

/// REPURPOSES the former `vector_enum_pop_refused_at_jit` (LUẬT 3: Slice D-1a
/// deliberately OPENS Enum pop — the old refuse assertion is now false BY
/// DESIGN, not a regression; `vector_struct_pop_refused_at_jit` below keeps
/// the still-true Struct half of the old guard). Two heap-bearing `Msg::Text`
/// pushed, `pop` moves the LIFO-last element ("bb") out — the popped local
/// frees it at match-arm scope-end Drop, and the SURVIVOR ("aa") is freed by
/// `xs`'s own Drop at function end. Each String freed exactly once → 2.
#[test]
fn vector_enum_pop_frees_active_arm_and_survivor() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Msg> = vector_new();\n\
         \x20   xs = push(xs, Msg::Text(\"aa\"));\n\
         \x20   xs = push(xs, Msg::Text(\"bb\"));\n\
         \x20   let out = pop(xs);\n\
         \x20   return match out {\n\
         \x20       ~+ msg => 0,\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 0, "main returns 0 (~+ arm taken, non-empty pop)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        2,
        "popped element (\"bb\", via match-arm scope-end Drop) + survivor \
         (\"aa\", via xs's own Drop) — each String freed exactly once. \
         == 0/1 ⇒ dest leaf-marshal leak (enum_slots branch missing); \
         == 3 ⇒ double-free of the POPPED element only (source cell not \
         tombstoned by len-- — the survivor is unaffected, so 2+1 not 2×2)."
    );
}

/// Empty-vector pop on a HEAP-BEARING Enum (fat, stride>8) — the shim writes
/// `NULL_SENTINEL` to `out_ptr[0]` (disc@0, no tag for `Enum?`), the `~0`
/// match arm must be taken, and NOTHING gets freed (no element was ever
/// popped). Proves the fat out_ptr routing (enum_slots, slot+0) survives the
/// empty case, not just the non-empty memcpy case above.
#[test]
fn vector_enum_pop_empty_returns_null_sentinel_no_leak() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Msg { Text(String), Empty }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Msg> = vector_new();\n\
         \x20   let out = pop(xs);\n\
         \x20   return match out {\n\
         \x20       ~+ msg => 1,\n\
         \x20       ~0 => 7,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 7, "~0 arm taken on empty-vector pop");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "nothing was popped from an empty vector — zero frees"
    );
}

/// T9 8B-path content correctness: a disc-only enum (`Color`, total_size==8,
/// no heap payload) rides the scalar-return path (`vector_pop_fat=false`),
/// which writes `ret_val` into the `enum_slots` StackSlot (not the Cranelift
/// Variable). Push Green then Red (LIFO), pop must return the REAL
/// discriminant (Red), not garbage — matched via nested `match` to a
/// distinguishing return code. A wrong T9-enum write would read back
/// whatever stale bytes sat in the slot (likely disc=0/Red by accident is
/// too weak a signal alone, so this also asserts zero string frees, proving
/// no heap leaf was ever touched).
#[test]
fn vector_enum_scalar_pop_returns_correct_disc() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("enum Color { Red, Green, Blue }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Color> = vector_new();\n\
         \x20   xs = push(xs, Color::Green);\n\
         \x20   xs = push(xs, Color::Red);\n\
         \x20   let out = pop(xs);\n\
         \x20   return match out {\n\
         \x20       ~+ c => match c {\n\
         \x20           Color::Red => 1,\n\
         \x20           Color::Green => 2,\n\
         \x20           Color::Blue => 3,\n\
         \x20       },\n\
         \x20       ~0 => 99,\n\
         \x20   };\n\
         }");
    assert_eq!(
        r, 1,
        "popped element must be the LIFO-last push (Red=1), not garbage \
         (T9 enum_slots write missing/wrong ⇒ misroutes to Green/Blue/~0)"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        0,
        "disc-only enum has no heap leaf — zero string frees"
    );
}

/// T-REFUSE-STRUCT-POP (Slice A REGRESSION): `pop` of a `Vector<Struct>` element
/// was a PRE-EXISTING latent double-free/invalid-pointer in Slice A (never
/// guarded, never tested). The AM1 refuse guard closes it. Poison: remove the
/// guard → this compiles (and, run, corrupts the popped struct's String handle).
#[test]
fn vector_struct_pop_refused_at_jit() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let err = compile_expect_refuse(
        "struct User { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<User> = vector_new();\n\
         \x20   let a = User { name: \"aa\" };\n\
         \x20   xs = push(xs, a);\n\
         \x20   let popped = pop(xs);\n\
         \x20   return 0;\n\
         }",
    );
    assert!(
        err.contains("move-out") || err.contains("deferred"),
        "Vector<Struct> pop must refuse (Slice A hole closed), got: {err}"
    );
}

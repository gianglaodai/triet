//! ADR-0079 §AMEND Slice 2 — get_ref BORROW of an aggregate element
//! (`Vector<Agg>` / `HashMap<scalarK, Agg>`, `Agg` heap-bearing).
//!
//! Free-count teeth (route-lower via `lower_source`, real allocator shims for
//! the vector/hashmap machinery, a records-only `__triet_string_free`
//! counting stub). This is the counting evidence behind the "Khải Hoàn Môn"
//! fixture (390): `get(&0 v, i)` on a `Vector<Tagged{name:String}>` is the
//! sound alternative to the E1049-refused get-by-value.
//!
//! WHAT THIS FILE PROVES (and only this): the element's String is freed
//! EXACTLY ONCE — by the container's own drop-glue — across repeated
//! borrows. A borrow that tombstoned/detached the element, or one whose
//! drop-glue lost the element, reads as 0 (leak — câm at the exit-code
//! level, only the COUNT catches it).
//!
//! ⚠️ HONEST LIMIT — this count does NOT witness the MINE-1 double-free.
//! D measured it (2026-07-17): with BOTH the MINE-1 poison (forcing the `&0`
//! aggregate route to the bitwise `_get_copy`) AND the JIT F3 heap-bearing
//! defense disabled, this count STAYS 1 — it does not go to 2. Reason, from
//! the MIR: `length(t.name)` lowers to `_16 = move _15.name` — the read
//! MOVES the String field out of the copy, so the copy's `Drop` sees a
//! tombstoned field and the aliased pointer is simply never freed a second
//! time (the alias leaks into a never-dropped temp instead). The
//! double-free direction is therefore NOT observable through this harness.
//! The route is guarded by TWO other teeth-verified layers instead:
//! typecheck E1049 and the JIT F3 defense (poisoning MINE-1 alone makes
//! fixture 390 red on the F3 refuse). Do not add a `== 2 ⇒ double-free`
//! claim here without first proving it can actually fire.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Counting stand-in for `__triet_string_free` (mirrors the real
/// null/sentinel guard so only LIVE frees count).
#[unsafe(no_mangle)]
extern "C" fn __gra_str_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_2_1("__triet_vector_get_ref", mir_lower::__triet_vector_get_ref),
        // ADR-0079 §AMEND Slice 2: the aggregate get_ref shims are DISTINCT
        // Rust functions (never deref a stride<=8 cell — an aggregate's cell
        // holds the struct's bits, not a handle), unlike `_get_copy` which
        // reuses `_get_ref` under a second symbol name.
        ShimSymbol::fn_2_1(
            "__triet_vector_get_ref_agg",
            mir_lower::__triet_vector_get_ref_agg,
        ),
        // The get-by-value COPY shims are registered here NOT because a
        // healthy tree ever calls them from a `&0` get — it does not — but so
        // the MINE-1 poison (forcing the `&0` aggregate route to `_get_copy`)
        // reaches the free-COUNT assertion below instead of dying earlier on
        // "shim not registered". Without these, the teeth would be vacuous:
        // red for the wrong reason, proving nothing about the count.
        ShimSymbol::fn_2_1("__triet_vector_get_copy", mir_lower::__triet_vector_get_ref),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_copy",
            mir_lower::__triet_hashmap_get_ref,
        ),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref",
            mir_lower::__triet_hashmap_get_ref,
        ),
        ShimSymbol::fn_2_1(
            "__triet_hashmap_get_ref_agg",
            mir_lower::__triet_hashmap_get_ref_agg,
        ),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_2_1("__triet_string_hash", mir_lower::__triet_string_hash),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
        // Leaf free is the COUNTING stub (the teeth surface).
        ShimSymbol::fn_2_0("__triet_string_free", __gra_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// T-VEC-BORROW-ONCE (the Khải Hoàn Môn count): a heap-bearing `Tagged` is
/// pushed into a `Vector`, then borrowed TWICE via `get(&0 xs, 0)` and its
/// String field read each time. The String must be freed EXACTLY once — by
/// `xs`'s own drop-glue at scope end.
///
/// - `== 0` ⇒ the borrow detached/tombstoned the element, or the container's
///   drop-glue lost it (leak — câm at the exit-code level).
/// - The `== 2` (double-free) direction is NOT witnessed here — see the
///   module-level "HONEST LIMIT" note; E1049 + the JIT F3 defense are the
///   teeth for that route.
#[test]
fn vector_aggregate_get_ref_borrow_twice_frees_string_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Tagged { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable xs: Vector<Tagged> = vector_new();\n\
         \x20   xs = push(xs, Tagged { name: \"abc\" });\n\
         \x20   let n1 = match get(&0 xs, 0) {\n\
         \x20       ~+ t => length(t.name),\n\
         \x20       ~0 => -1,\n\
         \x20   };\n\
         \x20   let n2 = match get(&0 xs, 0) {\n\
         \x20       ~+ t => length(t.name),\n\
         \x20       ~0 => -1,\n\
         \x20   };\n\
         \x20   return n1 + n2;\n\
         }");
    assert_eq!(
        r, 6,
        "both borrows read the live element's String (len 3each)"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "ADR-0079 §AMEND Slice 2: a get_ref BORROW is zero-copy and \
         non-destructive — the element's String is freed exactly once, by \
         the container's drop-glue. == 0 ⇒ borrow detached the element, or \
         drop-glue lost it (leak). (The double-free direction is not \
         observable here — see the module-level note.)"
    );
}

/// T-HM-BORROW-ONCE: HashMap sibling of the above — same claim through the
/// `__triet_hashmap_get_ref_agg` key-marshal path.
#[test]
fn hashmap_aggregate_get_ref_borrow_frees_string_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run("struct Tagged { name: String }\n\
         function main() -> Integer = {\n\
         \x20   let mutable m: HashMap<Integer, Tagged> = hashmap_new();\n\
         \x20   m = insert(m, 1, Tagged { name: \"abcd\" });\n\
         \x20   return match get(&0 m, 1) {\n\
         \x20       ~+ t => length(t.name),\n\
         \x20       ~0 => -1,\n\
         \x20   };\n\
         }");
    assert_eq!(r, 4, "borrow reads the live value's String (len 4)");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "HashMap value borrow is zero-copy + non-destructive — String freed \
         exactly once by the map's drop-glue. == 0 ⇒ leak. (The double-free \
         direction is not observable here — see the module-level note.)"
    );
}

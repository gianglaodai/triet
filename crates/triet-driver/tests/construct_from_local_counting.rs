//! ADR-0067 AMEND — route-lower free-count teeth for CONSTRUCTION-INTO-FIELD
//! from a NAMED LOCAL. This is the live HEAD double-free Mentor O found:
//!
//!     let i = Inner { name: "Giang" };       // heap-owning struct in a local
//!     let h = Holder { inner: i, tag: 5 };   // move `i` INTO a field
//!     return h.tag;                          // scope-end Drop(i) + Drop(h)
//!
//! Before the fix, the JIT aggregate byte-copy (`mir_lower.rs:1759`) did NOT
//! tombstone the source local `i`, so BOTH `i` and `h.inner` owned the same
//! String ptr → scope-end freed it twice (SIGABRT 134 with the real shim;
//! count==2 with the records-only shim here). Inline construction
//! (`Holder { inner: Inner { .. } }`) escaped because the inner aggregate is a
//! TEMP with no scope-end Drop — which is why fixtures 263/264 stayed green and
//! the hole slipped past ADR-0067 §2a/2b+.
//!
//! The fix (Option A, lower-side): `lower_expr`'s StructLiteral path emits a
//! `Deinit(field_val)` immediately after the field Assign when the field is a
//! nested struct OR a nested enum, atomic in the same BB. The JIT's existing
//! recursive Deinit tombstone (collect_heap_leaves / 2b-3 enum payload zero)
//! then makes the moved-from local's Drop a no-op.
//!
//! Teeth (Mentor O, on the final tree):
//! - struct_from_local_frees_once / enum_from_local_frees_once: FREE_COUNT==1.
//!   Poison = comment the AMEND `Deinit(field_val)` push in
//!   `triet-lower/src/lib.rs` → BOTH flip to count==2 (the double-free returns).
//!   This pins the Deinit as LOAD-BEARING (poison-must-be-red).
//! - inline_construct_frees_once: regression guard — the inline temp path must
//!   stay FREE_COUNT==1 (the AMEND must not over-free the temp).
//!
//! ⚠ Records-only shims (no real dealloc) + process-global AtomicUsize: a Mutex
//! serializes the shared counter (the gate runs `cargo test` in parallel).
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_CAP: AtomicI64 = AtomicI64::new(-1);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __cfl_str_free(ptr: i64, cap: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_CAP.store(cap, Ordering::SeqCst);
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
        ShimSymbol::fn_2_0("__triet_string_free", __cfl_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const STRUCT_DECL: &str = "struct Inner { name: String }\n\
                           struct Holder { inner: Inner, tag: Integer }\n";

const ENUM_DECL: &str = "enum Msg { Text(String), Empty }\n\
                         struct Wrapper { msg: Msg, tag: Integer }\n";

/// The live HEAD double-free: construct a heap-owning STRUCT from a named local,
/// then move it into a field. Must free the String exactly once. Poison the
/// AMEND Deinit → count==2.
#[test]
fn struct_from_local_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    STR_CAP.store(-1, Ordering::SeqCst);
    let r = run(&format!(
        "{STRUCT_DECL}function main() -> Integer = {{\n\
         \x20   let i = Inner {{ name: \"Giang\" }};\n\
         \x20   let h = Holder {{ inner: i, tag: 5 }};\n\
         \x20   return h.tag;\n\
         }}"
    ));
    assert_eq!(
        r, 5,
        "scalar field after the nested heap struct must read 5"
    );
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "AMEND ADR-0067: a heap struct moved from a named local into a field must \
         free its String exactly once (Deinit tombstones the moved-from local)"
    );
    assert_eq!(
        STR_CAP.load(Ordering::SeqCst),
        5,
        "the freed cap must be the REAL cap (5 for \"Giang\"), not stack garbage"
    );
}

/// Same hole for an ENUM-owning named local moved into a struct field.
#[test]
fn enum_from_local_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{ENUM_DECL}function main() -> Integer = {{\n\
         \x20   let m = Msg::Text(\"Giang\");\n\
         \x20   let w = Wrapper {{ msg: m, tag: 5 }};\n\
         \x20   return w.tag;\n\
         }}"
    ));
    assert_eq!(r, 5);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "AMEND ADR-0067: an enum moved from a named local into a field must free \
         its payload exactly once"
    );
}

/// R-atomic (LUẬT THÉP): the Deinit of a moved-from heap aggregate local must
/// sit IMMEDIATELY after the field Assign in the SAME block — no statement may
/// slip between the byte-copy and the tombstone, or a future CFG split could
/// free twice. Structural assertion on the lowered MIR (more durable than a
/// textual grep on the dump). Scalar/`String` field stores are excluded: they
/// carry no Deinit (the JIT M1-zeroing path handles the scalar leaf).
#[test]
fn deinit_is_atomic_after_field_assign() {
    use triet_mir::{MirType, Projection, Statement};
    let bodies = lower_source(&format!(
        "{STRUCT_DECL}function main() -> Integer = {{\n\
         \x20   let i = Inner {{ name: \"Giang\" }};\n\
         \x20   let h = Holder {{ inner: i, tag: 5 }};\n\
         \x20   return h.tag;\n\
         }}"
    ));
    let main = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .expect("main body");
    let mut found = false;
    for block in &main.blocks {
        for (i, stmt) in block.statements.iter().enumerate() {
            // A construction-into-field move of a heap AGGREGATE: a single-field
            // projection dest with a plain-local source whose type is a
            // Struct/Enum (not a scalar field, not the `String` heap leaf).
            let Statement::Assign { dest, source, .. } = stmt else {
                continue;
            };
            if !matches!(dest.projection.as_slice(), [Projection::Field(_)])
                || !source.projection.is_empty()
            {
                continue;
            }
            let src_ty = &main.local_decls[source.local.0].ty;
            let is_heap_aggregate = matches!(src_ty, MirType::Struct(n) if n != "String")
                || matches!(src_ty, MirType::Enum(_));
            if !is_heap_aggregate {
                continue;
            }
            // The VERY NEXT statement MUST be Deinit of the moved-from local.
            let next = block.statements.get(i + 1);
            assert!(
                matches!(next, Some(Statement::Deinit(l, _)) if *l == source.local),
                "AMEND ADR-0067 LUẬT THÉP: a field Assign of a heap aggregate \
                 local must be IMMEDIATELY followed by Deinit of that local \
                 (same BB, no statement between); got next = {next:?}"
            );
            found = true;
        }
    }
    assert!(
        found,
        "expected a construction-into-field Assign of a heap aggregate in main"
    );
}

/// Regression guard: inline construction (the inner aggregate is a TEMP with no
/// scope-end Drop) must STILL free exactly once — the AMEND Deinit on the temp
/// must not double-count or over-free.
#[test]
fn inline_construct_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(&format!(
        "{STRUCT_DECL}function main() -> Integer = {{\n\
         \x20   let h = Holder {{ inner: Inner {{ name: \"Giang\" }}, tag: 5 }};\n\
         \x20   return h.tag;\n\
         }}"
    ));
    assert_eq!(r, 5);
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "inline construction must remain FREE_COUNT==1 after the AMEND"
    );
}

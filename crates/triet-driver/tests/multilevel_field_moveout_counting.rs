//! WO-0075 (ADR-0070 §AMEND Phase 3) tooth-H — route-lower free-count tooth for
//! a MULTI-LEVEL heap-field MOVE-OUT `let x = h.inner.x`. The leaf String moves
//! into a fresh local; the base `h` is later dropped (it still owns the Copy
//! field `n`). The base's recursive Drop must NOT free the moved leaf (Site-G
//! tombstones the leaf at its ABSOLUTE offset in the base slot), so the String
//! frees EXACTLY once across `Drop(h)` + `Drop(x)`.
//!
//! Tooth (Mentor O re-verifies independently on the final tree):
//!   - H no-double-free: revert the Site-G gate widen (`mir_lower.rs`, restore
//!     `[Projection::Field(_)]` exactly-1) → the 2-Field path `h.inner.x` no
//!     longer matches the tombstone gate → the base slot keeps the moved leaf
//!     ptr → `Drop(h)` frees it AND `Drop(x)` frees it → STR_FREES == 2 (a clean
//!     RED count, the recording shim never deallocs).
//!
//! Site-H probe (Lower): this test ALSO proves `place_result_type` resolves the
//! multi-level leaf type — if the dest `x` were Unknown-typed it would get no
//! JIT slot and the aggregate copy would SIGSEGV instead of returning 7. A clean
//! run (FREE==1, r==7) is the in-suite witness that Site-H needs no change.
//!
//! ⚠ RAM: process-global AtomicUsize + no-mangle shims (N7 fork-bomb hazard).
//! `TEST_LOCK` serializes the reset-then-read window; recording-only shims (no
//! real dealloc) so a poisoned double-free is an observable count, not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __ml_str_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_0("__triet_string_free", __ml_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const SRC: &str = "struct Inner { x: String, n: Integer }\n\
                   struct Holder { inner: Inner, n: Integer }\n\
                   function main() -> Integer = {\n\
                   \x20   let i = Inner { x: \"hi\", n: 1 };\n\
                   \x20   let h = Holder { inner: i, n: 7 };\n\
                   \x20   let x = h.inner.x;\n\
                   \x20   return h.n;\n\
                   }";

/// Site-H probe (explicit MIR check, not a run-witness): lower SRC and confirm
/// the multi-level move-out dest carries the REAL leaf type `String`, not
/// `Unknown`. `place_result_type` (triet-lower) loops every Field projection, so
/// `h.inner.x` resolves Inner → x:String → the dest gets a typed slot. An
/// Unknown-typed dest would get no JIT slot → SIGSEGV; this pins the cause, not
/// just the symptom.
#[test]
fn multilevel_dest_is_typed_not_unknown() {
    use triet_mir::{MirType, Projection, Statement};
    let bodies = lower_source(SRC);
    let main = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .expect("main body");
    // Find the Assign whose source is the 2-Field path `h.inner.x`.
    let dest_local = main
        .blocks
        .iter()
        .flat_map(|bb| bb.statements.iter())
        .find_map(|s| match s {
            Statement::Assign { dest, source, .. }
                if source.projection.len() == 2
                    && source
                        .projection
                        .iter()
                        .all(|p| matches!(p, Projection::Field(_))) =>
            {
                Some(dest.local)
            }
            _ => None,
        })
        .expect("multi-level move-out Assign `x = h.inner.x` not found in MIR");
    let dest_ty = &main.local_decls[dest_local.0].ty;
    assert_eq!(
        dest_ty,
        &MirType::String,
        "Site-H: multi-level move-out dest must be typed String (got {dest_ty:?}); \
         an Unknown-typed dest gets no slot → SIGSEGV"
    );
}

/// tooth-H: `let x = h.inner.x` (2-Field path) moves the leaf String out; the
/// base `h` is dropped (still owns `n`). FREE must be EXACTLY 1.
#[test]
fn multilevel_field_moveout_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC);
    assert_eq!(r, 7, "base must still own the Copy field n after move-out");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "WO-0075: a multi-level moved-out String leaf must free EXACTLY once \
         (base recursive Drop sees the tombstoned leaf ptr=0)"
    );
}

//! WO-0074 (Phase 3 — Nợ A) — route-lower free-count teeth for a single
//! HEAP-CARRYING ENUM field MOVE-OUT `let e = h.msg`. The enum field's value
//! moves into a fresh local `e`; the base `h` is later dropped (it still owns
//! the Copy field `n`). The base's tag-switch Drop glue must NOT free the moved
//! enum's heap payload (Site-3 tombstones the payload ptr@field_off+8), so the
//! String payload frees EXACTLY once across `Drop(h)` + `Drop(e)`.
//!
//! Teeth (Mentor O re-verifies independently on the final tree):
//!   - tooth-2 no-double-free (⚔ G): remove the Site-3 `MirType::Enum(_)` arm
//!     in mir_lower's move-out tombstone block → the base slot keeps the moved
//!     payload ptr → `Drop(h)` frees it AND `Drop(e)` frees it → STR_FREES == 2
//!     (a clean RED count, not a segfault — the recording shim never deallocs).
//!   - tooth-3 leak (negative pole of tooth-2): if `e` never drops or the disc
//!     is wrong, STR_FREES == 0. The `== 1` assertion catches BOTH poles.
//!   - tooth-4 cap (G #2): the SAME test asserts STR_CAP == 5 (cap of "Giang",
//!     a 5-byte string) AND STR_FREES == 1. The 16/32B enum byte-copy preserves
//!     cap through the aggregate move; a wrong byte → cap != 5 → panic.
//!
//! ⚠ RAM: process-global AtomicUsize/AtomicI64 + no-mangle shims (N7 fork-bomb
//! hazard). `TEST_LOCK` serializes the reset-then-read window; recording-only
//! shims (no real dealloc) so a poisoned double-free is an observable count,
//! not a crash.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static STR_CAP: AtomicI64 = AtomicI64::new(-1);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __mo_enum_str_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_0("__triet_string_free", __mo_enum_str_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

const SRC: &str = "enum Msg { Text(String), Code(Integer) }\n\
                   struct Holder { msg: Msg, n: Integer }\n\
                   function main() -> Integer = {\n\
                   \x20   let h = Holder { msg: Msg::Text(\"Giang\"), n: 7 };\n\
                   \x20   let e = h.msg;\n\
                   \x20   return h.n;\n\
                   }";

/// teeth 2/3/4 in one test (G #2 mandate: cap + count together):
///   - `r == 7` — base still owns the Copy field `n` after the partial move.
///   - `STR_FREES == 1` — moved-out enum payload frees EXACTLY once: not 2
///     (double-free, Site-3 tombstone dropped) and not 0 (leak / wrong disc).
///   - `STR_CAP == 5` — the aggregate enum byte-copy preserved the real cap of
///     "Giang" (5 bytes) into `e`'s slot; the base tombstone only zeroes the
///     payload ptr, so `Drop(e)` frees with the REAL cap, not stack garbage.
#[test]
fn enum_field_moveout_frees_once_with_cap() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    STR_CAP.store(-1, Ordering::SeqCst);
    let r = run(SRC);
    assert_eq!(r, 7, "base must still own the Copy field n after move-out");
    assert_eq!(
        STR_FREES.load(Ordering::SeqCst),
        1,
        "WO-0074: moved-out enum String payload must free EXACTLY once \
         (base tag-switch Drop sees the tombstoned payload ptr=0)"
    );
    assert_eq!(
        STR_CAP.load(Ordering::SeqCst),
        5,
        "WO-0074: the enum byte-copy must preserve the REAL cap (5 for \"Giang\"), \
         not stack garbage"
    );
}

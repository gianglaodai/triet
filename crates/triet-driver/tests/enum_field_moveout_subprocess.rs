//! WO-0074 (Phase 3 — Nợ A) tooth-5 ⚔ (G #1 mandate: IN-SUITE, no manual run).
//!
//! Site-1 (`triet-lower`) types the enum field move-out dest `let e = h.msg` as
//! `MirType::Enum(_)` so the JIT pre-pass allocates an enum stack slot. WITHOUT
//! that typing the dest is Unknown-typed → NO slot → the aggregate enum copy
//! writes through a garbage `use_var` address → SIGSEGV.
//!
//! A segfault aborts the whole process, so the scenario runs in a SUBPROCESS
//! (mirror of `capability_defer_trap.rs`): the parent spawns `current_exe` with
//! the test name + `--exact --test-threads=1` + the `_TRIET_EFM` env marker; the
//! child JIT-runs `let e = h.msg` and either returns cleanly (patched tree →
//! exit 0) or dies from SIGSEGV (poisoned Site-1 → signal 11 / code 139). The
//! crash is CONTAINED in the child, so a no-slot regression does NOT kill the
//! test runner — the parent asserts `status.success()`.
//!
//! Tooth (Mentor O re-verifies independently on the final tree):
//! - R-no-slot-segv: revert the Site-1 `matches!(field_ty, MirType::Enum(_))`
//!   branch in `triet-lower` → the child segfaults (signal 11) → `success()`
//!   fails → RED. The patched tree → child exit 0 → GREEN.
#![allow(unsafe_code)]

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

const SRC: &str = "enum Msg { Text(String), Code(Integer) }\n\
                   struct Holder { msg: Msg, n: Integer }\n\
                   function main() -> Integer = {\n\
                   \x20   let h = Holder { msg: Msg::Text(\"Giang\"), n: 7 };\n\
                   \x20   let e = h.msg;\n\
                   \x20   return h.n;\n\
                   }";

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Lower + JIT-compile + run `main` for the enum-field move-out program, with
/// the REAL String shims.
fn run_enum_field_moveout() -> i64 {
    let bodies = lower_source(SRC);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = [
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", mir_lower::__triet_string_free),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Child guard: if `_TRIET_EFM` matches `test_name`, run `child_fn` then exit.
/// Otherwise return (the parent goes on to spawn). Prevents a fork-bomb from
/// the `--exact` race.
fn efm_child_guard(test_name: &str, child_fn: impl FnOnce()) {
    if let Ok(name) = std::env::var("_TRIET_EFM") {
        if name == test_name {
            child_fn();
        }
        std::process::exit(0);
    }
}

/// Spawn this test binary running ONLY `test_name`, single-threaded, with the
/// `_TRIET_EFM` marker set so the child guard fires.
fn spawn_efm_child(test_name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env("_TRIET_EFM", test_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
}

/// R-no-slot-segv: the patched tree types the move-out dest as `Enum(_)` → the
/// JIT allocates an enum slot → the aggregate copy lands in a real slot → the
/// child returns 7 and exits cleanly. Revert Site-1 → no slot → SIGSEGV in the
/// child → `success()` fails → RED.
#[test]
fn enum_field_moveout_has_slot() {
    efm_child_guard("enum_field_moveout_has_slot", || {
        let r = run_enum_field_moveout();
        assert_eq!(r, 7, "enum field move-out must run to completion");
    });
    let status = spawn_efm_child("enum_field_moveout_has_slot");
    assert!(
        status.success(),
        "Site-1 enum-typing must give the move-out dest a slot so the aggregate \
         copy does NOT segfault (got {status:?})"
    );
}

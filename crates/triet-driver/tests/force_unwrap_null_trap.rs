//! ADR-0089 Slice 2c (ForceUnwrap `!!`, Mirror-Elvis, ADR-0041 §AMEND) — null
//! arm teeth. `expr!!` on a null (`~0`) nullable MUST trap (SIGILL), never
//! silently produce a garbage value.
//!
//! Corresponds to WO fixtures 496 (`Integer?` null) and 498 (`String?` null).
//! These do NOT live in `tests/fixtures/*.tri` because that corpus is run
//! IN-PROCESS by `integration_test_corpus` (integration_tests.rs) — a real
//! SIGILL there would abort the whole test binary, taking every other test
//! down with it (confirmed: `triet-driver run` on this source exits 132 =
//! 128+4 = SIGILL). Mirrors the existing `capability_defer_trap.rs` /
//! `enum_field_moveout_subprocess.rs` pattern: run the trap in a spawned
//! child process and assert on its exit signal, so a poisoned tree fails
//! loudly instead of killing the harness.
//!
//! Teeth (Mentor O re-verifies independently on the final tree):
//! - R-null-traps: the null branch of the ForceUnwrap lowering must reach
//!   `Terminator::Trap`. Poison: replace the null-arm `Trap` with a `Goto` to
//!   the present arm (or drop the branch) → child returns a value instead of
//!   dying → `status.success()` → the `assert_fu_signal` check fails (RED).
#![allow(unsafe_code)]

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

const SRC_INTEGER_NULL: &str = "function f() -> Integer? = ~0\n\
                                 function main() -> Integer {\n\
                                 \x20   let x = f();\n\
                                 \x20   return x!!;\n\
                                 }\n";

const SRC_STRING_NULL: &str = "function f() -> String? = ~0\n\
                                function main() -> Integer {\n\
                                \x20   let x = f();\n\
                                \x20   return len(x!!);\n\
                                }\n";

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Lower + JIT-compile + run `main` for the given source, with the real
/// String shims registered (String? case needs `len`'s shim; the Integer?
/// case never calls any shim, but registering both is harmless).
fn run_force_unwrap(source: &str) -> i64 {
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
        ShimSymbol::fn_2_0("__triet_string_free", mir_lower::__triet_string_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Child guard: if `_TRIET_FU` matches `test_name`, run `child_fn` then exit.
/// Otherwise return (the parent goes on to spawn). Prevents a fork-bomb from
/// the `--exact` race.
fn fu_child_guard(test_name: &str, child_fn: impl FnOnce()) {
    if let Ok(name) = std::env::var("_TRIET_FU") {
        if name == test_name {
            child_fn();
        }
        std::process::exit(0);
    }
}

/// Spawn this test binary running ONLY `test_name`, single-threaded, with the
/// `_TRIET_FU` marker set so the child guard fires.
fn spawn_fu_child(test_name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env("_TRIET_FU", test_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
}

/// Assert the child died from signal `expected` (4 = SIGILL from `trapnz`).
fn assert_fu_signal(test_name: &str, status: std::process::ExitStatus, expected: i32) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(expected),
            "{test_name}: expected signal {expected}, got {:?} (success={})",
            status.signal(),
            status.success()
        );
    }
    #[cfg(not(unix))]
    {
        assert!(!status.success(), "{test_name}: child should have trapped");
    }
}

/// Fixture 496: `Integer?` = `~0` (null), `!!` must trap — SIGILL, not a
/// silently-returned `i64::MIN` sentinel value.
#[test]
fn force_unwrap_null_integer_traps() {
    fu_child_guard("force_unwrap_null_integer_traps", || {
        let _ = run_force_unwrap(SRC_INTEGER_NULL); // SIGILL fires before return
    });
    let status = spawn_fu_child("force_unwrap_null_integer_traps");
    assert_fu_signal("force_unwrap_null_integer_traps", status, 4);
}

/// Fixture 498: `String?` = `~0` (null), `!!` must trap — SIGILL.
#[test]
fn force_unwrap_null_string_traps() {
    fu_child_guard("force_unwrap_null_string_traps", || {
        let _ = run_force_unwrap(SRC_STRING_NULL); // SIGILL fires before return
    });
    let status = spawn_fu_child("force_unwrap_null_string_traps");
    assert_fu_signal("force_unwrap_null_string_traps", status, 4);
}

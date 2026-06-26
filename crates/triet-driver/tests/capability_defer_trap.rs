//! ADR-0069 Lát 3 — Defer runtime-hook trap teeth (§5 LOCK: check at mint-site,
//! fail-closed). A `defer` capability mint emits ONE `__triet_cap_check(cap_id)`
//! call + a fail-closed trap: result ≤ 0 (Deny −1 OR Unknown 0) → `trapnz`
//! `unwrap_user(2)` → SIGILL (signal 4). Result > 0 (Grant +1) → token flows.
//!
//! A trap aborts the whole process, so each scenario runs in a SUBPROCESS
//! (mirror of the JIT N7 pattern): the parent spawns `current_exe` with the
//! test name + `--exact --test-threads=1` + the `_TRIET_CAP` env marker; the
//! child sets the process-global policy, JIT-runs the defer mint, and either
//! returns 0 (allow) or dies SIGILL (deny / unknown). The parent asserts the
//! exit status.
//!
//! Teeth (Mentor O re-verifies independently on the final tree):
//! - R-defer-deny-trap: policy=−1 → SIGILL. Poison: drop the `trapnz` → no trap
//!   → `defer_deny_traps` goes green (child exits 0).
//! - R-fail-closed (G's spine): policy=0 (Unknown) → MUST trap. Poison the icmp
//!   `≤0` → `<0` (only Deny traps, Unknown leaks) → `defer_unknown_traps` green.
//! - R-allow-flows: policy=+1 → runs, no trap. Poison always-trap → green flips
//!   to a SIGILL the success assert catches.
#![allow(unsafe_code)]

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

const SRC: &str = "capability Cap defer\n\
                   function main() -> Integer {\n\
                   \x20   let a = mint Cap;\n\
                   \x20   return 0;\n\
                   }\n";

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Lower + JIT-compile + run `main` for the defer-mint program, with the REAL
/// `__triet_cap_check` shim. The caller sets the policy first.
fn run_defer_mint() -> i64 {
    let bodies = lower_source(SRC);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = [ShimSymbol::fn_1_1(
        "__triet_cap_check",
        mir_lower::__triet_cap_check,
    )];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Child guard: if `_TRIET_CAP` matches `test_name`, run `child_fn` then exit.
/// Otherwise return (the parent goes on to spawn). Prevents a fork-bomb from
/// the `--exact` race.
fn cap_child_guard(test_name: &str, child_fn: impl FnOnce()) {
    if let Ok(name) = std::env::var("_TRIET_CAP") {
        if name == test_name {
            child_fn();
        }
        std::process::exit(0);
    }
}

/// Spawn this test binary running ONLY `test_name`, single-threaded, with the
/// `_TRIET_CAP` marker set so the child guard fires.
fn spawn_cap_child(test_name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env("_TRIET_CAP", test_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
}

/// Assert the child died from signal `expected` (4 = SIGILL from `trapnz`).
fn assert_cap_signal(test_name: &str, status: std::process::ExitStatus, expected: i32) {
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

/// R-allow-flows: policy = +1 (Grant) → defer mint runs, no trap, returns 0.
#[test]
fn defer_allow_runs() {
    cap_child_guard("defer_allow_runs", || {
        mir_lower::__set_cap_policy(1);
        let r = run_defer_mint();
        assert_eq!(r, 0, "allow policy must run main to completion");
    });
    let status = spawn_cap_child("defer_allow_runs");
    assert!(
        status.success(),
        "policy=+1 (Grant) → defer mint must run without trapping (got {status:?})"
    );
}

/// R-defer-deny-trap: policy = −1 (Deny) → SIGILL at the mint-site gate.
#[test]
fn defer_deny_traps() {
    cap_child_guard("defer_deny_traps", || {
        mir_lower::__set_cap_policy(-1);
        let _ = run_defer_mint(); // SIGILL fires before the return
    });
    let status = spawn_cap_child("defer_deny_traps");
    assert_cap_signal("defer_deny_traps", status, 4);
}

/// R-fail-closed (G's spine): policy = 0 (Łukasiewicz Unknown) → MUST trap.
/// Unknown is "not proven allowed" → fail-closed, NOT a silent pass.
#[test]
fn defer_unknown_traps() {
    cap_child_guard("defer_unknown_traps", || {
        mir_lower::__set_cap_policy(0);
        let _ = run_defer_mint(); // SIGILL: Unknown ≤ 0 → fail-closed
    });
    let status = spawn_cap_child("defer_unknown_traps");
    assert_cap_signal("defer_unknown_traps", status, 4);
}

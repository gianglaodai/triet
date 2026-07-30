//! WO-String-Eq-Content-Compare-And-Aggregate-Refuse §5(c) — subprocess-
//! isolated CONTENT read for `String ==`/`!=`.
//!
//! `string_eq_content_counting.rs` proves the alloc/free pairing is correct
//! but only exercises SHORT strings (2-3 bytes) — every one of them fits
//! comfortably inside a single page, so even a badly wrong `len` passed to
//! `__triet_string_eq`'s `memcmp` might not actually fault. Content-compare
//! DEREFERENCES BOTH pointers for `len` bytes each; a `len` that were ever
//! wrong (garbage, or read from the wrong offset) would read out of bounds
//! and risk a real SIGSEGV (mirrors `sret_string_field_subprocess.rs`'s same
//! concern for the sret hole). A crash aborts the whole process, so this
//! runs in a subprocess (mirror of `enum_field_moveout_subprocess.rs`): the
//! parent spawns `current_exe` with the test name + `--exact
//! --test-threads=1` + an env marker; the child JIT-runs the comparison and
//! either asserts the correct result and exits cleanly, or dies (SIGSEGV /
//! panic). Either failure mode makes `status.success()` false in the parent.
#![allow(unsafe_code)]

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

const ENV_MARKER: &str = "_TRIET_SEC";

/// 80 bytes — comfortably past a single cache line / any small-string
/// optimization, well within the range where a wrong `len` reading out of
/// the allocation is likely to cross into an unmapped page under ASan-less
/// debug builds too (the allocator still tends to place larger chunks with
/// guard-adjacent unmapped regions less reliably than a huge string would,
/// but content-correctness is the primary tooth here — the crash containment
/// is defense-in-depth, not the sole assertion).
fn long_string(byte: u8) -> String {
    std::iter::repeat_n(byte as char, 80).collect()
}

fn src_long_eq(same_last_byte: bool) -> String {
    let a = long_string(b'a');
    let b = if same_last_byte {
        a.clone()
    } else {
        let mut s = long_string(b'a');
        s.replace_range(79..80, "b");
        s
    };
    format!(
        "function main() -> Integer {{\n\
         \x20   let a = \"{a}\";\n\
         \x20   let b = \"{b}\";\n\
         \x20   if a == b {{ return 1; }} else {{ return 0; }}\n\
         }}"
    )
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
        ShimSymbol::fn_2_0("__triet_string_free", mir_lower::__triet_string_free),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Child guard: if the env marker matches `test_name`, run `child_fn` then
/// exit. Otherwise return (the parent goes on to spawn). Prevents a
/// fork-bomb from the `--exact` race (N7 lesson — never spawn unfiltered).
fn child_guard(test_name: &str, child_fn: impl FnOnce()) {
    if let Ok(name) = std::env::var(ENV_MARKER) {
        if name == test_name {
            child_fn();
        }
        std::process::exit(0);
    }
}

fn spawn_child(test_name: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env(ENV_MARKER, test_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"))
}

// ══════════════════════════════════════════════════════════════════════
// P1 — two 80-byte strings, IDENTICAL content. Full-length content-compare
// must read all 80 bytes on both sides without faulting and report TRUE.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn p1_long_identical_strings_eq_true() {
    child_guard("p1_long_identical_strings_eq_true", || {
        let r = run(&src_long_eq(true));
        assert_eq!(r, 1, "80-byte identical strings must content-compare TRUE");
    });
    let status = spawn_child("p1_long_identical_strings_eq_true");
    assert!(
        status.success(),
        "child must exit cleanly (a wrong `len` reading out of bounds over \
         80 bytes would SIGSEGV instead): {status:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════
// P2 — two 80-byte strings differing ONLY in the LAST byte. Catches a shim
// (or a dispatch bug) that short-circuits on a byte-prefix match without
// reading through to the true end of `len`.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn p2_long_strings_diff_last_byte_eq_false() {
    child_guard("p2_long_strings_diff_last_byte_eq_false", || {
        let r = run(&src_long_eq(false));
        assert_eq!(
            r, 0,
            "80-byte strings differing only in the last byte must content-compare FALSE"
        );
    });
    let status = spawn_child("p2_long_strings_diff_last_byte_eq_false");
    assert!(
        status.success(),
        "child must exit cleanly (a wrong `len` reading out of bounds over \
         80 bytes would SIGSEGV instead): {status:?}"
    );
}

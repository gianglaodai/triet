//! ADR-0087 Teeth — print/println 4-overload (owned move+free /
//! `&0 String` borrow+no-free), end-to-end through the real production
//! shims, subprocess-isolated.
//!
//! # Why subprocess (mirror of `enum_field_moveout_subprocess.rs`)
//!
//! `__triet_print`/`__triet_println` (owned overload) call
//! `__triet_string_free` via a DIRECT Rust function call inside their own
//! body (`crates/triet-jit/src/mir_lower.rs`) — NOT through the JIT's
//! dynamic shim registry (`ShimSymbol`/`get_or_declare_shim`). That means a
//! registry-substituted "counting-only" shim (the `shim_arg_consumes_spof_
//! canary.rs` style — never a real `dealloc`) CANNOT observe that first
//! free: it only sees whatever the CALLER's own end-of-scope `Drop`
//! statement calls (which IS JIT-emitted and DOES go through the
//! registry). A poisoned M3-zeroing or lower-arm routing decision therefore
//! risks a REAL second `dealloc` on an already-freed pointer — genuine UB,
//! not a safely-countable-only event — so each scenario runs in its own
//! subprocess exactly like `enum_field_moveout_subprocess.rs` contains a
//! SIGSEGV. The child's real stdout is piped back (the program's own
//! `print`/`println` output); the FREE count is obtained by having the
//! child self-report via a `FREE=<n>` trailer line on STDERR (the count
//! lives in the child's own address space and cannot otherwise cross the
//! process boundary).
//!
//! The delegating counter (`__ppo_str_free`, mirrors triet-jit's own
//! `__test_counting_free`) counts the call and THEN performs the real free
//! — this matters because in the correct/baseline tree this is always a
//! harmless `free(0)` no-op (M3 already zeroed the caller's slot), but
//! under a poisoned tree it becomes a genuine double-`dealloc` on the same
//! pointer the owned shim already freed for real — the exact crash/garbage
//! signature is DATA to record at poison time (per repo convention), not
//! assumed in advance.
//!
//! # Manual poison procedure (per test, done by hand — not shipped)
//!
//! `cp` the target file to `/tmp` first, poison via `Edit`, `cargo build`
//! (rebuilds the whole workspace since the shim/meta/lowerer changed),
//! re-run ONLY the relevant `#[test]` (`--exact --test-threads=1`), observe
//! RED, restore via `cp` (NEVER `git checkout`/`restore`/`stash`), rebuild,
//! re-run to confirm GREEN again.
//!
//! - **T1** (`crates/triet-mir/src/lib.rs`, `"__triet_println"` entry):
//!   flip `arg_consumes: &[true]` → `&[false]`.
//! - **T2** (`crates/triet-jit/src/mir_lower.rs`, `__triet_println_ref`
//!   body): add a spurious `__triet_string_free(ptr, len)` call. NOT a meta
//!   flip — `&0 String` is `Copy` at the MIR level, so M3/borrowck already
//!   skip a poisoned `arg_consumes` entry for it (vacuous, mirrors the
//!   `length(&0 s)` case documented in `shim_arg_consumes_spof_canary.rs`).
//! - **T3** (`crates/triet-lower/src/lib.rs`, print/println routing arm):
//!   swap the `println`/`print` shim-name selection (println → the `print`
//!   shim and vice versa).
//! - **T4** (same routing arm): make the `is_ref` branch always select the
//!   `_ref` shim name regardless of `is_ref`.
#![allow(unsafe_code)]

use std::io::Read;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Delegating counting wrapper for `__triet_string_free` — counts the call,
/// then performs the REAL free (mirrors `__test_counting_free` in
/// triet-jit's own test module). Own symbol name (`__ppo_*`) so it never
/// collides with other test binaries' no-mangle counting shims.
#[unsafe(no_mangle)]
extern "C" fn __ppo_str_free(ptr: i64, cap: i64) {
    STR_FREES.fetch_add(1, Ordering::SeqCst);
    mir_lower::__triet_string_free(ptr, cap);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __ppo_str_free),
        ShimSymbol::fn_3_0("__triet_print", mir_lower::__triet_print),
        ShimSymbol::fn_2_0("__triet_print_ref", mir_lower::__triet_print_ref),
        ShimSymbol::fn_3_0("__triet_println", mir_lower::__triet_println),
        ShimSymbol::fn_2_0("__triet_println_ref", mir_lower::__triet_println_ref),
    ]
}

/// Delimiters sandwiching the compiled program's OWN stdout output. The
/// libtest harness that re-runs this same binary for the child process
/// writes its own "running 1 test\ntest NAME ... " progress text to the
/// SAME raw stdout fd before the test body (and therefore `run_and_report`)
/// ever runs — there is no way to suppress that from inside a `#[test]`
/// function, so the parent must extract the program's real output by
/// slicing between these markers instead of trusting the whole captured
/// stream.
const START_MARKER: &str = "\u{1}PPO_STDOUT_START\u{1}";
const END_MARKER: &str = "\u{1}PPO_STDOUT_END\u{1}";

/// Lower + verify + JIT-compile + run `main`, with the REAL production
/// print/println shims (their stdout writes go to this process's REAL
/// stdout) and the delegating counting `__triet_string_free`. Reports the
/// final count to STDERR as a `FREE=<n>` trailer, and sandwiches `main`'s
/// own stdout output between `START_MARKER`/`END_MARKER`.
fn run_and_report(source: &str) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = shims();
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(START_MARKER.as_bytes());
        let _ = out.flush();
    }
    let r = unsafe { main.call_i64_0() };
    {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(END_MARKER.as_bytes());
        let _ = out.flush();
    }
    // Raw write (not the `eprintln!` macro) — like `print!`, `eprintln!` is
    // captured/buffered by the libtest harness re-running this same test
    // binary as the child; a raw `io::stderr()` write bypasses that
    // capture and reaches the piped fd the parent reads (mirrors the
    // stdout marker rationale above).
    {
        use std::io::Write;
        let mut err = std::io::stderr();
        let _ = writeln!(err, "FREE={}", STR_FREES.load(Ordering::SeqCst));
        let _ = err.flush();
    }
    r
}

/// Extract the substring between `START_MARKER`/`END_MARKER` from the raw
/// captured stdout — discards the libtest harness's own progress text.
fn extract_program_stdout(raw: &str) -> &str {
    raw.split(START_MARKER)
        .nth(1)
        .and_then(|s| s.split(END_MARKER).next())
        .unwrap_or_default()
}

const ENV_MARKER: &str = "_TRIET_PPO";

/// Child guard: if `ENV_MARKER` matches `test_name`, run `source` and exit
/// with its return value as the process exit code. Otherwise return
/// (parent goes on to spawn). Prevents a fork-bomb from the `--exact` race.
fn child_guard(test_name: &str, source: &str) {
    if let Ok(name) = std::env::var(ENV_MARKER) {
        if name == test_name {
            let r = run_and_report(source);
            std::process::exit(i32::try_from(r).unwrap_or(1));
        }
        std::process::exit(0);
    }
}

#[derive(Debug)]
struct ChildOutput {
    success: bool,
    stdout: String,
    // Read only via the derived Debug impl in assertion failure messages
    // (dead_code analysis doesn't count Debug-formatting as a read) — kept
    // for diagnosing a failing run, not asserted on directly.
    #[allow(dead_code)]
    stderr: String,
    free_count: Option<u64>,
}

/// Spawn this test binary running ONLY `test_name`, single-threaded, with
/// the env marker set so the child guard fires. Captures both streams —
/// stdout is the program's own `print`/`println` output; stderr carries the
/// `FREE=<n>` trailer.
fn spawn_child(test_name: &str) -> ChildOutput {
    let exe = std::env::current_exe().expect("current_exe");
    let mut child = std::process::Command::new(&exe)
        .args([test_name, "--exact", "--test-threads=1"])
        .env(ENV_MARKER, test_name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|_| panic!("spawn child for {test_name}"));
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("child stdout")
        .read_to_string(&mut stdout)
        .expect("read child stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");
    let status = child.wait().expect("wait child");
    let free_count = stderr
        .lines()
        .find_map(|l| l.strip_prefix("FREE="))
        .and_then(|n| n.trim().parse::<u64>().ok());
    ChildOutput {
        success: status.success(),
        stdout,
        stderr,
        free_count,
    }
}

// ══════════════════════════════════════════════════════════════════════
// T1 — println(owned String): move+free. Poison target: builtin_shim_meta
// `"__triet_println"` arg_consumes [true] -> [false].
// ══════════════════════════════════════════════════════════════════════

const SRC_T1: &str = "function main() -> Integer = {\n\
     \x20   let s: String = \"x\";\n\
     \x20   println(s);\n\
     \x20   return 0;\n\
     }";

#[test]
fn t1_println_owned_moves_and_frees_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    child_guard("t1_println_owned_moves_and_frees_once", SRC_T1);
    let out = spawn_child("t1_println_owned_moves_and_frees_once");
    assert!(out.success, "child must exit cleanly: {out:?}");
    assert_eq!(extract_program_stdout(&out.stdout), "x\n");
    assert_eq!(out.free_count, Some(1), "child output: {out:?}");
}

// ══════════════════════════════════════════════════════════════════════
// T2 — println(&0 s) then reuse s (owned println) at the end: the &0
// overload must never free, so s is still valid for the second call.
// Poison target: __triet_println_ref's OWN shim body (add a spurious
// free) — NOT builtin_shim_meta (Reference is Copy, so a meta flip for
// this shim is vacuous, mirrors the documented length(&0 s) case).
// ══════════════════════════════════════════════════════════════════════

const SRC_T2: &str = "function main() -> Integer = {\n\
     \x20   let s: String = \"x\";\n\
     \x20   println(&0 s);\n\
     \x20   println(s);\n\
     \x20   return 0;\n\
     }";

#[test]
fn t2_println_ref_never_frees_reuse_after_succeeds() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    child_guard("t2_println_ref_never_frees_reuse_after_succeeds", SRC_T2);
    let out = spawn_child("t2_println_ref_never_frees_reuse_after_succeeds");
    assert!(out.success, "child must exit cleanly: {out:?}");
    assert_eq!(
        extract_program_stdout(&out.stdout),
        "x\nx\n",
        "s must still be valid (unfreed) after the &0 print: {out:?}"
    );
    assert_eq!(out.free_count, Some(1), "child output: {out:?}");
}

// ══════════════════════════════════════════════════════════════════════
// T3 — print(owned String): no trailing newline. Poison target: the
// lowerer's print/println shim-name selection swapped (println's calls
// route to the print shim and vice versa).
// ══════════════════════════════════════════════════════════════════════

const SRC_T3: &str = "function main() -> Integer = {\n\
     \x20   let s: String = \"x\";\n\
     \x20   print(s);\n\
     \x20   return 0;\n\
     }";

#[test]
fn t3_print_owned_no_trailing_newline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    child_guard("t3_print_owned_no_trailing_newline", SRC_T3);
    let out = spawn_child("t3_print_owned_no_trailing_newline");
    assert!(out.success, "child must exit cleanly: {out:?}");
    assert_eq!(
        extract_program_stdout(&out.stdout),
        "x",
        "print must not append a newline: {out:?}"
    );
    assert_eq!(out.free_count, Some(1), "child output: {out:?}");
}

// ══════════════════════════════════════════════════════════════════════
// T4 — owned vs &0 routing, exercised together: `&0 a` (never frees, reused
// after) and owned `b` (moves+frees), then owned `a` again at the end
// (moves+frees). Poison target: the lowerer's `is_ref` branch always
// selecting the `_ref` shim name regardless of `is_ref`.
// ══════════════════════════════════════════════════════════════════════

const SRC_T4: &str = "function main() -> Integer = {\n\
     \x20   let a: String = \"a\";\n\
     \x20   println(&0 a);\n\
     \x20   let b: String = \"b\";\n\
     \x20   println(b);\n\
     \x20   println(a);\n\
     \x20   return 0;\n\
     }";

#[test]
fn t4_owned_and_ref_route_to_distinct_shims() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    child_guard("t4_owned_and_ref_route_to_distinct_shims", SRC_T4);
    let out = spawn_child("t4_owned_and_ref_route_to_distinct_shims");
    assert!(out.success, "child must exit cleanly: {out:?}");
    assert_eq!(
        extract_program_stdout(&out.stdout),
        "a\nb\na\n",
        "child output: {out:?}"
    );
    assert_eq!(
        out.free_count,
        Some(2),
        "owned `b` and owned `a` (reused) must each free exactly once, \
         the &0 print of `a` must never free: {out:?}"
    );
}

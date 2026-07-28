//! WO-SRet-Aggregate-StringField-Corruption (O+G, 2026-07-29) — subprocess-
//! isolated CONTENT read for the fat-String `len`/`cap` sync bug (Hole A:
//! sret return path, this commit).
//!
//! `sret_string_field_counting.rs` proves the `(ptr, cap)` pairing is
//! correct but only reads LENGTH (`length(l.s)`), never the string's actual
//! BYTES. The measured pre-fix failure mode for reading bytes through a
//! corrupted fat pointer is a real SIGSEGV (139) — `println` reads `len`
//! bytes starting at `ptr`; a garbage `len` (uninitialized stack word) reads
//! out of bounds. That crash must be contained in a child process (mirrors
//! `print_println_overload_subprocess.rs` / `enum_field_moveout_subprocess.rs`
//! — a poisoned tree run here would otherwise kill this entire test binary).
//!
//! The parent asserts `status.success()` — a crashed child fails the
//! assertion instead of aborting the harness.
//!
//! Hole B (STEP-4 `Nullable(String)` construct-time sync) and the combined
//! Hole-A+B case are fixed and tested in the follow-up commit (`p2`/`p3`
//! tests land there).
//!
//! # `free_count` is a CALL count, not a "real dealloc" count
//!
//! Owned `println` frees its argument via a DIRECT Rust function call inside
//! its own body (`mir_lower.rs:5791`, `__triet_string_free(ptr, cap)`) — NOT
//! through the JIT shim registry `__srsp_str_free` wraps (mirrors
//! `print_println_overload_subprocess.rs`'s documented caveat). So the REAL
//! dealloc for the printed String is invisible to this counter; what it
//! counts is the zeroing-drop no-op calls (`ptr == 0`) at each scope-end
//! `Drop` downstream of the move-into-println. A shape with an intermediate
//! `make()` local (P1/P3, sret path) has TWO such Drop sites (the callee's
//! own temp + the caller's `l`) → `free_count == Some(2)`; a shape with no
//! intermediate function-local (P2) has ONE → `Some(1)`. Neither is a
//! double-free — both recorded calls are `ptr == 0` no-ops in the healthy
//! tree; a genuine double-free would show as the SAME NONZERO pointer twice
//! (not asserted here — the counting harness's paired oracle is the layer
//! that would catch a corrupted cap on a real dealloc).
#![allow(unsafe_code)]

use std::io::Read;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __srsp_str_free(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_2_0("__triet_string_free", __srsp_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_3_0("__triet_print", mir_lower::__triet_print),
        ShimSymbol::fn_2_0("__triet_print_ref", mir_lower::__triet_print_ref),
        ShimSymbol::fn_3_0("__triet_println", mir_lower::__triet_println),
        ShimSymbol::fn_2_0("__triet_println_ref", mir_lower::__triet_println_ref),
    ]
}

const START_MARKER: &str = "\u{1}SRSP_STDOUT_START\u{1}";
const END_MARKER: &str = "\u{1}SRSP_STDOUT_END\u{1}";

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
    {
        use std::io::Write;
        let mut err = std::io::stderr();
        let _ = writeln!(err, "FREE={}", STR_FREES.load(Ordering::SeqCst));
        let _ = err.flush();
    }
    r
}

fn extract_program_stdout(raw: &str) -> &str {
    raw.split(START_MARKER)
        .nth(1)
        .and_then(|s| s.split(END_MARKER).next())
        .unwrap_or_default()
}

const ENV_MARKER: &str = "_TRIET_SRSP";

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
    #[allow(dead_code)]
    stderr: String,
    free_count: Option<u64>,
}

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
// P1 — Hole A: sret String field, content read via println. Pre-fix this
// is the SIGSEGV 139 repro (garbage len@+8 read out of bounds).
// ══════════════════════════════════════════════════════════════════════

const SRC_P1: &str = "struct Leaf { s: String }\n\
     function make() -> Leaf {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return p;\n\
     }\n\
     function main() -> Integer {\n\
     \x20   let l = make();\n\
     \x20   println(l.s);\n\
     \x20   return 0;\n\
     }";

#[test]
fn p1_sret_string_field_println_content() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    child_guard("p1_sret_string_field_println_content", SRC_P1);
    let out = spawn_child("p1_sret_string_field_println_content");
    assert!(out.success, "child must exit cleanly: {out:?}");
    assert_eq!(extract_program_stdout(&out.stdout), "hi\n");
    // 2 zeroing no-op calls: make()'s own temp + main()'s `l` (see module
    // doc — the real dealloc is println's own direct call, invisible here).
    assert_eq!(out.free_count, Some(2), "child output: {out:?}");
}

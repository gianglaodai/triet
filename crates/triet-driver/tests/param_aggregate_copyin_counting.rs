//! WO-Param-Aggregate-CopyIn (O+G, 2026-07-28) — route-lower pointer-dedup
//! free/leak teeth for the plain-Struct param callee copy-in fixed in
//! `crates/triet-jit/src/mir_lower.rs`'s entry-block per-param loop (the
//! `MirType::Struct` branch added right after the pre-existing Enum
//! copy-in, mirroring String/Enum/Outcome).
//!
//! Bug (measured on `35f4f02`, unpatched — the commit this WO's base sits
//! on): the Lát 2 derived-locals loop explicitly excludes parameters
//! (`i < reserved_locals`) from `struct_slots`, so a plain heap-bearing
//! Struct param never got a StackSlot. Every downstream `struct_slots.get()`
//! gate (field-move-out tombstone, whole-move Deinit, call-site forwarding)
//! then either silently no-op'd (treating the CALLER'S raw pointer as if it
//! were an unpopulated Cranelift Variable) or read/wrote through that raw
//! caller pointer directly — the caller's own buffer, never a private copy.
//! Depending on the exact shape this produced a double-free (134), a
//! SIGSEGV (139), or a SIGILL (132, via the field-sync gap corrupting a
//! `len` word that the ADR-0044 range-check then trapped on) — see each
//! fixture file's own doc comment for the specific mechanism.
//!
//! ⚠ Like `aggregate_move_tombstone_counting.rs`, a raw free-COUNT is not
//! enough: freeing the SAME pointer twice and freeing TWO distinct legitimate
//! allocations both produce count==2. This harness records the actual
//! POINTER VALUE on every alloc/free call and asserts on the DEDUPED SET.
//!
//! ⚠ SCOPE NOTE (S3/V excluded here, by design, not oversight): two cells in
//! the WO's fixture table (545 "S3" return-escape, 550 "V" Vector field) use
//! `Vector<Integer>`, not `String`. `__triet_vector_push`'s Rust
//! implementation (`mir_lower.rs:6009`) ALWAYS reallocates on every push —
//! calling `__triet_vector_alloc`/`__triet_vector_free` as PLAIN INTRA-CRATE
//! RUST CALLS, not through the JIT's shim-symbol dispatch table. A
//! JIT-shim-level pointer recorder (like this file's `__pac_str_*` wrappers)
//! can only observe calls the COMPILED .tri PROGRAM dispatches through the
//! shim table — it is structurally blind to `push`'s internal
//! realloc-and-free-the-old-buffer step, so naive dedup counting on a
//! Vector-field fixture that calls `push` produces FALSE leak signals
//! unrelated to any real bug. 545/550 are instead verified by (a) the
//! integration corpus completing without a process abort (a live
//! double-free/SIGSEGV under Luật 15 would kill the whole `cargo test`
//! binary — it did not), (b) their `EXPECT`-value oracle reading through the
//! moved/returned data, and (c) the manual poison verification in the WO
//! report (cp-snapshot + standalone `triet-driver run`, not an in-file
//! poison test — see below for why).
//!
//! ⚠ NO in-file "poison" sub-tests here either (unlike
//! `aggregate_move_tombstone_counting.rs`, which models its bug via a
//! swappable, non-crashing stand-in `free` shim). THIS bug's root cause is
//! the callee copying through — or failing to copy through — REAL MEMORY
//! (a StackSlot vs. a raw caller pointer): disabling the fix reproduces an
//! ACTUAL double-free / SIGSEGV / SIGILL in the running process, which
//! would abort this test binary, not just flip an assertion. Poison
//! verification for this WO is a MANUAL cp-snapshot + rebuild +
//! subprocess-isolated (`triet-driver run`) procedure, pasted as raw output
//! in the WO report, per Luật "TEETH KHÔNG BAO GIỜ git checkout".
//!
//! ⚠ RAM: run with `--exact --test-threads=1` if isolating a single test —
//! process-global shim state + no-mangle symbols (N7 fork-bomb hazard per
//! project convention). The Mutex below also serializes within this binary
//! for a default parallel `cargo test` run.
#![allow(unsafe_code)]

use std::sync::Mutex;

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static ALLOCATED: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static FREED: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn reset() {
    ALLOCATED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    FREED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[unsafe(no_mangle)]
extern "C" fn __pac_str_from_bytes(src: i64, len: i64) -> i64 {
    let ptr = mir_lower::__triet_string_from_bytes(src, len);
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        ALLOCATED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ptr);
    }
    ptr
}

#[unsafe(no_mangle)]
extern "C" fn __pac_str_free(ptr: i64, cap: i64) {
    if ptr != 0 && ptr != triet_mir::NULL_SENTINEL {
        FREED.lock().unwrap_or_else(|e| e.into_inner()).push(ptr);
    }
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

fn run(source: &str) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1("__triet_string_from_bytes", __pac_str_from_bytes),
        ShimSymbol::fn_2_0("__triet_string_free", __pac_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
    ];
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Returns (distinct_allocated, distinct_freed, duplicate_free_count).
fn dedup_stats() -> (usize, usize, usize) {
    let allocated = ALLOCATED.lock().unwrap_or_else(|e| e.into_inner());
    let freed = FREED.lock().unwrap_or_else(|e| e.into_inner());
    let distinct_allocated: std::collections::HashSet<i64> = allocated.iter().copied().collect();
    let distinct_freed: std::collections::HashSet<i64> = freed.iter().copied().collect();
    let dup_count = freed.len() - distinct_freed.len();
    (distinct_allocated.len(), distinct_freed.len(), dup_count)
}

// ── S1 (fixture 543): read-only field access through a heap-bearing param ──

const SRC_S1: &str = "struct Leaf { s: String }\n\
     function take(p: Leaf) -> Integer = {\n\
     \x20   return length(p.s);\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return take(p);\n\
     }";

#[test]
fn s1_field_read_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_S1);
    assert_eq!(r, 2, "\"hi\".length() == 2");
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1, "exactly one String allocated");
    assert_eq!(freed, 1, "exactly one distinct pointer freed");
    assert_eq!(dup, 0, "no pointer freed twice");
}

// ── S2 (fixture 544): field move-out `let s = p.s;` ─────────────────────────

const SRC_S2: &str = "struct Leaf { s: String }\n\
     function take(p: Leaf) -> Integer = {\n\
     \x20   let s = p.s;\n\
     \x20   return length(s);\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return take(p);\n\
     }";

#[test]
fn s2_field_moveout_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_S2);
    assert_eq!(r, 2);
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1);
    assert_eq!(freed, 1);
    assert_eq!(dup, 0);
}

// ── P4 (fixture 546): whole-struct move `let q = p;` ────────────────────────

const SRC_P4: &str = "struct Leaf { s: String }\n\
     function take(p: Leaf) -> Integer = {\n\
     \x20   let q = p;\n\
     \x20   return length(q.s);\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return take(p);\n\
     }";

#[test]
fn p4_whole_move_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_P4);
    assert_eq!(r, 2);
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1);
    assert_eq!(freed, 1);
    assert_eq!(dup, 0);
}

// ── S4 (fixture 547): nested struct move `let c = b;` ───────────────────────

const SRC_S4: &str = "struct Leaf { s: String }\n\
     struct Box { l: Leaf, n: Integer }\n\
     function take(b: Box) -> Integer = {\n\
     \x20   let c = b;\n\
     \x20   return length(c.l.s) + c.n;\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let b = Box { l: Leaf { s: \"hi\" }, n: 3 };\n\
     \x20   return take(b);\n\
     }";

#[test]
fn s4_nested_struct_move_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_S4);
    assert_eq!(r, 5, "length(\"hi\")=2 + n=3 == 5");
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1);
    assert_eq!(freed, 1);
    assert_eq!(dup, 0);
}

// ── S9 (fixture 548): call-site forwarding `outer(p) { inner(p) }` ──────────

const SRC_S9: &str = "struct Leaf { s: String }\n\
     function inner(p: Leaf) -> Integer = {\n\
     \x20   return length(p.s);\n\
     }\n\
     function outer(p: Leaf) -> Integer = {\n\
     \x20   return inner(p) - 2;\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   return outer(p);\n\
     }";

#[test]
fn s9_forwarding_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_S9);
    assert_eq!(
        r, 0,
        "inner(p)==2, 2-2==0 — self-checking, not a bare exit-0"
    );
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 1, "only ONE String ever created (in main)");
    assert_eq!(
        freed, 1,
        "freed exactly once — inner's own copy, not main's or outer's"
    );
    assert_eq!(dup, 0, "no double-free across the forwarding chain");
}

// ── S8 (fixture 549): TWO heap params, combined by Add ──────────────────────

const SRC_S8: &str = "struct Leaf { s: String }\n\
     function take(p: Leaf, q: Leaf) -> Integer = {\n\
     \x20   return length(p.s) + length(q.s);\n\
     }\n\
     function main() -> Integer = {\n\
     \x20   let p = Leaf { s: \"hi\" };\n\
     \x20   let q = Leaf { s: \"world\" };\n\
     \x20   return take(p, q);\n\
     }";

#[test]
fn s8_two_heap_params_no_leak_no_double_free() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let r = run(SRC_S8);
    assert_eq!(r, 7, "length(\"hi\")=2 + length(\"world\")=5 == 7");
    let (allocated, freed, dup) = dedup_stats();
    assert_eq!(allocated, 2, "two distinct Strings allocated");
    assert_eq!(freed, 2, "both freed exactly once each");
    assert_eq!(dup, 0, "neither pointer freed twice");
}

// ── WO §2③ / §4-C canary: the `Deinit(arg)` invariant this fix DEPENDS on ──
//
// ADR-0042 Q1 (`triet-lower/src/lib.rs:~4462`) unconditionally emits
// `Deinit(arg)` in the CALLER right after `CallDispatch` for every Move-type
// call argument. Callee copy-in is sound ONLY because of this: the caller's
// own buffer is tombstoned regardless of what the callee does with its
// private copy. If this invariant silently disappeared (e.g. a future
// refactor of the lowerer's call-lowering path), copy-in would turn the
// double-free this WO fixes into a LEAK instead (both callee's copy freed
// AND caller's original never tombstoned... wait — actually the OTHER way:
// without the caller's Deinit, the caller's original heap allocation would
// be freed a SECOND time by the caller's own end-of-scope Drop, since the
// callee already freed its independent copy of the SAME underlying
// String/Vector/HashMap allocation the copy-in duplicated the POINTER of,
// not the heap bytes). This canary pins the MIR-structural fact directly,
// independent of the JIT: it does not care whether copy-in exists, only
// that the LOWERER keeps emitting the tombstone the JIT's soundness proof
// relies on.
#[test]
fn caller_emits_deinit_after_struct_arg_call() {
    let bodies = lower_source(SRC_S1);
    let main_body = bodies
        .iter()
        .find(|b| b.signature.name == "main")
        .expect("main body");

    let mut found_call_with_struct_arg = false;
    for block in &main_body.blocks {
        if let triet_mir::Terminator::CallDispatch {
            args, return_bb, ..
        } = &block.terminator
        {
            for arg in args {
                let is_struct = matches!(
                    main_body.local_decls[arg.0].ty,
                    triet_mir::MirType::Struct(_)
                );
                if !is_struct {
                    continue;
                }
                found_call_with_struct_arg = true;
                let target_block = &main_body.blocks[return_bb.0];
                let has_deinit = target_block
                    .statements
                    .iter()
                    .any(|s| matches!(s, triet_mir::Statement::Deinit(l, _) if l == arg));
                assert!(
                    has_deinit,
                    "ADR-0042 Q1 invariant missing: caller must Deinit(arg) after \
                     CallDispatch for a Struct-typed Move argument — copy-in's \
                     soundness proof (WO-Param-Aggregate-CopyIn §2③) depends on \
                     this. Found CallDispatch with struct arg {arg:?} whose \
                     return_bb {return_bb:?} has no matching Deinit."
                );
            }
        }
    }
    assert!(
        found_call_with_struct_arg,
        "test setup: SRC_S1's main must call take(p) with a Struct-typed arg — \
         canary would be vacuous otherwise"
    );
}

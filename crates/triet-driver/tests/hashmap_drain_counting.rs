//! WO-HashMap-Drain-PA2 (ADR-0089 §AMEND-2, O✅/G✅/Giang✅ 2026-07-27) —
//! permanent FREE-count teeth for `for (k, v) in <HashMap>.drain()`.
//!
//! Mirrors the established real-pipeline + counting-shim-swap pattern from
//! `drain_iter_counting.rs` (Vector's own drain teeth): real `HashMap`
//! shims (`alloc`/`insert`/`free`/`drain_next`) so bytes actually move and
//! `len--`/tombstone actually run, with `__triet_string_free` swapped for a
//! counting stand-in in the String-key/value tests, or `__triet_hashmap_free`
//! swapped in the container-buffer test.
//!
//! Stricter than the Vector precedent (G mandate, WO §5): every counting
//! stand-in here tracks the ACTUAL freed POINTERS (not just a call count)
//! and asserts BOTH `count == N` and `dup == 0` — a bare call-count is
//! blind to a double-free that happens to land on 3 calls where only 2
//! distinct objects exist (2 real objects + 1 duplicate free would pass a
//! naive `count == 3`-only check; dedup catches it).
//!
//! What these tests prove that the fixture-only regression tests (520-525,
//! EXPECT-value) cannot: the tombstone contract (`state -> 2` +
//! `len--`, `triet-jit/src/mir_lower.rs`'s `__triet_hashmap_drain_next`) is
//! what keeps the container's own end-of-scope `Drop` (which only walks
//! `state == 1` slots) from re-visiting an already-drained slot. A leaked
//! or double-freed key/value does not change `main`'s return value on its
//! own (the fixture is vacuous for THIS soundness property) — these tests
//! make the exact freed-pointer set the assertion.
#![allow(unsafe_code)]

use std::sync::Mutex;

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static TEST_LOCK: Mutex<()> = Mutex::new(());
static STR_FREED: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static MAP_FREED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

/// Counting stand-in for `__triet_string_free`. Mirrors the real free's
/// `ptr == 0 || ptr == NULL_SENTINEL` guard so it only counts frees of LIVE
/// allocations, and records the POINTER itself (not just a tally) so the
/// caller can dedup-check afterward.
#[unsafe(no_mangle)]
extern "C" fn __hm_drain_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(ptr);
}

/// Counting stand-in for `__triet_hashmap_free` (container BUFFER free,
/// distinct from the per-entry key/value frees above). Same null/sentinel
/// guard + pointer-recording discipline.
#[unsafe(no_mangle)]
extern "C" fn __hm_drain_map_free(ptr: i64) {
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    MAP_FREED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(ptr);
}

/// `(count, duplicate_count)` — `duplicate_count` is how many entries in
/// `ptrs` are NOT the first occurrence of their value. A sound free-walk
/// has `duplicate_count == 0`; any double-free shows up here as > 0
/// regardless of what the total `count` happens to add up to.
fn count_and_dup(ptrs: &[i64]) -> (usize, usize) {
    let mut seen: Vec<i64> = Vec::new();
    let mut dup = 0usize;
    for &p in ptrs {
        if seen.contains(&p) {
            dup += 1;
        } else {
            seen.push(p);
        }
    }
    (ptrs.len(), dup)
}

/// Replicates the driver's source→bodies pipeline (main.rs phases 1-3).
fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Shim set with `__triet_string_free` swapped for the counter — every
/// other shim (String alloc/from_bytes/len, HashMap alloc/free/insert/
/// drain_next) is the REAL implementation, so a real leak/double-free on
/// the CONTAINER side would still abort the test process.
fn string_counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", __hm_drain_str_free),
        ShimSymbol::fn_1_1("__triet_string_len", mir_lower::__triet_string_len),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1(
            "__triet_hashmap_drain_next",
            mir_lower::__triet_hashmap_drain_next,
        ),
    ]
}

/// Shim set with `__triet_hashmap_free` (container BUFFER free) swapped
/// for the counter — used for the scalar-key/value container-FREE proof
/// (no String involved at all, isolating the buffer-free count).
fn map_counting_shims() -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", __hm_drain_map_free),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1(
            "__triet_hashmap_drain_next",
            mir_lower::__triet_hashmap_drain_next,
        ),
    ]
}

/// Compile `source` with `shims`, call `main`, return its result.
fn run(source: &str, shims: &[ShimSymbol]) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(shims);
    let compiled = ctx
        .compile_multi(&body_refs)
        .expect("hashmap-drain program must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

/// Drain trọn: `HashMap<String, String>` with 3 entries — every KEY and
/// every VALUE string must free EXACTLY ONCE (6 total: 3 keys + 3 values),
/// zero duplicates. The container's own end-of-scope Drop then sees
/// `len == 0` (tombstoned by `drain_next`'s `len--` every call) and frees
/// NOTHING further — a broken `state -> 2` (P1) would double-free here
/// (drop-glue re-walking an already-surfaced slot); a broken `len--` (P2)
/// would either hang (if drain's OWN loop also depended on it) or leave
/// stale survivors for the container Drop to re-free.
#[test]
fn drain_full_frees_every_key_and_value_exactly_once_no_dup() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable m: HashMap<String, String> = hashmap_new();\n\
         \x20   m = insert(m, \"a\", \"aa\");\n\
         \x20   m = insert(m, \"b\", \"bb\");\n\
         \x20   m = insert(m, \"c\", \"cc\");\n\
         \x20   let mutable total = 0;\n\
         \x20   for (k, v) in m.drain() {\n\
         \x20       total = total + length(v);\n\
         \x20   }\n\
         \x20   return total;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 6, "main must return 2+2+2 (aa/bb/cc lengths)");
    let freed = STR_FREED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (count, dup) = count_and_dup(&freed);
    assert_eq!(
        count, 6,
        "3 keys + 3 values must free exactly once each — fewer is a leak, \
         more is a double-free (freed = {freed:?})"
    );
    assert_eq!(
        dup, 0,
        "zero duplicate pointers — a nonzero dup means the SAME allocation \
         was freed more than once even though the total count matched \
         (freed = {freed:?})"
    );
}

/// Break-mid hygiene (G mandate tử huyệt #2): drain 4 `HashMap<String,
/// String>` entries, `break` after the 2nd. The 2 already-processed
/// entries free via the loop body's own end-of-iteration/break-path Drop;
/// the 2 un-drained survivors (still `state == 1`) free via the map's own
/// end-of-scope Drop. Total must be 8 entries freed (2 keys and 2 values
/// processed, plus 2 keys and 2 values from the survivors), with zero
/// duplicates: a total of 8 with a nonzero dup would mean some entry was
/// freed twice while another leaked, masking a bug behind a
/// coincidentally-correct total.
#[test]
fn drain_break_mid_frees_processed_and_survivors_no_dup() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable m: HashMap<String, String> = hashmap_new();\n\
         \x20   m = insert(m, \"a\", \"1\");\n\
         \x20   m = insert(m, \"b\", \"2\");\n\
         \x20   m = insert(m, \"c\", \"3\");\n\
         \x20   m = insert(m, \"d\", \"4\");\n\
         \x20   let mutable count = 0;\n\
         \x20   for (k, v) in m.drain() {\n\
         \x20       count = count + 1;\n\
         \x20       if count == 2 {\n\
         \x20           break;\n\
         \x20       }\n\
         \x20   }\n\
         \x20   return count;\n\
         }",
        &string_counting_shims(),
    );
    assert_eq!(r, 2, "main must return 2 (loop breaks after the 2nd entry)");
    let freed = STR_FREED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (count, dup) = count_and_dup(&freed);
    assert_eq!(
        count, 8,
        "2 processed entries (2 keys + 2 values) + 2 survivor entries \
         (2 keys + 2 values) via the container's own Drop — every string \
         freed EXACTLY once across the whole HashMap (freed = {freed:?})"
    );
    assert_eq!(
        dup, 0,
        "zero duplicate pointers across processed + survivor frees \
         (freed = {freed:?})"
    );
}

/// Container-survives (G mandate tử huyệt #3), DIRECT `len()` read — the
/// P2 poison probe (WO §6 "test chưa đủ mạnh" mandate): the fixture-level
/// re-insert test (523/`drain_then_reinsert_frees_container_buffer_
/// exactly_once_no_dup` below) does NOT actually observe `len--` itself —
/// re-inserting into a `cap == 4` buffer never crosses the resize
/// threshold whether `len` is 0 or stale-2, so a REMOVED `len--` silently
/// passes both of those. This test reads `__triet_hashmap_len` directly
/// (the ONE concrete, reachable consumer of the tombstone chain's `len--`
/// step besides the resize-threshold math) immediately after a full
/// drain — a broken `len--` makes this return the STALE pre-drain count
/// instead of 0.
#[test]
fn drain_full_leaves_len_exactly_zero() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable m: HashMap<Integer, Integer> = hashmap_new();\n\
         \x20   m = insert(m, 1, 10);\n\
         \x20   m = insert(m, 2, 20);\n\
         \x20   m = insert(m, 3, 30);\n\
         \x20   for (k, v) in m.drain() {\n\
         \x20       let mutable ignore = v;\n\
         \x20   }\n\
         \x20   return len(m);\n\
         }",
        &map_counting_shims(),
    );
    assert_eq!(
        r, 0,
        "len() must read 0 immediately after a full drain — a nonzero \
         result means `len--` did not actually run on every drained entry \
         (the drain LOOP's own termination is state-gated, not len-gated, \
         so a broken len-- does not hang or mis-terminate the loop; it \
         only surfaces here and in the resize-threshold math)"
    );
}

/// Container-survives (G mandate tử huyệt #3): a full drain of a
/// `HashMap<Integer, Integer>` must leave `len == 0` on the SAME buffer
/// handle (no resize triggered — 2 inserts then 1 re-insert never crosses
/// the 75%-load-factor threshold on a `cap == 4` default allocation), so
/// re-inserting afterward and then letting the map drop must free the
/// buffer EXACTLY ONCE. 0 would be a leak (drain's tombstone chain
/// orphaning the handle), 2 a double-free (drain accidentally freeing the
/// buffer itself, then the container's own Drop freeing it again).
#[test]
fn drain_then_reinsert_frees_container_buffer_exactly_once_no_dup() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    MAP_FREED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let r = run(
        "function main() -> Integer {\n\
         \x20   let mutable m: HashMap<Integer, Integer> = hashmap_new();\n\
         \x20   m = insert(m, 1, 10);\n\
         \x20   m = insert(m, 2, 20);\n\
         \x20   for (k, v) in m.drain() {\n\
         \x20       let mutable ignore = v;\n\
         \x20   }\n\
         \x20   m = insert(m, 9, 42);\n\
         \x20   return match get(m, 9) {\n\
         \x20       ~+ found => found,\n\
         \x20       ~0 => 0,\n\
         \x20   };\n\
         }",
        &map_counting_shims(),
    );
    assert_eq!(r, 42, "main must return 42 (re-inserted after full drain)");
    let freed = MAP_FREED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (count, dup) = count_and_dup(&freed);
    assert_eq!(
        count, 1,
        "the drained-then-reinserted map must free its buffer EXACTLY \
         once at main's own scope exit (freed = {freed:?})"
    );
    assert_eq!(dup, 0, "zero duplicate buffer frees (freed = {freed:?})");
}

/// Harness self-test (WO §5 mandate: the counting mechanism itself must
/// be proven alive, not dead infrastructure that always reports "correct"
/// regardless of input). Directly drives `count_and_dup` — the same
/// dedup logic every test above relies on — with a deliberately POISONED
/// input (one pointer freed twice) and confirms the assertion machinery
/// actually reacts: `dup` must be nonzero, `count` must include the
/// duplicate. If a future refactor accidentally made `count_and_dup`
/// dup-blind (e.g. reverted to a bare `.len()`), THIS test would go red
/// even though every fixture-driven test above still passed.
#[test]
fn counting_harness_dedup_detects_a_poisoned_double_free() {
    let (count, dup) = count_and_dup(&[100, 200, 100, 300]);
    assert_eq!(count, 4, "raw count includes the duplicate entry");
    assert_eq!(
        dup, 1,
        "pointer 100 appears twice — dedup must report exactly 1 duplicate"
    );

    let (count_clean, dup_clean) = count_and_dup(&[100, 200, 300]);
    assert_eq!(count_clean, 3);
    assert_eq!(
        dup_clean, 0,
        "no duplicates in a clean set — the harness must not false-positive"
    );
}

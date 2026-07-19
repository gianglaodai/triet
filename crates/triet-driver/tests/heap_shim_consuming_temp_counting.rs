//! WO-ShimTempOwnership T0 SUPPLEMENT (O ✅ · G ✅, 2026-07-19) — the "leaks
//! across the whole shim-BORROW array" verdict (`heap_shim_temp_leak_counting.rs`:
//! concat/contains/eq) only measured `arg_consumes: false` shims. G blocked any
//! fix until the CONSUMING group (`arg_consumes: true` — `push`/`insert`) is
//! measured too: does an anonymous owned-`String` temp (field-access move-out
//! or literal) fed directly as a CONSUMED argument come out LÀNH (freed exactly
//! once, by the container it was moved into), also RỈ (leaked — never makes it
//! into the container), or DOUBLE-FREE (both the temp's own — nonexistent —
//! Drop and the container's element-free both fire)?
//!
//! Shapes (each with a fully-`let`-bound control):
//!   PA: `push(v, h.name)`   — `Vector<String>`, FIELD-ACCESS element arg
//!   PB: `push(v, "hello")`  — `Vector<String>`, LITERAL element arg
//!   PC: `insert(m, 1, h.name)` — `HashMap<Integer,String>`, FIELD-ACCESS value arg
//!
//! Each program ends by dropping the resulting container (`v2`/`m2`, `let`-bound
//! so IT is definitely freed) without ever removing the pushed/inserted element —
//! the container's own Drop must free exactly the elements/values it holds
//! (1 each here). FREE-count isolates whether the ORIGINAL temp additionally
//! leaked (impossible to observe as "still 1" vs "should be 1" without a
//! reference point) — the discriminating signal is: does inline vs let-bound
//! change the count at all? Per the WO's three possible verdicts:
//!   - LÀNH: inline count == let-bound count == 1 (container's own element
//!     free is the only free either way — consuming shim already transfers
//!     ownership into the container regardless of caller `push_owned` state).
//!   - RỈ: inline count == 0 (the container ends up NOT holding the string —
//!     e.g. some other path swallowed it) while let-bound == 1.
//!   - DOUBLE-FREE: inline (or let-bound) count == 2 — both the "phantom"
//!     original-temp Drop and the container's Drop fire.
//!
//! ⚠ RAM: run `--exact --test-threads=1` (process-global AtomicUsize and
//! no-mangle shim — N7 fork-bomb hazard). `TEST_LOCK` Mutex serializes a
//! default parallel `cargo test` run within this binary.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use triet_jit::mir_lower::{self, JitContext, ShimSymbol};

static STR_FREES: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[unsafe(no_mangle)]
extern "C" fn __hsct_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

/// POISON shim: simulates a dropped free-arm (leak) — never counts.
#[unsafe(no_mangle)]
extern "C" fn __hsct_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: simulates a double-free — counts twice per real call.
#[unsafe(no_mangle)]
extern "C" fn __hsct_str_free_poison_double(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(2, Ordering::SeqCst);
}

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

fn shims_with(free_fn: extern "C" fn(i64, i64)) -> Vec<ShimSymbol> {
    vec![
        ShimSymbol::fn_3_1("__triet_vector_alloc", mir_lower::__triet_vector_alloc),
        ShimSymbol::fn_1_0("__triet_vector_free", mir_lower::__triet_vector_free),
        ShimSymbol::fn_2_1("__triet_vector_push", mir_lower::__triet_vector_push),
        ShimSymbol::fn_1_1("__triet_vector_len", mir_lower::__triet_vector_len),
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_1_1("__triet_hashmap_len", mir_lower::__triet_hashmap_len),
        ShimSymbol::fn_2_1("__triet_string_alloc", mir_lower::__triet_string_alloc),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_0("__triet_string_free", free_fn),
    ]
}

fn run_with(source: &str, free_fn: extern "C" fn(i64, i64)) -> i64 {
    let bodies = lower_source(source);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let shims = shims_with(free_fn);
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let mut ctx = JitContext::with_shims(&shims);
    let compiled = ctx.compile_multi(&body_refs).expect("must JIT-compile");
    let main = compiled.get("main").expect("main compiled");
    unsafe { main.call_i64_0() }
}

fn run(source: &str) -> i64 {
    run_with(source, __hsct_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// PA: push(Vector<String>, FIELD-ACCESS) — consuming shim, element arg
// is an inline field-access temp (never `push_owned` by the lowerer).
// ══════════════════════════════════════════════════════════════════════

const SRC_PA_INLINE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let v2 = push(v, h.name);\n\
     \x20   return 0;\n\
     }";

#[test]
fn pa_push_field_access_inline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PA_INLINE);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PA (push field-access inline): FREE={count}");
    assert_eq!(
        count, 1,
        "T0 measured: LÀNH — push's consuming element arg frees exactly \
         once (the container's own element-free) whether the source was an \
         inline field-access temp or a let-bound local; SAME as PA-ctrl"
    );
}

const SRC_PA_CONTROL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let n = h.name;\n\
     \x20   let v2 = push(v, n);\n\
     \x20   return 0;\n\
     }";

#[test]
fn pa_control_push_field_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PA_CONTROL);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PA-ctrl (push field let-bound): FREE={count}");
    assert_eq!(count, 1, "T0 measured: sound baseline — frees exactly once");
}

// ══════════════════════════════════════════════════════════════════════
// PB: push(Vector<String>, LITERAL) — consuming shim, element arg is an
// inline string-literal temp.
// ══════════════════════════════════════════════════════════════════════

const SRC_PB_INLINE: &str = "function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let v2 = push(v, \"hello\");\n\
     \x20   return 0;\n\
     }";

#[test]
fn pb_push_literal_inline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PB_INLINE);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PB (push literal inline): FREE={count}");
    assert_eq!(
        count, 1,
        "T0 measured: LÀNH — inline literal element frees exactly once, \
         SAME as PB-ctrl"
    );
}

const SRC_PB_CONTROL: &str = "function main() -> Integer = {\n\
     \x20   let v: Vector<String> = vector_new();\n\
     \x20   let lit = \"hello\";\n\
     \x20   let v2 = push(v, lit);\n\
     \x20   return 0;\n\
     }";

#[test]
fn pb_control_push_literal_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PB_CONTROL);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PB-ctrl (push literal let-bound): FREE={count}");
    assert_eq!(count, 1, "T0 measured: sound baseline — frees exactly once");
}

// ══════════════════════════════════════════════════════════════════════
// PC: insert(HashMap<Integer,String>, 1, FIELD-ACCESS) — consuming shim,
// VALUE arg is an inline field-access temp.
// ══════════════════════════════════════════════════════════════════════

const SRC_PC_INLINE: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let m: HashMap<Integer, String> = hashmap_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let m2 = insert(m, 1, h.name);\n\
     \x20   return 0;\n\
     }";

#[test]
fn pc_insert_field_access_inline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PC_INLINE);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PC (insert field-access inline): FREE={count}");
    assert_eq!(
        count, 1,
        "T0 measured: LÀNH — insert's consuming value arg frees exactly \
         once, SAME as PC-ctrl"
    );
}

const SRC_PC_CONTROL: &str = "struct H { name: String }\n\
     function main() -> Integer = {\n\
     \x20   let m: HashMap<Integer, String> = hashmap_new();\n\
     \x20   let h: H = H { name: \"hello\" };\n\
     \x20   let n = h.name;\n\
     \x20   let m2 = insert(m, 1, n);\n\
     \x20   return 0;\n\
     }";

#[test]
fn pc_control_insert_field_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_PC_CONTROL);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PC-ctrl (insert field let-bound): FREE={count}");
    assert_eq!(count, 1, "T0 measured: sound baseline — frees exactly once");
}

// ══════════════════════════════════════════════════════════════════════
// Non-vacuous proof: poison the free shim on PA-ctrl (a shape that must
// have at least 1 real free — the container's own element) and confirm
// the count moves in both directions.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn poison_leak_on_pa_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_PA_CONTROL, __hsct_str_free_poison_leak);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PA-ctrl POISON(leak): FREE={count}");
    assert_eq!(
        count, 0,
        "poison-leak (free shim never counts) must read 0 — proves the \
         counter observes real free calls, not a hardcoded pass"
    );
}

#[test]
fn poison_double_on_pa_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_PA_CONTROL, __hsct_str_free_poison_double);
    assert_eq!(r, 0);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("PA-ctrl POISON(double): FREE={count}");
    assert_eq!(
        count, 2,
        "poison-double must read exactly 2x the healthy count (1 real free)"
    );
}

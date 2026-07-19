//! WO-ShimTempOwnership — teeth for the WIDENED blast radius discovered
//! post-fix (O ✅ · G ✅, 2026-07-19): the `emit_shim_call` chokepoint fix
//! (registers `push_owned` for any BORROW-classified shim-call argument —
//! `arg_consumes[i] == false`, or no `builtin_shim_meta` entry at all) does
//! not stop at `concat`/`contains`/`eq` — it also reaches the SEARCH-KEY
//! argument of `HashMap<String,V>`'s `get`/`remove` (`__triet_hashmap_get`/
//! `__triet_hashmap_remove`, both `arg_consumes: [false, false]` — the key
//! position is BORROW, same as `concat`'s args). This was caught as an
//! unexpected regression in `typed_hashmap_counting.rs`
//! (`hashmap_string_key_struct_value_remove_frees_key_and_value`, oracle
//! 2 -> 3, verified NOT a double-free via a pointer-identity probe: 3 frees,
//! 3 distinct pointers, dup=0). "G mandate: đã vá là phải có răng cắn giữ" —
//! this file is the dedicated, MINIMAL, isolated tooth for the two call
//! sites, decoupled from that aggregate-value regression test.
//!
//! Both groups use an EMPTY `HashMap<String,Integer>` (no `insert`) so the
//! ONLY String allocation in play is the caller's own search-key argument —
//! isolates the search-key leak from the (unrelated, already-sound) resident-
//! key-on-drop mechanism.
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
extern "C" fn __hkb_str_free(ptr: i64, cap: i64) {
    let _ = cap;
    if ptr == 0 || ptr == triet_mir::NULL_SENTINEL {
        return;
    }
    STR_FREES.fetch_add(1, Ordering::SeqCst);
}

/// POISON shim: simulates a dropped free-arm (leak) — never counts.
#[unsafe(no_mangle)]
extern "C" fn __hkb_str_free_poison_leak(ptr: i64, cap: i64) {
    let _ = (ptr, cap);
}

/// POISON shim: simulates a double-free — counts twice per real call.
#[unsafe(no_mangle)]
extern "C" fn __hkb_str_free_poison_double(ptr: i64, cap: i64) {
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
        ShimSymbol::fn_6_1("__triet_hashmap_alloc", mir_lower::__triet_hashmap_alloc),
        ShimSymbol::fn_1_0("__triet_hashmap_free", mir_lower::__triet_hashmap_free),
        ShimSymbol::fn_4_1("__triet_hashmap_insert", mir_lower::__triet_hashmap_insert),
        ShimSymbol::fn_2_1("__triet_hashmap_get", mir_lower::__triet_hashmap_get),
        ShimSymbol::fn_4_1("__triet_hashmap_remove", mir_lower::__triet_hashmap_remove),
        ShimSymbol::fn_2_1(
            "__triet_string_from_bytes",
            mir_lower::__triet_string_from_bytes,
        ),
        ShimSymbol::fn_2_1("__triet_string_hash", mir_lower::__triet_string_hash),
        ShimSymbol::fn_4_1("__triet_string_eq", mir_lower::__triet_string_eq),
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
    run_with(source, __hkb_str_free)
}

// ══════════════════════════════════════════════════════════════════════
// Group 1: remove(EMPTY HashMap<String,Integer>, SEARCH-KEY)
// ══════════════════════════════════════════════════════════════════════

const SRC_REMOVE_KEY_INLINE: &str = "function main() -> Integer = {\n\
     \x20   let m: HashMap<String, Integer> = hashmap_new();\n\
     \x20   let out = remove(m, \"k\");\n\
     \x20   return match out {\n\
     \x20       ~+ v => v,\n\
     \x20       ~0 => -1,\n\
     \x20   };\n\
     }";

#[test]
fn remove_key_literal_inline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_REMOVE_KEY_INLINE);
    assert_eq!(r, -1, "empty map, key not found -> ~0 arm -> -1");
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("remove_key_literal_inline: FREE={count}");
    assert_eq!(
        count, 1,
        "POST-FIX: the caller's own \"k\" search-key literal frees exactly \
         once (remove's key position is BORROW — arg_consumes:[false,false] \
         — emit_shim_call's push_owned fix reaches it too); matches the \
         let-bound control exactly"
    );
}

const SRC_REMOVE_KEY_CONTROL: &str = "function main() -> Integer = {\n\
     \x20   let m: HashMap<String, Integer> = hashmap_new();\n\
     \x20   let k = \"k\";\n\
     \x20   let out = remove(m, k);\n\
     \x20   return match out {\n\
     \x20       ~+ v => v,\n\
     \x20       ~0 => -1,\n\
     \x20   };\n\
     }";

#[test]
fn remove_key_control_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_REMOVE_KEY_CONTROL);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("remove_key_control_let_bound: FREE={count}");
    assert_eq!(count, 1, "sound baseline — frees exactly once");
}

#[test]
fn poison_leak_on_remove_key_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_REMOVE_KEY_CONTROL, __hkb_str_free_poison_leak);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("remove_key_control POISON(leak): FREE={count}");
    assert_eq!(count, 0, "poison-leak must read 0, not the healthy 1");
}

#[test]
fn poison_double_on_remove_key_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_REMOVE_KEY_CONTROL, __hkb_str_free_poison_double);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("remove_key_control POISON(double): FREE={count}");
    assert_eq!(count, 2, "poison-double must read exactly 2x the healthy 1");
}

// ══════════════════════════════════════════════════════════════════════
// Group 2: get(EMPTY HashMap<String,Integer>, SEARCH-KEY)
// ══════════════════════════════════════════════════════════════════════

const SRC_GET_KEY_INLINE: &str = "function main() -> Integer = {\n\
     \x20   let m: HashMap<String, Integer> = hashmap_new();\n\
     \x20   let out = get(m, \"k\");\n\
     \x20   return match out {\n\
     \x20       ~+ v => v,\n\
     \x20       ~0 => -1,\n\
     \x20   };\n\
     }";

#[test]
fn get_key_literal_inline() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_GET_KEY_INLINE);
    assert_eq!(r, -1, "empty map, key not found -> ~0 arm -> -1");
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("get_key_literal_inline: FREE={count}");
    assert_eq!(
        count, 1,
        "POST-FIX: the caller's own \"k\" search-key literal frees exactly \
         once (get's key position is BORROW — arg_consumes:[false,false], \
         comment at crates/triet-mir/src/lib.rs:1210 says \"copy key\" — \
         that describes the SHIM's own read, not caller-side ownership); \
         matches the let-bound control exactly"
    );
}

const SRC_GET_KEY_CONTROL: &str = "function main() -> Integer = {\n\
     \x20   let m: HashMap<String, Integer> = hashmap_new();\n\
     \x20   let k = \"k\";\n\
     \x20   let out = get(m, k);\n\
     \x20   return match out {\n\
     \x20       ~+ v => v,\n\
     \x20       ~0 => -1,\n\
     \x20   };\n\
     }";

#[test]
fn get_key_control_let_bound() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run(SRC_GET_KEY_CONTROL);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("get_key_control_let_bound: FREE={count}");
    assert_eq!(count, 1, "sound baseline — frees exactly once");
}

#[test]
fn poison_leak_on_get_key_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_GET_KEY_CONTROL, __hkb_str_free_poison_leak);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("get_key_control POISON(leak): FREE={count}");
    assert_eq!(count, 0, "poison-leak must read 0, not the healthy 1");
}

#[test]
fn poison_double_on_get_key_control_proves_tooth_is_live() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    STR_FREES.store(0, Ordering::SeqCst);
    let r = run_with(SRC_GET_KEY_CONTROL, __hkb_str_free_poison_double);
    assert_eq!(r, -1);
    let count = STR_FREES.load(Ordering::SeqCst);
    eprintln!("get_key_control POISON(double): FREE={count}");
    assert_eq!(count, 2, "poison-double must read exactly 2x the healthy 1");
}

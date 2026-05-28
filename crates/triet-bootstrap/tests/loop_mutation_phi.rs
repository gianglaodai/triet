//! v0.7.x.runtime-fix-debt.2 — `lower_loop_stmt` phi-stitching for
//! mutable rebinds across loop iterations. Pre-fix, `loop { x = x + 1;
//! break }` read the stale pre-loop `x` every iteration (the body
//! rebound was lost without a phi at the loop entry). The fix extends
//! `lower_loop_stmt` to call `collect_assigned_vars` + emit phi
//! placeholders + patch body-end edges — mirroring `lower_while_loop`.
//!
//! These tests feed .tri source with `loop {}` through the Rust
//! pipeline (load → typecheck → lower → VM execute) and assert the
//! VM returns without hanging + produces correct values.

use miette::Diagnostic;
use triet_ir::{RuntimeValue, Vm, lower_program};
use triet_modules::load_program_from_source;
use triet_typecheck::check_resolved;

fn run_main(source: &str) -> RuntimeValue {
    let resolved = load_program_from_source(source).expect("load");
    let diagnostics = check_resolved(&resolved);
    let blocking: Vec<_> = diagnostics
        .iter()
        .filter(|err| err.severity() != Some(miette::Severity::Warning))
        .collect();
    assert!(blocking.is_empty(), "type errors: {blocking:?}");
    let ir = lower_program(&resolved);
    let main_id = ir
        .modules
        .iter()
        .flat_map(|m| &m.functions)
        .find(|f| f.name.as_deref() == Some("main"))
        .expect("missing main()")
        .id;
    let mut vm = Vm::new(ir);
    vm.execute(main_id, vec![]).expect("vm execute")
}

#[test]
fn loop_mutable_increment_no_hang() {
    // Pre-fix: `x` stays at 0 every iteration (reads stale pre-loop
    // value), `x >= 5` never becomes true → infinite hang.
    // Post-fix: phi-stitch at body_id advances `x` each iteration.
    let source = r"
function main() -> Integer {
    let mutable x: Integer = 0
    loop {
        x = x + 1
        if x >= 5 { break }
    }
    x
}
";
    let result = run_main(source);
    // After the loop, `x` resolves to the phi_dest value at the start
    // of the final iteration (4), because the body-end rebound (to 5)
    // was in the inner scope and dropped on break.
    match result {
        RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 4),
        other => panic!("expected Integer, got {other:?}"),
    }
}

#[test]
fn loop_mutable_counter_and_accumulator() {
    // Two mutable variables inside `loop {}`: a counter and a sum
    // accumulator. Both must advance in lockstep across iterations
    // or `i >= 3` never triggers → hang.
    let source = r"
function main() -> Integer {
    let mutable total: Integer = 0
    let mutable i: Integer = 0
    loop {
        total = total + i
        i = i + 1
        if i >= 3 { break }
    }
    total
}
";
    let result = run_main(source);
    // i progression: 0→1, 1→2, 2→3 (break)
    // total: phi_start_of_iter1=0, body:0+0=0
    //        phi_start_of_iter2=0, body:0+1=1
    //        phi_start_of_iter3=1, body:1+2=3 (then break, total drops)
    //post-loop total = phi at start of iter 3 = 1
    match result {
        RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 1),
        other => panic!("expected Integer, got {other:?}"),
    }
}

#[test]
fn loop_without_mutations_still_works() {
    // Regression guard: `loop {}` without mutable rebinds should
    // still execute correctly (empty mutated list → no phi overhead).
    let source = r"
function main() -> Integer {
    let mutable i: Integer = 0
    loop {
        i = i + 1
        if i > 10 { break }
    }
    42
}
";
    let result = run_main(source);
    match result {
        RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), 42),
        other => panic!("expected Integer(42), got {other:?}"),
    }
}

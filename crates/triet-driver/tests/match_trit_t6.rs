//! ADR-0061 T6 — match-on-Trit lowering.
//!
//! Routes through the REAL pipeline (parse → typecheck → lower) — no
//! hand-built MIR (persona mẫu #12). Asserts the value-keyed SwitchInt
//! (NOT enum GetDiscriminant) and the GAP-2 default→Trap soundness guard.
//!
//! Teeth (Mentor O re-verifies on final tree):
//!   - Cases map each Trit value to a block (poison case value → red).
//!   - GAP-2: a non-exhaustive Trit match (no `_`) has default→Trap
//!     (poison: drop the trap → default points at a real block → red).
//!   - Value read correctly: -1_trit → case key -1 (not +1/0).

/// `match t { -1_trit => .. 0_trit => .. 1_trit => .. }` — exhaustive,
/// no wildcard, so default MUST be a Trap.
const EXHAUSTIVE: &str = "function classify(t: Trit) -> Integer = match t {\n\
     \x20   -1_trit => 10\n\
     \x20   0_trit => 20\n\
     \x20   1_trit => 30\n\
     }\n\
     function main() -> Integer = 0";

/// `match t { -1_trit => .. 0_trit => .. }` — missing `1_trit`, no
/// wildcard → the uncovered value must hit a Trap (GAP-2).
const NON_EXHAUSTIVE: &str = "function classify(t: Trit) -> Integer = match t {\n\
     \x20   -1_trit => 10\n\
     \x20   0_trit => 20\n\
     }\n\
     function main() -> Integer = 0";

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, expr_resolutions, pattern_resolutions, method_resolutions) =
        triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(
        &program,
        &expr_resolutions,
        &pattern_resolutions,
        &method_resolutions,
    )
    .expect("lowering failed")
}

/// Find the single SwitchInt in `classify` and return (cases, default_bb).
fn switch_of(
    bodies: &[triet_mir::Body],
) -> (Vec<(i64, triet_mir::BasicBlock)>, triet_mir::BasicBlock) {
    let body = bodies
        .iter()
        .find(|b| b.signature.name == "classify")
        .expect("classify body");
    for block in &body.blocks {
        if let triet_mir::Terminator::SwitchInt {
            cases, default_bb, ..
        } = &block.terminator
        {
            return (cases.clone(), *default_bb);
        }
    }
    panic!("no SwitchInt in classify — match-on-Trit must lower to SwitchInt");
}

/// True if `bb` is a Trap block in `classify`.
fn is_trap(bodies: &[triet_mir::Body], bb: triet_mir::BasicBlock) -> bool {
    let body = bodies
        .iter()
        .find(|b| b.signature.name == "classify")
        .unwrap();
    matches!(
        body.blocks[bb.0].terminator,
        triet_mir::Terminator::Trap { .. }
    )
}

/// First integer constant emitted in block `bb` of `classify` (the arm
/// body's literal). Distinguishes which arm a case lands in.
fn first_const_in(bodies: &[triet_mir::Body], bb: triet_mir::BasicBlock) -> Option<i128> {
    let body = bodies
        .iter()
        .find(|b| b.signature.name == "classify")
        .unwrap();
    body.blocks[bb.0].statements.iter().find_map(|s| match s {
        triet_mir::Statement::Const {
            value: triet_mir::ConstValue::Integer(n),
            ..
        } => Some(*n),
        _ => None,
    })
}

#[test]
fn match_trit_case_value_maps_to_correct_arm() {
    // T6.1 + GAP-1: each Trit value routes to the RIGHT arm body
    // (-1_trit => 10, 0_trit => 20, 1_trit => 30). This checks the
    // value→body MAPPING, not just the set of keys — so swapping
    // -1↔1 (e.g. negating the key) is caught (a set-only check would
    // be vacuous since {-1,0,1} is symmetric under negation).
    let bodies = lower_source(EXHAUSTIVE);
    let (cases, _default) = switch_of(&bodies);
    for (key, bb) in &cases {
        let got = first_const_in(&bodies, *bb);
        let expected = match key {
            -1 => Some(10),
            0 => Some(20),
            1 => Some(30),
            other => panic!("unexpected Trit case key {other}"),
        };
        assert_eq!(
            got, expected,
            "Trit case {key} must route to arm body {expected:?}, got {got:?}"
        );
    }
    assert_eq!(cases.len(), 3, "expected 3 Trit cases, got {}", cases.len());
}

#[test]
fn non_exhaustive_trit_match_defaults_to_trap() {
    // GAP-2 (the soundness teeth): a match missing `1_trit` with no
    // wildcard must have default→Trap. Poison: route default to a real
    // block instead of a Trap → this goes red (uncovered value would
    // silently fall through).
    let bodies = lower_source(NON_EXHAUSTIVE);
    let (cases, default_bb) = switch_of(&bodies);
    let mut keys: Vec<i64> = cases.iter().map(|(v, _)| *v).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec![-1, 0], "only -1/0 covered, got {keys:?}");
    assert!(
        is_trap(&bodies, default_bb),
        "non-exhaustive Trit match must trap on the uncovered value"
    );
}

#[test]
fn match_trit_jit_compiles() {
    // T6: the lowered program JIT-compiles (run-to-value is T7).
    use triet_jit::mir_lower::{JitContext, ShimSymbol};
    let bodies = lower_source(EXHAUSTIVE);
    for body in &bodies {
        body.verify().expect("MIR verify");
    }
    let body_refs: Vec<&triet_mir::Body> = bodies.iter().collect();
    let shims = &[ShimSymbol::fn_2_1(
        "__triet_pow",
        triet_jit::mir_lower::__triet_pow,
    )];
    let mut ctx = JitContext::with_shims(shims);
    ctx.compile_multi(&body_refs)
        .expect("match-on-Trit program must JIT-compile");
}

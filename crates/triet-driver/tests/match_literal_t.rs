//! ADR-0064 — match-on-literal lowering for Integer + Trilean.
//!
//! Routes through the REAL pipeline (parse → typecheck → lower) — no hand-built
//! MIR. Mirror of match_trit_t6.rs: asserts the value-keyed SwitchInt (NOT enum
//! GetDiscriminant) and the GAP-2 default→Trap soundness guard, for the two new
//! scalar scrutinee types.
//!
//! Teeth (Mentor O re-verifies on the final tree):
//!   - Integer cases map value→arm body (poison case value → red).
//!   - Integer no-wildcard → default is Trap (poison Trap→Goto → red).
//!   - Integer with wildcard → default is the wildcard body (NOT a trap).
//!   - Trilean true/false/unknown map to keys 1/-1/0 (poison encoding → red).
//!   - Trilean exhaustive-no-wildcard → default is Trap.

/// Integer, no wildcard → uncovered value must trap (GAP-2).
const INT_NO_WILDCARD: &str = "function classify(x: Integer) -> Integer = match x {\n\
     \x20   1 => 10\n\
     \x20   2 => 20\n\
     }\n\
     function main() -> Integer = 0";

/// Integer, with wildcard → default is the wildcard body, not a trap.
const INT_WITH_WILDCARD: &str = "function classify(x: Integer) -> Integer = match x {\n\
     \x20   1 => 10\n\
     \x20   2 => 20\n\
     \x20   _ => 99\n\
     }\n\
     function main() -> Integer = 0";

/// Trilean, exhaustive 3-way, no wildcard → default trap (GAP-2).
const TRILEAN_EXHAUSTIVE: &str = "function classify(t: Trilean) -> Integer = match t {\n\
     \x20   true => 10\n\
     \x20   false => 20\n\
     \x20   unknown => 30\n\
     }\n\
     function main() -> Integer = 0";

fn lower_source(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (type_errors, pattern_resolutions, method_resolutions) = triet_typecheck::check(&program);
    assert!(type_errors.is_empty(), "type errors: {type_errors:?}");
    triet_lower::lower_program(&program, &pattern_resolutions, &method_resolutions)
        .expect("lowering failed")
}

/// Như `lower_source` nhưng KHÔNG assert type-clean. CHỈ dùng cho test trap
/// defense-in-depth: non-exhaustive scalar match nay bị typecheck reject (E1026,
/// ADR-0064 §8), nên driver thật (main.rs:59) bail TRƯỚC khi lower. Helper này cố
/// ý bypass cổng typecheck để chạm safety-net GAP-2 default→Trap của lowerer —
/// trap chỉ kích nếu một ngày typecheck escape lọt non-exhaustive match. Trap giữ
/// load-bearing per ADR-0064 §8 decision #3.
fn lower_bypassing_typecheck(source: &str) -> Vec<triet_mir::Body> {
    let (program, parse_errors) = triet_parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let (_type_errors, pr, mr) = triet_typecheck::check(&program);
    triet_lower::lower_program(&program, &pr, &mr).expect("lowering failed")
}

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
    panic!("no SwitchInt in classify — match-on-literal must lower to SwitchInt");
}

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
fn match_integer_case_value_maps_to_correct_arm() {
    // INT_WITH_WILDCARD (type-clean per ADR-0064 §8): the wildcard routes to
    // the default, so the value→arm cases are still exactly {1→10, 2→20}.
    let bodies = lower_source(INT_WITH_WILDCARD);
    let (cases, _default) = switch_of(&bodies);
    for (key, bb) in &cases {
        let got = first_const_in(&bodies, *bb);
        let expected = match key {
            1 => Some(10),
            2 => Some(20),
            other => panic!("unexpected Integer case key {other}"),
        };
        assert_eq!(
            got, expected,
            "Integer case {key} must route to arm body {expected:?}, got {got:?}"
        );
    }
    assert_eq!(
        cases.len(),
        2,
        "expected 2 Integer cases, got {}",
        cases.len()
    );
}

#[test]
fn non_exhaustive_integer_match_defaults_to_trap() {
    // GAP-2: Integer match with no wildcard → default→Trap. Poison: route
    // default to a real block → red (uncovered value would fall through).
    // Bypass typecheck: INT_NO_WILDCARD is now rejected at typecheck (E1026,
    // ADR-0064 §8), so the only way to reach the lower GAP-2 safety-net is to
    // route around the typecheck gate. The trap stays load-bearing.
    let bodies = lower_bypassing_typecheck(INT_NO_WILDCARD);
    let (_cases, default_bb) = switch_of(&bodies);
    assert!(
        is_trap(&bodies, default_bb),
        "non-exhaustive Integer match must trap on the uncovered value"
    );
}

#[test]
fn integer_match_with_wildcard_default_is_not_trap() {
    // With a wildcard, the default routes to the wildcard body (99), NOT a
    // trap — distinguishes the two default paths.
    let bodies = lower_source(INT_WITH_WILDCARD);
    let (_cases, default_bb) = switch_of(&bodies);
    assert!(
        !is_trap(&bodies, default_bb),
        "Integer match WITH wildcard must route default to the wildcard body"
    );
    assert_eq!(
        first_const_in(&bodies, default_bb),
        Some(99),
        "wildcard default body must be 99"
    );
}

#[test]
fn match_trilean_value_maps_to_correct_key() {
    // true/false/unknown encode to 1/-1/0 (ADR-0064 §3). Checks value→body
    // mapping so a wrong encoding (e.g. true↔false) is caught.
    let bodies = lower_source(TRILEAN_EXHAUSTIVE);
    let (cases, default_bb) = switch_of(&bodies);
    for (key, bb) in &cases {
        let got = first_const_in(&bodies, *bb);
        let expected = match key {
            1 => Some(10),  // true
            -1 => Some(20), // false
            0 => Some(30),  // unknown
            other => panic!("unexpected Trilean case key {other}"),
        };
        assert_eq!(
            got, expected,
            "Trilean case {key} must route to arm body {expected:?}, got {got:?}"
        );
    }
    assert_eq!(
        cases.len(),
        3,
        "expected 3 Trilean cases, got {}",
        cases.len()
    );
    assert!(
        is_trap(&bodies, default_bb),
        "exhaustive Trilean match (no wildcard) still traps on out-of-domain i64"
    );
}

#[test]
fn match_literal_jit_compiles() {
    use triet_jit::mir_lower::{JitContext, ShimSymbol};
    // INT_NO_WILDCARD dropped: non-exhaustive → typecheck E1026 (ADR-0064 §8),
    // never reaches JIT via the real pipeline.
    for src in [INT_WITH_WILDCARD, TRILEAN_EXHAUSTIVE] {
        let bodies = lower_source(src);
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
            .expect("match-on-literal program must JIT-compile");
    }
}

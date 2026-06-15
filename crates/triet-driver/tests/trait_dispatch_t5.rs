//! ADR-0061 T5 — lower trait dispatch + impl methods.
//!
//! Routes through the REAL pipeline (parse → typecheck → lower) so the
//! `MethodResolution` annotations are produced by the type checker, not a
//! hand-built map (persona mẫu #12). Asserts on the emitted MIR.
//!
//! Teeth (Mentor O re-verifies on final tree):
//!   - GAP-C consistency: the dispatch `callee_name` matches an emitted Body.
//!     Poison either mangle end → callee has no Body → assertion red.
//!   - GAP-B: the impl method's `self` param lowers to `for_type` (Integer).
//!     Poison the SelfType arm (→ Unknown) → red.
//!   - T5.2: the impl method appears in the body list under its mangled name.
//!   - T5.4: the whole program JIT-compiles (no new mechanism).
#![allow(unsafe_code)]

use triet_jit::mir_lower::{JitContext, ShimSymbol};

const PROGRAM: &str = "trait Comparable { function compare(self, other: Integer) -> Trit }\n\
     implement Comparable for Integer { function compare(self, other: Integer) -> Trit = 0_trit }\n\
     function use_it(a: Integer, b: Integer) -> Trit = a.compare(b)\n\
     function main() -> Integer = 0";

const MANGLED: &str = "Integer$Comparable$compare";

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

/// Collect every `CallDispatch` callee name across all bodies.
fn callee_names(bodies: &[triet_mir::Body]) -> Vec<String> {
    let mut names = Vec::new();
    for body in bodies {
        for block in &body.blocks {
            if let triet_mir::Terminator::CallDispatch { callee_name, .. } = &block.terminator {
                names.push(callee_name.clone());
            }
        }
    }
    names
}

#[test]
fn impl_method_emitted_with_mangled_name() {
    // T5.2: the `implement` method becomes a Body named Type$Trait$method.
    // Poison: drop the Item::Implementation loop → no such Body → red.
    let bodies = lower_source(PROGRAM);
    assert!(
        bodies.iter().any(|b| b.signature.name == MANGLED),
        "expected a Body named {MANGLED}; got {:?}",
        bodies.iter().map(|b| &b.signature.name).collect::<Vec<_>>()
    );
}

#[test]
fn dispatch_callee_matches_emitted_body() {
    // GAP-C consistency (the SSOT teeth): the call site dispatches to
    // MANGLED, AND a Body of that exact name exists. A divergence between
    // the two mangle ends (or a wrong hardcoded callee) breaks this.
    let bodies = lower_source(PROGRAM);
    let callees = callee_names(&bodies);
    assert!(
        callees.iter().any(|n| n == MANGLED),
        "expected a CallDispatch to {MANGLED}; got callees {callees:?}"
    );
    // Every dispatched trait callee must resolve to an emitted Body.
    for callee in &callees {
        if callee.contains('$') {
            assert!(
                bodies.iter().any(|b| &b.signature.name == callee),
                "dispatch callee {callee} has no matching Body"
            );
        }
    }
}

#[test]
fn self_param_lowers_to_for_type() {
    // GAP-B: inside the impl method, `self` (params[0]) lowers to the
    // for_type's MirType (Integer), NOT Unknown. Poison the SelfType arm
    // (→ Unknown) → this goes red.
    let bodies = lower_source(PROGRAM);
    let method = bodies
        .iter()
        .find(|b| b.signature.name == MANGLED)
        .expect("impl method Body must exist");
    // Local 0 is the first parameter (`self`).
    assert_eq!(
        method.local_decls[0].ty,
        triet_mir::MirType::Integer,
        "self param must lower to for_type Integer, got {:?}",
        method.local_decls[0].ty
    );
}

#[test]
fn trait_program_jit_compiles() {
    // T5.4 end-to-end: the lowered program JIT-compiles with no new
    // mechanism (direct CallTarget::Jit). Run-to-value is T7.
    let bodies = lower_source(PROGRAM);
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
        .expect("trait dispatch program must JIT-compile");
}

# ADR-0055 — CFG Tail-Expression: block-form function body evaluates to its tail expression

- **Status:** 🔒 LOCKED — Approved by G on 2026-06-11. Drafted by Mentor O on 2026-06-11, grounded in 8 MIR probes.
- **Date:** 2026-06-11
- **Drafted by:** Mentor O (dissected `triet-lower`, constructed probes p1–p8).
- **Signatures:** O ✅ (grounded in real MIR + 8 probes) · G ✅ (approved and stamped 2026-06-11).
- **Related:** [ADR-0052](0052-outcome-abi-implementation.md) (`lower_outcome_return_values` 2-register), [ADR-0053](0053-heap-payload-outcome.md) (heap Outcome drop), [ADR-0054](0054-borrowck-drop-kills-liveness.md) (foundational use-after-Drop). SPEC §expression-based sections.

---

## 1. Context — Semantic Violation in an Expression-Based Language

Triet claims to be **expression-based**: a `Block` IS an `Expression`, and its value is its **tail expression** (the final expression without a trailing `;`). When a developer writes:

```triet
function classify(c: Color) -> Integer {
    match c {
        Red => 100,
        Blue => 200,
        Green => 300,
    }
}
```

they expect the function to return the value of the `match`. **The current compiler silently discards that value and returns `0`.** This is a severe silent misbehavior — no diagnostic, no warning, just an incorrect number returned silently. For an expression-based language, this is a foundational semantic flaw that must be locked by an ADR to prevent regressions.

## 2. Root Cause — MEASURED FROM CODE, NOT GUESSED

There are **two** parallel block-lowering paths in `triet-lower/src/lib.rs`, one correct and one broken:

| Path | Location | Tail Expression | Used For |
|---|---|---|---|
| `lower_block()` | `lib.rs:866` | **DISCARDED** (lines 879–881: `lower_expr(e)` then drops `Local`) | function-body Block + while-body |
| `Expr::Block` arm | `lib.rs:2131` | **EMITTED CORRECTLY** (`result = lower_expr(e)`) | block-as-expression (`= {…}`, RHS let, match arm) |

`lower_function` (lines 642–660) branches based on body syntax:
- `FunctionBody::Block { block }` → `lower_block()` → discards tail → falls through to synthetic `Return { values: vec![] }` (lines 666–677) → **returns unit/0**.
- `FunctionBody::Expression { expr }` → `lower_expr()` + `lower_outcome_return_values()` + correct Return (lines 646–657) → **correct**.

**MIR Evidence (probe, not speculation).** Block-body, tail literal `7`, no `return`:

```
fn main(...) -> Integer {
  bb0: {
    StorageLive(_0)
    _0 = const 7          ← computation finished, sits ready in _0
    Return(())            ← Return carries vec![], discards _0 → returns 0
  }
}
```

Probe matrix (8 cases, measured via `triet-driver run`):

| Probe | Body Form | Tail | Result | |
|---|---|---|---|---|
| p1 | `{ 7 }` | literal | **0** | BUG |
| p5 | `{ if…{a}else{b} }` | if/else | **0** | BUG |
| p7 | `{ match c {…} }` | enum match | **0** | BUG |
| p2 | `{ return 7; }` | — | 7 | ✓ (explicit return) |
| p3 | `= 7` | — | 7 | ✓ (expr-body) |
| p8 | `= { match c {…} }` | enum match | **200** | ✓ (expr-body) |

**p7 vs p8 differ by exactly ONE character `=`.** The absolute root cause: the coexistence of two duplicate code paths. This is UNRELATED to match/if (which are merely common tail expressions), and UNRELATED to JIT (MIR commanded `Return(())`, so JIT returning unit was doing exactly as told). The bug resides entirely in the **lowerer**.

## 3. Decision (G Final Ruling — APPROVED 2026-06-11)

**Theorem locked into history:** *A block-form function body IS an expression; its return value IS the value of its tail expression — identical to an expr-body `= expr`.* The semantic distinction between `FunctionBody::Block` and `FunctionBody::Expression` is **spurious** and is hereby eradicated.

**UNIFY, do not patch.** (G ruling: strictly forbid the cowardly workaround of making `lower_block` return `Option<Local>`.) `lower_function` treats both body forms as an `ExprId` and routes them through the SAME `lower_expr` path:

```rust
let body_expr = match &func.body {
    FunctionBody::Block { block }     => Some(*block),
    FunctionBody::Expression { expr } => Some(*expr),
    FunctionBody::External { .. }     => None,
};
if let Some(e) = body_expr {
    let val = lower_expr(e, arena, &mut c)?;
    if c.is_open(c.cur) {                       // block ending with `return` already closed cur
        let values = lower_outcome_return_values(val, &mut c);
        let span = arena.expression(e).span.clone();
        let cur = c.cur;
        c.term(cur, Terminator::Return { values, span });
    }
}
```

**Four inviolable soundness invariants:**

1. **Guard `c.is_open(c.cur)`** — mandatory. Block bodies frequently terminate with explicit `return` (`{ return a+b; }`) which already closes `cur`; lacking this guard → terminator overwrite / duplicate Return. The guard only ADDS a condition without altering pure expr-body behavior (where `cur` is always open → guard always passes).
2. **`lower_outcome_return_values`** — block-body Outcome-tails previously suffered a double-failure (empty `vec![]` + un-split 2-register ADR-0052). Routing through here fixes Scalar + Fat-Pointer + Outcome in ONE place.
3. **Synthetic fall-through (lines 666–677) KEPT INTACT** — safety net for unit functions falling off the end (no tail, no return); after a guarded Return, it becomes a no-op.
4. **`lower_block` CONFINED to while-bodies** (line 1141) — where discarding the tail is SEMANTICALLY CORRECT (loop body values are discarded). Do not delete the function, no dead code.

**Parity Principle — DO NOT reinvent drop/escape.** The expr-body tail-return path TODAY already runs correctly including heap operations (`100_endgame_string_roundtrip` fixture passes). The fix achieves parity by sharing this EXACT path, WITHOUT crafting new drop logic.

## 4. Teeth (Mandatory Red→Green — Implementer Must Transition)

TWO-WAY teeth: each cell has a fixture **evaluating to the correct value** AFTER the fix, and a poison-revert (removing the fix, restoring `lower_block`-for-function-body) that causes it to **return 0 / incorrect**.

| Tail Form | Scalar | Fat-Pointer (String/Vector) | Outcome (T~E) |
|---|---|---|---|
| bare literal | `{ 7 }`→7 | `{ "hi" }`→correct len | `{ ~+ 5 }`→success |
| `if/else` | `{ if…{a}else{b} }` | String both branches | — |
| enum `match` | `{ match…}`→correct arm | String by arm | error-arm |
| nested block | `{ { 7 } }`→7 | `{ { s } }` | — |
| **parity-return-heap** | — | `{ let s=…; s }` returns the EXACT heap it owns | — |
| control: live `return` | `{ return 7; }`→7 remains correct (guard does not break existing path) | | |

**The `parity-return-heap` cell = THE BOUNDARY OF LIFE AND DEATH.** Any ownership errors (Use-After-Free, Double-Free, mislocated Drops) hidden in the expression lowering path will be EXPOSED here. The fixture must: run green + free-exactly-once (no double-free when heap is simultaneously returned and dropped by scope-pop). If this cell breaks → it exposes an existing expr-body flaw, NOT a flaw introduced by this fix — but it must still be closed before O signs off.

## 5. Execution Order

1. Write matrix teeth §4 (fixtures in `triet-driver/tests/fixtures/`) — RED prior to fix.
2. Unify `lower_function` body path according to §3.
3. Teeth TRANSITION red→green; poison-revert demonstrates reversion to 0.
4. Full gate (`bash scripts/gate.sh`) — raw 4 sections.

## 6. Consequences

- **Positive:** eliminates duplicate body paths; locks expression-based semantics; simultaneously fixes Outcome-tails + Fat-Pointer-tails (previously double-dead).
- **Scope:** 1 file (`triet-lower/src/lib.rs`), ~15 lines in `lower_function`. NO changes to MIR types, NO JIT modifications, NO borrowck modifications.
- **Risks:** the `parity-return-heap` cell may expose existing expr-body drop-escape bugs → if exposed, open a sub-task to resolve before signing (no swallowing).
- **Out of Scope (Rule 4 — DO NOT expand scope):** match-on-integer-literal is currently unsupported in the lowerer (`expected enum variant`); out of scope for this campaign, set aside. Match teeth only use enums.

## 7. Operational Directives for Implementer (NO Additional Blueprints — §3 is the Template)

- DO NOT touch `lower_block` other than keeping it for while-bodies. DO NOT delete it.
- DO NOT remove the `is_open` guard — it is the sharp edge of early `return` in function bodies.
- Counting/structural tests must prioritize route-lower (`lower_source` through the real pipeline), DO NOT hand-build `MirBuilder`.
- The `parity-return-heap` cell must have verified teeth (poison red + free-exactly-once) — O will manually re-verify on final code, no claims accepted on trust.

## 8. Amendment 2026-06-11 — DESCOPE 3 Teeth Cells (Append-Only, §3 Intact)

**Context:** Prior to writing teeth, each cell in §4 was probed via expr-body `= …` (the exact path block-bodies will follow after the fix). O independently verified (NEVER trusting implementer claims), grounded in MIR:

| Cell §4 | expr-body Today | Verdict |
|---|---|---|
| if/else **heap** | `length` reads 0 (merge loses len/cap) | 🔴 BLOCKED |
| enum match **heap** | garbage (merge loses len/cap) | 🔴 BLOCKED |
| enum match **outcome** | MIR verify `arity expected 2 got 1` | 🔴 BLOCKED |

**Root Cause (Independent of ADR-0055):** branch value-merge emits `_5 = move _4` — a 1-slot move (1 i64 = `ptr` only). A 24-byte heap `{ptr,len,cap}` → loses len/cap; a 2-slot Outcome `{disc,payload}` → arity mismatch. Flaw lies in `Expr::If`/`Expr::Match` value-merge, NOT in the `lower_function` body path. Reproduces identically in `= if…/= match…` (pre-fix), hence UNRELATED to ADR-0055.

**Ruling by O (Gatekeeper for teeth scope):** **(A) DESCOPE.** ADR-0055 scoped itself as `~15 lines in 1 file, no JIT/borrowck` (§6); fixing multi-slot branch-merge involves branch-codegen → out of scope. ADR-0055 proceeds with **9 sound cells** (literal/if/match/nested × scalar + heap-literal/heap-nested + parity-return-heap + outcome-literal).

**The `parity-return-heap` Cell — O PROVED sound, not merely claimed:** MIR exposes `Drop(_1); Return(_1)`; O measured (a) stress realloc post-return → `length=5`, status 0 directly, no SIGABRT; (b) explicit `return s` generates **identical** MIR → standard heap-return pattern of the codebase (`100_endgame` PASS), governed by ADR-0054 Return-leniency. ADR-0055 tail-form = pure parity, 0 new behavior.

**Follow-up:** "if/match value-merge multi-slot" campaign → **ADR-0056** (2 signatures from O+G prior to implementation; touches `Expr::If`/`Expr::Match` lowerer + potentially JIT branch-codegen).

- **Amendment Signatures:** O ✅ (grounded in probes B1/B2/B3 + parity stress 2026-06-11) · G ✅ (approved descope 2026-06-11 — teeth narrowed, §3 decision UNCHANGED).

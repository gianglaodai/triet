# ADR-0055 — CFG Tail-Expression: block-form function body evaluates to its tail expression

- **Status:** 🔒 LOCKED — Approved by G 2026-06-11. Drafted by Mentor O 2026-06-11, grounded from 8 MIR probes.
- **Date:** 2026-06-11
- **Drafted by:** Mentor O (analyzed `triet-lower`, constructed probes p1–p8).
- **Signatures:** O ✅ (grounded from real MIR + 8 probes) · G ✅ (approved + sealed 2026-06-11).
- **Related:** [ADR-0052](0052-outcome-abi-implementation.md) (`lower_outcome_return_values` 2-register), [ADR-0053](0053-heap-payload-outcome.md) (heap Outcome drop), [ADR-0054](0054-borrowck-drop-kills-liveness.md) (use-after-Drop foundation). SPEC §expression-based sections.

---

## 1. Context — The semantic crime of an expression-based language

Triet identifies as **expression-based**: a `Block` IS an `Expression`, and its value is its **tail expression** (the final expression, without a `;`). A programmer writes:

```triet
function classify(c: Color) -> Integer {
    match c {
        Red => 100,
        Blue => 200,
        Green => 300,
    }
}
```

and expects the function to return the value of the `match`. **The current compiler implicitly discards that value and returns `0`.** This is a vile implicit behavior—no diagnostics, no warnings, just a silent, incorrect number. For an expression-based language, this is a fundamental semantic flaw that must be locked via ADR to prevent any future regressions.

## 2. Root cause — MEASURED FROM CODE, NOT GUESSWORK

There are **two** parallel block-lowering paths in `triet-lower/src/lib.rs`, one correct and one incorrect:

| Path | Location | Tail expression | Used for |
|---|---|---|---|
| `lower_block()` | `lib.rs:866` | **DISCARDED** (line 879–881: `lower_expr(e)` then drops `Local`) | function-body Block + while-body |
| `Expr::Block` arm | `lib.rs:2131` | **PROPERLY PROPAGATED** (`result = lower_expr(e)`) | block-as-expression (`= {…}`, RHS let, match arm) |

`lower_function` (line 642–660) branches the body based on syntax:
- `FunctionBody::Block { block }` → `lower_block()` → discards tail → falls through to synthetic `Return { values: vec![] }` (line 666–677) → **returns unit/0**.
- `FunctionBody::Expression { expr }` → `lower_expr()` + `lower_outcome_return_values()` + correct Return (line 646–657) → **correct**.

**MIR Evidence (probe-based, not inferential).** Block-body, tail literal `7`, no `return`:

```
fn main(...) -> Integer {
  bb0: {
    StorageLive(_0)
    _0 = const 7          ← computed value, already residing in _0
    Return(())            ← Return carries vec![], discards _0 → returns 0
  }
}
```

Probe matrix (8 cases, measured via `triet-driver run`):

| Probe | Body Type | Tail | Result | |
|---|---|---|---|---|
| p1 | `{ 7 }` | literal | **0** | BUG |
| p5 | `{ if…{a}else{b} }` | if/else | **0** | BUG |
| p7 | `{ match c {…} }` | enum match | **0** | BUG |
| p2 | `{ return 7; }` | — | 7 | ✓ (explicit return) |
| p3 | `= 7` | — | 7 | ✓ (expr-body) |
| p8 | `= { match c {…} }` | enum match | **200** | ✓ (expr-body) |

**p7 vs p8 differ by exactly ONE character: `=`.** The absolute root cause: the existence of two redundant code-paths. This is NOT related to match/if (they are merely common tail types), and NOT related to JIT (the MIR already commanded `Return(())`, so the JIT returning unit is correct). The error is purely at the **lowerer** level.

## 3. Decision (G finalized the decision — APPROVED 2026-06-11)

**Theorem locked into history:** *A block-form function body IS an expression; its return value IS the value of its tail expression—identical to an expr-body `= expr`.* The semantic distinction between `FunctionBody::Block` and `FunctionBody::Expression` is **artificial** and shall be abolished.

**UNIFICATION, not patching.** (G forbids the cowardly approach of forcing `lower_block` to return `Option<Local>`.) `lower_function` shall treat both body types as a single `ExprId` and pass them through the SAME `lower_expr` path:

```rust
let body_expr = match &func.body {
    FunctionBody::Block { block }     => Some(*block),
    FunctionBody::Expression { expr } => Some(*expr),
    FunctionBody::External { .. }     => None,
};
if let Some(e) = body_expr {
    let val = lower_expr(e, arena, &mut c)?;
    if c.is_open(c.cur) {                       // block ends with a `return` that already closed cur
        let values = lower_outcome_return_values(val, &mut c);
        let span = arena.expression(e).span.clone();
        let cur = c.cur;
        c.term(cur, Terminator::Return { values, span });
    }
}
```

**Four inviolable soundness points:**

1. **Guard `c.is_open(c.cur)`** — mandatory. Block-bodies often end with an explicit `return` (`{ return a+b; }`) that has already closed `cur`; without the guard → overwriting the terminator / duplicate Return. The guard only ADDS a condition; it does not change the behavior of a pure expr-body (where `cur` is always open → guard always passes).
2. **`lower_outcome_return_values`** — block-body Outcome-tails currently suffer a double failure (empty `vec![]` + no 2-register split per ADR-0052). Routing through here fixes Scalar + Fat-Pointer + Outcome all in ONE pass.
3. **Synthetic fall-through (line 666–677) RETAINED** — a safety net for unit-fns falling through the tail (no tail, no return); after the guarded Return, it becomes a no-op.
4. **`lower_block` REMAINS in while-body

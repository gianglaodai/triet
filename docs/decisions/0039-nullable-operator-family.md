# ADR 0039 — Nullable Operator Family `?`-family (`?+>`, Fate of `?0>`, Prohibition of `?->`)

**Status:** **Approved — DESIGN LOCK** (Author, 2026-06-05; Mentor O verified
ADR-0020 §3.1 flatten + lexer precedent).
**Implementation:** **DEFERRED** — awaiting nullable/Outcome lowering (Tier B/C).
Backend currently cannot lower even `?.`.

## Context

The author's original question: what syntax replaces map/flatMap (Monad) for plain `T?`?
The Outcome family (`~+>`/`~0>`/`~->`, ADR-0020) already covers `T~E`/`T?~E`; plain `T?` only has
`?.` (safe call) + `?:` (Elvis) — lacking a general transformer (mapping a value without
member access, e.g., `opt ?+> |s| parse(s)`).

Guiding principle: **morphological symmetry** — the Outcome family uses prefix `~`, the
Nullable family uses prefix `?`. Learning one family teaches the other, fitting AI-first.
However, symmetry is a means toward predictability, not an end in itself — Clause 2
below eliminates a redundant token in service of this very principle.

## Clause 1 — `expr ?+> |bind| body` (Unified Map + FlatMap)

**Semantics** (symmetric to `~+>` ADR-0020 §3.1):
- `expr` has a concrete value → bind to `bind`, evaluate `body`, replace value with result.
- `expr` is null (`~0`) → **pass through unchanged** (null propagates, body does not execute).

**Body return** (directly inherited from ADR-0020 §3.1 "Body return", lines 378-380):
- Plain `U` → auto-wrap into `U?`. This is **map**.
- Nullable `U?` → **auto-flatten**; never produces `U??`. This is **flatMap**.
  This flattening rule is NOT new — `~+>` already flattens nested outcomes from ADR-0020
  (§3.1: "Outcome `T'~E` or `T'?~E` (same error type) → flatten; nested
  outcome unfolded"). The nullable variant is even cleaner: no "same
  error type" constraint is required since null has only one representation.
- Early-return forms (`return ...`, `panic(...)`) → exit enclosing function,
  identical to Outcome.

```triet
// map: body returns String (plain) → result String?
let name: String? = get_name() ?+> |n| n.to_uppercase()

// flatMap: parse returns Integer? → auto-flatten, result Integer? (NOT Integer??)
let n: Integer? = get_input() ?+> |s| parse(s)
```

**Consequence:** `T??` (nested nullable) remains UNDEFINED in Triet —
auto-flattening guarantees no path can produce it via `?+>`.

## Clause 2 — `?:` RHS is an Expression; `?0>` is NOT Generated

**Firmly locked in SPEC:** the right-hand side of Elvis `?:` is an **Expression** — including
Block expressions and `Return` (Return is an Expr, `ast_expr.rs:131`):

```triet
let val = opt ?: {
    log("fallback")
    default_val()
}
let user = find_user(id) ?: return ~- AppError.NotFound   // guard pattern
```

Because `?:` already covers both short fallbacks and block/guard patterns, **`?0>` is redundant
syntax — killed at conception** (Simplicity First, "one way to do it"). Outcome's `~0>`
exists because Outcome LACKS Elvis (`~:` was eliminated in ADR-0020 §3.7); since Nullable
already has `?:`, it is not duplicated.

> Scope Note: Triet **has no `throw`** (kernel language, no exceptions —
> all non-return early-exits are `panic(...)`, ADR-0020 §3.1). `break`/`continue`
> are loop-control constructs (SPEC around §899-915); their validity in the RHS of Elvis
> follows general Expression context rules, without dedicated commitments in this ADR.

## Clause 3 — Strict Prohibition of `?->`: **E1046 NullableHasNoErrorState**

`T?` has only 2 poles: concrete value (`+`) and null (`0`). It has NO negative (error) pole.
A developer typing `opt ?-> |e| ...` triggers an immediate compile error:

```
E1046 NullableHasNoErrorState
Type `T?` has no error state.
[Fix 1] Use `T~E` (Outcome) if you need error handling, then `~-> |e| body`.
[Fix 2] Use `?:` to provide a default when the value is null.
```

(Format per ADR-0027. Error code E1046 = monotonic-next after E1045.)

> **Correction 2026-06-17:** Original ADR (written 2026-06-05) recorded this code as
> **E1041** (which was vacant at the time). Subsequently E1041 was assigned to
> `NoMatchingOverload` → the official code for `NullableHasNoErrorState` is **E1046**
> (monotonic-next after E1045, avoiding backfilling the E1038 gap belonging to ADR-0020). All
> E1041 references in this ADR are corrected to E1046 (Phase 14.3, approved by Mentor O).

**Implementation note:** reserve token `?->` in the lexer (lexable but rejected
intentionally) to emit E1046 with a clear diagnostic, rather than splitting into
`?` + `->` and failing with a vague parse error — matching the lexer-rejection technique
used for deprecated `~?`/`~:` (ADR-0020 §3.7).

## Lexer

`?+>` (and reserved token `?->`) are 3-character compound tokens, **with no internal
whitespace**, longest-match positioned BEFORE `?~`/`?.`/`?:`/`?` — following the precedent
where `?~` precedes `Question` (`token.rs:267-269`). No collisions:
Triet lacks ternary `a ? b : c` and lacks an independent `+>` operator.
Precedence: same tier as `~+>`/`~->` (postfix transformer, SPEC §4.6).

## Alternatives Considered

- **`?0>`** — redundant, see Clause 2.
- **`?->`** — permanently prohibited, E1046 (Clause 3). Not "deferred".
- **`T??` nested nullable** — remains undefined; auto-flattening avoids it.
- **Applying `~+>` directly to plain `T?`** — rejected; `T?` uses the `?` family, Outcome uses
  the `~` family. The two families remain cleanly separated by prefix, preserving symmetry.

## Summary Table of the Two Families

| Action | Nullable `T?` | Outcome `T~E` / `T?~E` |
|---|---|---|
| Safe member access | `?.` | — (explicit unwrap via `~+>`/`~->`) |
| Fallback (short + block + guard) | `?:` (RHS = any Expression) | — (`~:` eliminated, ADR-0020 §3.7) |
| Positive map + flatMap | `?+> \|v\| body` | `~+> \|v\| body` |
| Zero pole handling (null) | `?:` | `~0> body` (`T?~E` only) |
| Negative pole handling (error) | ❌ **E1046** | `~-> \|e\| body` |

## References

- [ADR-0020](0020-outcome-error-handling.md) §3.1 (semantics + Body return +
  flatten — direct inheritance source), §3.7 (`~:`/`~?` deprecated, lexer refuses).
- [ADR-0027](0027-diagnostic-format-standard.md) — E1046 diagnostic format.
- SPEC.md around §339-342 (`?.`/`?:`), around §1345 (Elvis precedence — updated
  to note "RHS is Expression" per Clause 2), around §899-915 (`break`/`continue`).
- `crates/triet-lexer/src/token.rs:267-304` — existing `?` family + longest-match
  precedent.
- [ADR-0038](0038-comparable-trait-deferred.md) — matching "design lock,
  implementation deferred" pattern.

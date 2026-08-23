---
name: future-comparable-trait-and-monad-gap
description: "Design session 2026-06-05: (1) the Comparable trait / compare()->Trit — design LOCKED in ADR-0038, deferred until the trait system exists; (2) map/flatMap — FULLY CLOSED in ADR-0039: ?+> (map + flatMap with auto-flatten), ?0> killed, ?-> forbidden with E1041."
metadata:
  node_type: memory
  type: project
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

Design session 2026-06-05 (Mentor O + the author), two language questions. Full detail lives in the repo — this memory keeps only what is NOT in the repo, plus pointers.

## 1. Three-state comparison → DECIDED, written up in `docs/decisions/0038-comparable-trait-deferred.md`
- `Comparable` is a **trait** (the author confirmed: Triết will definitely have traits, not interfaces — there is NO trait system yet: `Item` is only Function/Struct/Enum, and the lexer has no `trait`/`impl` keyword).
- `compare() -> Trit` (NOT an Ordering enum — an i64 discriminant betrays the ternary identity, and the name Ordering is already taken by atomics). Total ordering only; unknown stays with the Ł3 `==`/`<` operators.
- **Deferred until the trait system exists; do NOT build a temporary built-in** (Option A + the dead-field lesson). TODO.md has a "Deferred — design locked" section.
- Do not reopen this debate — read ADR-0038 first.

## 2. map/flatMap (monads) for `T?` → CONCLUSION: nearly all of it already exists, with ONE open gap
The author asked "what syntax replaces a monad's map/flatMap for T?". The answer, after verifying against SPEC: **Triết has ALREADY designed this family** (ADR-0020, SPEC.md §Outcome operators around lines 385-407):
- `expr ~+> |val| body` = the functor **map** (SPEC says so verbatim), `~0>` supplies a null default, `~-> |err| body` transforms the error.
- **flatMap/bind** = `~-> |e| return …` in EARLY-RETURN mode — the compiler infers MAP versus bind from the presence of `return`. No "flatMap" name is needed.
- For a plain `T?`: `?.` optional chaining + `?:` default (SPEC.md around lines 339-342).
- The old `~?`/`~:` are deprecated and the lexer refuses them (ADR-0020 §3.7).

**THE GAP IS CLOSED (2026-06-05, the same day):** settled in `docs/decisions/0039-nullable-operator-family.md` (proposed by Mentor G, approved by the author, verified by Mentor O):
- **`?+>`** = map and flatMap unified for `T?` (auto-flattening `U?`→`U?`, inheriting the flattening of `~+>` from ADR-0020 §3.1:379 — it never produces a `T??`).
- **`?0>` was killed** — the RHS of `?:` is settled as an Expression (Block + Return), so `?0>` is redundant.
- **`?->` is permanently forbidden** — E1041 NullableHasNoErrorState; the lexer reserves the token so the diagnostic stays clean.
- Prefix symmetry: the `?` family is for `T?`, the `~` family is for Outcome — `~+>` does NOT apply to a plain `T?`.
The lesson from that session: Mentor O raised a false alarm ("(b) flattening breaks the symmetry") by not reopening ADR-0020 §3.1 (the flattening was already there) — verify-before-ruling applies to mentors too. In the other direction, it caught Mentor G inventing "Throw" (Triết has no exceptions — only panics). An outside advisor (Mentor G) goes through the same verification as the author.

General note: these operators are design-locked but **NOT implemented** in the rewritten backend (Tier A only reached scalars/structs/enums). SPEC is the correct source when the time comes to lower them.

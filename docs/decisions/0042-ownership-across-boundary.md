# ADR-0042: Ownership Across Function Boundary — B7-lift (Move-only)

**Status:** CLOSED — Signed by Mentor O (semantics/soundness, 2026-06-07) + Signed by Mentor G (layout/ABI, 2026-06-07). Implementation complete at `86b7034`.
**Date:** 2026-06-07
**Author:** AI (implementation), final decision: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Move semantics for heap types (String, Vector) across user-defined function boundaries. DO NOT touch borrow params (`&+ T`, `&0 T`, `&- T`) — defer to Level C.

---

## Summary

B7-lift removes two refusals at `triet-lower/src/lib.rs:492` (heap type param) and `:1360` (heap type arg), adds caller zeroing at `CallDispatch::Jit` + borrowck move-marking for user functions (keyed to `CallTarget::Jit`, not `builtin_shim_meta`). Move-only scope — borrow params are out of scope per consensus of the two mentors. Return path is already wired (M4 return-escape applies to user fn).

---

## §0 — Verified Data (Phase 0 probe, 2026-06-07)

5 probes run on the actual driver; temporary tree removes 2 refusals (already restored).

| # | Probe | Result | Findings |
|---|-------|---------|-----------|
| P1 | `consume(my_string)` — caller zeroing? | **SIGABTRB** `double free` | Caller DOES NOT zero slot — M1-M3 do not extend across call boundary |
| P2 | Same as P1 | Same as P1 | Confirmation of duplication |
| P3 | `len(s)` after `consume(s)` | **SIGABRT** (double-free before `len`) | Borrowck DOES NOT catch use-after-move across call — no E2420 |
| P4 | `let t = make()` → `len(t)` | **5** (success) | Return path working: M4 skips callee Drop, caller Drops once |
| P5 | `Integer?` across boundary + Elvis | **7** (success) | PA-3c MIN sentinel preserved across boundary |

O independently reproduced 5/5 probes; results match 100%.

---

## §1 — Decision (6 questions)

### Q1: Caller zeroing at CallDispatch::Jit

After `CallDispatch` with `CallTarget::Jit`, the caller must zero the Move-type arguments passed to the callee. Mechanism: loop through args, `!is_copy(arg_ty)` → emit `Statement::Const { value: 0 }` + `Statement::Assign { dest: arg, source: 0 }` within the caller's return block. Maintain the current M1 mechanism — only extend the scope from builtin to user functions.

### Q2: Callee drop remains unchanged

The callee already drops parameters upon scope exit (`owned_locals` + `pop_scope` mechanism). No change required. Coordinate with Q1 to avoid double-free: caller zeros → callee receives original value → callee drops once → caller has already zeroed, so drop is a no-op.

### Q3: Borrow params CUT — move-only

`&+ T`, `&0 T`, and `&- T` parameters across user function boundaries are DEFERRED to Level C. Both mentors agree: this B7-lift scope only covers move semantics. The current refusal for heap-type parameters does not distinguish between Move vs. Borrow → remove refusal ONLY for Move-passing; borrow-passing remains an Error (currently all heap params use Move passing by default, so in practice, all are removed).

### Q4: E2420 keyed to CallTarget::Jit, check-then-mark

Current Borrowck M3 (`checker.rs:790-805`) only marks `Moved` for `CallDispatch` with `builtin_shim_meta`. Fix: add a `CallTarget::Jit` branch — loop args, `!is_copy(arg_ty)` → mark `VarState::Moved`.

**Check-then-mark:** Before marking, check if the argument is already `Moved` → if so, trigger E2420 (aliased double-move: `foo(s, s)` → callee receives 2 params with the same pointer → drops both → double-free WITHIN the callee). Pattern: `matches!(state.var_states.get(arg), Some(VarState::Moved))` → error, then proceed to mark.

### Q5: T? nullable across boundary

- `Integer?` (Copy): already works, MIN sentinel is preserved (P5). No new code required.
- `String?` (Move): repr is defined (null=MIN)

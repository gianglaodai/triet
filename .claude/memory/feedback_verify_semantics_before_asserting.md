---
name: feedback_verify_semantics_before_asserting
description: A recurring pattern — the author asserts compiler semantics from a guess and then encodes the guess into a test; the mentor must demand experimental evidence.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cbfcad37-8830-40cb-a053-1a01523fea6d
---

During the 2026-06-04 rewrite session, the author repeated the same mistake **three
times**: asserting something about compiler/language semantics **from a guess**, then
encoding that guess into a test or a comment:

1. **Outcome pass-through** — guessed the compiler was wrong (it happened to be right, by luck).
2. **Fixture 20 borrow_fail** — guessed "a double `&0 mutable` SHOULD be rejected with
   E2440"; in reality two unused mutable borrows are LEGAL under NLL → the compiler was
   RIGHT to accept it. The test asserted a bug the compiler does not have → it nearly
   drove a false-positive regression.
3. **Fixture 21 drop_while_borrowed** — guessed "a latent E2450"; in reality that
   construct fundamentally cannot be E2450 (an unused borrow of a Copy scalar), AND
   E2450 was dead end to end because the lowerer emitted no Drop at all.

**2026-06-09 — THE 4th TIME (A1 is_propagated):** the author labelled the
is_propagated guard "future-proof / MIR cannot produce a Drop-before-deref pattern"
— twice, and both times wrong. O built a MIR probe proving a LIVE reachable bomb: a
nested-scope return borrow really does produce `Drop(_0)` before `length(_2)` → a UAF
slipping through silently. A wrong label nearly shipped a bomb.

**Why:** the author is not a compiler engineer; their intuition about NLL/S6/Outcome
is not reliable. When a wrong guess gets encoded into the safety net, the WRONG test
drags the project toward the bug (whoever comes next "fixes" the compiler to satisfy
the wrong test = a real regression).

**How to apply (to both of us):**
- Whenever the author asserts something about semantics (a borrow rule, a type rule,
  "this should fire EXXXX"), DEMAND experimental evidence BEFORE accepting it: run
  `triet-driver`, diff against a proven example, or quote SPEC §10.
- **G's rule §2 (REFUSE OVER GUESS — extended 2026-06-09):** before calling a guard
  or code path "dead", "future-proof", "unreachable", or "unproducible from MIR", you
  MUST personally insert `panic!("Unreachable")` / `Err(JitError::Unsupported)` there
  and prove no test reaches it. If you cannot prove it → it is a HOLE, not dead code.
  Never accept a bare "latent/should/probably/future-proof".
See [[feedback_stability_over_speed]].

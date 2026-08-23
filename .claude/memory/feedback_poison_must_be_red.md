---
name: feedback-poison-must-be-red
description: "G's IRON LAW (2026-06-09) — every test claiming to fix a structural bug must be poisoned red before it is accepted; never trust a test's name."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7f9fbd79-3ba3-4ebd-b376-fd8db532831b
---

**IRON LAW (formalized by G on 2026-06-09, after the B1a S1 round-4 lesson):** for every PR or commit claiming a "structural fix", O's review procedure **MUST** add one step: **poison the core logic → if the test does not go red → REJECT to the author's face.**

**Why:** B1a S1 had a test named `mirtype_structural_fixes_nullable_vec_misclassification` — the name was right (it was meant to guard the `Vector<Integer>?` ordering bug), but the input was `Integer?` (= Nullable(Integer)). An Integer is never a vec → asserting `!is_vec()` was trivially true and NEVER touched the dangerous case. O poisoned `is_vec` to match through Nullable → 44 tests passed (no bite). A bland green test is worth less than no test at all — it manufactures a feeling of safety about exactly the bug it claims to have fixed.

**How to apply:**
1. Never trust a test's NAME. Make it prove itself with blood (the red of a panic).
2. Manual teeth: cp a snapshot to /tmp FIRST ([[feedback-teeth-never-git-checkout]]), poison the core logic, run the named test — it MUST go red; restore with cp, NEVER with git checkout.
3. A structural-fix test must use exactly the input that reproduces the bug (e.g. Nullable(Vector), not Nullable(Integer)).
4. Apply it to any "I already teeth-verified this" claim from the author or D — O rebuilds it personally and trusts nothing.

**THE NAMED-LOCAL LAW (carved by G on 2026-07-01, HM-P1b round 2 — the 2nd vacuous tooth):** a test about Move/Consume/drop obligations **MUST bind the value to a NAMED variable** (`let s = "hi"; insert(m,1,s)`), never an inline literal or temporary (`insert(m,1,"hi")`). Reason: a literal temporary **has no drop obligation** in scope (MIR emits NO `Drop` for it) → poisoning the consume flag (`arg_consumes`/zero-on-move) is **inert**, because there is no caller-side Drop to double-free. D submitted tooth #1 with a SIGABRT 134 using a literal → O planted the poison `arg_consumes[2]=false` → the test STAYED GREEN (vacuous); O proved it with the MIR: the literal version drops `Drop(_2) Drop(_5)` (the value is missing), the named-local version drops `Drop(_2) Drop(_3) Drop(_5)` (it has `Drop(_3)`) → poisoning the named-local version exits 134. **The compiler only puts a noose around the lifetime of a NAMED variable.**

Related: [[mentor-o-persona]] (verify-don't-trust), [[feedback-verify-semantics-before-asserting]] (the author guessing semantics and encoding them into a test), [[feedback-failure-mode-precision]] (measuring the right signal, 134/139/leak).

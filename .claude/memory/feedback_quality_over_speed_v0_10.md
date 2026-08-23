---
name: feedback-quality-over-speed-v0-10
description: "The user was explicit mid-v0.10 (2026-05-30) — speed is not a concern; code quality is the main contract. I (the AI) am the technical-quality owner, and the author depends on my competence to verify."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 5ad1339c-d4d2-4c07-9823-7d76ba88d258
---

**The quality bar for v0.10 (the author, explicitly, 2026-05-30 after v0.10.x.interp.1):**
> "Do not rush; your speed will be more than enough. What matters is that we guarantee code quality. At this point I have to depend on your competence."

**Why:** the author has no compiler-engineering background ([[user_role_webapp_dev_visionary]]) and cannot eyeball every line of unsafe Rust, ABI marshalling, or borrow-checker enforcement. The AI is the technical-quality owner for the v0.10 implementation. The trust comes with responsibility — it is not freedom to move fast, it is freedom to raise the bar.

**How to apply:**

1. **A pre-commit self-audit is MANDATORY** for every sub-task. Re-read the diff with fresh eyes before committing; ask "would a senior engineer approve this PR?". Surface any code smell (a dead-code workaround, a misleading error variant, a missing SAFETY comment, an untested error path) before committing. Either fix it now or declare it deferred with a reason.

2. **Plan before coding for any sub-task involving unsafe or the ABI** (jit.1 and jit.2 specifically). Write a short plan (5-10 bullets) before typing Rust:
   - Files touched + estimated LOC
   - The test gate before committing (happy path + error path)
   - Per unsafe block: the SAFETY invariant + the audit comment shape (ADR-0032 §5, mandatory)
   - Deferred items + an explicit reason

3. **Refuse over guess** (VISION §6) when no ADR covers it. If ADR-0032/0033 do not lock a detail, **ask the author** before writing code; never silently pick and ship.

4. **The test-coverage bar for every sub-task:**
   - Happy path: at least 1 test per new op/feature
   - Error path: at least 2 tests (typical wrong args, edge cases)
   - Integration: if there is a cross-crate path, write an integration test in `tests/`
   - Benches / proptests for ABI-critical paths (per ADR-0032 §7)
   - A VM↔JIT parity test for each of the 43 shims (per ADR-0032 §7.2)

5. **Tier down honestly:** if a sub-task blows out its LOC estimate (>2× the plan), pause and report to the author with 3 options: (a) tier down the scope, (b) split it into 2 commits, (c) defer part of it. NEVER silently inflate scope.

6. **Verify end to end** before marking `[x]`: run `dao run` on the relevant example, not just the unit tests. If it is UI-adjacent (CLI output, error message format), check the format matches ADR-0027.

**Precedent (the v0.10.x.interp.1 post-mortem):** after commit `be9e535` I found 2 code smells myself (a redundant `const _:` + `compare_exchange` using the wrong error variant). The author could not catch them because they are internal Rust matters. That is exactly the failure mode this feedback prevents. The handling: admit it and fix it in the follow-up commit `[v0.10.x.interp.1b]`.

Cross-reference: [[feedback_proactive_audit]] (the audit window near a phase close); [[feedback_stability_over_speed]] (architectural decisions get an ADR, nothing is shipped carelessly); [[feedback_implementer_choice]] (the delegation precedent — freedom with responsibility).

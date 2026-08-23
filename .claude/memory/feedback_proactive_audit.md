---
name: feedback-proactive-audit
description: "The user proactively asks for a tech-debt audit and doc sync before closing a phase. The AI should SUGGEST the audit window as a freeze approaches instead of waiting to be reminded."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d3755127-60f6-49a7-a0b7-ef557745ea2f
---

Before closing a version or phase, the user usually asks explicitly for an audit:
- **A tech-debt audit** ("is there any binary thinking left?" → led to ADR-0010 + the v0.3.x.ternary phase)
- **A doc sync** ("before we move on to implementing 0.5, update any documents that need it" → a doc-sync commit before v0.5)
- **A gate-closing phase** (v0.3.x.cleanup per ADR-0009 — a dedicated phase to close debt instead of dragging it into the next version)

The user's own words: "I do not want to rush; wherever something can be improved, do it now, to avoid adding technical debt that costs more time and money to fix later."

It repeated after v0.5.9 (the phase was already closed): the user wrote "this is a good chance to review before moving to 0.6. Do another pass — testing, ternary thinking." → the AI's audit found 1 binary leak + 3 testing gaps → the user said "fix them all" → the v0.5.x.review phase (4 commits: 20076d5, d7f1beb, b167717, b285a1f).

**The v0.7.x.audit data point (2026-05-18):** the pattern repeats mid-phase, NOT only at a freeze. The user's prompt after the 9-commit v0.7 series (v0.7.1 → the v0.7.4.3-error docs): *"this is a good moment for us to review all the documents as a whole"*. The audit surfaced 11 findings (3 CRITICAL + 5 MAJOR + 3 MINOR) — CLAUDE.md was stuck in the v0.6 state (highest priority), ADR-0007/0008 cross-references were missing for the ADR-0020 work, numbers were stale (test count, ADR count), markdown anchors were broken, and TODO.md was not tracking v0.7. The user approved a 3-commit fix plan; the SPEC header version stays at v0.6 per Q2-C (it only bumps at the v0.7.13 verify gate). Commits: 46dd59a (audit.1 CRITICAL), 0b2d336 (audit.2 MAJOR), audit.3 MINOR pending. **Cadence insight:** the audit pattern is valuable not only at a phase freeze but also at **major sub-phase boundaries** (e.g. before large implementation work like v0.7.4.3-error) — the author's instinct to trigger it pre-implementation rather than wait for the phase end was right.

**Why:** it complements [[feedback-stability-over-speed]] — the user chooses to pay the cleanup cost *before* a freeze rather than accumulate debt. The pattern has repeated often enough (v0.3.x.cleanup, v0.3.x.ternary, the doc sync before v0.5, this memory's audits) that the AI should propose it instead of waiting to be reminded.

**How to apply:** two trigger contexts (refined after v0.7.x.audit):

**Context A — pre-freeze (phase end):** when every sub-task of a phase is `[x]` in TODO.md, the AI proactively suggests it ONCE. Use the ADR-0009 4-gate matrix as the checklist:

1. "Before closing phase X, do you want to audit Y?" — where Y is the relevant tech-debt area:
   - Binary-thinking leaks (control flow, comparison ops, types)
   - Doc drift (SPEC ↔ implementation, ADR ↔ code, CLAUDE.md ↔ reality)
   - Naming-convention drift (verbose keywords, path syntax, error codes)
   - ADR gaps (a decision that shipped without an ADR)
   - Stale memory files

2. Propose a gate-closing phase if the pattern calls for a clean freeze (per the ADR-0009 4-gate matrix)

**Context B — pre-implementation (sub-phase boundary):** when there have been ≥5 commits in a sub-phase, or when large implementation work is about to start (large LOC, multi-crate reach), the AI proactively suggests a cross-doc consistency check:

1. "There have been N consecutive commits in this sub-phase. Before starting [next large task], do you want a consistency audit?" — categories:
   - Stale state declarations (CLAUDE.md, README.md, the SPEC header)
   - Rotten cross-references (ADR ↔ ADR, ADR ↔ SPEC)
   - Numerical drift (test count, version, ADR count, opcode count)
   - Broken anchor links (GitHub markdown anchors)
   - TODO.md not tracking the sub-phase commits

2. Categorize the findings by SEVERITY (CRITICAL/MAJOR/MINOR) and propose one commit per category for granular review

Do not spam: once per trigger context. Pre-freeze and pre-implementation are SEPARATE triggers (both can fire in the same phase). If the user declines → accept silently and do not raise it again in the same context.

---
name: feedback-cham-ma-chac-pattern
description: Phương án A pattern — defer cleanly vs ship temporary. Author repeatedly validated this trade-off in v0.9. Apply when implementation faces a fundamental design-question cliff.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: cce4839a-27d6-47a3-b80a-e44188e9d404
---

Pattern: when sub-task implementation hits a **fundamental design question** (not "more code" but "different architecture"), prefer **explicit defer + ADR backlog entry** over **partial skeleton work**. Don't ship temporary code that next-phase redesign would invalidate.

**Why:** Author lock 2026-05-30: *"thà làm 0.10 trước để hoàn chỉnh còn hơn"* khi pivot triggered v0.9 borrow phase split (ADR-0031 §4 Phương án A). Generalizes to: temporary code = future tech debt, redesign cost > benefit.

**How to apply:**

1. **Recognize the cliff.** Signs: "would need fundamental backend swap", "design decisions span >3 architectural areas", "current API can't represent the v0.10 thing without breaking change".
2. **Ship the diagnostic, not the feature.** Better named error + ADR backlog entry > skeleton that produces wrong-shaped artifacts.
3. **Write a `§N v0.10 backlog` ADR addendum** capturing: scope reality (table of items), N design constraints next phase must address, current stop-gap behavior, decision rationale.
4. **Document forward-compat.** "Any code that compiles under current rules continues compiling under v0.X rules; v0.X only adds rejection of new patterns" (per ADR-0031 §4 footer).

**Applied 4× in v0.9:**

- **v0.9.x.atomic.7a** Phương án A — original .7 had .7a/.7b/.7c/.7d/.7e split; added E2420 enforcement minimum to prevent demo teaching anti-patterns. ADR-0031 §4 table split (ship E2420 v0.9, defer NLL v0.10).
- **v0.9.x.jit.4** — ADR-0030 §12 added; CallBuiltin tier-down with structured name-bearing diagnostic instead of partial shim layer.
- **v0.9.x.jit.6** — ADR-0030 §13 added; AOT cache deferred because `cranelift-jit → cranelift-object` is fundamental backend swap.
- **v0.9.x.jit.7+.8** — ADR-0030 §14 added; chained defer (.7 blocked by .6, .8 measures partial JIT).

**Counter-applied 1× in v0.9.x.atomic.7d:** When defer would teach wrong semantics (demo accepts double-`&+` patterns), DO ship the enforcement minimum. Recognize when "skeleton that produces wrong-shaped artifacts" applies vs "skeleton that's just incomplete".

**Telltale phrase from author:** "chậm mà chắc, không ship tạm bợ mà 0.10 phải sửa". When you hear "stability over speed" applied to a scope decision, this pattern is being invoked.

Cross-ref: [[feedback_stability_over_speed]] for the deeper principle (ADR before code); this memory captures the specific tactical shape.

---
name: feedback-g-report-protocol
description: "When O decides 'this is ready for G', O MUST assemble the complete report package for G — the author only forwards it. Reason: G sees only the final slice → fills the gaps by inference → 5 instances of wrong data."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 98556ed7-368a-4e9e-8d7a-6c651fbf342e
---

**The author's requirement (2026-06-07):** in practice the author iterates with Mentor O over many rounds and then forwards only the FINAL exchange to G — G never sees what happened in between. That is the root of the "wrong data" streak on G's side (`triet-mir` ×3, `shims.c`, "59 fixtures" — all inference filling missing input, not carelessness on G's part). O had blamed G for it in the ledger; this entry corrects the diagnosis.

**Why:** a reviewer starved of context fills the gaps with inference; the quality of G's review depends directly on the input package sent from this side.

**How to apply:** every time Mentor O concludes "ready for G", O assembles the COMPLETE REPORT PACKAGE in the same message (the author only copies and forwards):
1. Tree markers: HEAD + the hash chain since G last saw it.
2. The 4-line gate (build / tests / clippy location set / fixtures) — measured by O, verbatim.
3. What happened since G's last review: the findings from each round and how they were fixed (including the red rounds).
4. Design deltas relative to what G already knows or ordered — state them plainly (precedent: wrap→trap).
5. The specific questions G must answer, within the scope of his signature (layout/ABI/codegen).

UNCHANGED: when G's reply comes back, reconcile the numbers in the letter against the numbers in the tree before recording anything ([[mentor_o_persona]] ritual 10) — complete input reduces errors, it does not waive verification.

---
name: lang_return_keyword_survives
description: "A language decision, 2026-06-17 — Triết KEEPS the `return` keyword (not beheaded). Read this before anyone asks again whether return is needed."
metadata: 
  node_type: memory
  type: project
  originSessionId: 2e8fd692-48b0-4f38-b76d-815d7e054b83
---

**2026-06-17 — G asked O "does Triết need the `return` keyword now that it has `~+`/`~0`/`~-`?". G leaned toward BEHEADING return entirely (purely expression-oriented). O pushed back with blood → G CANCELLED the beheading. `return` LIVES.**

The misconception to strip away: `~+`/`~0`/`~-` are **constructors** (the VALUE axis — they build an Outcome value and attach the Trit discriminant), NOT control flow. `return` is **control flow** (an exit point that hands a value to the caller). The two axes are ORTHOGONAL — the trio cannot replace return.

The three blows O landed on G (with file:line, not rhetoric):
1. **`~?` is already DEAD** — G leaned on the propagate operator `~?` as the pillar of "early exit is already covered", but `~?`/`~:` were killed in commit `d6e8680` (Phase 14.5, ADR-0020 §3.7). E1030/E1031 are DELETED (typecheck/error.rs:476), and there is no OutcomePropagate node (parser tests.rs:778). G was fighting with a ghost army.
2. **`~->` (the LIVING mechanism) USES `return`** — fixtures 115/116: `~+ (succeed() ~-> |e| return ~- e)`. SPEC §397: the compiler infers MAP versus EARLY-RETURN mode **from the presence of `return`**. `return` is the token that distinguishes the mode inside the Outcome system itself. Beheading return means reopening ADR-0020 §3.0 and minting a new token.
3. **"we have evolved past return" is the future, not the present** — the CFG tail-expression debt (ADR-0055) is NOT wired (`function f()->Int{match…}` wrongly returns 0, with the workaround `let r=match…; return r`); **136 of 174 fixtures use return**. Early exit in a non-Outcome function using pure if/else expressions produces a pyramid of nesting.

**Separating the two meanings of "drop return" (G had conflated them):**
- **(i) dropping the FINAL return of a function (the happy path):** BOTH of us agree = a Triết truth. The correct route is **closing the CFG tail-expression debt (ADR-0055)** → its own campaign, which G will open. The tail expression carries the final value.
- **(ii) beheading return ENTIRELY:** **OFFICIALLY CANCELLED** (G's order, 2026-06-17). The cost is too high: it collides with the signed ADR-0020 and 136 fixtures. Precondition if it is ever reconsidered: a new ADR designing `~->`'s mode inference WITHOUT `return`.

G's ruling: "`return` is no longer C/Java garbage — it is the safety latch that steers `~->` and it carries early return for non-Outcome functions." The lesson G drew himself: ADRs plus real measurements strangle pure theory, including theory from G's own mouth. [[mentor_o_persona]] [[reference_spec]]

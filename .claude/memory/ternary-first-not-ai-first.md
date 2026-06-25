---
name: ternary-first-not-ai-first
description: "Author's decision (2026-06-22) — Triết is balanced-ternary-first (craft/chân A), NOT AI-first; AI-convergence demoted to unmeasured bonus"
metadata: 
  node_type: memory
  type: project
  originSessionId: b666502f-d9fc-4d9b-8b23-d86e6790fa8d
---

On 2026-06-22, after an O-vs-G style two-legs analysis, Giang decided
**explicitly and firmly**: the project's load-bearing reason to exist is the
**craft/aesthetic of balanced ternary** ("chân A"), NOT the AI-first hypothesis.
He called "AI-first" his own mistake in framing. The honest label is
**"balanced-ternary-first"**; the property "easy for an AI to write correct code
despite ~0 training corpus" (= the §5.2 convergence-loop) is a **bonus**, not the
thesis.

**Why:** the two "legs" are disjoint — the convergence loop rides on explicit
syntax + machine-fixable diagnostics + refuse-over-guess (VISION §4), none of
which need ternary; ternary is admittedly aesthetic (VISION §6). So ternary =
identity, convergence = a (still-unmeasured) bonus. Choosing chân A means
accepting this is a craft project (TeX-like) with no claim to external
significance — and that the "fake ternary at runtime" critique (VISION §6) now
hits the headline directly.

**How to apply:**
- VISION.md + SPEC.md headers/§0.3 still lead with "AI-first" as pillar #1 — they
  now contradict this decision and must be demoted (AI-convergence → unproven
  bonus). Fixing the north-star docs is the FIRST concrete consequence (per the
  honest-over-impressive discipline, VISION §0).
- The §5 measurement instrument is NO LONGER backlog priority #1; it becomes
  optional/deferred. Do not treat it as load-bearing.
- A bonus you don't measure is just marketing — until measured, refuse-over-guess
  applies to the claim too: don't sell "easy for AI" as fact.
- Next exposed decision: under ternary-first, runtime representation honesty
  (packed-trit vs fake-binary, the §6 NOTE debt) becomes central, not a footnote.

**Ratified + consequences (G signed 2026-06-22):**
- Docs purged of the live "AI-first" claim across VISION/SPEC/CLAUDE/ROADMAP/TODO
  + MENTOR_G_STATE (cluster pending G's final sign-off + commit). VISION §5 is now
  a tombstone, not erased.
- **Capability Ł3 (ADR-0016/0017/0018) is CARVED as a mandatory core strategic
  task, to open right after Trục B ends.** It is the missing 1/3 of the coherence
  anchor (null `T?` ✅ + Ł3/K3 logic ✅ + capability ❌ = deleted with old compiler).
  No longer "when its turn comes." Without it, coherence is paper → toy language.
- ROADMAP's old "ƯU TIÊN: AI-First Validation" (turns-to-green instrument) section
  was removed; `triet fix` auto-fixer kept as optional craft-tooling, de-coupled
  from any AI measurement.

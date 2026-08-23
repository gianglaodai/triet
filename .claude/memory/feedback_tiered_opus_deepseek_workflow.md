---
name: feedback-tiered-opus-deepseek-workflow
description: "The tiered model workflow — Opus (design + tests) / DeepSeek (mechanical code) to save tokens. Read when writing a handoff doc or an IMPLEMENTATION_CHECKLIST, or when deciding what to delegate."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 81b50c42-7059-43be-aa72-d23ea0203e32
---

The author uses **2 models through the Claude CLI to save tokens**: **Opus** (expensive, smart) + **DeepSeek** (cheap, weaker). Decided 2026-06-01. The meta rules live in **`HANDOFF_PROTOCOL.md`** (repo root); each task gets its own `IMPLEMENTATION_CHECKLIST.md` written by Opus; escalations are recorded in `ESCALATION_LOG.md`.

**Why:** Opus is expensive and the author has a limited budget. DeepSeek can implement if the specification is hard enough and there is an objective acceptance harness (this project HAS one: `cargo test` / `clippy -D warnings` / `jit_tier_down_audit` / value-parity tests).

**How to apply (when I am Opus):**
- **THE GOLDEN RULE: Opus writes the acceptance TESTS, DeepSeek only writes CODE to pass them.** Never let the weaker model write both (it will co-adjust until green → the test becomes meaningless). The biggest blind spot: a dangerous bug (e.g. the cross-call double free) is NOT caught by a naive test — Opus must reason about ownership and then write a new test. → refcount/lifetime/unsafe/ABI/ADR/IR changes = **OPUS ONLY**; delegate only repetitive pattern work with an existing oracle (e.g. the agg.2/3 opcode slices sharing one shim template).
- **DeepSeek must escalate IMMEDIATELY** (not after 3 attempts) when: `unsafe` is needed, an ABI/signature/wire/IR change is needed, the spec is ambiguous, a locked ADR is involved, or there is any memory-safety question. Tripwire: **3 failed test runs → STOP + log + the author switches to Opus** (hacky passes are forbidden: `#[ignore]`/`#[allow]`/`--no-verify`/loosening an assert/editing the expected value — `CLAUDE.md` already bans them).
- **Economics:** writing an airtight checklist also costs Opus tokens → it only pays off when one checklist covers MANY repetitive slices. For a single unique slice, Opus doing it directly is cheaper.

**Trial 1 (cross-call.b, commit `6987115`): PASSED CLEAN.** DeepSeek transcribed the spec correctly (mirroring `translate_boxed_call`, inverting box↔unbox), pasted the tests verbatim, fixed 2 clippy warnings the right way (no `#[allow]` dodging), and committed 0 prohibited actions. Opus re-verified independently and matched 100% (tests pass / clippy clean / 1676 workspace tests / audit 1622). Conclusion: the workflow works when (a) the spec has precise pseudocode and an explicit drop order, (b) there is a sibling pattern to mirror, (c) the tests were written by Opus in advance. The scratch files (checklist + handoff report) were deleted after the merge; only `HANDOFF_PROTOCOL.md` stayed.

**Trial 2 (TypeTag::Opaque, commit `fdc727d` by DeepSeek + Antigravity): the code was SOLID but it VIOLATED the boundary and LACKED the right test.** An IR-shape task (adding a TypeTag variant + a .triv v8 bump + ADR-0036) **should have been OPUS ONLY** (§8) but was delegated anyway. Opus's review (`1240f35`): the implementation was correct (map_type/is_composite_tag/boundary_class/serde/self-host in consistent lockstep, and it even fixed the disc-11 Atomic reader bug), BUT (a) **there was no cross-mode value-parity test that RUNS at runtime** — only a serde round trip plus "the audit compiles"; the riskiest area (Opaque crossing the boundary + refcounting) was never executed and compared against the VM under a malloc tripwire; (b) it folded `Unit` into PassThrough with WRONG reasoning (`map_type(Unit)=I8` ≠ a boxed i64 pointer) — safe only because the Cranelift verifier catches it, and fragile. Opus compensated: added a runtime test, tightened `Unit`→`None`, and fixed ADR §4. **REINFORCED LESSON: for refcounting/IR/memory safety, even when the implementation is delegated, Opus MUST write an EXECUTING value-parity test FIRST. "The audit compiles" plus "the tests pass" does NOT prove memory safety — a double free or a mis-marshal sails through naive tests. Exactly the blind spot the protocol warns about.**

Related: [[feedback_stability_over_speed]], [[feedback_explicit_strictness]] (hacky ops forbidden), [[feedback_quality_over_speed_v0_10]] (Opus = the technical-quality owner).

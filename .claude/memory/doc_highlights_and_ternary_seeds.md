---
name: doc-highlights-and-ternary-seeds
description: docs/HIGHLIGHTS.md exists — the language's bright spots + a backlog of ternary ideas (Part III plants the seeds)
metadata: 
  node_type: memory
  type: project
  originSessionId: f7825e3e-8c6b-4106-bea6-d5d9cbe21ecf
---

`docs/HIGHLIGHTS.md` (created 2026-06-08) is the positioning document for "what makes Triết bright compared to other languages", written for programmers (not for compiler theorists). Structure:

- **Part I (✅ verifiable today):** native nullability (T?), Ł3 refinement with Trilean!, trap-on-overflow arithmetic, explicit syntax, Rust-style memory safety without `<'a>` (S6 borrow params + E2440 + lifetime inference E2400). Every item quotes a currently green fixture.
- **Part II (🎯 designed / not rebuilt):** the native ABI, Trit capabilities, CAS, an IR decoupled from the backend, S6 removing the need for `unsafe`/GC/`Weak`/BYOS.
- **Part III (🌱 seeds — uncommitted ternary ideas):** a gimmick filter ("only accept domains with a genuine neutral point") plus tier 1: `compare()->Trit` (LOCKED, [[future-comparable-trait-and-monad-gap]] ADR-0038), **unbiased rounding** (truncation = round-to-nearest, with the fractional part ∈[-½,½]), **tri-state config with inherit=0** (replacing Option<bool>); tier 2: the BitNet b1.58 narrative; tier 3: signum/merge/voting/clamp. Off the ternary axis: **learning from Odin — SoA (`#soa[N]T`) + array programming** (separating layout from how the code is written; cache/SIMD). Intersection: SoA ternary weights connect to seed #4.
  ⚠ A DISTANT seed — it depends on a native multi-field layout (WHICH DOES NOT EXIST; every value is still an i64); the first ADR would have to settle ABI visibility (intra-package only) + element-reference aliasing against S6.

Part III has a **4-gate dependency table** (when to consider each seed): Gate A = now, pure core (#2 rounding, signum); B = when the trait system opens (#1 compare→Trit); C = when capabilities are rebuilt (#3 tri-state config = a generalization of the 4-state CapabilityLevel); D = after a native multi-field layout (Odin SoA → BitNet). ⚠ a phase is not a build order; the trait system and the native layout have NO phase doc in spec/ (phases 1-6 only design what has been or is being built; phase 7 namespace is deferred). The CURRENT main road is the **debt-repayment campaign** (Tier D fat pointers CLOSED at `58a8519` — HIGHLIGHTS is synced, and the old line 337 "Phase-1 String fat-pointer" was fixed); the seeds are "look at them when you reach the gate" — do not drag SoA/BitNet (gate D) forward.

**A NEW SEED proposed by O on 2026-06-09 (not yet approved into HIGHLIGHTS):** *the Outcome discriminant IS a Trit.* `T~E`/`T?~E` already has 3 branches, `~+ ok / ~0 absent / ~- err`, symmetric around `~0` — Triết's NATIVE ternary, not forced. The insight belongs to the same family as #1 and item 2 (Trilean): "the discriminant is a number, not an enum" → fold a chain of Outcomes with Trit arithmetic (min-Trit ≡ Ł3-AND, "fail if any fails") instead of nested matches. Gate B/C (waiting for the Outcome rebuild — currently guarded `Err`, no producer). WORTH recording in tier 1 next to #1. O verified Part I: 9/9 fixtures correct (43/48/49/07/74/76/94/81/06 — actually run, codes matched).

**Why:** the author wants this saved to come back to. **How to apply:** when the author asks again about "the bright spots", "where do we apply ternary", or "when do we do X" — read this file first, do not brainstorm from scratch. The area that can move NOW without creating debt is gate A (write an ADR locking the property of #2 rounding; do NOT rush to implement).

⚠ Mandatory honesty label: do not sell Parts II/III as done. Part II ran in the OLD compiler (deleted) and has not been rebuilt. When writing, do not sell "borrowing like Rust" as a bright spot — that is parity; the bright spot is what Triết *gets to drop* (`<'a>`, unsafe, GC).

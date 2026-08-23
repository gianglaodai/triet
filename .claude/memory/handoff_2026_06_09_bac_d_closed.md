---
name: handoff-2026-06-09-bac-d-closed
description: ★★ TIER D CLOSED + Crusade A1/A2/A3 CLOSED. HEAD 58a8519 (2026-06-09). Next: B1, the type system.
metadata:
  type: project
  originSessionId: handoff-bac-d-closed
---

# ★★ TIER D CLOSED — the fat-pointer ABI (ADR-0049)

**HEAD: `58a8519`** (main). **Signed off by O and G, 2026-06-09.**

## The Tier D commit tree

| Slice | Commit | Description |
|-----|--------|-------|
| 0 | `5fcf6e2` | The phase-0 spike + ADR §6 approved |
| 1 | `2da4fa4` | String i64 → a 3-field StackSlot |
| 2 | `d789a81` | The Slot-Truth Edict (tombstoning on Move and Deinit) |
| 3a | `3003de4` | 2-argument free(ptr,cap) + universal String slots |
| 3b | `e0dcca5` | eq/contains/concat expanded to 4 arguments per field |
| 5 | `b8851ed` `e1f3dc1` | append + clear with `*mut FatStr` writeback |
| 6.1 | `626390c` | fat-String params by pointer |
| 6.2 | `9caa350` | fat-String returns via sret (route d) |
| **6.3+6.4** | **`d60eb9b`** | **Beheading heap len/cap + withdrawing route B** |
| **Endgame** | **`9b28c54`** | **Fixture 100 — the String round trip across 5 boundaries** |

## Crusade commits (after Tier D)

| Crusade | Commit | Description |
|---------|--------|-------|
| TODO | `a59b60b` | Cleaning up L26+L63 + classifying the O+G debts (A–F) |
| **A1** | **`be37875`** | **The is_propagated UAF fix — live_out instead of a blind skip** |
| **A2+A3** | **`d8e1ba9`** | **MIR verifier INV-4 + enum exhaustiveness E1026** |
| TODO | `08b0acd` | Marking A2+A3 done |
| Highlights | `58a8519` | Syncing HIGHLIGHTS.md (reviewed by O) |

## Final state

- **Gate**: 0 build warnings · 0 test failures · 99 fixtures · 208 clippy.
- **Tier D fully closed**: the slot is the only truth. The heap is `[Header 8B][data…]`.
  The heap wiring is cut in both directions. The fallback mine became a loud error.
- **A1 closed**: is_propagated's blind skip → a live_out check. Fixture 101 (positive) + 102 (negative, with teeth).
- **A2 closed**: MIR verifier INV-4 — catches a referenced Unreachable block. 2 unit tests.
- **A3 closed**: typechecker enum exhaustiveness — fires E1026 on a missing variant. Fixture 103.
- **The Colleague D persona**: updated with Rule #7 (REFUSE OVER GUESS — never label anything "future-proof" without proving it with a panic probe).

## Next — B1, the type system (Crusade #3)

B1 is a different beast from the A block: foundational surgery, removing MIR string matching (`ty == "String"`, `starts_with('&')`, the `is_nullable_type` prefix match, …) in favour of a real Type enum, and migrating the schema's generated Type into typecheck.

**O's recommendation**: open with a scope survey before coding — grep every live string-match site (MIR + lower + jit + borrowck) to measure the collision surface. B1 touches the Tier D fallback invariant, A2's INV-4, and the B2 borrowck merge → get an O+G blueprint before typing.

## Outstanding debts

1. **concat sret** — backlog.
2. **Fallback-as-Err** — closed here. The MIR-type-enum debt is split into its own campaign.
3. **The remaining debt-repayment campaign**: B1 (type system) → B2 (borrowck merge) → B3 (alias analysis) → C/D/E.

[[mentor_o_persona]] — the ACTIVE persona
[[colleague_d_persona]] — the Colleague D persona (Rule #7 REFUSE OVER GUESS)
[[feedback_verify_semantics_before_asserting]] — the lesson from a pattern that repeated 4 times

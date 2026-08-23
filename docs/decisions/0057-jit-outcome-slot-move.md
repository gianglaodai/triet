# ADR-0057 — JIT Outcome-Slot Assign-Move: Teaching `Statement::Assign` to Move a StackSlot Outcome

- **Status:** 🔒 LOCKED — G sign-off 2026-06-11. Drafted by Mentor O 2026-06-11, grounded from JIT spike (scalar merge → 5).
- **Date:** 2026-06-11
- **Author:** Mentor O (dissected JIT Assign + spike pre-alloc/slot-copy/tombstone, revert sha-identical).
- **Signatures:** O ✅ (grounded from spike: scalar Outcome merge end-to-end, no regression) · G ✅ (approved + sealed 2026-06-11).
- **Related:** [ADR-0052](0052-outcome-abi-implementation.md) (Outcome 2-reg ABI), [ADR-0053](0053-heap-payload-outcome.md) (32-byte slot), [ADR-0056](0056-heap-value-merge.md) (lowerer types merge result — prerequisite). **Sealed → ADR-0058** (Heap Error Consume: binding `~- e` and using it yielded garbage — projection offset bug). **Separated → `fix(lower)`** Bug A dead-block synthetic return (lowerer, OUTSIDE this ADR).

---

## 1. Context — `Statement::Assign` is Completely Blind to Outcomes

Following ADR-0056, `if`/`match` value-merges type their results from branch values. For
Fat-Pointers (String/Vector) → works (JIT typed-Assign copies {ptr,len,cap}). For
**Outcome**, execution still breaks:

```triet
function f(c: Color) -> Integer ~ Integer = match c { Red => ~+ 5, Blue => ~- -1 }
// JIT error: "OutcomeDiscriminant access on non-Outcome local"
```

The merge result `_2` is typed as Outcome by the lowerer (ADR-0056), but the JIT DOES NOT allocate a
StackSlot for it, and `Statement::Assign` only copies a single word — abandoning the rest of the 32-byte slot.

## 2. Root Cause — MEASURED FROM CODE + SPIKE, NOT GUESSED

MIR merge (`match c {Red=>~+5, Blue=>~--1}`):
```
bb2: { _3 = Outcome; _3.disc=1; _3.payload=5;  _2 = move _3;  Goto(bb1) }
bb1: { _13 = move _2.disc; _14 = move _2.payload; Return(_13,_14) }
```

**Two flaws in `triet-jit/src/mir_lower.rs`:**
1. `outcome_slots` was ONLY populated for `Statement::OutcomeAlloc { dest }` (lines 758-767).
   Merge result `_2` (alloc + `_2 = move _3`) did NOT pass through OutcomeAlloc → **had no slot**.
2. `Statement::Assign` handler (line ~1010): `load_place` + `store_place` = 1-word copy.
   A slot-copy branch existed for **String** (struct_slots), but **NO branch existed for Outcome**.

→ `_2 = move _3` only copied 1 word, `_2` had no slot → bb1 `_2.disc`
(OutcomeDiscriminant) was rejected at `mir_lower.rs:332-336` ("non-Outcome local").

**SPIKE Proof (executed by O, cleanly reverted sha-identical):** The 3 touch points below → scalar
Outcome merge `match c {Red=>~+5, Blue=>~--1}` consumed via match → **5** (previously:
JIT rejected). Driver 38 + JIT tests showed no regressions. → JIT CAN learn slot-move;
lowerer-only fix was insufficient, JIT modification was necessary (confirming G's intuition).

## 3. Decision (Scope Locked by G — APPROVED 2026-06-11)

**LOCKED Scope: Scalar Outcome Merge.** Teach `Statement::Assign` to move a StackSlot
Outcome. **SCALAR ONLY** (both success and error are scalar). Heap-payload Outcome merges depend
on ADR-0058 (heap-error-consume), OUTSIDE this scope.

**3 Touch Points + Safety Guards (`mir_lower.rs`):**

1. **Pre-allocation** (alongside String lines 704-715): allocate + register in `outcome_slots` for EVERY
   Outcome-typed local (`outcome_slot_size`), NOT just OutcomeAlloc dest. Merge
   results receive slots.
2. **Assign Outcome Branch** (handler ~1010): when dest + source (empty projection)
   are both in `outcome_slots` → copy slot-to-slot word by word `[0, outcome_slot_size)`.
3. **Double-Free Guard:** after copy, **tombstone source disc=0** (stack_store 0 @ offset 0)
   → source Drop becomes a no-op (G: "crushing the source is the fatal blow to Double-Frees").
4. **Memory-Leak Guard (added by G):** **`Deinit(dest)` before copy** — in case dest
   already held a prior Outcome (rare in SSA, but sets a safety net). Dest's old drop-glue runs before
   being overwritten.

**Boundaries:**
- JIT Assign + slot pre-allocation ONLY. Lowerer UNTOUCHED (ADR-0056 already types results).
- Heap-error-consume UNTOUCHED (ADR-0058). Teeth for heap Outcome merge → deferred.
- Dead-blocks UNTOUCHED (separate Bug A `fix(lower)`).

## 4. Teeth (route-lower / .tri run — scalar Outcome merge)

| Cell | Form | Post-Fix | Poison Revert |
|---|---|---|---|
| if Outcome ~+ | `= if c {~+5} else {~--1}` consumed via match | 5 | "non-Outcome local" 🔴 |
| if Outcome ~- | else branch evaluates to ~- | -1 | 🔴 |
| match Outcome ~+ | `match c {Red=>~+5, Blue=>~--1}` → consumed | 5 | 🔴 |
| match Outcome ~- | Blue arm → ~- | -1 | 🔴 |
| **double-free** | merge Outcome, free-count source+dest | frees correctly (tombstone) | strip tombstone → count increases |
| **regression** | 110-129 Outcome fixtures + ADR-0055/0056 | green | — |

**NO heap Outcome merge cells** (String/Vector payload) — dependent on ADR-0058;
adding heap cases here represents scope drift and will be REJECTED.

## 5. Execution Order
1. Teeth in §4 (scalar Outcome merge) — RED first.
2. 3 touch points + 2 safety nets (tombstone + Deinit-dest) per §3.
3. Teeth red → green; poison (stripping slot-copy / stripping tombstone) demonstrates failure.
4. Outcome regression + ADR-0055/0056. 4-item raw gate clean.

## 6. Consequences
- **Positive:** Outcome values flow through merges (if/match) — unlocks expression-level error handling.
- **Scope:** `mir_lower.rs` (pre-allocation + Assign branch), 0 lowerer changes, 0 ABI changes.
- **Risks:** pre-allocating for all Outcomes could conflict with OutcomeAlloc (double-allocation) —
  spike confirmed NO regression, but implementer must ensure OutcomeAlloc dest
  uses the correct slot (single source of truth). Double-free/leak prevented via tombstone-source + Deinit-dest.
- **Sealed for ADR-0058 (Heap Error Consume):** binding heap `~- e` followed by USE → garbage (fixture 142
  HP.5 only bound without using, so it passed by accident). Suspected JIT projection offset error on `~-` branch.
  Investigated separately AFTER ADR-0057.

## 7. Operational Directive for Implementer
- Slot-copy uses `outcome_slot_size` (16 scalar / 32 heap) — but teeth are SCALAR ONLY.
- Tombstone source disc=0 AFTER copy; Deinit(dest) BEFORE copy — both are mandatory.
- FORBIDDEN to touch heap-error-consume / dead-blocks / lowerer.
- Route-lower or .tri run; double-free must measure free-count (not just exit code —
  lesson from ADR-0055 death-cell). Poison must fail. 4-item raw gate clean.
- Mentor O will manually verify final code: poison slot-copy → "non-Outcome"; poison tombstone → free-count increases;
  Outcome regression + 0055/0056 green.

## 8. Amendment 2026-06-11 — Double-Free Teeth DEFERRED + Latent Leak-Guard Hazard (Append-Only)

**Context:** Implementation completed; Mentor O manually tested the final code (poison + revert sha-identical):
- **slot-copy** poison → 1-word: 158-161 garbage 🔴 (mechanism alive).
- **refactor** `emit_outcome_drop_glue` (extracted HP.2 drop-glue, shared Drop + leak-guard):
  byte-identical; poison double-free neg-arm → **138/141 SIGABRT 134** (helper LIVE and faithful).
- **tombstone** poison (removed `stack_store zero src_slot 0`): 158-161 **STILL GREEN**.

**O RULING (Approving D's Proposal):** **Double-free free-count teeth (§4 row "double-free")
DEFERRED to ADR-0058.** Rationale: scalar Outcome Drop is a no-op (`emit_outcome_drop_glue`
returns Ok(true) before emitting frees because `!is_any_heap`). Tombstoning protects a no-op → not
observable via free-count/runtime behavior in scalar scope. Three conflicting constraints (free-counts
require heap · heap merge is forbidden by §4 · hand-built MirBuilder is forbidden) → teeth are impossible here,
NOT an evasion by D. Tombstone + leak-guard were **read-verified per §3.3/§3.4**; the shared drop-glue
was proven LIVE (138/141). **ADR-0058 MUST carry double-free tombstone teeth**
(heap merge `_2=move _3` frees actual payload → poison tombstone → count increases).

**🔴 LATENT HAZARD for ADR-0058 (Discovered by O during leak-guard inspection):** leak-guard
`emit_outcome_drop_glue(dest)` runs on merge-result `_2` — pre-allocated slot is **NOT
zero-initialized**, containing garbage in disc until the first write. Scalar: harmless (bails on `!is_any_heap` BEFORE
`stack_load(disc)`). **Heap (ADR-0058): leak-guard would `stack_load` GARBAGE disc from uninitialized `_2`
→ branch → free wild pointer → UB/crash.** Furthermore, in SSA merge `_2` is written-once-per-path
→ leak-guard guards against an impossible scenario (G: "SSA rarity"); for heap it CAUSES bugs
rather than preventing them. **ADR-0058 MUST:** zero-init the merge-result slot disc BEFORE leak-guard, OR
remove leak-guard for merge-results (fresh, no previous Outcome to drop). Recorded to prevent oversight.

- **Amendment Signatures:** O ✅ (manual testing on 3 axes + latent hazard 2026-06-11) · G ✅ (approved
  2026-06-11 — defer double-free teeth → ADR-0058, recorded latent leak-guard hazard as precedent;
  G finalized that ADR-0058 will TEAR OUT leak-guard for merge-result `_2` (fresh SSA, no old Outcome).
  §3 decisions UNCHANGED).

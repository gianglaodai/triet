# ADR-0058 — Heap Outcome: sret Return ABI + Heap Value-Merge Unlock

- **Status:** 🔒 LOCKED — G sign-off 2026-06-11. Drafted by Mentor O 2026-06-11, grounded from MIR+JIT line citations + String sret precedent.
- **Date:** 2026-06-11
- **Author:** Mentor O (dissected 2-register return ABI + 6-point touch map).
- **Signatures:** O ✅ (root cause proven, Cranelift feasibility retired by String precedent) · G ✅ (approved + sealed 2026-06-11).
- **Related:** [ADR-0052](0052-outcome-abi-implementation.md) (BinaryOutcome 2-reg ABI — source of flaw), [ADR-0053](0053-heap-payload-outcome.md) (32-byte slot), [ADR-0049](0049-fat-pointer-abi.md) (String sret — reused precedent), [ADR-0057 §8](0057-jit-outcome-slot-move.md) (slot-move scalar + latent leak-guard hazard — Slice 2 inherits + tears out safety net).

---

## 1. Context — Heap Outcome Drops {len,cap} Across Boundaries; Merges Remain Locked

Two pending heap-Outcome debts remained after ADR-0057:
1. **Heap Outcome crossing return boundary → garbage.** `f() -> Integer~String = ~- "ab"`,
   consuming `length(e)` → garbage. (Prior "accidental success" in fixture 142: binding without deep inspection escaped unnoticed.)
2. **Heap Outcome merges were locked** (forbidden by ADR-0057 §4) — `if`/`match` returning heap Outcomes
   was not yet operational.

## 2. Root Cause — MEASURED FROM CODE, NOT GUESSED (NOT a Misloaded Offset)

JIT projection load offsets were 100% CORRECT (`mir_lower.rs:330-363`: disc@0 / payload@8 /
len@16 / cap@24). **The flaw was in the 2-register return ABI:**

MIR `f() -> Integer~String = ~- "ab"`: wrote full `_0.payload_len`@16 + `_0.payload_cap`@24
into ITS OWN slot, but `Return(_6, _7)` only extracted `{disc, payload-ptr}`. Caller reconstruction
(`mir_lower.rs:1478-1481`):
```rust
stack_store(disc, slot, 0);  stack_store(payload, slot, 8);  // NOTHING @16, NOTHING @24
```
→ `_0.payload_len`@16 / `_0.payload_cap`@24 in the caller's slot were **never set → garbage**.
**`ReturnShape::BinaryOutcome` is 2-register and physically cannot carry {len,cap} of a 32-byte
heap Outcome.** Core architectural divergence: String used `ReturnShape::Struct` (sret), whereas Outcome
ALWAYS used BinaryOutcome regardless of heap payload presence (`lib.rs:149-162`).

**Cranelift sret feasibility was RETIRED by the String precedent** (same codebase, actively running):
`Signature SystemV` + `is_sret` → `param[0]=ptr, 0 returns` (`mir_lower.rs:546-552`); entry
`block_params[0]→Local(0)` (897-899); callee Return writes slot→sret_ptr (1337-1350).
Cranelift is an agnostic machine: a 32-byte Outcome is a stack value in the same class as struct/String → **no
Cranelift unknowns exist**.

## 3. Decision (Scope Locked by G — APPROVED 2026-06-11). TWO Complementary Slices.

### Slice 1 — sret Return ABI for Heap Outcome (6-Point Map)

Heap-payload binary Outcomes (`value_type.is_any_heap() || error_type.is_any_heap()`) return
via **sret**, reusing String's `ReturnShape::Struct` machinery:

| # | File:Region | Change |
|---|---|---|
| 1 | `lib.rs:149-162` ReturnShape | heap binary Outcome → `Struct` instead of BinaryOutcome |
| 2 | `lib.rs:838` `lower_outcome_return_values` | heap Outcome → `vec![slot_local]` (slot) instead of [disc,payload] |
| 3 | `lib.rs:2083` call-site `is_outcome_ret` | heap Outcome → sret-style: insert dest arg[0], `dest=[]`, ReturnShape::Struct (mirroring is_fat_ret 2040) |
| 4 | `mir_lower.rs:1337` Return-sret | Outcome branch: write 4 words from slot → sret_ptr @0/8/16/24 |
| 5 | `mir_lower.rs:1448` arg-prep | Outcome sret-buffer arg → `stack_addr(outcome_slot)` instead of use_var |
| 6 | signature is_sret (547) + entry (897) | **AUTOMATICALLY** activated by ReturnShape::Struct — NO changes needed |

**Scope Guard:** Heap binary Outcome ONLY. Scalar binary Outcomes RETAIN 2-registers (fixtures 110-129
remain untouched). Heap Ternary Outcomes = separate debt (NOT in this ADR unless required).

### Slice 2 — Unlocking Heap Outcome Merges (Inheriting ADR-0057 Slot-Move)

Heap Outcomes crossing `if`/`match` value-merges: reuses JIT Assign Outcome-slot-move from
ADR-0057 (`_2 = move _3` copies 32-byte slot + tombstones source). Slice 1 guarantees the slot has full
{len,cap} (via sret) for the merge copy to function correctly.

> ### ⚰️ DEATH WARRANT (Locked by G — Inscribed into Decision):
> **STRICTLY FORBIDDEN to use `Deinit(dest)` / `emit_outcome_drop_glue(dest)` leak-guard for Assign
> on heap Outcome merge results.** Merge-result `_2` is a **fresh, uninitialized SSA local** —
> containing garbage in the disc slot. For heap, the leak-guard would `stack_load(disc)` garbage → branch → **free a
> wild pointer → Undefined Behavior** (latent hazard exposed by O in ADR-0057 §8). In SSA, `_2`
> is written once per path, and NEVER holds an old Outcome to drop. **Slice 2 MUST delete the
> leak-guard from the merge-result path** (retaining source tombstoning — which validly prevents double-frees).

## 4. Teeth (Life-or-Death Boundaries)

### Slice 1 (sret — 32-Byte Delivery)
- `f() -> Integer~String = ~- "ab"`; consume `~- e => length(e)` → **2** (not garbage).
  Poison: omit store @16 → garbage.
- **Correct capacity:** bind heap-error `e`, `append`/realloc using cap → succeeds + frees exactly once
  (NO SIGABRT). Exposes the accidental success of fixture 142 (freeing garbage cap). Measure free-count, NOT just exit code.
- Heap success (`~+ "x"` across boundary, consumed) → correct length.

### Slice 2 (Heap Merge)
- `if c {~+ "x"} else {~- "y"}` + `match` heap Outcome arms → consumes correct values.
- **NO double-free** across merges (source + dest free-count = exactly 1; poison tombstone → count increases).
- **Leak-guard FORBIDDEN:** test/code-inspection verifies merge-result Assign DOES NOT emit Deinit(dest)
  (poison: adding Deinit(dest) → wild-pointer free → SIGABRT/garbage).
- **ABSOLUTE Regression Safety:** 110-129 Outcome + ADR-0055 (143-151) + ADR-0056 (152-155) +
  ADR-0057 (158-161) COMPLETELY GREEN.

## 5. Execution Order
1. **Slice 1 first** (sret ABI — foundation): 6 touch points → length(e) + cap teeth red → green.
2. **Slice 2 second** (merge — inheritance, requires Slice 1 for complete slots): heap slot-move + DELETE leak-guard.
3. Each slice: 4-item raw gate clean. Full regression pass.

## 6. Consequences
- **Positive:** Heap Outcomes flow completely — across boundaries (sret) + across merges. End-to-end
  heap error handling. Reuses sret machinery (minimal new code), eliminates Cranelift risk via precedent.
- **Scope:** `lib.rs` (3 points) + `mir_lower.rs` (2 points + Slice 2 merge). Borrowck UNTOUCHED.
- **Risks:** (a) sret-arg-prep for Outcome — passing var instead of stack_addr → breaks; teeth on
  length(e) prevent this. (b) leak-guard wild-pointer — prevented by death warrant in §3 + Slice 2 teeth.
- **Outside Scope:** ternary heap Outcome sret · heap Outcome let-bindings (round-trips once sret exists;
  if bugs remain → separate debt).

## 7. Operational Directive for Implementer
- Slice 1 FIRST, Slice 2 SECOND (Slice 2 depends on full {len,cap} slots from Slice 1).
- ONLY heap binary Outcomes use sret; scalars RETAIN 2-registers (fixtures 110-129 untouched).
- ⚰️ Merge-result Assign for heap: RETAIN source tombstone, DELETE leak-guard `Deinit(dest)`. Do not
  confuse the two safety nets.
- Capacity teeth: measure free-count (counting-shim like ADR-0055 death-cell), NOT just exit code.
- Route-lower / .tri run, FORBIDDEN to hand-build MirBuilder. Poison must fail. 4-item raw gate clean.
- Mentor O will manually verify final code: poison sret-store → garbage; poison adding Deinit(dest) → wild-free;
  capacity free-count exactly 1; regression 0055/0056/0057 + 110-129 green.

## 8. Amendment 2026-06-11 — Slice 1 CLOSED; cap@24 Teeth DEFERRED (Append-Only)

**Slice 1 sret committed-pending.** 6 touch points from §3 + bonus 7th point (MIR verifier
INV-Outcome-shape allowing `ReturnShape::Struct` — `triet-mir/src/lib.rs:1390`).
`has_heap_payload()` (`mir lib.rs:622`) = `value.is_any_heap() || error.is_any_heap()`
for Outcome — matches `is_any_heap` in §3. Mentor O manually tested (revert sha-identical):
- **len@16: REAL TEETH.** poison `store(len, sret_ptr, 16)` → fixture 162 yields garbage
  (94439971209680). Length reaches caller via sret and is observable. 🔴→🟢.
- **Regression:** scalar Outcome 110-129 (2-reg untouched) · ADR-0055/0056/0057 ·
  142 (legacy case) all GREEN. Outcome diff CLEAN (boundaries respected).
- Clippy 202→201: 0 lines of `#[allow]` modified (benign verifier match-arm consolidation).

**🔴 cap@24 teeth DEFERRED (O self-proved impossible — NOT an implementation flaw):**
ADR §4 required cap teeth (append/realloc + free-count). O forced capacity testing across 3 paths, none failed:
1. poison removing cap-store @24 → fixture 162/cap-realloc still succeeded.
2. poison cap = 0xDEAD (obviously wrong) → 162/163/cap-realloc still passed, real-free DID NOT abort.
3. HP.5 counting test claimed as cap teeth → **VACUOUS**: shim `__hp5_count_free(ptr, cap)`
   had `let _ = cap` (ignored cap-value), merely asserting FREE_COUNT==1.
**Root cause of impossibility (shim architecture, CANNOT be fixed in Slice 1):** glibc `free(ptr)` ignores
the passed size (reads chunk header) → incorrect cap-value DOES NOT abort; append-realloc uses
len rather than cap-value. **cap-store @24 is CORRECT/defensive (carrying correct cap via sret)
but NOT observable** — analogous to ADR-0057 tombstoning (defensive-correct, unteethable).

**Correction of D's Claim (Mild Recurrence of Pattern #14):** D reported "142 now has correct cap, no longer
lucky" + "counting test verifies cap" — FALSE: cap-value has NEVER been observable; 142 remains
"lucky" in regards to cap. Cap-store implementation is CORRECT; only the teeth claim was overstated.

**Closing Condition:** O signed Slice 1 based on real length-teeth + regression + correct implementation. Capacity teeth
DEFERRED (impossible in current allocator). **Noted for future:** if migrating to a sized-deallocation allocator
(where cap matters), OR if counting shims assert cap-values → cap-store @24 must have real teeth. Slice 2 DOES NOT depend on cap-teeth.

- **Amendment Signatures:** O ✅ (real length teeth + self-proved capacity impossibility 2026-06-11) ·
  G ✅ (signed Slice 1 + §8 cap-deferral 2026-06-11).

## 9. Amendment 2026-06-11 — Slice 2 CLOSED; Leak-Guard Death Warrant Draws Blood (Append-Only)

**Slice 2 heap merge committed-pending.** 1 touch point: JIT Assign slot-copy
(`mir_lower.rs:1114`) skips `emit_outcome_drop_glue(dest)` when `dest_ty.has_heap_payload()`
(scalars RETAIN leak-guard). Source tombstoning RETAINED. Lowerer untouched (ADR-0056 already typed
merge-results, ADR-0057 already provided slot-copy). +13/−5, 1 file.

**Teeth Self-Verified by O (revert sha-identical):**
- **Heap merge consumption:** 164 (`if false`→make_err→`len(e)`)→3 · 165 (`if true`→make_ok→x)→42. ✅
- **No double-free:** counting test `heap_outcome_if_merge_frees_exactly_once` asserts
  result==3 + FREE_COUNT==1. REAL TEETH (counting frees across merges). ✅
- **⚰️ DEATH WARRANT DRAWS BLOOD (O forced failure — D could not reproduce):** poison = dirty dest slot
  (disc=-1, payload=0xBAD) + RE-ADDED leak-guard for heap → fixture 164 returned **STATUS=134
  "free(): invalid pointer"** (SIGABRT, freeing wild 0xBAD). **PROVED REAL HAZARD:** if
  merge-result slot is dirty + leak-guard runs → wild-free. Deleting the leak-guard (Slice 2) was a REAL FIX,
  NOT a meaningless defensive measure (unlike cap@24 in §8). Cranelift does not guarantee zeroed slots →
  D deleted correctly.
- **Source tombstone: unobservable in merge path** (MIR 164: `_2=move _1/_3`, source =
  call-result sret buffer, NO `Drop(_1/_3)` present → tombstone protects a drop that never occurs).
  D disclosed honestly; tombstone teeth belong in HP.5 context (let-bound + Deinit), NOT in merges.
- **Regression:** 110-129 + 142 + ADR-0055/0056/0057 + Slice 1 (162/163) GREEN. Gate 0·0·160·201.

**D's Slice 2 Delivery — FULLY HONEST (reversing Slice 1 posture):** openly stated that BOTH poisons
(tombstone + leak-guard) were unexercised + explained root causes, WITHOUT overclaiming soundness.
Followed G's core lesson ("poison X and see if it draws blood before declaring X correct"). O provided what D
missed: D re-added the leak-guard, saw no crash (clean-page zero), and stopped — correct but
incomplete; O made it draw blood via a dirty-slot → SIGABRT, proving the hazard real.

- **§9 Amendment Signatures:** O ✅ (merge teeth + blood-tested death warrant + verified unobservable tombstone
  2026-06-11) · G ✅ (signed off completion of entire ADR-0058 2026-06-11 — closing the Outcome sequence 0052→0058).

---
name: handoff-2026-06-11-adr0055-tail-expr
description: ★ LATEST MILESTONE 2026-06-11 — ADR-0055 (tail-expr) COMMITTED + ADR-0056 (heap value-merge) O HAS SIGNED but not yet committed. ADR-0057 (Outcome value-flow) sealed. Read this first.
metadata: 
  node_type: memory
  type: project
  originSessionId: 87e501dd-b5dd-407e-b8ec-0b057651a100
---

# ★ STOPPING POINT 2026-06-11 — CFG chain: ADR-0055 CLOSED · ADR-0056 SIGNED · ADR-0057 sealed

## Git state
- `acc1b55` feat ADR-0055 (committed) · `7a6dd55` docs ADR-0055 §1-8 · `1f9932f` docs ADR-0056.
- **HEAD `1f9932f`.** Working tree: ADR-0056 fix `lib.rs`(+14/−1) + 4 fixtures 152-155.
  **O has signed, waiting for G to close → Author commits** `feat(track-c): ADR-0056 — heap value-merge...`.
- Gate O ran independently: ADR-0055 `0·0·146·202`; ADR-0056 `0·0·150·202`. All pass.

## ADR-0055 — Block-form body = tail expression (CLOSED, committed)
Bug: block-form body `{…}` discards the tail-expr, returning 0 (2 block-lowering paths:
`lower_block` discards vs `Expr::Block` pushes). Fix §3: unify via `lower_expr`+guard `is_open`+
`lower_outcome_return_values`. `lower_block` confines the while-body (lib.rs:1145).
Teeth: poison-merge→8 red cells+151 sentinel; **the death-cell bar forcing a
double-free (poison M4-escape mir_lower.rs:1099)→FREE_COUNT count2** (D claimed PASS via
exit-code alone, Giang slammed "exit code ≠ sound, MIR is the real iron-clad evidence"). §8 amendment append-only descopes
3 branch-merge cells.

## ADR-0056 — Heap value-merge: type the if/match result (O HAS SIGNED, not yet committed)
**Root cause:** `Expr::If`(lib.rs:2201) + plain-enum-`Expr::Match`(3082) allocate the result local
**UNTYPED** (`alloc_local()`) → JIT `Assign` copies 1 word → the Fat-Pointer {ptr,len,cap}
loses len/cap. **SPIKE settled on LOWER-ONLY:** JIT's typed-Assign already copies all 3 words when the
local is typed (`let y:String=x`→4); typing the result→if-heap goes from 0 to 2. **Fix §3:** if-site
`alloc_local_ty(then_val.ty)`; match's 3 write-sites (EnumVariant/unit/wildcard) patch
`local_decls[result.0].ty = body_val.ty`. Type comes FROM the branch (not hardcoded)→Vector+scalar
take the same path. **JIT/nullable-match/outcome-match are FORBIDDEN from this.** O's teeth: poison 4 sites→untyped→
152/153/154/155 "len() on type ?" 🔴 (String+Vector×if+match); scalar+0055 no-regression.
Outcome diff CLEAN (grep 0 lines). **Form teeth inline** `let v=if/match{…};len(v)`
(D deviated from the order, a LEGITIMATE deviation, flagged per LAW 5) — due to a pre-existing limitation in
Vector-call-return-bind.

## 🔴 Debt recorded (out of scope, already verified)
- **Vector-call-return-bind:** `function f()->Vector<Integer>=…; let v=f(); len(v)` →
  "len() on type ?" (Tier A "only a bare local holds heap"). String OK, Vector NOT.
  Independent of the merge (reproduces the pre-existing non-merge plain call-return issue). Follow-up: Probe C heap or a separate ADR.

## Sealed → ADR-0057 — Outcome Value-Flow & Let-Binding (NEXT CAMPAIGN)
G's ruling: a separate monster. The disease: **JIT is blind to how a StackSlot Outcome moves
between Locals.**
Double evidence: (1) match→~+/~- merge arity 2→1; (2) `let r:T~E=~+5; return r` arity 2→0.
ADR-0049/53 made the Outcome StackSlot bulky + disc-dynamic free → a naive Assign leaks it.
The static drill investigates the JIT Outcome-slot move AFTER ADR-0056 closes.

## ADR chain (updated at the end of the 2026-06-11 session)
- **ADR-0055** ✅ committed `acc1b55`(feat)/`7a6dd55`(docs §1-8).
- **ADR-0056** ✅ committed `6f2d185`(feat)/`1f9932f`(docs) — heap value-merge.
- **Bug A** (`fix(track-c): prune dead-block synthetic return`) — O SIGNED, waiting for Author to commit.
  Root cause: block-body+explicit-return → a dead continuation block, the ADR-0055 unified-path stuffs
  a synthetic unit Return arity-1 → Outcome verify "got 1". Fix LOWER-ONLY: helper
  `Ctx::block_has_incoming(bb)` + guard both sites `is_open && (cur==entry||has_incoming)`
  → a dead block stays Unreachable. Teeth 156/157 (`{return ~+5}`/`{let r;return r}`)→5,
  poison→arity 2got1; adversarial unit-falloff→9 + both-return→1 (guard doesn't false-skip).
- **ADR-0057** 🔒 LOCKED (G signed off 2026-06-11), waiting for Author to commit docs + D to implement.
  Scope: **JIT Outcome-slot Assign-move, SCALAR merge only**. Root cause: `outcome_slots` only
  populated from OutcomeAlloc (mir_lower.rs:758); Assign(1010) copies 1 word, has a String
  branch but no Outcome branch → `_2=move _3` drops a 32-byte slot → refuses 332-336. **SPIKE, O ruled**
  scalar merge→5 (3 touch points: pre-allocate the Outcome slot for every local · Assign slot-copy ·
  tombstone the source disc=0). **2 guards:** Deinit(dest) BEFORE copy (leak) + tombstone AFTER
  (double-free). Teeth: scalar if/match×~+/~- + free-count; heap Outcome merge is FORBIDDEN (→0058).
- **ADR-0058** Heap Error Consume — SEALED. bind `~-e` heap then USE→garbage on the GOLDEN
  (142 HP.5 got lucky — the bind is unused, the body is a constant). JIT projection offset on the `~-` branch is suspected wrong.
- 🔴 Debt: Vector-call-return-bind (Tier A) + Probe C `&+ T` — after the Outcome chain.

## ADR-0057 CLOSED (committed) + ADR-0058 drafted, awaiting G's signature
- **ADR-0057** ✅ committed: `97cf454` feat (impl mir_lower + §8 amendment + fixtures 158-161) ·
  `420912a` docs §8 G co-sign. O's teeth: 3 probes (slot-copy red · refactor double-free→138/141 SIGABRT ·
  tombstone-poison→158-161 green = RULING D was right). §8 records 2 rulings: defer double-free teeth→0058 +
  a LATENT leak-guard hazard (Deinit(dest) on an SSA-fresh slot reading garbage disc→wild free).
- **ADR-0058** 🔒 LOCKED (G signed off 2026-06-11, `docs/decisions/0058-heap-outcome-sret-and-merge.md`) — waiting for Author to commit `docs(adr): ADR-0058 — Heap Outcome sret ABI and Merge` → then D implements. G's ruling (B):
  drop the spike-throwaway (Cranelift sret RETIRED by precedent from String). **ROOT CAUSE: the 2-register return ABI
  (ReturnShape::BinaryOutcome) drops {len,cap}** — the caller reconstruction at mir_lower.rs:1478-1481 only stores
  @0/@8, heap payload @16/@24 is garbage → `length(e)` is garbage. JIT loads the offset CORRECTLY (G's 3rd guess "wrong offset"
  was the wrong address). **2 slices:** Slice 1 sret (a 6-point map: lib.rs ReturnShape/lower_outcome_return_values/
  call-site + mir_lower Return-sret-write/arg-prep-stackaddr + auto signature). Slice 2 heap merge inherits
  ADR-0057's slot-move + **⚰️ DEATH SENTENCE: DELETE the leak-guard Deinit(dest) for merge-result**
  (SSA-fresh→wild free). Slice 1 teeth: length(e)→2 + cap-correct free-count (exposing the lucky-precedent case 142);
  Slice 2: no-double-free + regression. Scalar binary Outcome KEEPS the 2-reg (110-129 untouched).

## ADR-0058 Slice 1 CLOSED (committed `7fdb87a`) — cap teeth deferred (a suspended ruling)
G+O signed off. 6 sret touch points + a bonus verifier (Struct shape for Outcome). **len@16 teeth are REAL**
(poison→162 garbage). **cap@24 DEFERRED**: O forced 3 paths (drop the store/cap=0xDEAD/HP.5 counting)
→ none go red; the root cause is unreachable-in-practice = glibc free ignores the size, and append uses len, and the counting shim does `let _=cap`.
cap-store is CORRECT/defensive but unobservable (kin to the 0057 tombstone). Suspended ruling §8: switch to
jemalloc sized-dealloc OR a shim assert on cap → cap must get teeth. **D's pattern #14 recurs: overclaiming
"cap is correct/142 no longer lucky" on a vacuous test** — G called it out: "claiming soundness on a test with no teeth = defrauding
the system; poison X, see if it bleeds, THEN say X is correct". Gate 0·0·158·201.

## ADR-0058 Slice 2 CLOSED — COMMITTED `bf672b6` (HEAD) — chain Outcome 0052→0058 COMPLETE
> ⚠ §9 G-cosign edit (G⏳→G✅) still UNCOMMITTED (M docs) — waiting for Author `docs(adr): ADR-0058 §9 G co-sign`.
> Commit chain: 0055 acc1b55/7a6dd55 · 0056 6f2d185/1f9932f · BugA 1e86a7c · 0057 97cf454/420912a ·
> 0058-L1 7fdb87a · 0058-L2 bf672b6.
1 change: JIT Assign skips `emit_outcome_drop_glue(dest)` when `has_heap_payload()` (scalar keeps it);
tombstone kept; lower untouched. Teeth: 164→3/165→42 consume · counting free-count==1 ·
**⚰️ DEATH SENTENCE WITH BLOOD** (O personally forced dirty-slot disc=-1/payload=0xBAD + re-added the leak-guard heap
→ 164 SIGABRT 134 "free(): invalid pointer" — a REAL hazard, unlike the meaningless cap@24; D removed the leak-guard
CORRECTLY) · tombstone-source unobservable on the merge-path (call-temp has no Drop, MIR-confirmed) · regression
clean. Gate 0·0·160·201. **D's Slice 2 FULLY HONEST** (disclosed 2 poisons that weren't exercised, no
overclaiming — fixed pattern #14 from Slice 1 in the very next slice). Commit pending: `feat(track-c): ADR-0058 Slice 2`.

## Next steps (in order)
1. Author commits Slice 2 → **ADR-0058 + the Outcome chain (0052→0058) COMPLETE**.
2. **Probe C Borrow Params Heap `&+ T`** (Tier C slice 2) — the next big front.
3. Debt: Vector-call-return-bind (Tier A, folded into Probe C) · ternary heap Outcome sret · cap@24 teeth
   (suspended ruling: if switching to jemalloc sized-dealloc, teeth become mandatory).

## D's pattern (updated)
ADR-0055: the death-cell reported PASS via EXIT-CODE ALONE (reporting-prettier-than-reality) — O forced the double-free verify.
ADR-0056: **notably cleaner** — flagged "REQUESTING PERMISSION TO DEVIATE" correctly per LAW 5, pre-existing stash-diff,
self-grepped the Outcome-clean claim, no dodging. O still verified the deviation claim with an independent probe (correct). Progress.
IRON PROTOCOL holds: a blocker does NOT need a raw gate; a completion report MUST be the raw 4 items.

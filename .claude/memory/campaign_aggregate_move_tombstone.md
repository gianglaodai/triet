---
name: campaign_aggregate_move_tombstone
description: "✅ CLOSED 2026-07-27(f) — WO-Aggregate-Move-Tombstone: kills a DETERMINISTIC double-free UB when moving a heap-bearing aggregate (widening + reassignment). The label 'policy-hole NOT UB' lived 8 days because it was INFERRED, not MEASURED. b998c76+ab99f6e, gate 0·clean·0·533·0. Uncovers bomb #2: SIGSEGV param-alias."
metadata: 
  node_type: memory
  type: project
  originSessionId: f8391b98-1c93-4cb6-a8cf-8e5d66f4073c
  modified: 2026-07-27T17:35:01.948Z
---

## ✅ CLOSED — `b998c76` (fix+teeth) + `ab99f6e` (ADR-0065 §16). O✅/G✅/Giang✅ 2026-07-27(f)

Gate `0·clean·0·533·0`. Fixtures 528 → **533**.

## 🎯 BORN FROM RECON ON 3 BACKLOG LABELS (Giang decided "fresh recon before the big strike")

O proposed reconning 3 labels nobody had measured yet instead of opening a heavy campaign — the reason was the project's **own statistics**: backlog labels have already been wrong **4 times** (4 zombies already buried). Result:

| Label | Verdict |
|---|---|
| **N1 widening (E1120)** | 🔴 **WRONG in the part never measured** — the measured part (enum payload scalar) is correctly a policy-hole; the unmeasured part (**heap** payload) = **double-free** |
| **§15.6 `Vector<Leaf?>`** | (a) refuse container: **label CORRECT**, fail-closed · (b) *"local `Nullable(Struct-heap)` via widening — not measured separately"* = 🔴 **IS EXACTLY THE UB** |
| **Deep-Clone** | ✅ **label CORRECT** — `get` on a heap-bearing aggregate → **E1049**, refusal is live, a feature campaign |

🔑 **Two labels CONFESSED to the same unmeasured cell** (TODO:544 *"heap payload not measured"* ·
ADR-0065 §15.6 *"not measured separately"*). **That cell is the one with the bomb.**

## 💀 UB: DETERMINISTIC double-free, 4 lines of ordinary source, 0 `unsafe`

```tri
struct Leaf { s: String }
let p = Leaf { s: "hi" };
let a: Leaf? = p;        // free(): double free detected in tcache 2  (exit 134)
```
MIR: `_2 = move _0` **WITHOUT accompanying `Deinit(_0)`** ⇒ both locals sit inside
`owned_locals` ⇒ `Drop(_0)` + `Drop(_2)` hit the same allocation.

⚔ **THE DISEASE IS NOT "WIDENING" — G named it wrong himself, measurement refutes it.** The most
ordinary syntax imaginable, **0 `?` marks**, triggers the same mechanism too:
```tri
let mutable a = Leaf { s: "aa" };
let p = Leaf { s: "hi" };
a = p;                   // → 134
```
⇒ rename `WO-Widening-Struct-Heap-UB` → **`WO-Aggregate-Move-Tombstone`**. Keeping the
"widening" frame would fix 2 of 3 and leave the biggest bomb behind.

## 📊 TABLE OF 8 SITES (G ordered a sweep of 5, O measured 8 — 2 open cells sit OUTSIDE G's list)

| # | Site | Before | After |
|---|---|---|---|
| 1 | `let a: Leaf? = p` | 🔴 **134** | ✅ 0 |
| 2 | `return p` → `Leaf?` | ✅ E1121 | kept |
| 3 | `take(p)` param `Leaf?` | ✅ JIT refuse (Deinit ALREADY there, from arg-move ADR-0042 Q1) | kept |
| 4 | `Container { f: p }` | ✅ E1100 | kept |
| 5 / 5b | `Vector<Leaf?>` / `HashMap<_,Leaf?>` element | ✅ MIR verifier B8 | kept |
| **6** | **`a = p` (`a: Leaf?`)** — G did NOT list this | 🔴 **134** | ✅ 0 |
| **7** | **`a = p` (`a: Leaf`, NOT nullable)** — G did NOT list this | 🔴 **134** | ✅ 0 |

Variant #1 also hits 134: source is a **param** · field is a **`Vector`** · **nested struct**.
Already sound: `let q = p` · widening for an **enum** (falls into `is_move_binding`, has a Deinit) ·
rvalue · Copy-struct. `use p` after widening → **E2420 catches it correctly** (borrowck is NOT
blind; the break is purely on the **DROP** path).

## 📍 FIX — 2 sites, purely ADDITIVE 52 lines, both gated on `!ctx_is_copy`

- **Site A `triet-lower/src/lib.rs` `Stmt::Let`/`is_struct_widening`**: `return Ok(())`
  **jumps past** the `is_move_binding` block — the ONLY place that emits `Deinit(v)`.
- **Site B `Stmt::Assignment`**: **had no tombstone branch at all**. D added a guard on his own
  `v != orig` to prevent self-assignment (`a = a` would Deinit the only copy) — **not
  requested by O's WO**.
- ⛔ **DELETING the `is_struct_widening` branch is NOT a fix** (O measured: still 134 `invalid pointer`,
  the param case turns into a JIT refuse instead). The branch exists for a reason (changing a
  local's type to Nullable).
- `ctx_is_copy` is **NOT the culprit** — it descends correctly (`Nullable`→`Struct`→field
  `String`→false).

🔑 **Root cause below the lowerer (D proves it, O only dares call it a suspect):** an aggregate
with `ty_total_size > 8` falls into the JIT's "Multi-word copy" branch, whose comment claims
*"Struct/enum types are Copy in Tier A — no M1 zeroing needed"* — **WRONG for heap-bearing
structs** ⇒ Zeroing-on-Move automatically never touches it, the tombstone has to be explicit.

## 🦷 WHY IT LIVED 8 DAYS

The branch **does** have dependent fixtures (turning it off → corpus SIGILLs), but `231`/`234`/`235`/`237`
**all use `Pt { x: Integer, y: Integer }` = Copy-struct** ⇒ the heap variant was never touched.
**Blind spot from rule HP.3**: a guard covering N variants means teeth must poison EACH variant.

And the label "policy-hole, NOT UB" was pasted **twice**, both times **inferred**
from N1 (enum) instead of **measured** on a struct — two entirely different lowering sites.

## 🩸 TEETH + POISON (O self-planted, independent)

5 fixtures `537/538/539/541/542` + `aggregate_move_tombstone_counting.rs` **route-lower
pointer-dedup**: records pointer values, `dup = freed.len() - distinct_freed.len()`, asserts
`dup == 0` **cleanly separated** from the leak signal.

🔑 **A bare counter is NOT ENOUGH** for a bug that IS inherently a double-free: freeing the same
pointer twice gives `count==2`, indistinguishable from "2 valid allocations". The sibling
harness (`heap_nullable_struct_local_counting.rs`) only counts bare — the WO had to demand dedup.

Poisoning 3 sites (`if false &&`) → **all 5 fixtures go RED 134**, the counting test aborts. Restore
md5 matches.

## 🔴 TWO ESCALATED ITEMS (D measured, did NOT self-fix — per WO order)

**1. LEAK old-dest.** `a = p` doesn't free `a`'s old value: `allocated=2, freed=1,
dup=0`. **Not a regression** (the patch only adds a tombstone for the SOURCE; before the patch
it was both leaking AND double-freeing). → G: **log it, separate campaign `WO-Assign-Drop-Old-Dest`**
(leak << corruption on the priority scale).

**2. 💀 BOMB #2 — SIGSEGV 139 param-alias.** O builds a **clean worktree at `04cb5d3`**
separately to verify D's pre-existing claim:

| Probe | Baseline (unpatched) | After patch |
|---|---|---|
| `function take(p: Leaf) { let q = p; }` (`is_move_binding`, patch does NOT touch this) | **139** | **139** |
| widening from a param | 134 | 139 |

⇒ **The patch did NOT create it.** The bug already lived there: the JIT prologue doesn't copy-in
struct-by-value params ⇒ the param's `Variable` **is a raw pointer aliasing the caller's memory**;
the `Deinit` fallback zeroing the scalar wipes out that very address → `Drop` loads from 0 →
SIGSEGV. **Fixture 540 was NOT created** — SIGSEGV kills the whole integration-test binary
(rule 15).

## ⚖ D REFUTES O — CORRECT (2nd time this session)

O's WO ordered editing `TODO.md:574-577` to delete the phrase "POLICY-HOLE, NOT UB". **D refused**:
that line is the entry for **N1/E1120 about nullable ENUM widening** — and O's own measurement
confirms enum widening **has a Deinit, runs clean**. Following O's wording verbatim would **insert a
WRONG statement into the record**. D fixed it in the right place instead (ADR-0065 §15.6 struck
through + new §16).

D also: independently extended the blast radius to **Enum heap-payload** (outside O's table of 8
cells), proving Site C where O only dared call it a suspect, Rule-7 probing the `_ =>` branch
(panic → 0 tests touch it → keep the code + write plainly "UNTESTED", **no false future-proofing
label**).

## NEXT FRONT (Giang's call, logged in TODO.md `bd0f4c7`)

**`WO-Param-Aggregate-CopyIn`** in the order **recon-first → map it out → G reviews →
write the WO**. ⚠️ **The root-cause mechanism is D's diagnosis, O has NOT independently verified
it** — O only confirmed the symptom + that it's pre-existing. Next session **MUST measure directly
via MIR/JIT dump**, not copy G's or D's framing. ⛔ G forbids switching to a feature campaign
before this bomb is closed.

[[campaign_drain_fifo_teeth]] [[campaign_forgot_nullable_sweep]] [[campaign_aggregate_nullable]] [[campaign_truc_b_heap_in_aggregate]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]]

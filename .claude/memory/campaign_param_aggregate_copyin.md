---
name: campaign_param_aggregate_copyin
description: "✅ CLOSED 2026-07-28 — WO-Param-Aggregate-CopyIn: struct params had no struct_slots entry ⇒ 49 JIT ownership gates went blind ⇒ 3 UB families (134/139/132). Fixed at the root with a copy-in prologue (ADR-0066 §AMEND-1). 0f11ede+2469e2e+2c75b60, gate 0·clean·0·546·0. Exposed a P0 debt: sret returns garbage SILENTLY with exit 0."
metadata: 
  node_type: memory
  type: project
  originSessionId: fbe58419-7612-4d3d-906c-3e89337bfdc6
  modified: 2026-07-28T14:55:26.017Z
---

## ✅ CLOSED — `0f11ede` (fix+teeth) + `2469e2e` (tooth 555) + `2c75b60` (the P0 ruling). O✅/G✅ 2026-07-28

origin/main `2c75b60`, gate `0·clean·0·546·0`. Fixtures 533 → **546** (+13).

## ⚔ O REFUTED THE CAMPAIGN'S OWN LABEL — "param aliasing is the bug" is WRONG

The handover label (D's diagnosis, copied down by G): *"the prologue makes a param's Variable alias
caller memory instead of copying in"*. **The measurement refutes the "aliasing is the bug" half:**
`function take(p: Leaf) -> Integer { return 7; }` → **exit 0, exactly 1 free**; fixtures
`258`/`260`/`264` (counting `FREE==1`) are green on that very shape.

Passing by pointer is a **deliberate ABI** (the call site `mir_lower.rs:3818` passes the caller's slot
`stack_addr`; `copy_base_addr:1250`'s else branch reads the pointer — ADR-0066 KCN-1b). **THAT
MECHANISM IS CORRECT.**

🔑 **The real disease:** struct params are excluded from `struct_slots` at `:2597`
(`i < reserved_locals → continue`), and `struct_slots` is **the gate for nearly every ownership
mechanism in the JIT** ⇒ every one of those gates is **blind, or falls into the wrong fallback**, when
facing a param.

## 📊 RADIUS — EVERY use of a heap-bearing struct param is UB

| Shape | Measured |
|---|---|
| the param is **never touched** · a Copy struct (even nested) · a `&0` borrow param | ✅ 0 |
| `return length(p.s)` — **merely READING a field** | 🔴 134 |
| `let s = p.s` · `return p` | 🔴 134 |
| `let q = p` · a nested struct · **`return inner(p)` forwarding** | 🔴 139 |
| two heap params, or heap+Copy, **with an addition** | 🔴 132 |
| a `Leaf?` param | ✅ fail-closed `"Struct? Drop without slot"` `:3476` |

**S9 forwarding is the scariest:** it needs no exotic syntax — the lowerer itself emits
`Deinit(_0); Drop(_0)` after `Call inner(_0)` (ADR-0042 Q1).

## 🔬 TWO SYMPTOM FAMILIES, ONE ROOT (O measured; nothing inferred)

- **M-α → 139.** `Deinit(param)` at `:2917`/`:2921` is gated on `struct_slots` → misses → falls into
  the `:2944` fallback `def_var(var,0)`, which **erases the caller's pointer** → `Drop` loads from
  address 0.
- **M-β → 134.** The field move-out tombstone at `:3239` is gated on
  `struct_slots.get(&source.local)` → **the whole block is SILENTLY skipped**, including the
  `len@8`/`cap@16` sync at `:3251`.
- **132 is NOT a separate bug.** Isolating the variable: the trigger is **the addition**, not the
  number of params. `_2 = move _0.s` → `_3 = move _2.len` reads a `len@8` that **was never written** =
  garbage → the `Add` violates the ADR-0044 range check → SIGILL.

🩸 **Two steel probes:** (1) disabling `:2944` → **139 → 134** (proving the zeroed pointer is the input
to the SIGSEGV, and that a real double free lies underneath). (2) writing the marker `777` into
`len@8` → **132 → 134** (proving the SIGILL comes from the garbage len cell). The control `c1` (a
local with **identical** MIR) did not move in either case.

## 🦷 THE BLIND SPOT — MEASURED FROM THE CORPUS, NOT INFERRED

533 fixtures. The three heap-struct-param fixtures (`258`/`260`/`264`) have callee bodies of **exactly
one line, `return 0`** = the only cell still clean. `14` does touch a param, but `Point` is **Copy**.
⇒ **ZERO real uses of a heap-bearing struct param.** An exact repeat of the `Pt{x,y}`-Copy blind spot
from session 07-27(f) — the HP.3 law.

## 📍 THE FIX — 53 lines, purely additive, in one place

The param prologue loop allocates a `StackSlot` and copies in `layout.total_size` bytes, **mirroring
String `:2691` / Enum `:2730` / Outcome `:2756` — Struct was the FOURTH missing aggregate ABI.**
The loop walks only `signature.parameters` ⇒ **`Local(0)` sret is never touched.**
The scope is locked to plain `MirType::Struct`; `Nullable(Struct)` keeps its fail-closed refusal.

🔑 **The argument that locked the root-fix order: piecemeal patches had failed 3 times** —
`WO-NullableEnumParamABI` (`:2704`), `WO-StructParamABI` (`:1343`), and this one.

🔑 **The load-bearing invariant O found:** copy-in is only sound because the lowerer emits
`Deinit(arg)` **unconditionally** for every Move-type argument. Remove it → the canary goes red at
**two layers** (the structural test FAILED + a real SIGABRT 6).

## ⚖ D REFUTED O — AND WAS RIGHT

O's WO pointed the invariant at `triet-lower/src/lib.rs:4462-4465`. **Wrong place.** There are **TWO**
identical sites: `:4462` (returning `ret_local`) and `:4544` (returning `dest`); `take(p) -> Integer`
goes through the **second**. D dumped it, corrected it, and O's poison canary proved D right.

## 🔴 THE P0 DEBT EXPOSED — sret returns garbage SILENTLY (G set it as the next front)

```tri
function make() -> Leaf { let p = Leaf { s: "hi" }; return p; }   // 0 PARAMETERS
```
→ returns **`94060113734544`**, **exit 0**. No crash, no diagnostic.
A clean worktree at `35f4f02` → also garbage ⇒ **pre-existing and orthogonal**.
⇒ `WO-SRet-Aggregate-StringField-Corruption`, **P0**, and no feature campaign may open before it.
⚠️ The mechanism (`_0` sret has no slot ⇒ the `len@+8`/`cap@+16` sync at `:3156-3168` never runs) is
still **D's diagnosis — O has NOT measured it independently.**

## ⚔ BLEMISHES THIS SESSION

- **O undercounted the gates: said "10", the truth was 49** (`grep -c`). Self-corrected inside the WO,
  and turned it into a requirement for D to submit a classification table covering all 49 sites.
- **O's `^FAIL` grep missed because the line is indented by 2 spaces** → nearly concluded "there are no
  teeth". The law *"no output ≠ green"* saved it for the second time in two consecutive sessions.
- **D's prose contradicted its own table** ("13/6/30" in the text versus **15/5/29** in the table). The
  table was right.
- **D submitted a gate whose `test failures` section was SUMMARIZED** → **O REJECTED outright**, read
  no files, ran no gate on D's behalf. The first time O enforced the decree exactly as G issued it
  (historically O conceded and turned the law into an empty threat). D resubmitted raw on the next round.
- **O refused G's order "O, add a fixture"** — the authority matrix hard-locks *D's exclusive pen on
  fixtures* (built after the APP.2b-1 incident). O handed it to D and verified it. The outcome G wanted
  was unchanged, and the flow was correct.

## 🟡 SMALL DEBTS RECORDED

1. `step_by(8)` writes past the slot when `total_size` is not a multiple of 8 (`triet-mir:1590`, which
   happens when every field is a Trit/Trilean/Tryte). O forced 2 probes → **currently NOT observable**
   (Cranelift pads the frame). **A pattern inherited from the Enum branch at `:2743`**, not spawned by
   this WO.
2. `param_aggregate_copyin_counting.rs` is **in-process**, not subprocess-isolated → under poison the
   whole binary dies and the 6 assertions cannot report individually. Real teeth, but crude on the red side.

[[campaign_aggregate_move_tombstone]] [[campaign_drain_fifo_teeth]] [[campaign_truc_b_heap_in_aggregate]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

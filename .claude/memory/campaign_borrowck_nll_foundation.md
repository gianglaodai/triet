---
name: campaign_borrowck_nll_foundation
description: "✅ CLOSED 2026-07-18 — BORROWCK FOUNDATION SURGERY: NLL liveness (kills Drop(reference)-as-read) + alias propagation across Assign/CFG-merge + E2450 overhaul; plus the &0 Enum chain (Slice A → P0 Enum-Return-sret → Slice B payload sub-borrow). origin/main 7cd387d, gate 0·0·407·0. TWO DISEASES MASKING EACH OTHER + fixture 102 as witness. MEMORY.md index only points here."
metadata: 
  node_type: memory
  type: project
  originSessionId: ded168f4-7a9c-48af-9ed8-d64a9122041c
---

## ✅ CLOSED — 4 fronts, 4 commits PUSHED (O+G signed, 2026-07-18)

```
7cd387d  Slice B — enum payload sub-borrow (String/Struct/Vector/HashMap)   gate 0·0·407·0
45a431c  P0 BORROWCK FOUNDATION — NLL liveness + alias propagation + E2450  gate 0·0·400·0
ae20d75  P0 Enum Return sret + plugs twin Nullable(Enum) silent-miss        gate 0·0·398·0
d4486ad  Slice A — consume &0 Enum via match (ADR-0084 §AMEND, E1050)       gate 0·0·393·0
```
ADR-0046 **UNFROZEN** (G lifts the "don't touch NLL" order — the very wart it protected is the bug we're removing).

## 🔑 BIGGEST FINDING — TWO OPPOSITE-DIRECTION DISEASES MASKING EACH OTHER

| Disease | Mechanism | Consequence |
|---|---|---|
| **Under-refuse** (alias loss) | `Statement::Assign` copying a reference **doesn't re-anchor the loan** to the new dest → the loan stays anchored to a now-dead local → NLL ends it per the rules correctly → the reference value lives on **unshielded** | **Silent UB leaks through** at EVERY CFG merge (match + if/else) |
| **Over-refuse** (lexical masquerading as NLL) | `liveness.rs:191` counts `Statement::Drop(l)` as a **READ**. The lowerer emits a Drop for every local including references — but `&0` **has no drop obligation** → the Drop is a semantic no-op yet liveness treats it as a real use → every loan stays live to the end of scope | Blocks correct code by mistake |

**The dataflow engine is NOT wrong** — the backward fixpoint is correct, `is_live_after` (`liveness.rs:107`) is already correct point-level NLL. **Only the INPUT is garbage.** (Both O and G guessed the wrong spot before measuring.)

**⚰️ FIXTURE 102 = THE CROWN JEWEL.** `102_nested_borrow_uaf_e2450` is a real UAF (a reference to a block-local escapes the block and then gets read). The fake `Drop(reference)` (disease 2) **accidentally masks** it. Fixing liveness ALONE → 102 **slips through silently (runs and returns 5)**. ⇒ **BOTH pillars MUST be brought down in one strike**, there's no safe step-by-step path. Blood runs both ways: with the fix locked in → E2450; remove **both** sites → runs and returns 5, exit 0.

## Four pillars (brought down)
1. **Liveness**: `Drop` counts as a read only when the local is **not** reference-like (`is_reference_like` unwraps Nullable — `(&0 T)?` from get_ref is still a reference). Thread types into `compute`.
2. **Alias**: `Assign` copying a reference → re-anchor every loan whose dest is that local to the new dest. **Reuse `PropagatedLoan` cross-call** (`checker.rs:1139`), don't reinvent it.
3. **E2450 overhaul**: at every point where the owner dies (Drop/StorageDead/move-out): `loan.source anchored to owner ∧ is_live_after(point, loan.dest)` ⇒ E2450. Drop the `!is_propagated` short-circuit (root cause of the over-refuse).
4. **E2440 UNTOUCHED** — O measured: already correct, doesn't piggyback on Drop.

**5 fixtures that "broke" split into 2 kinds:** 94/95/21/24 = **over-refuse, breaking IS the FEATURE** (the shape "create a reference then don't use it again" — NLL should correctly let it through) → rewritten as real violations. 102 = **real UAF** → had to be saved.

## ⚖ D REFUTES O — 4 TIMES, CORRECT ALL 4
1. §AMEND-3.2 Copy predicate · 2. `&0` dangling failure-mode · 3. **handle-repr vs inline-repr** · 4. **Pillar 3 (not Pillar 2) is what saves 102**.

**#3 (Slice B):** O's WO wrote the general rule `Struct or is_any_heap() → Borrow`. **WRONG for Vector/HashMap** — they are **handle-repr** (the value IS ALREADY an i64 pointer), `Borrow` returns the address-of-the-cell-holding-the-handle → the shim reads the wrong thing → **silent MISS** (measured: `94891986642280` instead of 3). Struct/Enum/**String** = **inline-repr** → `Borrow` is correct. **O slipped because the probe used String+Struct — two examples of the SAME repr class.**

## 🩸 LESSONS O ATE HIMSELF (etched)
- **★ A DEFENSE MECHANISM HAS A TWIN SIBLING — hit 3 TIMES.** Loan-ending has 2 sites (`checker.rs:1091` statement-level + `:1347` terminator-level); after the E2450 fix it also has 2 sites (Drop + StorageDead). **Poisoning just one = INERT** (the other one carries the load). Nearly concluded "innocent" and headed the wrong direction. ⇒ **When poisoning a defense mechanism doesn't go red → ASK IMMEDIATELY "is there a twin site?"** This is variant (c) of poison-doesn't-go-red: not (a) unobservable in principle, not (b) a weak test, but **two mechanisms masking each other**.
- **★ STALE BINARY — hit AGAIN** (lesson #12 already in the record). Restored `checker.rs` + ran the gate but **forgot to rebuild release** → reported to G that if/else "caught it" (actually it LEAKED through). Got suspicious seeing "caught it but no site fired" — that's what exposed it. **`cargo build --release` BEFORE EVERY binary run.**
- **★ A GIT COMMAND RETURNING NO ERROR ≠ DOING THE RIGHT THING.** While rewriting a commit message: `git cherry-pick` returned **empty** (no clear error) → `--amend` immediately after **overwrote the Slice B message onto the foundation commit, erasing it**. Caught because there's a mandatory step requiring `git diff <origin> HEAD` to be empty — it was NOT empty. Recovered via a `safety/` branch planted before touching the knife, rebuilt with **`git commit-tree`** (splices the exact tree onto a specified parent — safer than cherry-pick for a message change).
- **★ TOPOLOGY LEAK inside a WO** (caught by G, not O): the foundation commit used fixtures 410/412 **written using the Slice B feature** → splitting them apart breaks it. **The foundation must NEVER borrow bricks from a feature as its load-bearing test.** O measures via a worktree cherry-picked onto `ae20d75` → FAIL 412 (E1050). Fix: move 410/412 into Slice B, backfill with fixture **413 struct match-merge** (Alias Loss **independent of Enum**).

## Dual Verification (G's mandate — every commit must stand on its own)
| Milestone | Gate | Evidence |
|---|---|---|
| Foundation `45a431c` | 0·0·400·0 | over-refuse gone · merge leak closed (`y_if`, struct-match-merge, **0 Slice B lines**) · 102→E2450 · **`x_latB`→E1050** (Slice B absent ⇒ foundation stands on its own) |
| Slice B `7cd387d` | 0·0·407·0 | 4 payloads work correctly (2/33/3/2) · P0 unchanged · **`x_latB`→E2440** (feature present + shield present) |

`x_latB`: **E1050 → E2440** across two milestones = a topology poem (G).

## Carried-over debt
- **Caller/callee ReturnShape divergence → panic** (`mir_lower.rs` `inst_results[0]`): pre-existing, shared with Struct, **unreachable from user input** (both read the same `func_return_types`). Needs **cross-body ABI verify = its own ADR**.
- **Full sret for `Enum?`**: touches disc-niche ADR-0065. Deferred, **guarded by a loud-siren refuse** (unit-only `Enum?` at return position, 3-tier predicate `Nullable ∧ Enum ∧ unit-only`).
- ~~**`Nullable(Enum)` in `resolve_aggregate_size`** (`lower/lib.rs:503` mistakenly looks up `struct_map`): a time-bomb, unobservable today~~ → **✅ CLOSED 2026-07-18 (`186bd1c`, PA-A).** ⚠️ **This note was WRONG IN TWO PLACES:** (1) the real coordinate is **`:567`**, not `:503` (line numbers drifted); (2) **NOT "unobservable"** — it's a **LIVE** bug, reachable with 5 lines of valid Triết (`struct Mid{e:E?,m:Integer}` → reading `mid.m` yields 42 instead of 5, exit 0). It survived because of **ZERO COVERAGE**, not because it was locked behind a guard. **Lesson: "unobservable" is a CLAIM, it must be measured before it's logged — don't infer it from "looks like it's covered by a refuse".** Details → [[campaign_nullable_enum_aggregate_pa_a]].
- Reference inside a struct field: lowerer refuses it (not an escape hatch).
- `&0 mutable` payload sub-borrow: ADR-0081 FROZEN.
- ⚠️ BOMB FIX-2 zero-@8 · ⚰️ ADR-0068 Box FORBIDDEN.

[[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]] [[campaign_typed_collections]]

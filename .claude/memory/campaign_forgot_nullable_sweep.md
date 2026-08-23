---
name: campaign_forgot_nullable_sweep
description: "✅ CLOSED 2026-07-20 — Campaign sweeping clean the family of bugs 'match exact, FORGOT Nullable' (6 members, 2 sitting INSIDE the safety net) + closes full-SRET nullable aggregate (Slice A Struct? + Slice B Enum?) + kills UB free(1) container-element. origin/main 51edd0e, gate 0·0·452·0. D refutes O 12/12 correct all 12; O wrong 6 times, same root 'ordering before measuring'."
metadata:
  node_type: memory
  type: project
  originSessionId: 61f64f48-b9b4-485d-9ae7-3e54b99fcbda
  modified: 2026-07-24T14:03:54.858Z
---

## ✅ CLOSED — 8 commits, all O ✅ + G ✅, PUSHED

```
51edd0e  docs(todo): close WO-5
61a8136  docs(adr): ADR-0065 §15 — §4 boundary
07ca203  test: WO-5 R1-R5 teeth + Step ① leak-counting
f432987  fix:  WO-5 kills free(1) UB (Vector/HashMap<_,Leaf?>)
19a7708  test: WO-4 B1 ty_total_size teeth + remove poison
ff1b751  fix:  WO-4 B1+B2 (ty_total_size + drop-glue arms)
dadd91c  docs: close ADR-0065 §14 (Slice A+B)
c320262  feat: Slice B Enum? disc-niche SRET
+ 5a61e74/056ce1f (WO-1) · 9e2b4c3/d624cba/235e376 (Slice A + §14) · 372ba7f (§13 sign-off)
```
Final gate `0·0·452·0 CLEAN`. Fixtures 439→452.

## 🧬 THE "match exact, FORGOT `Nullable`" FAMILY — SIX members in ONE session

| # | site (file:line) | with `Nullable(_)` | fixed at |
|---|---|---|---|
| ① | `is_struct_return` `triet-lower:264/320` | Scalar miscompile | Slice A |
| ② | `is_fat_ret` `Expr::Call` `triet-lower:3103` | ABI arg-count panic | Slice A (**D found it, O missed it**) |
| ③ | `is_enum_return`/`is_enum_ret` `:305/3130` | Scalar miscompile | Slice B |
| ④ | 🔴 `INV-Enum-shape` verifier `triet-mir:1883` | **ESCAPES the safety net** | Slice B (**O dug it up himself via grep**) |
| ⑤ | 🔴 `ty_total_size` `triet-jit:981` `_=>8` | silent garbage (future caller) | WO-4 B1 |
| ⑥ | 🔴 `emit_heap_free_at` (drop dispatch) | silent leak | WO-4 B2 (unwrapped in place) |

**Two (④⑤) sit INSIDE the very net/API that is supposed to be safe** ⇒ a systemic disease, not an accident.
🦷 **Etched rule §14.7:** `is_fat_ret` has **THREE copies** (`:320` callee · `:3103` Call caller · `:5219` method-call fail-closed) — whoever touches one MUST grep the other two. There's now a teeth unit test (B3 `ty_total_size`).
🦷 **New rule:** a predicate/API at the foundation layer → **grep the ENTIRE family before scoping the blast radius**, don't read sequentially.

## 🔴 THE ONE LIVE UB (WO-5) — `free(1)` container-element

`Vector<Leaf?>`/`HashMap<_,Leaf?>` (Leaf carries a String): `emit_vector_element_free_loop:1802` strips `Nullable` BEFORE calling `emit_heap_free_at` → loses the tag-guard/+8-shift → reads the **TAG(=1) as if it were a heap pointer → `free(1)` SIGABRT 134**. Passes typecheck + borrowck, blows up at runtime. **Fix:** refuse container-element heap-nullable at `Body::verify()` (Copy-gated — `Vector<P?>` Copy still runs). Remove the dead branch B2.

## ⚖ TWO TIMES O ORDERED SOMETHING THAT NEARLY BROKE THE PROJECT — D BLOCKED BOTH

**T5 (Slice A):** O wrote an acceptance criterion demanding D build a **counting tooth `FREE==1` for heap-bearing `Struct?`** — i.e. drop-glue, which **§4 forbids in capital letters, in a sentence naming D directly**. O wrote T5 into §14.6 of the SAME ADR without rereading its own body. D STOPPED-BEFORE-TYPING, asked. → T5 withdrawn, became T5' negative.

**R2 (WO-5):** O ordered refusing local heap `Struct?` "as policy". **O's own poison proves it: refusing → 15 fixtures BREAK** (338-346 `pop`/`remove` return `T?` = `Nullable(Struct-heap)` local with the SAME MirType as user-written code; `Body::verify` can't see the AST). **O's WO was WRONG.** → exposed a **constitutional contradiction §4 ↔ ADR-0082**.

🦷 **Same recurring flaw:** seeing a missing mechanism → the reflex is to *add a mechanism*, instead of asking *is this shape ALLOWED TO EXIST at all*. **An engineer fixes holes; an architect asks whether the hole should exist.**

## ⚖ CONSTITUTIONAL AMENDMENT — ADR-0065 §15

§4 "no drop-glue" was written as absolute, but ADR-0076 (heap-`T?` field) + ADR-0082 (`pop`/`remove` returning `T?`) **had already legalized** `Nullable(Struct-heap)` and **had already built CORRECT drop-glue** (`struct_drop` arm: tag-guard, niche=8, +8-shift). Step ① measurement: **local `Leaf?` FREE=1 dup=0 SOUND**. §15 settles it: §4 applies NARROWLY to **repr-slot construction ADR-0065**, does NOT forbid the shape existing at local/pop-result. R2 REVOKED, fixture 455 = permanent control. **Forbidden going forward to invoke §4 to refuse local/pop-result.**

## 🩸 O WRONG 6 TIMES — same root "acting/ordering BEFORE MEASURING"
1. **The measuring instrument is blind to borrowck** — counting harness `lower_source()` SKIPS borrowck; O measured 3 pretty shapes then discovered the real driver REFUSES all 3 (E2423). Nearly reported a false result. Cross-checking against the driver saved it.
2. **T5 broke through the B8 fence** (above).
3. **Edited the ADR tree while D was holding the pen** → gate contamination (436/439, build 2), looked exactly like a real regression. The commit was blocked by the hook, no harm done — but it blinded the measuring instrument on its own.
4. **Fabricated an error code `E<code>`** that doesn't exist in `LowerError` (D verified, reported the discrepancy, did not fabricate a fake code).
5. **Classified local as "policy-hole needing a patch"** — WRONG, local is sound + is already-shipped behavior.
6. **R2** (above).
🔑 3 times self-caught or blocked by D; **the frequency IS the data**. The discipline "measure first" **hasn't become reflex yet, it's still a process that has to be remembered**.

## Role notes — D (Sonnet 5): session MVP
**Refuted O 12/12 times, correct all 12.** 5 times **STOPPED-BEFORE-TYPING** to ask (locking down #8 ABI · the B8 fence · StructAlloc+8 site 5 · a nonexistent error code · R2). 2 times saved the project from tear-down-and-rebuild. 0 technical fabrications. Self-reported forgetting a poison, self-removed it.
**Remaining blemish = reporting discipline:** left the turn hanging waiting on the gate **4 times** (once leaving a live `panic!` in the RULE7 tree), dodged via Monitor/background. 🔑 **O's conclusion: an INFRASTRUCTURE limit, not an attitude one** — nudging by reminder stops working after the 2nd time; **needs a hard constraint (foreground+timeout written into the template), not a 4th repeated reminder.** Died mid-task twice from quota (session + weekly) — early WIP commits are real insurance.

## 🔴 DEBT STILL OUTSTANDING (packaged as separate campaigns)
1. **`LowerError` has NO error-code system** — violates `CLAUDE.md` (every error must be `miette::Diagnostic` + `E<code>`). Every lowerer refuse is prose, not machine-fixable per ADR-0027. Separate cleanup campaign.
2. **`mir_lower.rs:3730` PANICs instead of returning `Err`** — violates Track B rule #1. Not yet reachable from valid source (only fires under poisoning), a bomb waiting on a shape mismatch.
3. **Container-element `Nullable(Struct-heap)` currently REFUSED** — to support it, let the free-loop KEEP the `Nullable` and route through the `struct_drop` arm (like local), instead of stripping it first (§15.6).
4. **N1 hole** (`let x:E?=~0` + widening bypass) — POLICY-HOLE not UB (measured FREE=1 dup=0, value correct). §13.
5. **`Struct?`/`Enum?` return via method-call** = over-refuse (copy of #3, probes 448/453).
6. ~~**WO-3 teeth guarding `builtin_shim_meta`** — SPOF `arg_consumes` has no teeth yet.~~ **✅ CLOSED 2026-07-24 — ADR-0085 Beat 1 (P-exist: full table + `verify()` gate) + Beat 2a (self-loan-exclusion, `mutates_arg:Some(0)`), origin/main `f6b569f`.** Beat 2b debt (P-flag canary — a lying flag) STILL outstanding. → [[campaign_shim_meta_spof_adr0085]]
7. **Deep-Clone · drain · BOMB FIX-2 zero-@8** (carried over from previous sessions).

[[campaign_nullable_position_and_temp_ownership]] [[campaign_nullable_enum_aggregate_pa_a]] [[campaign_aggregate_nullable]] [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]] [[mentor_o_persona]] [[colleague_d_persona]]

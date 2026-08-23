---
name: campaign_methodcall_fat_return_and_triage
description: "🏁 WO-MethodCallFatReturn (the ADR-0065 §14.7 AMEND closing copy #3 of is_fat_ret) + a full debt-ledger TRIAGE, 2026-07-25(h). O caught 1 of its own false alarms (P1) + 1 zombie label (KM-P1b)."
metadata: 
  node_type: memory
  type: project
  originSessionId: bea909b1-7952-44fb-9f1e-e3c4d5ba9035
  modified: 2026-07-25T18:51:20.433Z
---

# WO-MethodCallFatReturn + the debt-ledger triage — session 2026-07-25(h)

origin/main **`45919cd`** (synced), gate `0·clean·0·463·0`. 2 commits: `c88a5ae` (the WO) + `45919cd` (the ledger triage).

## 🏁 WO-MethodCallFatReturn (the ADR-0065 §14.7 AMEND — closing copy #3)

**The problem:** `is_fat_ret` exists in **3 copies** (§14.7): #1 in the callee `Ctx::new :446` · #2 in the
caller `Expr::Call :3358` · #3 in the caller `Expr::MethodCall :5501`. Copies #1 and #2 unwrap `Nullable` →
handling `Enum`/`Struct?`/`Enum?` returns through sret; **copy #3 does NOT unwrap** (a bare
`matches!(callee_ret, Struct(_))`, missing `is_enum_ret`) → those three shapes fell into an **E1100
over-refusal**. Fail-closed (an Err before any MIR is emitted; the comment at `:5493` is a deliberate
refuse-over-guess) → **NO UB**. Fixtures 448/453 were **deliberate tripwires** from the 07-20 session
(EXPECT=ERROR, "do not fix :5219").

**The fix:** mirror copy #2 into copy #3 — add `is_enum_ret` + the `Nullable` unwrap in `is_fat_ret`; have
`sret_layout_name` unwrap the inner type; add the `is_enum_ret → EnumAlloc` + `ReturnShape::Enum` branch;
drop the word "Enum" from the refusal message. Copy #3's argument ordering `[sret, receiver, explicit]` is
unchanged. **Vector/HashMap/Reference KEEP the refusal** (G's red line).

**O's teeth (INDEPENDENT granular poisons — stronger than D's blanket revert):** Poison A neuters
`is_enum_ret=false` → 453 and 469 refuse while **448 passes** (the enum path is independent of struct);
Poison B turns the struct unwrap into a bare match → 448 refuses while **453 and 469 pass** (the struct path
is independent of enum). The two paths are **orthogonal** — no fixture rides on the other's fix. A red-line
probe: a `Vector<Integer>?` method return → E1100 (the unwrap does not leak). Fixtures 448→10, 453→5, 469 a
bare-Enum exhaustive match→5. D honestly declared 1 real deviation (fixture 179's substring message).
D = Sonnet 5, 0 deadlocks.

## 📋 A FULL DEBT-LEDGER TRIAGE (Giang ordered it before closing the session, 5 parallel recon spears)

**Not one entry is a live UB pit — everything is fail-closed.** The results (verified against the code and
probes at HEAD `c88a5ae`):

- **🩸 P1 Vector-scalar-return (which O had flagged this same session) = A FALSE ALARM.** Vector/HashMap are a
  **single-i64 handle** (a pointer to a 3-field heap buffer), UNLIKE String (3 fields inside a 24B slot →
  sret). `ReturnShape::Scalar` is CORRECT; the caller at `:3507` and the callee at `:466` are
  **symmetrically Scalar**. Probes: `make()->Vector`→42, fixture 166→3. **Flag withdrawn.**
  O had planted it from half-reading the code without probing — the "conclude before measuring" pattern;
  refuse-over-guess applies to O's claims too.
- **KM-P1b HashMap<String,V> = A ZOMBIE LABEL.** Already closed by `381979e` and locked; the TODO's
  "[ ] D is coding it" was stale. Probes: String-key insert + get→1, E1048 wired. Ticked [x] and closed.
- **N1 widening = A MISLABEL.** It is not "a clean E1120 refusal" — E1120 **SLIPS** at widening and on the
  fast path (`let x:E?=E::V(42)` / `=~0` exit 0). **G classified it as a POLICY HOLE, NOT UB** (measured
  2026-07-20, ADR-0065 §13).
- **Panic Group C (the layout `.unwrap()`) = reachability MEASURED → NOT a P1 ICE.** `i64_to_usize`'s
  debug_assert compiles out in release, BUT **no builtin takes a user i64 as a size** (`vector_new`/
  `hashmap_new` take 0 arguments and grow by doubling from the header) → negatives are impossible (a floor),
  and an overflow would need exabytes that do not exist (the OOM null catches it first). Unreachable from
  source, purely defensive. 🚩 A tripwire: re-verify if a builtin with a capacity hint, a resize, or a repeat
  is ever added.
- **Panic Group B (the host ISA) = a legitimate PARK** (RATIONALE: a fatal environment error, like rustc
  without LLVM).
- **REAL and correctly labelled (their own campaigns):** ADR-0088 double nullables · deep Clone · drain ·
  the §15.6 `Vector<Leaf?>` refusal.
- **🆕 Secondary findings:** (a) `for/loop/break/continue` pass parsing and typecheck but the lowerer refuses
  with E1100 (`lib.rs:2144`) — a half-silent trap, related to the drain design; (b) a bare `T??` declaration
  (outside `get`) slips through typecheck → the MIR verifier reports "heap-nullable B8", POINTING THE WRONG
  WAY (the verifier over-matches the inner `Integer?`) — ADR-0088 does not cover it yet.
- **🔵 AN OUTSTANDING VERIFICATION DEBT — ADR-0084 field auto-deref (Slices 1a/1b):** the code landed
  (`d02c0c4`+`006b6c7`) and the corpus is green (381→30 / 383→5 / 385→4 / 387→E2440) BUT **the ADR is still a
  DRAFT awaiting O's signature** and **tooth-386 is VACUOUS** (the file says E2450 while the CLI binary gives
  **E2400**, fatal in typecheck; E2450 is only visible through the phase-merging test harness — the tooth
  does not bite a real user). Next session: O opens a dedicated verification spear, examines tooth-386, and G
  signs. This is the lesson of O's laws #15/#21.

## Session lessons
1. **Refuse-over-guess applies to O's own claims** — the P1 false alarm came from flagging "a suspected bomb"
   without probing. Half-reading code → a wrong conclusion. The 12th instance of the same root, "acting
   before measuring" (persona laws 18/20).
2. **Backlog labels are NOT trustworthy — triaging by probe exposed 3 pieces of garbage** (1 false alarm by
   O, 1 zombie, 1 mislabel) in a single pass. The same law by which recon-before-WO saved us 4 times in
   earlier sessions.
3. **A tooth in a phase-merging harness can be VACUOUS** (386): the test PASSES while the tooth does not bite
   a real user (the CLI stops at an earlier phase).

→ [[campaign_aggregate_nullable]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]

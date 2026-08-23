---
name: campaign_spof_container_nullable
description: "✅ CLOSED 2026-07-25 — WO-SPOF-1: sealing the single-point-of-failure refusal of container `Nullable(Struct-heap)` at struct fields and enum payloads (Option B, ADR-0065 §15). origin/main 0285bf2, gate 0·clean·0·455·0. O flipped G with measurements (the framing 'the UB is sticking out' was WRONG — there is no live UB; the REAL pit was a single-layer SPOF)."
metadata:
  node_type: memory
  type: project
  originSessionId: 1cdd999a-6278-40aa-b41e-b788b111d906
  modified: 2026-07-24T17:34:17.136Z
---

# WO-SPOF-1 — sealing the container `Nullable(Struct-heap)` refusal SPOF (closed 2026-07-25)

origin/main **`0285bf2`** (cleanly synced). Gate `0·clean·0·455·0`. Fixtures 452→455.
The front G chose this session = "container Nullable(Struct-heap) refusal, §15.6" from the debt ledger.
D = a Sonnet 5 subagent (Giang ordered the spawn); O gatekept with blood verification; G approved the recon
and signed it closed.

## 🔑 O FLIPPED G WITH MEASUREMENTS — the "the UB is sticking out" framing measured as FALSE
G declared *"the free(1) UB tail is still sticking out through the refusal gap, tolerating the UB's rebirth"*.
O applied **verify-don't-trust to G as well** (ritual #10): probes P1/P3/P4/P5 → **there is NO reachable
observable UB**. WO-5 (`f432987`) had already sealed the exploding site; the construction-side scan
`find_refused_nullable_container` runs over **every** local_decl (`triet-mir/src/lib.rs:1976`) and return
(`:1957`), catching every materialization of `Vector<Leaf?>` — including an inline `vector_new()` (P4's
temporary `_3` is caught) and params (P5's `_1`).

## The REAL pit (a different kind): the refusal is a single-layer SPOF, not a live UB
The struct-field/enum-payload predicate `find_refused_nullable_field` (`:1884`) has arms for
`Nullable/Reference/Outcome/_=>None` — with **NO `Vector`/`HashMap` arm**. Its doc comment rationalizes
"a field is a position, not a container" — **a false argument**: a field of type `Vector<Leaf?>` is both a
position AND a container whose element is a nullable heap value. The two verification loops, struct field
(`:1990`) and enum payload (`:2002`), call ONLY that arm-less predicate and never the container scan (unlike
returns and locals, which run BOTH). **P6/P7 prove it:** `function consume(b: Bag)` with
`struct Bag{v: Vector<Leaf?>}` / `enum Box{Full(Vector<Leaf?>)}` **passes verification and the JIT cleanly,
exit 0**, emitting latent `free(1)` drop glue. Safety hanging on ONE gate (the construction scan) = a **SPOF**,
exactly the ADR-0085 `builtin_shim_meta` pattern. Classifying (a)/(b): (a) currently unobservable (no caller
can build a non-empty vector) + (b) the predicate gap is REALLY reachable during verification → filed as a
**latent-UB SPOF**, neither live fire nor cosmetic.

## The fix — Option B (G's ruling): defence in depth, NOT loosening the feature
Add `find_refused_nullable_container(&field.ty/&payload.ty, self)` to the struct-field loop (`:1988`) and the
enum-payload loop (`:1999`), mirroring returns and locals. **Do NOT touch** `find_refused_nullable_field`
(keep it checking the direct position — G's order), `_container`, or `is_field_payload_lowerable`.
`+35/-8` in one file. New positions: `struct field \`Bag.v\` (container element)` /
`enum payload \`Box.Full\` (container element)`.

## O's blood teeth (harness level, laws 15/21)
- **Poisoning the load-bearing path:** swap in the PRE-D lib.rs (removing the 2 checks) while keeping the
  3 fixtures → the corpus reports `FAIL 459/460: pipeline succeeded with 0` — the hole opens and both
  fixtures go RED. That also covers harness-genuineness (459/460 flip when the fix is removed).
- **Proving 461 is non-vacuous:** fabricate `EXPECT 0→999` → `FAIL 461: expected 999, got 0`.
- Restored from `cp` snapshots (PRE_D/POST_D md5 `ff3d1fd1…`, 461.orig), never git checkout. The final
  independent gate was `0·clean·0·455·0`. The diff against the pre-D baseline matches the scope exactly
  (only lib.rs's 2 loops + 3 fixtures).

## D's blemish + discipline
- **D submitted a SUMMARIZED gate the first time** (the `=== test failures ===` line replaced by "(see the
  full log above)"). O **REJECTED outright** per the IRON PROTOCOL ("paste the raw gate or get out") — no
  verifying on D's behalf, no concession. D coughed up the raw output and admitted the error. The
  reporting-discipline pattern is an infrastructure limit (already concluded); a hard constraint in the WO
  only bites when O actually enforces the rejection instead of running rescue. G's praise: "cold enough to
  hold the baton".
- The push used `timeout 300` (the pre-push hook runs clippy + tests); ls-remote confirmed `6f546b6→0285bf2`.

## Debts carried forward
- **§15.6 support** (removing the refusal so `Vector<Leaf?>` RUNS through the `struct_drop` arm) — a
  feature, **deferred**.
- **THE NEXT FRONT G HAS ALREADY CHOSEN (recon only, no WO yet): push_owned-vs-M3 isolation** — the
  `arg_consumes` SPOF (the lowerer's `push_owned` and the JIT's M3 read the same table, with 0 cross-check
  armour). G ordered 3 pieces of recon: (1) name the file:line sites that are lowerer-based versus
  JIT-based/blindly-trusting; (2) the isolation tactic (the JIT guards itself independently instead of
  blindly trusting the table); (3) a poison that feeds `arg_consumes` a wrong answer → forcing a bad
  compile through → the JIT must trap/panic and block it once the armour is installed. **No WO is to be
  written before G approves the recon.**

[[campaign_forgot_nullable_sweep]] [[campaign_shim_meta_spof_adr0085]] [[campaign_nullable_position_and_temp_ownership]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_teeth_never_git_checkout]]

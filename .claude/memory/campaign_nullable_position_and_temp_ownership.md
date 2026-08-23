---
name: campaign_nullable_position_and_temp_ownership
description: "✅ CLOSED 2026-07-19 — 5 consecutive WOs: the family 'match exact, forgot Nullable' at 3 POSITIONS (Enum? param · Struct? param · Struct? return) + INV-HeapNullable probe (SOUND, busts a lying doc-comment) + ShimTempOwnership (silent leak across THE WHOLE ARRAY of borrowed shims). origin/main aa9e584, gate 0·0·439·0. Biggest finding: SPOF arg_consumes."
metadata:
  node_type: memory
  type: project
  originSessionId: 9f55d317-ec4f-4bce-bded-eac47a8223a5
  modified: 2026-07-19T16:22:40.715Z
---

## ✅ CLOSED — 5 WOs, all O ✅ + G ✅, PUSHED

```
aa9e584  docs(todo): refresh handoff header + SPOF debt
72a0bd6  WO-ShimTempOwnership CLOSED (8 commits 04b6174…72a0bd6)
c88832a  WO-INV-HeapNullable-Probe (a) — busts a lying doc comment
645ae61  WO-StructReturnRefuse (e7aab8c fix + fixtures 437-445)
ec7ecd8  WO-StructParamABI  (7d59b7c fix + fixtures 428-436)
ccb8db3  WO-NullableEnumParamABI (fixtures 419-427)
```
Final gate `0·0·439·0 CLEAN`. Fixtures 419→445 + 4 new counting files.

## 🧬 THE THREAD: the family **"match exact, FORGOT `Nullable`"** — 4 members

| # | Site | Symptom | Status |
|---|---|---|---|
| ① | `Enum?` param copy-in (`mir_lower.rs` matching `MirType::Enum` exact) | **silent garbage**, the `~0` branch dead across every call boundary | fixed `ccb8db3` |
| ② | `Struct?` param bare-read (`load_place:1248-58` no slot → `use_var` = pointer) | **silent garbage** | fixed `7d59b7c` |
| ③ | `Enum?` return-shape | — | already refused before |
| ④ | **`Struct?` return-shape** (`is_struct_return = matches!(ret, MirType::Struct(_))`) | 4 holes: silence · garbage address · SIGILL 132 · **SIGABRT 134** | POLICY GATE refused `e7aab8c` |

⚠️ **④ sits exactly 10 lines from a comment written by O HIMSELF in a previous session** — that comment names the phenomenon *"P0-sibling gap"*, patches ① sibling then leaves the remaining sibling behind.

## 🔑 SHARED MECHANISM
- **param**: the Variable holds a **pointer** into the caller's slot → sentinel-compare against `i64::MIN` compares addresses → **always "present"** ⇒ the null branch is dead. Field-read is still CORRECT (reads through the pointer) → only the tag-read is broken.
- **return**: the two branches produce **incompatible** representations for the same type — null → `const NULL_SENTINEL` (scalar), present → `struct P{..}` **BARE, no tag**. Both tag-read AND field-read are broken.
- **Working precedent:** `Integer?`→`Scalar` (correct, sentinel fits in i64) · `String?`→`Struct` fat/sret via `is_string_repr()` — this predicate **deliberately covers** the `Nullable` wrapper too. That's the correct pattern that `is_struct_return` is missing.

## 🩸 SILENT LEAK ACROSS THE WHOLE ARRAY OF BORROWED SHIMS (WO-ShimTempOwnership)

Surfaced **thanks to** the counting infrastructure from the previous WO, not from reading code.

```
length(h.name)            FREE=0 RED     userfn f(h.name)     FREE=1 OK  <- refutes the "shared temp-lifetime" hypothesis
length(o.inner.name)      FREE=0 RED     length(s) local      FREE=1 OK
length("hello")           FREE=0 RED  <- NO field-access -> kills the name "InlineFieldTempLeak"
concat 3->1 · contains 2->0 · eq 2->0   (every case actually leaks exactly 2 temps)
push/insert (CONSUME)     FREE=1 both inline AND let-bound -> HEALTHY, is CONTROL
```
**Correct spec:** an **anonymous** temp (field-access OR literal) used as an arg to a builtin that **borrows** never gets `push_owned` → nobody drops it. `let` is fine (registered via `let`), user-fn is fine (ownership transferred via `Deinit`, ADR-0042 Q1).

**Fix:** chokepoint `emit_shim_call` looks up `arg_consumes` (borrow/missing-entry → `push_owned`; consume → forbidden) + a separate fast-path fix for `length()`. **Blast radius wider than the signed scope** (also sweeps `remove`/`get` key) — G's review keeps it wide: *"narrowing = writing `if name=="remove" { keep_leaking_on_purpose() }`, that's STUPID, deliberately creating a sibling gap"*.

**⚠️ Oracle `hashmap_string_key_struct_value_remove_frees_key_and_value` 2→3:** the old value was **pinned to a LEAKING baseline**. O verifies via **pointer-identity `frees=3 distinct=3 dup=0`** ⇒ NOT a double-free. Whoever reverts to 2 reopens the leak.

## 🔴 BIGGEST DEBT LEFT BEHIND — **SPOF `arg_consumes`**

`builtin_shim_meta().arg_consumes` is read by **BOTH** layers: `push_owned` (lowerer `emit_shim_call`) + **M3 zero-on-consume** (JIT `mir_lower.rs:4717`).
⇒ **NOT defense-in-depth — it's ONE decision applied to two layers.** One entry lying about itself breaks both:
- declared **borrow** but actually **consumes** → leak
- declared **consume** but actually **borrows** → double-free
- **both are SILENT** at the value layer. `contains` has no entry → falls to an implicit default.

**No teeth guard this table yet.** Direction: a unit test that sweeps the whole table against the real shim signatures.

## ⚖ O WRONG 11 TIMES — SAME ROOT: **acting before measuring**

1-9. Generalizing from ONE observed variable: exit-code used as oracle (6 controls tagged ✅ by luck) · `Struct?` param "healthy" from a single cell · `T7 refuse ✅` from one constructor shape (`~+`, missing `~0`) · naming the bug after field-access (P6 `length("hello")` kills that name) · "small hole in `length`" (measurement by D refutes it: the whole array).

**10. WRONG failure-mode label:** wrote "SIGILL 132" for `Struct?` param; re-measured 5/5 → reading **TWO** fields = SIGILL (garbage+garbage exceeds the threshold → trap **ADR-0044**, **SECONDARY**), reading **ONE** field = silent garbage. **Silent garbage is the root; SIGILL is the thunder.**

**11. HEAVIEST — designed the ACCEPTANCE CRITERION around an ASSUMED mechanism.** O declared *"if poison-reverse doesn't blow up ⇒ reject"*. Forced a two-way measurement:
- M3 **on** + no distinction → FREE=1 **doesn't blow up** (D is right)
- M3 **off** + correct distinction → **SIGABRT double-free**
⇒ **M3 is actually the load-bearing layer**; the `!consumed` branch was hidden. **O withdraws the criterion.** Had it been kept, O would have **rejected a CORRECT fix** and forced D to fix something that wasn't broken.
🔑 **The very poison placed in the WRONG spot is what dragged the SPOF out — disciplined failure produced the finding.**

## 🦷 NEW TEETH RULES (etched into the persona)
- **The oracle is also an assumption — it must be verified.** exit-code doesn't measure value; **value doesn't measure leak**; **FREE-count doesn't distinguish 3 objects from a double-free** (must dedup pointers).
- **Teeth must be proven at the HARNESS LAYER.** `integration_test_corpus()` is ONE test running a loop ⇒ one fixture crash kills the whole process, every fixture after it **never runs**. "Suite is red" does NOT prove your own tooth. How to prove it: change `EXPECT`/`ERROR` to a fabricated value → must produce `FAIL <name>: expected …, got …`.
- **A green test can be guarding a WRONG status quo** (oracle pinned to a leaking baseline). The correct fix turns it red — **forbidden to fix the oracle back to green without independent evidence**.
- **"Poison doesn't go red" must be pushed to the end**: remove the covering layer (turn off M3) then re-measure, don't conclude from the happy path alone.

## Role notes — D (Sonnet 5)
**Refuted O 8/8 times, correct all 8.** Technical track record: **0 fabrications**. Biggest bright spot: **stopped when the test went red, did NOT fix the oracle 2→3 to make it green on its own** — the opposite behavior would have shipped a double-free wearing the mask of "updated expectation".
Remaining blemish = **loop discipline**: 4 violations of the foreground rule (once **the WO was returned by O**), 3 times left work hanging uncommitted. 🔑 **Rules must ban BEHAVIOR, not ban a TOOL** — ban `run_in_background` and D dodges via `Monitor`; ban *"end the turn before you're holding the output"* and there's no dodge.

## Debt still outstanding
🚩 **SPOF `arg_consumes`** (above) · 🚩 **ADR "Full SRET for Nullable Aggregate"** (removes BOTH policy gates `Struct?`+`Enum?` return; requires fixing the widen-path — the present arm doesn't write the tag — AND sret sizing `{tag@0,fields@8+}`) · `key_marshal` >8B param (**over-refuse noise, NOT UB** — O measured, deprioritized) · N1 `~0` bypass hole (policy-hole) · `is_empty` · `HashMap<String,V>` key-position.

[[campaign_nullable_enum_aggregate_pa_a]] [[campaign_borrowck_nll_foundation]] [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]] [[mentor_o_persona]] [[colleague_d_persona]]

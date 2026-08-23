---
name: campaign_adr0088_lane_a_nested_nullable
description: "✅ CLOSED 2026-07-27 — ADR-0088 Lane A: E1055 fence for nested-nullable T?? 2 levels / 2 source layers + 9 teeth (511-519). Kills ICE E1190 + misdirected MIR message. Core finding: the old refuse lived parasitically off the allow-list with 0 guard teeth = a silent SPOF waiting for Lane B. Lane B deferred indefinitely. d9b659a, gate 0·clean·0·511·0."
metadata:
  node_type: memory
  type: project
---

## ✅ CLOSED — `d9b659a` (D) + docs (O co-sign). O✅/G✅/Giang✅ 2026-07-27

Gate `0·clean·0·511·0 CLEAN`. Fixtures 502 → **511**.

## 🎯 Recon flips the frame — the body of ADR-0088 covers ONLY the `get`-family

Giang signed off on opening ADR-0088. O recon: **20 probes on the release binary** (rebuild
first, rule 12) → **REFUTES the frame "double-nullable = a problem of `get`"**. `T??` written
DIRECTLY had never been measured by anyone:

- **NO UB** — every path fails closed (exit 2/3, 0 silently-wrong exit-0 cases, 0 signal
  132/134/139).
- **But diagnostics are BROKEN: 5 error codes for the SAME concept.**
  - 🔴 `struct S{v:Integer??}`+match → **E1190 = the ICE code** "please report compiler bug"
    for syntactically valid user input ⇒ violates the ADR-0086 taxonomy.
  - ⚠️ local/param/return/pop/pop_front/remove/`!!` → the MIR message says *"heap-nullable…
    ADR-0065 §4 B8 Struct?/Enum? Copy-only… [Fix] Remove the heap field"* — `Integer??`
    is NOT heap, NOT struct ⇒ completely misdirected, violates ADR-0027 machine-fixable.
  - ⚠️ enum payload → E1141 (wrong cause) · `Integer??~E` → E0001 parse.

## 🎯 THE BIGGEST FINDING — 0 GUARD TEETH, a silent SPOF waiting for Lane B

The old refuse lived parasitically off the **allow-list** `is_lowerable_nullable_payload`
(`triet-mir:1796`: scalar/heap/Enum/Struct/Reference). `Nullable(_)` **has no entry
in the list** ⇒ falls outside ⇒ refuses **by default** = **structural luck**, not
an active fence. And grepping `??` across **lines of code** in the whole corpus = **0 hits**.

⇒ The FIRST thing Lane B will do is **add a `Nullable` arm to that very allow-list** —
at that point 7 paths slip through the JIT at once, **silently, gate still green**. The same
one-layer-SPOF shape already plugged in WO-SPOF-1. **This is the real reason Lane A is
worth doing, not "cleaning up the message".**

## TWO SOURCE LAYERS (why the WO can't be "one touch-point")

| Layer | Where `Nullable(Nullable(_))` is generated | Covers |
|---|---|---|
| **A declaration** | `resolve_type` **2 COPIES**: `check.rs:1365` + `check_resolved.rs:597` | local·param·return·struct field·enum payload·`!!` |
| **B inference** | `check_call` after `return_type.substitute(&sub_map)` (`check/exprs.rs`) | `pop`/`pop_front`/`remove` on `<T?>` |

`let x = pop(v)` has **NO annotation** — the shape only exists AFTER substitution.
🦷 Layer A has **2 copies** = the same shape as the `is_fat_ret` 3-copy pattern (ADR-0065 §14.7):
touch one copy, you MUST grep for the other.

## ⚖ TWO OVERRULES — both correct

**D refutes O's WO location:** O suggested `env.rs:374/394/506`; D pointed out that there `T`
is still an **abstract** `TypeParameter`, with no knowledge of what it binds to at the
call-site ⇒ moved to `check_call`. O's targeted poison confirms D placed it correctly. The
guard is **not name-gated**.

**O refutes G's verify protocol — G withdraws it:** G ordered *"poison the allow-list → 7 fixtures red"*.
O measured FIRST and proved it would **NOT go red** (typecheck blocks it, sealing off the MIR
layer) ⇒ a 1-probe protocol pushes the verifier into a "poison doesn't go red" trap that then
forces **fabricating a fake probe to force a blow-up just to pass the gate**. G approved
**2 independent probes**. 🔑 This is exactly the mark O once ate himself (rule 16
"acceptance criteria are also an assumption") — this time caught **before the work was
handed off**.

## 🩸 O VERIFIES IN BLOOD — 3 probes + specificity in BOTH DIRECTIONS

| Probe | Poison | Measured |
|---|---|---|
| 1a | disable Layer A `check.rs:1374` | **6 red** 511·512·513·517·518·519 · 514-516 **green** |
| 1b | disable Layer B `exprs.rs:1054` | **3 red** 514·515·516 · the other 6 **green** |
| 2 | widen the MIR allow-list | **only unit test red · 0 fixture** |

**6+3=9 ⇒ the two layers are CLEANLY SEPARATE**, neither impersonates the other. Under
probe 1a, **517/518 re-expose the EXACT old ICE** (`unsupported match pattern` / `requires an
expected type`) ⇒ the new guard is exactly what killed E1190/E1141, not a coincidence.
**Teeth at the HARNESS LAYER** (rule 15): changing `// ERROR:` 514→`E9999` → produces the
line `FAIL expected 'E9999', got E1055`. Restore via `cp`+md5 matching 4 files, **0 git
checkout**; `git diff` vs D's commit = **empty**.

## ⚔ NEW INVARIANT — a runtime message is FORBIDDEN from containing another layer's error code

D self-discovered + self-fixed: the draft MIR message contained the string `"(E1055)"` →
the harness compares via `.contains(code)`, so poisoning away the typecheck guard still
went **falsely green** (the MIR layer blows up, the message incidentally contains "E1055").
D removed the code string from the runtime message and re-ran with the real harness.
🦷 **If one layer's message contains another layer's error code, every cross-layer poison
is SILENTLY neutralized.** D did not take the shortcut of faking it to pass the gate —
0 fabrication marks.

## Control against over-refuse (keep forever)

struct `Integer?` ONE layer → **16** · `HashMap<K,Integer?>` insert-store → **5** ·
flatMap 175/212/213 green (`exprs.rs:361-364` keeps the body nullable, **never generates
`U??`**) · 465/466/467 **keep E1051**, not hijacked by E1055 · 468 positive control.

**E-code boundary:** `E1051` = `get`/`get_ref` (untouched) · **`E1055`
`NestedNullableUnsupported`** = nested `T??` at every other position.

## 🩸 O's blemish

Missed **item ④ Documentation Integrity** from G when drafting the WO (not handed to D)
→ O carried out the docs himself instead of calling D back. Lesson: a WO must be checked
back against every condition G issued, not drafted from memory.

## ⏸️ LANE B — DEFERRED INDEFINITELY

A real `T??` design = a 3-state repr (the `i64::MIN` sentinel currently has only **1
null bit**, not enough room for 2 independent layers) + ABI + match ergonomics + parser
`??~`. G's ruling: without a use-case demanding a distinction between *"key doesn't
exist"* vs *"the stored value is null"*, this is **building a bridge before there's a
river**. Workaround: sentinel value / a wrapper Struct with a `present` flag. **The 9
teeth 511-519 will blow up red if Lane B opens the allow-list without doing enough — that
is exactly why they exist.**

## §AMEND-1 §88A.4 — correction to the body of ADR-0088

The sentence *"The guard does NOT block `contains`"* describes **behavior that does
not exist**. Actually measured: `contains(m,1)` with `V=Integer?` → **E1041
NoMatchingOverload** (the overload table doesn't declare a generic `V`) — it is not let
through. Anyone who thinks `contains` is a valid workaround for `HashMap<K,V?>` will slip.
**A wrong documentation label, not a hole.**

[[campaign_typed_collections]] [[campaign_forgot_nullable_sweep]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]

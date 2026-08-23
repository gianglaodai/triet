---
name: campaign_nullable_enum_aggregate_pa_a
description: "✅ CLOSED 2026-07-18 — WO-NullableEnumAggregate-Refuse (PA-A): fixing a LIVE silent miss where `Nullable(Enum)` consulted the wrong struct_map inside the co-fixpoint sizing. G's ruling SWAPPED THE ROLES: N3 is the soundness fix, N1 is the policy gate. D refuted O's WO for the 5th time out of 5 — and was right. A new debt: the `Enum?` param ABI, SIGILL 132. origin/main 5c713c4, gate 0·0·412·0."
metadata:
  node_type: memory
  type: project
  originSessionId: 78c71263-44c7-40b2-be94-2fae740e93dd
---

## ✅ CLOSED — 2 commits PUSHED (signed by O and G, 2026-07-18)

```
5c713c4  docs(todo): close PA-A, record G ruling, log Enum? param ABI debt
186bd1c  fix(track-c): refuse payload-bearing nullable enum in aggregates (PA-A)   gate 0·0·412·0
```

## 🔑 THE BUG — a LIVE silent miss, not a time bomb

⚠️ **The old ledger was WRONG on two points, now corrected:** the coordinates are
**`triet-lower/src/lib.rs:567`** (not `:503` — the line numbers drifted), and it is **NOT "unobservable"** —
it is a **LIVE** bug, reachable with 5 lines of valid Triết:

```triet
enum E { V(Integer), N }
struct Mid { e: E?, m: Integer }
Mid { m: 5, e: E::V(42) }   //  mid.m reads back as 42, NOT 5.  exit 0.
```

**The mechanism:** `:632` seeds a field at 8B; the co-fixpoint exists ONLY to correct that seed. The
`Nullable(Enum)` branch consults **`struct_map`** — where enums are **never** registered (that function's own
comment says so, two lines away in the `MirType::Enum` branch) → **always a MISS → fallback → the fixpoint is
a NO-OP**. A payload-bearing 16B enum (disc@0 + payload@8) crammed into an 8B slot ⇒ it overflows and stomps
the next field.

**Why it survived:** it was not caged — there was **ZERO COVERAGE**. The gate was CLEAN at 407 even with the
fix applied. The old refusal at `Expr::OutcomeConstructor` only guards `~+`/`~0`; **assigning a bare enum into
an `E?` field (an implicit widening, `Expr::EnumVariant`) walks straight around the cage.**

⚰️ **The 13/07 debt "Enum-Payload-Aggregate Sizing Fix" does NOT share this fate** — it was closed by
`9a1799c` (ADR-0067 §AMEND co-fixpoint, 16/07). What was just patched is a **SIBLING bug** in the same
function. Struck from the ledger.

## ⚖ G'S RULING — SWAPPING THE N1/N3 ROLES

| | Role | Evidence |
|---|---|---|
| **N3** (`:567` `struct_map`→`enum_map`) | **THE SOUNDNESS FIX** — the real fix | remove N1 and keep N3 → **no shape corrupts** |
| **N1** (the declaration-layer scan in `lower_program` after the fixpoint) | **A POLICY GATE** — blocking until ADR-0065 blesses the representation | **NO** observable failure mode exists to poison red |

**🚫 FORBIDDEN from now on to cite N1 as evidence that a UB path was closed.** G keeps N1 absolutely, because
the `Enum?` surface is still broken elsewhere (the SIGILL 132 below) — "opening a surface before it is
properly prepared is suicide".

**O verified 6 shapes independently:** a 24B aggregate payload · reading back the nullable field itself · a
nested nullable enum payload · 3 heap shapes (heap is blocked by `heap_type_not_supported`, **not** by N1).
All correct.

## 🦷 N3 HAS EXACTLY ONE TOOTH — and why it must

N1 blocks EVERY fixture-level path that touches N3 ⇒ N3 would become **ghost code**: the day ADR-0065 removes
N1, whoever reverts `enum_map`→`struct_map` revives the bug **silently, with a green gate**. G approved O's
demand for this.

The tooth is the unit test `resolve_aggregate_size_nullable_enum_reads_enum_map_not_struct_map` (in `lib.rs`'s
mod tests, able to call the private fn from the same module). **O verified independently:**
- flip the token → **RED** (`left: 8 / right: 16`)
- **it is specific**: poisoning the twin site for bare `Enum` at `:570` → the test **STAYS GREEN**
- that twin site at `:570` **has its own tooth** → `enum_field_moveout_frees_once_with_cap` goes red under
  poison. **Both sites are guarded.**

## 🔴 A NEW DEBT — the `Enum?` PARAMETER ABI IS BROKEN (SIGILL 132)

D found it, O verified it. `function pick(u: U?)` with a **unit-only** `U`: `pick(a)` → exit **132** for BOTH
the present arm (`~+ U::A`) AND the null arm (`~0`); the `SwitchInt` falls into the default Trap arm.
**Pre-existing** — reproduced on the pristine `564f0f7`. It matters because a unit-only `Enum?` is something
we **currently allow** (417/418 guard the field and local positions, **not the param position**); none of the
412 fixtures touches `Enum?` in a parameter. **It needs its own WO.**

## 🩸 THE LESSON O SWALLOWED (carved)

**★ A POISON PROTOCOL MUST BE CROSS-CHECKED AGAINST YOUR OWN MEASUREMENTS.** O measured the control variable
at the start of the session (`:567`, change one token → p1/p2/p3 give **7** ⇒ N3 alone already fixes it), then
hours later wrote a WO whose teeth said *"remove N1 → observe 42"* — **the two propositions cannot both be
true**. No missing data, **no leap of logic**. D caught what O did not.
⇒ Verify-don't-trust used to apply to *conclusions*; it now applies to **test design** as well.

**★ D REFUTED O FOR THE 5th TIME OUT OF 5 — right all five.** See [[colleague_d_persona]].

**★ A subagent deadlock:** D ran `gate.sh` in the **background** and then ended its turn waiting for a
notification → a subagent ending its turn means it is finished, so the notification NEVER arrives. Stuck for 2
turns (~40 minutes). ⇒ **A subagent must run any command whose result it needs in the FOREGROUND with a long
timeout.** Credit to D: while stuck it did **not fabricate `(all pass)`** to escape.

**★ O did NOT run the gate on D's behalf while D had not submitted a raw gate** — the law held, unlike the
2026-06-11 incident where G cut O down for conceding on procedure three times.

## Role notes
D's model = Sonnet 5. Blemishes this session: **no fabrications** (the SIGILL 132 failure mode was described
PRECISELY — the opposite of the old blemish "invented a SIGSEGV when it was really a leak"). The only blemish:
being stuck without asking (LAW 4).

[[campaign_borrowck_nll_foundation]] [[campaign_aggregate_nullable]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]]

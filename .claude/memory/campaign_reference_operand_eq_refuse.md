---
name: campaign_reference_operand_eq_refuse
description: "✅ CLOSED 2026-07-30(b) — WO-Reference-Operand-Eq-Refuse: the E1058 fence was blind to every operand passed through a reference (&0 Struct/Vector/HashMap/String?/Enum → compares ADDRESSES, silently wrong with exit 0). origin/main fdbd66d, gate 0·clean·0·581·0, 1 commit, +6 fixtures. Law 34 was born: patching an N-arm match ⇒ N orthogonal poison spears."
metadata:
  node_type: memory
  type: project
---

## ✅ CLOSED — 1 commit, PUSHED. O✅/G✅ 2026-07-30(b)

`origin/main` **`fdbd66d`** (verified with `git ls-remote`), gate **`0·clean·0·581·0`**. Fixtures 575 → **581** (+6).
A single commit (G's ruling: the code and the 6 fixtures protect the SAME invariant, splitting them is meaningless):
`fix(typecheck): refuse eq on reference aggregates and borrowed enums (E1058)`.

## The disease — the twin sibling of the previous session's `WO-String-Eq`

The E1058 fence (`check/exprs.rs:779` → `Type::is_eq_refused()` `types.rs:263-274`) **does not cover
operands passed through a reference**. In the JIT, `Statement::Borrow` (`mir_lower.rs:3467`, branch
`:3494-3501`): a **slot-backed** local (`struct_slots`/`enum_slots`) → `stack_addr` = an **ADDRESS** ⇒
the generic `icmp` compares addresses ⇒ two allocations with identical contents **always** give `0`
(or `1` with `!=`). Exit 0, no diagnostic.

🔑 **Task A missed `Reference` in the JIT, Task B missed `Reference` in typecheck — O's matrix was
blind on BOTH SIDES OF THE SAME AXIS.** That is why the hole survived the very campaign that patched
next to it.

## Measurement table (19 probes, measured by O on a rebuilt binary)

| Operand | Measured | Note |
|---|---|---|
| `&0 P{x,y}` — **all 5 forms** `&0`/`&+`/`&-`/`&0 mutable`/`&+ mutable` | `0` 🔴 | each form measured, nothing inferred |
| `&0 Vector<Integer>` · `&0 HashMap<String,Integer>` · `&0 String?` | `0` 🔴 | |
| `&0 Color` with **NO payload**, same variant | `0` 🔴 | **owned = `1`, CORRECT** (fixture 572) |
| `&0 Color?` | `0` 🔴 | the hole G pointed out, confirmed by O's measurement |
| `&0 String` (5 forms) · `&0 Integer`/`Integer?`/`Trilean`/`Trit` | `1` ✅ | left unchanged |
| owned `Color?`/`Integer?` via `let r: Trilean = a == b` + `match r` | CORRECT ✅ | ⇒ the refusal **may only live inside the `Reference` branch** |

**The divergence mechanism (dumped, not inferred):** the MIR for `&0 Integer` and `&0 Color` is
**identical** (`_2 = &0 _0` then `_2 = _0 == _1`). The divergence is in the JIT: a scalar is **not
slot-backed** → `use_var` = **COPY THE VALUE** ⇒ it is correct **because of the representation, not
because of dispatch**. `String` is correct thanks to the explicit dispatch at `:3551` (`d8fa041`).

## The fix — 1 touch point, 1 caller

```rust
Self::Reference(_, inner) => match inner.as_ref() {
    Self::String => false,                                                    // content-compare, d8fa041
    Self::UserEnum { .. } => true,                                            // owned is correct; borrowed compares ADDRESSES
    Self::Nullable(n) if matches!(n.as_ref(), Self::UserEnum { .. }) => true,  // its OWN arm, see below
    other => other.is_eq_refused(),
},
```

⚠ **`Nullable(UserEnum)` MUST have its own arm** — falling into `other.is_eq_refused()` means
`Nullable(inner)` is only refused through the `String`-inner rule or by recursion, and a payload-free
enum returns `false` ⇒ **the hole reopens one layer down**. G spotted it before D typed anything; O
confirmed by measurement (`q1` → `0`).

## 🔑 LAW 34 (issued by G) — PATCHING AN N-ARM `match` ⇒ **N ORTHOGONAL POISON SPEARS**

O's WO specified only **3** spears (String-exempt + the 2 enum arms) ⇒ **none of them proved that
`585/586/587/588` had teeth**. O added **P4** itself (blinding `other => other.is_eq_refused()`).

| Spear | Break | Red | Green (orthogonal) |
|---|---|---|---|
| P1 | `String => false`→`true` | `579-582` | 585-590 |
| P2 | delete the `UserEnum` arm | `589` "succeeded with **0**" | **`590`** |
| P3 | delete the `Nullable(UserEnum)` arm | `590` "succeeded with **0**" | **`589`** |
| **P4 (added by O — a hole in O's own WO)** | blind the `other` branch | `585-588` | `589 590` |

`587` goes red with *"succeeded with **1**"* — the silent-wrong signature of `!=` (two different
addresses ⇒ true). The restore `md5 efc7277…` matched after **all 4** rounds (`cp` snapshot, **never**
`git checkout`).

## ⚔ BLEMISHES THIS SESSION

- 🩸 **Law ㉔ caught G:** G disputed O's fixture numbering ("there are 584 fixtures, `584` is free, the
  expected gate is `589`"). O refuted with `grep`: the corpus has **575** files · the highest number
  **584 is ALREADY USED** (`584_owned_string_eq_control.tri`, landed with `d8fa041`) · there are **9
  numbering gaps** (`16-19 123 304 496 498 540`) ⇒ `584−9=575`. The correct gate = **581**. **The
  highest number is not the count** — assuming contiguous numbering is the trap. G accepted and kept
  `585-590`.
- 🩸 **A hole in O's own WO** (P4) — law ⑫ (teeth must sweep the whole variant space) applied to O's
  own poison design, and O missed a branch. That gave birth to Law 34.
- ✅ **D got it right on the first round:** 1 touch point verbatim, 6 fixtures with the right numbers,
  a complete raw gate, no touching the JIT / owned paths / E1033, no inline literals, no overwriting
  old fixtures. **0 red rounds.** The only blemish: the comment on `587` gave the reason for refusing
  `Nullable(String)` as *"fat-String sync"* — the real reason (`577:2-7`) is that the shim only
  understands a bare `{ptr,len}` String **plus the Ł3 null-compare semantics being unspecified**. It
  omits the main clause without being factually wrong ⇒ recorded, not worth burning a round.
- ✅ D took the `cp` snapshot **AFTER** patching (the WO said "cp FIRST") — which fits the purpose here
  (the restore point is the submitted state), and md5 proves it. Not flagged as a deviation, but harmless.

## 🩸 DEBTS AFTER THIS FRONT — G'S ORDER (2026-07-30(b))

1. **P2** `WO-Literal-Temp-Drop-Leak` — `if s == "hi"` ⇒ `_1 = const "hi"` is never dropped, 2 allocs
   and 1 free. The root is in the lowerer's rvalue temporaries. **A memory leak ⇒ highest priority.**
2. **P3** `WO-Harness-Subprocess-Isolation` — the counting harness is in-process, and one SIGILL kills
   every assertion after it (a repeat of Law 15).
3. **P4** `WO-String-Ordering-Spec-Gap` — `SPEC.md:870` allows `String < String`, the implementation
   refuses with E1004.
4. **P5** `Nullable-Eq-Unknown-Spec-Gap` — `Integer? == ~0` → `0` (false), **never `Unknown`**.
   `SPEC.md:870` only specifies non-nullable primitives ⇒ null equality **has no specification**. The
   type system says it "may be Unknown" (which is why E1033 exists) but the runtime **never** returns
   Unknown. **G: this needs an ADR, not an ad-hoc WO** — fold the SPEC normalization together with P4.
   Ranked after P2/P3/P4 (no UB, no crash).
5. **Recorded debt: `&0 T?` bypasses E1033.** An owned `T? ==` is blocked by E1033 at the `if`;
   through a reference, `eq_result_type` sees a top-level `Reference` ⇒ returns `Trilean!` ⇒ the `if`
   lets it through. **That path is exactly what let the silent-wrong hole survive.** The invariant
   "bare Nullable equality is forbidden" is **still broken** when wrapped in a reference — E1058 only
   seals the silent-wrong path, not E1033.

**Not measured, forbidden to claim as covered:** `Outcome ==` · `Capability ==` ·
`Reference{Reference{…}}` · an Enum sret with a String payload (E2423 blocks the probe) ·
`&0 Long`/`&0 Tryte` (the literal syntax blocks the probe).

[[campaign_sret_stringfield_and_string_eq]] [[mentor_o_persona]] [[colleague_d_persona]]
[[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]]

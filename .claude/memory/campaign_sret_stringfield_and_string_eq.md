---
name: campaign_sret_stringfield_and_string_eq
description: "✅ CLOSED 2026-07-30 — TWO silently-wrong-exit-0 campaigns: ① WO-SRet-Aggregate-StringField-Corruption (2 orthogonal holes: Hole A projected→projected + Hole B Nullable(String)) · ② WO-String-Eq-Content-Compare-And-Aggregate-Refuse + Task C borrowed (E1058 + content comparison, owned and borrowed). origin/main d8fa041, gate 0·clean·0·575·0, 6 commits, +29 fixtures. 4 CORRECTIONS from the same shape-inference root: D 1, G 2, O 1."
metadata: 
  node_type: memory
  type: project
  originSessionId: d369b9e2-ec32-4a93-b7f7-c978937dbfee
  modified: 2026-07-30T07:56:26.451Z
---

## ✅ CLOSED — 6 commits, PUSHED. O✅/G✅ 2026-07-30

`origin/main` **`d8fa041`** (verified with `git ls-remote`), gate **`0·clean·0·575·0`**. Fixtures 546 → **575** (+29).

| Commit | Content |
|---|---|
| `b29394d` | Hole A — syncing fat-String len/cap for projected→projected (the sret path) |
| `e5db13c` | Hole B — `Nullable(String)` in STEP 4 |
| `cfbbaae` | String-Eq Task A+B code (**bundled, violating §8.8** — see below) |
| `8726908` | fixtures 565-572 + 2 harnesses |
| `9f4c0a5` | fixtures 573-578 (E1058) + CLAUDE.md |
| `d8fa041` | Task C — borrowed String `==`/`!=` through a `Reference` operand |

## 🔴 THE THEME RUNNING THROUGH THE SESSION: SILENTLY WRONG WITH `exit 0` — 5 holes, 0 corpus coverage

Every hole this session was a **silent wrong value with exit 0**, sailing through every CI gate. Not one was denounced by a crash.

## ① `WO-SRet-Aggregate-StringField-Corruption`

⛔ **D's DIAGNOSIS WAS REFUTED:** *"the sret `_0` has no `struct_slots` ⇒ the sync block never runs"* — WRONG.
A probe on exactly that changed **nothing**. That phrasing is forbidden from being reused.

**TWO ORTHOGONAL HOLES** in the fat-String sync of `Statement::Assign`:
- **Hole A** — a `projected → projected` move (`_0.s = move _1.s`). It slips past all three gates: `:3146`
  (`ty_total_size(String)==8` ⇒ `is_aggregate=false` ⇒ the scalar path writes only `ptr@0`) · `:3209`
  (`source.projection.is_empty()`) · `:3318` (`dest.projection.is_empty()`).
- **Hole B** — `:3210` `matches!(dest_ty, MirType::String)` **drops `Nullable(String)`** ⇒
  a `String?` field breaks **even WITHOUT sret**.

🩸 **Two steel probes:** the marker `777` → `len@8` ⇒ `length()` correctly returns `777`; the marker `100000000` → `cap@16`
⇒ `__triet_string_free` receives it and feeds it into `std::alloc::dealloc`.
**The real state:** a `dealloc` layout size of garbage **~9.4e13 (~94 TB)** on every run (live UB, tolerated by glibc) ·
`println` → **SIGSEGV 139** · with 2 String fields → an extra `free(real ptr, cap=0)`.

🔑 **The 2×2 table (`dest.projection × source.projection`) is what dragged Hole B out of the dark** — O planted it in the
WO **before D typed a single line**. Patching only Hole A would have made G's fixture `558` **impossible to turn green, forever**.

## ② `WO-String-Eq-Content-Compare-And-Aggregate-Refuse` + Task C

`mir_lower.rs:3462` `Statement::BinaryOp` **does not dispatch on type** ⇒ `load_place` on a String returns 1 i64 =
`ptr@0` ⇒ `icmp` compares **pointers**. `"hi"=="hi"`→`0`, `"hi"!="hi"`→`1`. The shim `__triet_string_eq` (`:5632`)
already existed but was wired only at `:786` (HashMap keys).

**A radius wider than the brief:** `P{x:1,y:2} == P{x:1,y:99}` → **`1`** (only the first 8 bytes compared) ·
`E::A(1) == E::A(2)` → **`1`** (payload ignored) · `String? ==` reaches the real JIT (`E1033` only shields the `if`).

**The fix:** Task A dispatches `MirType::String` → the shim · Task B fences with **`E1058`** (`exprs.rs:762-775` +
`Type::is_eq_refused()`) covering Struct · Vector · HashMap · enums WITH payloads · `Nullable(String)` ·
Task C extends the dispatch to `MirType::Reference{inner: String}`.

🔑 **Authority:** ADR-0038 §4 DESIGN LOCK — *"the `==` and `!=` operators stay as they are and return Trilean"*; only
`compare()` is deferred. ⇒ `String` must content-compare and must **NOT** be refused. `SPEC.md:870` specifies only
**primitives** ⇒ refusing aggregates is correct.
⚠ **REFUSING PAYLOAD-FREE ENUMS IS FORBIDDEN** — measured: `E::A==E::A`→`1`, `E::A==E::B`→`0`, currently CORRECT.

**The borrowed mechanism (O dumped it, did not guess):** `&0 String` carries the owner's **StackSlot address**, not a heap
`ptr` — the evidence is the running shim `__triet_string_len` (`:5790-5805`, the doc says so explicitly, plus `len@+8`).
⇒ `{ptr,len}` are fetched with a `load` through the pointer (`use_var` + `load +0`/`+8`), NOT a `stack_load` from
`struct_slots` (`Reference{inner:String}` does not satisfy `is_string_repr()`).
Every form `&+`/`&0`/`&-` shares the representation — **each form was measured**, nothing inferred. Mixing forms inside one
`==` is caught by **E1004** at the type level (each form is its own type).

## ⚔ 4 CONSECUTIVE CORRECTIONS — THE SAME ROOT, "INFERRING THE SHAPE INSTEAD OF DUMPING IT"

| Who | The wrong label | The measurement that refuted it |
|---|---|---|
| **D** | sret is missing `struct_slots` | a probe on exactly that → nothing changed |
| **G** | `return Leaf{s:"hi"}` lowers to `_0.s = move _2` | the MIR: the lowerer STILL builds the temporary `_1` ⇒ the same shape as Hole A, **there is no fourth hole** |
| **G** | "100% certain `< > <= >=` also compare pointers" | **E1004** — typecheck already fences them (`exprs.rs:776-789` requires `is_numeric()`) ⇒ 2 operators, not 6 |
| **O** | "`is_string_repr()` is enough for `String?`" | after Hole A was patched, `558` was still red ⇒ exposing Hole B |

**Nobody is exempt.** G submitted to the measurements both times and withdrew the orders.

## 🦷 THE BLIND SPOT — HP.3 recurring for the THIRD→FIFTH time

- 546 fixtures · 16 files return a struct by value · **2** are heap-bearing, and both are **outside the sick cell**
  (`440` refuses with E1121; `545` `Vector` = an 8B handle) ⇒ **ZERO of 546** touch a `String` field through sret.
- **10 fixtures** have a `: String?` field — **all of them `return 0`**, no fixture ever READ the contents.
- `grep -lE '(==|!=) *"'` across 555 → **1 file**, and it is an E1052 refusal fixture ⇒ **0 of 555** guard `String ==`.
- **0 of 569** guard `&0 String ==`. **0 of 575** guard `&0 Struct ==`.

## 🩸 NEW DEBTS — G'S PRIORITY ORDER (2026-07-30)

1. **P1 `WO-Reference-Operand-Eq-Refuse`** — add `Type::Reference` to `is_eq_refused()`. The E1058 fence
   **does not cover operands passed through a reference**: `f(&0 P, &0 P)` with identical contents → **`0`** (silently wrong).
   **O verified it is pre-existing with a clean worktree at `e5db13c`** — `n5`/`n6` give `0` identically ⇒ orthogonal.
   🔑 **Twin siblings:** Task A missed `Reference` in the JIT, Task B missed `Reference` in typecheck —
   O's matrix was blind on **both sides of the same axis**.
2. **P2 `WO-Literal-Temp-Drop-Leak`** — `if s == "hi"` (an inline literal operand) ⇒ `_1 = const "hi"`
   is **never dropped** ⇒ a leak (2 allocs, 1 free). With two `let` bindings the drops are complete. The root is in the
   lowerer's rvalue temporaries, a different system from the JIT's `BinaryOp` ⇒ G approved splitting it out.
3. **P3 `WO-Harness-Subprocess-Isolation`** — the counting harness is in-process: one SIGILL poison kills every
   assertion after it; O had to run in isolation with `--exact --test-threads=1` to prove the green half. A repeat of Law 15.
4. **P4 `WO-String-Ordering-Spec-Gap`** — `SPEC.md:870` lists `String < String` as valid while the implementation
   refuses with E1004 ⇒ a **SPEC-versus-implementation divergence**. Fail-closed, no UB.

**Not measured, forbidden to claim as covered:** `Integer? ==` · `Outcome ==` · `Capability ==` (behind the E1058 fence) ·
`&0 HashMap ==` · `Reference{Nullable(String)}` · an Enum sret with a String payload (borrowck E2423 blocks the probe).
**Recorded as out of scope:** comparing a String field that **consumes** the field (`_4 = move _0.s`).

## ⚔ BLEMISHES THIS SESSION

- 🩸 **Law ㉔ caught O for the THIRD time:** O labelled `573-578` RED under Poison A — wrong, because O's oracle grepped
  a short string that occurs inside a longer one. **Your own grep pattern is also an assumption.**
- 🩸 **A command error by O:** the order "COMMIT WIP NOW" while D was dying on quota produced `cfbbaae`, bundling Task A+B
  code and violating §8.8. D flagged it correctly and **refused to run `reset --soft` itself**. G accepted it as is and
  warned O: next time an override must be written explicitly as `override §8.8 due to context limits` in the commit body.
- 🩸 **Hook output is NOT proof of a push.** The first push turn printed "Gate B clean. Push proceeding" while
  `ls-remote` still showed `e5db13c`; it landed only on the second turn. **Only `git ls-remote` is proof.**
- ⚖ **G's acceptance criteria must also read the code first:** G issued the criterion *"100% of the 2×2 table must use
  `is_string_repr()`"*; the cell at `:3183` **has no `MirType` predicate at all** (it filters on `struct_slots` +
  `layout.name`). Enforcing it literally would have made D fix a cell that is not broken.
- ✅ **D was honest 3 times in a row:** (a) `557` before the fix was **exit 132 SIGILL** (a garbage `len` × `*10` hitting
  the ADR-0044 range check), not a wrong value — reported precisely instead of shoving it into an existing box; (b) O's
  WO §6.2 said "565-570 must be red", when in reality only `565/567/569` were (`566/568/570` stayed green because the
  pointer comparison was accidentally right when the contents differed) — reported back instead of bending the fixtures
  to the wording; (c) `582` mixed-form could not be built (E1004) → used the escape clause, documented it in a comment,
  and **did not fabricate a fixture to hit a number**.
- 🔑 **The PAIRED `(ptr,cap)` oracle is the new weapon.** The old harness recorded only the **pointer** ⇒ **completely
  blind** to a garbage cap (it still reports `alloc==free==1, dup==0`). It must record `(ptr,cap)` and assert
  `cap_freed == cap_alloc[ptr]`. And it must wrap **both** `__triet_string_alloc` **and** `__triet_string_from_bytes` —
  `from_bytes` calls `alloc` through an **internal Rust call** that bypasses the shim table, so wrapping only one leaves
  the oracle silently empty.
- 🔑 **Poisoning each hole SEPARATELY** is the condition for proving orthogonality. Poison A red on group A / green on B;
  Poison B red on group B / green on A; Poison C red on the borrow cases **only**, **green across all of `565-578`**.

[[campaign_param_aggregate_copyin]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[feedback_verify_producer_before_consumer]]

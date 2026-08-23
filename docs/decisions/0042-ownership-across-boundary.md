# ADR-0042: Ownership Across Function Boundary — B7-lift (Move-only)

**Status:** CLOSED — Mentor O SIGNED (semantics/soundness, 2026-06-07) + Mentor G SIGNED (layout/ABI, 2026-06-07). Implementation complete at `86b7039`.
**Date:** 2026-06-07
**Author:** AI (implementation), final decision: Giang Hoang
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Move semantics for heap types (String, Vector) across user-defined function
boundaries. EXCLUDES borrow params (`&+ T`, `&0 T`, `&- T`) — deferred to Tier C.

---

## Summary

B7-lift removes two refusals in `triet-lower/src/lib.rs:492` (param heap type) and
`:1360` (arg heap type), adds caller zeroing at `CallDispatch::Jit` + borrowck
move-marking for user functions (keyed `CallTarget::Jit`, rather than
`builtin_shim_meta`). Move-only scope — borrow params pruned per consensus of both
mentors. Return path already wired (M4 return-escape applies to user fns).

---

## §0 — Verified Data (Phase 0 Probes, 2026-06-07)

5 probes executed on real driver with tree temporarily lifting 2 refusals (now restored).

| # | Probe | Result | Finding |
|---|-------|---------|-----------|
| P1 | `consume(my_string)` — caller zeroing? | **SIGABRT** `double free` | Caller does NOT zero slot — M1–M3 do not reach across call boundary |
| P2 | Same as P1 | Same as P1 | Double confirmation |
| P3 | `len(s)` after `consume(s)` | **SIGABRT** (double-free before `len`) | Borrowck does NOT catch use-after-move across call — no E2420 |
| P4 | `let t = make()` → `len(t)` | **5** (success) | Return path works: M4 skips callee Drop, caller Drops once |
| P5 | `Integer?` across boundary + Elvis | **7** (success) | PA-3c MIN sentinel preserved across boundary |

O independently reproduced 5/5 probes with 100% matching results.

---

## §1 — Decisions (6 Questions)

### Q1: Caller zeroing at CallDispatch::Jit

Following `CallDispatch` with `CallTarget::Jit`, caller must zero all Move-type args
passed to the callee. Mechanism: loop args, `!is_copy(arg_ty)` → emit
`Statement::Const { value: 0 }` + `Statement::Assign { dest: arg, source: 0 }`
in caller return block. Preserves existing M1 mechanism — simply extends scope
from builtins to user functions.

### Q2: Callee drop preserved

Callee already Drops params upon scope exit (`owned_locals` + `pop_scope` mechanism).
No changes needed. Coordinates with Q1 to prevent double-frees: caller zeroes → callee
receives original value → callee drops once → caller is already zeroed so drop is a no-op.

### Q3: Borrow params PRUNED — move-only

`&+ T`, `&0 T`, `&- T` params across user fn boundaries DEFERRED to Tier C. Both
mentors agree: B7-lift scope is move semantics only. Existing refusal for heap-type
params does not distinguish Move vs Borrow → lift refusal ONLY for Move-passing;
borrow-passing retains Err (currently all heap params use default Move-passing, so
in practice fully lifted).

### Q4: E2420 keyed CallTarget::Jit, check-then-mark

Borrowck M3 currently (`checker.rs:790-805`) only marks Moved for `CallDispatch`
having `builtin_shim_meta`. Patch: add `CallTarget::Jit` branch — loop args,
`!is_copy(arg_ty)` → mark `VarState::Moved`.

**Check-then-mark:** Prior to marking, verify whether arg is already `Moved` → if
so, emit E2420 (aliased double-move: `foo(s, s)` → callee receives 2 params with
identical pointer → drops both → double-free INSIDE callee). Pattern:
`matches!(state.var_states.get(arg), Some(VarState::Moved))` → error, only then mark.

### Q5: T? nullable across boundary

- `Integer?` (Copy): already works, MIN sentinel preserved (P5). No new code needed.
- `String?` (Move): repr already defined (null=MIN) but lacks producer. When producer
  exists, Q1+Q2+Q4 apply identically because `is_copy("String?") → Move`.

### Q6: trap-on-0 retrofit

Direct question to Mentor G: *"trap-on-0 retrofit alters behavior of 5 legacy shims —
do you have any objections given that it lies within the B7-lift move semantics zone?"*

G's response (verbatim): "Double-free is not a trap-on-0 gap; M1-M3 have not reached
CallDispatch." — G confirms the trap-on-0 mechanism is unrelated to the current
double-free vulnerability; double-free stems from missing caller zeroing, not
faulty trap-on-0. Recorded in `MENTOR_G_STATE.md`.

---

## §2 — Acceptance Criteria (G's Checklist, Cross-checked by O)

| # | Criterion | Verification Method |
|---|----------|-------------|
| C1 | Caller zeroes out pointer after call (M1–M3 extended across boundary) | P1: `consume(s)` → no SIGABRT |
| C2 | Callee calls `free()` exactly once upon scope exit | P1+P2: drop trace via MIR |
| C3 | Returning heap value does not double-free (M4 return-escape) | P4: `make() → String` → `len(t)` → 5 |
| C4 | E2420 when reusing variable moved into user fn | P3: `len(s)` after `consume(s)` → E2420 |
| C5 | E2420 on aliased move: `foo(s, s)` | P6: double-move → E2420 |
| C6 | `Integer?` across boundary intact | P5: `maybe_value(0) ?: 7` → 7 |
| C7 | Deinit tombstone ≠ user re-init | PB: `s = "xyz"` after `consume(s)` → 3; PC: `v = push(v,5)` → 1 |
| C8 | Deinit + use without re-init → E2420 | 59 (len after consume → E2420), 62 (foo(s,s) → E2420) |

### §2.1 — Δ4: Deinit vs Assign (2026-06-07)

**Problem:** Zeroing after call (Const 0 + Assign) overwrote VarState Moved → Owned
in borrowck, eliminating E2420. But initial Δ4 patch (sticky-Moved) killed valid
user re-initialization (`s = "xyz"` after `consume(s)`, `v = push(v, x)`).

**Solution:** Separate tombstone from user Assign via `Statement::Deinit`:

- **MIR:** `Statement::Deinit(Local, Span)` — compiler-emitted tombstone, not a user value.
- **Lowerer:** Emit `Deinit(arg)` instead of `Const(0)+Assign(temp→arg)` after
  `CallDispatch::Jit`.
- **Borrowck:** `Deinit` → set `VarState::Moved` (tombstone). `Assign`/`Const` revert
  to legacy behavior: always revive `Owned` (valid user re-init).
- **JIT:** `Deinit` → `def_var(iconst 0)` — identical machine code.

**Soundness:** Deinit + use without re-init → E2420 (fixtures 59, 62).
Deinit + user Assign → Owned revived (fixtures 64, 65).

---

## §3 — Scope (IN / OUT)

| IN | OUT (defer) |
|----|-------------|
| Move heap param across user fn | Borrow param (`&+`, `&0`, `&-`) — Tier C |
| Caller zeroing after CallDispatch::Jit | `String?`/`Vector?` param (awaiting producer) |
| Borrowck move-marking for CallTarget::Jit | Struct/enum heap payload across boundary |
| E2420 use-after-move + aliased-move | |
| Return heap value from user fn (already wired) | |

---

## §4 — Implementation Plan (4 Commits, Full Gate on Each)

1. **docs(adr): 0042** — this ADR
2. **feat(track-b): borrowck move-marking for CallTarget::Jit** — checker.rs:
   add `CallTarget::Jit` branch alongside M3, check-then-mark. Hand-built
   MIR unit test: heap arg → reuse → E2420; removing marking → failing test.
3. **feat(track-b): caller zeroing after CallDispatch::Jit** — lowerer: after
   `CallDispatch::Jit`, emit `Statement::Deinit(arg)`. MIR-level unit test
   (deinit_tombstone_user_assign_revives, deinit_without_reinit_is_e2420).
4. **feat(track-b): B7-lift — remove refusals + Deinit + fixtures 58-65** — delete
   2 guards `:492`/`:1360`. Fixtures: 58=P1, 59=P3 (E2420), 60=P4 (→5), 61=P5
   (→7), 62=P6 (E2420 aliased), 63=temp-arg, 64=PB (re-init →3), 65=PC
   (v=push(v,5) →1).

---

## §5 — Related ADRs / Documents

| Document | Relationship |
|----------|--------------|
| ADR-0040 | M1–M4 zeroing-on-move, builtin shim ABI, arg_consumes |
| ADR-0041 | PA-3c uniform sentinel, nullable repr, `is_copy` delegation |
| SPEC §10 | S6 ownership, 5 reference forms, move semantics |
| `spec/plans/MENTOR_G_STATE.md` | Records Q6 trap-on-0 response from G |

# ADR-0048: Mutable Borrow — Tier C Slice 5

**Status:** ACCEPTED — O + G sign-off 2026-06-08
**Date:** 2026-06-08
**Author:** AI (collaborator D, implement)
**Reviewers:** Mentor O (semantics, soundness) · Mentor G (codegen, ABI)
**Scope:** Enable `&0 mutable String` parameters + mutation op `clear` (in-place len=0).
Exclusivity E2440 reuse. Return-mut-borrow `-> &0 mutable T` CUT.
Append/grow CUT (realloc landmine).

---

## Summary

Slice 2 (ADR-0045) sealed `&0 mutable`, and Slice 3 (ADR-0046) opened `-> &0 T` return-borrow.
Slice 5 opens `&0 mutable` parameters for String, with exactly ONE mutation op: `clear` (sets
len=0, pointer immutable, no realloc). Append/push is CUT because reallocation changes the pointer → caller
holds the stale handle pointing to freed memory. Return-mut-borrow `-> &0 mutable T` is CUT (E1042) — avoiding
`is_propagated` × mutable technical debt.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | `Loan.form: ReferenceForm` is available, distinguishing BorrowReadOnly/BorrowExclusiveMutable/… | `checker.rs:93-104` |
| F2 | Two-tier exclusivity: typecheck `borrow_check.rs` Pass-3 `forms_conflict` (ADR-0025 §2, fatal first-line) + MIR `checker.rs:113-124` `conflicts_with` (defense-in-depth). `BorrowExclusiveMutable` → conflicts with ANY loan. | `borrow_check.rs` (typecheck), `checker.rs:113-124` |
| F3 | E2440 fire site in MIR: `places_conflict` + `loan.conflicts_with(*form)` → emits `NllExclusivityViolation`. Fire site in typecheck: `forms_conflict` returns E2440. | `checker.rs:507-515`, `borrow_check.rs` (typecheck) |
| F4 | Probe: `2× &0 mutable m` → E2440 triggers. Exclusivity infrastructure already functional, no rewrite needed. | Phase-0 |
| F5 | Probe: `modify(&0 mutable m)` e2e compile+RUN — `&0 mutable` param passes through typecheck+lower+jit without issue. | Phase-0 |
| F6 | E1042 gate (`check.rs:398-408`) currently blocks ALL `-> Ref T` except `BorrowReadOnly`. `&0 mutable` return is already blocked — no additional action needed. | `check.rs:398-399` |
| F7 | `PropagatedLoan` copies `form: orig.form` — if return-mut is opened later, form will be propagated correctly. But currently CUT. | `checker.rs:804` |
| F8 | String layout: `{len: i64@0, cap: i64@8, bytes@16}`. `clear` writes 0 to len@0 — pointer immutable, does not touch cap, does not allocate. | `mir_lower.rs:1509-1516` (len read), `1531-1547` (contains layout) |

---

## §1 — Op: `clear(&0 mutable String)` — in-place len=0

**Decision:** Shim `__triet_string_clear(ptr)` → writes `0` to `len@0`. No
realloc, does not touch cap, does not touch bytes.

**Rationale:** `clear` is a completely safe mutation op: only sets len=0, ptr handle does
not change. No data races because exclusivity E2440 guarantees only ONE `&0 mutable` exists
at any given point in time.

**Shim signature:** `fn(ptr: i64) -> i64` (takes handle, returns 0 = Unit).

**Location:** `mir_lower.rs`, next to `__triet_string_len` (line 1509).

**Append/grow CUT — explicit rationale (for Tier D):**
- `append(&0 mutable String, suffix)` requires realloc when `len + suffix.len > cap`.
- Realloc = new `std::alloc::alloc` → new ptr → new handle.
- But `&0 mutable String` = handle i64 by value — caller retains the OLD handle (i64 value on stack).
- Callee realloc → ptr changes → new handle DOES NOT propagate back to caller → caller holds old handle
  pointing to freed memory → use-after-free.
- Tier D solution: handle-indirection (fat-pointer `{handle_ptr}`) or pointer-to-handle
  in ABI. Requires ABI redesign — outside Slice 5 scope.

---

## §2 — Exclusivity: REUSE E2440, do not rewrite

**Decision:** Exclusivity mechanisms `conflicts_with` (`checker.rs:113-124`) +
`places_conflict` (`checker.rs:507`) already exist and have been verified (Phase-0). Do not
rewrite, do not add new rules.

**E2440 rules for `&0 mutable` — two parallel tiers:**

1. **Typecheck (fatal first-line):** `borrow_check.rs` Pass-3 checks
   `forms_conflict` (ADR-0025 §2) — rejected early in typecheck before reaching MIR.

2. **MIR borrowck (defense-in-depth):** `checker.rs:113-124` `conflicts_with` —
   `BorrowExclusiveMutable` → `true` for EVERY form. Fire site `checker.rs:507-515`.

- Consequence: while a local has an active `&0 mutable` borrow → no one can create a new borrow
  on the same local (whether shared or exclusive). Guarded redundantly across at least 4 paths
  (typecheck + MIR + E2450 drop-while-borrowed), over-defended.

- This enforces the exclusivity guarantee — aliasing XOR mutability.

**TECH-DEBT (O, 2026-06-08):** Two parallel borrow-checking tiers (typecheck-era
v0.10 ADR-0025 + new MIR-borrowck) represent architectural debt. Teeth-isolation on E2440
fails due to excessive redundant guards — not incorrect (defense-in-depth), but should be
unified when Tier C closes.

---

## §3 — ABI: handle i64 by value, mutate in-place

**Decision:** `&0 mutable String` passes handle i64 by value, identical ABI to
`&0 String` (shared) and `String` (owned). Callee receives handle, mutates data at
`handle + offset`.

**No ABI change.** JIT does not need to distinguish Borrow vs MutableBorrow — same
`use_var` path.

**⚠ Realloc-dangling landmine (noted for Tier D):** Grow ops (append/push) are FORBIDDEN
in Tier C. Rationale: realloc changes ptr → caller handle points to freed memory. Tier D
solution = handle-indirection (fat-pointer containing `*mut i64` pointing to the real handle,
callee writes new handle through the pointer). This is why G noted "kills 90% of early compilers" —
fat-pointer is a comprehensive ABI change, not simply adding an op.

---

## §4 — Return-mut-borrow: CUT (retain E1042)

**Decision:** `-> &0 mutable T` continues to be blocked by E1042 (`check.rs:398-399` only
whitelists `BorrowReadOnly`). No additional action required.

**Rationale:**
1. `is_propagated` skipping E2450 currently relies on a no-nested-scope assumption (ADR-0046
   TECH-DEBT). Combining mutation + propagated loans has not been audited.
2. Return-mut-borrow introduces exclusive mutable aliases across function boundaries → requires
   auditing full dataflow (who mutates? who reads? order?). Too extensive for Slice 5.
3. E1042 already blocks this — only whitelists BorrowReadOnly, rejecting all other forms (including
   BorrowExclusiveMutable).

---

## §5 — Teeth (3 fixtures)

### 93 — clear RUN (sine-qua-non)

| Fixture | Directive | Content |
|---------|-----------|---------|
| `93_clear_run.tri` | EXPECT: 0 | `clear(&0 mutable m)` → `length(m)` = 0 |

Teeth: removing `write_unaligned(0)` in shim (turning clear into a no-op) → length=5≠0 → fails.

### 94 — mut overlap (E2440)

| Fixture | Directive | Content |
|---------|-----------|---------|
| `94_mut_overlap.tri` | ERROR: E2440 | 2× `&0 mutable m` → exclusivity violation |

Teeth: removing `BorrowExclusiveMutable => true` in `conflicts_with` → slips through → fails.

### 95 — mut vs shared conflict (E2440)

| Fixture | Directive | Content |
|---------|-----------|---------|
| `95_mut_shared_conflict.tri` | ERROR: E2440 | `&0 mutable m` + `&0 m` → conflict |

Teeth: removing check `conflicts_with` for BorrowReadOnly vs BorrowExclusiveMutable → slips through → fails.

---

## Implementation Plan

| # | Task | Primary Files | Pattern |
|---|------|---------------|---------|
| 1 | ADR → commit | `docs/decisions/0048-mutable-borrow.md` | O+G sign-off |
| 2 | Shim `__triet_string_clear` | `mir_lower.rs` (next to 1509) | `__triet_string_len` |
| 3 | Typecheck overload `clear` | `env.rs` (next to 204-349) | `length` overloads |
| 4 | Lower dispatch `clear` | `lib.rs` (next to 1316) | `contains` dispatch |
| 5 | Register shim in driver + harness | `main.rs` + `integration_tests.rs` | 1 shim × 2 |
| 6 | Fixtures 93-95 | `fixtures/` | 3 fixtures |
| 7 | Gate + commit | `scripts/gate.sh` | |

---

## Q&A

### O-Q1: Why only clear, not append?

Append requires realloc → ptr changes → caller handle points to freed memory. `clear` sets len=0,
pointer is immutable, no realloc. (§1)

### O-Q2: Does exclusivity require additional rules?

No. `conflicts_with` (`checker.rs:113`) already contains `BorrowExclusiveMutable => true`
— conflicts with ALL forms. Fire site (`checker.rs:513`) is already functional. (§2)

### G-Q1: ABI for &0 mutable?

Handle i64 by value, identical to `&0` / `&0 mutable` / `String`. JIT does not distinguish them. (§3)

### G-Q2: When will append/push be opened?

Tier D — after introducing handle-indirection/fat-pointers. Requires a comprehensive ABI redesign. (§3)

### G-Q3: Return-mut-borrow?

CUT. E1042 remains blocking. Will be reopened after re-auditing `is_propagated` × mutable. (§4)

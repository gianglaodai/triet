# ADR-0044: Integer Arithmetic Range Enforcement — Tier C Priority 1

**Status:** CLOSED — Mentor O SIGNED (semantics & soundness, 2026-06-07) + Mentor G SIGNED (layout/ABI, 2026-06-07). Implementation complete at `1fbf6ab`.
**Date:** 2026-06-07
**Author:** AI (investigation + proposal), final decision: Giang Hoang
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Trap-on-overflow for all `Integer` (27 trit) operations in the JIT layer
+ literal range check in typecheck. Closes D1, D1-literal, D2, D3.

---

## Summary

Current JIT arithmetic uses raw i64 — without enforcing ternary 27-trit range limits.
`NULL_SENTINEL = i64::MIN` is protected only by mathematical inequalities rather than
runtime mechanisms. This ADR locks: **all `Integer` operations exceeding
`[−(3²⁷−1)/2, +(3²⁷−1)/2]` → trap (panic)** — matching SPEC §3.3 verbatim:
"default **panic** — fail-fast, easily catching bugs".

Trapping is cheaper than wrapping: 1–2 cycles compared to 15–35. And trapping is locked
language semantics — requiring no temporary workarounds or convoluted arguments.

---

## §0 — Facts / Ground Truth

| # | Fact | Location |
|---|------|----------|
| F1 | `Integer` = 27 trit, range `M = (3²⁷−1)/2 ≈ ±3.81×10¹²` | SPEC §2.1 |
| F2 | JIT arithmetic raw i64: `iadd/isub/imul/sdiv` lack range checks | `mir_lower.rs:1142-1146` |
| F3 | SPEC §3.3: overflow defaults to **panic** | SPEC:502 |
| F4 | `Neg` is symmetric — `-x` of any in-range value remains in-range | SPEC §3.2 |
| F5 | `|a±b| ≤ 2M ≈ 7.6×10¹² ≪ i64::MAX` — Add/Sub DOES NOT overflow carrier | Arithmetic |
| F6 | `|a*b| ≤ M² ≈ 1.45×10²⁵ ≫ i64::MAX` — Mul OVERFLOWS carrier, requires smulhi | Arithmetic |
| F7 | `|a/b| ≤ |a| ≤ M`, `|a%b| < |b| ≤ M` — Div/Mod require no checks | Arithmetic (induction from in-range inputs) |
| F8 | Literal Integers lack range-checks — MIN passes typecheck cleanly | Probe O |
| F9 | `HashMap::insert` rejects `v == MIN` (D2) | `mir_lower.rs:1714` |
| F10 | `Long` (81 trit) does not exist in Tier A | ADR-0041 F3 |

**Inductive Invariant:** if every source generating an `Integer` enforces range bounds
(literals: E1036, BinOps: trap, shim returns: len ≤ memory ≪ M), then all inputs to
BinOps are already in-range → only the result needs validation.

---

## §1 — Decisions

### Q1: Trap or wrap?

**Trap (panic).** Rationale:
1. **SPEC §3.3:** "default **panic** — fail-fast". Trapping is the locked language
   semantics, not a temporary defense-in-depth measure.
2. **Cheaper than wrap:** 1–2 cycles (icmp + brif predicted-not-taken) vs. 15–35.
3. **Wrapping belongs to `add_and_truncate`** — an opt-in method for explicit modular
   arithmetic. When method dispatch lands (Tier C+), the balanced-modular formula
   in §A will power that method.

**Trap mechanisms — two signal families:**
- **JIT `trapnz`:** Cranelift `trapnz` (→ `ud2` → **SIGILL** (4) on x86_64, SIGTRAP
  on macOS). Used for Add/Sub/Mul in `lower_binop`. No cold block required — `trapnz`
  is a conditional trap instruction.
- **Shim `abort()`:** `std::process::abort()` (→ **SIGABRT** (6)). Used for `__triet_pow`
  and D2 reject-MIN.
- All N7 tests use `assert_n7_signal(name, status, expected_signal)` — asserting the
  exact signal of its family.

### Q2: Per-op table

| Op | Requirement | Mechanism |
|----|-------------|-----------|
| Add/Sub | Range check | `\|r\| > M` → trap. Carrier does not overflow (F5). |
| Mul | Carrier overflow + range check | `smulhi` ≠ sign-extension of `smlo` → trap (F6). Then `\|r\| > M` → trap. |
| Div/Mod | None | In-range inputs (induction) + native Cranelift div-by-zero trap. |
| Neg | Exempt | Symmetric (F4). |

Only 3 ops require code (Add/Sub/Mul), not 5.

### Q3: smulhi — soundness patch for Mul

`imul a, b` on i64 can overflow the carrier before post-checks detect it. The 128-bit
product of `a × b` has its high half in `smulhi` and low half in `smlo`. If
`smulhi ≠ sign_extension(smlo)` → overflow occurred → trap. After passing the carrier
check: `smlo` is the correct 64-bit value; range-check `|smlo| > M` → trap if exceeded.

### Q4: D2 — retain reject-MIN

Retain `HashMap::insert` trap on `v == MIN` as defense-in-depth. Cost is 1
compare/insert ≈ 0, follows the Outcome-guard precedent (guarding provably-unreachable
paths), and serves as a backstop if unspotted inductive gaps exist.

### Q5: Literal range check (D1-literal)

Typecheck: literal Integer outside `±M` → E1036 `IntegerLiteralOverflow`. Independent
concern — handled in typecheck, touching no JIT code.

### Q6: 4 debts after trap

| Debt | Action |
|------|--------|
| D1 (phantom null) | **CLOSED** — arithmetic never produces out-of-range values |
| D1-literal | **CLOSED** — E1036 in typecheck |
| D2 (HashMap reject MIN) | **RETAINED** — defense-in-depth |
| D3 (shim MIN-input) | **CLOSED** — MIN is no longer reachable |

---

## §2 — Acceptance Criteria

| # | Criterion | Verification Method |
|---|----------|-------------|
| A1 | `M + 1` → SIGABRT (trap) | N7 subprocess |
| A2 | `−M − 1` → SIGABRT (trap) | N7 subprocess |
| A3 | Mul carrier: two large in-range numbers (e.g. `M × M`) → SIGABRT, NOT garbage | N7 subprocess — key test for `smulhi` |
| A4 | Mul in-range (e.g. `1_000_000 × 1_000`) → correct value | Fixture EXPECT |
| A5 | Boundary: `M + 0`, `−M + 0`, `M + (−M)` → pass (no false traps) | Fixture EXPECT |
| A6 | Literal outside `±M` → E1036 | Fixture ERROR (compile-time) |
| A7 | D2 reject-MIN remains active | Existing N7 test |

A5 is as critical as A3: boundary off-by-one errors (`>` vs `>=`) are classic range-check
bugs — false traps at `M` reject valid programs.

---

## §3 — Implementation Plan

1. **feat(track-c): Integer range constants** — `INTEGER_MAX`/`MIN`/`RANGE` in
   `triet-core`, used across typecheck and JIT.
2. **feat(track-c): JIT trap-on-overflow** — Add/Sub: range check; Mul: smulhi
   + range check in `lower_binop`.
3. **feat(track-c): Typecheck E1036 literal overflow** — range-check literal
   Integers, rejecting outside ±M.
4. **feat(track-c): D2 update + fixtures** — retain reject-MIN (update comments
   "bounded by D1" → "defense-in-depth"), fixtures: overflow trap, literal reject,
   large Mul trap.

---

## §4 — Migration Path

| Milestone | Task |
|-----------|------|
| Tier C method dispatch | `add_and_truncate` uses balanced-modular formula (§A) |
| Tier C constant folding | Eliminate traps for compile-time-known in-range constants |
| Tier C Long (81 trit) | Distinct carrier, requires similar smulhi with 128+ width |

## §5 — Addendum (2026-06-07): Pow checked_mul

ADR-0044 §1 Q2 per-op table omitted `__triet_pow` — this shim used `wrapping_mul`
producing values outside the ternary range without trapping. This represented an
inductive loophole: pow was an Integer generator outside Add/Sub/Mul enforcement.
Both O and G missed this initially.

**Fix:** `__triet_pow` replaces `wrapping_mul` with `checked_mul` +
`std::process::abort()` on None. A8: `2 ** 100 → SIGABRT`.

---

## §A — Balanced Modular Formula (For future `add_and_truncate`)

```
M = (3²⁷−1)/2
R = 2M + 1 = 3²⁷

wrap(x) = ((x + M) % R + R) % R − M   // shift-positive → mod → shift-back
```

This formula is NOT used for default `+` — only for opt-in methods.

---

## §B — Related ADRs / Documents

| Document | Relationship |
|----------|--------------|
| SPEC §2.1 | Integer 27 trit range |
| SPEC §3.2 | Balanced ternary properties |
| SPEC §3.3 | Overflow semantics: default panic |
| ADR-0041 §6.2 | D1 — phantom null |
| ADR-0043 Q6 | D2 — HashMap reject-MIN |
| TODO.md | D1 + D1-literal + D2 + D3 |

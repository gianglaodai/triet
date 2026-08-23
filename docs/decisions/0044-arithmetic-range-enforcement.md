# ADR-0044: Integer Arithmetic Range Enforcement — Level C Priority 1

**Status:** CLOSED — Signed by Mentor O (semantics & soundness, 2026-06-07) + Signed by Mentor G (layout/ABI, 2026-06-07). Implementation complete at `1fbf6ab`.
**Date:** 2026-06-07
**Author:** AI (survey + proposal), final decision: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** Trap-on-overflow for all `Integer` (27 trit) operations at the JIT layer + literal range check at typecheck. Closes D1, D1-literal, D2, D3.

---

## Summary

Current JIT arithmetic uses raw i64 — it does not enforce the 27-trit ternary range.
`NULL_SENTINEL = i64::MIN` is protected by range inequality rather than a runtime mechanism. This ADR finalizes: **all `Integer` operations exceeding the range `[−(3²⁷−1)/2, +(3²⁷−1)/2]` → trap (panic)** — strictly adhering to SPEC §3.3: "default **panic** — fail-fast, easy bug detection".

Trapping is cheaper than wrapping: 1-2 cycles vs 15-35. Furthermore, trapping is the locked SPEC semantics — there is no need for "temporary" or "defense-in-depth" workarounds, nor for convoluted arguments.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | `Integer` = 27 trit, range `M = (3²⁷−1)/2 ≈ ±3.81×10¹²` | SPEC §2.1 |
| F2 | JIT arithmetic raw i64: `iadd/isub/imul/sdiv` lacks range checks | `mir_lower.rs:1142-1146` |
| F3 | SPEC §3.3: overflow defaults to **panic** | SPEC:502 |
| F4 | `Neg` is symmetric — `-x` for all in-range values remains in-range | SPEC §3.2 |
| F5 | `|a±b| ≤ 2M ≈ 7.6×10¹² ≪ i64::MAX` — Add/Sub does NOT overflow the carrier | arithmetic |
| F6 | `|a*b| ≤ M² ≈ 1.45×10²⁵ ≫ i64::MAX` — Mul OVERFLOWS the carrier, requires `smulhi` | arithmetic |
| F7 | `|a/b| ≤ |a| ≤ M`, `|a%b| < |b| ≤ M` — Div/Mod requires no check | arithmetic (via induction from in-range input) |
| F8 | Literal Integer lacks range-check — MIN is handled via clean typecheck | probe O |
| F9 | `HashMap::insert` rejects `v == MIN` (D2) | `mir_lower.rs:1714` |
| F10 | `Long` (81 trit) does not exist in Level A | ADR-0041 F3 |

**Induction:** If all `Integer` sources enforce the range (literal: E1036, BinOp: trap, shim return: len ≤ memory ≪ M), then all BinOp inputs are in-range → only the result needs to be checked.

---

## §1 — Decision

### Q1: Trap or wrap?

**Trap (panic).** Reasons:
1. **SPEC §3.3:** "default **panic** — fail-fast". Trapping is the correct locked language semantics, not a "temporary" or "defense-in-depth" measure.
2. **Cheaper than wrapping:** 1-2 cycles (icmp + brif predicted-not-taken) vs 15-35.
3. **Wrapping is the responsibility of `add_and_truncate`** — an opt-in method for explicit modular arithmetic. When method dispatch is available (Level C+), the balanced-modular formula in §A will be used for that method.

**Trap mechanism — two signal families:**
- **JIT `trapnz`:** Cranelift `trapnz` (→ `ud2` → **SIGILL** (4) on x86_64, SIGTRAP on macOS). Used for Add/Sub/Mul in `lower_binop`. No cold block is required — `trapnz` is a conditional trap instruction.
- **Shim `abort()`:** `std::process::abort()` (→ **SIGABRT** (

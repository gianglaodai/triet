# ADR-0041: Nullable (`T?`) Representation — Level A

**Status:** CLOSED — SIGNED by Mentor O (semantics & soundness, 2026-06-06) + SIGNED by Mentor G (layout/ABI/codegen, 2026-06-07). Implementation Steps 1-4 verified, 43 fixtures, 1070 tests, 0 warnings. Implementation complete at `28c1a5f`.
**Date:** 2026-06-06
**Author:** AI (survey + proposal), final decision: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** pure `T?` only. DOES NOT touch `T~E` / `T?~E` (Outcome requires packed ABI, deferred to Level C — current guard is at `triet-jit/src/mir_lower.rs:758-789`).

---

## Summary

Decision on the runtime representation for `T?` at Level A ("every value = 1×i64"), to unlock the builtin `get(vector, index) -> Integer?` — the first consumer of nullables in the new backend. This ADR presents **6 options** (PA-1 … PA-6) with soundness analysis for each. After two review rounds, both mentors finalized **PA-3c (uniform sentinel)**: `NULL_SENTINEL = i64::MIN` for **all** `T?` — both scalar and heap. Accompanied by: a read-shim trap-on-0 (defense-in-depth for heap), a canary N1 tied to `triet_core::Integer::MIN`, and an addendum to ADR-0001 to correct both the trit assignment table and the `T??` clause.

## Motivation

1. **`get` is the next gateway.** Level A Vector (4.3b) has `push`/`len` but DOES NOT have a way to read elements. `get` must be total (no panic — safety contract `feedback_explicit_strictness`: property access 100% safe), meaning it must return `Integer?`. Without a representation for `Integer?`, there is no `get`.
2. **ADR-0039 (`?`-family) design is locked but implementation is deferred** because "the Backend cannot yet lower `?.`" — every operator with `?` is waiting for this representation.
3. **ADR-0040 §6 flagged a pending debt:** "Nullable String: **representation not yet designed.** Note the sentinel-0 conflict (moved-out ≡ null value)." This ADR settles that debt — and chooses a definitive solution: uniform MIN so that moved-out (0) and null (MIN) **never overlap**.

---



## §0 — Verified Facts (no speculation)

| # | Fact | Location | Design Implication |
|---|------|----------|--------------------|
| F1 | `Integer` = 27 trit, range `±3_812_798_742_493` ≈ ±2^41.8 | `triet-core/src/integer.rs:39-42` | Carrier i64 is ~4 million times wider than the valid range → provides a massive "niche" for a sentinel |
| F2 | `Tryte` range `±9_841`, `Trit` `±1`, `Trilean` 3 values | `triet-core/src/tryte.rs:39-42`, `trit.rs` | Every Triết scalar has a niche within i64 |
| F3 | `Long` = 81 trit, MAX ≈ 2.2×10³⁸ — **does not fit in i64** | `triet-core/src/long.rs:53-54` | Long has never been a valid Level A value → `Long?` is unconditionally deferred |
| F4 | JIT arithmetic is **raw i64**: `BinOp::Add → iadd`, `Mul → imul`, `__triet_pow` uses `wrapping_mul` | `triet-jit/src/mir_lower.rs:1124-1126,1251-1270` | Ternary range is NOT enforced at runtime → the niche is not "guarded" (debt D1, §6.2) |
| F5 | Heap value = 1×i64 body_ptr; moved-out can be zeroed (M1-M4); `free(0)` = no-op | ADR-0040 §1.3, §2.5; `mir_lower.rs` Drop handler | `free(0)` no-op remains UNCHANGED — Drop of a moved-out value must be safe (C4) |
| F6 | Compiled Enum: `StackSlot` + `EnumLayout` (disc i64@0, payload@8), match works (fixtures 25-32) | `mir_lower.rs:168-173,511-546`; ADR-0037 | Tagged-union machinery is available if a two-word representation is chosen |
| F7 | Shim C ABI: fixed digital signature `fn_1_0/fn_1_1/fn_2_1` — returns **exactly 1×i64** | `triet-driver/src/main.rs:123-141` | Shim cannot return a 2-word value; requires an out-parameter to do so |
| F8 | Typecheck already has `Type::Nullable(Box<T>)` + widening `T ⊂ T?`; `Type::Outcome{allow_null_state}` is separate | `triet-typecheck/src/types.rs:44,97-104,165-203` | Frontend is ready; only lowering + representation are missing |
| F9 | MIR type is a **string** (`LocalDecl::ty: String`); `is_copy` defaults to Move for unknown; canonical `is_vec_type()` in triet-mir (lesson 4.3c) | `triet-mir/src/lib.rs:163-174,2047-2073` | `"Integer?"` will fall into default-Move unless a rule is added — must have a canonical `is_nullable_type()`. **Collision:** `is_vec_type("Vector<Integer>?")` = true (§5.1) |
| F10 | MIR has `OutcomeDiscriminant/OutcomeUnwrap/OutcomeUnwrapError` but JIT **rejects** them (not yet reachable) | `triet-mir/src/lib.rs:245-274`; `mir_lower.rs:758-789` | MIR-level parking exists if specific statements are needed; current guard is not removed by this ADR |
| F11 | Discriminator semantics LOCKED: `Trit::Positive` = value, `Trit::Zero` = null, `Trit::

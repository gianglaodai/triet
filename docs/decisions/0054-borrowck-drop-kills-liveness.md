# ADR-0054 — Core-Borrowck-Patch: Drop Kills Liveness (Use-After-Drop → E2421 UseAfterStorageEnd)

- **Status:** 🔒 LOCKED — G sign-off 2026-06-11. ⛔ RED ALERT (critical foundation soundness hole).
- **Date:** 2026-06-11
- **Author:** Mentor O. Front decisions + §7 finalized by G (2026-06-11).
- **Signatures:** O ✅ (grounded from HP.0 spike) · G ✅ (approved 2026-06-11).
- **Related:** [ADR-0053 §9](0053-heap-payload-outcome.md) (HP.0 spike exposed hole), ADR-0025 (E24XX borrowck).

---

## 1. Context — Red Alert from HP.0

The HP.0 spike (ADR-0053 §9.3) revealed: when MIR emits `Drop(x)` followed immediately by `move x` into another variable,
borrowck reported **"OK (no borrow errors)"**. Independent teeth (WITHOUT involving Outcome) confirmed this at the foundation:

```rust
// hand-built MIR Body, s: String (Move type)
Drop(s)                      // frees String
assign(other, s)             // moves s AFTER drop
// check_body(&body).errors == []   ← BLIND. Must be E2420/E2421.
```

This is **NOT an Ergonomics or Outcome bug** — it is a **foundational soundness hole in NLL borrowck**, breaking
**ALL Move types** (String/Vector/HashMap), regardless of Outcome. A compiler that misses use-after-Drop is a
compiler that corrupts memory. It must be patched BEFORE any heap-payload work (HP.4 halted — ADR-0053 §9.4).

## 2. Root Cause (Measured from Code, NOT Guessed)

`VarState` (`checker.rs:134-145`) has 3 states:
- `Owned` — usable.
- `Moved` — already moved → **any use is E2420**.
- `Ended` — storage ended by Drop/StorageDead. Doc (lines 140-143): *"Return can still consume,
  but **any other use is E2420**. Separated from `Moved` so that E2450 checks at Return do not trigger false-positive E2420."*

The `Drop(l)` handler (`checker.rs:720-722`) **correctly sets** `var_states[l] = Ended` (if not already `Moved`).
**The flaw:** use-sites (Assign-source move-check, BinOp operands, …) **ONLY** treat `Moved` as E2420,
**completely ignoring `Ended`**. ⟹ the contract "`Ended` + use ⟹ E2420" was **written in the doc-comments but NEVER enforced**.
Drop set Ended, but nothing blocked move-of-Ended. That is the entire monster.

## 3. Decision (Finalized by G — Locked)

1. **Enforce `Ended` contract with a NEW ERROR CODE E2421 (G decision):** every **read/move/borrow** of an
   `Ended` **Move-type** local (post-Drop) → **E2421 UseAfterStorageEnd** (new error code, NOT merged with E2420).
   Drop = **liveness kill** operation. Two distinct mental models: E2420 = "transferred to another owner" (active);
   **E2421 = "lifetime destroyed/ended, cannot resurrect from the dead"** (lifecycle/automatic). Requires variant
   `BorrowError::UseAfterStorageEnd` + `#[diagnostic(code(triet::borrow::E2421))]`.
2. **Preserve Return EXCEPTION:** terminator `Return` consuming an `Ended` local remains VALID (no E2421) —
   this is the exact reason `Ended` was separated from `Moved` originally. MUST NOT be broken (otherwise
   false-positive Return-of-dropped errors arise). Fix must distinguish **ordinary use (E2421)** vs **Return-consume (OK)**.
3. **Scope: Move types ONLY (G decision).** Copy types (scalar, non-heap): Drop is a no-op, data remains
   safe on the stack, NO UAF risk → **DO NOT enforce** (constraining Copy creates noisy false-positives, making
   the language pointlessly rigid). E2421 targets Move-only types (String/Vector/HashMap/Structs containing them).
   Fixed in borrowck core (`checker.rs` use-site checks), WITHOUT touching lowerer/MIR.

   > **§3 Amendment (2026-06-11, D implementation):** Variant named `UseAfterStorageEnd` rather than
   > `UseAfterDrop` as in G's signed draft. Rationale: `VarState::Ended` is set by both `Drop` and
   > `StorageDead` — the name `UseAfterStorageEnd` accurately covers both sources. Approved by G.

## 4. Teeth (Mandatory Red→Green)

- **T1 (Entry point, O constructed & proved `got: []`):** Hand-built `Body { Drop(s:String); assign(other, s) }`
  → `check_body` MUST emit **`E2421 UseAfterStorageEnd`**. Currently blind → fails. (Test `drop_then_move_must_be_rejected`.)
- **T2 (Return exception does not break):** `Body { ... Drop(s); Return([s]) }` or valid move-then-Return cases
  — MUST REMAIN GREEN (no false-positive E2421). All 20 existing borrowck tests remain green post-fix.
- **T2b (Copy types not over-rejected):** `Body { Drop(n:Integer); assign(other, n) }` — Copy type, Drop=no-op
  → MUST BE GREEN (NO E2421). Proves scope is restricted to Move-types.
- **T3 (Post-fix regression, tied to ADR-0053):** Once F1 (heap-aware desugaring) is fixed, HP.0 map chain
  `~+> |v| v` (String) no longer does Drop-then-move → borrowck passes PROPERLY (not blindly).

## 5. Execution Order (Finalized by G)
1. **ADR-0054 (this front) FIRST** — patch borrowck core, T1 red→green, T2 remains green.
2. Then resume ADR-0053 HP.1→HP.4 (heap payload Outcome). HP.4 (map) requires F1 desugar + T3.
3. Assigned to: D — bar = T1 green + T2 intact + raw gate clean.

## 6. Consequences
- **Positive:** Closes foundational soundness hole; all Move types are safe from use-after-Drop; paves the way for heap Outcomes.
- **Fix Risks:** Subtle boundary between `Ended`-use (E2421) vs `Ended`-Return (OK) — prone to false-positive
  Return errors if enforced coarsely. Teeth T2 serves as safety net. May expose existing sites implicitly relying on
  `Ended` leniency (counted during fix).
- **No ABI/Lowerer Changes:** Pure borrowck fix — does not touch JIT/MIR shapes.

## 7. G Ruling (LOCKED 2026-06-11 — closes §7)
1. **Error Code: SEPARATE NEW CODE E2421 (UseAfterStorageEnd).** DO NOT merge with E2420. Two entirely distinct
   mental models: E2420 "transferred ownership to another party, rights lost" (active) vs E2421 "lifetime destroyed/ended,
   cannot resurrect dead data" (lifecycle/automatic). A quality compiler matches user mental models precisely.
2. **Scope: Move types ONLY.** Copy types have no-op Drops, data is safe on the stack, no UAF → restricting
   Copy produces useless false positives, making the language rigid and unhelpful. E2421 strictly targets Move-only.

**§7 closed. ADR-0054 signed and approved by G — Foundation Patch Campaign begins.**

## 8. Operational Directive for Implementer (G)
"Enter `checker.rs`, seal the `VarState::Ended` hole, emit **E2421** upon violation, and PRESERVE
`Return-leniency` so that 20 legacy tests do not break. Ensure Mentor O's MIR test (`drop_then_move_must_be_rejected`)
fails loudly with E2421." O Gatekeeper Bar: T1 red-with-E2421 → green · T2/T2b intact · RAW gate clean. Steel protocol applies.

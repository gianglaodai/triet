# ADR-0040: Heap Aggregate Value Model & Layout — Tier A

**Status:** Draft v4 (v4 review)
**Date:** 2026-06-05
**Author:** Giang Hoàng (value model, semantics), AI (layout, MIR shims, verification)
**Reviewers:** Mentor G (layout, ABI, runtime codegen), Mentor O (semantics, soundness)
**Changes v3→v4:** §1.3 added M4 (Return-escape). §3.2 added Return-escape mechanism. §3.5 fixed loop leak example. §3.7 fixed citations (754-757). §4 added B7 refusal (heap via user-fn boundary). §5 added B7 steps. §7 fixed fixture 35/36 (return len).

---

## Summary

Deciding the value model and memory layout for heap aggregates (String, Vector, HashMap) at Tier A. Finalized move-only owned as the default semantic for slice 1 (String, Vector), with `ObjectHeader` reserved in the layout but refcounting not yet enabled. Runtime safety mechanisms: **Zeroing-on-move** (JIT writes null to the source at **every move-site**) + **Null-guard-free** (Drop check `ptr != 0` before calling the free shim) + **Return-escape** (JIT skips Drop for locals contained within Return values). Four types of move-sites: `Assign` (M1), let-Move-type→Assign (M2), `CallDispatch` consume-arg (M3), and Return-values escape (M4). All operations occur via `extern "C"` shims following the `__triet_pow` precedent.

## Motivation

1. **Copy/Move type-aware borrowck is closed** (HEAD `6e2843c`) — borrowck now distinguishes between Copy vs. Move types, enforcing single-owner move semantics. This provides the safety infrastructure for heap types with actual destructors.
2. **F1 gap is closed** — Sticky "Moved" during Drop → Return now triggers E2420. Heap locals moved into a payload and subsequently reused will be detected.
3. **String/Vector/HashMap are currently vulnerabilities** — The lowerer returns `Err` for all aggregate types. The layout must be finalized before implementing lowering.

---

## §1 — Value Model (Author decision, two mentor inputs)

### 1.1 — Move-only owned for Tier A

**Decision:** String and Vector are **move-only owned** — single-owner, no implicit copy, no implicit clone.

Three facts verified from the code (not speculation):

| Fact | Location | Significance |
|---------|--------|---------|
| Copy/Move borrowck enforces single-owner | `triet-borrowck/src/checker.rs:586-589` (Δ1), `683-688` (Δ2) | Move types are marked Moved upon assignment and cannot be reused → ensures single-owner safety |
| `ObjectHeader` is LIVE, but has no consumer | `triet-core/src/memory.rs:51-58` | `refcount: AtomicU32` + `reserved: AtomicU32`, `repr(C, align(8))` — already defined and tested |
| `&+` strong forms are not yet lowered | `triet-jit/src/mir_lower.rs` — no code path for borrow lowering | No one is incrementing/decrementing refcounts currently |

**Consequences:** Refcount is dead code if enabled immediately — there is no producer (strong form lowering) and no consumer (`Drop::decrement`). Move-only semantics leverage the **entirety** of the recently built borrowck infrastructure without adding overhead.

### 1.2 — ObjectHeader reserved, refcount = 1 (no inc/dec)

The heap object layout uses the full `ObjectHeader`, but for Tier A:

- `refcount` is initialized to 1 (compatible with `ObjectHeader::new()`)
- **No increment** (no `&+ T` lowering yet)
- **No decrement** (Drop calls `free` directly, bypassing the refcount→0 check)
- `reserved` = 0 (reserved for drop flags / type tags in Tier B/C)

**Migration path:** When `&+ T` lowering arrives (Tier B/C), using the same layout, we only need to:
1. Lower `&+ T` → call `ObjectHeader::increment()`
2. Drop with refcount > 1 → decrement; refcount = 1 → actual free
3. **Layout remains unchanged** — backward binary compatibility.

### 1.3 — Drop semantics: Zeroing-on-Move + Null-guard-free + Return-escape

**Problem:** Borrowck is a static analysis — it does not modify MIR. The lowerer generates `Statement::Drop` for **all** owned locals at the end of a scope, regardless of whether that local has been moved on certain control-flow paths. Drop-on-Moved is permitted by design (F1 teaches: Return accepts Ended, does not reject Moved).

For Copy types, Drop = no-op (stack primitive). However, for Move types (heap), if the JIT emits `free(ptr)` unconditionally for every Drop — then double-free and dangling pointers will appear at ownership boundaries where a null guard is insufficient.

The claim "sticky-Moved ensures Drop does not run" in v1 is **incorrect** — sticky-Moved only affects the Return check (E2420) and the transition of `VarState`; it does not remove `Statement::Drop` from the MIR.

**Decision — Four types of move-sites, JIT zeros or skips at each type:**

There are **four** ownership boundaries at runtime that the JIT must handle:

| # | Move-site | Mechanism | Specification |
|---|-----------|-----------|---------------|
| M1 | `Statement::Assign` plain-source Move-type | After copying the value to the destination, store 0 in the source variable | §3.7 |
| M2 | `let b = a` where `a` is a Move type | Lowerer emits `Assign` instead of an alias local (§3.7); JIT zeros as in M1 | §3.7 |
| M3 | `CallDispatch` argument in a consuming position | JIT zeros the variable after the call, using the shared `BuiltinShimMeta` table (§3.6) | §3.6 |
| M4 | `Return(values)` — values escape the function | JIT skips Drop for locals $\in$ values (§3.2) | §3.2 |

**M1 — Assign (already present in infrastructure):**

1. JIT codegen for `Statement::Assign` with a Move-type source:
   - Copy the `i64` value from source to destination (standard)
   - **Store 0 in the source variable** (null pointer)
   - JIT identifies the type via `body.local_decls[source.local.0].ty` $\to$ `triet_mir::is_copy`

**JIT codegen for `Statement::Drop` with a Move-type local:**

- If the local is part of the Return values of the current block $\to$ **skip** (M4, §3.2)
- Otherwise: call `call __triet_<type>_free(ptr)` — the shim handles the null guard.
  In Tier A, the null guard resides in the shim (`if ptr == 0 { return; }`), not in the JIT codegen. A JIT-side null-check branch is a Tier B optimization (to avoid call overhead for null pointers).

**`__triet_<type>_free` shim receives `ptr: i64`:**

- `if ptr == 0 { return; }` — shim-level null guard (Tier A)
- Calculate `header_ptr = ptr - 8`, then free the entire allocation.

**Why not use the reserved field as a drop flag:** The reserved field is on the heap and requires a memory load to check. Null-on-move uses the stack value itself (already present in a register/Cranelift Variable) — it is cheaper than one memory load and avoids touching the `ObjectHeader` for drop-tracking purposes. The reserved field remains untouched for Tier B/C.

**Borrowck does NOT change** — sticky-Moved + E2420 + E2450 remain unchanged. Zeroing-on-Move + Return-escape are **supplementary runtime mechanisms** for static analysis, not replacements.

### 1.4 — Why not implement refcounting immediately

| | Immediate Refcount (Tier A) | Move-only (Tier A) |
|---|---|---|
| Number of shims to write | `alloc`, `increment`, `decrement`, `free` | `alloc`, `free` |
| Producer of increment | None (`&+` not yet lowered) | Not required |
| Consumer of decrement | Drop with refcount check | Drop = direct free (with null guard) |
| Lines of dead code | ~

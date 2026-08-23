# ADR-0060 — Nested Aggregate Layout (P2): struct-in-struct sizing + JIT nested projection

- **Status:** 🔒 LOCKED — Implemented (O+G approved 2026-06-12). Drafted by Mentor O 2026-06-12, grounded from probe driver-run + line-cite + separation of P1/P2.
- **Date:** 2/6/2026
- **Drafted by:** Mentor O (probe `a.b.c` hits JIT bottom; separating P2 nested-aggregate from P1 sub-8B packing).
- **Signatures:** O ✅ (root cause proven via driver-run; flat-struct sound / nested-broken measured directly; P1/P2 separation) · G ✅ (approved scope 3 points 2026-06-12 — revoked P1 value-model destruction order after YAGNI analysis, keeping Group E locked).
- **Related:** [phase10-native-struct-layout.md](../../spec/plans/phase10-native-struct-layout.md) (P1 sub-8B packing — Group E sealed, KEEP LOCKED), [ADR-0049](0049-fat-pointer-abi.md) (String 3-word fat-pointer copy — multi-word precedent), [ADR-0057](0057-jit-outcome-slot-move.md) (Outcome slot-move word-by-word — multi-word precedent), [ADR-0050](0050-mir-type-enum.md) (MirType — Struct/Enum bare).

---

## 1. Context — `a.b.c` broken; flat struct sound; P1 ≠ P2

`a.b.c` (nested field access) is significant technical debt (TODO Phase 4 line 7). Probe O 2026-06-12 measured via `triet-driver run`, separating two layers that were previously incorrectly merged:

| Layer | Nature | Hits value-model? | Use-case | Decision |
|---|---|---|---|---|
| **P1 — Sub-8B packing** | field `Trit`(1B)/`Tryte`(2B) at actual offset → `stack_load(I64)` overflow read | **YES** (14 loads + 21 stores I64→typed-width + extend) | **0 fixture** | **KEEP LOCKED** (Group E, phase10) |
| **P2 — Nested aggregate** | Struct/Enum type field assigned 8B → store overflow / data loss | **NO** (leaf=Integer 8B, I64 is correct) | `a.b.c` (Integer) actual | **THIS ADR** |

**Measurement: what is SOUND today (MUST NOT regress):**
- Flat multi-field struct: `Point{x,y}; p.x+p.y` → 3. Whole-copy `let p2=p; p2.x+p2.y` → 3.
  Param by-pointer `sum(p:Point)` → 3. 3-field `t.a+t.b+t.c` → 6. **All green.**

**Measurement: what is BROKEN (only nested aggregate):**
- `Outer{inner:Inner, tag}; o.inner.x` → CHECK OK (lower+borrowck consumes nested projection),
  **RUN: `JIT unsupported: nested projections not/not supported`** (`mir_lower.rs:272` load + `:381` store).
- `Outer{inner:i, tag:7}; o.tag` → **7 (runs BUT fails silently)**: construction only copies
  1 word of Inner (loses `i.y`); tag@8 remains intact because inner is under-sized 8B, masking the error.

## 2. Rationale — MEASURED FROM CODE, three points

1. **Layout under-size field aggregate.** `triet-lower/src/lib.rs:466` hardcodes
   `(f.name, ty, 8, 8)` — EVERY field 8B including struct-type fields. `Outer{inner:Inner(16B)}` →
   inner assigned 8B (missing 8B). `Struct

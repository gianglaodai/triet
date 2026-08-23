# ADR-0060 — Nested Aggregate Layout (P2): struct-in-struct sizing + JIT nested projection

- **Status:** 🔒 LOCKED — Implemented (Approved by O+G on 2026-06-12). Drafted by Mentor O on 2026-06-12, grounded in driver-run probes + line citations + P1/P2 separation.
- **Date:** 2026-06-12
- **Drafted by:** Mentor O (probe `a.b.c` hit bottom in JIT; separated P2 nested-aggregate from P1 sub-8B packing).
- **Signatures:** O ✅ (root cause proven via driver-run; flat-struct sound / nested-broken measured directly; separated P1/P2) · G ✅ (approved 3-point scope 2026-06-12 — withdrew order to dismantle P1 value-model after YAGNI analysis, kept Group E locked).
- **Related:** [phase10-native-struct-layout.md](../../spec/plans/phase10-native-struct-layout.md) (P1 sub-8B packing — Group E sealed, KEPT LOCKED), [ADR-0049](0049-fat-pointer-abi.md) (String 3-word fat-pointer copy — multi-word precedent), [ADR-0057](0057-jit-outcome-slot-move.md) (Outcome slot-move word-by-word — multi-word precedent), [ADR-0050](0050-mir-type-enum.md) (MirType — Struct/Enum bare).

---

## 1. Context — `a.b.c` Broken; Flat Struct Sound; P1 ≠ P2

`a.b.c` (nested field access) is a genuine foundational debt (TODO Phase 4 line 7). Mentor O's probe on 2026-06-12 measured using `triet-driver run`, cleanly separating two tiers that had previously been conflated:

| Tier | Nature | Touches Value-Model? | Use-Case | Decision |
|---|---|---|---|---|
| **P1 — Sub-8B packing** | `Trit` (1B) / `Tryte` (2B) fields at real offsets → `stack_load(I64)` reads out of bounds | **YES** (14 loads + 21 stores I64 → typed-width + extend) | **0 fixtures** | **KEPT LOCKED** (Group E, phase10) |
| **P2 — Nested aggregate** | Struct/Enum-typed fields allocated only 8B → store overflow / data loss | **NO** (leaves are 8B Integer, I64 is correct) | `a.b.c` (Integer) real | **THIS ADR** |

**Measured: What is SOUND today (MUST NOT regress):**
- Flat multi-field structs: `Point{x,y}; p.x+p.y` → 3. Whole-copy `let p2=p; p2.x+p2.y` → 3. By-pointer parameter `sum(p:Point)` → 3. 3-field `t.a+t.b+t.c` → 6. **All green.**

**Measured: What is BROKEN (only nested aggregates):**
- `Outer{inner:Inner, tag}; o.inner.x` → CHECK OK (lowerer + borrowck consume nested projection), **RUN: `JIT unsupported: nested projections not supported`** (`mir_lower.rs:272` load + `:381` store).
- `Outer{inner:i, tag:7}; o.tag` → **7 (runs BUT fails silently)**: construction copies only 1 word of Inner (losing `i.y`); tag@8 remains intact only because inner was under-sized to 8B, masking the error.

## 2. Root Cause — MEASURED FROM CODE, Three Points

1. **Layout under-sizes aggregate fields.** `triet-lower/src/lib.rs:466` hardcodes `(f.name, ty, 8, 8)` — EVERY field is treated as 8B, even struct-typed fields. `Outer{inner:Inner(16B)}` → inner receives 8B (missing 8B). `StructLayout::compute` (mir) correctly accumulates `offset += size` according to input, but the input was false.
2. **JIT nested projection hard-blocked.** `mir_lower.rs:272`/`:381`: `if projection.len() != 1 { Err("nested projections not supported") }`. The field-offset calculation ITSELF was clean (using `field.offset`, not `index*8` — phase10 Q2).
3. **Construction copies 1-word.** Default `Statement::Assign` (`mir_lower.rs:1137-1139`): `val = load_place(source); store_place(dest, val)` — a single i64. Struct-typed fields (≥2 words) lose all words beyond the first.

## 3. Decision (G Approved Scope 2026-06-12). ONE Campaign, Three Points.

**Fix nested aggregates COMPLETELY within the i64-uniform value model. Leaves remain 8B Integer → `stack_load(I64)` is preserved. P1 (sub-8B packing) REMAINS LOCKED.**

### Point 1 — Layout Sizing (Lowerer)
`lib.rs:466`: fields of type `MirType::Struct(name)`/`Enum(name)` → `size = struct_map[name].total_size` (multiples of 8 when leaves are Integer), `align = 8`. Primitive fields remain `8, 8` (DO NOT touch sub-8B = P1). `struct_map` is already available at `lower:472`. Requires topological ordering (nested definitions before outer) or a 2-pass approach — determined during implementation probing.

### Point 2 — JIT Nested Offset-Walk (`load_place` + `store_place`)
Remove the `projection.len() != 1` block. Walk the projection chain: start with type = `local.ty`, for each `Field(name)` → locate field in current layout, **accumulate `field.offset`**, descend into `field.ty`'s layout (consulting `body.struct_layouts`). Leaf load/store remains `I64` at the total accumulated offset.

### Point 3 — Multi-Word Copy for Field-Aggregate Construction/Assignment
When Assign destination or source is an aggregate-typed field/local (≥2 words): copy word-by-word `while off < size { stack_load(I64, src, base_src+off); stack_store(dest, base_dest+off); off+=8 }`. **REUSE PRECEDENTS:** Outcome slot-move (`mir_lower.rs:1127-1132`) + String 3-word (`:1140-1156`, `:921-930`). `size` = aggregate layout `total_size`.

## 4. Teeth (Boundary of Life and Death) — Route-Lower via `lower_source`/driver-run, FORBID Hand-Building MirBuilder

### Positive (New fixtures, sequential number = max+1, D checks `ls fixtures`)
- `Outer{inner:Inner{x,y}, tag}; return o.inner.x + o.inner.y + o.tag` → correct value (reads nested 2 levels + flat field in same struct).
- Nested write: `o.inner.x = 5; return o.inner.x`.
- **No-regress flat:** existing flat-struct fixtures remain green (Point / parameters / 3-field).

### Poison (One lethal poison per point, measured directly)
- **Point 1 poison:** revert `lib.rs:466` to hardcoded `8` for aggregate fields → `o.inner.y` returns an **incorrect value** (overwriting tag / data loss) OR wrong layout → shifted values. Test must fail.
- **Point 2 poison:** revert unblocking → `JIT unsupported: nested projections not supported` returns. Test fails.
- **Point 3 poison:** force construction to copy 1-word (remove while loop) → `o.inner.y` is lost (returns garbage/0). Test fails.
- ⚠️ Distinction: these are **incorrect-value** teeth (observable via mismatched results), NOT requiring SIGABRT. If poison overflows slot and overwrites subsequent field → produces a specific wrong value; teeth catch it via EXPECT of exact number. Restore via `cp` `/tmp`, STRICTLY FORBID git checkout.

## 5. OUT OF SCOPE (KEPT LOCKED)
- **P1 sub-8B packing** (Trit 1B / Tryte 2B in struct) — Group E sealed. 0 fixtures. Value-model load-width UNCHANGED. Unlocked only if real Trit-in-struct fixtures + ADR byte-size mapping are authored.
- **Tuple types** — no syntax exists yet. Native-pack Outcome (C4) — 0 producers.
- Aggregate fields of **heap** type (String/Vector in struct) — already blocked at `lib.rs:2503-2510` B8 (`is_copy` reject). Move-type-in-struct is a separate campaign (drop/ownership), not P2.

## 6. Consequences
- (+) `a.b.c` nested ≥2 levels runs correctly, closing Phase 4 debt. Future tuples/enum-payload-structs reuse the offset-walk + multi-word copy mechanism.
- (+) The i64-uniform value model IS PRESERVED — minimal blast radius (`lower:466` + 2 JIT functions + Assign), WITHOUT touching the 35 I64-width sites of P1.
- (−) Multi-word copy increases instruction count for aggregate assignment — acceptable (proportional to size).
- (−) Enum payloads containing structs + nested across param/sret require offset-walk propagation to callee field-loads (`mir_lower.rs:921-952`, `:706-712`) — **implementation probing must confirm** nothing is missed.

## 7. Operational Directives
1. **Implementer proceeds** along the 3 points; may split into 2 slices (nested read first: points 1+2+3-construct → `o.inner.x` correct; then nested-write store-walk) — implementer proposes, O approves slice.
2. Submit to O for review + **raw gate first line** (auto-reject if not raw) → **O manually tests teeth BEFORE commit** (poison 3 points, measure wrong-value/JIT-error on FINAL code) → G signs → commit. **NO skipping steps** (lesson from C.1).
3. Every slice: update TODO.md + handoff. Explicitly record that P1 remains locked.

## 8. Amendment: P2-Boundary (B+C) Defuses Mines in §6

Red flags planted in §6 (sret & enum-payload) were confirmed broken (driver-run 2026-06-12) and HAVE BEEN RESOLVED.
Crucially: B (sret) and C (enum payload) have TWO DISTINCT root causes, despite sharing the symptom `has no slot`.
- **B (Sret nested return):** Broken because destination local `_0` (return pointer) lacked `struct_slot`. Solution: Separate `resolve_addr` per-side, provide pointer-fallback for block copy.
- **C (Enum payload struct):** Resolved via modifications in `lowerer StructAlloc`.
Both were independently verified via poison. P2 is officially closed completely (local + sret + enum-payload). Signatures: O ✅ G ✅.

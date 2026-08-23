# ADR-0065 — Nullable Aggregate: `Enum?` & `Struct?` (Nullable Stack-Slot)

- **Status:** 🔒 LOCKED — O+G sign-off 2026-06-20, Giang finalized direction (Option A for Struct? + 2 construction slices + sent WO Slice 1 to D). Drafted by Mentor O, grounded from MIR/JIT line citations.
- **Date:** 2026-06-20
- **Author:** Mentor O (dissected Enum/Struct repr in `triet-mir`/`triet-jit` + compared niche disc + separated Enum?/Struct? by underlying repr root).
- **Signatures:** O ✅ (repr grounded, Enum/Struct asymmetry measured directly file:line; rejected box/niche-fill with justification) · G ✅ (signed off ADR + WO Slice 1, 2026-06-20) · Giang ✅ (finalized Option A + 2 slices Enum?→Struct?).
- **Related:** [ADR-0062](0062-heap-nullable-ptr-sentinel-repr.md) (ptr-sentinel String/Vector/HashMap — THIS ADR extends "sentinel in one cell" to disc/tag cells; §6 ADR-0062 deferral of Struct?/Enum? is the exact debt this ADR resolves) · [ADR-0041](0041-nullable-representation-bac-a.md) (scalar `T?` PA-3c `i64::MIN`; `NULL_SENTINEL` constant + canary N1) · [ADR-0040](0040-heap-aggregate-layout.md) §4 (**B8** — heap field/payload in aggregates refused; allocator barrier for this ADR) · [ADR-0037](0037-enum-tagged-union-layout.md) (EnumLayout disc@0/payload@8) · [ADR-0060](0060-nested-aggregate-layout.md) (multi-word copy + nested offset-walk — precedent for Struct? tag-prepend) · [ADR-0057](0057-jit-outcome-slot-move.md) (slot `{disc, payload}` + word-by-word slot-move — precedent for tag-words).

---

## 1. Context — Debt from ADR-0062 §6, Aggregate Repr Needs Completion

`T?` already operates for scalars (ADR-0041) and heap String/Vector/HashMap (ADR-0062). Both rely on **a single invariant**: null ⟺ a **pointer-carrying i64 cell** == `NULL_SENTINEL` (`i64::MIN`). Aggregates (`Struct`/`Enum`) were **transparently deferred** in ADR-0062 §6 because "there is no natural `ptr` cell to plant a sentinel" — and `Body::verify()` (`triet-mir:1478-1519`) rejects them (`HeapNullableNotLowered`), with comments at `triet-mir:1397-1398` explicitly stating "stay refused until a later ADR". **ADR-0065 is that ADR.**

Recon (measured from code, not guessed) reveals a **fundamental asymmetry that determines everything**:

| Type | Current Repr (file:line) | Has a cell to plant a sentinel? |
|---|---|---|
| **Enum** | `EnumLayout` disc**@0** (i64, 8B) + payload@8; `discriminant_value: i64` ∈ {0,1,2,…} (`triet-mir:1097-1120`, `compute():1158-1166`) | **YES** — disc@0 cell is a full i64 containing small non-negative integers → **massive niche** |
| **Struct** | `StructLayout` = N inline fields {offset, size}, NO disc, NO ptr cell (`triet-mir:1064-1090`) | **NO** — pure data, no spare cells |

→ **Enum? = low-hanging fruit** (niche inside an existing cell). **Struct? = requires adding a tag cell.** Two distinct foundations → one overarching ADR (the complete picture), two execution slices.

## 2. Decision

**Unified System Invariant for All Nullables** (extending ADR-0062 §2):

> **`tag_cell == NULL_SENTINEL` (`i64::MIN`) ⟺ null.** `tag_cell` is:
> - `ptr` cell (heap — ADR-0062): String slot[0] / Vector·HashMap handle;
> - **`disc@0`** cell (Enum? — niche, THIS ADR);
> - **`tag@0`** cell (Struct? — disc-word prepend, THIS ADR).
>
> Null-check = **ONE load + ONE `icmp eq i64::MIN`** on `tag_cell`. NEVER memcmp the entire slot. FORBIDDEN to use `tag_cell == 0` for null (0 = uninit/dead — defense-in-depth ADR-0041 §6.1).

### 2.1 `Enum?` — Discriminant-Sentinel (Niche, 0-Byte Overhead)
Reuses the existing disc@0 cell. Actual discriminants ∈ {0,1,2,…} never collide with `i64::MIN`. `disc@0 == i64::MIN` ⟺ null; otherwise == valid variant. **Widening `Enum → Enum?` = NO-OP** (discriminant is already a valid value ≠ sentinel). This is IDENTICAL to ADR-0062 ptr-sentinel, swapping `ptr` cell → `disc` cell.

### 2.2 `Struct?` — Discriminant-Word Prepend (Option A, +8B)
Structs lack a natural spare cell → **prepend an i64 tag word at offset 0**, shifting all fields by +8:
`Struct?` slot = `{ tag@0 : i64, fields@8… }`, `total_size = struct.total_size + 8`.
`tag@0 == i64::MIN` ⟺ null; `tag@0 == +1` (`Trit::Positive`, per ADR-0020 §10.1 "value" polarity) ⟺ present. **Widening `Struct → Struct?` IS NOT a no-op** — stores tag + performs multi-word-copy of fields (reusing ADR-0060 §Point-3). Tag-word `{tag, payload}` operates identically to the Outcome slot in ADR-0057.

## 3. Memory Layout (Specific Offsets per G's Requirement)

### 3.1 `Enum?` — Unchanged Layout
```
offset:  0          8
        +----------+------------------+
slot:   |  disc    |   payload union  |    total = 8 + max_payload (same as standard Enum)
        +----------+------------------+
        ↑ disc@0 ∈ {0,1,2,…} = variant | i64::MIN = null
   null-check = stack_load(I64, slot, 0) == i64::MIN ?  (BEFORE GetDiscriminant)
```

### 3.2 `Struct?` — +8B Tag Prepend
```
        Struct (current)             Struct? (this ADR, +8B)
offset: 0      8                     0      8      16
       +------+------+              +------+------+------+
       |  x   |  y   |   16B   →    | tag  |  x   |  y   |   24B
       +------+------+              +------+------+------+
                                    ↑ tag@0: i64::MIN=null | +1=present
   null-check = stack_load(I64, slot, 0) == i64::MIN ?
   present arm bind: inner struct lives at slot+8; field x = load(slot + 8 + field.offset)
   (offset-walk +8, identical to enum payload_offset=8 / Outcome OutcomePayload offset 8)
```

## 4. ⛔ B8 BARRIER — INSCRIBED IN STONE, DO NOT VIOLATE ⛔

> # 🔴 **NULLABLE AGGREGATES CONTAIN COPY FIELDS/PAYLOADS ONLY.** 🔴
> # 🔴 **NO DROP GLUE. NO ALLOCATIONS. NO FREES. NO TOUCHING THE ALLOCATOR.** 🔴

`Enum?`/`Struct?` in ADR-0065 applies **EXCLUSIVELY** to aggregates where all fields/payloads are **Copy** (scalars: Integer/Trit/Tryte/Long/Trilean/Unit + nested Copy aggregates). Heap fields/payloads (String/Vector/HashMap) **were refused** by **B8** (ADR-0040 §4) and **REMAIN REFUSED** — `Body::verify()` `triet-mir:1500/1513` (`is_scalar_nullable_payload` for field/payload) MUST NOT be relaxed.

**Core Invariants:**
- Copy-only → Drop of `Enum?`/`Struct?` = **no-op** → **0 drop-glue, 0 new free-shims**.
- Inline stack slot → **0 allocations, 0 heap pointers** → **0 allocator/GC interaction**.
- Widening = stack value copy (multi-word), NO allocations.

Anyone (including D) attempting to introduce `String`-inside-`Struct?`, drop-glue, or free-shims for aggregate-nullables in this campaign = **VIOLATION OF INVARIANTS**, blocked immediately in review. Heap-in-aggregates is a **separate campaign** (ownership/drop), OUTSIDE ADR-0065.

> ⚠️ **§4 BOUNDARY — READ §15 BEFORE INVOKING §4 FOR REFUSALS.** The phrase "NO DROP GLUE" above
> applies to the **repr-slot construction path of ADR-0065 ITSELF** (`~+`/widening → slot
> `{tag@0, fields@8+}`). It DOES NOT prohibit `Nullable(Struct-heap)` from **existing** globally:
> ADR-0076 (heap-`T?` field/payload) and ADR-0082 (`pop`/`remove` returning `T?`) **legalized**
> that shape and **built CORRECT drop-glue** (`struct_drop` arm, measured sound). Invoking §4
> to refuse a local/pop-result `Nullable(Struct-heap)` = **misreading the architecture** — see §15.
> (WO-5, 2026-07-20: directive to refuse locals nearly broke `pop`/`remove` — 15 fixtures, caught by D.)

## 5. Alternatives Considered (Documented Comprehensively)

- **`Struct?` (B) Box on heap:** `Struct?` = i64 handle → heap struct; null = handle SENTINEL. Reuses ptr-sentinel — **however**, turns Struct? into a Move/heap type, requiring struct alloc/free shims + Drop glue + free-no-op-on-sentinel, **violating the B8 §4 barrier**, touching the allocator, and breaking the inline-stack model. **REJECTED** (G finalized 2026-06-20: "separating from the allocator at this phase is the wise move").
- **`Struct?` (C) Niche-fill first field:** borrow `i64::MIN` from the first scalar field (0 bytes, no-op widening) — **however**, lowerer must traverse type-dependent trees to find fields with niches; empty structs / structs starting with nested-structs lack obvious niches; null-check offset becomes shape-dependent. **REJECTED** (G finalized 2026-06-20: "niche-fill is the most vicious optimization a young compiler can touch… LOCK IN Option A, pay the 8 bytes"). Preserved for future optimization (Rust-style), NOT for the initial cut.
- **`Struct?` (A) Discriminant-word prepend:** +8B/value, uniform, type-independent, 0 allocator involvement, easy to poison. **CHOSEN.**

## 6. Safety & Dispatch (G Question 3)

- **Where checks are inserted:** Lowerer inserts null-checks at **match `~+/~0`** and **Elvis `?:`** — load `tag_cell` (Enum?: disc@0; Struct?: tag@0) → `icmp eq i64::MIN` → branch null vs present. Pattern identical to Elvis in ADR-0041 §5.3 (`triet-lower:2566-2581`). For Enum?, null-check runs **BEFORE** `GetDiscriminant` + `SwitchInt` (`triet-lower` tail match ~3050-3180; pattern "must run BEFORE the enum GetDiscriminant fallthrough" already exists for Trit/Integer scrutinees `triet-lower:2927-2932`).
- **★ NO sentinel-dereference hazard → NO segfaults:** Option A + Enum? are both **inline, NO pointers to dereference**. The sentinel is merely an inline tag/disc; when null, the field/payload region is **don't-care and NEVER read** because match enforces the `~0` branch (E1026 exhaustiveness — ADR-0064 — mandates `~0`). Dereference hazards ONLY exist in Option B (boxing) — another reason Option B was rejected. This is the core reason Option A wins on soundness.
- **Poison testing (wrong-values, NO SIGABRT required):** poison the tag/disc-sentinel store instruction (write to wrong offset or wrong value) → match branches to wrong arm → yields an **observable incorrect value** (model of ADR-0062 §8 sentinel-vs-zero: poison `triet-jit:1189` → returns `-9…808`). Teeth assert exact EXPECT values.

## 7. GC/Allocator Impact (G Question 4) — NONE

- **NO GC** (move-only, refcounting disabled in Tier A — ADR-0040 §1.2).
- Due to B8 §4 barrier (Copy-only) → Drop = no-op → **no cleaner needs to be taught to distinguish sentinels from real pointers.** No heap pointers exist in this aggregate-nullable cut.
- Option A + Enum? = inline stack → **0 allocator interaction.** (This is the primary reason Option A is unified into this ADR rather than Option B.)

## 8. Scope + 2 Construction Slices

**IN Scope:** `Enum?`/`Struct?` with **Copy-only** aggregates (B8 §4). Constructs (mirroring ADR-0041 §7 + ADR-0062 §7): `~0` (null materializes sentinel into tag_cell), widening `T → T?`, match `~+/~0`, Elvis `?:`. Return + local positions.

**OUT of Scope — Deferred:** heap-in-aggregate (B8 retains refusal); `?+>` map/flatMap on aggregate-nullables; `T?~E` (Outcome aggregates); nested `Struct?`-field-inside-`Struct` (field-level nullable aggregates — dedicated slice if needed).

**Two Slices (Locked by G 2026-06-20):**
- **Slice 1 — `Enum?` (Low-hanging fruit, 0 bytes):** niche disc@0. Gate `is_lowerable_nullable_payload` (`triet-mir:1399`) += `MirType::Enum(_)`. `~0` / match / Elvis null-check on disc@0. Widening = no-op.
- **Slice 2 — `Struct?` (8B tag, Option A):** gate += `MirType::Struct(_)`; layout +8B tag prepend; widening = tag-store + multi-word-copy (ADR-0060); match present-arm binds at slot+8 (offset-walk).

## 9. Risks + Mandatory Teeth

- **Sentinel-vs-disc collision (Enum?):** MUST prove no real discriminant == `i64::MIN`. Discriminants assigned {0,1,2,…} (`VariantLayout:1119` "0, 1, 2, …") — safe. Teeth: enum with multiple variants + match present (each variant) + match null.
- **Tag-offset (Struct?):** field binding at wrong offset (+8) → overwrites tag / data-loss. Teeth: `Struct?` present-arm reads fields → correct values; poison removing +8 offset-walk → wrong values.
- **Widening Struct? (Slice 2):** multi-word-copy missing words → subsequent fields corrupted. Teeth: struct with ≥2 fields, widen then read both. Poison copying 1 word → subsequent fields = garbage.
- **`~0` Materialization:** poison sentinel store (offset/value) → match branches incorrectly → wrong value (model of ADR-0062 §8).
- **Blind-spot Rule:** teeth must exercise **BOTH Enum? AND Struct?** (Slice 2), and **BOTH present AND null arms** (each branch is a front — lesson from HP.3).
- **B8 Guard (Regression):** teeth assert `String`-inside-`Struct?` / `Vector`-inside-`Enum?` payload STILL reject with `HeapNullableNotLowered` (negative fixture). Poison: if anyone relaxes scalar→heap gate for field/payload, negative fixture must fail.

### 9.1 Amendment (Slice 1, 2026-06-20) — TWO Distinct B8 Refusal Portals with Different Error Codes (Measured by O, Traceable Edit)

Verifying Slice 1 revealed that B8 refuses via **two distinct checkpoints** — teeth must target the exact checkpoint for this slice:
- **`Enum?` with nullable-heap payload** `Has(String?)` → `Body::verify()` enum-payload gate (`triet-mir:1500/1513`, preserving `is_scalar_nullable_payload`) → **`MirError::HeapNullableNotLowered`** "at enum payload `Bag.Has`". **THIS is ADR-0065's guard** (negative fixture 230). Relaxing scalar→heap gate for field/payload → fixture 230 fails.
- **`Enum` with plain-heap payload** `Has(String)` (non-nullable) → blocked earlier at is_copy construction gate (ADR-0040 **pre-existing B8**, `triet-lower`) → `LowerError "heap types not supported"`. **Orthogonal** — not this slice's guard; only triggers when CONSTRUCTING variants.

Slice 1 B8 teeth use `String?` payload (`HeapNullableNotLowered` portal), NOT plain `String` (different portal). §9 bullet "B8 guard" above points to the `HeapNullableNotLowered` portal.

### 9.2 Amendment (Slice 2, 2026-06-20) — Widening `Struct → Struct?` MUST Emit Assign (NO In-Place Retyping)

Verifying Slice 2 revealed a flawed assumption in the original WO (recon oversight by O, patched in-scope, invariants unchanged):

- **Actual lowerer mechanism:** `let x: T? = y` in `triet-lower/src/lib.rs:1207` by default **retypes y's local in-place** (`local_decls[v].ty = ann_ty`) + aliases — DOES NOT emit `Assign`. This is PRECISELY why Slice 1 widening (`Enum → Enum?`) was a **no-op**: niche disc@0 shared the slot, relabeling was sufficient.
- **Why Struct? breaks:** `Struct?` expands by +8B (tag prepend). Retyping in-place retains the 16B `StructAlloc` slot (x@0, y@8) while labeling it `Nullable(Struct)` → `walk_projections +8` reads OOB → garbage values (fixture 231 returned 6 instead of 7). Existing TODO at lines `1200-1206` correctly predicted this case ("emit an Assign to a new typed local (M2 pattern) instead of mutating").
- **Delta 0 (Fix):** when `init_ty == Struct(_)` **and** `ann_ty == Nullable(Struct(_))` → allocate a NEW `Nullable(Struct)` local + `Assign{new ← v}` (M2 pattern), ACTIVATING the JIT widening branch (Delta 4a). **Strictly scoped** to `Struct→Struct?`; Enum?/scalar/String? retain in-place behavior (fixture 229 remains green).
- **Tag-store Teeth (P3):** storing `tag=present(1)` in 4a is load-bearing, but fresh-slot fixtures did NOT catch omissions (uninitialized slot happened to be ≠ MIN). Requires a **reassign-widen-over-null** fixture (237: `let mutable n: Pt? = ~0; n = p;` on a slot previously holding `~0`=MIN): omit store → old MIN tag persists → match incorrectly hits `~0` → fails. §9 bullet "`~0` materialize" expanded: widening-tag teeth MUST use recycled-null slots, not fresh slots.

## 10. Consequences

- (+) Completes the nullable system across all types; invariant `tag_cell == i64::MIN ⟺ null` unifies scalars/heap/enums/structs.
- (+) Enum? = 0 bytes, 0 allocator involvement. Struct? = +8B, 0 allocator involvement. Value-model i64 **UNTOUCHED** (leaf loads remain I64; expands only slot layouts, in the same family as Outcome/nested-aggregates).
- (−) Struct? incurs +8B overhead/value (accepted by G: "RAM is cheap; debugging type-dependent offsets is not").
- **Frozen Scope:** Heap-in-aggregates + drop-glue transparently deferred (B8 §4) — no empty promises, no dead-code skeletons.

## 11. Migration

| Milestone | Task | Repr Changed? |
|---|---|---|
| Future: niche-fill (C) | Struct? eliminates tag-word for structs with niche fields → 0 bytes | Possible, localized, new ADR |
| Heap-in-aggregate | Drop-glue + free-shims for aggregates containing Move fields | Dedicated campaign |
| Tier C packed ABI | Tag-word can unify with Outcome discriminant | Possible, localized |

---

## 12. AMENDMENT (Slice 3, 2026-06-20) — Nested Nullable Aggregate of Copy (Axis A)

**Context:** §8 noted "OUT of scope — deferred: nested `Struct?`-field-inside-`Struct`". §4/§9 locked
B8 (Copy-only). Slice 1 (`Enum?`) + Slice 2 (`Struct?`) operated solely at the **top-level** (return/local).
When `Struct?`/`Enum?` sits **as a field/payload of another aggregate** (`struct Holder { p: Point? }`),
three layers blocked execution: field/payload gate (`is_scalar_nullable_payload`, scalar-only), sizing fixup (only mapped
`Struct/Enum`, not `Nullable(Struct/Enum)` → `Point?` field defaulted to 8B = WRONG), offset-walk
(`walk_projections` failed to unwrap `Nullable` mid-walk → `h.p.x` failed with "field access on non-aggregate").
**Slice 3 (Axis A)** enables Case 1 — `struct Holder { p: Point? }`, Point all-scalar — pure **layout math**,
NO allocator, NO drop-glue.

### 12.1 Mechanism (Inherited from Slice 1/2, applied at field/payload position)
- **`Nullable(Struct)` field** = `inner.total_size + 8` (tag-word prepended @ field-offset, identical to top-level
  Slice 2 §2.2). Tag @ field-offset == `i64::MIN` ⟺ null; == `+1` ⟺ present. Real fields of Point live at
  `field-offset + 8 + Point.field.offset`.
- **`Nullable(Enum)` field** = `inner.total_size` (disc-niche, **0 bytes** overhead, identical to Slice 1 §2.1).
  disc @ field-offset == `i64::MIN` ⟺ null.
- **Offset-Walk**: when `walk_projections` traverses INTO a field of type `Nullable(Struct)` → add tag-shift +8
  then unwrap to `Struct`; `Nullable(Enum)` → +0 then unwrap to `Enum`. Reuses the principle of
  `nullable_struct_base_offset` (struct→8 / enum→0) applied **mid-walk** rather than solely at the base.

### 12.2 Copy Condition — INSCRIBED IN STONE (Soundness Crease)
Applies ONLY when `inner.is_copy(Some(body))` (`triet-mir:666` — recursive + body-aware: inspects fields/variants).
- Heap-in-nested-nullable (`struct Bad { s: String }`, `Holder2 { p: Bad? }`) → `Bad.is_copy = false`
  → field/payload gate **RETAINS refusal** `HeapNullableNotLowered`. **B8 §4 REMAINS INTACT.**
- `Nullable(String/Vector/HashMap)` field → inner is NOT Struct/Enum → refused (B8 intact).
- **Warning (Inscribed by G):** field/payload gate MUST be **body-aware**. Relaxing the gate to accept `Nullable(Struct/Enum)`
  **purely structurally** (without checking `is_copy`) = B8 loophole → `Bad?` field would copy String bytes as Copy =
  latent double-free/leak. `find_refused_nullable` currently takes `allow: fn(&MirType)->bool` WITHOUT body access
  → relaxation mechanism must pass body into field/payload branches.

### 12.3 Layout Math — Nested Recursive Offset
Outer tag-word (if outer aggregate is also nullable) + inner tag-word accumulate; 8-byte aligned padding.
Case 1 (outer aggregate is NOT nullable):

```
struct Point  { x: Integer, y: Integer }      → Point.total  = 16  (x@0, y@8)
                Point?                          → Point?.total = 24  (tag@0, x@8, y@16)  [+8 tag]
struct Holder { p: Point? }                    → Holder.total = 24  (p@0)

  Holder slot:  offset 0      8      16
                +------+------+------+
                | tag  |  x   |  y   |     p@0 → tag@0, Point.x@8, Point.y@16  (absolute)
                +------+------+------+
  read h.p.x = load(slot + p.offset(0) + tag-shift(8) + Point.x.offset(0)) = load(slot+8)
  read h.p.y = load(slot + 0 + 8 + 8)                                       = load(slot+16)
```

`Nullable(Enum)` field: 0-byte tag → field-offset does not shift (disc @ field-offset is the niche).

### 12.4 NO Drop-Glue, NO Allocator
Copy-only (§12.2) → Drop = no-op → 0 drop-glue, 0 free-shims, 0 allocator interaction (inheriting §4 + §7 + §9).
Widening `Holder{ p: ~+ Point{...} }` = stores tag + multi-word-copies Point fields (reusing Slice 2 §2.2
+ ADR-0060 §Point-3), NO allocations.

### 12.5 ⚰️ Axis B Ruling — BOOK OF DEATH (Dedicated VISION Campaign)
**Heap-in-aggregate + recursive drop-glue = Axis B = DEDICATED VISION CAMPAIGN, NOT ADR-0065.**
The ADR for Axis B is completely blank — not a single line written. **B8 (§4) locks all heap-in-aggregate field-offsets
regardless of nullability.** ADR-0065 (including this §12) DOES NOT imply Axis B is touched in this campaign.
Anyone introducing heap-in-nested-nullables, drop-glue, or free-shims for aggregate-nullables = VIOLATION OF INVARIANTS,
blocked in review.

### 12.6 Mandatory Teeth
- **Body-Aware Gate (Slice 2 WO):** poison relaxing gate to purely structural (omitting `is_copy`) → `Bad?`-heap-field fixture
  CEASES to be refused → FAILS (proves Copy-check is load-bearing against B8 leaks). Control: `Nullable(String)` field remains refused.
- **Sizing (Slice 3 WO):** poison removing `+8` for Nullable(Struct) fields → Holder.total wrong → walk OOB → garbage/SIGSEGV FAILS.
- **Offset-Walk (Slice 4 WO):** poison nested tag-shift 8→0 (or 8→16) → `h.p.x` reads misaligned bytes → wrong value/SIGSEGV FAILS.
- **Construction + Read-Back (Slice 5 WO):** reading back `h.p.x` is MANDATORY (construct-only does not count);
  **adjacent-field fixture** (struct with 2 fields + Point? following, reading subsequent field) proves +8 DOES NOT corrupt subsequent fields.
- **B8 Regression:** negative fixtures for `Bad?` (heap-in-struct) + `Nullable(String)` fields STILL refuse with `HeapNullableNotLowered`.

### 12.7 Construction Taxonomy (Re-scoped 2026-06-20 — Traceable Edit, G Signed Option a)

**Recon Oversight in Original WO (Acknowledged by O):** §12.4 stated "reuses widening from Slice 2 §2.2 (Delta 4a)". INCORRECT. Delta 4a/4b
in JIT (`mir_lower.rs:1375/1418`) **gate `projection.is_empty()` ON BOTH SIDES** → only executed for top-level
`let x: Struct? = y`. Construction (`_0.p = move v` — dest projected) + read-back (`_2 = move h.p` — source
projected) **NEVER** reached 4a/4b → fell to general-copy. **Field-position construction had never been implemented.**
This was a core gap, not a trivial bug.

**Three Root Bugs (Traced to MIR by O):**
- **Bug A — JIT base-downcast swallowed tag** (`walk_projections:297`): `nullable_struct_base_offset` baked `+8`
  into all `Nullable(Struct)` bases. `load_place`/`store_place` empty projections read directly from `slot@0` (WITHOUT calling walk
  → top-level match 231-237 was CORRECT), BUT Assign-copy called walk on both src + dest → whole-slot move added +8 → **skipped
  tag@0** → null returned garbage. Blast radius when removed = **NARROW, Assign-copy only**.
- **Bug B — Lowerer `~+ aggregate` routed to Outcome** (`lib.rs:1557`): `~+ Point` → `OutcomeAlloc` with
  `outcome_ty = return_type` (Integer of main) → `OutcomeAlloc non-Outcome Integer`. `~+` was purely Outcome,
  lacking a nullable-present branch.
- **Bug C — Lowerer implicit field-widening failed to set tag** (`lib.rs:2920`): `Point{..}` → plain Struct →
  `_0.p = move _1` plain Assign, WITHOUT widening, WITHOUT SetTag → present **passed by luck** (uninitialized tag happened to be ≠ MIN).

**Solution (Option a — Faithful Walk + 4-Case Taxonomy):** remove base-downcast from `walk_projections` (making it
**faithful** — returning true offsets with intact `Nullable(Struct)` types). Relocate downcast/widen/whole-copy decisions to
the **Assign-copy gateway**, dispatching on `(src_ty, dest_ty)` AFTER the faithful walk:

| dest \ src | plain `Struct` | `Nullable(Struct)` |
|---|---|---|
| **plain `Struct`** | general copy (legacy, ADR-0060) | **Case 3 DOWNCAST**: copy fields `src+8 → dest+0` (= match-bind `pt = scrut`) |
| **`Nullable(Struct)`** | **Case 2 WIDEN**: set `tag=1@dest+0`, copy fields `src+0 → dest+8` (= old 4a + field implicit) | **Case 1 WHOLE-COPY**: `N+8` bytes, **tag@0 FIRST**, `src_off → dest_off` (= old 4b + construction + readback) |

**5 Invariant Principles:**
1. **Faithful Walk:** `walk_projections` returns true offsets (base bare-`Nullable(Struct)` DOES NOT add +8); retains Slice 4
   `nested_nullable_shift` for field-INTO-nullable mid-walk.
2. **Subsumption:** Taxonomy consolidates Delta 4a (→ Case 2) + 4b (→ Case 1). **DELETE legacy 4a/4b**, do not maintain in parallel.
   Downcast +8 (previously baked blindly in walk) is now the **explicit** behavior of Case 3.
3. **Tag Invariants INTACT:** `{tag@0, fields@8}`, `tag@0==MIN ⟺ null`. Case 1 copies **tag-first** → preserved verbatim.
4. **Enum? Field Analog:** +0 (niche), tag = `disc@0 == MIN`.
5. **Copy-Only:** NO drop-glue/allocator. Heap (Axis B) = book of death, B8 §4 locked, STRICTLY FORBIDDEN to touch.

**Lowerer (Slice 3'):** `~+ inner` at struct-field when `field_ty == Nullable(Struct/Enum)` → lower `inner` plain
(DOES NOT route to `OutcomeConstructor`); field Assign widens automatically via **Case 2**. Bug C implicit widening requires NO lowerer changes —
Case 2 in JIT automatically widens plain Assigns. Top-level `~+` (`let x: Struct? = ~+ y`) if broken → **recorded as technical debt, OUTSIDE
scope** (G finalized separation).

**Teeth (Re-scoped):**
- **LOCKED 231-237 REMAIN GREEN** (regression harness — Cases 1/2/3 subsume correctly).
- Poison Case 1 (whole-copy → forcing +8 downcast) → readback-null misaligned → null fixture FAILS.
- Poison Case 2 (omitting set-tag=1) → present loses tag → present fixture FAILS. **MUST be an observable fixture, NOT
  passing by luck** (lesson from Slice 2 P3 + rejected Slice 5).
- Poison Case 3 (omitting +8) → match-bind `pt.x` reads tag instead of field → 231-237 + present FAILS.
- Poison `~+` special case → `Holder{p:~+ Point{...}}` hits `OutcomeAlloc` FAILS.
- **⚔ Adjacent Field** (`struct H2{a, p:Point?, z}`): construct then read `z` → 8B tag + nested content DOES NOT
  corrupt the address of `z` behind it (offset verified via sizing-fixup + walk, NOT relying on suggested numbers).

### 12.8 Amendment (WO-~+-NULLABLE-UNIFY, 2026-06-21) — Comprehensive `~+ nullable-present`: Top-Level `let` + Field Scalar

**Context:** §12.7 resolved field-position construction for aggregates (`Struct?`/`Enum?`) but recorded debt for
**two remaining survival paths** of Bug B (`OutcomeAlloc on non-Outcome type 'T?'`):
- **Nest 1 — top-level `let x: T? = ~+ v`:** `~+` lowered directly to `OutcomeConstructor` → `outcome_ty = return_type`
  (Integer of `main`, non-Outcome) → invalid `OutcomeAlloc`. Broken for scalars / Structs / Enums alike
  (`Integer?`/`Point?`/`Color?` — O probed all 3 with identical errors).
- **Nest 2 — field scalar `Holder{f: ~+ 5}` with `f: T?` scalar:** §12.7 gate only accepted `Nullable(Struct|Enum)`
  → scalars fell to else branch → same `OutcomeAlloc`. (Field `Struct?`/`Enum?` already operational in §12.7 — fixtures 247/249, untouched.)

**Decision — LOWERER-ONLY, Reusing 100% of Widening Infrastructure, NO New ADR:**

- **Fix 1 (Top-level let, `lib.rs` head of else branch ~1210):** before `lower_expr(*init)`, if `init` =
  `OutcomeConstructor{ arm: Positive, payload: Some(inner) }` AND annotation lowers to `MirType::Nullable(_)`
  → lower `*inner` (plain payload) INSTEAD of `*init`. Existing widening block (Slice 2 Delta 0) handles the rest:
  - `Nullable(Struct)` → `is_struct_widening` → fresh Assign → JIT taxonomy **Case 2 WIDEN** (proven by fixture 252).
  - `Nullable(Enum)` → in-place retyping → **niche disc@0** (253, mirroring 229/225).
  - `Nullable(scalar)` → in-place retyping → **PA-3c no-op** (251).

  NO branching on types — all 3 flow through proven Axis A widening paths. Symmetrically mirrors
  field-position redirect in §12.7 (StructLiteral).

- **Fix 2 (Field gate, `lib.rs` StructLiteral ~2940):** relax `field_is_nullable_agg`
  (`Nullable(Struct|Enum)`) → `field_is_nullable = matches!(field_decl_ty, Some(Nullable(_)))`. Scalar
  `~+ 5` → lowers `inner=5` plain → field Assign stores i64 (scalar nullable: **value IS repr**, present
  5 ≠ MIN). **B8 REMAINS INTACT:** `is_copy` check executes AFTER all branches — `f: String?` initialized with `~+ "hi"` → inner String
  → `is_copy` false → rejected (fixture 255).

**Teeth (O Verified — 3 Independent Red Teeth, 1 per Path):**
- **P1** disable Fix 1 redirect → **251+252+253 all fail with `OutcomeAlloc on non-Outcome 'Integer?'/'Point?'/'Color?'`**
  → proves redirect is load-bearing across ALL 3 types.
- **P2** revert Fix 2 gate to `_agg` → **254 fails with `OutcomeAlloc on non-Outcome 'Integer'`** (isolates field-scalar).
- **P3** relax `is_copy` for String → **255 FAILS** (refusal "heap types…" vanishes). B8 has **2-layer defense-in-depth**:
  `is_copy` (lowerer, pinned by fixture 255 message) layer 1 + verifier `heap-nullable T? not yet lowered` layer 2.
- Per-type widening (Case 2 / niche / PA-3c no-op) already possesses Axis A teeth (231/229/249) — NOT re-poisoned.

**⛔ OUT OF SCOPE — Deferred (G Finalized Separation of Concerns):** direct `match h.f` on **scalar-nullable
FIELDS** fails with `unsupported match pattern (expected enum variant)` — this is a **READ-PATH gap**
(field-read temporary typed Unknown, `lib.rs:2904-2911`, intentionally preserving scalar-leaf-as-i64 for arithmetic),
DISTINCT from Bug B (the WRITE path). Fixing it requires modifying field-read typing at 2904, with unmeasured blast radius
across 245+ fixtures → **Technical Debt Ledger, DO NOT open WO-2 at this time**. Fixture 254 reads via
`let y: Integer? = h.f` (typed-let widening Unknown→Nullable) as an acceptance bridge for the WRITE path.

**Consequences:** Following Fix 1+2, NO sites remain where `~+` generates `OutcomeAlloc-on-non-Outcome`. The Nullable Aggregate
construction pipeline is fully closed (top-level + field, scalar + aggregate). Pure Outcome `~+` (`T~E`/`T?~E`) REMAINS
UNCHANGED in behavior (non-Nullable annotations do not trigger the redirect → lower `OutcomeConstructor` normally).

**§12.8 Signatures:** O: ✅ (verified — P1/P2/P3 independent failures, gate `0·0·250·0`, clean lowerer-only diff, 2-layer B8 intact) · G: ✅ (approved 2026-06-21 — WO-~+-NULLABLE-UNIFY).

## 13. HOTFIX (2026-07-15, O Recon on `9a1799c`) — Payload-Bearing `Enum?` REFUSED, Disc-Niche is Unit-Only

**O Discovery:** Disc-niche §2.1/§12.7 was validated on **unit-only** enums (`Color{Red,Green,Blue}`, 8B — disc@0
IS the entire value). It was never proven for an enum with a **payload-bearing variant** (>8B: disc@0 +
payload@8…). When `E?` is used as a **function's own return type**, the single-i64 return ABI truncates the
aggregate crossing the call boundary: the caller receives a corrupted discriminant, and the enum drop-glue
`SwitchInt` on that garbage value falls to `default` → `Trap` → **SIGILL (exit 132)**. Reproduced for BOTH aggregate payloads
(`enum E{V(Big),N}`, `Big{p,q}` two `Integer` fields) and scalar payloads (`enum E{V(Integer),N}`) — not aggregate-specific.

**Fix (Surgical, lowerer-only, single bottleneck):** `Expr::OutcomeConstructor`'s `Nullable` branch
(`crates/triet-lower/src/lib.rs`, guarding both `~+` and `~0` arms) now refuses, at construction time,
any `E?` where `E`'s `EnumLayout` has at least one variant with `payload.is_some()`. This is a **structural**
refusal at the **constructor bottleneck** — guarding both `~+` and `~0` arms **WHEN CONTROL FLOW TRAVERSES IT**.

> ⚠️ **CORRECTION (Re-measured by O 2026-07-20, Signed by G).** The original text stated *"fires at every `E?`-value construction
> site (top-level `let`, function `return`, struct field)"* — **OVERSTATED**. Measured on `235e376`:
> `return ~+ E::V(42)` → refused ✓ exit 3 · struct-field → refused ✓ exit 3 (separate aggregate guard, not
> this bottleneck) · **`let x: E? = ~0` → SLIPPED THROUGH, exit 0, evaluated to `0`** · **`let x: E? = E::V(42)`
> (implicit widening) → SLIPPED THROUGH, exit 0, evaluated to `1`**.
>
> Mechanism of bypass: `Stmt::Let` fast-path `is_null_expr` routed directly to `Statement::Const`, **bypassing**
> `Expr::OutcomeConstructor`; while implicit widening `E → E?` is not a constructor, so it **never**
> reached the bottleneck. The guard operates exactly as described — but those two paths never traversed it.
>
> ⇒ **"Sealing the entire surface" holds true ONLY for the CONSTRUCTOR path.** The remainder is **hole N1**, classified by G as a
> **POLICY-HOLE (NOT UB)** on 2026-07-19, confirmed on 2026-07-20: heap payload local/param
> `FREE=1 distinct=1 dup=0`, correct values, struct-field path refused, `i64::MIN` unrepresentable
> from source. **FORBIDDEN to cite §13 as proof of having sealed a UB path.**

Per refuse-over-guess: disc-niche for payload-bearing enums has not been proven safe outside the return-ABI hazard,
so the constructor path is sealed rather than granting an exception "only at return position".

**NOT Fixed Here (Deferred Front):** A proper `Enum?` representation for payload-bearing enums (e.g. real disc-niche
marshaling across the return ABI, or falling back to the `Struct?` +8B tag-word scheme) — tracked as new debt
"nullable-enum-payload niche marshal" pending a future slice. `Struct?` (§2.2/§3.2, +8B tag prepend) is
UNCHANGED and out of scope for this hotfix.

**Regression:** Unit-only `Enum?` (§12.7 taxonomy, fixtures 249/250) is untouched — the refusal predicate only
fires when `payload.is_some()` for some variant; a unit-only enum's `EnumLayout` has `payload: None` on every
variant, so the guard never trips for it.

**Teeth:** Fixtures 374 (aggregate payload, function-return shape, proven poison-red exit 132) / 375 (scalar
payload, same shape, proven exit 132) / 376 (struct-field construction path — refusal proven, crash NOT
independently reproduced for this exact shape; refused structurally regardless) / 377 (unit-only, local `let`
— non-vacuous negative control, still compiles + runs).

**§13 Signatures:** D (Sonnet 5) ✅ implemented + poison-red (374 only, per WO) · **O ✅ 2026-07-20** — independently verified on `235e376`: refusal correct on constructor path (`return ~+`) and struct-fields; **proved the statement "every construction site" was OVERSTATED** (`let = ~0` and implicit widening slipped through) → corrected above prior to signing · **G ✅ 2026-07-20** — approved correction, confirmed N1 is a POLICY-HOLE.

---

## 14. AMENDMENT (WO-2 Slice A, 2026-07-20) — `Struct?` at RETURN Position: Unlocking Full-SRET

**Status:** O ✅ drafted + measured · G ✅ finalized 2026-07-20 · D: implemented.

### 14.1 Context — Repr Settled in §2.2, Only RETURN Position was Locked

§2.2/§3.2 established the `Struct?` representation as **disc-word prepend `{tag@0, fields@8+}`**, and it operates
correctly for **derived locals** (`mir_lower.rs:2462-2489`, slot = `layout.total_size + 8`). However,
the **RETURN position was never wired**: `is_struct_return` (`triet-lower/src/lib.rs:320`) matched
`MirType::Struct(_)` **exactly**, causing `Nullable(Struct)` to fall directly to `_ => ReturnShape::Scalar`.

On 2026-07-19 (WO-StructReturnRefuse, `e7aab8c`), a **POLICY GATE** was erected in `Ctx::new`
to block measured miscompilations. That gate was **NOT a soundness fix** — it was a temporary checkpoint awaiting
this exact amendment. §14 removes it.

### 14.2 Evidence — O Removed BOTH Gates + Rebuilt, Measured 6 Shapes (2026-07-20)

| Shape | Exit | Result |
|---|---|---|
| `P?` present, reads 1 field | 0 | **silent garbage** (`93851586002064`, varies with ASLR) |
| `P?` **null** | 0 | **silent garbage** — `~0` branch **DEAD** |
| `P?` present, reads 2 fields | **132** | SIGILL (garbage + garbage exceeds ADR-0044 range — **secondary** effect) |
| `U?` present, reads disc | **132** | SIGILL (belongs to Slice B) |
| `U?` **null**, unread | 0 | returns `1`, should be `0` — **silent** (Slice B) |
| `P?` present, unread | 0 | `1` — correct **by accident** |

**Mechanism** (identical to the `Struct?` PARAM bug patched in WO-StructParamABI): the sret return local holds
the **POINTER** to the caller's buffer; sentinel comparison compares the pointer bit-pattern with `i64::MIN` → stack addresses
are never equal to `i64::MIN` ⇒ **always evaluated as "present"** ⇒ `~0` branch is dead. Present-arms read
garbage because the tag was never written and the `+8` offset was never applied at the return position.

**Silent garbage is the ROOT failure mode; SIGILL is merely thunder** when adding two garbage values exceeds ADR-0044 thresholds.

### 14.3 Decision

`Nullable(Struct)` at the return position uses **sret**, with the **EXACT repr locked in §3.2** —
`{tag@0, fields@8+}`, buffer size `layout.total_size + 8`. **DO NOT invent new representations.**
This applies existing mechanisms to a previously locked position, recorded in this amendment without creating new ADRs.

### 14.4 ⛔ INSCRIBED IN STONE — DO NOT Stuff `Struct?` into `is_string_repr()`

`is_string_repr()` (`triet-mir/src/lib.rs:663`) means **"shares String's 24B fat representation"**.
`Nullable(String)` belongs there because it *truly* shares the fat slot. `Struct?` **does not** — it is
a tag-prepend layout. Stuffing it there makes the predicate **lie about its own name** and corrupts all downstream consumers.

The correct place to fix: `is_struct_return` (`:320`) unwraps `Nullable` using idioms already established in the repository
(`ty.nullable_payload().unwrap_or(ty)` — `mir_lower.rs:2437, 2472`).

### 14.5 Scope — Slice A ONLY, and PURELY COPY-ONLY

> ⛔ **§4 (B8 barrier) REMAINS FULLY IN EFFECT IN THIS AMENDMENT.** `Struct?` carries **Copy** fields only.
> Heap-bearing `Struct?` (fields of `String`/`Vector`/`HashMap`) at return position **REMAINS REFUSED** —
> it demands drop-glue which §4 explicitly forbids.

✅ In scope: `Nullable(Struct)` return position **where all fields are Copy**.
❌ Out of scope — **Dedicated Slice B:** `Nullable(Enum)` returns. Reason for separation: **DIFFERENT REPR** — `Enum?` is
disc-niche, slot = `layout.total_size` (`mir_lower.rs:2444`, **no +8**), whereas `Struct?` is
tag-prepend `+8`. Bundling both representations into one slice obscures root causes when failures occur (finalized by G).
❌ Out of scope: N1 payload-bearing `Enum?` (§13) — **untouched**. Gate `Enum?` return `:289-297` currently
only blocks **unit-only**; payload-bearing was already blocked from construction by §13.

### 14.6 Mandatory Teeth

| # | Shape | Oracle | Catches |
|---|---|---|---|
| T1 | `P?` present, `return p.x` | EXPECT exact value | silent garbage |
| T2 | `P?` **null**, `~0 => -1` | EXPECT `-1` | ⭐ **dead null branch** — most dangerous silent failure |
| T3 | `P?` present, `p.x + p.y` | EXPECT correct sum | SIGILL 132 |
| T4 | `P?` present, NO field read | EXPECT | "correct by accident" case — prevents false regressions |
| **T5'** | `H?` **heap-bearing** (`struct{s:String}`) at return | **REFUSE** (fixture 440 remains green, B8/§4 reason) | B8 barrier — negative test |

**🚫 T5 (Original Version) REVOKED — O Erred, Caught by D (2026-07-20).** The original draft demanded a counting tooth
`FREE==1 && dup==0` for heap-bearing `Struct?`, effectively **mandating drop-glue explicitly forbidden in §4 in capitalized text**.
O drafted T5 without re-reading the INSCRIBED constraints of the ADR being amended.
If D had implemented it, it would have breached the barrier and reopened `SIGABRT 134` guarded by fixture 440 (`is_lowerable_nullable_payload`,
`triet-mir:1679-1687`, has an unconditional `Struct(_)` branch — with no secondary net beneath).
Replaced by **negative T5'**. Revocation signed by G 2026-07-20.

🦷 **Established Rule:** When drafting amendments, **re-read ALL INSCRIBED INVARIANTS of the original ADR prior to appending** —
amendments inherit all constraints of the body, and the strongest invariants often reside **furthest** from the edit site.

⚠️ **Harness Note:** `integration_test_corpus()` is a **SINGLE** test running in a loop — if T3 triggers SIGILL, it
kills the entire process and subsequent fixtures **never run**; thus, a "failing suite" DOES NOT prove T1/T2
have teeth. Each tooth must be proven by changing `EXPECT` to an **arbitrary invalid** value → yielding
`FAIL <name>: expected …, got …` → then restored.

### 14.7 ⛔ INVARIANT — `is_fat_ret` Has THREE Copies, Must Remain Synchronized

The predicate deciding "whether this return uses sret" exists in **three** locations:

| # | Location | Role | With `Struct?` (Prior to Slice A) |
|---|---|---|---|
| 1 | `triet-lower/src/lib.rs:320` (`Ctx::new`) | **callee-side** | exact match → miscompile |
| 2 | `triet-lower/src/lib.rs:3103` (`Expr::Call`) | **caller-side** | exact match → miscompile |
| 3 | `triet-lower/src/lib.rs:5501` (`Expr::MethodCall`) | caller-side | ✅ **sret (WO-MethodCallFatReturn)** |

**Caller/callee mismatch = JIT panic OR silent Scalar miscompile.** Patching one copy while forgetting others is a
classic class of bugs in this family.

**Slice A patched #1 + #2.** #3 was initially unpatched — failing closed (over-refusal, not UB), proven by
probes (fixtures 448/453 `_refused`).

**Standing Rule:** Anyone modifying one of the three copies MUST grep the remaining two within the same slice.

#### AMENDMENT (WO-MethodCallFatReturn, 2026-07-25) — Closing Copy #3

**Status:** O ✅ · G ✅ · D ✅

Copy #3 (`Expr::MethodCall`) now mirrors copy #2: `is_fat_ret` unwraps `Nullable` to recognize bare
`Enum`, `Struct?` (Nullable Copy-only), and `Enum?` (Nullable unit-only) → dispatching sret (`EnumAlloc`/
`StructAlloc` + `ReturnShape::Enum`/`ReturnShape::Struct`), rather than falling into refusal. Closed scope:
Enum (bare) · `Struct?` · `Enum?`. **Vector/HashMap/Reference (+ `Vector?`/`HashMap?`) REMAIN REFUSED** —
over-refusal > miscompilation, as they lack dedicated ABIs. `Struct?` heap-fields / `Enum?` payload-bearing are UNTOUCHED
(callee-side E1121/E1120 fire before reaching here). Fixtures 448/453 flipped to `EXPECT`;
469 added probing bare `Enum` method returns.

---

## 15. AMENDMENT (WO-5, 2026-07-20) — §4 Boundary: `Nullable(Struct-heap)` is SOUND via `struct_drop` Arm; True UB is Container-Elements

**Status:** O ✅ measured + verified · G ✅ finalized 2026-07-20 · D ✅ implemented (`f432987`+`07ca203`).

### 15.1 Context — Two Conflicting ADRs, Constitutional Alignment with Empirical Reality

§4 stated in absolute terms *"AGGREGATE NULLABLES CONTAIN COPY FIELDS/PAYLOADS ONLY — NO DROP GLUE"*.
However, **following** ADR-0065, two other ADRs enabled the exact constructs §4 thought were permanently banned:
- **ADR-0076** — heap-`T?` at fields/payloads of aggregates (String?/Vector?/HashMap?), featuring sentinel-no-op drop glue.
- **ADR-0082** — `Vector::pop` / `HashMap::remove` returning `T?`. With `T` = **heap-bearing struct** (`User { name: String }`),
  the result is a `Nullable(Struct-non-Copy)` local, and its Drop routes through the **`struct_drop` arm**
  (`mir_lower.rs`, `Nullable(inner) => Some((name, niche:8, is_nullable:true))`) — tag-guarded + `+8`-shifted inline,
  **freeing CORRECTLY** (measured: FREE=1 dup=0, WO-5 Step ①).

When ADR-0076/0082 established these paths, **the §4 barrier was NOT updated** — leaving a contradictory constitution.
WO-5 read §4 literally ("seal all `Nullable(Struct-heap)` locals"), nearly breaking `pop`/`remove`.

### 15.2 Evidence (Measured by O, No Speculation)

**Step ① Measurement — Local heap-bearing `Struct?` DOES NOT leak:**
```
CONTROL bare Leaf:        FREE=1  dup=0
LOCAL  Leaf? heap field:  FREE=1  dup=0   ← sound, routes through struct_drop arm
```
**Poison Verification — Refusing non-Copy `Nullable(Struct/Enum)` locals → 15 fixtures BROKE**, including
`338-342` (`Vector<UserStruct>.pop()`) and `343-346` (`HashMap<_,UserStruct>.remove()`):
`pop`/`remove` returning `T?` generates a `Nullable(Struct-heap)` local with the **SAME `MirType`** as user-written `let a: Leaf? = ~+…`.
`Body::verify()` only inspects `MirType`, **WITHOUT AST visibility** ⇒ cannot distinguish origin. Refusing at the verifier = killing shipped `pop`/`remove`.

### 15.3 Decision — Clarifying Boundaries, NO Relaxation of Soundness

**§4 "NO DROP GLUE" applies NARROWLY to ADR-0065's repr-slot construction path** (`~+`/widening →
slot `{tag@0, fields@8+}`, where Drop is a no-op due to Copy-only). It **DOES NOT** prohibit:
- **local / pop-result `Nullable(Struct-heap)`** — SOUND via `struct_drop` arm (built by ADR-0076/0082, measured FREE=1). **Permitted, not refused.**

**True UB (patched by WO-5) existed in ONE location: container-element free path.**
`emit_vector_element_free_loop` / `emit_hashmap_value_free_loop` computed `eff = inner_ty.nullable_payload().unwrap_or(inner_ty)`
— **unwrapping `Nullable` BEFORE** calling `emit_heap_free_at`, **discarding tag-guards and +8-shifts** ⇒
`emit_heap_free_at` received bare `Struct("Leaf")`, reading the String field at **offset 0** =
**treating the TAG (=1) as a heap pointer → `free(1)`** (SIGABRT 134).

### 15.4 Fix (Implemented)
- **Refuse** heap-bearing `Nullable(Struct/Enum)` at **container-element** positions (`Vector<Leaf?>`,
  `HashMap<_,Leaf?>`) in `Body::verify()` — Copy-gated (reusing `find_refused_nullable_field`),
  prior to JIT execution. Copy-only elements (`Vector<P?>`) continue to operate.
- **Remove** the `Nullable(Struct/Enum)` drop-glue branch erroneously added to `emit_heap_free_at` (WO-4 B2) —
  RULE7-probe proved 0 tests exercised it; all callers unwrapped `Nullable` prior to invocation.
- **DO NOT refuse** local/pop-results (§15.3). Directive R2 of WO-5 **officially revoked**.

### 15.5 ⚰️ Revoked Directive + Standing Teeth
- **R2 (refuse local heap `Struct?`) REVOKED** — based on a misreading of §4. Revocation signed by G 2026-07-20.
- Fixture **455** (`local_nullable_heap_struct_control_run`, EXPECT 0) = permanent control protecting
  functioning locals — if anyone accidentally re-refuses it, this fails.
- Fixtures **454/456** (container refusal) + **457/458** (Copy-only anti-over-refusal) = bidirectional teeth.

### 15.6 Pending Debt
- **Container-element `Nullable(Struct-heap)` is currently REFUSED, unsupported.** Supporting it requires
  `emit_vector_element_free_loop`/`emit_hashmap_value_free_loop` to **retain** `Nullable` and route through
  the `struct_drop` arm (like locals), rather than unwrapping upfront. Dedicated campaign.
- ~~Local `Nullable(Struct-heap)` via widening (`let a: Leaf? = plain`) — identical to N1 hole (§13),
  policy-hole, unmeasured individually. Recorded as debt.~~ **CORRECTED 2026-07-27(f), see §16 — the "policy-hole"
  label here was INCORRECT: measured as DETERMINISTIC double-free (UB). Sealed by WO-Aggregate-Move-Tombstone.**

---

## 16. AMENDMENT (WO-Aggregate-Move-Tombstone, 2026-07-27(f)) — §15.6's "Policy-Hole" Corrected to LIVE Double-Free UB; Blast Radius Expands from "Widening" to "Aggregate Moves" in General

### 16.1 Why the Previous Label was Incorrect

§15.6 (written 2026-07-20, WO-5) labeled "Local `Nullable(Struct-heap)` via widening" as
"same as N1 hole, policy-hole, unmeasured individually" — extrapolating by analogy from N1 (§13, nullable
ENUM widening, measured FREE=1 distinct=1, NO UB), WITHOUT direct measurement on structs.
This was an invalid extrapolation: N1 and struct-widening use entirely different lowering sites (N1 via direct construction `E::V(42)`;
struct-widening via `is_struct_widening` branch, `crates/triet-lower/src/lib.rs:2240-2254` legacy). Direct measurement
(2026-07-27(f)) on `04cb5d3`:

```
struct Leaf { s: String }
function main() -> Integer {
    let p = Leaf { s: "hi" };
    let a: Leaf? = p;   // widening
    return 42;
}
```
→ `free(): double free detected in tcache 2` (exit **134**), deterministic across all runs.

### 16.2 The REAL Blast Radius is Broader than "Widening" — All Aggregate MOVEs Missing Tombstones

Broader measurements revealed the bug was NOT restricted to `Struct?` widening — ordinary reassignment syntax
(`Stmt::Assignment`) without any `?` triggered the **exact same mechanism**:

```
let mutable a = Leaf { s: "aa" };
let p = Leaf { s: "hi" };
a = p;   // → 134, NO `?` anywhere in the program
```

JIT root cause (measured directly in `crates/triet-jit/src/mir_lower.rs`, `Statement::Assign`
codegen): for aggregates with `ty_total_size > 8` (any struct with >1 scalar field, or
1 heap field such as `String`), the **"Multi-word copy for struct/enum aggregate"** branch
(`:3081-3122`) performed a RAW word-by-word memcpy, with comments admitting **"Struct/enum
types are Copy in Tier A — no M1 zeroing needed"** — a FALSE assumption for heap-bearing structs.
The "M1 Zeroing-on-Move" mechanism (`:3189-3212`, automatically zeroing the source when
`is_aggregate == false`) executed ONLY in the ELSE branch (scalar / String-as-thin-handle) —
**never touching the aggregate branch**. This is why `String`/`Vector`/`HashMap`/
`Nullable(scalar)`/`Outcome` (all sized ≤8B at the Variable level or maintaining dedicated sync) WERE
already safe — whereas `Struct` AND `Enum` (both >8B in JIT) were exposed.

### 16.3 Fix — Explicit Deinit in Lowerer, DO NOT Modify M1/JIT

`crates/triet-lower/src/lib.rs`, two touch points, mirroring the PROVEN pattern
of `is_move_binding` (`let q = p;`, which possessed Deinit previously):
- **Site A** (`Stmt::Let`'s `is_struct_widening` branch): add `Deinit(v)` immediately
  before `return Ok(())`, guarded by `!ctx_is_copy(&v_ty, c)`.
- **Site B** (`Stmt::Assignment`): add `Deinit(v)` after `Assign` across BOTH branches
  (`Expr::Identifier` — guarded by `v != orig` to avoid self-assignment zeroing `a = a`;
  and field/projection branch `_ =>` — **measured as UNREACHABLE via the current parser**,
  `crates/triet-parser/src/stmt.rs:292` accepts only `Expr::Identifier` as assignment targets,
  all other targets yielding `E0007` parse errors. Deinit added here is unverifiable
  via real fixtures; retained for arm-agnostic robustness and future parser expansions, without claiming
  fixes in this branch).

`Statement::Deinit`'s JIT codegen (pre-existing, unchanged) recursively traverses ALL
heap leaves of the struct (`tombstone_slot_leaves`) — unlike M1's single-word zeroing —
correctly handling multi-level nested structs (§16.4 table, nested struct row).

### 16.4 8-POSITION TABLE — Measured on `04cb5d3` (BEFORE Fix) and Patched Tree (AFTER)

| # | Shape (`struct Leaf { s: String }`) | BEFORE Fix | AFTER Fix |
|---|---|---|---|
| 1 | `let a: Leaf? = p;` (widening) | 🔴 134 (double-free) | ✅ exit 0, correct values |
| 2 | `return p;` inside `-> Leaf?` | ✅ E1121 (refused, unchanged) | ✅ E1121 (unchanged) |
| 3 | `take(p)` param `Leaf?` | ✅ JIT refuse "Struct? Drop without slot" | ✅ unchanged |
| 4 | `Container { f: p }` field init | ✅ E1100 (refused, unchanged) | ✅ E1100 (unchanged) |
| 5 | `push(v, p)` `Vector<Leaf?>` | ✅ MIR verifier B8 | ✅ unchanged |
| 5b | `insert(m, 1, p)` `HashMap<_, Leaf?>` | ✅ MIR verifier B8 | ✅ unchanged |
| 6 | `a = p` with `a: Leaf?` | 🔴 134 | ✅ exit 0, correct values |
| 7 | `a = p` with `a: Leaf` (non-nullable) | 🔴 134 | ✅ exit 0, correct values (⚠ old-dest leak, §16.5) |

Additional variations of #1 measured (BEFORE fix, all three yielded 134): nested structs
(`Outer{Inner{String}}`) → AFTER fix exit 0 correct · field `Vector<Integer>` instead of
`String` → AFTER fix exit 0 correct · **source is function PARAM** → AFTER fix **NOT green,
changed to SIGSEGV (139)** — INDEPENDENT JIT bug, see §16.6, NOT closed in this WO.

Controls remaining green: `let q = p` (is_move_binding, unchanged) · widening enum · Copy-struct via widening
(fixtures 231/234/235/237, `Pt{x,y}` all-scalar — `ctx_is_copy` returns `true` so it is NOT Deinit'd, readable after widening).

### 16.5 New Documented Debt — Old-Dest Leak, NOT Fixed in this WO

`a = p` (case #7) correctly tombstones source `p` (0 double-frees) but DOES NOT drop the OLD value
of `a` prior to overwriting — leaking the old value (measured via pointer deduplication:
2 Strings allocated, only 1 pointer freed, 0 pointers double-freed — see
`crates/triet-driver/tests/aggregate_move_tombstone_counting.rs`,
`reassign_plain_no_double_free_but_leaks_old_dest`). This is an independent semantic decision
(dropping old dest before overwrite), OUTSIDE the scope of this WO — recorded as debt for future campaigns.

### 16.6 New Documented Debt — Deinit on Struct-by-Value PARAM Causes SIGSEGV, NOT Fixed

Measured (2026-07-27(f)): `function take(p: Leaf) -> Integer { let q = p; ... }`
(even WITHOUT widening — pattern `is_move_binding` existed prior to this WO,
NOT a regression of this patch) — SIGSEGV (**exit 139**) on raw `04cb5d3`, PRE-EXISTING
this WO. Root cause: struct-by-value parameters LACK entries in `struct_slots`/`enum_slots`
(`crates/triet-jit/src/mir_lower.rs` prologue `:2684-2771` copies-in String/Enum/Outcome params only, omitting bare Structs) —
the parameter's Cranelift `Variable` IS a raw pointer referencing CALLER memory (aliasing, not copied).
`Statement::Deinit`'s generic scalar fallback (`:2943-2945`, `builder.def_var(self.var(*l), zero)`) zeroes
THAT EXACT Variable/address (correct for regular locals, FALSE for param aliases — destroying the sole handle to the data) →
subsequent `Drop` loads from address `0` → SIGSEGV. Adding `Deinit(v)` in Sites A/B
DID NOT create this bug (it existed in `is_move_binding` previously) — this WO merely opened an additional path
(widening-from-param) encountering IT. Fixing requires updating JIT Deinit codegen (dedicated handling for struct-param-aliases)
or altering how lowerer handles widening-from-params — both requiring G/O review. A fixture for the param variant
(number 540 in initial planning) was **NOT created** — a SIGSEGV in the integration test suite would kill the binary process,
masking all other fixtures.

### 16.7 Why It Survived 7 Days (2026-07-20 §15/WO-5 → 2026-07-27 §16) Undetected

The `is_struct_widening` branch AND all of `Stmt::Assignment` were covered in the EXISTING
fixture suite exclusively via **Copy** structs (`Pt { x: Integer, y: Integer }`, fixtures
231/234/235/237) — `ctx_is_copy(Pt) == true` prevented the Move-tombstone path from ever executing,
even though tests traversed that code branch. This is the core lesson of rule HP.3: **when a guard/branch
applies to N variants (both Copy and Move), teeth must poison/measure EACH variant individually — traversing
a branch with SAFE variants PROVES NOTHING about DANGEROUS variants.**

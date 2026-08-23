# ADR 0076 — Heap-Nullable in Aggregate Fields/Payloads (Intersection of B8 — Closing the Nullable Era)

> # 🩸 CORE PRINCIPLE (Carved in stone by G, 2026-06-29)
> # A language where `let x: String? = …` runs smoothly but `struct S { x: String? }`
> # is rejected by the compiler is a **truncated, inconsistent** language. The struct/enum
> # layout foundations must FULLY BEAR the language's type system before opening any new frontiers.
> # Leave no leaks behind.

**Status:** 🔒 **DECISION (SEALED) — Blood-verified by O + FINAL sign-off by G 2026-06-29** (commit `6327890`, gate `0·0·306·0`).
Applicable to Tier C+. This is the **final step closing the Nullable era**: heap-`T?` (`String?`/`Vector?`/
`HashMap?`) in **struct-field / enum-payload** positions — the sole remaining refused B8 intersection.
**Implementation = SINGLE ATOMIC SLICE** (5 interlocking prongs: gate+layout+drop+construct+borrowck lock together —
lifting gates without layout causes SIGSEGV; having layout without drop causes leaks; assembling in one unified stroke fires the engine, finalized by G).

> **Single slice `6327890` — Blood-verified independently by O across 3 core safeguards (snapshot tests, byte-identical restore):**
> #1 CASE B tombstone (`lower:3700` removing Deinit-after-present-bind → double-free SIGABRT 134 across 3 variants) ·
> #2 **vital** `is_copy(Nullable(heap))==false` (`mir:691` poisoned `→true` → heap-`T?` becomes Copy → no drop → **7 counting tests fail LEAK RED**) ·
> #3 drop-arm prong 3 (`jit:472` removing collect arm → 7 counting tests leak FREE==0).
> **Gap caught by O in round 1:** match-present-bind-move on heap-`T?`-aggregates **compiled to double-free while borrowck remained silent** (NEW due to gate-lift — pre-WO exit 4, round 1 = 134). D resolved STATICALLY (tombstoning outer-Nullable = tag-niche drop-flag, NO dynamic flags). Run-witness fixtures 180/230/236/255/311/312 + 310→E2423.

**Issue — single-intersection incoherence:** heap-nullable was already operational almost everywhere (23 RUN fixtures:
top-level `String?`/`Vector?`/`HashMap?` null/present/use-after-move/match `~+~0`/Elvis/`?+>`-map/method-
return; `Enum?`/`Struct?` aggregates; nested). Only **4 refused fixtures** remained, sharing the same shape:
heap-`T?` as struct fields or enum payloads (`struct S{x:String?}`, `enum Bag{Has(String?)}`).
The compiler accepted them at top-level but rejected them inside aggregates → language incoherence. This ADR closes that gap.

---

## 0. Current Reality (Recon measured by O at file:line, 2026-06-29 — carved in stone, not guessed)

| # | Finding | Evidence (file:line) | Architectural Consequence |
|---|---|---|---|
| R1 | Position-dependent gates, 2 predicates | `triet-mir/src/lib.rs:1431` (`is_lowerable_nullable_payload`), `:1482` (`is_field_payload_lowerable`) | Return/local/param: heap-`T?` IS LOWERED. Field/payload: heap-`T?` REFUSED |
| R2 | Chokepoint refusal | `triet-mir/src/lib.rs:1573-1596` (`Body::verify` scans `struct_layouts`+`enum_layouts` via `find_refused_nullable_field`) | Lifting = relaxing `is_field_payload_lowerable` for heap types |
| R3 | ptr-sentinel ALREADY present | `triet_mir::NULL_SENTINEL` | Null heap = ptr@slot == NULL_SENTINEL (`i64::MIN`) |
| R4 | Drop-shims ALREADY no-op on sentinel | `triet-jit/src/mir_lower.rs:2942, 3214, 3437` (`if ptr == 0 \|\| ptr == NULL_SENTINEL { return }`) | **Conditional-drop DOES NOT require JIT branching** — shim handles condition |
| R5 | `collect_heap_leaves` skips Nullable | `triet-jit/src/mir_lower.rs:450-466` (arm `_ => {}` skips scalars) | Field heap-`T?` currently **leaks** (no drop emitted) |
| R6 | Field sizing is heap-aware | `triet-lower/src/lib.rs:549` (`String → 24`, else 8) | `String?` field reuses `String` layout (24B fat); `Vector?`/`HashMap?` = 8B handles |
| R7 | LeafKind | `triet-jit/src/mir_lower.rs:214` (`{Heap(MirType), Enum(String)}`) | Reusing `Heap(inner)` is viable (see Alternative #1) |
| R8 | Tombstone + free-dispatch sites | `mir_lower.rs:1541` (tombstone zero@abs), `~2041` (free-shim) | Both need to recognize Nullable(heap) |

**Firm boundary:** 4 refused fixtures — `180_heap_nullable_struct_field_refused`,
`230_enum_nullable_heap_payload_refused`, `236_struct_nullable_heap_field_refused`,
`255_field_string_nullable_b8_refuse` — all represented heap-`T?` at field/payload positions. NOTHING else was refused.

---

## Decision

Relax the field/payload gate for heap-`T?`, allocate heap slots (bearing sentinels) at **field offsets**,
and reuse sentinel-safe drop shims. **5 surgical prongs**, connecting directly to the heap-aggregate machinery
(WO-0073/74/75) — WITHOUT major surgery to the value-model.

### Concrete Execution

**Prong 1 — Gate Lifting** (`triet-mir/src/lib.rs:1482`):
`is_field_payload_lowerable(inner, body)` relaxed: `scalar || (Struct/Enum Copy) || inner.is_any_heap()`.
Heap-`T?` fields/payloads pass the gate. (Heap-bearing aggregates still filtered via `is_copy` in Prong 5.)

**Prong 2 — JIT Layout at field offset** (`triet-jit/src/mir_lower.rs`, struct/enum slot sizing):
`String?` field allocated 24B fat `{ptr,len,cap}` @offset (= `String` layout, R6); `Vector?`/`HashMap?`
allocated 8B handle @offset. **Null = store NULL_SENTINEL into ptr@field-offset** (not 0 — distinguishing
moved-out=0 vs null=sentinel; shims no-op on both ensuring sound drops, while preserving sentinel for matching `~0`).

**Prong 3 — Drop-Glue** (`mir_lower.rs:450-466` collect + `:1541`/`:2041` dispatch):
`collect_heap_leaves` adds arm `Nullable(inner) if inner.is_any_heap() => push heap-leaf @abs`.
Drop/tombstone emitted **unconditionally** on ptr@abs → sentinel-safe shim (R4) automatically no-ops when null.
**NO `brif` in Cranelift** — this is an architectural dividend of PA-3c (§Conditional-drop).

**Prong 4 — Construct/Widen into fields** (fixture 255 `Bad{s: ~+ "hi"}`):
`~+ <heap>` materializes actual fat-pointer → store @field-offset; `~0`/null → store NULL_SENTINEL
@field-offset. Reuses top-level widening path (`NullableStructCopy`-analog), mapped to field offset.

**Prong 5 — Borrowck Verification** (Mandated by G — prevent UAM leakage):
Ensure structs/enums containing heap-`T?` fields are classified as **Move (non-Copy)** → equipped with drop-glue + move-tracking;
ADR-0070 `partial_moves` projection-paths cover field states. **DOES NOT unlock partial-heap-field-move-out**
(`let s = b.s` — remains deferred debt under ADR-0070, requiring dynamic drop-flags) — scope of this ADR covers only
construct + whole-struct-move + drop, NOT extracting heap fields individually. **Ruled by G 2026-06-29: encountering
`let s = b.s` (where b.s is heap) → directly return E2423** (maintaining current refusal, NO scope creep).

### §Conditional-drop = sentinel-no-op (Architectural dividend — answer to G)

Question: *"When drop-glue encounters a potentially-null heap field, how does it branch soundly?"*
**Answer: IT DOES NOT BRANCH.** All three field-ptr@offset states are safe under a single Drop instruction:
| Field state | ptr@offset | Drop-shim (R4) |
|---|---|---|
| present | real ptr | free → correct |
| null (`~0`) | NULL_SENTINEL | no-op → sound |
| moved-out (tombstone) | 0 | no-op → sound |
Conditionality is absorbed into the shim. Emitted Drop is idempotent. PA-3c ptr-sentinel (ADR-0041) pays dividends.

---

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-------------|------|------|------------|
| 1 | **New LeafKind `NullableHeap` vs reusing `Heap(inner)`** | NullableHeap: explicit intent | Heap: 0 new variants, since offset+shim match plain heap | **Reuse `Heap(inner)`** (implementer's choice D) — invariant: slot always holds {real ptr, 0, sentinel}. If D finds distinguishing semantics necessary → NullableHeap, documenting rationale |
| 2 | Gate at **typecheck** vs **MIR-verify** | typecheck: early failure | MIR: allows stdlib to DECLARE heap-nullable APIs; only refuses compilation | **Keep MIR-verify** (ruling β, ADR-0062) — consistent across saga |
| 3 | Unlock **partial-heap-field-move-out** (`let s=b.s`) in this ADR | Symmetrical | Requires JIT dynamic drop-flags (ADR-0070 deferred debt); scope explosion | **NO** — keep deferred debt; this ADR only handles construct+whole-move+drop |
| 4 | Null = store **0** vs **NULL_SENTINEL** @field-offset | 0: simpler | Loses distinction between null vs moved-out for matching `~0` | **NULL_SENTINEL** — matching `~+/~0` requires discrimination; drops sound in both cases |

---

## Consequences

### Positive
- Closes the Nullable era: heap-`T?` behaves consistently across all positions (local/return/field/payload).
- Struct/enum layout foundations fully bear the language's types → opens path for next frontiers (Outcome ABI/native layout).
- 4 negative fixtures FLIP → positive run-witnesses (IRON LAW #3, matching 298/302 in WO-0075).
- 0 lines added to value-model; pure extension of existing heap-aggregate machinery.

### Negative
- Field sizing adds Nullable(heap) branch → expands layout-code surface area (surgical, bounded).

### Risks to Mitigate
- **`is_copy(Nullable(String))` MUST = false** — if true → struct classified as Copy → no drop → LEAK. Mandatory safeguard (Prong 5).
- **Null-store incorrect offset/value** → shim reads garbage → SIGSEGV. Counting safeguards + poison tests (Prongs 2/4).
- **Tombstone vs sentinel confusion** → double-free. Move-then-drop FREE==1 safeguard (Prong 3).

---

## Safeguards (Blood-verified by O independently — poison must fail RED, restore via cp WITHOUT git checkout)

| Prong | Safeguard | Poison → Expected RED Failure |
|---|---|---|
| 1 | gate-lift load-bearing | re-add heap-field refusal → 180/230/236/255 return to refused (control: scalar fields still pass) |
| 2/4 | layout + null-store | counting: field `String?` present → FREE==1, null → FREE==0; poison store-0-instead-of-sentinel → `~0` match fails / SIGSEGV |
| 3 | drop arm load-bearing | remove `Nullable(heap)` arm in `collect_heap_leaves` → present field LEAKS (FREE==0 fails RED) |
| 3 | no-double-free | construct→move struct→drop: FREE==1; poison by omitting tombstone → FREE==2 |
| 5 | is_copy non-Copy | poison `is_copy(Nullable(heap))→true` → struct becomes Copy → no drop → LEAK fails RED |
| 5 | UAM | use-after-move on struct with heap-`T?` field → E2420 |

Each prong tests across `String`/`Vector`/`HashMap` variants (SAFEGUARDS sweep full variant space — lesson HP.3).

## Related ADRs

- **Inherits:** ADR-0041 (PA-3c sentinel — conditional-drop dividend), ADR-0062 (top-level heap-nullable
  + ruling β gate-at-MIR), ADR-0065 (nullable aggregate `Enum?`/`Struct?` niche/tag-prepend),
  ADR-0066/0067 (No-Box heap-in-aggregate, `collect_heap_leaves`/drop-glue), ADR-0070 (partial-move
  projection-path move-state).
- **Untouched:** ADR-0068 (Box/recursive — STRICTLY FORBIDDEN). Opens deferred debt: partial-heap-field-move-out.

## Effective Date

- Tier C+ — lift gate + field-layout + drop-arm as each slice lands (blood-verified by O, signed by G per slice).
- Not retroactive. Top-level heap-nullable (ADR-0062/0065) behavior unchanged.

---

## ✚ AMEND — Deferred Debt CLOSED: nullable-heap field move-out (2026-06-29, signed by G)

Prong 5 above noted: *"`let s = b.s` (b.s heap) → directly return E2423... requiring dynamic drop-flags"*.
**The premise "requires dynamic drop-flags" HAS BEEN WITHDRAWN** — proof and resolution mechanism documented
in [ADR-0070 §AMEND Phase 4](0070-partial-move-field-level-move-state.md): the slot ITSELF is the flag (STATIC tombstone),
drop-side `mir_lower.rs:472` already anticipated `0 (moved-out)`, 0 `brif`. `let s = b.s` with `b.s : String?`/
`Vector?`/`HashMap?` now **RUNS** (fixture 310 FLIPPED `*_e2423` → `*_run`). This represents the **FINAL source-reachable
E2423** on Field-Move-out.
Deferred retention: Index/Deref/Payload non-Field projections (Collection-Semantics).

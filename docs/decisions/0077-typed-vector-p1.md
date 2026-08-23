# ADR 0077 — Typed Vector P1 (element-type via type-erasure, built-in element only)

> # 🩸 CORE PRINCIPLE (G khagda 2026-06-30)
> # A language that DOES NOT allow pushing a `String` into a `Vector` is a **useless** language.
> # Ownership is closed; the next spear pierces the **Type-Erasure** defenses of
> # collections. The Element-SIZE of ALL built-ins is a compile-time CONSTANT → NO
> # native-layout required. UserStruct/Enum elements are SILENCED (E-code) at the P1 boundary — this is
> # the bridge to the later native-layout phase, NO entanglement here.

**Status:** 📝 **DRAFT — awaiting implementation + O verification (blood) + G sign-off.** Applied at Level C+.
Open `Vector<T>` where T = built-in (corresponding scalar / String / Vector / HashMap / Nullable);
**REFUSE `Vector<UserStruct>` / `Vector<Enum>`** by-value (→ P2, requires a later native-layout ADR).
An organic continuation of heap-aggregates (ADR-0066/0067/0076) — reusing the tombstone/free machinery.

**Siblings/Inheritance:** ADR-0066/0067 (No-Box heap-in-aggregate, `collect_heap_leaves`/drop-glue),
ADR-0076 (heap-`T?` field — sentinel-no-op free, R4), ADR-0060 (P1/P2 separation pattern).
**DO NOT touch:** ADR-0068 (Box/recursive — FORBIDDEN), native multi-field layout (Option D — defer).
HashMap<K,V> = **a SEPARATE campaign later** (2 element-types K+V, 24B slot, no dedicated
`Type::HashMap(K,V)` typecheck yet) — G has decided to decouple this from the current phase.

---

## Issue

At the backend, `Vector` is **bare / Integer-only**. Typecheck HAS `Type::Vector(Box<Self>)`
(`types.rs:40`) but the lowerer **erases** the element-type to a bare `MirType::Vector`
(`lib.rs:975/1018/1082/1119` — `"Vector"`/`starts_with("Vector<")` → bare). Consequently:
`push(vector_new(), "hi")` → typecheck REFUSES (`expected Integer, found String`). The language
lacks collections capable of holding Strings/heap data. Three coupling points are hardcoded to Integer (measured by file:line):

1. **STRIDE hardcoded to 8:** `vector_layout` is `HEADER+8+8+cap*8` (`mir_lower.rs:3259`); push
   is `old_len*8` + `(new_data as *mut i64).add(old_len)` (`3375/3377`); get is `(data as
   *const i64).add(idx)` (`3416`).
2. **DROP-GLUE is element-blind:** `__triet_vector_free` (`3305`) only performs `dealloc(block)` —
   NO loop to free elements. `Vector<String>` = a **leak pump** (every String element leaks).
3. **ELEMENT-ABI is 1 i64:** `push(vec, elem: i64)` (`3347`), `get → i64` (`3401`). A 24B fat String
   cannot fit in a single register.

---

## Decision

Open **Typed Vector P1** = `Vector<T>` for T with **known built-in size**, via 4 interconnected thrusts.

### P1/P2 Boundary (the crux — clean separation from native-layout)
- **P1 (built-in, CONSTANT element-size):** T ∈ {Integer, Trit, Tryte, Long, Trilean, String,
  Vector\<_\>, HashMap\<_\>, Nullable\<those above\>\}. **Nested `Vector<Vector<String>>` IS ALSO P1** —
  element = 8B handle, inner-size is irrelevant.
- **P2 (requires native-layout):** `Vector<UserStruct>` / `Vector<Enum>` by-value (element-size =
  arbitrary struct layout). **REFUSE via a new E-code in P1** — no silent failure, no panic. This is
  the boundary preventing entanglement with Option D.

The separation holds because P1 only requires element-size for **built-ins (constant 8/24)** + memcpy size-known
+ free-shim per-kind. NO walking struct fields, NO packing registers. **Vector P1 ⊥ native-layout.**

### Thrust 1 — Element-type into MIR: `MirType::Vector` → `Vector(Box<MirType>)`
Mirror the typecheck `Type::Vector(Box<Self>)`. **Blast ~25 sites** to match the bare version (mir/lib.rs 14 ·
lower/lib.rs 10 · jit/mir_lower.rs 1 · borrowck 0) — bounded, mechanical (similar to `Nullable(inner)`
ADR-0062). The erasure point is fixed in the lowerer (`975/1018/1082/1119`): `Vector<E>` → `Vector(Box::new(
lower(E)))`; bare `"Vector"` (no arg) → maintain compatibility = `Vector(Box::new(Integer))` (Default Level A)
OR E-code for missing annotation (implementer-choice D, with justification).

### Thrust 2 — `elem_size(MirType) -> usize` (compile-time constant)
scalar/handle/Nullable(scalar) = 8 · String/Nullable(String) = 24 · Vector/HashMap handle = 8 ·
**Struct/Enum → REFUSE (P1 E-code, do not return size).** ⚠️ DO NOT reuse `ty_total_size`
(jit:483) — it returns 8 for String (incorrect for a 24B stride). Use a SEPARATE helper.
The shim changes `*8` → `*stride`, and `.add(idx)` → byte-offset `idx*stride` on `*mut u8`.

### Thrust 3 — Typed drop-glue: `__triet_vector_free_typed(ptr, elem_kind, stride)`
Loop through `len` elements @stride; for each heap element → call the free-shim according to `elem_kind`
(0=scalar/no-drop · 1=String · 2=Vector · 3=HashMap; Nullable(heap) shares the same kind, sentinel-no-op).
**Reuse the ORIGINAL free-shim + sentinel-no-op (R4 ADR-0076)** — element ptr ∈ {ptr→free, 0/NULL_SENTINEL→no-op}.
The JIT Drop-glue site (`mir_lower.rs` Drop arm for Vector) changes
`__triet_vector_free` → typed variant + passing `elem_kind`/`stride` from `Vector(inner)`.

### Thrust 4 — By-pointer ABI for fat elements
`push`/`get` with fat elements (e.g., 24B String): pass **by-pointer** (push receives `*const elem`,
memcpy `stride` bytes; get returns via sret/out-ptr). Scalars/handles (8B) retain by-value i64 (fast
path, backward-compatible with `Vector<Integer>`). By-pointer ⊥ native-layout (size is a known constant).

---

## Alternatives Considered

| # | Alternative | Pros | Cons | Conclusion |
|---|-----------|---|-------|----------|
| 1 | **Inline element by stride** (Selected) | 1 alloc/vector, cache-local, symmetric to struct-field | shim requires stride-param + by-ptr ABI for fat elements | **SELECTED** — built-in element-size is constant → decouples native-layout |
| 2 | Box every element (uniform 8B ptr) | stride is always 8, ABI is always i64 | +1 alloc/element, +1 indirection, drop = free box then free inner | Rejected — excessive allocation overhead, violates value-semantics |
| 3 | Bundle HashMap into this ADR | One-shot implementation | K+V involves 2 types, 24B slot, no typecheck variant yet | Rejected (G's decision) — separate campaign later |
| 4 | Open `Vector<UserStruct>` immediately | General purpose | Equivalent to native-layout (Option D major surgery) | Rejected — REFUSE at the P1 boundary, bridge to P2 |
| 5 | Keep bare `Vector`, side-map element-type | 0 impact on MIR variant | side-channel = architectural rot (lesson from ADR-0072) | Rejected — explicit element-type in MIR |

---

## Consequences

### Positive
- Collections can contain String/heap/nested data → true data structures, piercing Type-Erasure.
- Explicit element-type in MIR (no side-channels) — foundation for HashMap<K,V> + iteration later.
- Reuse of tombstone/free machinery (continuation, 0 new machines in drop).
- ⊥ native-layout — does not open Option D.

### Negative
- `MirType::Vector(Box)` touches ~25 sites (mechanical).
- By-ptr ABI for fat elements adds a new path (bounded — scalars retain fast i64 path).

### Risks to Mitigate
- **Element-blind Drop-glue P1** → massive leak if free does not loop. Mandatory "Teeth": (`Vector<String>` drop → FREE==N).
- **Incorrect Stride** → misaligned element read/write → SIGSEGV / corruption. Teeth: push multiple Strings, get reads back correctly.
- **`UserStruct` leakage** → arbitrary struct element-size → entanglement with native-layout. Negative tooth: E-code lock.
- **moved-out element / sentinel** → double-free. Teeth: pop-then-drop FREE must be correct.

---

## Teeth (O independent blood verification — poison must be RED, restore cp MUST NOT git checkout)

| # | Tooth | Scenario | Poison → RED |
|---|---|---|---|
| 1 💀 leak | `Vector<String>` push 3 → drop entire array | remove typed-free loop → FREE==0 (leak), instead of 3 |
| 2 💀 double-free | push String, **pop**, drop array (G mandate) | tombstone pop error → FREE==2 / SIGABTR 134 |
| 3 stride | push 3 Strings, get[0/1/2] reads back correct content | stride remains 8 → misaligned read → error/SIGSEGV |
| 4 negative | `Vector<MyStruct>` (UserStruct element) | remove E-code → arbitrary struct element-size → leaks into P2/native-layout |
| 5 backward-compat | legacy `Vector<Integer>` (72 fixture corpus) | regression if fast-path i64 breaks |
| 6 nested P1 | `Vector<Vector<String>>` (element handle 8B) | inner-drop error → leak inner String |

Each tooth must scan element variants (String/Vector?/Nullable(String)) — lesson from HP.3.
G mandate for tooth #2: **array of Strings → pop → drop, memory corruption = neck-wringing.**

## ADR Relationships
Inherits: ADR-0060 (P1/P2 separation), 0066/0067 (heap-in-aggregate drop-glue), 0076 (sentinel-no-op free R4). DOES NOT touch: 0068 (Box FORBIDDEN), native-layout (defer). Paves way for: typed HashMap<K,V> (later campaign), collection iteration / Index-move (Collection-Semantics).

## Effective Date
Level C+ — element-type-MIR + elem_size + typed-free + by-ptr-ABI when landed (O blood verification, G sign-off).
No retroactivity for `Vector<Integer>` (fast-path i64 preserves byte-compatibility).

---

## ✚ AMEND — Re-scope 2-slice + 💀 O's under-scoping error (D enforces, G finalized 2026-06-30)

### 💀 Under-scoping error admitted by O (D correctly blocked via RULE 4 after THRUST 1)
The above draft claimed the block was at **lower erasure** — WRONG/INCOMPLETE. The actual block is at **monomorphic typecheck**: `vector_new()`/`push`/`get` are hardcoded to declare `Vector<Integer>`, with `type_parameters` EMPTY (`env.rs:252/262/291`). `push(vector_new(), "hi")` → **E1003** (expected Integer, found String).
**`Vector<String>` is UNCONSTRUCTIBLE at the source** — Thrusts 1-4 in the backend are merely **hibernating** if typecheck is not opened. Without the 4 backend thrusts, it is NECESSARY but NOT SUFFICIENT → **missing THRUST 5 (typecheck-open)**.
Lesson (repeating WO-0073): *Verify-don't-trust cuts even O's own WO — recon must scan FROM the typecheck funnel down to JIT, not just focus on the backend.* D detected the mine, stopped, and reported (RULE 4) — no hibernation.

### G's Decision — campaign = 2 SLICES

**Slice A — Backend & Storage** (ownership-in-vector machinery, verifying route-lower hand-built MIR):
- THRUST 1 ✓ `Vector(Box<MirType>)` (committed WIP `d0t39d1`).
- THRUST 2 **stride-in-HEADER** (D proposed via RULE 5, G APPROVED): write `stride`+`elem_kind` into the header during `alloc` — NO parameter passing. Avoids the "empty-default-buffer" edge case (vector_new defaults to Integer-8, then push String-24: free reads stride from header → deallocs correctly). Precedent: free already reads cap@header.
- THRUST 3 typed-free: **JIT-EMITTED element-free loop** (NO `_typed` shim — D identified vacuity, O approved 2026-06-30). Reason: shim-internal free = Rust→Rust direct call (like `push`@3380), bypassing the JIT registry → counting harness (symbol swap) cannot see it → **tooth #1 becomes VACUOUS** (FREE is already=0 before poison). A JIT-emitted call goes through `declare_func_in_func`→registry→stub and is countable → poison loop → FREE 3→0 RED. **Architectural Bonus: reuse `emit_heap_free_at` (jit:944) = SINGLE drop-glue source** (currently used for Outcome/struct/enum) instead of duplicating free-by-kind in the Rust shim. At the Vector Drop site (`mir_lower.rs:2163`): `Vector(inner)` where `inner.is_any_heap()` → emit Cranelift loop (len from header, `i` induction, `elem_addr = data + i*stride`, `emit_heap_free_at(elem_addr, inner)` per element — sentinel-no-op R4), followed by the loop, `__triet_vector_free(block)` (existing, deallocs buffer). `inner` is Copy → skip loop (byte-compat). Nested `Vector<Vector<String>>`: element handle 8B → `emit_heap_free_at(elem, Vector(String))` recursively triggers this same loop.
- THRUST 4 (renamed) **shim `pop()`** = move-out from the end of the array (len-1), returns owned element, ownership is COMPLETELY SEVERED (NO clone, NO hole in the middle of the array). This is the ONLY heap-element-out operation in P1.
- Test: hand-built MIR route-lower + counting (push N → drop → FREE==N; push→pop→drop ownership).

**Slice B — Typecheck-open** (piercing source→JIT, structural + expected-type, **AVOIDING generics**):
- **PA1 finalized (G): structural element-check + expected-type (ADR-0072), NO HM-unification

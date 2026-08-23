# ADR 0082 — Aggregate by-value as Collection Elements (Struct/Enum in Vector/HashMap, NO native-packing)

> # 🩸 CORE PRINCIPLE (Proposed by O, awaiting G's decision)
> # `Vector<User>` must work. BUT the cost MUST NOT be the destruction of invariants.
> # **"every value = one i64" (8B-granular)** — the sole anchor keeping the JIT solvable.
> # The fatal flaw is NOT size — but **RECURSIVE DROP-GLUE**: dropping a
> # `Vector<User>` (where User contains `String`) and freeing memory elements = **LEAK**;
> # byte-copying an element pointer and dropping it twice = **DOUBLE-FREE**. This ADR locks
> # the conservative invariant + designs the recursive free engine, and **kills sub-8B packing**
> # (B-β) for this release.

**Status:** 📝 **DRAFT — awaiting G's review + signature. NO code has been written yet.** Applying Level C+.
Open `Vector<UserStruct>` / `Vector<Enum>` and `HashMap<K, UserStruct>` / `HashMap<K, Enum>`
(value-side) by-value. This is exactly the **P2** promised by ADR-0077/078 and REFUSED at the P1 boundary.

**Scope finalized (Approved by G 2026-07-08):** = **B-α** (aggregate by-value element).
- ✅ IN: Struct/Enum as Vector elements, as HashMap VALUES.
- ⛔ OUT — **B-β sub-8B packing** (Trit=1B…): KILLED. Maintain 8B-granular. The i64 value-model is inviolable.
- ⛔ OUT — **B-γ multi-register struct return**: deferred indefinitely.
- ⛔ OUT — Struct/Enum as HashMap **KEYS**: requires recursive hash+eq on aggregates $\rightarrow$ separate campaign.
- ⛔ OUT — `get()` **by-value** of an aggregate element: REFUSED (like String, ADR-0077) — retrieve via `pop`/`remove` (move-out) or borrow via `get_ref` (ADR-0079).

**Siblings/Inheritance:** ADR-0066/067 (No-Box heap-in-aggregate — `collect_heap_leaves`,
recursive drop-glue, `LeafKind`), ADR-0076 (heap-`T?` sentinel-no-op R4), ADR-0077 (Typed Vector P1 —
fat-element ABI stride>8 by-pointer, element-free loop), ADR-0078/080 (Typed HashMap value/key —
`emit_hashmap_free_value`), ADR-0079 (get-borrow — `get_ref` stride-conditional).
**DO NOT touch:** ADR-0068 (Box/recursive — FORBIDDEN), true native multi-field layout (B-β — deferred),
ADR-0081 (get-borrow-mutable — FROZEN, requires deref-assign).

---

## Issue

ADR-0077/078 opened collections for **built-in elements** (element-size fixed at compile-time:
scalar/handle=8B, String=24B). Aggregate by-value is LOCKED at exactly one point:

- **`vector_elem_size` REFUSES Struct/Enum** — `mir_lower.rs:524-531`:
  `Struct(_) | Enum(_) | Capability(_) | Outcome{..} $\rightarrow$ Err(JitError::Unsupported("... by-value
  aggregate elements need native-layout, deferred to P2"))`. This is the only P1/P2 boundary.

Consequence: `Vector<Point>`, `HashMap<String, User>` will not compile. The language has structs and collections,
but CANNOT put structs into collections — exactly the "discarded" behavior that ADR-0077's core principle condemns,
at a deeper level.

**The trap to avoid:** the name "native multi-field layout" tempts us to pack fields sub-8B (Trit=1B) to
"standard C". That is **B-β** — it directly breaks the i64 value-model invariant (JIT load/store every field
via `stack_load(I64, slot, off)`, `mir_lower.rs:633-770`), forcing typed load/store I8/I16/I32 + extension at
EVERY field site, in exchange for a few bytes of density that **NO ONE asked for**. This ADR does NOT do B-β.

---

## Decision

Open **Aggregate-by-value collection elements (B-α)** through exactly **one controlled extension** of the
existing engine, under **one hard-locked invariant**.

### §1 — FOUNDATIONAL INVARIANT (hard-locked, this is the "byte-image definition" G requires)

> **INV-B-α: One layout, two homes, byte-identical.**
> The byte-image of a struct/enum in a **collection cell** = its byte-image in the
> **StackSlot** — SAME `StructLayout`/`EnumLayout` (same field-offset, same 8B-granular size,
> same heap-leaf repr: String=24B fat {ptr@0,len@8,cap@16}, Vector/HashMap=8B handle). NO
> second layout. NO sub-8B packing. `stride = total_size` from `struct_layouts`/`enum_layouts`.

**Why INV-B-α is load-bearing:** recursive drop-glue calculates field-offsets from `struct_layouts`
(`collect_heap_leaves`, `mir_lower.rs:433`). If the image in the cell DIFFERS from the image on the stack (e.g., if someone later packs it to save space), the drop walk reads the wrong offset $\rightarrow$ frees a garbage pointer $\rightarrow$ SIGSEGV/double-free.
A single layout = the drop walk is always correct. This is why **maintaining 8B-granular is SURVIVAL**, not
laziness: it ensures the image on the stack (where fields are `stack_store(I64)`) and the image in the cell (where the drop walk reads) are **the same thing, for free**.

### §2 — Marshal side (cell ingress/egress): LEVERAGE existing fat-element ABI, NO new work

ADR-0077 fat-element ABI is already generic via `stride`, with NO special-case for String:
- **push** (`mir_lower.s:3027-3059`): `stride > 8` $\rightarrow$ pass the `stack_addr` of the element slot $\rightarrow$
  shim `copy_nonoverlapping(elem, dst, stride)` (`4171`). Struct elements are **already present in

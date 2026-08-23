# ADR-0062 — Heap-Nullable repr: ptr-sentinel (`T?` for T ∈ {String, Vector, HashMap})

- **Status:** 🔒 LOCKED — Approved by G 2026-06-18. Drafted by Mentor O 2026-06-18, grounded in MIR/JIT line-cite + precedent of runtime no-op foundation.
- **Date:** 2026-06-18
- **Drafted by:** Mentor O (analyzed String/Vector/HashMap slot layout + cross-referenced free-shim no-op foundation).
- **Signatures:** O ✅ (repr grounded, runtime foundation already prepared) · G ✅ (approved 2026-06-18 — scope locked to String/Vector/HashMap, defer Struct?/Enum?; tombstone mandatory for all Drop phases; invariant `ptr==NULL_SENTINEL` is immutable, `ptr==0` is forbidden).
- **Related:** [ADR-0041](0041-nullable-pa3c.md) (scalar `T?` PA-3c `i64::MIN` sentinel — this repr EXTENDS to heap) · [ADR-0049](0049-fat-pointer-abi.md) (String 24-byte slot `{ptr@0,len@8,cap@16}`, "slot is sole truth") · [ADR-0042](0042-ownership-across-boundary.md) (Deinit tombstone — Phase 5 avoids double-free) · [ADR-0044](0044-arithmetic-range.md) (`NULL_SENTINEL` canary lies outside all ranges).

---

## 1. Context — stdlib requires it, but the compiler cannot lower it

`T?` for **scalar** `T` (Integer/Trit/Tryte/Long/Trilean/Unit) has been operational since ADR-0041:
a single-i64 sentinel `NULL_SENTINEL = i64::MIN` (`triet-mir/src/lib.rs:2334`), a canary N1 proving it lies outside all scalar ranges.

`T?` for **heap** `T` (String/Vector/HashMap) is currently **hard-blocked** at
`Body::verify()` (`triet-mir/src/lib.rs:1440-1464`, `MirError::HeapNullableNotLowered`):
`find_heap_nullable` (1380) searches for `Nullable(inner)` where `inner` is outside the scalar whitelist
(`is_scalar_nullable_payload` 1362) → refusal. The gate reason (ruling β, approved by G 2026-06-18): stdlib
**declares** heap-nullable as an API (`env.get`/`path.parent`/`fs.ray -> String?`); the declaration is harmless (stub `= ~0`), but **compilation** results in a miscompile — a single-i64 sentinel cannot accommodate a 24-byte fat-pointer. The gate is at the LOWERING stage (not typechecking) to allow declarations to pass, while compilation is blocked.

**Consequence:** a function `function read() -> String? = ...` typechecks OK but is impossible for the JIT. This is a true feature-gap, blocking all stdlib optional-return I/O.

## 2. Decision — repr (a) ptr-sentinel, LOCKED

**`T?` for heap uses the SAME slot as `T`, adding no additional bytes.** The null state is encoded
by the **`ptr` field carrying the value `NULL_SENTINEL`** (`i64::MIN`). No boolean flag, no
discriminant word, no boxing.

- Null-check = **ONE i64 comparison** on the `ptr` field, NOT a `memcmp` of the entire slot.
- Widening `T → T?` = **NO-OP at the repr level** (same slot; non-null means `ptr` points to the actual allocation).
- `~0` (null) = write `NULL_SENTINEL` into the `ptr` field.
- Dropping null = **free** and safe thanks to the runtime foundation already performing no-ops on `NULL_SENTINEL` (§4).

## 3. Memory layout — MIR analysis (G requires explicit offsets)

The three heap types have TWO different slot shapes — but the ptr-sentinel applies **uniformly** because all three contain a field carrying a pointer:

### 3.1 String — 2/3-byte stack slot (fat-pointer)
```
offset:  0        8        16
        +--------+--------+--------+
slot:   |  ptr   |  len   |  cap   |     (mir_lower.rs:2301 "Must match StackSlot: ptr@0,len@8,cap@16")
        +--------+--------+--------+
        ↑
   null-check inspects EXACTLY this field: stack_load(I64, slot, 0) == NULL_SENTINEL ?
```
- `String?` is null  → `ptr@0 = NULL_SENTINEL`; `len@8`/`cap@16` = don't-care.
- `String?` is non-null → identical to a standard String (ptr points to buffer; len/cap in slot — ADR-0049 "slot is sole truth").
- Null-check = `stack_load(I/64, slot, 0)` followed by `icmp eq NULL_SENTINEL` — **1 load + 1 cmp**, without touching len/cap.

### 3.2 Vector / HashMap — single i64 handle
```
handle (i64): ptr to [header | len | cap | data...]    (__triet_vector_alloc/__triet_hashmap_alloc -> i64)
              ↑
        handle == NULL_SENTINEL ? = null
```
- `Vector?`/`HashMap?` = the i64 handle itself. Null → handle = `NULL_SENTINEL`.
- len/cap/data reside in the heap header (not in the slot) → null-check = compare handle, **0 dereferences**.
- This is the SIMPLEST case: the i64 handle is already the "ptr field," compared directly.

### 3.3 Why ptr-sentinel applies uniformly
Every heap type reduces to "having one i64 field carrying a pointer" (String: `slot[0]`; Vector/HashMap: handle). Null = that field == `NULL_SENTINEL`. No type requires additional storage for the null state → **0 byte overhead**, adhering to G's principle ("do not create 8-byte garbage boolean flags").

## 4. Perfect alignment with the runtime no-op foundation (already exists — DO NOT build new)

The entire free-shim ALREADY treats `ptr == NULL_SENTINEL` (and `ptr == 0`) as a no-op — the foundation for Phase 3
(conditional Drop) **is already present**, verified by:

| Shim | Location | Behavior on NULL_SENTINEL |
|---|---|---|
| `__triet_string_free` | mir_lower.rs:4024 + test 4786 | no-op (confirmed by test) |
| `__triet_vector_free` | mir_lower.rs:2469-2470 | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| `__TR_hashmap_free` | mir_lower.rs:2692-2693 | `if ptr == 0 \|\| ptr == NULL_SENTINEL` → return |
| string ops (append…) | mir_lower.rs:2198 | guards against NULL_SENTINEL |
| vector get OOB / hashmap key-miss | mir_lower.rs:2575/2848 | RETURNS NULL_SENTINEL (already a null producer) |

**Design consequence:** The JIT can call `free(ptr)` **unconditionally** on a null heap-nullable without crashing — the shim absorbs it. Dropping a null `String?`/`Vector?`/`HashMap?` = free shim = no-op. Phase 3 (conditional Drop) is primarily about **verification + teeth**, not building a new mechanism.
(Conditional logic is still required in borrowck/lowerer for move-out semantics, not solely dependent on the shim — see §8.)

## 5. Rejected Alternatives

- **(b) Separate boolean flag** (`{is_null: i64, ptr, len, cap}` = 32-byte): +8 bytes/value, memory bloat,

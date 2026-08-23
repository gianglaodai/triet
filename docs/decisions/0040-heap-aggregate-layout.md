# ADR-0040: Heap Aggregate Value Model & Layout — Tier A

**Status:** Draft v4 (Round 4 review)
**Date:** 2026-06-05
**Author:** Giang Hoang (value model, semantics), AI (layout, MIR shims, verification)
**Reviewers:** Mentor G (layout, ABI, runtime codegen), Mentor O (semantics, soundness)
**Changes v3→v4:** §1.3 added M4 (Return-escape). §3.2 added Return-escape mechanism. §3.5 fixed loop leak example. §3.7 fixed citation (754-757). §4 added B7 refusal (heap across user-fn boundary). §5 added B7 steps. §7 fixed fixtures 35/36 (return len).

---

## Summary

Decides the value model and memory layout for heap aggregates (String, Vector, HashMap) in Tier A. Locks move-only owned as the default semantics for slice 1 (String, Vector), with ObjectHeader reserved in the layout while refcounting remains unactivated. Runtime safety mechanisms: **Zeroing-on-Move** (JIT writes null to source at **all move-sites**) + **Null-guard-free** (Drop checks `ptr != 0` before invoking free shim) + **Return-escape** (JIT elides Drop for locals contained in Return values). Four types of move-sites: `Assign` (M1), let-Move-type→Assign (M2), CallDispatch consume-arg (M3), Return-values escape (M4). All operations execute via extern "C" shims following the `__triet_pow` precedent.

## Motivation

1. **Copy/Move type-aware borrowck completed** (HEAD `6e2843c`) — borrowck now distinguishes Copy vs Move types, enforcing single-owner move semantics. This provides safety infrastructure for heap types with active destructors.
2. **F1 gap closed** — sticky Moved across Drop → Return triggers E2420. Heap locals moved into payloads and subsequently reused are detected.
3. **String/Vector/HashMap currently missing** — lowerer returns `Err` for all aggregate types. Layout must be finalized before implementing lowering.

---

## §1 — Value Model (Author decision, dual mentor input)

### 1.1 — Move-only owned for Tier A

**Decision:** String and Vector are **move-only owned** — single-owner, no implicit copying, no implicit cloning.

Three facts verified directly from code (not speculation):

| Fact | Location | Significance |
|------|----------|--------------|
| Copy/Move borrowck enforces single-owner | `triet-borrowck/src/checker.rs:586-589` (Δ1), `683-688` (Δ2) | Move types marked Moved on assignment, preventing reuse → single-owner safety |
| `ObjectHeader` LIVE, lacks consumer | `triet-core/src/memory.rs:51-58` | `refcount: AtomicU32` + `reserved: AtomicU32`, `repr(C, align(8))` — defined and tested |
| `&+` strong forms not yet lowered | `triet-jit/src/mir_lower.rs` — no code path for borrow lowering | No active inc/dec of refcount today |

**Consequence:** Refcounting is dead code if enabled immediately — lacking both a producer (strong form lowering) and a consumer (Drop::decrement). Move-only fully leverages the newly built borrowck infrastructure without unnecessary machinery.

### 1.2 — ObjectHeader reserved, refcount = 1 (no inc/dec)

Heap object layout incorporates the complete ObjectHeader, but in Tier A:

- `refcount` is initialized to 1 (compatible with `ObjectHeader::new()`)
- **No increment** (lacks `&+ T` lowering)
- **No decrement** (Drop directly invokes `free` without refcount→0 checks)
- `reserved` = 0 (reserved for drop flags / type tags in Tier B/C)

**Migration path:** When `&+ T` lowering lands (Tier B/C), with an identical layout:
1. Lower `&+ T` → call `ObjectHeader::increment()`
2. Drop with refcount > 1 → decrement; refcount = 1 → execute free
3. **Layout remains unchanged** — backwards binary compatible.

### 1.3 — Drop semantics: Zeroing-on-Move + Null-guard-free + Return-escape

**Problem:** Borrowck is static analysis — it does not modify MIR. Lowerer emits `Statement::Drop` for **all** owned locals at scope end, regardless of whether a local has been moved along certain control-flow paths. Drop-on-Moved is permitted by design (F1 precedent: Return accepts Ended, does not reject Moved).

For Copy types, Drop is a no-op (stack primitives). But for Move types (heap), if the JIT emits unconditional `free(ptr)` for every Drop — double-frees and dangling pointers emerge at ownership boundaries where null guards alone are insufficient.

The claim that "sticky-Moved guarantees Drop will not run" in v1 was **incorrect** — sticky-Moved only affects Return checks (E2420) and VarState transitions; it does not remove `Statement::Drop` from the MIR.

**Decision — Four move-site categories, JIT zeroes or skips at each:**

There are **four** runtime ownership boundaries; the JIT must handle all four:

| # | Move-site | Mechanism | Specification |
|---|-----------|-----------|---------------|
| M1 | `Statement::Assign` plain-source Move-type | After copying value to dest, store 0 into source variable | Below |
| M2 | `let b = a` where a is Move type | Lowerer emits Assign instead of local aliasing (§3.7); JIT zeroes as M1 | §3.7 |
| M3 | `CallDispatch` arg in consumed position | JIT zeroes variable after call, sharing BuiltinShimMeta table (§3.6) | §3.6 |
| M4 | `Return(values)` — values escaping function | JIT elides Drop for locals ∈ values (§3.2) | §3.2 |

**M1 — Assign (in existing infrastructure):**

1. JIT codegen for `Statement::Assign` with Move-type source:
   - Copy i64 value from source to dest (standard)
   - **Store 0 into source variable** (null pointer)
   - JIT determines type via `body.local_decls[source.local.0].ty` → `triet_mir::is_copy`

**JIT codegen for `Statement::Drop` with Move-type local:**

- If local is contained in Return values of current block → **skip** (M4, §3.2)
- Otherwise: invoke `call __triet_<type>_free(ptr)` — shim guards against null.
  In Tier A, null guards reside inside shims (`if ptr == 0 { return; }`), rather than in JIT codegen. JIT-side null check branching is a Tier B optimization (avoiding call overhead on null pointers).

**`__triet_<type>_free` shim takes `ptr: i64`:**

- `if ptr == 0 { return; }` — shim-level null guard (Tier A)
- Computes `header_ptr = ptr - 8`, frees entire allocation

**Why not use the reserved field as a drop flag:** the reserved field resides on the heap, requiring memory loads to check. Null-on-move uses the stack value directly (already present in registers/Cranelift Variables) — saving memory loads without modifying ObjectHeader for drop-tracking. The reserved field remains untouched for Tier B/C.

**Borrowck REMAINS UNCHANGED** — sticky-Moved + E2420 + E2450 are preserved. Zeroing-on-Move + Return-escape represent **runtime mechanisms** complementing static analysis, not replacing it.

### 1.4 — Why refcounting is not enabled immediately

| | Immediate Refcount (Tier A) | Move-only (Tier A) |
|---|---|---|
| Shims required | `alloc`, `increment`, `decrement`, `free` | `alloc`, `free` |
| Increment producer | None (`&+` not yet lowered) | None needed |
| Decrement consumer | Drop with refcount check | Drop = direct free (null-guarded) |
| Dead code lines | ~100 (increment/decrement paths) | 0 |
| Soundness risk | Faulty refcount → silent leak or use-after-free | Move-only + M1–M4 → borrowck catches statically, runtime catches dynamically |

**Conclusion:** Refcounting is needed eventually, but not today. Move-only is immediately protected by borrowck statically and M1–M4 runtime mechanisms dynamically.

---

## §2 — Memory Layout (Implementer — G's domain)

### 2.1 — Object header

Every heap allocation uses `ObjectHeader` from `triet-core/src/memory.rs:51`:

```text
Address:  HEADER_ADDR              BODY_ADDR = HEADER_ADDR + 8
          |                        |
          v                        v
          [ refcount: u32 | reserved: u32 ] [ user data ... ]
          |<--- 8 bytes (64-bit) ------->|
```

- `refcount` @ offset 0: `AtomicU32`, init = 1
- `reserved` @ offset 4: `AtomicU32`, init = 0
- `repr(C, align(8))` — compatible with Cranelift `i64` alignment
- Body pointer = `header_ptr + 8` — pattern matching Objective-C/Swift

### 2.2 — `String` layout

```text
Stack (i64)                         Heap
┌──────────────────┐                ┌──────────────────────────────────────┐
│ body_ptr: i64    │──────────────> │ ObjectHeader (8 bytes)               │
└──────────────────┘                │  refcount: u32 = 1 (reserved)        │
                                    │  reserved: u32 = 0                   │
                                    ├──────────────────────────────────────┤
                                    │ len: i64 (bytes in use)              │
                                    │ cap: i64 (bytes allocated)           │
                                    ├──────────────────────────────────────┤
                                    │ data: [u8; cap] (UTF-8 bytes)        │
                                    └──────────────────────────────────────┘
```

### 2.3 — `Vector<T>` layout

```text
Stack (i64)                         Heap
┌──────────────────┐                ┌──────────────────────────────────────┐
│ body_ptr: i64    │──────────────> │ ObjectHeader (8 bytes)               │
└──────────────────┘                │  refcount: u32 = 1 (reserved)        │
                                    │  reserved: u32 = 0                   │
                                    ├──────────────────────────────────────┤
                                    │ len: i64 (element count)             │
                                    │ cap: i64 (elements allocated)        │
                                    ├──────────────────────────────────────┤
                                    │ data: [T; cap] (contiguous elements) │
                                    └──────────────────────────────────────┘
```

- Tier A: `T = i64` for all elements (generics not yet lowered)

### 2.4 — `HashMap<K, V>` layout

**Deferred to Tier B.**

### 2.5 — Fat pointer representation

On the stack, String/Vector = **1 i64 value** (pointer to body).
No 3×i64 fat pointers — maintaining consistent Tier A ABI.

When len/cap are needed, JIT loads from heap: `len = load(ptr+0)`, `cap = load(ptr+8)`.

---

## §3 — MIR + Runtime Shims (Architecture)

### 3.1 — Shim signatures + Ownership contracts

Pattern following `__triet_pow` precedent (`triet-jit/src/mir_lower.rs:1178-1207`, `triet-driver/src/main.rs:123`):

| Shim | Signature | Per-arg ownership | Description |
|------|-----------|-------------------|-------------|
| `__triet_string_alloc` | `fn(len: i64, cap: i64) -> i64` | copy, copy → new | Allocate String, return body_ptr |
| `__triet_string_from_bytes` | `fn(ptr: i64, len: i64) -> i64` | borrow, copy → new | Copy bytes from read-only memory to fresh heap |
| `__triet_string_free` | `fn(ptr: i64)` | **consume** → void | Free String. No-op if ptr=0. |
| `__triet_string_concat` | `fn(a: i64, b: i64) -> i64` | borrow, borrow → new | Concatenate 2 Strings, return new ptr. a and b are **not** freed. |
| `__triet_string_eq` | `fn(a: i64, b: i64) -> i64` | borrow, borrow → scalar | Equality check: returns 1 (true) or 0 (false). |
| `__triet_string_len` | `fn(ptr: i64) -> i64` | borrow → scalar | Return `len` of String |
| `__triet_vector_alloc` | `fn(len: i64, cap: i64) -> i64` | copy, copy → new | Allocate Vector, return body_ptr |
| `__triet_vector_free` | `fn(ptr: i64)` | **consume** → void | Free Vector. No-op if ptr=0. |
| `__triet_vector_push` | `fn(vec: i64, elem: i64) -> i64` | **consume**, copy → new | Append element (may realloc). **Shim frees old vec if reallocating.** Returns new ptr. |
| `__triet_vector_len` | `fn(ptr: i64) -> i64` | borrow → scalar | Return `len` of Vector |

**Conventions:** consume = caller relinquishes ownership (JIT zeroes after call, Drop is no-op); borrow = caller retains ownership; copy = i64; new = allocated by shim.

### 3.2 — Drop codegen: Null-guard-free + Return-escape

JIT codegen for `Statement::Drop(local)` in block `bb`:

```
if is_copy(type_of(local)) {
    // no-op
} else if local ∈ terminator_return_values(body.blocks[bb].terminator) {
    // M4: Return-escape — value is being returned to caller.
    // Elide free: ownership transfers to caller, caller will Drop.
    // Both E2450 mechanisms remain active because Drop stays in MIR:
    //   1. Drop-before-Return (lowerer Gate B) — borrowck sees Drop
    //      with active loan → E2450.
    //   2. Return-terminator check (checker.rs:720) — borrowck sees
    //      Return of local with active loan → E2450.
    no-op
} else {
    // Move type, not escaping: call free shim.
    // Tier A: null-guard in shim (if ptr == 0 → return).
    // Tier B: move null-check branch to JIT (avoid call overhead on null).
    call ___triet_<type>_free(ptr);
}
```

**Example:**
```
function f() -> String = {
    let s = "hi";     // s = ptr_to_heap
    return s;         // lowerer: flush_all_for_return → Drop(s); Return(s)
                      // JIT: Drop(s) → s ∈ Return values → skip (M4)
                      //      Return(s) → caller receives ptr_to_heap ✓
                      // Caller: Drop(s_caller) → free ✓
}
```

### 3.3 — String literal: `__triet_string_from_bytes`

**Decision:** In-process JIT mechanism. String literals are UTF-8 bytes in `ConstValue::String` of MIR `Body`. JIT emits `iconst(&bytes, len)` → call `__triet_string_from_bytes(ptr, len)` → body_ptr i64.

**⚠️ Lifetime obligation:** Bytes in `ConstValue::String` reside in `Body`. If `Body` is dropped prior to JIT execution → dangling pointer. **Obligation:** `JitContext::compile_multi` must guarantee all `Body` instances outlive the JIT module. Driver retains `Vec<Body>` alive throughout the lifetime of JIT-compiled code.

**Limitation:** Only valid in JIT. For AOT → `define_data`. Note in JIT code: `// AOT: replace with define_data`.

### 3.4 — Minimal new MIR

No new MIR statements needed. Operations route through `CallDispatch` to builtin shims.

### 3.5 — Temporary heap values: documented leak in Tier A

`push_owned` only tracks let-bindings + parameters (Gate B). Intermediate expressions are not pushed → temporaries are never Dropped → leaked:

```
call(__triet_string_concat(s1, s2))  // temporary from concat → leaked

while eq(concat(a,b), c) {           // every iteration: concat creates temp
    ...                              // no let → no Drop → leaked
}
```

(Note: `let result = concat(a,b)` in a loop creates a let-binding → push_owned → pop_scope at iteration end → Drop → null-guard-free → **no** leak. Direct consequence of §1.3.)

Beyond non-let temporaries, another leak source exists in Tier A:
```
s = concat(a, b);  // M1 zeroes temp concat correctly, but old value of s
                   // (overwritten heap ptr) is never freed → leaked
```
Mutable rebinding of Move-type locals: Assign overwrites destination with new value without freeing old value — JIT cannot track that destination previously held heap memory. Belongs to the same temporary-leak class, accepted in Tier A.

**Decision:** Accept leaks for non-let temporaries in Tier A. JIT executes `main()` once and exits → OS reclaims all memory. Explicitly documented.

**Tier B Fix:** Lowerer calls push_owned for temporaries, emitting Drop following expression evaluation.

### 3.6 — BuiltinShimMeta: metadata table in triet-mir, dual consumers

**Problem:** `CallDispatch` args are currently treated as reads (`checker.rs:806`), neither marked Moved nor zeroed. For shims consuming args:

1. **Borrowck unaware** → use-after-free (static)
2. **JIT does not zero** → Drop calls free(old_ptr) → double-free (dynamic)

**Design:** `BuiltinShimMeta` in `triet-mir` — single source of truth, dual consumers:

```rust
// triet-mir/src/lib.rs
pub struct BuiltinShimMeta {
    pub name: &'static str,
    /// Per-arg: true = consume (caller loses ownership)
    pub arg_consumes: &'static [bool],
}
```

**Consumer 1 — Borrowck:** CallDispatch to builtin name → for args in consumed positions, if Move type → mark Moved (prevents static use-after-move).

**Consumer 2 — JIT:** After `call` instruction → for args in consumed positions, emit `store 0` into Cranelift Variable (M3). Subsequent Drop → null guard → no-op.

Initial table: per §3.1 — `push` consumes vec, `free` consumes ptr, remainder borrow/copy.

### 3.7 — Lowerer: let-binding Move-type → Assign (M2)

**Problem:** Lowerer (`triet-lower/src/lib.rs:535-538`) handles `let b = a` via aliasing: `lower_expr(init)` returns existing Local (`Expr::Identifier` arm at `754-757`), `vars.insert(name, local)` — b and a share **the exact same Local**. For Copy types, this is a valid optimization. For Move types: no Assign → no M1 → Zeroing-on-Move never triggers across let → `let b = a; use(a)` compiles cleanly → §1.1 semantics rendered ineffective.

**Decision — M2:** Lowerer differentiates:

```
If init is Expr::Identifier { name } and type of name is Move (is_copy = false):
    1. alloc_local_ty(type_name)  → fresh Local
    2. emit Statement::Assign(dest=new, source=old)  → JIT zeroes old (M1)
    3. vars.insert(name, new)
    4. push_owned(new)
Otherwise (Copy type or init is not Identifier):
    retain existing aliasing behavior
```

---

## §4 — Scope-Out (Intentional Deferrals)

| Item | Rationale | Deferred to |
|------|-----------|-------------|
| **HashMap** | Hash function + bucket table + collisions | Tier B |
| **Real refcounting** (increment/decrement) | `&+` lowering not yet implemented | Tier B/C |
| **Implicit clone** | Violates explicit-strictness | Never |
| **Drop flags** (reserved field) | Zeroing-on-Move + Return-escape sufficient for Tier A | Tier B/C |
| **Generic Vector\<T\>** | Monomorphization not yet available | Tier B |
| **Outcome returns from shims** | C ABI returns i64; Outcome requires 2 values | Tier B |
| **AOT string literals** (define_data) | In-process JIT mechanism is sufficient | Tier B/C |
| **Temporary heap leaks** (non-let) | Gate B only tracks let-bindings + parameters | Tier B |
| **Heap across user-function boundaries** | **B7:** User-fn with Move-type param or CallDispatch to user-fn with Move-type arg → `Err(LowerError)`. Lacks ownership calling conventions to know if callee consumes or borrows — cleanly reject rather than guessing. Use shims for all heap operations in Tier A. | Tier B (requires calling convention + metadata for user-fn) |
| **Aggregates containing heap payloads/fields** | **B8:** Enum constructors / struct literals with Move-type payloads/fields → `Err(LowerError)`. Drop-glue for aggregate-containing-heap does not exist — enum/struct locals reside in StackSlots, not i64 Variables, requiring dedicated machinery (read discriminant, load ptr from slot, call free). Heap values live only in bare locals in Tier A slice 1. | Tier B/C (4.3c: drop-glue for aggregates) |

---

## §5 — Implementation Sequence (String first, Vector second)

### Phase 4.3a — String Tier A

1. `triet-mir`: add `BuiltinShimMeta` struct + `BUILTIN_SHIM_META` table
2. `triet-lower`: `Stmt::Let` with Move-type Identifier init → emit Assign + fresh local (M2, §3.7)
3. `triet-lower`: string literal → `ConstValue::String` + `alloc_local_ty("String")`
4. `triet-lower`: reject user-fn with Move-type param → `Err` (B7, §4)
5. `triet-lower`: reject CallDispatch to user-fn (non-shim) with Move-type arg → `Err` (B7)
6. `triet-jit`: implement String shims (§3.1)
7. `triet-jit`: codegen `Assign` Move-type source → Zeroing-on-Move (M1)
8. `triet-jit`: codegen `Drop` Move-type → null-guard-free + Return-escape check (M4, §3.2)
9. `triet-jit`: codegen `ConstValue::String` → `__triet_string_from_bytes` (with lifetime invariant §3.3)
10. `triet-jit`: codegen `CallDispatch` shim → zero consume-arg vars after call (M3, using `BuiltinShimMeta`)
11. `triet-borrowck`: `CallDispatch` checks `BuiltinShimMeta` → mark Moved consume args (M3)
12. `triet-driver`: register String shims + retain `Vec<Body>` alive (§3.3)

### Phase 4.3b — Vector Tier A

Mirroring String, add Vector entries to `BUILTIN_SHIM_META`. M2, M3, M4 automatically operate for Vector via identical mechanisms.

### Obligations

- **Do not `alloc_local_ty("?")` for heap values** — temporaries must carry real types.
- **Let-Move-types must emit Assign (M2)** — otherwise all null guards are bypassed across let bindings.
- **All heap operations through shims** — user-defined functions with heap types are rejected (B7).

---

## §6 — ADR Dependencies

| ADR | Relationship |
|-----|--------------|
| **ADR-0022** (S6 Ownership) | Move semantics + 5 reference forms |
| **ADR-0025** (Borrow Checker Rules) | E2420/E2450/Drop order |
| **ADR-0026** (Actor/Send Rules) | ObjectHeader refcount + Send derivation |
| **ADR-0037** (Enum Layout) | Enum payload with heap type → Partial-Moved deferral |
| **COPY/MOVE BORROWCK** (`6e2843c`) | is_copy + sticky Moved + E2423 — static infrastructure |
| **ADR-0038** (Comparable trait defer) | `__triet_string_eq` returns 1/0 (equality). 3-way comparison requires Trait → deferred. |
| **ADR-0039** (?-family) | Nullable String: **representation unfinalized.** Note sentinel-0 conflict (moved-out ≡ null value). |

---

## §7 — Verification (Test Plan)

### 7.1 — Test axes

| Axis | Values to sweep | Fixture |
|------|-----------------|---------|
| String length | empty (""), 1 char, multi-byte UTF-8, length > default cap | `33_string_empty`, `34_string_utf8` |
| Concat chain | 2 strings, 3 strings, concat then return len | `35_string_concat` |
| Move chain (M2) | `let a = "x"; let b = a; let c = b;` → return len(c) | `36_string_move_chain` |
| **F1 end-to-end (negative)** | Hand-built MIR: Move-type local → Assign into enum Payload → Return original local → expect E2420. Core F1 test (M1 + sticky-Moved + Return check) with Move type, without requiring enum runtime. | borrowck unit test `f1_enum_payload_move_type` |
| **E2420 use-after-move** | `let a = "x"; let b = a; use(a)` → compile error | borrowck unit test |
| **E2423 field-copy** | struct with String field, project → compile error | borrowck unit test (existing) |
| **E2450 heap** | `&0 s` then Drop s → E2450 | borrowck unit test |
| **Use-after-push (M3)** | `let v2 = push(v, x); use(v)` → E2420 | borrowck unit test |
| Vector push chain | push 3 times (realloc), push then read len | `38_vector_push` |
| Vector move (M2+M3) | `let v2 = v1; push(v2, x)` — v1 marked Moved | borrowck unit test |
| **B7 refusal** | user-fn with String param → `Err` from lowerer | lowerer unit test |

### 7.2 — Invariants

| # | Invariant | Verification Method |
|---|-----------|---------------------|
| 4i-1 | M1: after Assign Move-type, source = 0 | JIT unit test |
| 4i-2 | Null-guard-free: Drop(local=0) does not call free | JIT unit test |
| 4i-3 | String free deallocates exact size | Rust unit test (valgrind) |
| 4i-4 | M3: push consume arg does not double-free | Instrumented allocator: alloc/free balance |
| 4i-5 | Borrowck rejects use-after-push (E2420) | Unit test: BuiltinShimMeta + CallDispatch |
| 4i-6 | `__triet_string_eq` returns 1/0 | Rust unit test |
| 4i-7 | M2: `let b = a` with String → two distinct Locals, Assign in MIR | Lowerer unit test |
| 4i-8 | M4: Return-escape — Drop of local in Return values does not free | JIT unit test: body Return(String), check heap remains live |
| 4i-9 | B7: user-fn String param → `Err(LowerError)` | Lowerer unit test |

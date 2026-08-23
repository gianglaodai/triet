# ADR 0078 — Typed HashMap P1 (value-typed: `HashMap<Integer, T>`, T built-in heap)

> # 🩸 CORE PRINCIPLES (G's Mandate 2026-06-30)
> # "Sparse arrays containing strings" (sparse array / ID-lookup table with String values) are
> # the foundation of every pragmatic data structure. VALUE ownership must be sound: 
> # insertion (insert), removal (remove), and destruction (drop) — without leaking a single byte. 
> # Reuse the Vector P1 engine (ADR-0077); DO NOT reinvent the wheel. 
> # KEY-typed = a different battleground.

**Status:** ✅ **IMPLEMENTED / CLOSED (Phase 1) — G signed 2026-07-01.** Applied Level C+.
Slice A (storage backend, `a0e60d8`) + Slice B/P1b (typecheck-open `HashMap<Integer,V>` end-to-end source).
Value-typed HashMap (V = built-in heap) construct/insert(Move)/remove(move-out `V?`)/drop sound via JIT real-allocator.
O verified through 3 rounds of blood testing (garbage-value root-fix `lower_type` carry value → vacuous-tooth literal-no-drop → named-local poison → RED SIGABRT 134). KEY-typed = Level 2 defer (subsequent ADR).
Expose `HashMap<Integer, T>` with T = built-in (correspondingly scalar / String / Vector / HashMap / Nullable).
**KEY remains hardcoded as Integer** (key-typed = subsequent ADR, involves per-type hash/eq + Comparable ADR-0038).
Continuation of ADR-0077 (Typed Vector P1) — reuse stride / typed-free loop / move-track / by-ptr ABI for VALUE.

**Sibling/Inheritance:** ADR-0077 (Typed Vector P1 — reuse the exact same value-storage engine), ADR-0060 (P1/P2 separation),
ADR-0043 (Original HashMap builtins), ADR-0076 (sentinel-no-op free R4).
**DO NOT touch:** key-typed (`HashMap<String, V>` — Level 2, defer), native-layout (Option D), ADR-0068 (Box PROHIBITED).

---

## Issue — 3 levels of complexity (recon O 2026-06-30, file:line)

HashMap is currently **hardcoded Integer→Integer**. Recon revealed that HashMap is NOT a "Vector pattern × 2" — it consists of 3 distinct levels:

1. **Level 1 — VALUE typing (= identical to Vector):** value only requires store/free/move → **matches the Vector engine** (stride/typed-free/move-track/by-ptr). Full reuse.
2. **Level 2 — KEY typing (NEW, truly "heavy"):** KEY requires **hash + equality per key-type**. `mir_lower.rs:4015-4027`: `hash = k % cap` (integer-modulo), `stored_k == k` (i64-eq) — i64-only. String keys require string-hash (`cap_id_hash`@3155 FNV-1a sample) + `__triet_string_eq`. **Vector elements NEVER require comparison; HashMap keys MUST.** → **DEFER (subsequent ADR).**
3. **Level 3 — typecheck representation:** HashMap = `Type::UserStruct { name:"HashMap", fields:[__key:Integer,__value:Integer] }` (env.rs:336) — NO dedicated `Type::HashMap(K,V)`. MIR uses bare `MirType::HashMap` (mir/lib.rs:498).

**HM-P1 = Level 1 + Level 3.** Level 2 is relegated to the backlog.

---

## Decision

Expose `HashMap<Integer, T>` (value-typed) via several approaches, with **K=Integer hardcoded**, symmetric to Typed Vector P1.

### Approach A — typecheck representation: replace `UserStruct` with dedicated `Type::HashMap(Box<K>, Box<V>)`
- `types.rs`: add variant `HashMap(Box<Self>, Box<Self>)`. Eliminate the pseudo `UserStruct{name:"HashMap",__key,__value}`.
- `extract_type_params` (check/exprs.rs:2274, Vector arm sample): add `(HashMap(pk,pv), HashMap(ak,av))` to walk both slots.
- `env.rs`: declare generic `hashmap_new<V>() -> HashMap<Integer,V>` · `insert<V>(HashMap<Integer,V>, Integer, V) -> HashMap<Integer,V>` · `get<V>(HashMap<Integer,V>, Integer) -> V?` · `remove<V>(HashMap<Integer,V>, Integer) -> V?`. K-slot = hardcoded Integer (NO type-param for key).
- MIR `MirType::HashMap` → `HashMap(Box<MirType>, Box<MirType>)` (repr fidelity; only VALUE drives typed-free because K=Integer Copy). Implementation follows Vector APPROACH 1 (rustc-guided).

### Approach B — slot fat-value: inline value using value-stride (NO boxing)
- Current slot `[key8 | value8 | state1 | pad7]` = 24B; an 8B value-cell **CANNOT hold a 24B String fat value**.
- **Decision: inline-grow** (symmetric to Vector, NO boxing — boxing = +alloc +indirection, rejected in ADR-0077 §option 2). Slot = `[key8 | value@value_stride | state]`; `value_stride` is derived from the value-type (8 for scalar / 24 for String) via **reused `vector_elem_size` helper** (ADR-0077). Probing remains unchanged (key@0, state@fixed offset after value cell).
- `insert` is value-stride-aware: fat value uses **by-ptr memcpy** (as in push@APPROACH 4); **rehash loop** (`mir_lowers.rs:3925`) performs memcpy on value-cells according to stride (NO `v_ptr.read_unaligned()` i64) — this is the "meticulous implementation" mandated by G.

### Approach C — typed drop-glue (JIT-emitted, reuse Vector APPROACH 3)
- HashMap Drop site: iterate `cap` slots, if `state==occupied(1)` → free value@value-cell via **`emit_heap_free_at`** (registry-routed, countable — prevents vacuity as in Vector). KEY=Integer does NOT require freeing. Sentinel-no-op R4.

### Approach D — move-track + take-out
- **insert = Move value:** `arg_consumes` value-arg is element-type-aware (heap→consume, Copy→no-op) — matches the push engine in Region 3 of ADR-0077 (borrowck move-track + M3-zero + JIT).
- **Take-out = `remove(map,key) -> V?` (NEW shim):** move-out value + tombstone slot (state→deleted). Ownership is severed (as in pop). **`get(HashMap<Integer, heap>)` → E1047 REFUSE** (copy-out heap value is prohibited; defer clone/borrow — symmetric to Vector get). `get(HashMap<Integer,Integer>)` is a Copy → still returns `V?`.

### Boundaries (defer — touching this is fatal)
KEY-typed `HashMap<String,V>` (Level 2: hash/eq per-type, Comparable ADR-0038) · get-clone/borrow heap value · `HashMap<_, UserStruct>` (P2 native-layout) · ADR-0068 Box.

---

## Alternatives Considered
| # | Alternative | Conclusion |
|---|-----------|----------|
| 1 | **Inline value by value-stride** (chosen) | reuse Vector machinery, 1 alloc, value-semantics |
| 2 | Box value (cell = 8B ptr→heap value) | rejected — +alloc +indirection (as per ADR-0077 §2) |
| 3 | Bundle key-typed in the same campaign | rejected (G) — Level 2 drags in Comparable ADR-0038, causing failure |
| 4 | get-heap-value copy-out | rejected — clone-shim/borrow-lifetime not yet implemented; use `remove` move-out |

## Consequences
**Positive:** sparse-array/ID-table containing heap values is sound; dedicated `Type::HashMap(K,V)` (replacing pseudo UserStruct) = foundation for future key-typed; reuse of Vector machinery (0 new free engines). **Negative:** `MirType::HashMap` arity changes → requires rustc-guided blast; insert rehash is value-stride-aware (bounded). **Risks:** rehash memcpy uses incorrect stride → corruption (teeth); insert fails to consume heap value → double-free (teeth SIGABRT 134); drop fails to loop through values → leak (teeth).

## Teeth (O's blood verification — poison must be RED, cp-snapshot MUST NOT git checkout)
| # | Tooth | Poison → RED |
|---|---|---|
| 1 💀💀 | insert heap value SIGABRT 134 (G gold standard) | value-arg consume $\rightarrow$ false $\rightarrow$ caller double-free (real-allocator) |
| 2 💀 | drop leak | remove typed-free slot-loop $\rightarrow$ occupied String value FREE==0 |
| 3 | rehash value-stride | poison rehash uses i64-read instead of memcpy stride $\rightarrow$ corruption during grow + fat value |
| 4 | remove take-out | remove move-out + tombstone $\rightarrow$ value freed once via caller; poison tombstone $\rightarrow$ double-free |
| 5 | get-heap refuse | `get(HashMap<Integer,String>)` $\rightarrow$ E1047 |
| 6 | backward-compat | `HashMap<Integer,Integer>` insert/get/remove corpus is green |

## Slices (symmetric to Vector A/B)
- **HM-P1a (backend):** Approach A-MIR + B slot

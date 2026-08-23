# ADR-0043: HashMap Representation & Runtime Shims — Tier B

**Status:** CLOSED — Mentor O SIGNED (semantics & soundness, 2026-06-07) + Mentor G SIGNED (layout/ABI, 2026-06-07). Implementation complete at `247a3be`.
**Date:** 2026-06-07
**Author:** AI (investigation + proposal), final decision: Giang Hoang
**Reviewers:** Mentor G (layout, ABI, codegen), Mentor O (semantics, soundness)
**Scope:** `HashMap<K, V>` with `K=Integer, V=Integer` in Tier B. String keys deferred to Tier C
(entailing hash-by-content + recursive free).

---

## Summary

HashMap is the final gate in the a→c→b trajectory. The prerequisite `get -> V?` is
established in ADR-0041; B7-lift (ADR-0042) opened function boundaries for heap types.
This ADR designs the memory layout + runtime shims for `HashMap<Integer, Integer>` —
minimal scope, leveraging the existing Vector shim template.

---

## §0 — Template (Existing Vector Shim)

Vector layout (`__triet_vector_alloc` at `mir_lower.rs:1465`):
```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: current length
       cap (8B)       i64: capacity
       data[]         i64 elements
```
Pattern: allocate with `Layout::from_size_align(total, 8)`, write header + body fields
via `write_unaligned`, free via `dealloc(header, layout)`. Trap-on-0 in read shims.
All shims are `extern "C" fn(i64, ...) -> i64`.

---

## §1 — Decisions (Q1–Q7)

### Q1: Memory layout

Uses Vector template: open addressing, flat array.

```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: count of live entries
       cap (8B)       i64: capacity (slot count)
       entries[]      array of cap slots, each slot: key(8B) + value(8B) + state(1B)
```

Entry state byte: `0 = EMPTY, 1 = OCCUPIED`. Byte value `2` reserved for
`TOMBSTONE` when `remove` lands (Tier C) — currently lacks producer.

Rationale for dedicated state byte rather than sentinel key: Q6 — `i64::MIN` is a valid
value for `V=Integer`. A sentinel key cannot be used to denote empty slots. A dedicated
state byte allows keys to take any i64 value including MIN.

Total slot size: 8 + 8 + 1 = 17 bytes. Padded to 24 bytes (align 8) so that
key/value fields align to 8 bytes → eliminating the need for `write_unaligned`.

```
Layout: HEADER(8) + len(8) + cap(8) + cap × 24
```

### Q2: Hash function

`K=Integer`: `hash(k) = (k % cap + cap) % cap` — Euclidean modulo, strictly non-negative
even for `k = i64::MIN` (Rust's `%` is a truncating remainder yielding negative results
on negative operands; double-modulo normalizes into `[0, cap)`). Identity hash — no
complex hashing needed for Integer keys.

When String keys arrive (Tier C), replace with a general hash function (FNV-1a or SipHash).

Note: key `i64::MIN` is valid and not rejected. The state byte (Q1) tracks occupancy
independently of the key value, avoiding sentinel keys. Only VALUE is rejected (Q6).

### Q3: Collision resolution — open addressing, linear probing

Open addressing with linear probing: `idx = (hash + i) % cap` for i = 0, 1, 2, …

- Insert: probe until OCCUPIED with matching key → **overwrite value (update)**, len unchanged,
  return. Existing keys do not create new entries.
- Insert (new key): probe until EMPTY → write new entry, len++, state → OCCUPIED.
- Get: probe until OCCUPIED with matching key → return value; until EMPTY → not found (return
  NULL_SENTINEL).
- Remove: NOT in Tier B scope (deferred). TOMBSTONE will be used for remove (Tier C) —
  where insert adds a branch "probe to TOMBSTONE → write entry".

Compared to chaining: open addressing is simpler for C-ABI shims — no separate
allocations for chain nodes, no linked list freeing.

**Termination Invariant:** load factor < 1 guarantees at least 1 EMPTY slot exists →
linear probing terminates in all cases. Reallocating at 0.75 maintains this invariant
(at most ¾ slots OCCUPIED → at least ¼ EMPTY).

### Q4: Load factor + realloc

Default load factor: 0.75 (75%). When `len >= cap * 3 / 4`, capacity doubles:
`new_cap = cap * 2`.

Realloc mirrors the `push` mechanism (tested in fixture 37):
1. Allocate new array with new_cap
2. Rehash all OCCUPIED entries from old array into new array
3. Free old array
4. Return new body pointer

Insert returns a new HashMap (functional style — consume-and-return like push):
`m = insert(m, k, v)`.

### Q5: Drop/free — scope cut

Only `K=Integer, V=Integer`: no heap values reside inside entries → free deallocates
a single flat allocation. No deep free.

`__triet_hashmap_free` guards both `0` and `MIN` → no-op — aligning with the free
contracts of ADR-0041 (`mir_lower.rs:1490` Vector free, `:1337` String free).

String keys + String values deferred to Tier C: requires hash-by-content rather than
identity, and freeing individual Strings prior to freeing the table.

### Q6: MIN sentinel collision (D2)

**Problem:** `get(map, k)` returns `V? = Integer?`. Missing key → returns
`NULL_SENTINEL` (`i64::MIN`). But `i64::MIN` is also a valid `Integer` value — if a
user calls `insert(m, k, i64::MIN)`, `get(m, k)` returning `i64::MIN` cannot distinguish
"present with value MIN" from "absent".

**Decision: REJECT-ON-INSERT — trap (SIGABRT).** `insert` encountering value
`i64::MIN` → `std::process::abort()`. Belongs to family D1 (ADR-0041 §6.2): non-wrapping
arithmetic debt creates phantom nulls; in containers, phantom nulls manifest as
ambiguous lookups.

Rationale for rejection over debt:
- D1 is bounded debt — sentinel lies outside the valid range. In containers, `i64::MIN`
  COULD be inserted legitimately → ambiguity is a bug, not a theoretical edge case.
- When arithmetic wraps mod-3²⁷ (Tier B), `Integer` spans only 27 trits → `i64::MIN`
  remains a sentinel, not a valid Integer value → D2 resolves automatically. But
  in Tier A without arithmetic wrapping, MIN is reachable.

Note: rejection is runtime-only — no compile-time range check for literal Integers
(such a mechanism does not exist; adding it is separate D1 debt). Recorded in
TODO.md as D2 with resolution condition (arithmetic wrap mod-3²⁷).

### Q7: Language surface API

| Builtin | Signature | Shim | Notes |
|---------|-----------|------|-------|
| `hashmap_new()` | `-> HashMap` | `__triet_hashmap_alloc(0, 4)` | `alloc(len=0, cap=4)` — initial cap = 4 |
| `insert(map, k, v)` | `(HashMap, K, V) -> HashMap` | `__triet_hashmap_insert` | consume-and-return; traps if v == MIN (Q6) |
| `get(map, k)` | `(HashMap, K) -> V?` | `__triet_hashmap_get` | key absent → MIN; total function |
| `len(map)` | `(HashMap) -> Integer` | `__triet_hashmap_len` | trap-on-0 |

**BuiltinShimMeta (arg_consumes) — synchronized with borrowck M3:**

| Shim | arg_consumes | Rationale |
|------|-------------|-----------|
| `__triet_hashmap_alloc` | `[false, false]` | len, cap are Copy |
| `__triet_hashmap_insert` | `[true, false, false]` | consumes map; k, v are Copy |
| `__triet_hashmap_get` | `[false, false]` | fixture 47 precedent: get does not consume |
| `__triet_hashmap_len` | `[false]` | fixture 47 precedent |

All use functional style: `insert` returns a new HashMap, consuming the old map. Pattern
`m = insert(m, k, v)` proven live in fixture 65 (Vector push idiom).

---

## §2 — Acceptance Criteria

| # | Criterion | Verification Method |
|---|----------|-------------|
| C1 | `hashmap_new()` → empty HashMap, len = 0 | Fixture |
| C2 | `insert(m, k, v)` → new HashMap, len increments | Fixture |
| C3 | `get(m, k)` after insert → v | Fixture |
| C4 | `get(m, k)` with missing key → NULL_SENTINEL (Elvis → default) | Fixture |
| C5 | `insert` with v = i64::MIN → SIGABRT (reject) | Unit test N7 style (subprocess — spawn child + env var + check signal; driver cannot catch SIGABRT directly) |
| C6 | Realloc when exceeding load factor → no data loss | Fixture (multiple inserts) |
| C7 | `insert` consume-and-return — reuse of old map → E2420 | Fixture |
| C8 | `m = insert(m, k, v)` idiom → functional update works | Fixture |
| C9 | Insert existing key → updates value, len unchanged | Unit test |

---

## §3 — Scope (IN / OUT)

| IN | OUT (defer) |
|----|-------------|
| `HashMap<Integer, Integer>` | `HashMap<K, V>` generic |
| insert/get/len/hashmap_new | remove/contains/keys/values |
| Open addressing, linear probing | String keys (hash-by-content) |
| Reject MIN value on insert | Deep-free for V=String |
| Functional insert (consume-and-return) | Mutable update in-place |

---

## §4 — Implementation Plan

1. **feat(track-b): HashMap shims** — `__triet_hashmap_alloc/insert/get/len/free`
   + register in driver/harness
2. **feat(track-b): HashMap typecheck + lowering** — overload `hashmap_new`,
   `insert`, `get`, `len`; type-string `"HashMap<Integer,Integer>"`. Note
   classifier order: `is_nullable_type` queried BEFORE `is_hashmap_type` (lesson
   from `is_vec_type` swallowing `"Vector<Integer>?"` in ADR-0041 §5.1)
3. **feat(track-b): HashMap fixtures 66-73** — acceptance C1–C8

---

## §5 — Related ADRs / Documents

| Document | Relationship |
|----------|--------------|
| ADR-0040 | Vector shim template, M1–M4 zeroing, arg_consumes |
| ADR-0041 | PA-3c NULL_SENTINEL, get → V?, D1 debt |
| ADR-0042 | B7-lift, Deinit, borrowck M3+ |
| ADR-0037 | Enum StackSlot (unused — HashMap is pure heap type) |

# ADR-0043: HashMap Representation & Runtime Shims — Level B

**Status:** CLOSED — Mentor O SIGNED (semantics & soundness, 2026-06-07) + Mentor G SIGNED (layout/ABI, 2026-06-07). Implementation complete at `247a3be`.
**Date:** 2026-06-07
**Author:** AI (survey + proposal), final decision: Giang Hoàng
**Reviewers:** Mentor G (layout, ABI, codens), Mentor O (semantics, soundness)
**Scope:** `HashMap<K, V>` with `K=Integer, V=Integer` at Level B. String keys deferred to Level C (involves hash-by-content + recursive free).

---

## Summary

HashMap is the final gateway in the a→c→b roadmap. The premise `get -> V?` was established in ADR-0041; B7-lift (ADR-0042) opened the boundary for heap types. This ADR designs the memory layout + runtime shims for `HashMap<Integer, Integer>` — minimum scope, using the current Vector shim template.

---

## §0 — Template (Current Vector shim)

Vector layout (`__triet_vector_alloc` at `mir_lower.rs:1465`):
```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: current length
       cap (8B)       i64: capacity
       data[]         i64 elements
```
Pattern: alloc with `Layout::from_size_align(total, 8)`, write header + body fields via `write_unaligned`, free via `dealloc(header, layout)`. Trap-on-0 in read shims. All shims are `extern "C" fn(i64, ...) -> i64`.

---

## §1 — Decision (Q1-Q7)

### Q1: Memory layout

Use the Vector template: open addressing, flat array.

```
HEADER (8B)           ObjectHeader: refcount(4B) + reserved(4B)
body → len (8B)       i64: number of active entries
       cap (8B)       i64: capacity (number of slots)
       entries[]      array of cap slots, each slot: key(8B) + value(8B) + state(1B)
```

Entry state byte: `0 = EMPTY, 1 = OCCUPIED`. Byte value `2` is reserved for `TOMBSTONE` when `remove` occurs (Level C) — no producer currently exists.

Rationale for choosing a separate byte instead of a sentinel key: Q6 — `i64::MIN` is a valid value for `V=Integer`. A sentinel key cannot be used to mark an empty slot. The state byte allows the key to take any i64 value, including MIN.

Total slot size: 8 + 8 + 1 = 17 bytes. Padded to 24 bytes (align 8) so that key/value are 8-byte aligned → `write_unlamigned` is not required.

```
Layout: HEADER(8) + len(8) + cap(8) + cap × 24
```

### Q2: Hash function

`K=Integer`: `hash(k) = (k % cap + cap) % cap` — Euclidean modulo, always non-negative even with `k = i64::MIN` (Rust `%` is a truncating remainder — results in a negative value for negative operands; double-mod normalizes to `[0, cap)`). Identity hash — no complex hash function is required for Integer keys.

When String keys arrive (Level C), replace this with a general hash function (FNV-1a or SipHash).

Note: the key `i64::MIN` is valid — it is not rejected. The state byte (Q1) marks occupancy independently of the key value, so no key sentinel is needed to distinguish empty slots. Only the VALUE is rejected (Q6).

### Q3: Collision resolution — open addressing, linear probing

Open addressing with linear probing: `idx = (hash + i) % cap` for i = 0, 1, 2, …

- Insert: probe until an OCCUPIED slot with a matching key is found → **overwrite value (update)**, len remains unchanged, return. If the key already exists, do not create a new entry.
- Insert (new key): probe until an EMPTY slot is found → write new entry, len++, state → OCCUPIED.
- Get: probe until an OCCUPIED slot with a matching key is found → return value; if an EMPTY slot is found → not found (return `NULL_SENTINEL`).
- Remove: NOT in scope for Level B (deferred). `TOMBSTONE` will be used for removal (Level C) — at which point `insert` will add a

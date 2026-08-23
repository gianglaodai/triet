# ADR-0047: Read-ops Extension — Tier C Slice 4

**Status:** ACCEPTED — Signed by O + G on 2026-06-08
**Date:** 2026-06-08
**Author:** AI (peer D, implementer)
**Reviewers:** Mentor O (semantics, soundness) · Mentor G (codegen, ABI)
**Scope:** Enable `contains` + `is_empty` for String, Vector, HashMap — pure read-ops via `&0 T`, pass-by-ref, no struct-containing-ref, no new ABI, no fat pointers.

---

## Summary

Slice 2 (ADR-0045) allowed `&0 T` parameters — but the only operation enabled was `length`/`len`. Slice 3 (ADR-0046) opened `-> &0 T` return-borrow. We now draw returns on this foundation: `contains` (search for key/element/substring) and `is_empty` (len == 0) — pure read-ops with no mutation, no new ownership allocations, and no structs containing references.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | `length`/`len` is already enabled for String, Vector, HashMap (Slice 2 §8). Standard pattern: typecheck overload (owned + &0) + lower dispatch on type-string + C shim. | `env.rs:204-349`, `lib.rs:1316-1349`, `mir_lower.rs:1509/1586/1774` |
| F2 | `contains` and `is_empty` DO NOT EXIST yet: no shims, no typecheck overloads, no lower dispatch. | grep confirmed: 0 results |
| F3 | `is_empty` can be derived from `len` (emit `len` shim → compare == 0) — no new shim required. OR a thin shim `__triet_*_is_empty(ptr)->i64`. | |
| F4 | `contains` requires a new shim for each type: String (substring search), Vector (linear scan), HashMap (key lookup). Returns Trilean! (determinate, never Unknown). | |
| F5 | `slice` is HALTED: ref-views require fat-pointer/struct-containing-ref → violates ADR-0046 Q3 (FieldPath::Field was CUT). slice-copy (owned output) is a separate feature — not a read-op. | ADR-0046 Q3, Phase-0 probe |
| F6 | All 3 types have basic shims (alloc/free/len/get/insert) in `mir_lower.rs`. New shims will sit adjacent to existing shims. | `mir_lower.rs:1368-1904` |

---

## §1 — `contains`: New Shims for 3 Types

**Decision:** Author 3 `extern "C"` shims — String, Vector, HashMap. Each shim receives an i64 handle (and key/element), returning i64 (1 = true, -1 = false), strictly never 0.

### String: `__triet_string_contains(haystack: i64, needle: i64) -> i64`

Substring search. Scans haystack bytes for needle. Returns 1 (true) if found, -1 (false) otherwise. **ABSOLUTELY NEVER returns 0** — 0 = Unknown, violating the `Trilean!` refinement type (statically ≠ Unknown).

Location: `mir_lower.rs`, adjacent to `__triet_string_len` (line 1509).

### Vector: `__triet_vector_contains(vec: i64, elem: i64) -> i64`

Linear scan. Iterates over element array, comparing via `==` (i64 equality). Returns 1 (true) if found, -1 (false) otherwise. Never 0.

Location: `mir_lower.rs`, adjacent to `__triet_vector_len` (line 1586).

### HashMap: `__triet_hashmap_contains(map: i64, key: i64) -> i64`

Key lookup. Scans buckets, comparing keys. Returns 1 (true) if key exists, -1 (false) otherwise. Never 0.

Location: `mir_lower.rs`, adjacent to `__triet_hashmap_len` (line 1774).

### Return Type: Trilean! (Not Trilean)

`contains` always returns determinate results (true/false), without the Unknown state present in `get(...)` (which may be null). → Return type `Trilean!` (refinement: statically ≠ Unknown). This allows `if contains(s, "needle")` without requiring an E1033 guard.

### Shim Registration

New shims must be registered in two locations:
1. **Driver** (`main.rs`): `shims` list in `main()` — otherwise JIT cannot locate the symbol.
2. **Harness** (`integration_tests.rs`): `ShimSymbol` list in `run_fixture()` — otherwise fixture tests cannot execute (lesson from §8 Slice 2).

---

## §2 — `is_empty`: Derived from `len` (No New Shim)

**Decision:** `is_empty(X)` → lowerer emits `len(X) == 0`. No new shim needed.

**Rationale:** `is_empty` is purely syntactic sugar for `len(...) == 0`. Deriving it in the lowerer avoids writing 3 shims × 2 languages (Rust shim + Triet wrapper) = 6 implementation points.

**Implementation:**
- Typecheck: `declare_overload("is_empty", fn(X) -> Trilean)` and `declare_overload("is_empty", fn(&0 X) -> Trilean)` — 2 overloads per type.
- Lowerer: when encountering `is_empty`, emit `len(arg)` shim → compare with `ConstValue::Integer(0)` → return Integer (1/-1 = true/false).

**Return Type:** `Trilean!` (like `contains` — determinate, allowing direct use in `if`).

**Encoding Confirmation:** `is_empty(x)` = `len(x) == 0` via JIT `BinOp::Eq` (`mir_lower.rs:1206-1209`) returns `select(cmp, 1, -1)` — inherently correct Trilean: empty → 1 (true), non-empty → -1 (false). No conversion or new shim required. This is a strong rationale for deriving rather than writing shims: inheriting established correct encoding from `Eq`.

---

## §3 — `slice`: SPLIT OUT (Not in this slice)

**Decision:** HALTED — do not implement slice.

**Rationale (3 Tiers):**

1. **Ref-view violates ADR-0046 Q3:** Slices returning references into a sub-range of String require a fat pointer `{ptr_offset, len}` → struct-containing-ref → requires `FieldPath::Field` for return-borrow. `FieldPath::Field` was CUT in ADR-0046 Q3.
2. **Slice-copy (new owned String):** Allocates + copies bytes → new String. This is an independent semantic feature (substring-clone), not a "read-op via &0". Requires its own dedicated ADR for copy/clone mechanisms.
3. **Fat-pointer requires a new ABI:** Currently all values are single i64. Fat-pointer = 2×i64 → breaks ABI. This is an architectural change, not merely "adding an op".

Decided by author (2026-06-08). Recorded in this ADR to prevent future re-proposals.

---

## §4 — Teeth (Positive RUN for Each Op)

Prerequisite condition (lesson from §8 Slice 2, happy-path Slice 3): each operation must have at least ONE RUN fixture yielding expected numerical output.

### contains

| Fixture | Directive | Description |
|---------|-----------|-------------|
| `85_contains_string_run.tri` | EXPECT: 1 | `contains("hello", "ell")` → true (Trilean true=1) |
| `86_contains_vector_run.tri` | EXPECT: 1 | `contains(v, 42)` → true |
| `87_contains_hashmap_run.tri` | EXPECT: 1 | `contains(m, key)` → true |
| `88_contains_miss_run.tri` | EXPECT: -1 | `contains("hello", "xyz")` → false (Trilean false=-1) |
| `89_contains_borrow_run.tri` | EXPECT: 1 | `contains(&0 s, "x")` via borrow + reuse owner |

### is_empty

| Fixture | Directive | Description |
|---------|-----------|-------------|
| `90_is_empty_run.tri` | EXPECT: 1 | `is_empty("")` → true (empty → true=1) |
| `91_is_empty_nonempty.tri` | EXPECT: -1 | `is_empty("hello")` → false (non-empty → false=-1) |
| `92_is_empty_borrow.tri` | EXPECT: 1 | `is_empty(&0 s)` via borrow + reuse owner |

---

## §5 — Implementation Plan

Following the 3-site pattern of `length` (F1):

| # | Task | Primary File | Template |
|---|------|--------------|----------|
| 1 | ADR → commit | `docs/decisions/0047-read-ops-extension.md` | Await O+G signatures |
| 2 | Shim: `__triet_string_contains` | `mir_lower.rs` (adjacent to 1509) | `__triet_string_len` |
| 3 | Shim: `__triet_vector_contains` | `mir_lower.rs` (adjacent to 1586) | `__triet_vector_len` |
| 4 | Shim: `__triet_hashmap_contains` | `mir_lower.rs` (adjacent to 1774) | `__triet_hashmap_len` |
| 5 | Typecheck: `contains` + `is_empty` overloads | `env.rs` (adjacent to 204-349) | `len`/`length` |
| 6 | Lowerer: `contains` dispatch | `lib.rs` (adjacent to 1316) | `len`/`length` dispatch |
| 7 | Lowerer: `is_empty` derive len+compare | `lib.rs` (adjacent to 1316) | New (no shim) |
| 8 | Register shims: driver + harness | `main.rs` + `integration_tests.rs` | 3 shims × 2 locations |
| 9 | Fixtures 85-92 | `fixtures/` | 8 fixtures |
| 10 | Gate + commit | `scripts/gate.sh` | |

---

## Q&A

### O-Q1: Why not write dedicated shims for `is_empty`?

Deriving from `len` → less code, reduced bug surface area. `len` is already available and thoroughly tested. `is_empty` = `len(...) == 0` is semantically exact and sufficient.

### O-Q2: Why does `contains` return Trilean! rather than Trilean?

The result of `contains` is always determinate (found or not found). There is no third state. Returning `Trilean!` allows direct use in `if` conditions without an E1033 guard.

### G-Q1: What is the Shim ABI for `contains`?

Handle i64 passed by-value, identical to all existing shims. No fat pointers, no new ABI.

### G-Q2: Does `is_empty` need an additional shim?

Not required. Lowerer derivation requires 0 new shims. If G wants to optimize (avoiding double-dispatch of len+compare), a thin shim can be added later — but is unnecessary for this slice.

### G-Q3: `slice` is deferred — when will it be implemented?

When fat-pointers/string-views land (Tier D+) or when the author settles substring-clone semantics. It is decoupled from the read-ops slice.

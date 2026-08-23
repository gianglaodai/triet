# ADR-0047: Read-ops Extension — Phase C, Iteration 4

**Status:** ACCEPTED — Signed by O + G 2026-06-08
**Date:** 2026-06-08
**Author:** AI (colleague D, implementer)
**Reviewers:** Mentor O (semantics, soundness) · Mentor G (codegen, ABI)
**Scope:** Expose `contains` + `im_empty` for String, Vector, HashMap — pure read-ops via `&0 T`, pass-by-ref, no struct-containing-ref, no new ABI, no fat-pointers.

---

## Summary

Iteration 2 (ADR-0045) allowed `&0 T` parameters — but the only exposed op was `length`/`len`.
Iteration 3 (ADR-0046) exposed `-> &0 T` return-borrow. Now leveraging the foundation: `contains` (key/element/substring search) and `is_empty` (len == 0) — pure read-ops, no mutation, no new ownership, no struct-containing-ref.

---

## §0 — Facts

| # | Fact | Location |
|---|------|----------|
| F1 | `length`/`len` are already exposed for String, Vector, HashMap (Iteration 2 §8). Standard pattern: typecheck overload (owned + &0) + lower dispatch via type-string + C shim. | `env.rs:204-349`, `lib.rs:1316-1349`, `mir_lower.rs:1509/1586/1774` |
| F2 | `contains` and `is_empty` are NOT yet present: no shim, no typecheck overload, no lower dispatch. | grep confirms: 0 results |
| F3 | `is_empty` can be derived from `im_empty` (emit `len` shim → compare == 0) — no new shim required. OR a thin shim `__triet_*_is_empty(ptr)->i64`. | |
| F4 | `contains` requires a new shim for each type: String (substring search), Vector (linear scan), HashMap (key lookup). Returns Trilean! (deterministic, not Unknown). | |
| F5 | `slice` is DEFERRED: ref-view requires fat-pointers/struct-containing-ref → violates ADR-0046 Q3 (FieldPath::Field has been CUT). slice-copy (owned output) is a separate feature — not a read-op. | ADR-0046 Q3, Phase-0 probe |
| F6 | All 3 types have basic shims (alloc/free/len/get/insert) in `mir_lower.rs`. New shims will be written alongside existing shims. | `mir_lower.rs:1368-1904` |

---

## §1 — `contains`: new shims for 3 types

**Decision:** Write 3 `extern "C"` shims — String, Vector, HashMap. Each shim receives an i64 handle (and key/element) and returns an i64 (1 = true, -1 = false), absolutely never 0.

### String: `__triet_string_contains(haystack: i64, needle: i64) -> i64`

Substring search. Iterate through the haystack bytes to find the needle. Return 1 (true) if found, -1 (false) if not. **ABSOLUTELY do not return 0** — 0 = Unknown, violating the `Trilean!` type refinement (statically ≠ Unknown).

Location: `mir_lower.rs`, alongside `__triet_string_len` (line 1509).

### Vector: `__triet_vector_contains(vec: i64, elem: i64) -> i64`

Linear scan. Iterate through the element array, comparing via `==` (i64 equality). Return 1 (true) if found, -1 (false) if not. Never 0.

Location: `mir_lower.rs`, alongside `__triet_vector_len` (line 1586).

### HashMap: `__triet_hashmap_contains(map: i64, key: i64) -> i64`

Key lookup. Iterate buckets, comparing keys. Return 1 (true) if the key exists, -1 (false) if not. Never 0.

Location: `mir_lower.rs`, alongside `__triet_hashmap_len` (line 1774).

### Return type: Trilean! (not Trilean)

`contains` always returns a deterministic value (true/false), without an Unknown state like `get(...)` (which can be null). → Return type `Trilean!` (refinement: statically ≠ Unknown). This allows `if contains(s, "needle")` without requiring an E1033 guard.

### Shim Registration

New shims must be registered in two places:
1. **Driver** (`main.rs`): the `shims` list in `main()` — otherwise, the JIT will not find the symbol.
2. **Harness** (`integration_tests.rs`): the `ShimSymbol` list in `run_fixture()` — otherwise, fixture tests will fail (lesson from §8 Iteration 2).

---

## §2 — `is_empty`: derive from `len` (no new shim)

**Decision:** `is_empty(X)` → lower emits `len(X) == 0`. No new shim required.

**Rationale:** `is_empty` is purely syntactic sugar for `len(...) == 0`. Deriving at the lower level avoids writing 3 shims × 2 languages (Rust shim + Triết wrapper) = 6 implementation points.

**Implementation:**
- Typecheck: `declare_overlag("is_empty", fn(X) -> Trilean)` and `declare_overload("is_empty", fn(&0 X) -> Trilean)` — 2 overloads per type.
- Lower: when encountering `is

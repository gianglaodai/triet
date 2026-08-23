# ADR 0085 — Full `builtin_shim_meta` table + existence gate in `Body::verify()`

**Status:** Decided (G ✅ 2026-07-24 · O ✅ 2026-07-24). Applicable to Level C+.

Eliminates the SPOF (Single Point of Failure) in the shim metadata table: a `CallDispatch` calling a system shim (`__triet_*`) without a corresponding table entry will now be rejected at the MIR well-formedness gate, instead of being silently swallowed and causing a miscompile.

**Issue:** `triet_mir::builtin_shim_meta(name) -> Option<BuiltinShimMeta>` is a **static table, read by FIVE sites** across three crates (borrowck ×3, JIT ×1, lowerer ×1 — see §Caller Table). All five sites use `if let Some(meta)` / `is_some_and`, so a **missing entry is silently ignored**:

- JIT M3 (`mir_lower.s:4784`) does not zero-on-consume → old heap pointer remains alive.
- Lowerer (`lib.rs:1517`) treats all args as borrows → `push_owned` schedules a Drop for an arg that the shim has already consumed → **double-free** when a heap-consuming shim lacks an entry.
- Borrowck (`checker.rs:1288/1319`) skips mark-Moved and skips mutate-while-borrowed checks → **silent E2420/E2440 bypass**.

This is NOT defense-in-depth (five independent locks) but rather **a single point of failure propagating to five locations**: if the table lies via omission, all five sites fail in the same direction. Currently latent — **eight** shims are missing entries (`__triet_string_contains`, `_hash`, `__triet_vector_contains`, `__triet_hashmap_contains`, `__triet_cap_check`, `__triet_pow`, `__triet_string_append`, `_clear`) and are all **borrow/scalar**, so the default all-borrow behavior happens to be correct (verified body: `__triet_string_append(slot, byte-scalar)` — the second parameter is a byte i64 Copy,

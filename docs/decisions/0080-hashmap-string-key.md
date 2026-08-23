# ADR 0080 — Key-typed HashMap P1 (`HashMap<String, V>`, content hash/eq)

> # 🩸 CORE PRINCIPLE (Giang carved in stone 2026-07-03)
> # `Ord ≠ Hash`. Comparable (ADR-0038, `compare() -> Trit`, TOTAL ORDER) is the WRONG vehicle —
> # mixing order into hashing is the shortest path to architectural collapse. HashMap requires **Hash + Eq**
> # of CONTENT, not `< >`. String keys carry a **BLOOD DEBT** (drop-obligation): every key in
> # a slot is a live heap-allocated String. The #1 concern is NOT "what the hash value is" but **HEAP LEAK &
> # DOUBLE FREE**. Not a single byte shall leak. FORBID `Hashable` trait, FORBID runtime dynamic dispatch.

**Status:** ✅ **APPROVED — Signed by Author + O + G on 2026-07-03.** Constitution passed; WO KM-P1a issued to D. No code yet (not retroactively "IMPLEMENTED" — only promoted when slice lands + O verifies via blood). Applied at Level C+.
Continuation of ADR-0078 (Typed HashMap P1 value) — opening **Tier 2 (KEY typing)** which 0078 left in the backlog.

**Siblings/Inheritance:** ADR-0078 (value-typed HashMap — value-storage / slot-stride / typed-free reuse ENGINE), ADR-0077 (Typed Vector P1 — stride helper), ADR-0049 §6.3 (String repr: heap = `header + data`, NO len/cap on heap), ADR-0079 (get-borrow `&0 container`), ADR-0069 Lát 3 (`cap_id_hash` FNV-1a — template for string-hash).
**REJECTED as a vehicle:** ADR-0038 (Comparable = Ord, not Hash). **NOT to be implemented:** `Hashable` trait, key ∈ {Tryte, UserStruct, Enum, …} (REFUSE), native-layout (Option D), ADR-0068 Box.

---

## Issue — recon by O 2026-07-03 (file:line)

HashMap currently uses **key = identity-hash on i64**. Correct for Integer keys (value = i64); INCORRECT for String keys.
Recon revealed **a real layout wall**, not just a matter of adding a shim:

1. **Key slot is FIXED at 8 bytes.** `mir_lower.rs:4054` — `hashmap_slot_size = 8 + value_stride + 1`.
   Value has a variable `value_stride` (ADR-0078); **the key does not** — always 8B, read/write via
   `hashmap_key_ptr → *mut i64` (`:4075`).
2. **Strings do NOT store `len` on the heap** (ADR-0049 §6.3, `string_layout` `:3428` = `HEADER_SIZE + cap`,
   no len/cap). `{ptr, len, cap}` lives in a **24B fat pointer on the stack**. Hard evidence:
   `__triet_string_eq(a_ptr, a_len

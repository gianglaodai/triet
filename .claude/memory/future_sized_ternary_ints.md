---
name: future-sized-ternary-ints
description: The converged design intent for a sized ternary integer family + literal suffixes. Deferred to its own type-system ADR (post-v0.11). Read when discussing numeric types, kernel footprint, or Integer width.
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ad1339c-d4d2-4c07-9823-7d76ba88d258
---

Discussed 2026-05-31 (while opening v0.11). The author wants numeric types capable of writing a kernel (without wasting resources the way Rust's i8/i16/… family does). **Convergence: most of it ALREADY EXISTS — we nearly built something redundant.**

## Current state (SPEC §2.1) — the ternary family by powers of 3 trits
- `Trit`(1) · `Tryte`(9=3², ±9841) · `Integer`(27=3³, ±3.8×10¹², **the default**) · `Long`(81=3⁴, ±2.2×10³⁸).
- Balanced ternary is symmetric → **there is NO unsigned** → the type family is half the size of Rust's (no `u`, no sign bugs). This is the "simpler" selling point.
- Binary types are the exception and must be marked: `BinaryInteger`(i32) · `BinaryLong`(i64) · `BinaryByte`(u8).

## The author's decisions (not implemented — recorded as intent only)
1. **Do not add new type names.** Keep Trit/Tryte/Integer/Long + the Binary* family.
2. **The only real gap is a 3-trit type** (3¹, ±13) — all it needs is a NAME (TBD; naming is the author's call per [[feedback_implementer_choice]]). Useful for a kernel: small flags/counters packed into 1 byte instead of a 6-byte Integer.
3. **`t3/t9/t27/t81` are literal-typing SUFFIXES**, not type names. Symmetric: ternary uses `tN`, binary uses `iN` (i8/i16/…). To be harmonized with the existing suffixes `_trit`/`_tryte`/`_long` (SPEC §1.5.1) — details to be settled in a later ADR.
4. **`Integer` STAYS FIXED at 27 trits and deterministic on every host.** Making Integer flexible per hardware (binary i64 / ternary i81) is ABSOLUTELY forbidden — it would break ternary-first semantics and break the **byte-identical bootstrap gate** (the very gate v0.11's AOT cache is raising). The resource-saving goal is met by small sized types (footprint) + `Binary*` (native binary speed) — Integer never needs to be touched.

## Light connection to v0.11
- ADR-0033 §5 **already splits the cache by `target_triple`** → the architecture already anticipates different codegen per target hardware; the door is open.
- Keep the JIT/AOT ABI (map_type/shim, [[ADR-0032]]) free of any hard-coded "Integer=i64 is the only integer". Extending the ABI for sized ints is future JIT work, not v0.11.

## Process
A genuine type-system feature → **its own ADR** (e.g. ADR-0034 "sized ternary integer family + numeric width policy") + **its own phase (~v0.12, type system)**. Do NOT fold it into v0.11 (the AOT cache). Per [[feedback_stability_over_speed]]. Related: [[reference_spec]], [[project_vision_os_capable]] (the ternary-hardware scenario is the original motivation).

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
5. **Pattern Reservation Shield (Decision by Giang Hoàng, 2026-08-25 — to implement in the self-hosting `trietc.tri`):**
   - Avoids creating 64+ dummy types in the type table by refusing three reserved identifier patterns
     at **declaration sites** instead:
     - `^T[0-9]+$` (`T1, T3, T9, T27, T81, ...` — ternary integers)
     - `^I[0-9]+$` (`I1, I8, I16, I32, I64, I128, ...` — binary integers)
     - `^F[0-9]+$` (`F16, F32, F64, F128, ...` — floating point numbers)
   - **Error code: `triet::parse::E0010 ReservedPrimitivePattern`.** (`E1002` was proposed first and is
     WRONG twice over: it is already live as typecheck `UndefinedName`, with tests holding it at
     `crates/triet-typecheck/src/check_resolved.rs:966`, and `E10XX` is the typecheck band, not the
     parser's. Parse codes run `E0001`-`E0009`; `E0010` is the next free slot.)
   - **Layer: PARSER, not lexer (Giang ruled 2026-08-25, option (b)).** The lexer has no syntactic
     context — it cannot tell a type NAME from a type PARAMETER, so a lexer-level shield would reserve
     `T1` everywhere and permanently kill `struct Pair<T1, T2>`, the most common multi-parameter naming
     convention (the parser's own docs use it as the canonical example:
     `crates/triet-parser/src/type_expr.rs:47,176`). The parser sees the declaration keyword, so it has
     exactly enough context and no more. Rationale from Giang: this costs nothing at runtime and only
     moves WHEN the error is detected, so the shield belongs wherever it actually serves its purpose.
   - **Radius = 4 declaration sites** (everything that claims a name in the TYPE namespace):
     `parse_struct` · `parse_enum` · `parse_trait` · **`parse_type_alias`** — the type alias was missing
     from the original note and is a real hole (`type I64 = Integer` shadows a future primitive just as
     badly as a struct would).
   - ⚠️ **Do NOT put the guard in the shared `parse_item_name` helper.** It has **10** callers
     (`crates/triet-parser/src/item.rs:165,216,247,277,396,427,623,678,732,768`); the other six are
     function / method / constant / module / capability names plus the trait REFERENCE in `impl X for Y`
     at `:277`, none of which should be refused by this shield. One guard in the helper is the smaller
     diff and the wrong fix.
   - Generic type PARAMETERS are untouched — `<T1, T2>` stays legal.
   - Verified 2026-08-25: **no `.tri` file currently uses `T*`/`I*`/`F*` as a type name or a generic
     parameter**, so the shield can land without migrating anything.

## Light connection to v0.11
- ADR-0033 §5 **already splits the cache by `target_triple`** → the architecture already anticipates different codegen per target hardware; the door is open.
- Keep the JIT/AOT ABI (map_type/shim, [[ADR-0032]]) free of any hard-coded "Integer=i64 is the only integer". Extending the ABI for sized ints is future JIT work, not v0.11.

## Process
A genuine type-system feature → **its own ADR** (e.g. ADR-0034 "sized ternary integer family + numeric width policy") + **its own phase (~v0.12, type system)**. Do NOT fold it into v0.11 (the AOT cache). Per [[feedback_stability_over_speed]]. Related: [[reference_spec]], [[project_vision_os_capable]] (the ternary-hardware scenario is the original motivation).


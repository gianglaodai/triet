# ADR 0038 — `Comparable` Trait with `compare() -> Trit` (Design Lock, Implementation Deferred)

**Status:** **Approved — DESIGN LOCK** (Author + Mentor, 2026-06-05).
**Implementation:** **DEFERRED** — awaiting Trait system landing. DO NOT build temporary built-ins.

## Context

The author desires a ternary-native 3-state comparison operation: instead of three
binary comparisons `a < b` / `a == b` / `a > b` (binary mindset), a single `compare(a, b)`
returns 3 states for 3-way `match` branching. This is the most ternary-aligned
operation possible — comparison equals the sign of the difference, and the SPEC
already provides the foundation: `function sign(n: Integer) -> Trit`
(SPEC.md around §1295) + the principle that "the sign function is the first
non-zero MSB trit — no dedicated comparison instruction needed" (SPEC.md around §486).

Concurrently, the author confirmed: **Triet will definitely use Traits** (not Interfaces).
As of now (2026-06-05), traits do not yet exist: AST `Item` contains only
Function/Struct/Enum (`ast_item.rs:112-120`), and the lexer lacks the `trait`/`impl`
keywords; "trait" only appears as `GenericBound` and paper-level built-in protocols
(Iterator ADR-0003 not yet landed, Display).

## Decision (4 Locked Points)

1. **`Comparable` is a TRAIT** (not an Interface, not a special-cased built-in protocol),
   with method `compare() -> Trit`. Implemented when the Trait system lands —
   Comparable is the first passenger on the Trait vehicle, NOT a pretext to
   gold-plate the entire trait system prematurely.
2. **The return value is `Trit`** (Negative = less, Zero = equal, Positive = greater) —
   **DO NOT** create `enum Ordering {Less, Equal, Greater}`. Rationale: (a) user
   enums use i64 discriminants (ADR-0037) — using an enum for ordering betrays
   ternary identity; (b) Trit ALREADY IS the 3-state type; (c) leverages existing
   `sign`. The name `Ordering` is also occupied by atomic memory ordering (ADR-0026,
   SPEC.md around §1098). Named constants (`less`/`equal`/`greater` =
   `-1_trit`/`0_trit`/`1_trit`) can be provided for readable matching — still
   fundamentally Trit.
3. **`compare` applies only to TOTAL orderings** (Integer/String/Tryte/…, without
   unknown). Comparisons involving **Trilean/unknown** REMAIN with the operators
   `==`/`<` (Ł3-aware, returning Trilean) per SPEC §4.2 — preserving the core
   identity that "compare with unknown ⇒ result unknown" (SPEC.md around §653)
   without being subsumed by `compare`. If partial ordering is needed later,
   `Trit?` (null = incomparable) will be used — decided later, not locked here.
4. **Operators `<` `<=` `>` `>=` `==` `!=` remain unchanged** (SPEC §4.2,
   returning Trilean). The two surfaces remain distinct via their return types:
   operators = Trilean-aware for branching; `compare` = Trit for `match`/sort.
   Do not desugar operators via compare in Tier A.

## Rationale: Why Defer (Not Laziness)

- **Temporary built-ins = throwaway code:** special-casing `Comparable` now creates
  a known skeleton destined for destruction when the real Trait system lands —
  violating Direction A (defer cleanly, do not ship temporary workarounds).
- **No consumers yet:** backend is in Tier A — no Vector to sort, no generic bounds
  to constrain `T: Comparable`. Building it now yields dead code with zero callers
  (lesson from `enum_layouts` dead-fields).
- **No blocking dependencies:** comparison operators (SPEC §4.2) are fully
  functional; only the 3-way matchable form waits.

## Implementation Notes (For Future Phase)

- `match compare(a,b) { -1_trit => …, 0_trit => …, 1_trit => … }` matches on
  **Trit literals** — a lowering path DIFFERENT from enum-match 4g (SwitchInt
  keyed on `enum_layouts`). Requires a dedicated match-on-Trit path.
- Implementation trigger: Trait system lands (trait declarations + impls +
  minimal dispatch) OR stdlib requires sort/BTree — whichever comes first
  brings Comparable along.

## References

- SPEC.md §4.2 (comparison operators, Ł3), around §486 (sign function), around
  §653 (unknown identity), around §1295 (`sign -> Trit`).
- [ADR-0003](0003-iterator-protocol.md) — Iterator protocol (not yet landed;
  companion to Comparable on Trait system).
- [ADR-0037](0037-enum-tagged-union-layout.md) — i64 discriminant for user enums
  (rationale against using enums for Ordering).
- [ADR-0026](0026-actor-boundary-send-rules.md) — `Ordering` (atomic) already
  occupies the name.

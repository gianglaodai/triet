# ADR 0038 — `Comparable` trait with `compare() -> Trit` (design lock, implementation deferred)

**Status:** **Approved — DESIGN LOCK** (Author + Mentor, 2026-06-05).
**Implementation:** **DEFERRED** — awaiting Trait system landing. NO temporary built-ins.

## Context

The author intends to implement a ternary-native 3-state comparison: instead of three separate operations `a < b` / `# a == b` / `a > b` (binary logic), a single `compare(a, b)` returns three states to facilitate `match` branching. This is the most ternary-native operation possible—comparison is equivalent to the sign of the difference, and the SPEC already provides the foundation: `function sign(n: Integer) -> Trit` (SPEC.md §around 1295) + the principle that "the sign function is the first non-zero Trit MSB—no separate comparison required" (SPEC.md §around 486).

Additionally, the author confirms: **The language will definitely feature Traits** (not Interfaces). As of now (2026-06-05), this is not yet implemented: the AST `Item` only includes Function/Struct/Enum (`ast_item.rs:112-120`), the lexer lacks `trait`/`impl` keywords; "trait" currently only appears as `GenericBound` and built-in protocols on paper (Iterator ADR-0003 has not landed, nor has Display).

## Decision

1. **`Comparable` is a TRAIT** (not an Interface, not a built-in protocol special-case), with the method `compare() -> Trit

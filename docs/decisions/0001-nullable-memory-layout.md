# ADR 0001 — Memory layout of `T?`

**Status:** Decided, language spec constraint. Applicable from v0.1; revisit in v0.3 if packing becomes a true bottleneck.

**Issue:** SPEC §13 #1 — How is `T?` stored in memory: a separate discriminator (1 trit/byte indicating nullability) or a sentinel value (occupying an "unused" representation of `T`)?

## Decision

**Discriminator** (1 auxiliary trit) for all `T?`. Standard layout:

```
T?  ::=  is_null: 1 trit  +  payload: T
         (-1 = null, +1 = present, 0 reserved for v0.2 "uninitialized")
```

## Rationale

- **Ternary symmetry.** The philosophy is balanced-ternary first — every n-trit representation of a type carries semantic meaning. `Trilean` uses all three values; `Tryte`/`Integer`/`Long` use symmetry around 0. There are no spare "sentinel slots" to exploit → the sentinel approach would force a reduction in the range of `T`, violating the §3.2 guarantee ("symmetric range").
- **AI-first / regular > exception.** A unified discriminator for all `T`. LLMs or developers do not need to memorize a lookup table for "where type X's sentinel resides." Fixed layout = correct generation on the first attempt.
- **Minimal overhead.** 1 trit ≈ 1.585 bits. With 5-trit/byte packing (§3.4), `T?` typically incurs an additional 1 byte (due to rounding up), but this can be packed alongside discriminators of adjacent `T?` types in optimized backends (v0.3+).
- **`?T` 3-state expands naturally.** The discriminator already utilizes three balanced ternary values → `is_null` can be extended with a third state ("uninitialized" / "moved-from") for the v0.2 borrow checker without breaking the layout.

## Consequences

- Each `T?` field in a struct (v0.2) must reserve an additional 1 trit. The compiler may pack multiple discriminators into fewer bytes (similar to Rust's niche optimization) when `T` possesses a "natural sentinel" — however, that is a v0.3+ optimization and **does not change the semantics**.
- Assumed ternary hardware backends (v2.0+) benefit: the discriminator is a true trit, rather than a hack via binary bit-packing.
- Nullable composition (`T??`) does **not** flatten — `T??` is `(is_null₂, (is_null₁, T))`, with two distinguishable layers. SPEC §2.5 implicitly prohibits this composition (preferring `Option<Option<T>>` in v0.2 if necessary), so no additional complexity arises.

## Implementation v0.1

The interpreter uses Rust's `Value::Null` enum variant — semantically equivalent to the discriminator (Rust enum tag = discriminator). The physical layout will be committed in v0.3 upon the arrival of the bytecode VM.

---

## Addendum — v0.7.4.3-error (null literal unification)

Per [ADR-0020 §10](0020-outcome-error-handling.md) (2026-05-17), the source-level literal for the Trit::Zero discriminator state is unified across the language:

| Pre-v0.7.4.3 | v0.7.4.3+ (canonical) | v1.0+ |
|---|---|---|
| `null` (deprecated synonym, W2001 warning) | `~0` | `~0` (only — `null` removed, E2002) |

**No change to memory layout.** The 1-trit discriminator + payload union encoding locked in this ADR is unchanged. Only the source syntax for the Trit::Zero state literal changes:

```triet
// Pre-v0.7.4.3 (still works through v1.0 with W2001 warning):
let user: User? = null

// v0.7.4.3+ canonical (no warning):
let user: User? = ~0
```

The lowerer accepts both source forms and emits the same `Constant::Null` IR opcode (see [ADR-0010 Addendum — v0.7.4.3-error](0010-ternary-native-ir.md#addendum--v0743-error-null-literal-unification)). No IR / wire-format change.

**Pattern match implications:** patterns for `T?` types must use explicit `~+ binding` for the success arm (no implicit T ⊂ T? widening in pattern position per ADR-0020 §10.4):

```triet
// Pre-v0.7.4.3 widening match (still works with W2001 if `null` used):
match maybe_user {
    user => greet(user),       // implicit T ⊂ T? widening — DEPRECATED in patterns
    null => prompt_login(),
}

// v0.7.4.3+ canonical:
match maybe_user {
    ~+ user => greet(user),
    ~0      => prompt_login(),
}
```

**Migration:** `dao fmt --fix --migrate-null` auto-rewrites both literal and pattern occurrences. See [ADR-0020 §10.5](0020-outcome-error-handling.md) for tool specification.

---

## Addendum — 2026-06-06 (ADR-0041 review: trit assignment + `T??` flatten)

Per [ADR-0041](0041-nullable-representation-bac-a.md) (2026-06-06), two
clauses in this ADR's original body are overridden by later LOCKED decisions:

### 1. Trit assignment table

The original body assigns `is_null: -1 = null, +1 = present, 0 reserved`.
[ADR-0020 §10.1](0020-outcome-error-handling.md) (2026-05-17, LOCKED) assigns
**`+1 = value, 0 = null, -1 = reserved/error`**. The v0.7.4.3 addendum above
changed the *syntax* for the null literal but did NOT update the trit encoding
table — the two ADRs have been in conflict since ADR-0020 was locked.

**Correction:** The canonical trit encoding for `T?` is:

```
T?  ::=  discriminator: 1 trit  +  payload: T
         (+1 = value ("present"),  0 = null,  -1 = reserved)
```

This matches ADR-0020 §10.1 and the entire `~+`/`~0`/`~-` operator family.
The encoding in the original body (`-1 = null, +1 = present`) is **superseded**.

### 2. `T??` non-flatten

The original "Consequences" section states: "Nullable composition (`T??`) **does not**
flatten — `T??` is `(is_null₂, (is_null₁, T))`, with two distinguishable layers."

[ADR-0039](003<0xA0>9-nullable-operator-family.md) (2026-06-05, LOCKED) overrides
this: **`T??` does not exist — auto-flatten.** Applying `?` to an already-nullable
type is a no-op at the type level; the typechecker folds `T??` → `T?`.

**Correction:** `T??` auto-flattens to `T?`. The two-layer discriminator model
in the original body is **superseded** — no backend ever implemented it, and
the rewrite (Track B) enforces C6 of ADR-0041: `T??` does not exist.

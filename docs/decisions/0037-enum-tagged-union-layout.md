# ADR 0037 — Enum Tagged-Union Layout on Tier-A StackSlot

**Status:** **Approved** (Author + Mentor sign-off 2026-06-05).

## Summary

Memory model specification for user-defined `enum` types on the Tier-A infrastructure (Cranelift StackSlot, all values i64 / 8-byte aligned). Three parts: (1) `EnumLayout` — size + alignment of tagged unions on the stack, (2) new MIR statements/terminators required to initialize, read discriminants, and dispatch matches, (3) ownership semantics: construct-move, destructure-copy (asymmetric, but both conform to pre-existing borrowck behavior).

---

## §1 — EnumLayout (Implementer)

### 1.1. Size & alignment

Tier-A constraint: all values are i64, 8-byte aligned. Enums follow the same principle:

```
┌──────────────────┬──────────────────────────────────────────┐
│ discriminant     │ payload (union of variant payloads)      │
│ i64 (8 bytes)    │ max_payload_size bytes                   │
│ offset 0         │ offset 8                                 │
└──────────────────┴──────────────────────────────────────────┘
```

- **discriminant:** `i64` at offset 0, alignment 8.
- **payload:** at offset 8 (already 8-byte aligned). Size = maximum of all variant payload sizes.
  For variants without a payload (unit variants), the payload area is unused —
  but still occupies space in the layout (sized-by-max).
- **total_size:** `8 + max_payload_size`, rounded up to alignment 8 (typically already aligned
  since max_payload_size is also a multiple of 8 in Tier A).
- **alignment:** 8.

**Example:** `enum Option<T> { Some(T), None }` with `T: Integer` (payload = 8 bytes):
total_size = 8 + 8 = 16 bytes.

**Example:** `enum Color { Red, Green, Blue }` (all unit variants, no payload):
max_payload_size = 0 → total_size = 8 + 0 = 8 bytes (contains only the discriminant).

### 1.2. Discriminant encoding

| Variant | Discriminant (i64) |
|---------|-------------------|
| First | 0 |
| Second | 1 |
| n-th | n-1 |

Numbered from 0 in declaration order in the source. This is a **deterministic** mapping
— identical source enums produce identical discriminants across all platforms (deterministic compilation
per ADR-0012 reproducibility).

**No Trit encoding for user-defined enums.** `T?` and `T~E` use trit discriminators
because they are built-in sum types with ≤ 3 states. User-defined enums have arbitrary n variants
(n ≥ 2, potentially > 3) → require integer discriminants. Using i64 maintains uniform representation
with Tier A's "everything i64" model.

**Discriminant value in MIR:** `ConstValue::Integer(i128)` or literal i64.
The JIT will perform `stack_store(I64, slot, 0)` to write the tag.

### 1.3. `EnumLayout` struct (proposed for `triet-mir`)

```rust
/// Layout of a user-defined enum on the stack (tagged union).
pub struct EnumLayout {
    /// Enum name (e.g., "Option").
    pub name: String,
    /// Byte offset of the discriminant field (always 0 for Tier A).
    pub discriminant_offset: usize,
    /// Size of the discriminant in bytes (always 8 for Tier A — i64).
    pub discriminant_size: usize,
    /// Byte offset of the payload union (always 8 for Tier A).
    pub payload_offset: usize,
    /// Total size in bytes, rounded up to `alignment`.
    pub total_size: usize,
    /// Required alignment (always 8 for Tier A).
    pub alignment: usize,
    /// Per-variant metadata.
    pub variants: Vec<VariantLayout>,
}

/// Metadata for one enum variant.
pub struct VariantLayout {
    /// Variant name.
    pub name: String,
    /// Integer discriminant value (0, 1, 2, …).
    pub discriminant_value: i64,
    /// Payload layout, if the variant carries data.
    pub payload: Option<PayloadLayout>,
}

/// Layout of a variant's payload.
pub struct PayloadLayout {
    /// Size of this variant's payload in bytes.
    pub size: usize,
    /// Alignment of this variant's payload.
    pub alignment: usize,
    /// If the payload is a struct/tuple, field layouts keyed by name.
    pub fields: Vec<FieldLayout>,
}
```

### 1.4. Interaction with StructLayout

An enum payload can be a struct (named fields), a tuple (positional fields), or a
single scalar. `PayloadLayout.fields` shares `FieldLayout` with `StructLayout` —
identical offset-within-payload mechanism, identical 8-byte alignment in Tier A.

`EnumLayout` and `StructLayout` are **distinct** — an enum has a discriminant + union payload,
whereas a struct has fixed fields. No inheritance, no composite pattern. Rationale: borrowck needs
to distinguish enum projections (Downcast→Field) from struct projections (direct Field).

---

## §2 — MIR Statements & Terminators (Architecture)

### 2.1. Construction pipeline

To construct `let a = Enum::Variant(x)`:

```
EnumAlloc(dest: _e, enum_name: "Option")
    → allocate StackSlot of size EnumLayout.total_size
SetDiscriminant(dest: _e, value: 0i64)
    → write discriminant to offset 0
Assign(dest: _e.Payload("Some"), source: Place::local(x))
    → copy/move payload into offset 8 of slot
```

#### `Statement::EnumAlloc`

```rust
EnumAlloc {
    /// Destination local.
    dest: Local,
    /// Enum name — key into `Body::enum_layouts`.
    enum_name: String,
    /// Source location.
    span: Span,
}
```

Similar to `StructAlloc` — only allocates stack space, writes no data. JIT creates a
`StackSlot` with `total_size` and `alignment` from `EnumLayout`.

#### `Statement::SetDiscriminant`

```rust
SetDiscriminant {
    /// The enum local to write into.
    dest: Local,
    /// Integer discriminant value (0, 1, 2, …).
    value: i64,
    /// Source location.
    span: Span,
}
```

Writes `value` to `enum_slot + discriminant_offset` (offset 0) as an i64.
Separated from `Assign` because:
- Discriminant is not a standard "place" — it has no independent address and cannot be borrowed.
- `SetDiscriminant` is an unconditional write and does not read a source.
- Borrowck does not need to track loans on the discriminant (tags cannot be borrowed).

**Why not use `Assign` + `Projection::Discriminant`?** Because `Projection`
is used to *track borrows* at the field level. Discriminants are never
borrowed separately — they are enum metadata, not user-accessible fields.
Using a dedicated statement keeps borrowck simple.

#### Payload assignment

Uses the existing `Statement::Assign`, with source `Place::local(x)` and dest
`Place::local(_e)` with projection chain `[Payload("Some")]` (for single variants)
or `[Payload("Some"), Field("value")]` (for variants with named struct payloads).

New `Projection::Payload(String)` — see §2.3.

### 2.2. Match dispatch

To match `match a { Variant1(x) => bb1, Variant2 => bb2, ... }`:

```
// Read discriminant
_disc = discriminant of _a  (read from offset 0 of slot)
// n-way Switch
SwitchInt(discriminant: _disc, cases: [(0, bb_variant1), (1, bb_variant2)], default_bb: bb_trap)
```

#### New Terminator: `SwitchInt`

```rust
SwitchInt {
    /// Local holding the integer discriminant to branch on.
    discriminant: Local,
    /// (discriminant_value, target_block) pairs.
    cases: Vec<(i64, BasicBlock)>,
    /// Default/fallthrough block for unknown discriminant values.
    /// **Tier A: always a Cranelift trap block, never Unreachable.**
    default_bb: BasicBlock,
    /// Source location.
    span: Span,
}
```

Different from `If` terminator (branching on a Trilean condition with +/0/-):
- `SwitchInt` branches on an integer i64 with n targets.
- Lacks the zero_bb/negative_bb semantics of `If`.
- `default_bb` catches any discriminant not matching cases.

**Why not reuse the `If` terminator?** `If` semantics represent an Ł3-aware 3-way branch
on trits (True/Unknown/False). Enum match is an n-way branch on integer
discriminants — differing in data type and target count. Separating them
keeps the IR clear and allows borrowck to know the exact semantics of each terminator.

#### `default_bb` = Cranelift trap (never Unreachable)

**Decision:** `default_bb` is always a basic block terminating in a Cranelift
`trap` instruction, **never** `Terminator::Unreachable`.

**Rationale:** The current type checker only performs exhaustiveness checking for Outcome (E1026,
`check_outcome_exhaustiveness`). There is no exhaustive check for user-defined
enums. If `default_bb = Unreachable` and a match arm is missing → `br_table` falls into
unreachable → **undefined behavior** (Cranelift unreachable = UB).

**Consequences:**
1. **Non-exhaustive match = runtime error**, not a compile-time error.
   This is a semantic gap — weaker than Rust (Rust enforces exhaustive matches at
   compile-time). Honestly documented.
2. **TODO:** Enum-exhaustiveness checker (or-pattern + guard + wildcard) is a
   separate task, outside Phase 4 scope. Once exhaustiveness checking exists, change `default_bb`
   from trap to `Unreachable` (and optimization passes can prune dead trap blocks).
3. **Borrowck:** The trap block is reachable and empty (contains no statements or accesses)
   → harmless to borrowck.
4. **JIT:** Dead traps for matches that are already exhaustive (discriminant always matches cases)
   → still present in codegen but never executed. Acceptable for Tier A.

#### Statement required to read discriminant

A statement is needed to load the discriminant into a local before SwitchInt.
Currently, `OutcomeDiscriminant` performs this for Outcome (reading the trit tag). We propose
a more general statement:

```rust
GetDiscriminant {
    /// Destination local for the discriminant value (i64).
    dest: Local,
    /// The enum local to read from.
    source: Local,
    /// Source location.
    span: Span,
}
```

JIT: `stack_load(I64, enum_slot, offset=0)` → write to `dest`.

**Borrowck:** `source` is counted as a **use** of the enum local (reading discriminant).
If `source` is already Moved → **E2420 UseAfterMove**. Similar to `OutcomeDiscriminant`
in the current `liveness.rs` — `GetDiscriminant` does not move the enum; it only reads the tag.

### 2.3. New Projection: `Payload`

To access the payload of a specific variant after proving the enum is in that variant
(via `SwitchInt`):

```rust
enum Projection {
    // ... existing variants ...
    /// Access the payload of a specific enum variant.
    /// Only valid after the borrowck has proven the enum is in this variant
    /// (via the SwitchInt branch).
    Payload(String),  // variant name
}
```

Used in combination with `Field` to access fields of a struct payload:

```
// Access field "value" of variant "Some"
place = Place::local(_e)
    .project(Payload("Some"))
    .project(Field("value"))
```

**Borrowck with `Payload` projection:** `Payload("Some")` is a refinement type —
borrowck knows (from the SwitchInt branch) that discriminant == 0 (Some). In this
branch, access to `Payload("Some")` is valid. Accessing `Payload("None")`
(unit variant) inside the Some branch → type error (payload does not exist, caught by
typecheck before borrowck).

**`places_conflict` for `Payload` — DEFER to Tier B/C.** In Tier A:
- No by-reference binding in match (§3.4) → no loans on `Payload` projections.
- Destructuring uses copy (§3.2) → no Partial-Moved tracking.
- → `places_conflict(Payload(..))` is never called in Tier A.

Implementing `places_conflict` for `Payload` now would be dead code. When Tier B/C
adds by-reference binding to match, conflict rules for Payload projection will be defined then
(two differently named variants = disjoint at type-level, but alias memory at offset 8 —
conflict rules must be based on proven variant refinement, not raw offsets). **Documented here, not implemented.**

---

## §3 — Ownership Semantics (Author)

### 3.1. Construction: move in

```
let x = 42;
let a = Option::Some(x);  // x is moved into a
```

- `x` is **moved** (consumed) into the payload of `a`.
- After line 2, `x` is no longer valid — compile error upon reuse.
- `a` owns the entire enum value, including both discriminant and payload.

**MIR:** `Assign { dest: _a.Payload("Some"), source: _x }`. Source is a plain local
(no projection) → `is_field_read = false` → borrowck marks `_x` as `Moved`
(`checker.rs:543`). This is existing behavior — enum payload construction acts identically
to struct field construction.

> **Known divergence (pre-existing, not ADR-0037):** `Integer` is a Copy type
> per SPEC §10.1 row 1 ("Stack primitives … Copy by value, no aliasing"),
> but borrowck marks all plain sources as Moved without type-awareness. `Some(x)` with
> `x: Integer` should ideally copy — this gap existed prior to Phase 4 and requires
> borrowck to be aware of type Copy-ness to fix. **Defer to Tier B/C.** Not fixed in Phase 4.

### 3.2. Destructuring: copy out

```
match a {
    Option::Some(y) => {
        // y is a by-value COPY of the payload
        consume(y);
    }
    Option::None => {
        // no payload
    }
}
// a remains valid after match — payload was copied, not moved
```

- `y` binds a **by-value copy** of `a`'s payload.
- After the arm, `a` **remains valid** — the payload has not been moved out.
- `_a` can be reused after the match (as long as it was not moved in its entirety by
  an arm capturing `a` without projection).

**MIR:** `Assign { dest: _y, source: _a.Payload("Some") }`. Source has a projection
→ `is_field_read = true` → borrowck **DOES NOT** mark `_a` as Moved
(`checker.rs:515-517`). This is existing behavior for struct field reads —
enum payload copy behaves identically.

**Why copy instead of move?**
1. SPEC §10.1 row 1: stack primitives (Integer/Trit/Tryte/Long/Trilean/Unit)
   are "Copy by value, no aliasing". In Tier A, all payloads are i64 = stack primitive
   → copy is **spec-mandated**, not merely "cheaper".
2. i64 payload has no destructor, Drop is a no-op → move vs copy is not
   observable at runtime in Tier A.
3. Existing borrowck behavior (`is_field_read` for projected source) is already
   copy → no new code required.

### 3.3. No Partial-Moved in Tier A

Because destructuring uses copy, an enum is never in a Partial-Moved state.
Borrowck does not need to track per-projection-path Moved state for enums.

- After match, `_a` remains Owned.
- No double-move risk (since there is no move-out).
- Drop for `_a`: discriminant + entire payload area remain valid (payload
  was copied, not moved).

Move-out semantics (where destructuring actually transfers payload ownership out of the enum,
rendering the enum Partial-Moved and forbidding reuse of the moved payload) → **defer to Tier B/C** when
heap payloads (String/Vector/HashMap) make moves observable and introduce real destructors.

### 3.4. Construct-move / destructure-copy asymmetry

| | Construct (`Some(x)`) | Destructure (`Some(y) =>`) |
|---|---|---|
| Behavior | **MOVE** source into payload | **COPY** payload out to binding |
| MIR source | Plain local | Projected (`_a.Payload("Some")`) |
| Borrowck | Marks source as Moved (`checker.rs:543`) | `is_field_read` → does not mark (`checker.rs:517`) |
| New code | 0 | 0 |

Asymmetric, but consistent with existing borrowck behavior. Both use the existing
`Statement::Assign` without requiring new borrowck logic. When SPEC §match is clarified
regarding move-vs-copy for pattern bindings (and when Tier B/C introduces heap payloads),
this asymmetry will be resolved.

### 3.5. Comparison with borrow-by-reference

In the future (Tier B/C), pattern matching may support bind-by-reference:

```
match &0 a {
    Option::Some(y) => {  // y: &0 Integer — shared reference into payload
        use(y);
    }
}
```

At this point `a` is neither moved nor copied — `y` is a reference to the payload of `a`.
Borrowck creates loan `{ source: _a.Payload("Some"), dest: _y, form: BorrowReadOnly }`.
While `y` remains live, `a` cannot be moved or mutated (shared borrow).

**Decision:** Tier A only supports by-value copy bindings in match. By-reference
bindings will be added in Tier B/C once borrowck has sufficiently robust per-field loan tracking
and `places_conflict(Payload)` is defined.

---

## §4 — Roadmap

| Step | Description | MIR Changes |
|------|-------------|-------------|
| 4a | `EnumLayout` + `VariantLayout` in `triet-mir` | New data structures |
| 4b | Lowerer: collect enum definitions → `enum_layouts: Vec<EnumLayout>` | `lower_program` |
| 4c | `Statement::EnumAlloc` + `Statement::SetDiscriminant` + `Statement::GetDiscriminant` | 3 statement variants |
| 4d | `Terminator::SwitchInt` + `Terminator::Trap` | 2 terminator variants |
| 4d' | Borrowck + liveness + JIT: treat `Trap` like `Unreachable` (leaf, 0 successors, 0 reads); JIT lowers to Cranelift `trap` | plumbing |
| 4e | `Projection::Payload(String)` | 1 projection variant |
| 4f | Lowerer: `Expr::EnumLiteral` → `EnumAlloc` + `SetDiscriminant` + payload `Assign` | AST→MIR |
| 4g | Lowerer: `match` → `GetDiscriminant` + `SwitchInt` + per-arm payload access | AST→MIR |
| 4h | JIT: `EnumAlloc` → StackSlot, `SetDiscriminant`/`GetDiscriminant` → stack access, `SwitchInt` → Cranelift `br_table` + trap default block | JIT codegen |
| 4i | **MIR verifier:** INV coverage for 5 new constructs (see below) | verifier assertions |

### 4i. MIR verifier invariants

The current verifier (`triet-mir/src/lib.rs:828`) is **structural** — checking block-bounds
(INV-1: all block references exist in `body.basic_blocks`) and local-bounds
(INV-2: all local references are within `local_decls`). No dominator tree,
no reaching-def analysis, not flow-sensitive.

**Invariant classification according to current verifier capabilities:**

| Invariant | Type | Feasible? | Handling |
|-----------|------|-----------|----------|
| `EnumAlloc.dest` has enum type (lookup `local_decls[dest].ty`) | structural | ✅ | Verifier assertion |
| `SetDiscriminant.value` ∈ `[0, n_variants)` | structural | ✅ | Verifier assertion |
| `GetDiscriminant.source` has enum type | structural | ✅ | Verifier assertion |
| `SwitchInt`: all blocks in `cases` + `default_bb` exist | structural (= extends INV-1) | ✅ | Extend INV-1 to traverse `SwitchInt.cases` + `default_bb` |
| `SwitchInt.default_bb` terminates with trap, not `Unreachable` | structural | ✅ | Verifier assertion |
| `SetDiscriminant`/`GetDiscriminant`: dest/source has been `EnumAlloc`ed | reaching-def, flow-sensitive | ❌ | **Lowerer responsibility** — lowerer generates correct MIR; verifier full-dataflow = Tier C defense-in-depth |
| `Payload(..)`: only appears in block dominated by corresponding case-target of `SwitchInt` | dominator tree, flow-sensitive | ❌ | **Lowerer responsibility** — lowerer 4g constructs match → automatically generates Payload in correct block per structure; verifier dominance = Tier C |

**Tier A Rule:** Lowerer is responsible for generating correct MIR for Payload placement
and EnumAlloc-before-use. Verifier only checks structural invariants (enum type,
discriminant range, block existence, trap default_bb). Dominator analysis +
reaching-def represent defense-in-depth for Tier C, outside Phase 4 scope.

**Specific structural invariants (implemented in 4i):**

| # | Invariant | Mechanism |
|---|-----------|-----------|
| 4i-1 | `EnumAlloc.dest`: `local_decls[dest].ty` is an enum type (has entry in `enum_layouts`) | structural: type lookup |
| 4i-2 | `SetDiscriminant.dest`: `local_decls[dest].ty` is an enum type | structural: type lookup |
| 4i-3 | `SetDiscriminant.value` ∈ `[0, enum_layout.variants.len())` | structural: range check |
| 4i-4 | `GetDiscriminant.source`: `local_decls[source].ty` is an enum type | structural: type lookup |
| 4i-5 | `SwitchInt`: every `(_, bb)` in `cases` + `default_bb` exists in `body.basic_blocks` | structural: extends INV-1 |
| 4i-6 | `SwitchInt.default_bb` terminates with `Terminator::Trap` (not `Unreachable`) | structural: terminator check |
| 4i-7 | `Payload(name)`: `name` is a variant present in `enum_layout` of the base local | structural: variant lookup |

---

## Alternatives Considered

- **Trit discriminant for user enums.** `T?`/`T~E` use trit tags because ≤ 3 states.
  User enums have arbitrary n variants → i64 is simple, uniform, and easy to codegen. Trit packing
  (3 trits = 1 tryte, 9 trits = 3 trytes...) is deferred to Tier C if size
  optimization is needed.
- **Nested enum flattening / niche optimization.** `Option<Option<Integer>>`
  wastes 2 discriminants. Rust uses niche optimization (Option<&T> = pointer
  with null sentinel). Triet Tier A **does not do this** — uniform layout is simple,
  serving as an oracle tier. Tier C can add this later.
- **Tag+payload amalgamation for unit variants.** Enums composed entirely of unit variants
  (like `Color { Red, Green, Blue }`) do not need a payload area. Currently still
  allocates 8 bytes payload (max_payload_size = 0 → total = 8). OK for Tier A.
- **`Payload` working with `Index`/`Deref`.** `Payload` is only valid after
  `Downcast` logic (proving variant). Combining `Payload` with `Index` or
  `Deref` has no current use case and is rejected by JIT (similar to current nested
  projections).
- **`places_conflict(Payload)` — defer to Tier B/C.** In Tier A, there are no loans on
  Payload (all copy + no by-reference bindings) → implementing it now is dead code.
- **Enum exhaustiveness checker — defer, separate TODO.** Currently non-exhaustive
  match = runtime trap. Exhaustiveness checking (or-patterns + guards + wildcards)
  is a separate task, outside Phase 4. Once implemented, `default_bb` will be switched
  from trap to `Unreachable`.
- **Partial-Moved tracking for enums — defer to Tier B/C.** Destructuring uses copy
  in Tier A → not needed. When heap payloads have real destructors, move-out
  semantics will provide actual value.
- **Enum variant resolution is context-free (2026-06-05 addendum).**
  > ⚠️ **Superseded by [ADR-0071](0071-path-separator-and-module-import.md) Slice 2
  > (2026-06-26).** The bare-name global-scan + E1018 + `TypeName.Variant`
  > dot-form below are ALL retired. A user variant is now referenced ONLY via
  > the qualified `Enum::Variant` form (or an import-bound symbol via `use`); a
  > bare variant name is a plain `undefined name` (E1002), and E1018
  > `AmbiguousEnumVariant` no longer exists. The text below is the historical
  > 2026-06-05 design.

  Type checker resolves bare variant names (`None`, `SomeInt`) based on a global scan
  of all enum types in the root frame. Type annotations (`let n: CD = None`)
  are **not** used to disambiguate — if two enums both contain variant `None`,
  the compiler emits E1018 requiring the user to qualify (`CD.None`). The compiler does not guess intent
  from type context. Qualified syntax (`TypeName.Variant`) always works for both
  unit variants (FieldAccess) and payload variants (MethodCall/Call+FieldAccess).

## References

- [ADR-0034](0034-jit-aggregate-coverage.md) — Tier A uniform boxing, enum opcode delegate-to-VM (legacy, triet-ir).
- [ADR-0036](0036-typetag-opaque-aggregate.md) — `TypeTag::Opaque` for aggregates (legacy, triet-ir).
- [ADR-0020](0020-outcome-error-handling.md) — Outcome `T~E`/`T?~E` with trit discriminator (built-in sum types).
- [SPEC.md](../../SPEC.md) §match — exhaustive match semantics, enum variant patterns.
- `spec/schema/triet-schema.yaml` — `Expr::EnumLiteral`, `Type::UserEnum`, `EnumVariant`.
- `spec/plans/phase3-cranelift-backend.md` — Bucket C: enums are Phase 4 scope.

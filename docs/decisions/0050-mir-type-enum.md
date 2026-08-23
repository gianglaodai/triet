# ADR-0050: MirType — Structuring the MIR Type System (Eliminating String-Matching)

## 1. Status
**Approved (O + G, 2026-06-09)** — Next Phase-0 Spike. This is the **Foundation for B2**
(unifying the two borrowck tiers): borrowck requires a solid, non-heuristic MIR type system
to compute liveness + exclusivity.

**G Rulings (invariants — must not be violated during implementation):**
1. **`MirType` is handwritten in `triet-mir`** (crate depends only on `triet-core`). MIR is the
   Internal Representation of the backend, **NOT the AST**. Forcing MIR to share the
   schema `Type` (which contains syntactic `TypeId` and unsupported frontend native ints) is a
   leaky abstraction. → MirType belongs to the same tier as `Body`/`Statement`, outside
   the schema-first scope (schema manages AST + S6 ownership, not backend IR).
2. **SEPARATE `Struct(String)` and `Enum(String)` — FORBIDDEN to merge into `UserType(String)`.**
   At lowering time, the compiler knows with 100% certainty whether a name is a struct or enum.
   Discarding that information only to force resolution to search both layout tables is lazy and
   inverts refuse-over-guess. Separate them so the system exposes cross-table lookup bugs
   (looking up `Struct("X")` when "X" is an enum).
3. **Transitional `MirType::parse(&str)` shim IS PERMITTED — but must die cleanly in the
   FINAL commit of B1a.** Tagged with `// TECH-DEBT(B1a): MUST KILL THIS SHIM`.
   The final stage unplugs the bridge to ensure the codebase fails wherever migration was missed.

## 2. Context & Motivation

### 2.1. The pathology: MIR's type system is runtime string-matching with implicit ordering rules
`triet-mir` **lacks a Type enum**. Every type is `pub ty: String` (3 field sites:
`mir/lib.rs:165` `LocalDecl`, `:794` + `:842` field-layout), carrying an **embedded DSL**
parsed at runtime across every consumer:

- primitive: `"Integer"|"Trit"|"Tryte"|"Long"|"Trilean"|"Unit"`;
- `"?"` = "unknown" sentinel (pin: **not** nullable-of-empty);
- heap: `"String"`, `"Vector"`/`"Vector<…>"`, `"HashMap"`/`"HashMap<…>"`;
- **suffix `?`** = nullable, with an **IMPLICIT ordering rule**: all consumers must invoke
  `is_nullable_type` BEFORE `is_vec_type`, otherwise `"Vector<Integer>?"` is mistakenly
  classified as a bare Vector. This is a *textual* invariant (doc-comment), not a
  *structural* one — if a consumer forgets the order, it fails silently while the gate remains green.
- **prefix `&+ /&+ mutable /&0 /&0 mutable /&- `** = 5 ref-forms, parsed via
  `starts_with('&')` (9 sites).

### 2.2. Blast surface (O grepped the entire backend, 2026-06-09 — no guesswork)
| Target | Sites |
|---|---|
| `is_copy` (consumer) | 49 |
| `is_nullable_type` | 14 |
| `nullable_payload` | 14 |
| `is_vec_type` | 14 |
| `is_hashmap_type` | 7 |
| ref-form `starts_with('&')` | 9 |
| jit type-dispatch | 22 (20 literal `__triet_*` are **shim names — RETAINED**) |
| Producer (single bottleneck) | `lower::type_name(arena,id)->String` (lower:613) |

### 2.3. Two secondary landmines (sentenced in B1a)
1. **`simple_is_copy` (lower:652) = SECOND copy** of move/copy logic. Test
   `simple_is_copy_agrees_with_canonical_is_copy` (lower:2906) exists *precisely because*
   the two implementations could drift → latent soundness vulnerability. **Unify into a single source.**
2. **nullable-before-vec ordering rule** (§2.1) — structurally eliminated by
   `Nullable(Box<MirType>)` wrapping instead of suffix strings.

### 2.4. Motivation — this is a FOUNDATION, not a bugfix
B2 (unifying the two borrowck tiers) requires a non-heuristic MIR-type system to compute liveness +
exclusivity. Runtime string-matching with implicit ordering is a ticking time bomb under B2.
Flawed foundations require extensive rework → ADR-before-code, signed off by O+G.

## 3. Architectural Decisions

### 3.1. Shape of `MirType` (handwritten in `triet-mir`)
```rust
pub enum MirType {
    // Scalars — Copy (SPEC §10.1)
    Integer, Trit, Tryte, Long, Trilean, Unit,
    Unknown,                                  // replaces "?" sentinel
    // Heap — Move (ADR-0042)
    String,
    Vector,                                   // BARE — see CORRECTION §3.1.1
    HashMap,                                  // BARE — see CORRECTION §3.1.1
    // Modifiers
    Nullable(Box<MirType>),                   // STRUCTURAL → eliminates ordering rule §2.1
    Reference { form: ReferenceForm, inner: Box<MirType> },  // reuses triet_mir::ReferenceForm
    // User types — SEPARATED per G Ruling #2
    Struct(String),                           // resolved via body.struct_layouts
    Enum(String),                             // resolved via body.enum_layouts
}
```

### 3.1.1. Post-probe CORRECTION (O, 2026-06-09) — BARE Vector/HashMap, NO payload
The initial signed ADR had `Vector(Box<MirType>)` + `HashMap{key,value}`. **O re-measured
prior to S1 and retracted this:** not a single backend consumer extracts element/key/value types
(`rg "split('<'|generic|element_type|key_type" mir/jit/lower/borrowck` → 0 hits);
no diagnostic/fixture prints generic-form `"Vector<Integer>"` (assertions on
`"Vector<Integer>"` only existed in unit tests of helpers slated for deletion; fixtures
`.expected` lack generic forms; lower:1581 diagnostic prints bare `"Vector or HashMap"`).
→ payload is a **dead field**, violating Track-B Rule #4. **Decision: bare variant.**
Consequences: (a) risks R1/R2 of blueprint phase7 (worrying about "having to traverse typechecker Type
to parse generic args") **DISSOLVED** — producer `lower_type` arena-only read is sufficient,
NO touching typecheck `type_map`; (b) B1a scope shrinks. When Tier C needs real generic
Vectors, add payload + consumer in the SAME commit (per Rule #4) — not now.

### 3.1.2. Naming + Trilean DECISIONS (preventing blueprint drift)
- **Enum name = `MirType`, NOT `Type`.** Bare `Type` conflicts with
  `typecheck::Type` and `generated::Type` (both sharing the name) → 3-layer confusion. `MirType` is mandatory.
- **BARE `Trilean`, NOT `Trilean { refined: bool }`.** Refinement (ADR-0021)
  is a FRONTEND gate (determining valid `if cond`), checked BEFORE reaching MIR. O measured:
  0 backend sites match `Trilean { refined }` or read that field (hits for "refined"
  were ordinary English words in doc-comments). → `refined` in MIR = dead field,
  Rule #4. Keep `Trilean` bare.
**Dependency note:** `ReferenceForm` ALREADY exists in `triet-mir` (`lib.rs:407`,
"mirrors triet_syntax"). `Statement::Borrow` already holds typed `form: ReferenceForm`.
→ `MirType::Reference` reuses it immediately, NO new dependencies, NO duplicate enums.

### 3.2. Decomposing 5 string helpers → single-source method/match
- `is_nullable_type` → `matches!(self, MirType::Nullable(_))`.
- `nullable_payload` → `if let MirType::Nullable(inner) = self { Some(inner) }`.
- `is_vec_type` / `is_hashmap_type` → `matches!(self, MirType::Vector)` / `matches!(self, MirType::HashMap)` (bare variants — §3.1.1).
- `is_copy(&self, body)` → match method (recursing into layouts for Struct/Enum preserving
  existing canonical semantics).
- **`simple_is_copy` (lower) DELETED** — calls `MirType::is_copy` directly. Companion test
  `simple_is_copy_agrees_with_canonical` deleted accordingly (no longer two implementations to compare).

### 3.3. Producer
`lower::type_name(arena,id)->String` → `lower::lower_type(arena,id)->MirType`.
This is the SOLE bottleneck producing types — change in one place, all downstream consumers receive MirType.

### 3.4. `Display` for diagnostics, transitional `parse` for strangler migration
- `impl Display for MirType` — round-trips to OLD string format, **exclusively** for
  diagnostic/error messages (preserving stable fixture output).
- `MirType::parse(&str) -> MirType` — transitional shim keeping gate green between
  stages. **`// TECH-DEBT(B1a): MUST KILL THIS SHIM`**. Deleted in final stage.

### 3.5. Outcome — guarded, NOT modeled in B1a
`T~E`/`T?~E` lacks producers (Outcome ops guarded with `Err`). `type_name` currently returns
`"?"` for Outcome → MirType returns `Unknown`. Consistent, avoids scope creep.

## 4. Scope & Deferral (YAGNI)

### 4.1. In B1a (doing now)
Backend tier only: `triet-mir` + `triet-lower` + `triet-jit` + `triet-borrowck`.

### 4.2. Defer B1b (separate slice, subsequent ADR) — DOES NOT block B2
Reconciling typecheck hand-written `Type` ↔ schema-generated `Type` belongs to
frontend/middle-end. Schema `Type` contains **design debt** preventing drop-in replacement, left for B1b:
- native ints `I8..U64/F64/Pointer` (disc 20-29) — unsupported by frontend, importing
  them introduces dead variants violating Track-B Rule #2;
- `UserStruct.fields: Vec<StructField>` with `field_type: TypeId` (syntactic) mixed
  into "resolved" types (semantic) = layering error within the schema itself.
G confirmed: B1a's MirType is more than sufficient to support B2; resolving B1b later properly isolates risk.

## 5. Regression Watch
- **A2 INV-4 verifier** reads `ty` — migrating to MirType must keep INV-4 failing when violated.
- **Tier D Fallback invariant** (fat-pointer String ABI) — JIT dispatches by
  type; string→enum transition must not alter String lowering behavior.
- **C1 fixture-27** (enum-payload-via-param) currently pinned by string matching — migration
  must keep fixture 27 green or document the reason.

## 6. Blueprint Implementation (Strangler, staged)

**Phase-0 Spike (throwaway, Tier D pattern):** prototype `MirType` + `Display` + `parse` +
`From`-producer on scratch branch; convert `is_copy` + 5 helpers; prove
layout lookups + Struct/Enum separation + ordering semantics are preserved; measure clippy/test
delta. **DISCARD.** Do not touch production tree.

**Production stages (each stage = 1 commit, teeth must fail BEFORE proceeding to next stage):**
1. **S1 — Parallel introduction.** Add `MirType` + `Display` + `parse` shim +
   methods (`is_copy`/`is_nullable`/…). `ty: String` REMAINS UNCHANGED. Gate green.
2. **S2 — Flip producer.** `lower::type_name` → `lower_type -> MirType`. Field
   `LocalDecl.ty` / field-layouts change `String→MirType`. Unmigrated consumers
   temporarily use `Display`/`parse` bridge. **Delete `simple_is_copy`** + companion test.
3. **S3 — Migrate consumers in clusters** (mir → lower → borrowck → jit). Replace
   `== "String"`/`is_vec_type(s)`/`starts_with('&')` with `MirType` pattern matching.
4. **S4 — Unplug the bridge.** DELETE `MirType::parse`. Build must fail at every site where
   migration was missed → fix until green. KEEP `Display` (diagnostics). 0 string-dispatches remaining.

**B1a Done Criteria (verified by tests, not assumptions):**
- `rg 'parse\(' triet-mir` → 0 hits for MirType::parse.
- `rg 'is_vec_type|is_hashmap_type|is_nullable_type|nullable_payload' src` → 0
  (converted to methods/matches).
- `simple_is_copy` no longer exists.
- Gate: 0 build errors · 0 test failures · 99 fixtures · justified clippy delta (baseline 208).
- A2 INV-4 + fixture-27 + String-lowering fail on poison.

## 7. Consequences
- **Positive:** eliminates implicit ordering rules (structured), single source of truth for move/copy, separated
  Struct/Enum catches cross-table lookup bugs, solid foundation for B2. Type safety at inception.
- **Negative:** blast radius of ~100+ sites (confined to ONE tier, staged). `Display` must
  accurately round-trip the old string format to preserve fixture diagnostics.
- **Carried-over Debt (untouched in B1a):** B1b reconcile typecheck ↔ schema Type ·
  concat → sret · B3 alias-analysis (replacing conservative=true).

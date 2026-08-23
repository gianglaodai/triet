# ADR 0023 — Lowerer SSA struct-tracking: unified `ValueKind`

**Status:** Decided. Applicable from v0.7.x.review.lowerer onward. Closes `parser_differential` finding in v0.7 review (review session 2026-05-22): "Lowerer struct-tracking turned into patch-upon-patch — each sub-task in v0.7.5.* added 1 new propagation rule; no endgame; silent field_idx fallback error violates VISION §6 *Refuse over guess*".

**Origin:** Author 2026-05-22 (after overall v0.7 review): chose Option A — "write a unifying ADR" instead of continuing ad-hoc per-case patching. Four struct-tracking fixes in v0.7.5.4a + four fixes in v0.7.5.6b were merely symptoms; the root cause was an ad-hoc tracking design.

## §1 — Problem: Patch-Stack Debt

Prior to ADR-0023, `crates/triet-ir/src/lowerer.rs` carried **four separate value-level HashMaps** + **two function-level HashMaps** + **three lookup-table HashMaps**:

```rust
// Value-level — mutated each time a new SSA value is created
value_struct_types:         HashMap<ValueId, String>,  // V is struct X
value_outcome_value_struct: HashMap<ValueId, String>, // V is Outcome<X, _>
// (Nullable tracking inline-merged into value_struct_types per v0.7.5.6b)

// Function-level — populated in declare_function
func_return_struct:               HashMap<FuncId, String>,
func_return_outcome_value_struct: HashMap<FuncId, String>,

// Lookup table — read-only after Pass 1a
struct_fields:           HashMap<String, Vec<String>>,
struct_field_types:      HashMap<(String, String), String>,
variant_payload_struct:  HashMap<String, String>,
```

Every time the lowerer added a **new construct** that produces an SSA value (function call, struct literal, pattern unwrap, phi merge, `~+` constructor, `!!`, `~?`, `~:`), a **separate propagation rule** had to be written for each map. v0.7 history:

| Sub-task | Rules Added | File:section |
|---|---|---|
| v0.7.4.3-debt.2 (WA-2) | OutcomeArm propagation + `func_return_outcome_value_struct` + `value_outcome_value_struct` | `bind_pattern_vars`, `declare_function`, call site |
| v0.7.5.1 | `variant_payload_struct` for enum variant payload | `bind_pattern_vars` Pattern::EnumVariant |
| v0.7.5.2 | `struct_field_types` for chained field access | `Expr::FieldAccess` |
| v0.7.5.4a (fix #1-5) | While-loop phi + match-arm mutated-var phi + match merge_dest + if merge_dest + `~+ StructLit` literal-side | Each phi-merge site |
| v0.7.5.4a (fix #6) | `let p: T = …` annotation seeding | `Stmt::Let` |
| v0.7.5.6b (fix #1-4) | Nullable return tracking + Nullable let annotation + `T?` pattern unwrap + `!!` propagation | 4 distinct sites |

**Total:** ~13 propagation rules across ~12 distinct call sites. Each rule is 5-15 lines of code. In total, ~150 LOC of propagation logic was scattered across the lowerer.

**Symptoms:**
1. **Silent bugs.** When a rule for a new construct was missing, `resolve_struct_field_idx` silently fell back to 0. The VM read the wrong slot. The output was incorrect, but it did NOT crash. Caught only via differential tests materializing span data (v0.7.5.6b); earlier pretty-printer-only smoke tests missed it. **This violates VISION §6 "Refuse over guess"** — the compiler was silently coercing rather than erroring.
2. **Linearly increasing coupling.** Each new AST node $\rightarrow$ each new propagation rule. v0.7.7 `typecheck.tri` + v0.7.8 `ir_lowerer.tri` will add many more constructs. The "patch-on-discovery" pattern does not scale.
3. **Four separate maps** for the same concept (value's type identity for field access). Maintaining 4 parallel maps imposes unnecessary cognitive overhead.

## §2 — Decision: Unified `ValueKind` Enum + Single Map

**Lock:** Replace the four value-level HashMaps with **a single `value_kinds: HashMap<ValueId, ValueKind>`** using the enum:

```rust
/// Per-SSA-value kind that the lowerer needs to resolve field_idx
/// and propagate identity through unwraps / phis / pattern bindings.
///
/// Distinct from `TypeTag` (which is wire-format-bound) — `ValueKind`
/// lives purely in the lowerer crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// User-defined struct. `field_idx` resolved via
    /// `struct_fields[name]`. Field access on this value emits
    /// FieldGet with the correct slot.
    Struct { name: String },

    /// `T~E` / `T?~E` outcome wrapping `inner`. Pattern-unwrap via
    /// `~+ pat =>` produces a value bound to `inner_kind`.
    Outcome { inner_kind: Box<ValueKind> },

    /// `T?` nullable. Triet bare-stores T at runtime (no boxing
    /// wrapper), so field accesses on a non-null `T?` resolve as
    /// if the value were `T` directly. Pattern-unwrap via `~+ pat
    /// =>` or `!!` produces a value bound to `inner_kind`.
    Nullable { inner_kind: Box<ValueKind> },

    /// Any value whose layout doesn't need field-index tracking:
    /// primitives (Trit / Tryte / Integer / Long / Trilean /
    /// String / Unit), collections (Vector / HashMap), generic
    /// param slots (type-erased per ADR-0019 §A7.1), etc.
    Other,
}
```

**Single source of truth:** `value_kinds: HashMap<ValueId, ValueKind>` replaces:
- `value_struct_types` $\rightarrow$ represented by `ValueKind::Struct`
- `value_outcome_value_struct` $\rightarrow$ represented by `ValueKind::Outcome { inner: Struct }`
- Nullable tracking inline (v0.7.5.6b) $\rightarrow$ represented by `ValueKind::Nullable { inner: Struct }`

Function-level maps unify analogously:
- `func_return_kind: HashMap<FuncId, ValueKind>` replaces both
  `func_return_struct` and `func_return_outcome_value_struct`.

Lookup tables (`struct_fields`, `struct_field_types`, `variant_payload_struct`, `variant_index`, `enum_variants`) **stay** — they encode static program structure, not per-value tracking, and are not part of this debt.

## §3 — Propagation: Single Recursive Helper

Replaces ~13 ad-hoc rules with **one recursive helper**:

```rust
/// Resolve a `TypeExpr` to the lowerer's `ValueKind`. Recurses
/// through Nullable / Outcome to find the inner user-struct (if
/// any). Returns `ValueKind::Other` for primitives / collections.
fn type_expr_to_value_kind(&self, type_id: TypeId, module: &Module) -> ValueKind {
    let arena = &self.program.arenas[module.arena_id.0];
    match &arena.type_expression(type_id).node {
        TypeExpr::Named(name) if self.struct_fields.contains_key(name) => {
            ValueKind::Struct { name: name.clone() }
        }
        TypeExpr::Nullable(inner) => {
            let inner_kind = self.type_expr_to_value_kind(*inner, module);
            ValueKind::Nullable { inner_kind: Box::new(inner_kind) }
        }
        TypeExpr::Outcome { value_type, .. } => {
            let inner_kind = self.type_expr_to_value_kind(*value_type, module);
            ValueKind::Outcome { inner_kind: Box::new(inner_kind) }
        }
        _ => ValueKind::Other,
    }
}
```

**Every value creation calls `set_value_kind(value_id, kind)` exactly once.** No more ad-hoc per-construct propagation.

Each construct's rule reduces to ONE LINE:

| Construct | Kind Resolution |
|---|---|
| Function parameter | `type_expr_to_value_kind(param.type_annotation, module)` |
| Function call dest | `func_return_kind.get(&func_id).cloned()` |
| Stmt::Let dest | If value's kind is `Other` and annotation resolves, fall back to annotation kind |
| StructNew | `ValueKind::Struct { name }` |
| OutcomeConstructor positive | `ValueKind::Outcome { inner: payload's kind }` |
| OutcomeConstructor zero / negative | `ValueKind::Outcome { inner: Other }` (no payload tracking) |
| OutcomeUnwrapValue (`~?` / `~:` / match-arm bind) | Strip one `Outcome` layer from scrutinee's kind |
| NullUnwrap (`!!`) | Strip one `Nullable` layer from operand's kind |
| FieldGet | Look up via inner kind chain (Nullable/Outcome are transparent) |
| Phi merge (match / while / if) | If every incoming has same kind, dest gets that kind; else `Other` |
| Pattern-bind Variable | Inherits scrutinee's kind |
| Pattern-bind EnumVariant payload | `variant_payload_struct[variant_name]` (lookup-table unchanged) |
| Pattern-bind OutcomeArm positive | Strip Outcome layer from scrutinee's kind |

**13 rules $\rightarrow$ 13 ONE-LINERS, all calling helpers in one section of lowerer.rs.**

## §4 — "Refuse Over Guess" Semantics

`resolve_struct_field_idx(value_id, field_name)` becomes:

```rust
fn resolve_struct_field_idx(&self, value_id: ValueId, field_name: &str) -> u32 {
    let mut kind = self.value_kinds.get(&value_id);
    // Transparent traversal through nullable / outcome layers.
    while let Some(k) = kind {
        match k {
            ValueKind::Struct { name } => {
                return self.struct_fields
                    .get(name)
                    .and_then(|fields| fields.iter().position(|n| n == field_name))
                    .and_then(|i| u32::try_from(i).ok())
                    .unwrap_or(0); // last-resort fallback — only fires on UNKNOWN field
            }
            ValueKind::Nullable { inner_kind } | ValueKind::Outcome { inner_kind } => {
                // Walk through transparent wrappers.
                // For `value: T?`, `value.field` works because T? bare-stores T.
                // For `value: T~E`, this is technically wrong — user should `~?` first —
                // but the v0.7 typecheck doesn't enforce that yet (post-v0.7 work).
                kind = Some(inner_kind);
            }
            ValueKind::Other => return 0,
        }
    }
    0
}
```

**Stronger contract than the v0.7.5.6b implementation:** if a value WAS tracked (any kind) but its struct field is not found in the resolved struct, return 0 only as the LAST resort (after exhausting the wrapper chain). If tracking is entirely absent $\rightarrow$ return 0 as before (preserves call-site behavior for `Other` values like raw integers).

**Future tightening:** v0.8+ could promote the fallback to a `panic!` or `unreachable!` once typecheck pipes per-expression types to the lowerer (Option B in §6). Today we keep the fallback for backward compatibility with type-erased generic functions per ADR-0019 §A7.1.

## §5 — Consequences

### Benefits

- **One source of truth** for per-value type identity. New AST construct $\rightarrow$ one `set_value_kind` call $\rightarrow$ done. No more "did I forget to update the 4th map?".
- **Recursive structure** (`Outcome<Nullable<Struct>>` etc.) naturally expressed. Pre-ADR-0023 ad-hoc rules could not compose — `Nullable<Outcome<...>>` would have needed its own propagation chain.
- **Refactor surface contained.** Only `crates/triet-ir/src/lowerer.rs` touched. No impact on:
  - Wire format `.triv` (ValueKind never serialized)
  - VM dispatch (RuntimeValue::Struct does not carry name)
  - TypeTag (separate enum, separate purpose)
  - Typecheck (uses its own Type enum)
- **Symbol clarity.** `value_kinds` is the only map the lowerer needs to consult for field access. The reader does not have to scan 4 maps to determine if a value has tracking.

### Trade-offs

- **Boxed recursive enum** = small heap allocations. Negligible compared to the dominant arena/Vec costs in the lowerer.
- **`ValueKind::Other` swallows generic type params.** Same behavior as today (TypeTag::Unit for `T` in generic functions per ADR-0019 §A7.1). When v2.0 LLVM AOT demands true monomorphization, both ADR-0023 and §A7.1 will evolve together.
- **`FieldGet` on `ValueKind::Outcome { ... }` is technically a type error** (user should use `~?` first) — but the helper traverses the wrapper anyway for now. Tightening this requires v0.7.7 `typecheck.tri` integration.

## §6 — Rejected Alternatives

- **Extend `TypeTag` with `UserStruct(String)`.** This approach would touch the wire format + VM + every IR consumer. Considered but rejected: lowerer tracking is a LOWERING-PHASE concern, not a runtime type-system concern. Modifying the wire format introduces excessive risk relative to benefit.
- **Pipe typecheck's per-expression type map to lowerer.** Option B was attractive but requires invasive changes to typecheck output and lowerer input plumbing. Deferred to v0.7.7+ when `typecheck.tri` is ported. ADR-0023 establishes the receiver shape (`ValueKind` enum) that future typecheck output can populate directly.
- **Panic-on-untracked instead of 0-fallback.** Considered but rejected for v0.7.x compatibility: generic functions per ADR-0019 §A7.1 produce type-erased values that legitimately have `ValueKind::Other`. A blanket panic would break that path. Future tightening will add `panic` to the `Other` arm once typecheck pipes types through.
- **Per-`Stmt::Assign` re-tracking** for mutable rebinds. Current `rebind_var` does not touch `value_kinds`. ValueKind is per-SSA-value; rebinding a name to a NEW SSA value means the new value already has its own `value_kinds` entry from wherever it was created. Tracking the NAME $\rightarrow$ VALUE map (via `scopes`) remains orthogonal.

## §7 — Migration Path

1. **Phase 1 (this commit):** Add `ValueKind` enum + `value_kinds` HashMap + `func_return_kind` HashMap + helpers (`type_expr_to_value_kind`, `set_value_kind`, `kind_of_value`, `unwrap_one_layer`). **No call-site changes yet.** Maps coexist additively.
2. **Phase 2 (same commit if scope allows):** Migrate every propagation site to call the new helpers. Old maps stop being written.
3. **Phase 3 (cleanup):** Remove old maps once every read switches to `value_kinds`. Verify via `cargo test --workspace` + `cargo clippy`.

Within this commit (v0.7.x.review.lowerer): ship Phases 1-3 together so the lowerer does not carry parallel tracking implementations even briefly.

## §8 — Status & Scope

- **Status:** Locked.
- **Scope:** lowerer crate only. No wire format / VM / typecheck / SPEC changes.
- **Compatibility:** All v0.7.* differential gates (lexer, parser) must stay byte-identical post-refactor. Verified via `cargo test --workspace` = 1315 $\rightarrow$ 1315.

## References

- [VISION §6](../../VISION.md) — "Refuse over guess" violation that motivates this ADR
- [ADR-0007](0007-ir-design.md) — IR design (TypeTag) — unchanged
- [ADR-0019 §A7.1](0019-self-hosting-compiler-bootstrap.md) — generic function type erasure — preserved as `ValueKind::Other`
- [ADR-0020](0020-outcome-error-handling.md) — Outcome — `ValueKind::Outcome` mirrors its structure
- [v0.7.5.4a commit `bcf9b19`](https://example/parser) — 6 lowerer fixes that this ADR consolidates
- [v0.7.5.6b commit `db158ab`](https://example/parser-diff) — 4 more lowerer fixes that surfaced the patch-stack problem

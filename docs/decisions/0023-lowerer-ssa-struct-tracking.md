# ADR 0023 — Lowerer SSA struct-tracking: unified `ValueKind`

**Trạng thái:** Quyết định. Áp dụng cho v0.7.x.review.lowerer onward. Closes `parser_differential` finding in v0.7 review (review session 2026-05-22): "Lowerer struct-tracking biến thành patch chồng patch — mỗi sub-task v0.7.5.* thêm 1 propagation rule mới; không có endgame; lỗi field_idx fallback im lặng vi phạm VISION §6 *Refuse over guess*".

**Origin:** Author 2026-05-22 (sau khi review tổng v0.7): chọn phương án A — "viết ADR thống nhất" thay vì tiếp tục vá per-case. Bốn fix struct-tracking ở v0.7.5.4a + bốn fix ở v0.7.5.6b chỉ là symptom; root cause là thiết kế tracking ad-hoc.

## §1 — Vấn đề: patch-stack

Hiện tại (trước ADR-0023) `crates/triet-ir/src/lowerer.rs` carry **bốn HashMap value-level riêng biệt** + **hai HashMap function-level** + **ba HashMap lookup-table**:

```rust
// Value-level — mutated mỗi lần tạo SSA value mới
value_struct_types:       HashMap<ValueId, String>,  // V là struct X
value_outcome_value_struct: HashMap<ValueId, String>, // V là Outcome<X, _>
// (Nullable tracking inline-merged vào value_struct_types per v0.7.5.6b)

// Function-level — populated ở declare_function
func_return_struct:               HashMap<FuncId, String>,
func_return_outcome_value_struct: HashMap<FuncId, String>,

// Lookup table — read-only sau Pass 1a
struct_fields:           HashMap<String, Vec<String>>,
struct_field_types:      HashMap<(String, String), String>,
variant_payload_struct:  HashMap<String, String>,
```

Mỗi khi lowerer thêm **construct mới** tạo ra SSA value (function call, struct literal, pattern unwrap, phi merge, ~+ constructor, !!, ~?, ~:), phải viết **propagation rule riêng** cho từng map. Lịch sử v0.7:

| Sub-task | Rules thêm | File:section |
|---|---|---|
| v0.7.4.3-debt.2 (WA-2) | OutcomeArm propagation + `func_return_outcome_value_struct` + `value_outcome_value_struct` | `bind_pattern_vars`, `declare_function`, call site |
| v0.7.5.1 | `variant_payload_struct` cho enum variant payload | `bind_pattern_vars` Pattern::EnumVariant |
| v0.7.5.2 | `struct_field_types` cho chained field access | `Expr::FieldAccess` |
| v0.7.5.4a (fix #1-5) | While-loop phi + match-arm mutated-var phi + match merge_dest + if merge_dest + `~+ StructLit` literal-side | Each phi-merge site |
| v0.7.5.4a (fix #6) | `let p: T = …` annotation seeding | `Stmt::Let` |
| v0.7.5.6b (fix #1-4) | Nullable return tracking + Nullable let annotation + `T?` pattern unwrap + `!!` propagation | 4 distinct sites |

**Tổng:** ~13 propagation rules across ~12 distinct call sites. Mỗi rule là 5-15 dòng code. Tổng ~150 LOC propagation logic spread across the lowerer.

**Triệu chứng:**
1. **Bugs im lặng.** Khi rule cho construct mới chưa tồn tại, `resolve_struct_field_idx` fallback về 0. VM đọc sai slot. Output sai nhưng KHÔNG crash. Bắt được chỉ bằng differential test materializing span data (v0.7.5.6b). Trước đó pretty-printer-only smoke không bắt được. **Đây vi phạm VISION §6 "Refuse over guess"** — compiler đang silently coerce thay vì error.
2. **Coupling tăng tuyến tính.** Mỗi AST node mới → mỗi propagation rule mới. v0.7.7 typecheck.tri + v0.7.8 ir_lowerer.tri sẽ thêm nhiều construct nữa. Pattern "patch-on-discovery" không scale.
3. **Bốn map riêng** cho cùng concept (value's type identity for field access). Bảo trì 4 map paralleled nhau là cognitive overhead.

## §2 — Quyết định: unified `ValueKind` enum + single map

**Lock:** Thay bốn value-level HashMap bằng **một `value_kinds: HashMap<ValueId, ValueKind>`** với enum:

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

    /// `T?` nullable. Triết bare-stores T at runtime (no boxing
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
- `value_struct_types` → represented by `ValueKind::Struct`
- `value_outcome_value_struct` → represented by `ValueKind::Outcome { inner: Struct }`
- Nullable tracking inline (v0.7.5.6b) → represented by `ValueKind::Nullable { inner: Struct }`

Function-level maps unify analogously:
- `func_return_kind: HashMap<FuncId, ValueKind>` replaces both
  `func_return_struct` and `func_return_outcome_value_struct`.

Lookup tables (`struct_fields`, `struct_field_types`, `variant_payload_struct`, `variant_index`, `enum_variants`) **stay** — they encode static program structure, not per-value tracking, and aren't part of this debt.

## §3 — Propagation: single recursive helper

Replaces ~13 ad-hoc rules với **one recursive helper**:

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

| Construct | Kind resolution |
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

**13 rules → 13 ONE-LINERS, all calling helpers in one section of lowerer.rs.**

## §4 — `Refuse over guess` semantic

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

**Stronger contract than the v0.7.5.6b implementation:** if a value WAS tracked (any kind) but its struct field isn't found in the resolved struct, return 0 only as the LAST resort (after exhausting the wrapper chain). If tracking is entirely absent → return 0 as before (preserves call-site behavior for `Other` values like raw integers).

**Future tightening:** v0.8+ could promote the fallback to a `panic!` or `unreachable!` once typecheck pipes per-expression types to the lowerer (Option B in §6). Today we keep the fallback for back-compat with type-erased generic functions per ADR-0019 §A7.1.

## §5 — Hệ quả

### Đạt được

- **One source of truth** cho per-value type identity. New AST construct → one `set_value_kind` call → done. No more "did I forget to update the 4th map?".
- **Recursive structure** (`Outcome<Nullable<Struct>>` etc.) naturally expressed. Pre-ADR-0023 ad-hoc rules couldn't compose — `Nullable<Outcome<...>>` would have needed its own propagation chain.
- **Refactor surface contained.** Only `crates/triet-ir/src/lowerer.rs` touched. No impact on:
  - Wire format `.triv` (ValueKind never serialized)
  - VM dispatch (RuntimeValue::Struct doesn't carry name)
  - TypeTag (separate enum, separate purpose)
  - Typecheck (uses its own Type enum)
- **Symbol clarity.** `value_kinds` is the only map the lowerer needs to consult for field access. Reader doesn't have to scan 4 maps to know if a value has tracking.

### Hạn chế

- **Boxed recursive enum** = small heap allocations. Negligible vs the dominant arena/Vec costs in the lowerer.
- **`ValueKind::Other` swallows generic type params.** Same behavior as today (TypeTag::Unit for `T` in generic functions per ADR-0019 §A7.1). When v2.0 LLVM AOT demands true monomorphization, both ADR-0023 and §A7.1 evolve together.
- **`FieldGet` on `ValueKind::Outcome { ... }` is technically a type error** (user should `~?` first) — but the helper traverses the wrapper anyway for now. Tightening this requires v0.7.7 typecheck.tri integration.

## §6 — Không làm

- **Extend `TypeTag` với `UserStruct(String)`.** Cách này sẽ touch wire format + VM + every IR consumer. Considered but rejected: the lowerer's tracking is a LOWERING-PHASE concern, not a runtime type-system concern. Cấp wire-format thấy quá nhiều rủi ro vs benefit.
- **Pipe typecheck's per-expression type map to lowerer.** Option B was attractive but requires invasive change to typecheck output + lowerer input plumbing. Deferred to v0.7.7+ when typecheck.tri ports. ADR-0023 establishes the receiver shape (`ValueKind` enum) that future typecheck output can populate directly.
- **Panic-on-untracked instead of 0-fallback.** Considered but rejected for v0.7.x compatibility: generic functions per ADR-0019 §A7.1 produce type-erased values that legitimately have `ValueKind::Other`. A blanket panic would break that path. Future-tighten by adding `panic` to `Other` arm once typecheck pipes types through.
- **Per-`Stmt::Assign` re-tracking** for mutable rebinds. Current `rebind_var` doesn't touch `value_kinds`. ValueKind is per-SSA-value; rebinding a name to a NEW SSA value means the new value already has its own `value_kinds` entry from wherever it was created. Tracking the NAME → VALUE map (via `scopes`) stays orthogonal.

## §7 — Migration path

1. **Phase 1 (this commit):** Add `ValueKind` enum + `value_kinds` HashMap + `func_return_kind` HashMap + helpers (`type_expr_to_value_kind`, `set_value_kind`, `kind_of_value`, `unwrap_one_layer`). **No call-site changes yet.** Maps coexist additively.
2. **Phase 2 (same commit if scope allows):** Migrate every propagation site to call the new helpers. Old maps stop being written.
3. **Phase 3 (cleanup):** Remove old maps once every read switches to `value_kinds`. Verify via cargo test --workspace + cargo clippy.

Within this commit (v0.7.x.review.lowerer): ship Phases 1-3 together so the lowerer doesn't carry parallel tracking implementations even briefly.

## §8 — Trạng thái

- **Trạng thái:** Locked.
- **Scope:** lowerer crate only. No wire format / VM / typecheck / SPEC changes.
- **Compatibility:** All v0.7.* differential gates (lexer, parser) must stay byte-identical post-refactor. Verified via cargo test --workspace = 1315 → 1315.

## Liên kết

- [VISION §6](../../VISION.md) — "Refuse over guess" violation that motivates this ADR
- [ADR-0007](0007-ir-design.md) — IR design (TypeTag) — unchanged
- [ADR-0019 §A7.1](0019-self-hosting-compiler-bootstrap.md) — generic function type erasure — preserved as `ValueKind::Other`
- [ADR-0020](0020-outcome-error-handling.md) — Outcome — `ValueKind::Outcome` mirrors its structure
- [v0.7.5.4a commit `bcf9b19`](https://example/parser) — 6 lowerer fixes that this ADR consolidates
- [v0.7.5.6b commit `db158ab`](https://example/parser-diff) — 4 more lowerer fixes that surfaced the patch-stack problem

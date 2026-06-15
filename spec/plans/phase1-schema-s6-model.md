# Phase 1 — Schema / S6 Ownership Model

**Status:** Partial — AST + ownership schema-driven; type system hand-written (2026-06-04)
**See also:** `TODO.md` (live backlog + debt registry). REPORT-2026-06-04.md đã xóa — git history giữ.

**Dependency note:** Phase numbering ≠ build order. Phase 4 (AST→MIR lowering)
is the prerequisite for Phase 2 (borrowck) and Phase 3 (JIT). The lowerer was
built first; the phase numbers reflect design-document order, not dependency order.

```
Actual dependency: 1(schema) → 4(lowerer) → 2(borrowck) → 3(JIT) → 5(S6) → 6(capability)
Phase doc order:   1 ← 2 ← 3 ← 4 ← 5 ← 6
```

---

## 1. What's done

### Schema codegen pipeline ✅

`spec/schema/triet-schema.yaml` → `codegen.py` → `crates/triet-syntax/src/generated/`

| Generated type | Consumers | Status |
|---|---|---|
| `ReferenceForm` | borrowck + parser + syntax | ✅ Live |
| `Visibility` | syntax (re-export) | ✅ Live |
| `Expr`, `Stmt`, `Item`, `Program` | parser, typecheck, lowerer | ✅ Live |
| `BinaryOperator`, `UnaryOperator` | parser, typecheck | ✅ Live |
| `PrimitiveType` | — | ⚠️ Dead (typecheck has own primitive classification) |
| `Type` | — | ❌ Dead — 0 consumers |
| `StructField`, `EnumVariant` | — | ⚠️ Dead (referenced by generated `Type` only) |
| `TypeParameter`, `ParameterPassing` | — | ⚠️ Dead |

### S6 ownership model in AST ✅

5 reference forms (`&+`, `&+ mutable`, `&0`, `&0 mutable`, `&-`) are:
- Lexed (longest-match disambiguates `&` from `&&`)
- Parsed into `ReferenceForm` enum
- Typechecked
- Lowered into MIR with correct `ReferenceForm` annotations
- Enforced by NLL borrowck (E2420, E2440)

---

## 2. What's NOT done — the type system gap

### Generated `Type` is dead code

The schema defines a canonical `Type` enum with 27 variants (Trit through
Unknown). The codegen emits it as `generated::types::Type` with a full
`TypeVisitor` trait. **Nobody imports this enum.** The typechecker uses a
hand-written `Type` in `triet-typecheck/src/types.rs` that diverges from
the schema:

| Aspect | Schema `Type` | Typecheck `Type` |
|---|---|---|
| Binary ints | I8, U8, I16, U16, I32, U32, I64, U64, F64, Pointer | Not resolved (typecheck doesn't handle binary-native types) |
| Outcome | `BinaryOutcome { value_type, error_type }` + `TernaryOutcome` | Combined outcome representation |
| Never | Not present | Present (bottom type for diverging expressions) |
| Helper methods | `accept()` visitor only | `matches()`, `is_send()`, `substitute()`, display, etc. |

### Track B rule #2 violation

> "Every `pub enum` / `pub struct` emitted by `codegen.py` must have at least
> one consumer in the workspace."

`Type`, `PrimitiveType`, `StructField`, `EnumVariant`, `TypeParameter`, and
`ParameterPassing` are all emitted but not consumed. `Type` is the worst
offender — it's the heart of the type system that schema was supposed to drive.

### Why this matters

The rewrite's defining principle is "schema-first." Phase 1 is the foundation
— if the type system isn't schema-driven, phases 2-6 are building on a
foundation that contradicts the stated architecture.

---

## 3. Decision (2026-06-04)

**Conscious deferral.** The author chooses to keep the schema `Type` as target
specification while the typechecker continues using its hand-written `Type`.
The `Type` enum in the schema is tagged `status: spec_only` — codegen emits it
with a warning annotation. The SSOT claim in CLAUDE.md and README.md has been
downgraded: schema is SSOT for AST + ownership, NOT for the type system.

**Rationale:** Migrating the typechecker to use the generated `Type` is a
full phase — it requires reconciling variant sets, implementing missing traits
(`matches()`, `is_send()`, `substitute()`) via codegen, and rewiring every
typecheck consumer. This is real compiler work, not a documentation fix.
It is deferred to a named future phase.

**Risk acknowledged:** The "schema-driven" pillar of VISION.md is consciously
downgraded for the type system. A future reader of CLAUDE.md may assume the
schema drives the type system. The annotation on the generated `Type` and the
updated SSOT claims in all 3 locations mitigate this.

---

## 4. Plan for Type migration (future phase, NOT scheduled)

When the migration phase opens:

1. Reconcile variant sets between schema `Type` and typecheck `Type`
2. Add missing helper methods to codegen (`matches()`, `is_send()`, `substitute()`, `Display`)
3. Rewire typecheck to use `generated::types::Type`
4. Remove hand-written `typecheck::types::Type`
5. Update schema `Type` status from `spec_only` to active
6. Restore SSOT claim for type system

Estimated effort: significant — touches schema, codegen, typecheck, lowerer, parser, and all type-consuming code.

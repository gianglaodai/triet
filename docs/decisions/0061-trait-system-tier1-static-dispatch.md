# ADR-0061 — Trait System (Tier 1): Static Dispatch via Method-Syntax + Mangled Names

- **Status:** 🔒 LOCKED — Signed by O+G.
- **Date:** 2026-06-15
- **Drafted by:** Mentor O (recon-before-typing: 3 fronts across typecheck/MIR/JIT, separating 3 dispatch tiers, disarming mine of ADR-0039 not belonging to trait).
- **Signatures:** O ✅ · G ✅ (G approved blueprint + 5 decisions on 2026-06-15).
- **Post-lock Amendment (2026-06-15, author directive):** Applied `feedback_no_abbreviations` across schema — keyword `implement` (NOT `impl`, aligned with ADR-0005 verbose-keyword); full names `MethodSignature`/`TraitDefinition`/`ImplementationDefinition`, fields `parameters`/`type_parameters`. `trait` retained (already a full word). Internal Rust names `Expr`/`Stmt`/`ExprId` RETAINED (not Triet identifiers).
- **Related:** [phase13-trait-system.md](../../spec/plans/phase13-trait-system.md) (implementation plan) · [ADR-0038](0038-comparable-trait-deferred.md) (`Comparable`/`compare()->Trit` — first consumer) · [ADR-0039](0039-nullable-operator-family.md) (`?+>` family — **SPLIT OUT** of this scope) · [ADR-0037](0037-enum-tagged-union-layout.md) (i64 discriminant) · [ADR-0050](0050-mir-type-enum.md) (MirType).

---

## 1. Context

Author confirmed on 2026-06-05 (ADR-0038): **Triet definitely adopts Traits, not Interfaces.** `Comparable` (`compare() -> Trit`) is the first consumer, design-locked waiting for the Trait system to land. This ADR establishes the Trait vehicle at the minimal viable level.

**Infrastructure Survey (Mentor O, 2026-06-15, reading real code):**

| Layer | Ground Truth | file:line |
|---|---|---|
| Lexer/Parser/Schema | **No** `trait`/`implement` keywords, no AST nodes. Clean slate. | grep parser/lexer; `triet-schema.yaml` only has `TypeParameter.bound` |
| `Type` enum | 17 variants, **no** Trait. UserStruct/UserEnum stored inline. | `triet-typecheck/src/types.rs:13-117` |
| Method dispatch | **Hardcoded table** `builtin_method_type()`; no user-defined `implement Type{}`. | `triet-typecheck/src/check/methods.rs:12-79` |
| Generic bounds | Only `GenericBound::Send`; no trait bounds. | `triet-syntax/src/item.rs:9-23`; `check.rs:991-1003` |
| Resolution annotation | `EnumVariantResolution` stamped onto `ExprId/PatternId`, consumed by lowerer. **Template** for method resolution. | `triet-syntax/src/lib.rs:36-51` |
| MIR call | `CallDispatch{callee_name:String, target:CallTarget{Jit\|Shim}}`. No monomorphization, no function pointers, no mangling. | `triet-mir/src/lib.rs:777-800` |
| StructLayout/EnumLayout | Memory layout only. No methods or vtables. | `triet-mir/src/lib.rs:1042`, `1082` |
| JIT | **Only** direct `ins().call(func_ref)`. `call_indirect` = 0 occurrences. | `mir_lower.rs:1583/1596/1624`; grep verified `NONE` |
| Lambda | **Cannot lower yet** ("NO Lambda handling in lower"). | `triet-lower/src/lib.rs` (Lambda → `unsupported_expr`) |

Current foundation: calls are resolved via **static string names → direct calls**. Static dispatch is virtually zero-cost; dynamic dispatch must be built from scratch.

## 2. Decision — Tier 1 Static Dispatch

Separate into 3 Tiers based on actual implementation cost. **Implement Tier 1, freeze Tier 2 + Tier 3.**

| Tier | Requirements | New Infrastructure Cost | Consumer |
|---|---|---|---|
| **Tier 1 — Concrete dispatch** | `implement T for ConcreteType`; invoke method on concrete type → resolve → impl function name → `CallDispatch::Jit`. | **~0** (direct calls already exist). | **ADR-0038 requires exactly this** |
| **Tier 2 — Generic monomorphization** | `function f<T: Trait>(...)` → generate specialized instances + mangling + monomorphization collection pass. | New pass (MIR currently lacks generic-arg/mangling: `FunctionId(0)`). | When stdlib generics arrive |
| **Tier 3 — Trait object / dynamic** | `dyn Trait` → vtables + `call_indirect` + runtime function address embedding. | ~200-400 new LOC. | **No ADR requires this** |

**G Ruling (2026-06-15):** Tier 1 is sufficient; Tier 2 + Tier 3 are **frozen under absolute YAGNI** — unlocked only when real consumers require them.

### 2.1 Trait/Impl AST — Schema-First (NO Exceptions)

Add to `spec/schema/triet-schema.yaml` **prior to writing Rust code** (G: "arbitrarily hand-crafting AST nodes without updating schema → breaks codebase integrity"):

- `Item::Trait { name, type_parameters, methods: Vec<MethodSignature> }` — `MethodSignature` = bodiless signature (name, params, return_type).
- `Item::Implementation { trait_name, for_type: TypeExpr, methods: Vec<FunctionDefinition> }`.

Invariable sequence: **Schema → Codegen → Parser → Typecheck → Lower.**

### 2.2 Storage — DO NOT Cram into `Type` Enum

Traits/implementations are **relationships**, not "the type of a value". Two new registries in typecheck, parallel to `name_table` (without modifying `Type::UserStruct/UserEnum`):

```
trait_table: HashMap<String, TraitDefinition>                  // trait name → method signatures
impl_table:  HashMap<(TypeName, TraitName), ImplInfo>    // (Integer, Comparable) → { compare → "Integer$Comparable$compare" }
```

`ImplInfo` maps method name → **concrete mangled function name**. Each method in `implement` lowers to a standard `Body` with a mangled name — nothing specialized in MIR/JIT.

**Binding impl to UserStruct/UserEnum:** when typecheck encounters `implement Comparable for Point`, resolve `Point` via `name_table` (already exists), verify methods match trait signatures, record into `impl_table`. **Minimal coherence:** forbid duplicate `(Type, Trait)` implementations → new error code (E1043).

### 2.3 Dispatch — Method-Syntax `a.compare(b)` (G Locked)

**G locked:** trait methods must be invoked via **method syntax `a.compare(b)`**, NOT free-functions. Rationale (G): "maintain structured object-oriented ergonomics rather than C-style clutter".

Execution flow:
1. Typecheck `check_method_call(receiver, method, args)` (`check/methods.rs`) — if `builtin_method_type()` fails, **consult `impl_table`**: query `(receiver_type_name, *)` for traits containing that method name, verify arity + argument types against `MethodSignature`.
2. Record `MethodResolution { concrete_fn: "Integer$Comparable$compare" }` on `ExprId` (template `EnumVariantResolution`, `triet-syntax/src/lib.rs:36-51`), returning the method's `return_type`.
3. Lowerer consumes annotation → `CallDispatch { callee_name: "Integer$Comparable$compare", target: CallTarget::Jit, args: [receiver, ...args] }`. Receiver becomes the first argument.
4. JIT: **no new mechanisms** — direct call path is already functional (`mir_lower.rs:1583-1624`).

### 2.4 Mangling — `Type$Trait$method` (Is a Feature, Not a Bug)

**G locked:** exposing mangled names such as `Integer$Comparable$compare` in MIR Display is a **feature**. Tests + MIR verifiers can directly assert whether typecheck resolved the correct implementation. Expose cleanly in MIR.

### 2.5 ADR-0038 (`Comparable`) — Tier 1 Consumer + One Real Auxiliary Task

- `trait Comparable { compare(other) -> Trit }` + `implement Comparable for Integer/String/Tryte`.
- Dispatched via §2.3.
- **Dedicated auxiliary task (outside dispatch):** ADR-0038 note records `match compare(a,b) { -1_trit => …, 0_trit => …, 1_trit => … }` as a **match on Trit literals — a lowering path DISTINCT from enum SwitchInt** (`enum_layouts`). Requires adding a small **match-on-Trit path**. Split into a separate sub-task in the plan, not bundled into dispatch.

## 3. Rejected Alternatives (Frozen)

- **Tier 2 (Generic monomorphization)** — frozen. `function f<T: Comparable>(v)` has no consumers yet; MIR lacks generic-arg/mangling. Unlocked when stdlib generics land.
- **Tier 3 (Trait object / vtable / dynamic dispatch)** — frozen under YAGNI. Requires `call_indirect` (0 occurrences currently) + runtime function address embedding (~200-400 LOC). No ADR needs this.
- **ADR-0039 (`?+>` nullable-operator family)** — **SPLIT OUT from Trait System** (G locked). That is an operator desugaring over `T?`, not polymorphic dispatch. Truly blocked on **Lambda/closure lowering** (verified "NO Lambda handling in lower"), not traits. Belongs to dedicated "Closure/Lambda Lowering" phase. Bundling into traits creates architectural distortions without addressing the root cause.
- **Default method bodies in traits** — uncommitted (Tier 1 requires complete method definitions in impls). Deferred.

## 4. Consequences

- Reuses existing and proven `EnumVariantResolution` + `CallDispatch::Jit` paths → minimal tech debt, no disruption to the i64 value model, no ABI changes, no new JIT mechanisms.
- Aligns with "stability over speed": avoids building vtables for nonexistent needs (preventing rotten skeleton code like the historical `enum_layouts` dead-field lesson).
- Preserves ternary identity: Comparable routes cleanly through direct dispatch + `match` on Trit.

## 5. References

- [ADR-0038](0038-comparable-trait-deferred.md) — `Comparable`/`compare()->Trit` design-lock.
- [ADR-0039](0039-nullable-operator-family.md) — `?+>` family (split out from this scope).
- [ADR-0037](0037-enum-tagged-union-layout.md) — i64 discriminant (rationale for Trit rather than Ordering enum).
- [ADR-0027](0027-diagnostic-format-standard.md) — format E1043 (coherence conflict).
- [phase13-trait-system.md](../../spec/plans/phase13-trait-system.md) — step-by-step implementation plan.
- Surveyed files:lines: `types.rs:13-117`, `check/methods.rs:12-79`, `syntax/lib.rs:36-51`, `mir/lib.rs:777-800`, `mir_lower.rs:1583-1624`.

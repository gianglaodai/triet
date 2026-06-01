# ADR 0036 — `TypeTag::Opaque`: disambiguating user-defined aggregates from true `Unit`

**Trạng thái:** **Locked** (v0.11.x.jit.4.agg.opaque, author sign-off received). Addresses [ADR-0035 §4](0035-jit-boxed-refcount-discipline.md) (the `TypeTag::Unit` ambiguity ceiling) and supersedes the lowerer's `_ => TypeTag::Unit` catch-all for user-defined struct, enum, and generic types. This is an **IR-shape change** with a `.triv` version bump (v7 → v8) and self-host lockstep requirement.

## Issue

The lowerer maps every user-defined type (struct, enum, generic type parameter) to `TypeTag::Unit` ([`lowerer.rs:757`](../../crates/triet-ir/src/lowerer.rs#L757)). The self-host compiler does the same ([`ir_lowerer.tri:1754`](../../compiler/ir_lowerer.tri#L1754)). This was an acceptable placeholder through v0.10 — the IR doesn't track field layout, and `Unit` served as "something composite, don't look inside." But it creates a **blocking ambiguity** at two JIT decision points:

1. **Cross-mode marshaling ([ADR-0035 §4](0035-jit-boxed-refcount-discipline.md)).** `boundary_class(TypeTag::Unit)` returns `None` → tier down, because the codegen can't tell a zero-sized `Unit` return (needs no action) from a `Rc<RuntimeValue>` struct pointer (needs pass-through). **410 functions** tier down on this ambiguity at 41.0% coverage — the single largest remaining cross-mode blocker.

2. **Boxing decision.** `is_composite_tag` does not include `Unit`, so a function whose only aggregate is a struct typed `Unit` appears to be all-scalar. If it's a composite return, `is_composite_tag` returns false → the unboxed mode skips the clone-on-return discipline → **latent double-free**. Currently masked because such functions hit the cross-mode tier-down first (finding 1), but architecturally unsound.

3. **Clone-on-return ([ADR-0035 §1](0035-jit-boxed-refcount-discipline.md)).** The unboxed `Ret` path uses `is_composite_tag(&func.return_type)` to decide whether to clone a borrowed return. A struct typed `Unit` is misclassified as scalar → no clone → double-free if the function returns a borrowed composite parameter. Same root cause as finding 2.

The compound impact is **~410 cross-mode + a substantial share of ~192 call blockers** — roughly 600 of the ~912 remaining tier-downs at 41.0% coverage. Resolving this ambiguity is the single highest-leverage step toward the bootstrap gate lift.

## Quyết định

**Add `TypeTag::Opaque` — a single new variant meaning "a user-defined aggregate type whose layout the IR does not track (struct, enum, or erased generic type parameter)." The lowerer (both Rust and self-host) emits `Opaque` instead of `Unit` for user-defined types. True `Unit` remains `Unit` and means exclusively the zero-sized unit type `()`. The `.triv` wire format bumps to v8.**

### §1 — The new variant

```rust
pub enum TypeTag {
    // ... existing variants unchanged ...

    /// Zero-sized unit type `()`. Now unambiguously "nothing."
    Unit,

    /// User-defined aggregate (struct, enum, erased generic type
    /// parameter). The IR does not track field layout — Opaque means
    /// "a composite `Rc<RuntimeValue>` pointer at runtime, but the
    /// exact shape is unknown to the IR." Introduced by [ADR-0036] to
    /// resolve the ambiguity where user aggregates and true-Unit were
    /// both `TypeTag::Unit`.
    ///
    /// [ADR-0036]: ../../../../docs/decisions/0036-typetag-opaque-aggregate.md
    Opaque,

    // ... Nullable, Vector, HashMap, Outcome, Atomic unchanged ...
}
```

`Opaque` is deliberately **unparameterized** — it carries no inner type, no name, no field list. It is the minimal signal: "this value is a heap-boxed composite, not a scalar." That's all the JIT's boxing/marshaling/clone decisions need. Carrying richer type information is a Bậc C concern ([ADR-0034](0034-jit-aggregate-coverage.md) Addendum — native aggregate codegen, post-v0.11).

### §2 — Lowerer changes (Rust + self-host lockstep)

**Rust lowerer** (`crates/triet-ir/src/lowerer.rs`, `type_expr_to_tag`):

```rust
// Before:
//   _ => TypeTag::Unit,  // user-defined type OR generic — placeholder

// After:
    _ => TypeTag::Opaque,  // user-defined struct/enum/generic → opaque aggregate
```

Three sites change (lines 757, 775, 778 — the `Named` catch-all, the `Generic` non-Vector/HashMap catch-all, and the `TypeExpr` wildcard).

**Self-host lowerer** (`compiler/ir_lowerer.tri`, `resolve_named_type_tag` + `resolve_type_expr_to_tag`):

```
// Before (line 1754-1756):
//   let r: AllocTagResult = alloc_tag_unit(ctx.type_arena)

// After:
    let r: AllocTagResult = alloc_tag_opaque(ctx.type_arena)
```

New self-host additions:
- `OpaqueTag(PrimitiveMarker)` variant in `enum TypeTag`
- `alloc_tag_opaque` function (mirrors `alloc_tag_unit`)
- `type_tag_display` arm for `OpaqueTag` → `"Opaque"`
- `resolve_named_type_tag` + `resolve_type_expr_to_tag` wildcards → `alloc_tag_opaque`

**Stage 2 ≡ Stage 3 lockstep:** the Rust lowerer and self-host lowerer emit the same `TypeTag::Opaque` disc (12) for user-defined types. Since both currently emit `Unit` (disc 6) for these, and both will change to `Opaque` (disc 12) in the same step, the `.triv` output stays byte-identical between stages. The bootstrap gate (`stage2_eq_stage3_main_tri_byte_identical`) is **not** broken by this change, because:
- Rust compiler (Stage 1) → emits `.triv` with `Opaque` (disc 12) for user types
- Self-host Stage 2 (run by Rust VM) → emits `.triv` with `Opaque` (disc 12) for user types
- Self-host Stage 3 (run by Stage 2 VM) → emits `.triv` with `Opaque` (disc 12) for user types
- Stage 2 `.triv` ≡ Stage 3 `.triv` ✓ (both use the self-host lowerer, which now emits `Opaque`)

The key invariant: **Stage 2 and Stage 3 use the same self-host compiler source** → their outputs are identical regardless of what the Rust compiler does. The Rust compiler's `.triv` output is compared against Stage 2 only for the bootstrap test, and that test compares the *self-host compiled* output, not the Rust-compiled output.

### §3 — `.triv` wire format bump (v7 → v8)

**New discriminant:** `Opaque` = disc **12** (payload-free, like `Unit` disc 6).

```rust
// write_type_tag:
TypeTag::Opaque => write_u8(buf, 12),

// read_type_tag:
12 => Ok(TypeTag::Opaque),
```

Version bump to v8 per [ADR-0008](0008-triv-binary-format.md) §"Version compatibility": a new type discriminant that old readers can't parse = patch-level bump. Old readers (v7) encountering disc 12 get `TrivError::UnknownTypeDiscriminant(12)` — the existing error path, correct behavior.

**Note:** while bumping, also add the missing disc 11 (Atomic) reader arm, which `write_type_tag` emits but `read_type_tag` currently falls through to the error case. This is a pre-existing gap — fixed opportunistically.

### §4 — JIT codegen integration

**`is_composite_tag`** — add `TypeTag::Opaque`:

```rust
const fn is_composite_tag(tag: &TypeTag) -> bool {
    matches!(
        tag,
        TypeTag::String
            | TypeTag::Vector(_)
            | TypeTag::HashMap(..)
            | TypeTag::Nullable(_)
            | TypeTag::Atomic(_)
            | TypeTag::Outcome { .. }
            | TypeTag::Opaque  // NEW
    )
}
```

**`boundary_class`** — classify `Opaque` as `PassThrough`:

```rust
fn boundary_class(tag: &TypeTag) -> Option<BoundaryClass> {
    // ...
    TypeTag::Opaque => Some(BoundaryClass::PassThrough),  // NEW — was tier-down via Unit
    // `Unit` remains None (true Unit: zero-sized, no ptr) — but see §5.
    // ...
}
```

Wait — true `Unit` should **also** be classifiable now. A function returning `Unit` returns nothing meaningful; the boxed mode boxes it as `Rc<RuntimeValue::Unit>`, but the cross-mode boundary can handle it as a **scalar** (box/unbox the Unit constant). Or simpler: a `Unit`-returning cross-mode call can just be treated as `PassThrough` — the `Unit` box is an `Rc<RuntimeValue::Unit>` pointer same as any composite, it'll be leaked (one box, cold path, within §3 tolerance of [ADR-0035](0035-jit-boxed-refcount-discipline.md)) or correctly handled.

**Decision for `Unit`:** classify true `Unit` as `PassThrough` too. `Unit` is still an `Rc<RuntimeValue>` pointer in boxed mode — it has the same representation as any composite. With `Opaque` absorbing all the user-aggregate traffic, `Unit` cross-mode calls are rare (only explicit `Unit`-typed parameters/returns), and `PassThrough` is the correct treatment (no box/unbox needed — same ptr repr).

```rust
fn boundary_class(tag: &TypeTag) -> Option<BoundaryClass> {
    let scalar = |kind, clif| Some(BoundaryClass::Scalar { kind, clif });
    match tag {
        TypeTag::Integer => scalar(JitConstKind::Integer, I64),
        TypeTag::Trilean => scalar(JitConstKind::Trilean, I8),
        TypeTag::Trit => scalar(JitConstKind::Trit, I8),
        TypeTag::Tryte => scalar(JitConstKind::Tryte, I16),
        TypeTag::Unit | TypeTag::Opaque => Some(BoundaryClass::PassThrough),
        _ if is_composite_tag(tag) => Some(BoundaryClass::PassThrough),
        // `Long` (i128) → tier down (deferred).
        _ => None,
    }
}
```

This removes `Unit` and `Opaque` from the tier-down path. Only `Long` (i128, ~0 occurrences in self-host) remains unclassified.

**Clone-on-return impact (ADR-0035 §1):** `is_composite_tag` now includes `Opaque` → the unboxed `Ret` correctly clones a borrowed `Opaque`-typed return. `Unit` is NOT added to `is_composite_tag` because a true `Unit` return in unboxed mode is value-copy (the `i64` encoding of `Unit` is a constant, not a refcounted pointer), so cloning it would be incorrect. If a `Unit`-returning unboxed function returns a borrowed parameter, it's returning a `Unit` constant — value-copy, no double-free possible.

### §5 — `is_boxed` impact

`is_boxed(func)` checks whether a function touches any aggregate opcode (struct/enum/outcome/nullable ops). It does **not** check `TypeTag` — it checks the *opcodes* in the function body. So `TypeTag::Opaque` does not directly affect `is_boxed`. However, functions that currently tier down on cross-mode `Unit` boundaries will now JIT successfully, and their opcode mix determines boxing. No change needed.

### §6 — Scope and file whitelist

Files modified:

| Crate | File | Change |
|---|---|---|
| `triet-ir` | `src/types.rs` | Add `Opaque` variant to `TypeTag` enum + `Display` arm |
| `triet-ir` | `src/lowerer.rs` | 3 sites: `_ => TypeTag::Unit` → `TypeTag::Opaque` |
| `triet-ir` | `src/serde.rs` | Disc 12 write/read + version bump 7→8 + disc 11 Atomic read fix |
| `triet-ir` | `src/lib.rs` | Re-export (TypeTag is already re-exported — no change expected) |
| `triet-ir` | `src/vm.rs` | `RuntimeValue::type_tag()`: `Struct/Enum/Closure => TypeTag::Opaque` (line 197); wildcard inner types stay `Unit` |
| `triet-jit` | `src/codegen.rs` | `map_type` (`Opaque` → I64) + `is_composite_tag` + `boundary_class` |
| `triet-ir` | tests | TypeTag display, serde round-trip, version pin v8 |
| self-host | `compiler/ir_lowerer.tri` | `OpaqueTag` variant + `alloc_tag_opaque` + lowerer wildcards |
| self-host | `compiler/pack_writer.tri` | `TYPE_TAG_OPAQUE = 12` + `TRIV_VERSION` 7→8 + `write_type_tag`/`type_tag_discriminator`/`type_tag_eq` |
| docs | `docs/decisions/0008-triv-binary-format.md` | Version history v8 entry |

**NOT modified** (scope guard):
- No new opcodes. No new instructions. No IR semantics change.
- No `.triv` section layout change (only a new type discriminant in the existing type table).
- No `unsafe` code.
- No ABI change — `Opaque` has the same runtime representation as any composite (`Rc<RuntimeValue>` pointer / `i64`).

### §7 — Acceptance criteria

1. **`cargo test --workspace`** — all green (baseline ≥ 1676).
2. **`cargo clippy --workspace --all-targets -- -D warnings`** — zero warnings.
3. **JIT audit re-measurement** — `cargo test -p triet-bootstrap --test jit_tier_down_audit -- --ignored --nocapture`: cross-mode blocker category drops from ~410 to near-zero (residual = `Long`-typed boundaries only). Coverage % increases significantly (target: > 50%).
4. **`.triv` round-trip** — existing programs serialize/deserialize correctly with the new disc 12. A v7 reader rejects disc 12 with `UnknownTypeDiscriminant`.
5. **Bootstrap gate** — Stage 2 `.triv` ≡ Stage 3 `.triv` (byte-identical) — the self-host lockstep holds.
6. **Value parity** — all existing JIT vs VM `assert_rv_eq` tests pass. No new double-frees (the clone-on-return discipline now covers `Opaque` returns).

### §8 — Implementation order

1. **Rust `TypeTag` + serde** — add `Opaque` variant, disc 12 write/read, version bump, disc 11 fix. Run `cargo test -p triet-ir`.
2. **Rust lowerer** — change 3 sites. Run `cargo test --workspace` (Stage 2 ≡ Stage 3 will FAIL here because self-host still emits `Unit` — expected).
3. **Self-host lockstep** — add `OpaqueTag` + `alloc_tag_opaque` + lowerer changes. Run `cargo test --workspace` (Stage 2 ≡ Stage 3 should pass — both now emit `Opaque`).
4. **JIT codegen** — update `is_composite_tag`, `boundary_class`, any `TypeTag::Unit` pattern matches in codegen.rs that need an `Opaque` arm.
5. **Re-audit** — measure coverage improvement.
6. **Cleanup** — `TypeTag::Display`, doc comments, ADR-0008 version history, this ADR status → Locked.

Steps 1–3 are the **self-host lockstep critical path** — they must be committed together (or in a sequence that never leaves Stage 2 ≢ Stage 3 on `main`). Step 4 is safe to separate.

## Không làm

- **Parameterized `TypeTag::Struct(name)` / `TypeTag::Enum(name, variants)`.** Would give the IR full structural type info. Rejected for v0.11: the JIT doesn't need field layout to box/marshal correctly (`Opaque` suffices), and carrying names/fields through the IR + `.triv` + self-host is a large, high-risk change. This is Bậc C territory — native aggregate codegen needs field layout, but that phase replaces boxing entirely and will introduce its own type-info mechanism. Don't pay the cost now for a benefit that only arrives post-v0.11.

- **Multiple `TypeTag` variants (`Struct`/`Enum`/`Generic`).** Would distinguish the three kinds of user aggregates. Rejected: the JIT's boxing and marshaling decisions don't vary by aggregate kind — struct, enum, and generic-instantiation are all `Rc<RuntimeValue>` pointers, all boxed, all pass-through at boundaries. One `Opaque` is sufficient and simpler.

- **Removing `TypeTag::Unit` entirely (replacing with `Opaque`).** Would simplify the enum but lose the "true zero-sized Unit" signal. True `Unit` has specific semantics: it's the return type of void functions, it's not a composite, it doesn't need cloning. Keeping `Unit` distinct from `Opaque` is essential for correct clone-on-return (§4) and prevents needlessly boxing void functions.

- **A breaking `.triv` major version (v2.0).** Would allow reshuffling all discriminants. Rejected: a patch bump (v7 → v8) with a new disc 12 is sufficient, backward-compatible on the read side (old readers reject cleanly), and doesn't require coordinating a major version migration across tools.

## Prior art

- **LLVM `%opaque = type opaque`** — LLVM IR supports opaque (incomplete) struct types whose body is not known. Used for forward declarations and type erasure. Triết's `TypeTag::Opaque` serves a similar purpose: "this is a composite, but the IR doesn't describe its structure."

- **JVM's `Object` type** — the JVM's type system erases generics to `Object` at the bytecode level, similar to how Triết erases user aggregates to `Opaque`. The JVM's verifier can still reason about reference/value distinction (which is all the JIT needs here).

- **Cranelift's `AbiParam`** — Cranelift's own IR doesn't have "opaque" types, but its calling convention layer treats all pointer-width values as `I64` regardless of what they point to. Triết's boxed mode does the same — `Opaque` confirms "this is an `I64` pointer to a heap object" at the Triết IR level.

## Tham chiếu

- [ADR-0035 §4](0035-jit-boxed-refcount-discipline.md) — the `TypeTag::Unit` ambiguity ceiling this ADR resolves.
- [ADR-0034](0034-jit-aggregate-coverage.md) — Bậc A per-function uniform boxing + the oracle/Bậc-C staging.
- [ADR-0008](0008-triv-binary-format.md) — `.triv` wire format + version compatibility rules.
- [ADR-0007](0007-ir-design.md) — IR design + `TypeTag` original specification.
- [ADR-0019](0019-self-hosting-compiler-bootstrap.md) — self-host compiler + 3-stage bootstrap + byte-identical gate.
- `TODO.md` — v0.11.x.jit.4 `TypeTag::Unit` ceiling entry.

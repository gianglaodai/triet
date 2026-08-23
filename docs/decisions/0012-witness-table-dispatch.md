# ADR 0012 — Witness table dispatch for cross-package generics

**Status:** Decision. Applicable to v0.4 cross-package linker + runtime dispatch. Referenced by ADR-0011 (ABI metadata) for generic encoding.

**Issue:** ABI stability for generics is the most challenging aspect of system-level packaging. Two common approaches:

| Approach | Language | ABI stability | Speed | Code size |
|---|---|---|---|---|
| **Monomorphization** | Rust, C++ templates | ❌ Breaks ABI when caller changes instantiation | ✅ Optimal speed | ❌ Bloat |
| **Type erasure** | Java generics | ✅ Stable | ⚠️ Boxing overhead | ✅ Compact |
| **Witness tables** | Swift | ✅ Stable | ✅ Near-native via vtable | ✅ Compact |

VISION §3.3 has finalized: **hybrid** — monomorphization intra-package (Rust speed), witness tables cross-package (Swift stability). This ADR locks the format and runtime semantics for witness table dispatch.

## Decision

### 1. Two dispatch modes

The lowerer distinguishes **at compile time** based on callee location:

| Callsite | Callee location | Dispatch | Rationale |
|---|---|---|---|
| `foo(x: T)` within the same package | Local | Monomorphize per instantiation | Hot path, full inlining opportunity |
| `foo(x: T)` from another package | External `.khi` | Witness call via table | ABI stability; recompiling the parent does not break the child |

This is a **compile-time decision**, not a runtime one — there is no dispatch-time cost for distinguishing.

### 2. Witness table layout

When a generic function `f<T>(...)` is exported from a package, the ABI metadata does NOT ship monomorphized copies. Instead:

```
Witness table for call site `f<Integer>(x)` from a consumer package:
┌────────────────────────────────────────────────────────────┐
│ slot 0: type metadata for T = Integer                       │
│   - size_in_trits: 27 (varint)                              │
│   - type_id: TypeRef per ADR-0011 § 2                       │
│ slot 1+: required-operation function pointers (currently 0)  │
│   - reserved for v0.6 capability checks                     │
│   - reserved for v0.7 trait/protocol dispatch               │
└────────────────────────────────────────────────────────────┘
```

The caller builds the witness table **at link time** (the caller package's compile time, when resolving cross-package calls). The witness table resides in the caller's `.khi` data section, referencing the callee package's ABI metadata.

### 3. New IR instruction: `WitnessCall`

Add a new opcode to the IR (additive to ADR-0007):

```rust
Instruction::WitnessCall {
    dest: Option<ValueId>,
    /// Path to cross-package function (resolved via dep table).
    path: AbsolutePath,
    /// Index of the witness table in the caller's data section.
    /// Linker populates this after resolving generic instantiation.
    witness_idx: u32,
    args: Vec<Operand>,
}
```

Unlike `CallCrossModule` in that:
- `CallCrossModule` uses a fully resolved function ID (non-generic).
- `WitnessCall` carries a `witness_idx` allowing the callee to dispatch based on type metadata.

VM dispatch:
1. Load the witness table at `witness_idx`.
2. Lookup the callee function via `path` in the cross-package symbol table.
3. Push the frame with args + the witness table as the implicit last argument.
4. The callee can read type info via the `__witness_type(0)` intrinsic (slot 0 = T's metadata).

### 4. Encoding in `.khi`

The ABI metadata exports table (ADR-0011 §3) already includes `type_param_count`. When the caller resolves a generic call:

```
Caller .tripack:
  abi_metadata.exports[caller_fn]
  abi_metadata.types[*]
  data.witness_tables[]:
    [0]: WitnessTable {
      callee_path: "math.scale",
      type_args: [TypeRef::Primitive(Integer)],
      // operation refs filled when v0.6 lands
    }
  code.* (uses WitnessCall { witness_idx: 0, ... })
```

Witness tables are **part of the caller**, not the callee. Each caller package ships witness tables for every unique generic instantiation **it creates**. The callee only ships a single generic function body.

### 5. Dispatch performance

| Operation | Cost (bytecode VM) | Cost (LLVM AOT, v2.0) |
|---|---|---|
| Witness call setup | 1 table lookup + 1 indirect call | 1 mov + 1 indirect call |
| Type metadata read | 1 array index | 1 mov |
| Compared to direct call | ~2× slower | <10% slower on modern CPUs |

For hot paths within the same package (monomorphized): zero overhead compared to v0.3.

### 6. Generic constraints supported in v0.4

v0.4 only implements **unconstrained generics** (as in the current v0.2). The witness table only contains type metadata, no operations. Future expansion:

- v0.6: capability constraints (`fn f<T: Send>(x: T)` requires a witness entry for the `Send` marker).
- v0.7+: trait/protocol constraints (`fn f<T: Display>(x: T)` requires entries for `display` methods).

The structure of reserved entry slots allows for later addition without bumping the `abi_version`.

### 7. Witness table identity & deduplication

Two call sites with the same generic instantiation share a witness table:

```triet
let a = math.scale<Integer>(5)
let b = math.scale<Integer>(10)  // shares witness table with a
let c = math.scale<Long>(20)     // different witness table
```

The linker deduplicates based on the `(callee_path, type_args)` key. This reduces the data section size for generic-heavy code.

### 8. Cross-package recompile invariant

When the caller modifies the body (impl), if the interface remains unchanged → witness tables remain the same → the callee package does not need to be rebuilt.

When the callee modifies the generic body (impl), if the `iface_hash` remains unchanged → the caller package does not need to be rebuilt → existing witness tables remain valid.

When the callee changes the ABI surface of a generic function (param/return type), the `iface_hash` changes → the caller must rebuild witness tables → semver checks kick in (ADrag-0013).

## Consequences

### For IR (ADR-0007)

- Add `Instruction::WitnessCall` (additive, does not break `.triv` v2).
- Bump `.triv` v2 → v3 when the `WITNESS_CALL` opcode is serialized.

### For VM

- Add a dispatch path for `WitnessCall` — load the witness table from the caller's data, lookup the callee.
- VM tests must cover the witness dispatch path.

### For the lowerer

- Distinguish local vs. cross-package generics at compile time.
- Local: monomorphize as in the current v0.2 (unchanged).
- Cross-package: emit `WitnessCall` + register a witness table entry.

### For the linker (v0.4.5)

- Build a witness table for each unique `(callee_path, type_args)`.
- Deduplicate tables across call sites.
- Output witness tables into the caller's `.khi` data section.

### For JIT (v0.9) and LLVM AOT (v2.0)

- Lower witness calls to indirect calls (1 address load + 1 call). Modern CPU branch predictors handle this well.
- Optional specialization can inline when the witness table is known at compile time.

### For the trytecode backend (v∞)

- The witness table layout uses native Trit slots (capability constraints are Trit grant/deny/ambient per VISION §3.5).
- On ternary hardware: 1 trit witness check instead of an 8-bit byte → memory efficiency.

## Alternatives Considered

- **Specialization of witness calls** (auto-inline within the same package): defer to v0.9+. v0.4 maintains dispatch via the witness table for cross-package calls.
- **Variance** (`<T : Sub>` vs `<+T>` vs `<-T>`): not available in v0.4. Sub-typing variance is a v0.7+ topic.
- **Higher-kinded types** (`F<G<_>>`): defer indefinitely. Triet does not commit to supporting this.
- **Const generics** (`fn arr<const N: Integer>`): defer to v0.5+. Requires hash stability for const values in ABI metadata.
- **Trait objects / dynamic dispatch at the function value level**: defer. v0.4 only supports generic functions, not generic values.

## Prior art

- **Swift witness tables** — primary reference. Triet's design is nearly identical.
- **Rust trait objects (vtables)** — similar concept but tied to dynamic dispatch, not compile-time-resolved generics.
- **Haskell type class dictionaries** — same idea from the academic side. Witness table is Swift's renaming of dictionary passing.
- **C++ vtables for virtual methods** — anti-prior-art: tied to runtime polymorphism, not a stable ABI.

## References

- [VISION §3.3 — Stable ABI generics](../../VISION.md)
- [ADR-0007 — IR design](0007-ir-design.md) (this ADR extends the opcode table)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (will bump version)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (companion)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (companion)
- [Swift Generics Manifesto](https://github.com/apple/swift/blob/main/docs/GenericsManifesto.md)
- ["Implementing Swift Generics" — WWDC 2017 talk](https://devstreaming-cdn.apple.com/videos/wwdc/2017/406hxqdgg2hbxop/406/406_implementing_swift_generics.pdf)

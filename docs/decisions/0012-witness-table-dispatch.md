# ADR 0012 — Witness table dispatch for cross-package generics

**Trạng thái:** Quyết định. Áp dụng cho v0.4 cross-package linker + runtime dispatch. Tham chiếu bởi ADR-0011 (ABI metadata) cho generic encoding.

**Issue:** ABI stability cho generics là vấn đề khó nhất của system-level packaging. Hai approach phổ biến:

| Approach | Ngôn ngữ | ABI stability | Speed | Code size |
|---|---|---|---|---|
| **Monomorphization** | Rust, C++ templates | ❌ Phá ABI khi caller đổi instantiation | ✅ Tốc tối ưu | ❌ Bloat |
| **Type erasure** | Java generics | ✅ Stable | ⚠️ Boxing overhead | ✅ Compact |
| **Witness tables** | Swift | ✅ Stable | ✅ Near-native qua vtable | ✅ Compact |

VISION §3.3 đã chốt: **hybrid** — monomorphization intra-package (Rust speed), witness tables cross-package (Swift stability). ADR này lock format và runtime semantics cho witness table dispatch.

## Quyết định

### 1. Hai chế độ dispatch

Lowerer phân biệt **at compile time** dựa trên callee location:

| Callsite | Callee location | Dispatch | Lý do |
|---|---|---|---|
| `foo(x: T)` trong cùng package | Local | Monomorphize per instantiation | Hot path, full inline opportunity |
| `foo(x: T)` từ package khác | External `.tripack` | Witness call qua table | ABI stability, recompile cha không phá con |

Đây là **decision tại compile time**, không phải runtime — không có cost phân biệt khi dispatch.

### 2. Witness table layout

Khi function generic `f<T>(...)` được export từ package, ABI metadata KHÔNG ship monomorphized copies. Thay vào đó:

```
Witness table cho call site `f<Integer>(x)` từ package consumer:
┌────────────────────────────────────────────────────────────┐
│ slot 0: type metadata for T = Integer                       │
│   - size_in_trits: 27 (varint)                              │
│   - type_id: TypeRef per ADR-0011 § 2                       │
│ slot 1+: required-operation function pointers (hiện tại 0)  │
│   - reserved cho v0.6 capability checks                     │
│   - reserved cho v0.7 trait/protocol dispatch               │
└────────────────────────────────────────────────────────────┘
```

Caller build witness table **at link time** (compile-time của caller package, khi resolve cross-pkg call). Witness table sống trong data section của caller `.tripack`, reference ABI metadata của callee package.

### 3. New IR instruction: `WitnessCall`

Bổ sung opcode mới vào IR (ADR-0007 additive):

```rust
Instruction::WitnessCall {
    dest: Option<ValueId>,
    /// Path to cross-package function (resolved via dep table).
    path: AbsolutePath,
    /// Index của witness table trong caller's data section.
    /// Linker populate sau khi resolve generic instantiation.
    witness_idx: u32,
    args: Vec<Operand>,
}
```

Khác với `CallCrossModule` ở chỗ:
- `CallCrossModule` đi qua function ID đã resolve hoàn toàn (non-generic).
- `WitnessCall` carry `witness_idx` cho phép callee dispatch dựa trên type metadata.

VM dispatch:
1. Load witness table tại `witness_idx`.
2. Lookup callee function via `path` trong cross-package symbol table.
3. Push frame với args + witness table as implicit last arg.
4. Callee có thể read type info qua intrinsic `__witness_type(0)` (slot 0 = T's metadata).

### 4. Encoding trong `.tripack`

ABI metadata exports table (ADR-0011 §3) đã có `type_param_count`. Khi caller resolve generic call:

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

Witness tables là **part of caller**, không phải callee. Mỗi caller package ship witness tables cho mỗi unique generic instantiation **mà nó tạo ra**. Callee chỉ ship một generic function body.

### 5. Dispatch performance

| Operation | Cost (bytecode VM) | Cost (LLVM AOT, v2.0) |
|---|---|---|
| Witness call setup | 1 table lookup + 1 indirect call | 1 mov + 1 call indirect |
| Type metadata read | 1 array index | 1 mov |
| Compare với direct call | ~2× chậm hơn | <10% slower trên modern CPU |

Đối với hot paths cùng package (monomorphize): zero overhead so với v0.3.

### 6. Generic constraint hỗ trợ ở v0.4

v0.4 chỉ implement **unconstrained generics** (như v0.2 hiện tại). Witness table chỉ chứa type metadata, không operations. Future expansion:

- v0.6: capability constraints (`fn f<T: Send>(x: T)` cần witness entry cho Send marker).
- v0.7+: trait/protocol constraints (`fn f<T: Display>(x: T)` cần entries cho display methods).

Cấu trúc reserved entry slots cho phép thêm sau mà không bump `abi_version`.

### 7. Witness table identity & deduplication

Hai call sites với same generic instantiation share witness table:

```triet
let a = math.scale<Integer>(5)
let b = math.scale<Integer>(10)  // share witness table với a
let c = math.scale<Long>(20)     // witness table khác
```

Linker dedup theo `(callee_path, type_args)` key. Reduce data section size cho generic-heavy code.

### 8. Cross-package recompile invariant

Khi caller modify body (impl), iface không đổi → witness tables stay same → callee package không cần rebuild.

Khi callee modify generic body (impl), iface_hash không đổi → caller package không cần rebuild → existing witness tables vẫn valid.

Khi callee thay đổi ABI surface của generic function (param/return type), iface_hash đổi → caller must rebuild witness tables → semver check kick in (ADR-0013).

## Hệ quả

### Đối với IR (ADR-0007)

- Thêm `Instruction::WitnessCall` (additive, không phá .triv v2).
- Bump `.triv` v2 → v3 khi opcode WITNESS_CALL được serialize.

### Đối với VM

- Thêm dispatch path cho WitnessCall — đọc witness table từ caller's data, lookup callee.
- VM tests cần cover witness dispatch path.

### Đối với lowerer

- Phân biệt local generic vs cross-package generic ở compile time.
- Local: monomorphize như v0.2 hiện tại (không đổi).
- Cross-package: emit WitnessCall + register witness table entry.

### Đối với linker (v0.4.5)

- Build witness table cho mỗi unique (callee_path, type_args).
- Dedup tables across call sites.
- Output witness tables vào caller's `.tripack` data section.

### Đối với JIT (v0.9) và LLVM AOT (v2.0)

- Witness call lower thành indirect call (1 đếm address load + 1 call). Native CPU branch predictor handle tốt.
- Specializer optional có thể inline khi witness table biết tại compile time.

### Đối với trytecode backend (v∞)

- Witness table layout dùng Trit slots tự nhiên (capability constraint là Trit grant/deny/ambient per VISION §3.5).
- Trên hardware tam phân: 1 trit witness check thay vì 8-bit byte → memory hiệu quả.

## Không làm

- **Specialization của witness calls** (auto-inline cùng package): defer v0.9+. v0.4 keep dispatch luôn qua witness table cho cross-package.
- **Variance** (`<T : Sub>` vs `<+T>` vs `<-T>`): không có ở v0.4. Sub-typing variance là v0.7+ topic.
- **Higher-kinded types** (`F<G<_>>`): defer indefinitely. Triết không cam kết support.
- **Const generics** (`fn arr<const N: Integer>`): defer v0.5+. Cần hash stability cho const values trong ABI metadata.
- **Trait objects / dynamic dispatch tại function value level**: defer. v0.4 chỉ generic functions, không generic values.

## Prior art

- **Swift witness tables** — chính. Triết design gần đúng identical.
- **Rust trait objects (vtables)** — similar concept nhưng tied vào dynamic dispatch, không phải compile-time-resolved generics.
- **Haskell type class dictionaries** — same idea từ academic side. Witness table là Swift's renaming của dictionary passing.
- **C++ vtables for virtual methods** — anti-prior-art: tied vào runtime polymorphism, không stable ABI.

## Tham chiếu

- [VISION §3.3 — Stable ABI generics](../../VISION.md)
- [ADR-0007 — IR design](0007-ir-design.md) (this ADR extends opcode table)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md) (will bump version)
- [ADR-0011 — ABI metadata format](0011-abi-metadata-format.md) (companion)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (companion)
- [Swift Generics Manifesto](https://github.com/apple/swift/blob/main/docs/GenericsManifesto.md)
- ["Implementing Swift Generics" — WWDC 2017 talk](https://devstreaming-cdn.apple.com/videos/wwdc/2017/406hxqdgg2hbxop/406/406_implementing_swift_generics.pdf)

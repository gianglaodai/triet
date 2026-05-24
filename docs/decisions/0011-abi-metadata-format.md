# ADR 0011 — ABI metadata format

**Trạng thái:** Quyết định. Áp dụng cho v0.4 Crate-Pack format và mọi linker/loader đọc cross-package interface kể từ v0.4. Format này tách biệt khỏi IR bytecode (ADR-0008) để cho phép linker reject mismatch trước khi load code.

**Issue:** v0.3 dừng ở mức single-package. Mỗi `.triv` file là một `IrProgram` flat — không có biên crate-pack, không có khái niệm "tôi expose gì cho người khác". Để v0.4 enable distribution + cross-package linking, cần một format binary mô tả **ABI surface** của một package:

- Danh sách function exports với signature đầy đủ (param types + return type).
- Type definitions (struct, enum) mà export functions reference.
- Generic constraints và type parameter slots.
- Dependency declarations (package này phụ thuộc package nào, version range nào).
- Capability claims (placeholder cho v0.6).
- Version field cho semver linking policy (ADR-0013).

VISION §3.3 cam kết: *"Compiler là gatekeeper: cross-package mismatch = refuse-to-link với diagnostic rõ ràng"*. Điều này yêu cầu ABI metadata phải:

1. **Hash-stable** — same source ⇒ same bytes. Tiền đề cho v0.5 CAS.
2. **Compact** — đọc cho linker rất nhanh, không cần decode toàn bộ code section.
3. **Versioned** — backwards-compat encoding (additive only sau v1.0).
4. **Self-describing** — tool ngoài compiler đọc được mà không cần Triết source.

ADR này lock format binary cho ABI metadata + relationship với `.triv` (ADR-0008) và `.khi` (ADR-0014 sẽ ghi, container format).

## Quyết định

ABI metadata là **một section binary độc lập** trong `.khi` container, encode theo cùng convention với `.triv` (little-endian, LEB128 varint, length-prefixed UTF-8). Linker đọc **chỉ section này** để quyết định refuse/accept link, không cần load IR code section.

### 1. Top-level layout

```
┌─────────────────────────────────────────────────────────────┐
│ ABI metadata section (in .tripack)                          │
├──────────────┬──────────────────────────────────────────────┤
│ abi_version  │ u32 LE — bumped khi format thay đổi (start = 1) │
│ pkg_name     │ length-prefixed UTF-8 — e.g. "std", "user.app"  │
│ pkg_version  │ semver triple (u32 major, u32 minor, u32 patch) │
│ iface_hash   │ 32 bytes — BLAKE3 của canonical ABI surface     │
│ impl_hash    │ 32 bytes — BLAKE3 của ABI + IR code (v0.5 prep) │
├──────────────┴──────────────────────────────────────────────┤
│ types        │ Type definition table (struct, enum, generic)  │
│ exports      │ Function export table (signature + capability) │
│ deps         │ Dependency declaration table                   │
│ caps         │ Capability claims (v0.6 placeholder, 0 entries)│
└─────────────────────────────────────────────────────────────┘
```

### 2. Type definition table

Mỗi entry mô tả một user-defined type được referenced bởi exports. Phân biệt struct vs enum vs generic-shell:

```
type_count: varint
for i in 0..type_count:
    type_kind: u8  // 0 = struct, 1 = enum, 2 = generic-shell
    name: length-prefixed UTF-8
    type_param_count: varint
    for j in 0..type_param_count:
        param_name: length-prefixed UTF-8
        // future: constraint slots (v0.6 capability)
    body: encoded inline (kind-specific)
```

**Struct body**:
```
field_count: varint
for each field:
    field_name: length-prefixed UTF-8
    field_type: TypeRef
    visibility: u8 (0 = public, 1 = package, 2 = private)
```

**Enum body**:
```
variant_count: varint
for each variant:
    variant_name: length-prefixed UTF-8
    payload_type: Option<TypeRef> (1 byte flag + TypeRef nếu Some)
```

**TypeRef** (referenced types — primitive, defined, or type-param):
```
ref_kind: u8
  0x00 = primitive (next byte = TypeTag from ADR-0007)
  0x01 = local type (next varint = type table index)
  0x02 = type parameter (next varint = type_param index in current scope)
  0x03 = external type (next varint = dep table index, then varint = type index in that pkg)
  0x04 = nullable wrapper (next: inner TypeRef)
  0x05 = generic instantiation (next varint = base type idx, then count + sequence of TypeRef)
```

### 3. Function export table

```
export_count: varint
for each export:
    name: length-prefixed UTF-8
    visibility: u8 (only 0 = public exported, but slot reserved)
    type_param_count: varint
    for each type param: name (length-prefixed UTF-8)
    param_count: varint
    for each param:
        param_name: length-prefixed UTF-8
        param_type: TypeRef
    return_type: TypeRef
    capability_count: varint (placeholder, 0 ở v0.4)
    for each capability: (reserved encoding)
    body_offset: varint
      // Offset vào IR code section của .khi — linker không cần đọc,
      // chỉ runtime/JIT cần để dispatch. 0 = abstract (no body, future).
```

### 4. Dependency table

```
dep_count: varint
for each dep:
    pkg_name: length-prefixed UTF-8
    version_min: semver triple (major, minor, patch)
    version_max_exclusive: semver triple  // 0,0,0 = open-ended
    iface_hash_pin: 32 bytes  // 0s = no hash pin (allow any matching version)
```

Khi `iface_hash_pin` non-zero, linker phải match exact hash → đây là cơ chế CAS-pinning của v0.5 (tiền đề, không enforce ở v0.4).

### 5. Capability claims (v0.6 placeholder)

```
cap_count: varint  // luôn 0 ở v0.4
// each entry sẽ encode: namespace (sys/dev/usr), capability name,
// grant/deny trit. Format hoàn thiện ở v0.6 ADR.
```

### 6. Canonical encoding rules (cho hash stability)

Để `iface_hash` ổn định qua re-compile (yêu cầu cho v0.5 CAS):

- **Type table order**: sort theo `name` lexicographically.
- **Export table order**: sort theo `name` lexicographically.
- **Dep table order**: sort theo `pkg_name` lexicographically.
- **Type param names**: bảo toàn order khai báo trong source (positional).
- **No comments / no whitespace** — binary format, mỗi byte có ý nghĩa.
- **Variable encoding**: LEB128 không bao gồm trailing zero padding.

`iface_hash` = BLAKE3 của bytes từ `pkg_name` end-to-end của `caps` section. Loại trừ `abi_version`, `pkg_version`, `impl_hash` (vì các field đó change mỗi commit dù ABI surface không đổi).

`impl_hash` = BLAKE3 của (`iface_hash` bytes + IR code section bytes). Khi impl đổi mà ABI không, `iface_hash` giữ nguyên, downstream không cần rebuild.

### 7. ABI version policy

`abi_version = 1` ở v0.4 launch. Bump khi:
- Thêm trường mới mà không backwards-compat (rare — additive fields nên dùng reserved space).
- Đổi encoding của trường hiện có (đừng).

Bumping yêu cầu ADR mới. Linker với `abi_version > supported` → refuse với error code E2301.

### 8. Relationship với `.triv` và `.khi`

| Format | Mục đích | ADR |
|---|---|---|
| `.triv` | IR bytecode của một compilation unit | ADR-0008 |
| ABI metadata | Interface surface, version, hashes, deps | **ADR-0011 (this)** |
| `.khi` | Container: ABI metadata + N `.triv` units + manifest | ADR-0014 (TBD) |

Linker workflow:
1. Mở `.khi` → đọc ABI metadata section đầu tiên (cheap).
2. Resolve deps (đọc ABI section của các `.khi` phụ thuộc).
3. Version check per ADR-0013 → refuse hoặc accept.
4. Khi accept: load `.triv` code section vào VM, build cross-package symbol table.
5. Lúc runtime/JIT: witness table dispatch (ADR-0012) cho generic cross-pkg calls.

## Hệ quả

### Đối với v0.5 (CAS)

- `iface_hash` đã hash-stable → CAS resolver dùng được ngay.
- Hai cấp `iface_hash` (ABI) + `impl_hash` (toàn nội dung) đã định nghĩa, không cần redesign.

### Đối với v0.6 (Capability)

- Capability claims slot đã reserved trong format. v0.6 chỉ cần populate, không bump `abi_version`.

### Đối với linker performance

- Linker đọc metadata ~1-10 KB vs full `.khi` ~100KB-1MB. Cheap version check trước khi load code.
- Refuse-to-link diagnostic show diff metadata, không cần show IR.

### Đối với generic ABI stability

- Generic type slot encode bằng index (varint) thay vì monomorphized type → cross-pkg call site giữ nguyên metadata bytes khi caller đổi instantiation.
- Witness table layout (ADR-0012) tham chiếu type table index → stable across recompiles.

## Không làm

- **Không text format đi kèm**: binary only. Tool `triet pack inspect` dump human-readable nhưng không phải canonical.
- **Không version field cho từng export riêng**: package-level versioning đủ. Granular versioning là Rust SemVer hell tránh được.
- **Không capability runtime enforcement ở v0.4**: chỉ slot reserved. Refuse-to-link dựa trên capability mismatch chỉ áp dụng từ v0.6.
- **Không cross-arch ABI** (32-bit vs 64-bit): IR là arch-independent (per ADR-0007 §4), ABI metadata kế thừa.
- **Không phát minh hash scheme**: BLAKE3 — chuẩn industry, không patent, fast, 32-byte output.

## Prior art

- **Swift `.swiftmodule` + `.swiftinterface`** — chính. Tách interface (text) và metadata (binary). Triết đi binary thuần để hash stability.
- **Mojo `.mojopkg`** — container format với metadata. Triết design tương tự nhưng đơn giản hơn (không có Mojo-specific tracing).
- **.NET assembly metadata tables** — same idea, nhiều entry kinds hơn. Triết minimal subset.
- **Java `.class` files** — không phải prior art tốt; Java mix bytecode + ABI vào một format → khó tách concern.

## Tham chiếu

- [VISION §3.3 — Stable ABI: Interface-First Design](../../VISION.md)
- [SPEC §10 — Memory model + ABI hooks (TBD ở v0.4)](../../SPEC.md)
- [ADR-0007 — IR design](0007-ir-design.md)
- [ADR-0008 — .triv binary format](0008-triv-binary-format.md)
- [ADR-0012 — Witness table dispatch](0012-witness-table-dispatch.md) (companion)
- [ADR-0013 — Semver linking policy](0013-semver-linking-policy.md) (companion)
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs)
